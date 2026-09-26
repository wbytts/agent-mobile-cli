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
}

async fn cmd(State(state): State<AppState>, Json(req): Json<CmdRequest>) -> Json<Value> {
    if let Some(cwd) = req.cwd {
        state.exec.set_cwd(cwd.into());
    }
    let argv: Vec<String> = std::iter::once("agent-mobile-cli".to_string())
        .chain(req.args)
        .collect();
    let parsed = <crate::cli::Cli as clap::Parser>::try_parse_from(argv);
    let out = match parsed {
        Ok(cli) => state.exec.execute(&cli.command),
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
}
