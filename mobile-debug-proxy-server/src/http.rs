//! REST 路由（design.md 决策 3）。
//!
//! 响应形状为契约固定格式：错误一律 `{"error": "<desc>"}` + 对应状态码；
//! `GET /healthz` 免认证，其余路由经 Bearer 中间件（见 `auth` 模块）。

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, Router};
use axum::{middleware, Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::auth::{require_owner, OwnerIdentity};
use crate::relay::{CommandBody, DispatchError};
use crate::AppState;

/// 组装完整路由：/healthz 免认证，/ws/device 与业务 API 分开挂 Bearer 中间件。
pub fn router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/devices", get(list_devices))
        .route("/devices/:name/commands", post(post_command))
        .route("/devices/:name/scripts", post(post_script))
        .route("/pairing-codes", post(issue_pairing_code))
        .route("/pairing-reset", post(pairing_reset))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_owner));
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws/device", get(crate::ws::ws_device))
        // /ws 别名：与 LAN daemon 桥接路径一致，App 连接代理时无需感知目标类型（决策 12）
        .route("/ws", get(crate::ws::ws_device))
        .merge(protected)
        .with_state(state)
}

/// GET /healthz：无认证健康检查。
async fn healthz() -> Json<Value> {
    Json(json!({"ok": true, "service": "mobile-debug-proxy-server"}))
}

/// POST /pairing-codes：签发一次性配对码 → 200 {"pairing_code":"483920"}。
async fn issue_pairing_code(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<OwnerIdentity>,
    // 显式消费请求体：handler 不读 body 时 hyper 关闭连接携带未读数据会触发 TCP RST，
    // 客户端（ureq）可能丢响应（压测间歇复现：200 + 空 body；生产 CLI 同样受影响）。
    axum::Json(_body): axum::Json<Value>,
) -> Response {
    match state.store.issue_pairing_code(&owner.0) {
        Some(code) => Json(json!({"pairing_code": code})).into_response(),
        // 认证已通过却找不到 owner：数据文件被外部清空，属内部异常
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "owner 不存在"})),
        )
            .into_response(),
    }
}

/// POST /pairing-reset：重置配对码、吊销该 owner 全部设备 token、断开其已连接设备
/// → 200 {"reset":true}。
async fn pairing_reset(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<OwnerIdentity>,
    // 同 issue_pairing_code：显式消费请求体，避免响应被 TCP RST 丢弃。
    axum::Json(_body): axum::Json<Value>,
) -> Response {
    // 与 WS hello 认证+注册互斥（FixReview IMPORTANT-1）：持锁完成吊销+断连两步，
    // 在途 hello 要么先于 reset 完成注册（随后被 disconnect_owner 断开），要么在锁后
    // 认证（此时 token/配对码已吊销，认证失败）。
    let _auth_guard = state.auth_lock.lock().await;
    match state.store.reset_owner(&owner.0) {
        Some(_revoked) => {
            state.relay.disconnect_owner(&owner.0);
            Json(json!({"reset": true})).into_response()
        }
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "owner 不存在"})),
        )
            .into_response(),
    }
}

/// GET /devices：owner 名下设备枚举（name/online/capabilities/last_seen RFC3339）。
async fn list_devices(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<OwnerIdentity>,
) -> Json<Value> {
    let devices: Vec<Value> = state
        .store
        .devices_of(&owner.0)
        .into_iter()
        .map(|d| {
            json!({
                "name": d.name,
                "online": state.relay.is_online(&owner.0, &d.name),
                "capabilities": d.capabilities,
                "last_seen": rfc3339(d.last_seen),
            })
        })
        .collect();
    Json(json!({"devices": devices}))
}

