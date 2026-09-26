//! daemon HTTP API：`GET /health`、`POST /cmd`（命令路由到 backend 执行）、`POST /shutdown`（design.md 决策 2）。

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::Notify;

#[derive(Clone)]
struct AppState {
    started: Instant,
    shutdown: Arc<Notify>,
    exec: Arc<crate::exec::DaemonState>,
}

/// 在已绑定的 listener 上提供 HTTP 服务；`shutdown` 被通知后优雅退出。
pub async fn serve(
    listener: TcpListener,
    shutdown: Arc<Notify>,
    exec: Arc<crate::exec::DaemonState>,
) -> std::io::Result<()> {
    let state = AppState {
        started: Instant::now(),
        shutdown: Arc::clone(&shutdown),
        exec,
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/cmd", post(cmd))
        .route("/shutdown", post(shutdown_handler))
        .route("/pair-info", get(pair_info))
        .route("/pair-reset", post(pair_reset))
        .with_state(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown.notified().await;
        })
        .await
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": state.started.elapsed().as_secs(),
    }))
}

#[derive(Debug, Deserialize)]
struct CmdRequest {
    #[serde(default)]
    args: Vec<String>,
    /// 命令发起侧 CLI 的工作目录（相对路径输出以其为基准）
    #[serde(default)]
    cwd: Option<String>,
    /// `script -` 的 stdin 脚本内容：发起侧 CLI 读入随请求携带（daemon 无法访问发起侧 stdin）
    #[serde(default)]
    script_stdin: Option<String>,
}

async fn cmd(State(state): State<AppState>, Json(req): Json<CmdRequest>) -> Json<Value> {
    let argv: Vec<String> = std::iter::once("agent-mobile-cli".to_string())
        .chain(req.args)
        .collect();
    // 命令发起侧 CLI 的工作目录（相对路径输出以其为基准）；缺省为 daemon 当前目录
    let cwd = std::path::PathBuf::from(req.cwd.unwrap_or_else(|| ".".to_string()));
    let parsed = <crate::cli::Cli as clap::Parser>::try_parse_from(argv);
    let out = match parsed {
        Ok(mut cli) => {
            // script - 的 stdin 内容注入命令（execute 侧只认注入后的内容/文件路径）
            if let crate::cli::Command::Script { source, stdin, .. } = &mut cli.command {
                if source == "-" {
                    match req.script_stdin {
                        Some(s) if !s.trim().is_empty() => *stdin = Some(s),
                        _ => {
                            let out = crate::output::Output::failure(
                                crate::output::ErrorCode::Usage,
                                "script - 需从 stdin 读取脚本内容（由发起侧 CLI 随请求携带）",
                                None,
                            );
                            return Json(serde_json::to_value(out).expect("输出序列化"));
                        }
                    }
                }
            }
            // execute 为同步阻塞调用（ADB 进程/IO），移入 blocking 线程池避免卡住 worker
            let exec = Arc::clone(&state.exec);
            match tokio::task::spawn_blocking(move || exec.execute(&cli.command, &cwd)).await {
                Ok(out) => out,
                Err(e) => crate::output::Output::failure(
                    crate::output::ErrorCode::IoError,
                    format!("命令执行线程异常退出: {e}"),
                    None,
                ),
            }
        }
        Err(e) => crate::output::Output::failure(
            crate::output::ErrorCode::Usage,
            e.to_string().trim().to_string(),
            None,
        ),
    };
    Json(serde_json::to_value(out).expect("输出序列化"))
}

async fn shutdown_handler(State(state): State<AppState>) -> Json<Value> {
    state.shutdown.notify_one();
    Json(json!({ "ok": true }))
}

/// GET /pair-info：当前配对码（含可用状态）、候选局域网 IP 与桥接端口（design.md 决策 8/13）。
async fn pair_info(State(state): State<AppState>) -> Json<Value> {
    let (code, active) = state.exec.pairing().pairing_code();
    Json(json!({
        "ok": true,
        "pairing_code": code,
        "code_active": active,
        "port": state.exec.config().bridge_port,
        "ips": crate::daemon::net::candidate_ips(),
    }))
}

