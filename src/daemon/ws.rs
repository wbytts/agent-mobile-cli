//! 桥接 WS 端口（design.md 决策 1/3/8）：hello 认证注册 → 心跳保活 → command/script 路由 → result 配对。
//!
//! 连接生命周期：
//! 1. 握手后首条消息必须是 hello（10s 超时），凭配对码或 token 认证（[`Pairing`]）；
//! 2. 认证通过注册到 [`BridgeRegistry`]，同设备重连替换旧连接；
//! 3. heartbeat 刷新 last_seen 并回 pong；result 按 id 唤醒等待方并回 result_ack；
//! 4. 连接断开时注册表标离线（设备记录保留）。

use crate::bridge_proto::{ClientMessage, DaemonMessage, HelloAck, ResultAck};
use crate::daemon::pair::Pairing;
use crate::daemon::registry::BridgeRegistry;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Notify};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// hello 等待超时：超时未收到合法 hello 即断开。
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// 在已绑定的 listener 上接受 WS 连接；`shutdown` 被通知后停止接受新连接。
pub async fn serve(
    listener: TcpListener,
    shutdown: Arc<Notify>,
    registry: Arc<BridgeRegistry>,
    pairing: Arc<Pairing>,
) -> std::io::Result<()> {
    loop {
        tokio::select! {
            _ = shutdown.notified() => return Ok(()),
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(handle_connection(
                    stream,
                    Arc::clone(&registry),
                    Arc::clone(&pairing),
                ));
            }
        }
    }
}

/// 单连接处理：握手 → hello 认证注册 → 读写循环 → 断线清理。
async fn handle_connection(
    stream: TcpStream,
    registry: Arc<BridgeRegistry>,
    pairing: Arc<Pairing>,
) {
    let ws = match accept_async(stream).await {
        Ok(ws) => ws,
        Err(_) => return, // 握手失败直接断开
    };
    let (mut sink, mut inbound) = ws.split();

    // 首条消息必须是 hello
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, inbound.next()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientMessage>(&text) {
            Ok(ClientMessage::Hello(h)) => Some(h),
            _ => None,
        },
        _ => None,
    };
    let Some(hello) = hello else {
        send_ack(
            &mut sink,
            HelloAck {
                ok: false,
                token: None,
                error: Some("首条消息必须是合法的 hello".to_string()),
            },
        )
        .await;
        return;
    };

    // 认证：配对码签发 token / token 直过 / 拒绝
    let ack = match pairing.authenticate(
        hello.pairing_code.as_deref(),
        hello.token.as_deref(),
        &hello.device_name,
    ) {
        crate::daemon::pair::AuthOutcome::Paired { token } => HelloAck {
            ok: true,
            token: Some(token),
            error: None,
        },
        crate::daemon::pair::AuthOutcome::TokenOk => HelloAck {
            ok: true,
            token: None,
            error: None,
        },
        crate::daemon::pair::AuthOutcome::Rejected { reason } => HelloAck {
            ok: false,
            token: None,
            error: Some(reason),
        },
    };
    if !ack.ok {
        send_ack(&mut sink, ack).await;
        return;
    }
    if !send_ack(&mut sink, ack).await {
        return;
    }

    // 注册设备：出站帧经 mpsc 由写循环发出
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let (device_id, conn_id) = registry.register(&hello, tx.clone());

    loop {
        tokio::select! {
            // 出站：注册表路由的 command/script/pong/result_ack
            out = rx.recv() => {
                let Some(text) = out else { break }; // 全部发送方消失
                if sink.send(Message::Text(text)).await.is_err() {
                    break;
                }
            }
            // 入站：heartbeat / result
            msg = inbound.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        registry.touch(&device_id, conn_id);
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Heartbeat) => {
                                let pong = serde_json::to_string(&DaemonMessage::Pong)
                                    .expect("pong 序列化");
                                let _ = tx.send(pong);
                            }
                            Ok(ClientMessage::Result(r)) => {
                                let id = r.id.clone();
                                registry.complete(r);
                                let ack = serde_json::to_string(&DaemonMessage::ResultAck(
                                    ResultAck { id },
                                ))
                                .expect("result_ack 序列化");
                                let _ = tx.send(ack);
                            }
                            // 重复 hello 或畸形帧：忽略保持连接
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // 二进制/ping/pong 帧忽略
                    Some(Err(_)) => break,
                }
            }
        }
    }
    registry.unregister(&device_id, conn_id);
}