/// POST /devices/:name/commands {method, params}：中继 command 并同步等待 result。
async fn post_command(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<OwnerIdentity>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(method) = body.get("method").and_then(Value::as_str) else {
        return bad_request("缺少 method 字段或类型不是字符串");
    };
    let params = body.get("params").cloned().unwrap_or(Value::Null);
    relay_command(
        &state,
        &owner.0,
        &name,
        CommandBody::Command {
            method: method.to_string(),
            params,
        },
    )
    .await
}

/// POST /devices/:name/scripts {source}：中继 script 并同步等待 result。
async fn post_script(
    State(state): State<Arc<AppState>>,
    Extension(owner): Extension<OwnerIdentity>,
    Path(name): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let Some(source) = body.get("source").and_then(Value::as_str) else {
        return bad_request("缺少 source 字段或类型不是字符串");
    };
    relay_command(
        &state,
        &owner.0,
        &name,
        CommandBody::Script {
            source: source.to_string(),
        },
    )
    .await
}

/// 中继公共路径：成功/设备错误 → 200；离线或不存在 → 404 同形状（不泄露存在性）；
/// 超时 → 504（契约固定形状）。
async fn relay_command(state: &AppState, owner: &str, name: &str, body: CommandBody) -> Response {
    match state.relay.dispatch(owner, name, body).await {
        Ok(res) => {
            if res.ok {
                Json(json!({"ok": true, "result": res.result})).into_response()
            } else {
                Json(json!({
                    "ok": false,
                    "error": res.error.unwrap_or_else(|| "设备侧执行失败".to_string()),
                }))
                .into_response()
            }
        }
        Err(DispatchError::Offline) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "device not found or offline"})),
        )
            .into_response(),
        Err(DispatchError::Timeout) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(json!({"error": "timeout waiting for device result"})),
        )
            .into_response(),
    }
}

/// 400 参数错误（契约固定形状）。
fn bad_request(desc: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": desc}))).into_response()
}

