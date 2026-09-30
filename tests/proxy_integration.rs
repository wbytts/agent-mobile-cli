//! 组 4.2 代理通路集成测试：进程内起真实 mobile-debug-proxy-server（path dev-dependency），
//! 验证 ProxyBackend 设备枚举、命令路由与 DaemonState 的合并枚举。
//! 运行方式：cargo test --test proxy_integration（无需真实设备，WS 客户端模拟 App）。
//!
//! 通过 #[path] 引入库层模块树（crate:: 路径与二进制目标一致），与 control_integration.rs 同模式。

#[path = "../src/adb/mod.rs"]
mod adb;
#[allow(dead_code)]
#[path = "../src/backend/mod.rs"]
mod backend;
#[allow(dead_code)]
#[path = "../src/bridge_proto.rs"]
mod bridge_proto;
#[allow(dead_code)]
#[path = "../src/cli.rs"]
mod cli;
#[allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[path = "daemon/mod.rs"]
mod daemon;
#[allow(dead_code)]
#[path = "../src/exec.rs"]
mod exec;
#[allow(dead_code)]
#[path = "../src/output.rs"]
mod output;
#[path = "../src/ui.rs"]
mod ui;

use backend::proxy::{ProxyBackend, ProxyClient};
use backend::{Backend, TapTarget};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::Arc;

/// 进程内起代理服务：临时目录数据文件 + 随机端口；返回 (base_url, owner_token, 临时目录守卫)。
async fn spawn_proxy() -> (String, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (store, generated) =
        mobile_debug_proxy_server::store::Store::open(&dir.path().join("proxy-data.json"), &[])
            .unwrap();
    let owner_token = generated.unwrap();
    let state = mobile_debug_proxy_server::AppState::new(
        store,
        mobile_debug_proxy_server::ProxyConfig::default(),
    );
    let (addr, _task) = mobile_debug_proxy_server::start("127.0.0.1:0".parse().unwrap(), state)
        .await
        .unwrap();
    (format!("http://{addr}"), owner_token, dir)
}

/// WS 客户端模拟 App：配对码 hello → 返回 (ws 流, 设备 token)。
async fn app_connect(
    base: &str,
    pairing_code: &str,
) -> (
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
) {
    let ws_url = format!("{}/ws/device", base.replacen("http", "ws", 1));
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let hello = json!({
        "type": "hello",
        "pairing_code": pairing_code,
        "device_name": "MuMu",
        "android_version": "12",
        "capabilities": ["tap", "swipe", "input", "key", "uiTree", "screenshot", "apps", "launch", "script"],
    });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        hello.to_string(),
    ))
    .await
    .unwrap();
    let msg = ws.next().await.unwrap().unwrap();
    let ack: Value = serde_json::from_str(&msg.into_text().unwrap()).unwrap();
    assert_eq!(ack["type"], json!("hello_ack"));
    assert_eq!(ack["ok"], json!(true));
    let token = ack["token"]
        .as_str()
        .expect("首次配对应下发 token")
        .to_string();
    (ws, token)
}