/// 发送 hello_ack；返回是否发送成功。
async fn send_ack(
    sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<TcpStream>,
        Message,
    >,
    ack: HelloAck,
) -> bool {
    let text = serde_json::to_string(&DaemonMessage::HelloAck(ack)).expect("hello_ack 序列化");
    sink.send(Message::Text(text)).await.is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_proto::{Capability, ClientMessage, Hello, ResultMessage};
    use crate::daemon::pair::Pairing;
    use crate::daemon::registry::BridgeRegistry;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    struct Fixture {
        port: u16,
        registry: std::sync::Arc<BridgeRegistry>,
        pairing: std::sync::Arc<Pairing>,
        shutdown: std::sync::Arc<Notify>,
        _tmp: tempfile::TempDir,
    }

    async fn spawn_server() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let pairing = std::sync::Arc::new(Pairing::new(tmp.path()));
        let registry = BridgeRegistry::new();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let shutdown = std::sync::Arc::new(Notify::new());
        let (r, p, s) = (
            std::sync::Arc::clone(&registry),
            std::sync::Arc::clone(&pairing),
            std::sync::Arc::clone(&shutdown),
        );
        tokio::spawn(serve(listener, s, r, p));
        Fixture {
            port,
            registry,
            pairing,
            shutdown,
            _tmp: tmp,
        }
    }

    async fn connect(
        port: u16,
    ) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
    {
        let (ws, _) = connect_async(format!("ws://127.0.0.1:{port}"))
            .await
            .expect("WS 握手");
        ws
    }

    fn hello_msg(pairing_code: Option<&str>, token: Option<&str>) -> String {
        let h = ClientMessage::Hello(Hello {
            pairing_code: pairing_code.map(str::to_string),
            token: token.map(str::to_string),
            device_name: "MuMu".into(),
            android_version: "12".into(),
            capabilities: vec![Capability::Tap, Capability::UiTree, Capability::Script],
        });
        serde_json::to_string(&h).unwrap()
    }

    async fn send_text(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        text: String,
    ) {
        ws.send(Message::Text(text)).await.unwrap();
    }

    async fn recv_json(
        ws: &mut tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    ) -> Value {
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), ws.next())
            .await
            .expect("收消息超时")
            .expect("流应存活")
            .expect("消息有效");
        let Message::Text(text) = msg else {
            panic!("应为文本帧: {msg:?}")
        };
        serde_json::from_str(&text).unwrap()
    }

    #[tokio::test]
    async fn hello_with_pairing_code_registers_device() {
        let fx = spawn_server().await;
        let (code, _) = fx.pairing.pairing_code();
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, hello_msg(Some(&code), None)).await;
        let ack = recv_json(&mut ws).await;
        assert_eq!(ack["type"], "hello_ack");
        assert_eq!(ack["ok"], true);
        let token = ack["token"].as_str().expect("首次配对应下发 token");
        assert_eq!(token.len(), 64);

        // 注册后设备出现在枚举中且在线
        let records = fx.registry.device_records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, "bridge:MuMu");
        assert_eq!(records[0].state, crate::backend::DeviceState::Online);

        // 配对码一次性：第二次用同一配对码被拒
        let mut ws2 = connect(fx.port).await;
        send_text(&mut ws2, hello_msg(Some(&code), None)).await;
        let ack2 = recv_json(&mut ws2).await;
        assert_eq!(ack2["ok"], false);
        assert!(ack2["error"].as_str().unwrap().contains("配对码"));

        // 凭 token 重连免配对，不重复下发 token
        let mut ws3 = connect(fx.port).await;
        send_text(&mut ws3, hello_msg(None, Some(token))).await;
        let ack3 = recv_json(&mut ws3).await;
        assert_eq!(ack3["ok"], true);
        assert!(ack3.get("token").is_none());

        fx.shutdown.notify_one();
    }

    #[tokio::test]
    async fn hello_with_bad_credentials_rejected() {
        let fx = spawn_server().await;
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, hello_msg(Some("000000"), None)).await;
        let ack = recv_json(&mut ws).await;
        assert_eq!(ack["ok"], false);
        assert!(ack["error"].is_string());
        // 拒绝后不注册设备
        assert!(fx.registry.device_records().is_empty());
        fx.shutdown.notify_one();
    }

    #[tokio::test]
    async fn heartbeat_gets_pong_and_refreshes() {
        let fx = spawn_server().await;
        let (code, _) = fx.pairing.pairing_code();
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, hello_msg(Some(&code), None)).await;
        recv_json(&mut ws).await;

        send_text(&mut ws, r#"{"type":"heartbeat"}"#.into()).await;
        let pong = recv_json(&mut ws).await;
        assert_eq!(pong["type"], "pong");
        fx.shutdown.notify_one();
    }

    #[tokio::test]
    async fn command_forwarded_and_result_correlated() {
        let fx = spawn_server().await;
        let (code, _) = fx.pairing.pairing_code();
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, hello_msg(Some(&code), None)).await;
        recv_json(&mut ws).await;

        // daemon 侧下发 command
        let wait = fx
            .registry
            .command("bridge:MuMu", "tap", json!({"x": 1, "y": 2}))
            .expect("设备在线");
        let frame = recv_json(&mut ws).await;
        assert_eq!(frame["type"], "command");
        assert_eq!(frame["method"], "tap");
        assert_eq!(frame["params"], json!({"x": 1, "y": 2}));

        // 设备回传 result → 收到 result_ack，daemon 侧等待方被唤醒
        let result = ResultMessage {
            id: frame["id"].as_str().unwrap().to_string(),
            ok: true,
            result: Some(json!({"tapped": [1, 2]})),
            error: None,
        };
        send_text(
            &mut ws,
            serde_json::to_string(&ClientMessage::Result(result)).unwrap(),
        )
        .await;
        let ack = recv_json(&mut ws).await;
        assert_eq!(ack["type"], "result_ack");
        assert_eq!(ack["id"], frame["id"]);

        let r = tokio::time::timeout(std::time::Duration::from_secs(5), wait)
            .await
            .expect("result 应及时回传配对")
            .unwrap();
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["tapped"], json!([1, 2]));
        fx.shutdown.notify_one();
    }

    #[tokio::test]
    async fn disconnect_marks_device_offline() {
        let fx = spawn_server().await;
        let (code, _) = fx.pairing.pairing_code();
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, hello_msg(Some(&code), None)).await;
        recv_json(&mut ws).await;
        assert_eq!(
            fx.registry.device_records()[0].state,
            crate::backend::DeviceState::Online
        );

        drop(ws);
        // 等连接循环清理
        for _ in 0..50 {
            if fx.registry.device_records()[0].state == crate::backend::DeviceState::Offline {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("断线后应标记离线");
    }

    #[tokio::test]
    async fn non_hello_first_message_rejected() {
        let fx = spawn_server().await;
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, r#"{"type":"heartbeat"}"#.into()).await;
        let ack = recv_json(&mut ws).await;
        assert_eq!(ack["type"], "hello_ack");
        assert_eq!(ack["ok"], false);
        assert!(fx.registry.device_records().is_empty());
        fx.shutdown.notify_one();
    }

    #[tokio::test]
    async fn script_forwarded_to_device() {
        let fx = spawn_server().await;
        let (code, _) = fx.pairing.pairing_code();
        let mut ws = connect(fx.port).await;
        send_text(&mut ws, hello_msg(Some(&code), None)).await;
        recv_json(&mut ws).await;

        let wait = fx
            .registry
            .script("bridge:MuMu", "mobile.tap(1,2)")
            .unwrap();
        let frame = recv_json(&mut ws).await;
        assert_eq!(frame["type"], "script");
        assert_eq!(frame["source"], "mobile.tap(1,2)");
        let result = ResultMessage {
            id: frame["id"].as_str().unwrap().to_string(),
            ok: false,
            result: None,
            error: Some("沙盒异常".into()),
        };
        send_text(
            &mut ws,
            serde_json::to_string(&ClientMessage::Result(result)).unwrap(),
        )
        .await;
        recv_json(&mut ws).await; // result_ack
        let r = wait.await.unwrap();
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("沙盒异常"));
        fx.shutdown.notify_one();
    }
}
