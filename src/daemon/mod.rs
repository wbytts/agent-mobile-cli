//! 常驻 daemon：启动锁 + HTTP API + 桥接 WS 双端口（design.md 架构节、决策 1/2/8）。
//!
//! - [`run`]：前台运行 daemon（`daemon` 子命令），ctrl-c 或 `POST /shutdown` 时清理锁退出；
//! - [`ensure_daemon`]：CLI 短进程侧自动拉起 daemon（健康检查 → 清理过期锁 → spawn → 轮询）；
//! - [`status`] / [`stop`]：`daemon-status` / `daemon-stop` 的实现。

pub mod http;
pub mod lock;
pub mod ws;

use crate::backend::BResult;
use crate::config::Config;
use crate::output::ErrorBody;
use lock::{DaemonLock, LockOutcome};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// 前台运行 daemon：获取启动锁，同时监听 HTTP 与 WS 端口；ctrl-c 或 /shutdown 时清理锁退出。
pub fn run(config: &Config) -> BResult<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| ErrorBody::io_error(format!("创建 tokio 运行时失败: {e}")))?;
    runtime.block_on(run_async(config))
}

async fn run_async(config: &Config) -> BResult<()> {
    let lock = match DaemonLock::acquire(&Config::dir(), config.http_port)? {
        LockOutcome::Acquired(lock) => lock,
        LockOutcome::AlreadyRunning { pid, port } => {
            return Err(ErrorBody::io_error(format!(
                "daemon 已在运行（pid={pid}，http 端口={port}），无需重复启动"
            )));
        }
    };
    let http_listener = tokio::net::TcpListener::bind(("127.0.0.1", config.http_port))
        .await
        .map_err(|e| {
            ErrorBody::io_error(format!("绑定 HTTP 端口 {} 失败: {e}", config.http_port))
        })?;
    let ws_listener = tokio::net::TcpListener::bind(("127.0.0.1", config.bridge_port))
        .await
        .map_err(|e| {
            ErrorBody::io_error(format!("绑定桥接 WS 端口 {} 失败: {e}", config.bridge_port))
        })?;
    // 启动行写入 stderr：后台运行时落入 daemon.log，便于排查
    eprintln!(
        "daemon 已启动：pid={} http=127.0.0.1:{} ws=127.0.0.1:{}",
        lock.info.pid, config.http_port, config.bridge_port
    );
    let shutdown = Arc::new(Notify::new());
    let exec_state = crate::exec::DaemonState::new(config.clone());
    tokio::select! {
        result = http::serve(http_listener, Arc::clone(&shutdown), exec_state) => {
            result.map_err(|e| ErrorBody::io_error(format!("HTTP 服务异常退出: {e}")))?;
        }
        result = ws::serve(ws_listener, Arc::clone(&shutdown)) => {
            result.map_err(|e| ErrorBody::io_error(format!("WS 服务异常退出: {e}")))?;
        }
        _ = tokio::signal::ctrl_c() => {}
    }
    // 通知另一个服务退出；锁文件随 DaemonLock Drop 清理
    shutdown.notify_waiters();
    drop(lock);
    Ok(())
}

/// 确保 daemon 在运行：健康检查失败时清理过期锁并后台拉起，轮询 /health 最多 5 秒。
pub fn ensure_daemon(config: &Config) -> BResult<()> {
    if health_check(config.http_port) {
        return Ok(());
    }
    lock::remove_stale_lock(&Config::dir())?;
    spawn_daemon()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if health_check(config.http_port) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(ErrorBody::timeout(format!(
        "daemon 启动超时（5 秒内健康检查未通过），日志见 {}",
        Config::dir().join("daemon.log").display()
    )))
}

