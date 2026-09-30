//! 端到端集成测试（design.md 决策 13）：进程内起服务绑随机端口，
//! tokio-tungstenite 模拟 App 走完「WS 连接 → 配对码 hello → hello_ack 拿 token →
//! HTTP POST tap 命令 → App 收 command 帧回 result → HTTP 返回成功结果」全链路。

use futures_util::{SinkExt, StreamExt};
use mobile_debug_proxy_server::bridge_proto::{
    Capability, ClientMessage, DaemonMessage, Hello, ResultMessage,
};
use mobile_debug_proxy_server::{start, store::Store, AppState, ProxyConfig};
use serde::Serialize;
use serde_json::Value;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_connect(addr: SocketAddr) -> Ws {
    let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/device"))
        .await
        .expect("WS 连接失败");
    ws
}

async fn ws_send<T: Serialize>(ws: &mut Ws, msg: &T) {
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        serde_json::to_string(msg).unwrap(),
    ))
    .await
    .unwrap();
}

async fn ws_recv<T: serde::de::DeserializeOwned>(ws: &mut Ws, wait: Duration) -> Option<T> {
    match tokio::time::timeout(wait, ws.next()).await {
        Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
            serde_json::from_str(&text).ok()
        }
        _ => None,
    }
}

fn http_post(
    addr: SocketAddr,
    path: &str,
    token: &str,
    body: Value,
) -> impl Future<Output = (u16, Value)> + Send + 'static {
    let url = format!("http://{addr}{path}");
    let token = token.to_string();
    async move {
        tokio::task::spawn_blocking(move || {
            let result = ureq::post(&url)
                .set("Authorization", &format!("Bearer {token}"))
                .send_json(body);
            match result {
                Ok(resp) => (
                    resp.status(),
                    resp.into_json::<Value>().unwrap_or(Value::Null),
                ),
                Err(ureq::Error::Status(status, resp)) => {
                    (status, resp.into_json::<Value>().unwrap_or(Value::Null))
                }
                Err(e) => panic!("HTTP 请求失败: {e}"),
            }
        })
        .await
        .unwrap()
    }
}

/// 全链路：配对 → 绑定 → tap 命令往返。
#[tokio::test]
async fn 端到端_配对绑定_tap命令往返() {
    // 进程内起服务（随机端口），数据文件放临时目录
    let dir = tempfile::tempdir().unwrap();
    let (store, generated) = Store::open(&dir.path().join("proxy-data.json"), &[]).unwrap();
    let owner_token = generated.expect("首启生成 owner token");
    let state = AppState::new(store, ProxyConfig::default());
    let (addr, _task) = start("127.0.0.1:0".parse().unwrap(), state.clone())
        .await
        .unwrap();

    // 1. CLI 侧签发一次性配对码
    let (status, body) =
        http_post(addr, "/pairing-codes", &owner_token, serde_json::json!({})).await;
    assert_eq!(status, 200, "{body}");
    let code = body["pairing_code"].as_str().unwrap().to_string();

    // 2. App 出站 WS 绑定 + 配对码 hello
    let mut ws = ws_connect(addr).await;
    ws_send(
        &mut ws,
        &ClientMessage::Hello(Hello {
            pairing_code: Some(code),
            token: None,
            device_name: "e2e-dev".to_string(),
            android_version: "14".to_string(),
            capabilities: vec![Capability::Tap],
        }),
    )
    .await;

    // 3. hello_ack：首次配对下发设备 token
    let ack: DaemonMessage = ws_recv(&mut ws, Duration::from_secs(5))
        .await
        .expect("应收到 hello_ack");
    let DaemonMessage::HelloAck(ack) = ack else {
        panic!("预期 hello_ack: {ack:?}")
    };
    assert!(ack.ok, "配对应通过: {:?}", ack.error);
    let device_token = ack.token.expect("首次配对下发设备 token");
    assert_eq!(device_token.len(), 64);

    // 4. CLI 侧枚举：设备在线
    let state2 = state.clone();
    let devices = state2.store.devices_of(&owner_token);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].name, "e2e-dev");
    assert!(state2.relay.is_online(&owner_token, "e2e-dev"));

    // 5. HTTP POST tap 命令 → App 收 command 帧 → 回 result → HTTP 返回成功
    let http = tokio::spawn(http_post(
        addr,
        "/devices/e2e-dev/commands",
        &owner_token,
        serde_json::json!({"method": "tap", "params": {"x": 540, "y": 1170}}),
    ));
    let frame: DaemonMessage = ws_recv(&mut ws, Duration::from_secs(5))
        .await
        .expect("App 应收到 command 帧");
    let DaemonMessage::Command(cmd) = frame else {
        panic!("预期 command 帧: {frame:?}")
    };
    assert_eq!(cmd.method, "tap");
    assert_eq!(cmd.params, serde_json::json!({"x": 540, "y": 1170}));
    assert!(cmd.id.starts_with("cmd-"));

    ws_send(
        &mut ws,
        &ClientMessage::Result(ResultMessage {
            id: cmd.id.clone(),
            ok: true,
            result: Some(serde_json::json!({"tapped": [540, 1170]})),
            error: None,
        }),
    )
    .await;

    let (status, body) = http.await.unwrap();
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["ok"], serde_json::json!(true));
    assert_eq!(body["result"], serde_json::json!({"tapped": [540, 1170]}));
}