// ProxyBackend 是同步阻塞 HTTP：多线程运行时，ureq 调用放 blocking 池
#[tokio::test(flavor = "multi_thread")]
async fn proxy_backend_devices_and_tap_against_real_server() {
    let (base, token, _dir) = spawn_proxy().await;
    let client = ProxyClient::new(&base, &token);
    let backend = ProxyBackend::from_client(ProxyClient::new(&base, &token));

    // 绑定前枚举为空
    let devices = tokio::task::spawn_blocking({
        let backend = ProxyBackend::from_client(ProxyClient::new(&base, &token));
        move || backend.devices().unwrap()
    })
    .await
    .unwrap();
    assert!(devices.is_empty());

    // App 经配对码绑定
    let code = tokio::task::spawn_blocking(move || client.create_pairing_code().unwrap())
        .await
        .unwrap();
    let (mut ws, _device_token) = app_connect(&base, &code).await;

    // 枚举出现 proxy:MuMu 在线记录
    let records = tokio::task::spawn_blocking(move || backend.devices().unwrap())
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "proxy:MuMu");
    assert_eq!(records[0].kind, backend::BackendKind::Proxy);
    assert_eq!(records[0].connection, backend::ConnectionKind::Proxy);
    assert_eq!(records[0].state, backend::DeviceState::Online);

    // tap 命令经代理路由到 App：App 收 command 帧并回 result
    let tap = tokio::task::spawn_blocking({
        let backend = ProxyBackend::from_client(ProxyClient::new(&base, &token));
        move || backend.tap("proxy:MuMu", TapTarget::Coord(540, 1170))
    });
    let msg = ws.next().await.unwrap().unwrap();
    let cmd: Value = serde_json::from_str(&msg.into_text().unwrap()).unwrap();
    assert_eq!(cmd["type"], json!("command"));
    assert_eq!(cmd["method"], json!("tap"));
    assert_eq!(cmd["params"], json!({ "x": 540, "y": 1170 }));
    let result = json!({ "type": "result", "id": cmd["id"], "ok": true, "result": { "tapped": [540, 1170] } });
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        result.to_string(),
    ))
    .await
    .unwrap();
    tap.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_state_devices_merges_proxy_records() {
    let (base, token, dir) = spawn_proxy().await;
    let client = ProxyClient::new(&base, &token);
    let code = tokio::task::spawn_blocking({
        let c = ProxyClient::new(&base, &token);
        move || c.create_pairing_code().unwrap()
    })
    .await
    .unwrap();
    let (_ws, _t) = app_connect(&base, &code).await;
    drop(client);

    let cfg = config::Config {
        proxy: Some(config::ProxyConfig {
            url: base.clone(),
            token,
        }),
        ..config::Config::default()
    };
    let state = exec::DaemonState::new_in(cfg, dir.path());
    let out =
        tokio::task::spawn_blocking(move || state.execute(&cli::Command::Devices, dir.path()))
            .await
            .unwrap();
    assert!(out.ok, "devices 命令应成功: {:?}", out.error);
    let devices = out.result.unwrap()["devices"].as_array().unwrap().clone();
    let proxy_dev = devices
        .iter()
        .find(|d| d["id"] == json!("proxy:MuMu"))
        .expect("合并枚举应包含代理设备");
    assert_eq!(proxy_dev["kind"], json!("proxy"));
    assert_eq!(proxy_dev["connection"], json!("proxy"));
}

#[tokio::test(flavor = "multi_thread")]
async fn daemon_state_proxy_auth_error_surfaces_explicitly() {
    let (base, _token, dir) = spawn_proxy().await;
    let cfg = config::Config {
        proxy: Some(config::ProxyConfig {
            url: base,
            token: "wrong-token".into(),
        }),
        ..config::Config::default()
    };
    let state = exec::DaemonState::new_in(cfg, dir.path());
    let out = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let dir_path = dir.path().to_path_buf();
        move || state.execute(&cli::Command::Devices, &dir_path)
    })
    .await
    .unwrap();
    // 降级列出其余设备，但 proxy_error 字段显式可见
    let result = out.result.expect("降级时应仍返回设备列表");
    let msg = result["proxy_error"].as_str().expect("认证失败须显式暴露");
    assert!(msg.contains("认证失败"), "proxy_error: {msg}");

    // 显式指定代理设备时传播认证错误而非降级
    let out = tokio::task::spawn_blocking(move || {
        state.execute(
            &cli::Command::Tap {
                device: Some("proxy:MuMu".into()),
                target: "100".into(),
                y: Some(200),
            },
            dir.path(),
        )
    })
    .await
    .unwrap();
    assert!(!out.ok);
    assert_eq!(out.error.unwrap().code, output::ErrorCode::ProxyAuth);
}