/// 后台拉起当前二进制的 `daemon` 子命令，输出重定向到 <配置目录>/daemon.log。
fn spawn_daemon() -> BResult<()> {
    let exe = std::env::current_exe()
        .map_err(|e| ErrorBody::io_error(format!("定位当前可执行文件失败: {e}")))?;
    let dir = Config::dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| ErrorBody::io_error(format!("创建配置目录失败 {}: {e}", dir.display())))?;
    let log_path = dir.join("daemon.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| {
            ErrorBody::io_error(format!("打开 daemon 日志失败 {}: {e}", log_path.display()))
        })?;
    let log_err = log
        .try_clone()
        .map_err(|e| ErrorBody::io_error(format!("复制日志句柄失败: {e}")))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 独立进程组：CLI 收到 ctrl-c 时不会连带终止 daemon
        cmd.process_group(0);
    }
    cmd.spawn()
        .map_err(|e| ErrorBody::io_error(format!("拉起 daemon 子进程失败: {e}")))?;
    Ok(())
}

/// `daemon-status`：锁文件 + /health 汇总运行状态。
pub fn status(config: &Config) -> BResult<Value> {
    let info = lock::read_lock(&Config::dir());
    let healthy = health_check(config.http_port);
    match (info, healthy) {
        (Some(info), true) => {
            let health = http_request(
                config.http_port,
                "GET",
                "/health",
                None,
                Duration::from_secs(2),
            )
            .ok()
            .and_then(|body| serde_json::from_str::<Value>(&body).ok())
            .unwrap_or_else(|| json!({}));
            Ok(json!({
                "running": true,
                "pid": info.pid,
                "http_port": config.http_port,
                "bridge_port": config.bridge_port,
                "started_at_epoch_secs": info.started_at_epoch_secs,
                "version": health.get("version").cloned().unwrap_or(Value::Null),
                "uptime_secs": health.get("uptime_secs").cloned().unwrap_or(Value::Null),
            }))
        }
        _ => Ok(json!({
            "running": false,
            "http_port": config.http_port,
            "bridge_port": config.bridge_port,
        })),
    }
}

/// `daemon-stop`：POST /shutdown；无 daemon 运行时报错。
pub fn stop(config: &Config) -> BResult<()> {
    if !health_check(config.http_port) {
        return Err(ErrorBody::io_error("daemon 未运行"));
    }
    http_request(
        config.http_port,
        "POST",
        "/shutdown",
        Some("{}"),
        Duration::from_secs(2),
    )
    .map_err(|e| ErrorBody::io_error(format!("请求 daemon 退出失败: {e}")))?;
    Ok(())
}

/// GET /health 是否通过（daemon 存活判定）。
pub fn health_check(port: u16) -> bool {
    http_request(port, "GET", "/health", None, Duration::from_millis(500))
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|v| v.get("ok")?.as_bool())
        .unwrap_or(false)
}

/// CLI 侧把原始命令参数转发给 daemon 执行；返回 daemon 的结构化输出。
pub fn post_cmd(
    port: u16,
    args: &[String],
    cwd: &std::path::Path,
) -> BResult<crate::output::Output> {
    let body = json!({ "args": args, "cwd": cwd }).to_string();
    let raw = http_request(port, "POST", "/cmd", Some(&body), Duration::from_secs(120))
        .map_err(|e| ErrorBody::io_error(format!("转发命令到 daemon 失败: {e}")))?;
    serde_json::from_str(&raw)
        .map_err(|e| ErrorBody::io_error(format!("daemon 响应解析失败: {e}（原始响应: {raw}）")))
}

/// 极简同步 HTTP/1.1 客户端（CLI 短进程侧使用）；返回响应 body，非 2xx 视为错误。
fn http_request(
    port: u16,
    method: &str,
    path: &str,
    body: Option<&str>,
    timeout: Duration,
) -> std::io::Result<String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    if let Some(body) = body {
        request.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    if let Some(body) = body {
        request.push_str(body);
    }
    stream.write_all(request.as_bytes())?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let status_line = raw.lines().next().unwrap_or("");
    let status_ok = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .map(|code| (200..300).contains(&code))
        .unwrap_or(false);
    if !status_ok {
        return Err(std::io::Error::other(format!(
            "HTTP 响应非 2xx: {status_line}"
        )));
    }
    Ok(raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}