/// unix 秒 → RFC3339（UTC，`YYYY-MM-DDTHH:MM:SSZ`），不引入 chrono。
fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, mi, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    // civil-from-days（Howard Hinnant 算法）
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_纪元与已知时刻() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_759_161_600), "2025-09-29T16:00:00Z");
    }

    /// POST /pairing-codes：签发 6 位配对码。
    #[tokio::test]
    async fn pairing_codes_签发() {
        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let (status, body) = crate::testutil::http_post(
            addr,
            "/pairing-codes",
            Some(&handle.owner_token),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        let code = body["pairing_code"].as_str().expect("返回 pairing_code");
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    /// POST /pairing-reset：吊销全部设备 token 并断开已连接设备；返回 {"reset":true}。
    #[tokio::test]
    async fn pairing_reset_吊销并断开设备() {
        use crate::bridge_proto::{Capability, ClientMessage, DaemonMessage, Hello};
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let owner = handle.owner_token.clone();
        let dev_token = handle
            .state
            .store
            .pair_device(&owner, "reset-dev", "14", &[Capability::Tap])
            .unwrap();

        // 设备在线
        let mut ws = crate::testutil::ws_connect(addr).await;
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Hello(Hello {
                pairing_code: None,
                token: Some(dev_token.clone()),
                device_name: "reset-dev".to_string(),
                android_version: "14".to_string(),
                capabilities: vec![Capability::Tap],
            }),
        )
        .await;
        let ack: DaemonMessage = crate::testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(ack, DaemonMessage::HelloAck(ref a) if a.ok));
        assert!(handle.state.relay.is_online(&owner, "reset-dev"));

        // reset：返回 {"reset":true}
        let (status, body) =
            crate::testutil::http_post(addr, "/pairing-reset", Some(&owner), serde_json::json!({}))
                .await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body, serde_json::json!({"reset": true}));

        // 已连接设备被断开
        let closed: Option<DaemonMessage> =
            crate::testutil::ws_recv(&mut ws, Duration::from_secs(2)).await;
        assert!(closed.is_none(), "reset 后已连接设备应被断开");
        assert!(!handle.state.relay.is_online(&owner, "reset-dev"));

        // 旧设备 token 已吊销：重连被拒
        let mut ws2 = crate::testutil::ws_connect(addr).await;
        crate::testutil::ws_send(
            &mut ws2,
            &ClientMessage::Hello(Hello {
                pairing_code: None,
                token: Some(dev_token),
                device_name: "reset-dev".to_string(),
                android_version: "14".to_string(),
                capabilities: vec![],
            }),
        )
        .await;
        let ack: DaemonMessage = crate::testutil::ws_recv(&mut ws2, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(
            matches!(ack, DaemonMessage::HelloAck(ref a) if !a.ok),
            "旧 token 应被拒"
        );
    }

    /// reset 后新配对码可完成完整配对；配对码一次性、旧 token 失效。
    #[tokio::test]
    async fn reset后重新配对_配对码一次性() {
        use crate::bridge_proto::{Capability, ClientMessage, DaemonMessage, Hello};
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let owner = handle.owner_token.clone();
        let old_token = handle
            .state
            .store
            .pair_device(&owner, "re-dev", "14", &[Capability::Tap])
            .unwrap();
        crate::testutil::http_post(addr, "/pairing-reset", Some(&owner), serde_json::json!({}))
            .await;

        // 新配对码配对成功
        let (status, body) =
            crate::testutil::http_post(addr, "/pairing-codes", Some(&owner), serde_json::json!({}))
                .await;
        let code = body["pairing_code"]
            .as_str()
            .unwrap_or_else(|| panic!("响应缺少 pairing_code: status={status} body={body}"))
            .to_string();
        let mut ws = crate::testutil::ws_connect(addr).await;
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Hello(Hello {
                pairing_code: Some(code.clone()),
                token: None,
                device_name: "re-dev".to_string(),
                android_version: "14".to_string(),
                capabilities: vec![Capability::Tap],
            }),
        )
        .await;
        let ack: DaemonMessage = crate::testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(ack, DaemonMessage::HelloAck(ref a) if a.ok && a.token.is_some()));

        // 旧 token 失效、配对码一次性
        assert!(handle.state.store.verify_device_token(&old_token).is_none());
        let mut ws2 = crate::testutil::ws_connect(addr).await;
        crate::testutil::ws_send(
            &mut ws2,
            &ClientMessage::Hello(Hello {
                pairing_code: Some(code),
                token: None,
                device_name: "re-dev-2".to_string(),
                android_version: "14".to_string(),
                capabilities: vec![],
            }),
        )
        .await;
        let ack: DaemonMessage = crate::testutil::ws_recv(&mut ws2, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(
            matches!(ack, DaemonMessage::HelloAck(ref a) if !a.ok),
            "配对码一次性"
        );
    }

    /// 模拟 App：配对并连接，返回 ws。
    async fn app_connect(
        addr: std::net::SocketAddr,
        owner: &str,
        state: &crate::AppState,
        name: &str,
    ) -> crate::testutil::WsClient {
        use crate::bridge_proto::{Capability, ClientMessage, DaemonMessage, Hello};
        use std::time::Duration;

        let token = state
            .store
            .pair_device(owner, name, "14", &[Capability::Tap, Capability::Script])
            .unwrap();
        let mut ws = crate::testutil::ws_connect(addr).await;
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Hello(Hello {
                pairing_code: None,
                token: Some(token),
                device_name: name.to_string(),
                android_version: "14".to_string(),
                capabilities: vec![Capability::Tap, Capability::Script],
            }),
        )
        .await;
        let ack: DaemonMessage = crate::testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(matches!(ack, DaemonMessage::HelloAck(ref a) if a.ok));
        ws
    }

    /// 等待下一条 command 帧（跳过 result_ack/pong 等协议帧）。
    async fn recv_command(
        ws: &mut crate::testutil::WsClient,
    ) -> crate::bridge_proto::CommandMessage {
        use crate::bridge_proto::DaemonMessage;
        use std::time::Duration;
        for _ in 0..8 {
            match crate::testutil::ws_recv::<DaemonMessage>(ws, Duration::from_secs(2)).await {
                Some(DaemonMessage::Command(c)) => return c,
                Some(_) => continue, // result_ack / pong 等
                None => panic!("等待 command 帧超时或连接关闭"),
            }
        }
        panic!("连续 8 帧均非 command")
    }

    /// 成功路径：HTTP command → App 收到 cmd-<n> 帧 → App 回 result → HTTP 200。
    #[tokio::test]
    async fn command_中继成功() {
        use crate::bridge_proto::{ClientMessage, DaemonMessage, ResultMessage};
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let mut ws = app_connect(addr, &handle.owner_token, &handle.state, "tap-dev").await;

        let owner = handle.owner_token.clone();
        let http = tokio::spawn(crate::testutil::http_post(
            addr,
            "/devices/tap-dev/commands",
            Some(&owner),
            serde_json::json!({"method": "tap", "params": {"x": 100, "y": 200}}),
        ));

        // App 侧：收到 command 帧（id 形如 cmd-<n>），回 result
        let cmd: DaemonMessage = crate::testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("App 应收到 command 帧");
        let id = match cmd {
            DaemonMessage::Command(c) => {
                assert_eq!(c.method, "tap");
                assert_eq!(c.params, serde_json::json!({"x": 100, "y": 200}));
                assert!(c.id.starts_with("cmd-"), "服务侧分配 cmd-<n>: {}", c.id);
                c.id
            }
            other => panic!("预期 command 帧: {other:?}"),
        };
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id: id.clone(),
                ok: true,
                result: Some(serde_json::json!({"tapped": true})),
                error: None,
            }),
        )
        .await;

        // App 收到 result_ack
        let ack: DaemonMessage = crate::testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("应收到 result_ack");
        assert!(matches!(ack, DaemonMessage::ResultAck(ref a) if a.id == id));

        // HTTP 返回成功结果
        let (status, body) = http.await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], serde_json::json!(true));
        assert_eq!(body["result"], serde_json::json!({"tapped": true}));
    }

    /// script 路径：POST scripts {source} → App 收到 script 帧；设备侧错误透传。
    #[tokio::test]
    async fn script_中继成功() {
        use crate::bridge_proto::{ClientMessage, DaemonMessage, ResultMessage};
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let mut ws = app_connect(addr, &handle.owner_token, &handle.state, "js-dev").await;

        let owner = handle.owner_token.clone();
        let http = tokio::spawn(crate::testutil::http_post(
            addr,
            "/devices/js-dev/scripts",
            Some(&owner),
            serde_json::json!({"source": "mobile.tap(1,2)"}),
        ));
        let frame: DaemonMessage = crate::testutil::ws_recv(&mut ws, Duration::from_secs(2))
            .await
            .expect("App 应收到 script 帧");
        let id = match frame {
            DaemonMessage::Script(s) => {
                assert_eq!(s.source, "mobile.tap(1,2)");
                s.id
            }
            other => panic!("预期 script 帧: {other:?}"),
        };
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id,
                ok: false,
                result: None,
                error: Some("脚本执行失败".to_string()),
            }),
        )
        .await;
        let (status, body) = http.await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["ok"], serde_json::json!(false));
        assert_eq!(body["error"], serde_json::json!("脚本执行失败"));
    }

    /// 设备离线（未连接）：立即 404 {"error":"device not found or offline"}。
    #[tokio::test]
    async fn command_设备离线立即404() {
        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        handle
            .state
            .store
            .pair_device(&handle.owner_token, "off-dev", "14", &[]);
        let (status, body) = crate::testutil::http_post(
            addr,
            "/devices/off-dev/commands",
            Some(&handle.owner_token),
            serde_json::json!({"method": "tap", "params": {}}),
        )
        .await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(
            body,
            serde_json::json!({"error": "device not found or offline"})
        );
    }

    /// 超时：App 收到命令但不回 result → 504；迟到 result 静默丢弃，后续命令不受影响。
    #[tokio::test]
    async fn command_超时与迟到result丢弃() {
        use crate::bridge_proto::{ClientMessage, DaemonMessage, ResultMessage};
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let mut ws = app_connect(addr, &handle.owner_token, &handle.state, "slow-dev").await;

        // 第一条：App 收到但暂不回
        let owner = handle.owner_token.clone();
        let http = tokio::spawn(crate::testutil::http_post(
            addr,
            "/devices/slow-dev/commands",
            Some(&owner),
            serde_json::json!({"method": "uiTree", "params": {}}),
        ));
        let id = recv_command(&mut ws).await.id;
        // command_timeout=150ms → 504
        let (status, body) = http.await.unwrap();
        assert_eq!(status, 504, "{body}");
        assert_eq!(
            body,
            serde_json::json!({"error": "timeout waiting for device result"})
        );

        // 迟到 result + 未知 id result：均被静默丢弃（不 panic、不串扰）
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id: id.clone(),
                ok: true,
                result: Some(serde_json::json!({"late": true})),
                error: None,
            }),
        )
        .await;
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id: "cmd-99999".to_string(),
                ok: true,
                result: None,
                error: None,
            }),
        )
        .await;
        // result_ack 仍回（协议语义），随后无异常帧
        let _ack: Option<DaemonMessage> =
            crate::testutil::ws_recv(&mut ws, Duration::from_secs(2)).await;

        // 第二条命令正常往返
        let http2 = tokio::spawn(crate::testutil::http_post(
            addr,
            "/devices/slow-dev/commands",
            Some(&owner),
            serde_json::json!({"method": "tap", "params": {"x": 1, "y": 1}}),
        ));
        let id2 = recv_command(&mut ws).await.id;
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id: id2,
                ok: true,
                result: Some(serde_json::json!({"ok": 1})),
                error: None,
            }),
        )
        .await;
        let (status, body) = http2.await.unwrap();
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["result"], serde_json::json!({"ok": 1}));
    }

    /// 串行化：并发两条命令，第二条须等第一条 result 后才下发。
    #[tokio::test]
    async fn command_串行下发() {
        use crate::bridge_proto::{ClientMessage, DaemonMessage, ResultMessage};
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let mut ws = app_connect(addr, &handle.owner_token, &handle.state, "serial-dev").await;
        let owner = handle.owner_token.clone();

        let h1 = tokio::spawn(crate::testutil::http_post(
            addr,
            "/devices/serial-dev/commands",
            Some(&owner),
            serde_json::json!({"method": "tap", "params": {"n": 1}}),
        ));
        let h2 = tokio::spawn(crate::testutil::http_post(
            addr,
            "/devices/serial-dev/commands",
            Some(&owner),
            serde_json::json!({"method": "tap", "params": {"n": 2}}),
        ));

        // 收到第一条；回复前不得收到第二条
        let id1 = recv_command(&mut ws).await.id;
        let early: Option<DaemonMessage> =
            crate::testutil::ws_recv(&mut ws, Duration::from_millis(200)).await;
        assert!(early.is_none(), "串行：回复前不应下发下一条: {early:?}");

        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id: id1.clone(),
                ok: true,
                result: Some(serde_json::json!(1)),
                error: None,
            }),
        )
        .await;
        let id2 = recv_command(&mut ws).await.id;
        assert_ne!(id2, id1, "id 全局递增不同");
        crate::testutil::ws_send(
            &mut ws,
            &ClientMessage::Result(ResultMessage {
                id: id2,
                ok: true,
                result: Some(serde_json::json!(2)),
                error: None,
            }),
        )
        .await;

        let (s1, b1) = h1.await.unwrap();
        let (s2, b2) = h2.await.unwrap();
        assert_eq!(s1, 200, "{b1}");
        assert_eq!(s2, 200, "{b2}");
    }

    /// GET /devices：枚举 name/online/capabilities/last_seen；离线设备在线后 online 翻转。
    #[tokio::test]
    async fn devices_枚举字段与在线状态() {
        use std::time::Duration;

        let (addr, handle, _dir) = crate::testutil::spawn_test_server().await;
        let owner = handle.owner_token.clone();
        handle.state.store.pair_device(
            &owner,
            "enum-dev",
            "14",
            &[
                crate::bridge_proto::Capability::Tap,
                crate::bridge_proto::Capability::UiTree,
            ],
        );

        // 离线：online=false，字段齐全
        let (status, body) = crate::testutil::http_get(addr, "/devices", Some(&owner)).await;
        assert_eq!(status, 200, "{body}");
        let dev = &body["devices"][0];
        assert_eq!(dev["name"], serde_json::json!("enum-dev"));
        assert_eq!(dev["online"], serde_json::json!(false));
        assert_eq!(dev["capabilities"], serde_json::json!(["tap", "uiTree"]));
        assert!(
            dev["last_seen"].as_str().unwrap().ends_with('Z'),
            "last_seen 为 RFC3339 字符串: {dev}"
        );

        // 上线：online=true
        let _ws = app_connect(addr, &owner, &handle.state, "enum-dev").await;
        let (_, body) = crate::testutil::http_get(addr, "/devices", Some(&owner)).await;
        assert_eq!(
            body["devices"][0]["online"],
            serde_json::json!(true),
            "{body}"
        );
        let _ = Duration::ZERO; // 占位避免未用导入
    }

    /// 多 owner 隔离：枚举互不可见；跨 owner 命令返回 404 同形状（不泄露存在性）。
    #[tokio::test]
    async fn devices_跨owner隔离() {
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let owner_b = "ef".repeat(32);
        let (_, gen) = crate::store::Store::open(&dir.path().join("data.json"), &[]).unwrap();
        let owner_a = gen.unwrap();
        // 二次启动注入 owner B（模拟 --owner-token 追加）
        let (store, _) = crate::store::Store::open(
            &dir.path().join("data.json"),
            std::slice::from_ref(&owner_b),
        )
        .unwrap();
        let state = crate::AppState::new(store, crate::testutil::test_config());
        let (addr, _task) = crate::start("127.0.0.1:0".parse().unwrap(), state.clone())
            .await
            .unwrap();

        // A、B 各有一台同名无关设备
        state.store.pair_device(&owner_a, "dev-a", "14", &[]);
        state.store.pair_device(&owner_b, "dev-b", "14", &[]);
        let _ws = app_connect(addr, &owner_b, &state, "dev-b").await;

        // 枚举隔离
        let (_, body_a) = crate::testutil::http_get(addr, "/devices", Some(&owner_a)).await;
        let names_a: Vec<&str> = body_a["devices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap())
            .collect();
        assert_eq!(names_a, ["dev-a"], "A 只能看到自己设备: {body_a}");
        let (_, body_b) = crate::testutil::http_get(addr, "/devices", Some(&owner_b)).await;
        assert_eq!(body_b["devices"][0]["name"], serde_json::json!("dev-b"));
        assert_eq!(body_b["devices"][0]["online"], serde_json::json!(true));

        // 跨 owner 访问在线设备 → 404 与不存在同形状
        let (status, body) = crate::testutil::http_post(
            addr,
            "/devices/dev-b/commands",
            Some(&owner_a),
            serde_json::json!({"method": "tap", "params": {}}),
        )
        .await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(
            body,
            serde_json::json!({"error": "device not found or offline"})
        );
        let _ = Arc::strong_count(&state); // state 显式保活
    }
}