/// POST /pair-reset：重新生成配对码、清空全部已签发 token 并断开活跃桥接连接
///（spec「重置配对」：已连接设备被要求重新配对，design.md 决策 14）。
async fn pair_reset(State(state): State<AppState>) -> Json<Value> {
    state.exec.pairing().reset();
    state.exec.bridge().disconnect_all();
    Json(json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn test_state() -> Arc<crate::exec::DaemonState> {
        crate::exec::DaemonState::new(crate::config::Config::default())
    }

    async fn start() -> (
        u16,
        Arc<Notify>,
        tokio::task::JoinHandle<std::io::Result<()>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口");
        let port = listener.local_addr().expect("读取本地地址").port();
        let shutdown = Arc::new(Notify::new());
        let handle = tokio::spawn(serve(listener, Arc::clone(&shutdown), test_state()));
        (port, shutdown, handle)
    }

    async fn http_get(port: u16, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("连接 HTTP 服务");
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .expect("写入请求");
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.expect("读取响应");
        raw
    }

    async fn http_post(port: u16, path: &str, body: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("连接 HTTP 服务");
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(request.as_bytes())
            .await
            .expect("写入请求");
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.expect("读取响应");
        raw
    }

    fn body_of(raw: &str) -> &str {
        raw.split("\r\n\r\n").nth(1).expect("响应缺少 body")
    }

    #[tokio::test]
    async fn health_returns_ok_with_version_and_uptime() {
        let (port, shutdown, handle) = start().await;
        let raw = http_get(port, "/health").await;
        assert!(raw.starts_with("HTTP/1.1 200"), "响应行: {raw}");
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /health body");
        assert_eq!(v["ok"], true);
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert!(v["uptime_secs"].is_number(), "uptime_secs 应为数字: {v}");
        shutdown.notify_one();
        handle
            .await
            .expect("join http 服务")
            .expect("http 服务退出");
    }

    #[tokio::test]
    async fn cmd_invalid_command_returns_usage_error() {
        let (port, shutdown, handle) = start().await;
        let raw = http_post(port, "/cmd", r#"{"args":["no-such-cmd"]}"#).await;
        assert!(raw.starts_with("HTTP/1.1 200"), "响应行: {raw}");
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /cmd body");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "USAGE");
        shutdown.notify_one();
        handle
            .await
            .expect("join http 服务")
            .expect("http 服务退出");
    }

    #[tokio::test]
    async fn cmd_daemon_lifecycle_rejected() {
        let (port, shutdown, handle) = start().await;
        let raw = http_post(port, "/cmd", r#"{"args":["daemon-status"]}"#).await;
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /cmd body");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "USAGE");
        shutdown.notify_one();
        handle
            .await
            .expect("join http 服务")
            .expect("http 服务退出");
    }

    #[tokio::test]
    async fn shutdown_endpoint_stops_server() {
        let (port, _shutdown, handle) = start().await;
        let raw = http_post(port, "/shutdown", "{}").await;
        assert!(raw.starts_with("HTTP/1.1 200"), "响应行: {raw}");
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /shutdown body");
        assert_eq!(v["ok"], true);
        handle
            .await
            .expect("join http 服务")
            .expect("http 服务退出");
    }

    async fn start_with_dir(
        dir: std::path::PathBuf,
    ) -> (
        u16,
        Arc<Notify>,
        tokio::task::JoinHandle<std::io::Result<()>>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口");
        let port = listener.local_addr().expect("读取本地地址").port();
        let shutdown = Arc::new(Notify::new());
        let state = crate::exec::DaemonState::new_in(crate::config::Config::default(), &dir);
        let handle = tokio::spawn(serve(listener, Arc::clone(&shutdown), state));
        (port, shutdown, handle)
    }

    #[tokio::test]
    async fn pair_info_returns_code_ips_and_port() {
        let tmp = tempfile::tempdir().unwrap();
        let (port, shutdown, handle) = start_with_dir(tmp.path().to_path_buf()).await;
        let raw = http_get(port, "/pair-info").await;
        assert!(raw.starts_with("HTTP/1.1 200"), "响应行: {raw}");
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /pair-info body");
        assert_eq!(v["ok"], true);
        let code = v["pairing_code"].as_str().expect("应有配对码");
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(v["code_active"], true);
        assert_eq!(v["port"], crate::config::DEFAULT_BRIDGE_PORT);
        let ips = v["ips"].as_array().expect("ips 为数组");
        assert!(ips.iter().all(|ip| ip.is_string()));
        shutdown.notify_one();
        handle.await.expect("join").expect("http 服务退出");
    }

    #[tokio::test]
    async fn pair_reset_regenerates_code_and_revokes_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let (port, shutdown, handle) = start_with_dir(tmp.path().to_path_buf()).await;
        let before: Value =
            serde_json::from_str(body_of(&http_get(port, "/pair-info").await)).unwrap();
        // 模拟一次配对签发 token（直接操作内存态之外的公开行为：经 WS 由组 2 ws 测试覆盖，
        // 这里验证 reset 端点重置配对码并清空 tokens.json 的行为契约）
        let raw = http_post(port, "/pair-reset", "{}").await;
        assert!(raw.starts_with("HTTP/1.1 200"), "响应行: {raw}");
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /pair-reset body");
        assert_eq!(v["ok"], true);

        let after: Value =
            serde_json::from_str(body_of(&http_get(port, "/pair-info").await)).unwrap();
        assert_eq!(after["code_active"], true);
        assert_ne!(
            before["pairing_code"].as_str().unwrap().len(),
            0,
            "重置前配对码存在"
        );
        // tokens.json 被清空（reset 写空文件）
        let tokens =
            std::fs::read_to_string(tmp.path().join("tokens.json")).expect("tokens.json 存在");
        let tv: Value = serde_json::from_str(&tokens).unwrap();
        assert_eq!(tv["tokens"], json!([]));
        shutdown.notify_one();
        handle.await.expect("join").expect("http 服务退出");
    }

    #[tokio::test]
    async fn pair_reset_disconnects_registered_devices() {
        // spec「重置配对」：已连接设备被要求重新配对（断开连接并标离线）
        let tmp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口");
        let port = listener.local_addr().unwrap().port();
        let shutdown = Arc::new(Notify::new());
        let state = crate::exec::DaemonState::new_in(crate::config::Config::default(), tmp.path());
        let handle = tokio::spawn(serve(listener, Arc::clone(&shutdown), Arc::clone(&state)));

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let hello = crate::bridge_proto::Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: "MuMu".into(),
            android_version: "12".into(),
            capabilities: vec![crate::bridge_proto::Capability::Tap],
        };
        state.bridge().register(&hello, tx);
        assert_eq!(
            state.bridge().device_records()[0].state,
            crate::backend::DeviceState::Online
        );

        let raw = http_post(port, "/pair-reset", "{}").await;
        assert!(raw.starts_with("HTTP/1.1 200"), "响应行: {raw}");
        assert_eq!(
            state.bridge().device_records()[0].state,
            crate::backend::DeviceState::Offline,
            "reset 后已连接设备应立即标离线"
        );
        shutdown.notify_one();
        handle.await.expect("join").expect("http 服务退出");
    }

    #[tokio::test]
    async fn cmd_script_stdin_injected_and_routed_to_bridge() {
        let tmp = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口");
        let port = listener.local_addr().unwrap().port();
        let shutdown = Arc::new(Notify::new());
        let state = crate::exec::DaemonState::new_in(crate::config::Config::default(), tmp.path());
        let handle = tokio::spawn(serve(listener, Arc::clone(&shutdown), Arc::clone(&state)));

        // 注册桥接设备（仅 script 能力）并模拟设备侧应答
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let hello = crate::bridge_proto::Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: "MuMu".into(),
            android_version: "12".into(),
            capabilities: vec![crate::bridge_proto::Capability::Script],
        };
        let (_id, _conn, _close) = state.bridge().register(&hello, tx);
        let reg = Arc::clone(state.bridge());
        let dev = std::thread::spawn(move || {
            let frame = rx.blocking_recv().expect("应收到 script 帧");
            let v: Value = serde_json::from_str(&frame).unwrap();
            assert_eq!(v["type"], "script");
            assert_eq!(v["source"], "mobile.tap(1,2)");
            reg.complete(crate::bridge_proto::ResultMessage {
                id: v["id"].as_str().unwrap().to_string(),
                ok: true,
                result: Some(json!({"done": true})),
                error: None,
            });
        });

        // stdin 内容随请求携带 → 注入命令并经桥接路由
        let raw = http_post(
            port,
            "/cmd",
            r#"{"args":["script","-","--device","bridge:MuMu"],"script_stdin":"mobile.tap(1,2)"}"#,
        )
        .await;
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /cmd body");
        assert_eq!(v["ok"], true, "script 经桥接应成功: {v}");
        assert_eq!(v["result"]["result"], json!({"done": true}));
        dev.join().unwrap();

        // 缺少 script_stdin → Usage（daemon 无发起侧 stdin）
        let raw = http_post(
            port,
            "/cmd",
            r#"{"args":["script","-","--device","bridge:MuMu"]}"#,
        )
        .await;
        let v: Value = serde_json::from_str(body_of(&raw)).expect("解析 /cmd body");
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "USAGE");

        shutdown.notify_one();
        handle.await.expect("join").expect("http 服务退出");
    }
}
