//! WS `/ws/device`：App 绑定端点（design.md 决策 3，帧协议与 bridge-protocol 一致）。
//!
//! 生命周期：握手后首帧必须是 hello（10s 超时，可注入）→ 配对码/token 双路径认证 →
//! hello_ack（首次配对下发设备 token）→ 注册到 relay → 消息循环 → 断开标离线（记录保留）。

use axum::extract::ws::{Message, WebSocket};
use std::sync::Arc;
use tokio::time::timeout;

use crate::bridge_proto::{ClientMessage, DaemonMessage, Hello, HelloAck, ResultAck};
use crate::relay::JobFail;
use crate::AppState;

/// 持锁发送 hello_ack 的硬超时：锁内网络写必须有界（ReReview SUGGESTION）。
const ACK_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// axum 路由入口：升级 WS 后交由 [`handle_socket`]。
pub async fn ws_device(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// 单连接处理：hello 认证注册 → 读写循环 → 断线清理。
async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    // 1. 首帧必须是 hello（超时静默关闭）。
    let hello = match timeout(state.config.hello_timeout, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Hello(h)) => h,
            _ => {
                send_ack(
                    &mut socket,
                    HelloAck {
                        ok: false,
                        token: None,
                        error: Some("首帧必须是 hello".to_string()),
                    },
                )
                .await;
                return;
            }
        },
        _ => return,
    };

    // 2. 认证：配对码或设备 token 二选一。
    //    auth_lock 持有至步骤 4 注册完成：与 pairing_reset 互斥（FixReview IMPORTANT-1）。
    let auth_guard = state.auth_lock.lock().await;
    let (owner, issued_token) = match authenticate(&state, &hello) {
        Ok(ok) => ok,
        Err(error) => {
            // 认证失败不参与注册，无需持锁——先放锁再回 ack，避免慢读客户端
            // 经锁内网络写占住全局认证锁（ReReview SUGGESTION）。
            drop(auth_guard);
            send_ack(
                &mut socket,
                HelloAck {
                    ok: false,
                    token: None,
                    error: Some(error),
                },
            )
            .await;
            return;
        }
    };

    // 3. hello_ack：首次配对附带新签发 token。
    //    持锁内网络写加超时：慢读/不读客户端不能无限期占住 auth_lock（ReReview SUGGESTION）。
    let ok = timeout(
        ACK_SEND_TIMEOUT,
        send_ack(
            &mut socket,
            HelloAck {
                ok: true,
                token: issued_token,
                error: None,
            },
        ),
    )
    .await
    .unwrap_or(false);
    if !ok {
        // 配对码已消费且 token 已签发但未送达/未确认：留下永不上线的孤儿设备记录。
        // 可恢复（重新签发配对码同名配对会轮换 token），此处记录便于排查
        // 「配对成功但设备从未上线」（ReReview SUGGESTION）。
        eprintln!(
            "hello_ack 发送失败/超时：owner 设备 '{}' 可能已配对但未收到 token",
            hello.device_name
        );
        return;
    }

    // 4. 注册在线；同设备重连替换旧连接。本任务即该设备的串行工作协程（design.md 决策 9）。
    let (conn_id, mut job_rx, close) = state.relay.register(&owner, &hello.device_name);
    state.store.touch_device(&owner, &hello.device_name);
    drop(auth_guard); // 注册完成，认证+注册原子段结束

    // 5. 消息循环：
    //    - heartbeat → pong 并刷新 last_seen；任意消息重置静默计时，超时断开标离线；
    //    - HTTP 命令入队（current 为空时才取下一条 → 严格串行），下发后挂起等 result；
    //    - result 按 id 配对唤醒 HTTP 等待方并回 result_ack；未知/迟到 id 静默丢弃。
    let hb_timeout = state.config.heartbeat_timeout;
    let cmd_timeout = state.config.command_timeout;
    let mut last_activity = tokio::time::Instant::now();
    let mut current: Option<CurrentJob> = None;
    loop {
        // 有在途命令时启用其超时分支（先拷出 deadline，避免借用冲突）
        let deadline = current.as_ref().map(|c| c.deadline);
        let cmd_deadline = async move {
            match deadline {
                Some(d) => tokio::time::sleep_until(d).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::pin!(cmd_deadline);
        tokio::select! {
            _ = close.notified() => break,
            _ = &mut cmd_deadline => {
                if let Some(c) = current.take() {
                    let _ = c.reply.send(Err(JobFail::Timeout));
                }
            }
            _ = tokio::time::sleep_until(last_activity + hb_timeout) => break,
            job = job_rx.recv(), if current.is_none() => {
                match job {
                    Some(job) => {
                        let text = match serde_json::to_string(&job.frame) {
                            Ok(t) => t,
                            Err(_) => {
                                drop(job.reply);
                                continue;
                            }
                        };
                        if socket.send(Message::Text(text)).await.is_err() {
                            drop(job.reply);
                            break;
                        }
                        current = Some(CurrentJob {
                            id: job.id,
                            reply: job.reply,
                            deadline: tokio::time::Instant::now() + cmd_timeout,
                        });
                    }
                    None => break, // 注册表已移除本设备（reset）
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_activity = tokio::time::Instant::now();
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Heartbeat) => {
                                state.store.touch_device(&owner, &hello.device_name);
                                let pong = serde_json::to_string(&DaemonMessage::Pong).unwrap();
                                if socket.send(Message::Text(pong)).await.is_err() {
                                    break;
                                }
                            }
                            Ok(ClientMessage::Result(result)) => {
                                // 协议语义：收到 result 即回 result_ack
                                let ack = serde_json::to_string(&DaemonMessage::ResultAck(
                                    ResultAck { id: result.id.clone() },
                                ))
                                .unwrap();
                                if socket.send(Message::Text(ack)).await.is_err() {
                                    break;
                                }
                                // 按 id 配对在途命令；未知/迟到 id 静默丢弃
                                if matches!(&current, Some(c) if c.id == result.id) {
                                    let c = current.take().unwrap();
                                    let _ = c.reply.send(Ok(result));
                                }
                            }
                            // 重复 hello 等帧忽略
                            _ => {}
                        }
                    }
                    // 协议层 ping/pong 也算活跃
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                        last_activity = tokio::time::Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }

    // 6. 断线清理：标离线（设备记录保留）；drop 在途/排队 reply 唤醒 HTTP 侧为离线错误。
    state.relay.unregister(&owner, &hello.device_name, conn_id);
    drop(current);
    while let Ok(job) = job_rx.try_recv() {
        drop(job.reply);
    }
}

/// 在途命令：等待设备 result。
struct CurrentJob {
    id: String,
    reply: tokio::sync::oneshot::Sender<crate::relay::JobOutcome>,
    deadline: tokio::time::Instant,
}

/// 认证：配对码路径签发设备 token；token 路径校验并要求设备名匹配。
fn authenticate(state: &AppState, hello: &Hello) -> Result<(String, Option<String>), String> {
    validate_device_name(&hello.device_name)?;
    if let Some(code) = &hello.pairing_code {
        let owner = state
            .store
            .consume_pairing_code(code)
            .ok_or_else(|| "配对码错误或已失效".to_string())?;
        let token = state
            .store
            .pair_device(
                &owner,
                &hello.device_name,
                &hello.android_version,
                &hello.capabilities,
            )
            .ok_or_else(|| "owner 不存在".to_string())?;
        Ok((owner, Some(token)))
    } else if let Some(token) = &hello.token {
        let (owner, record) = state
            .store
            .verify_device_token(token)
            .ok_or_else(|| "设备 token 无效".to_string())?;
        if record.name != hello.device_name {
            return Err("token 与设备名不匹配".to_string());
        }
        state.store.update_device_meta(
            &owner,
            &hello.device_name,
            &hello.android_version,
            &hello.capabilities,
        );
        Ok((owner, None))
    } else {
        Err("缺少配对码或设备 token".to_string())
    }
}

/// 设备名校验（FixReview IMPORTANT-2）：非空、≤64 字符，拒绝 `/ ? # %` 与控制字符
/// （设备名会进入 HTTP 路径段 `/devices/:name/...`，这些字符破坏路径语义；
/// 空格与中文等合法——CLI 侧做百分编码）。
fn validate_device_name(name: &str) -> Result<(), String> {
    let len = name.chars().count();
    if len == 0 || len > 64 {
        return Err("设备名长度非法（1-64 字符）".to_string());
    }
    if name
        .chars()
        .any(|c| matches!(c, '/' | '?' | '#' | '%') || c.is_control())
    {
        return Err("设备名含非法字符（/ ? # % 或控制字符）".to_string());
    }
    Ok(())
}

/// 发送 hello_ack；返回是否发送成功。
async fn send_ack(socket: &mut WebSocket, ack: HelloAck) -> bool {
    let text = match serde_json::to_string(&DaemonMessage::HelloAck(ack)) {
        Ok(t) => t,
        Err(_) => return false,
    };
    socket.send(Message::Text(text)).await.is_ok()
}

#[cfg(test)]
mod tests {
    use crate::bridge_proto::{Capability, ClientMessage, DaemonMessage, Hello};
    use crate::testutil;
    use std::sync::Arc;
    use std::time::Duration;

    fn hello(pairing_code: Option<&str>, token: Option<&str>, name: &str) -> ClientMessage {
        ClientMessage::Hello(Hello {
            pairing_code: pairing_code.map(str::to_string),
            token: token.map(str::to_string),
            device_name: name.to_string(),
            android_version: "14".to_string(),
            capabilities: vec![Capability::Tap, Capability::UiTree],
        })
    }

    /// 配对码路径：hello 提交一次性配对码 → hello_ack ok + 下发 64 hex 设备 token。
    #[tokio::test]
    async fn 配对码hello_签发设备token() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let code = handle
            .state
            .store
            .issue_pairing_code(&handle.owner_token)
            .unwrap();

        let mut ws = testutil::ws_connect(addr).await;
        testutil::ws_send(&mut ws, &hello(Some(&code), None, "dev-1")).await;
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 hello_ack");
        match ack {
            DaemonMessage::HelloAck(ack) => {
                assert!(ack.ok, "配对码合法应通过: {:?}", ack.error);
                let token = ack.token.expect("首次配对下发设备 token");
                assert_eq!(token.len(), 64);
                // 设备记录已持久化，token 可反查
                let (owner, rec) = handle.state.store.verify_device_token(&token).unwrap();
                assert_eq!(owner, handle.owner_token);
                assert_eq!(rec.name, "dev-1");
            }
            other => panic!("预期 hello_ack，实际 {other:?}"),
        }
    }

    /// /ws 别名路径（与 LAN daemon 一致）：App 连接代理无需感知目标类型（决策 12）。
    #[tokio::test]
    async fn ws别名路径_同样完成配对握手() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let code = handle
            .state
            .store
            .issue_pairing_code(&handle.owner_token)
            .unwrap();

        let (mut ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .expect("/ws 别名应可连接");
        testutil::ws_send(&mut ws, &hello(Some(&code), None, "dev-alias")).await;
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 hello_ack");
        match ack {
            DaemonMessage::HelloAck(ack) => assert!(ack.ok, "别名录径配对应通过: {:?}", ack.error),
            other => panic!("期望 hello_ack，实际 {other:?}"),
        }
    }

    /// token 路径：已配对设备凭 token 重连 → hello_ack ok 且不重复下发 token。
    #[tokio::test]
    async fn token_hello_恢复在线() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let token = handle
            .state
            .store
            .pair_device(&handle.owner_token, "dev-2", "14", &[Capability::Tap])
            .unwrap();

        let mut ws = testutil::ws_connect(addr).await;
        testutil::ws_send(&mut ws, &hello(None, Some(&token), "dev-2")).await;
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 hello_ack");
        match ack {
            DaemonMessage::HelloAck(ack) => {
                assert!(ack.ok, "token 合法应通过: {:?}", ack.error);
                assert!(ack.token.is_none(), "token 路径不重复下发");
            }
            other => panic!("预期 hello_ack，实际 {other:?}"),
        }
    }

    /// 设备名校验（FixReview IMPORTANT-2）：含 `/` 等路径保留字符的设备名被拒绝。
    #[tokio::test]
    async fn 非法设备名拒绝并断开() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let code = handle
            .state
            .store
            .issue_pairing_code(&handle.owner_token)
            .unwrap();

        let mut ws = testutil::ws_connect(addr).await;
        testutil::ws_send(&mut ws, &hello(Some(&code), None, "bad/name")).await;
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 hello_ack");
        match ack {
            DaemonMessage::HelloAck(ack) => {
                assert!(!ack.ok, "含 / 的设备名应被拒绝");
                assert!(
                    ack.error.unwrap_or_default().contains("设备名"),
                    "错误应指明设备名非法"
                );
            }
            other => panic!("预期 hello_ack，实际 {other:?}"),
        }
        // 拒绝的 hello 不得注册设备
        assert!(handle
            .state
            .store
            .devices_of(&handle.owner_token)
            .is_empty());
    }

    /// 认证+注册与 pairing_reset 互斥（FixReview IMPORTANT-1）：持锁期间 hello 阻塞，
    /// 放锁后正常完成。锁的存在保证「reset 执行中无设备能以旧凭证注册上线」。
    #[tokio::test]
    async fn hello_与reset互斥_持锁期间阻塞() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let token = handle
            .state
            .store
            .pair_device(&handle.owner_token, "dev-lock", "14", &[Capability::Tap])
            .unwrap();

        let guard = handle.state.auth_lock.lock().await;
        let mut ws = testutil::ws_connect(addr).await;
        testutil::ws_send(&mut ws, &hello(None, Some(&token), "dev-lock")).await;
        // 锁被持有：300ms 内不应收到 hello_ack
        let early: Option<DaemonMessage> =
            testutil::ws_recv(&mut ws, Duration::from_millis(300)).await;
        assert!(early.is_none(), "持锁期间 hello 应阻塞，实际收到 {early:?}");
        drop(guard);
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("放锁后应收到 hello_ack");
        match ack {
            DaemonMessage::HelloAck(ack) => assert!(ack.ok, "放锁后认证应通过: {:?}", ack.error),
            other => panic!("预期 hello_ack，实际 {other:?}"),
        }
    }

    /// 拒绝路径：错误配对码、错误 token、无凭证均 hello_ack ok:false 后断开。
    #[tokio::test]
    async fn 非法凭证拒绝并断开() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let valid = handle
            .state
            .store
            .pair_device(&handle.owner_token, "dev-3", "14", &[])
            .unwrap();

        for bad_hello in [
            hello(Some("999999"), None, "dev-x"),         // 错误配对码
            hello(None, Some(&"ff".repeat(32)), "dev-3"), // 错误 token
            hello(None, None, "dev-x"),                   // 无凭证
            hello(None, Some(&valid), "dev-other"),       // token 与设备名不匹配
        ] {
            let mut ws = testutil::ws_connect(addr).await;
            testutil::ws_send(&mut ws, &bad_hello).await;
            let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
                .await
                .expect("拒绝也回 hello_ack");
            match ack {
                DaemonMessage::HelloAck(ack) => {
                    assert!(!ack.ok, "{bad_hello:?} 应被拒绝");
                    assert!(ack.error.is_some(), "拒绝需附原因");
                }
                other => panic!("预期 hello_ack，实际 {other:?}"),
            }
        }
    }

    /// hello 首帧超时：连接后沉默超过 hello_timeout 即被断开。
    #[tokio::test]
    async fn hello超时断开() {
        let (addr, _handle, _dir) = testutil::spawn_test_server().await;
        let mut ws = testutil::ws_connect(addr).await;
        // testutil hello_timeout = 500ms；1s 内必须收到关闭
        let closed = testutil::ws_recv::<DaemonMessage>(&mut ws, Duration::from_secs(2)).await;
        assert!(closed.is_none(), "超时后连接应被服务端关闭");
    }

    /// 已配对设备快捷连接：token hello 成功返回 (ws, token)。
    async fn paired_ws(
        addr: std::net::SocketAddr,
        handle: &Arc<crate::testutil::TestHandle>,
        name: &str,
    ) -> (crate::testutil::WsClient, String) {
        let token = handle
            .state
            .store
            .pair_device(&handle.owner_token, name, "14", &[Capability::Tap])
            .unwrap();
        let mut ws = testutil::ws_connect(addr).await;
        testutil::ws_send(&mut ws, &hello(None, Some(&token), name)).await;
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 hello_ack");
        match ack {
            DaemonMessage::HelloAck(a) if a.ok => {}
            other => panic!("hello 应通过: {other:?}"),
        }
        (ws, token)
    }

    /// 心跳：heartbeat → pong。
    #[tokio::test]
    async fn 心跳回pong() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let (mut ws, _token) = paired_ws(addr, &handle, "hb-dev").await;
        testutil::ws_send(&mut ws, &ClientMessage::Heartbeat).await;
        let pong: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 pong");
        assert!(matches!(pong, DaemonMessage::Pong), "预期 pong: {pong:?}");
    }

    /// 静默超过 heartbeat_timeout 标离线；断开连接保留设备记录。
    #[tokio::test]
    async fn 静默超时标离线_记录保留() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let owner = handle.owner_token.clone();
        let (_ws, _token) = paired_ws(addr, &handle, "idle-dev").await;
        assert!(handle.state.relay.is_online(&owner, "idle-dev"));

        // heartbeat_timeout=150ms；轮询最多 2s 等待离线标记（连接仍持有但无消息）
        let mut offline = false;
        for _ in 0..60 {
            if !handle.state.relay.is_online(&owner, "idle-dev") {
                offline = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(offline, "静默超时后应标离线");
        assert!(
            handle
                .state
                .store
                .devices_of(&owner)
                .iter()
                .any(|d| d.name == "idle-dev"),
            "离线后设备记录保留"
        );
    }

    /// 主动断开：标离线；凭 token 重连恢复在线。
    #[tokio::test]
    async fn 断开重连恢复在线() {
        let (addr, handle, _dir) = testutil::spawn_test_server().await;
        let owner = handle.owner_token.clone();
        let (ws, token) = paired_ws(addr, &handle, "re-dev").await;
        drop(ws);
        for _ in 0..60 {
            if !handle.state.relay.is_online(&owner, "re-dev") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            !handle.state.relay.is_online(&owner, "re-dev"),
            "断开后离线"
        );

        // 重连：token 仍在，hello 后恢复在线
        let mut ws = testutil::ws_connect(addr).await;
        testutil::ws_send(&mut ws, &hello(None, Some(&token), "re-dev")).await;
        let ack: DaemonMessage = testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(ack, DaemonMessage::HelloAck(ref a) if a.ok));
        assert!(handle.state.relay.is_online(&owner, "re-dev"), "重连后在线");
    }
}
