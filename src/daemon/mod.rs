//! 常驻 daemon：启动锁 + HTTP API + 桥接 WS 双端口（design.md 架构节、决策 1/2/8）。
//!
//! - [`run`]：前台运行 daemon（`daemon` 子命令），ctrl-c 或 `POST /shutdown` 时清理锁退出；
//! - [`ensure_daemon`]：CLI 短进程侧自动拉起 daemon（健康检查 → 清理过期锁 → spawn → 轮询）；
//! - [`status`] / [`stop`]：`daemon-status` / `daemon-stop` 的实现。

pub mod http;
pub mod lock;
pub mod net;
pub mod pair;
pub mod registry;
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

/// HTTP 管理端点只绑 loopback（/pair-info 含配对码，绝不上 LAN）。
const HTTP_BIND_ADDR: &str = "127.0.0.1";
/// 桥接 WS 绑全部接口：design.md 决策 8 的 LAN 威胁模型要求真机直连可达，
/// 暴露面由一次性配对码 + token 认证兜底。
const WS_BIND_ADDR: &str = "0.0.0.0";
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
    let http_listener = tokio::net::TcpListener::bind((HTTP_BIND_ADDR, config.http_port))
        .await
        .map_err(|e| {
            ErrorBody::io_error(format!("绑定 HTTP 端口 {} 失败: {e}", config.http_port))
        })?;
    let ws_listener = tokio::net::TcpListener::bind((WS_BIND_ADDR, config.bridge_port))
        .await
        .map_err(|e| {
            ErrorBody::io_error(format!("绑定桥接 WS 端口 {} 失败: {e}", config.bridge_port))
        })?;
    // 启动行写入 stderr：后台运行时落入 daemon.log，便于排查
    eprintln!(
        "daemon 已启动：pid={} http={HTTP_BIND_ADDR}:{} ws={WS_BIND_ADDR}:{}",
        lock.info.pid, config.http_port, config.bridge_port
    );
    let shutdown = Arc::new(Notify::new());
    let exec_state = crate::exec::DaemonState::new(config.clone());
    let registry = Arc::clone(exec_state.bridge());
    let pairing = Arc::clone(exec_state.pairing());
    tokio::select! {
        result = http::serve(http_listener, Arc::clone(&shutdown), exec_state) => {
            result.map_err(|e| ErrorBody::io_error(format!("HTTP 服务异常退出: {e}")))?;
        }
        result = ws::serve(ws_listener, Arc::clone(&shutdown), registry, pairing) => {
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
    ensure_daemon_in(config, &Config::dir())
}

/// [`ensure_daemon`] 的实现，锁目录可注入（测试用临时目录，避免污染用户配置目录）。
fn ensure_daemon_in(config: &Config, dir: &std::path::Path) -> BResult<()> {
    if health_check(config.http_port) {
        // 版本守卫：daemon 版本与当前二进制不一致（升级后残留旧 daemon）时自动接管
        if daemon_version(config.http_port).as_deref() == Some(env!("CARGO_PKG_VERSION")) {
            return Ok(());
        }
        let _ = stop(config);
        wait_daemon_gone_in(dir, Duration::from_secs(10))?;
    }
    // 配置端口不通但锁内 daemon 在其他端口健康存活：端口配置漂移，
    // 直接给出指引，而不是再拉一个 daemon 造成端口冲突
    if let Some(info) = lock::read_lock(dir) {
        if info.port != config.http_port
            && lock::pid_alive(info.pid)
            && lock::port_listening(info.port)
        {
            return Err(ErrorBody::io_error(format!(
                "daemon 已在端口 {} 运行（pid {}），与配置端口 {} 不一致；请将配置改回 {} 或先执行 daemon-stop",
                info.port, info.pid, config.http_port, info.port
            )));
        }
    }
    lock::remove_stale_lock(dir)?;
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
        dir.join("daemon.log").display()
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
    status_in(config, &Config::dir())
}

/// [`status`] 的实现，锁目录可注入（测试用临时目录）。
fn status_in(config: &Config, dir: &std::path::Path) -> BResult<Value> {
    let info = lock::read_lock(dir);
    // 健康检查端口：优先锁内端口（配置端口可能被改），锁缺失时回落配置端口
    let port = info.as_ref().map(|i| i.port).unwrap_or(config.http_port);
    let healthy = health_check(port);
    match (info, healthy) {
        (Some(info), true) => {
            let health = http_request(port, "GET", "/health", None, Duration::from_secs(2))
                .ok()
                .and_then(|body| serde_json::from_str::<Value>(&body).ok())
                .unwrap_or_else(|| json!({}));
            Ok(json!({
                "running": true,
                "pid": info.pid,
                "http_port": port,
                "bridge_port": config.bridge_port,
                "started_at_epoch_secs": info.started_at_epoch_secs,
                "version": health.get("version").cloned().unwrap_or(Value::Null),
                "uptime_secs": health.get("uptime_secs").cloned().unwrap_or(Value::Null),
            }))
        }
        // 锁缺失但端口健康（例如锁被手工删除）：报运行中，pid/started_at 无从得知置 null
        (None, true) => {
            let health = http_request(port, "GET", "/health", None, Duration::from_secs(2))
                .ok()
                .and_then(|body| serde_json::from_str::<Value>(&body).ok())
                .unwrap_or_else(|| json!({}));
            Ok(json!({
                "running": true,
                "pid": Value::Null,
                "http_port": port,
                "bridge_port": config.bridge_port,
                "started_at_epoch_secs": Value::Null,
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
    stop_in(config, &Config::dir())
}

/// [`stop`] 的实现，锁目录可注入（测试用临时目录）。
fn stop_in(config: &Config, dir: &std::path::Path) -> BResult<()> {
    let port = if health_check(config.http_port) {
        config.http_port
    } else {
        // 配置端口不通：若锁内端口在监听（配置端口被改），按锁内端口关停
        match lock::read_lock(dir) {
            Some(info) if lock::port_listening(info.port) => info.port,
            _ => return Err(ErrorBody::io_error("daemon 未运行")),
        }
    };
    http_request(
        port,
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

/// GET /health 返回的 daemon 版本（版本守卫：升级后残留旧 daemon 时自动接管）。
fn daemon_version(port: u16) -> Option<String> {
    let body = http_request(port, "GET", "/health", None, Duration::from_millis(500)).ok()?;
    let v: Value = serde_json::from_str(&body).ok()?;
    Some(v.get("version")?.as_str()?.to_owned())
}

/// 等待旧 daemon 退出：以「锁文件消失或锁内 pid 退出」为准（每 100ms 轮询）。
/// 健康检查在 /cmd 有在飞长命令时会持续通过，不能作为退出判据。
pub fn wait_daemon_gone_in(dir: &std::path::Path, timeout: Duration) -> BResult<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let exited = match lock::read_lock(dir) {
            None => true,
            Some(info) => !lock::pid_alive(info.pid),
        };
        if exited {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(ErrorBody::timeout(format!(
                "旧 daemon 未在 {} 秒内退出（可能有长命令在飞，可稍后重试 daemon-status 确认）",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// CLI 侧把原始命令参数转发给 daemon 执行；返回 daemon 的结构化输出。
/// `script_stdin`：`script -` 时发起侧从 stdin 读入的脚本内容（daemon 无法访问发起侧 stdin）。
pub fn post_cmd(
    port: u16,
    args: &[String],
    cwd: &std::path::Path,
    script_stdin: Option<&str>,
) -> BResult<crate::output::Output> {
    let mut body = json!({ "args": args, "cwd": cwd });
    if let Some(s) = script_stdin {
        body["script_stdin"] = json!(s);
    }
    let body = body.to_string();
    let raw = http_request(port, "POST", "/cmd", Some(&body), Duration::from_secs(120))
        .map_err(|e| ErrorBody::io_error(format!("转发命令到 daemon 失败: {e}")))?;
    serde_json::from_str(&raw)
        .map_err(|e| ErrorBody::io_error(format!("daemon 响应解析失败: {e}（原始响应: {raw}）")))
}
/// `pair` / `pair --reset`：经 daemon HTTP 管理端点取配对信息，组装配对 URI 与终端二维码
///（design.md 决策 8/13；管理端点只绑定 127.0.0.1）。
pub fn pair(config: &Config, reset: bool) -> BResult<Value> {
    if reset {
        http_request(
            config.http_port,
            "POST",
            "/pair-reset",
            Some("{}"),
            Duration::from_secs(5),
        )
        .map_err(|e| ErrorBody::io_error(format!("POST /pair-reset 失败: {e}")))?;
    }
    let body = http_request(
        config.http_port,
        "GET",
        "/pair-info",
        None,
        Duration::from_secs(5),
    )
    .map_err(|e| ErrorBody::io_error(format!("GET /pair-info 失败: {e}")))?;
    let info: Value = serde_json::from_str(&body)
        .map_err(|e| ErrorBody::io_error(format!("/pair-info 响应解析失败: {e}")))?;
    let code = info["pairing_code"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let active = info["code_active"].as_bool().unwrap_or(false);
    let port = info["port"].as_u64().unwrap_or(config.bridge_port as u64);
    let ips: Vec<String> = info["ips"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let host = ips
        .first()
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let uri = format!("agent-mobile://pair?host={host}&port={port}&code={code}");
    let qr = qr_unicode(&uri)?;
    let mut out = json!({
        "pairing_code": code,
        "code_active": active,
        "ips": ips,
        "uri": uri,
        "qr_unicode": qr,
    });
    if reset {
        out["reset"] = json!(true);
    }
    Ok(out)
}

/// `pair --proxy` / `pair --proxy --reset`：CLI 短进程直连代理服务签发配对码
/// （rulings「代理配对由 CLI 直连」；不经 daemon 中转）。输出代理配对 URI 与终端二维码。
pub fn pair_proxy(config: &Config, reset: bool) -> BResult<Value> {
    let proxy_cfg = config.proxy.as_ref().ok_or_else(|| {
        ErrorBody::proxy_error("未配置代理服务：请在 config.json 配置 proxy.url 与 proxy.token")
    })?;
    let client = crate::backend::proxy::ProxyClient::from_config(proxy_cfg);
    if reset {
        client.pairing_reset()?;
    }
    let code = client.create_pairing_code()?;
    let (host, port, scheme) = proxy_authority(&proxy_cfg.url)?;
    // TLS 部署时 URI 需携带 scheme，App 扫码后按 wss:// 连接（FixReview SUGGESTION）。
    let scheme_param = scheme
        .filter(|s| matches!(s.as_str(), "https" | "wss"))
        .map(|_| "&scheme=wss".to_string())
        .unwrap_or_default();
    let uri = format!("agent-mobile://pair?host={host}&port={port}&code={code}{scheme_param}");
    let qr = qr_unicode(&uri)?;
    let mut out = json!({
        "pairing_code": code,
        "server": proxy_cfg.url,
        "uri": uri,
        "qr_unicode": qr,
    });
    if reset {
        out["reset"] = json!(true);
    }
    Ok(out)
}

/// 从代理 URL 提取 host、port 与 scheme（供配对 URI；缺省端口按 http=80/https=443）。
fn proxy_authority(url: &str) -> BResult<(String, u16, Option<String>)> {
    let (scheme, rest) = url
        .split_once("://")
        .map(|(s, r)| (Some(s.to_string()), r))
        .unwrap_or((None, url));
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port = p
                .parse::<u16>()
                .map_err(|_| ErrorBody::proxy_error(format!("代理地址端口无效: {url}")))?;
            (h.to_string(), port)
        }
        None => {
            let port = match scheme.as_deref() {
                Some("https") | Some("wss") => 443,
                _ => 80,
            };
            (authority.to_string(), port)
        }
    };
    if host.is_empty() {
        return Err(ErrorBody::proxy_error(format!("代理地址缺少主机: {url}")));
    }
    Ok((host, port, scheme))
}

/// 配对 URI 渲染为终端 unicode block 二维码字符串（亮色块为实，适配深色终端背景）。
fn qr_unicode(content: &str) -> BResult<String> {
    let code = qrcode::QrCode::new(content.as_bytes())
        .map_err(|e| ErrorBody::io_error(format!("配对二维码生成失败: {e}")))?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Light)
        .light_color(qrcode::render::unicode::Dense1x2::Dark)
        .build())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::lock::{LockInfo, LOCK_FILE_NAME};
    use std::io::ErrorKind;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn bridge_ws_binds_lan_http_stays_loopback() {
        // design.md 决策 8 威胁模型：桥接 WS 对 LAN 可达（真机扫码/直连），
        // HTTP 管理端点（/pair-info 含配对码）必须保持 loopback。
        assert_eq!(WS_BIND_ADDR, "0.0.0.0");
        assert_eq!(HTTP_BIND_ADDR, "127.0.0.1");
        // 绑定行为可执行性：WS 通配地址可绑且接受本机连接
        let listener = tokio::net::TcpListener::bind((WS_BIND_ADDR, 0))
            .await
            .expect("WS 应可绑定 0.0.0.0");
        let port = listener.local_addr().unwrap().port();
        let accept = tokio::spawn(async move { listener.accept().await });
        let client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("应可连接 WS 监听端口");
        drop(client);
        accept.await.unwrap().expect("应接受连接");
    }
    /// 极简 HTTP stub：任何请求返回 200 + JSON；记录 /shutdown 命中次数。
    struct StubServer {
        port: u16,
        shutdowns: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl StubServer {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("绑定随机端口");
            listener.set_nonblocking(true).expect("设置非阻塞");
            let port = listener.local_addr().expect("读取本地地址").port();
            let shutdowns = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let shutdowns2 = Arc::clone(&shutdowns);
            let stop2 = Arc::clone(&stop);
            let handle = std::thread::spawn(move || {
                while !stop2.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_millis(500)))
                                .expect("设置读超时");
                            let mut buf = [0u8; 4096];
                            let n = stream.read(&mut buf).unwrap_or(0);
                            let req = String::from_utf8_lossy(&buf[..n]);
                            let body = if req.starts_with("POST /shutdown") {
                                shutdowns2.fetch_add(1, Ordering::SeqCst);
                                r#"{"ok":true}"#
                            } else {
                                r#"{"ok":true,"version":"stub","uptime_secs":1}"#
                            };
                            let response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes());
                        }
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                port,
                shutdowns,
                stop,
                handle: Some(handle),
            }
        }
    }

    impl Drop for StubServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("创建临时目录")
    }

    /// 取一个当前无人监听的空闲端口（绑定后立刻释放）。
    fn unused_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定随机端口");
        listener.local_addr().expect("读取本地地址").port()
    }

    fn write_lock(dir: &std::path::Path, port: u16) {
        let info = LockInfo {
            pid: std::process::id(),
            port,
            started_at_epoch_secs: 1,
        };
        std::fs::write(
            dir.join(LOCK_FILE_NAME),
            serde_json::to_string(&info).expect("序列化锁"),
        )
        .expect("写入锁文件");
    }

    #[test]
    fn proxy_authority_parses_forms() {
        assert_eq!(
            proxy_authority("http://proxy.example.com:28777").unwrap(),
            (
                "proxy.example.com".to_string(),
                28777,
                Some("http".to_string())
            )
        );
        assert_eq!(
            proxy_authority("https://debug.example.com").unwrap(),
            (
                "debug.example.com".to_string(),
                443,
                Some("https".to_string())
            )
        );
        assert_eq!(
            proxy_authority("http://10.0.0.2:9000/base").unwrap(),
            ("10.0.0.2".to_string(), 9000, Some("http".to_string()))
        );
        assert_eq!(
            proxy_authority("192.168.1.5:28777").unwrap(),
            ("192.168.1.5".to_string(), 28777, None)
        );
        assert!(proxy_authority("http://:28777").is_err());
        assert!(proxy_authority("http://host:notaport").is_err());
    }

    #[test]
    fn pair_proxy_requires_config() {
        let dir = temp_dir();
        std::env::set_var("AGENT_MOBILE_HOME", dir.path());
        let cfg = crate::config::Config::default();
        let err = pair_proxy(&cfg, false).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::ProxyError);
        std::env::remove_var("AGENT_MOBILE_HOME");
    }

    /// 代理配对 URI 组成：http 部署不带 scheme 参数；code 来自服务端响应。
    /// （https 部署的 &scheme=wss 由 proxy_authority 提取测试 + 组合逻辑覆盖。）
    #[test]
    fn pair_proxy_uri_http_无scheme参数() {
        let dir = temp_dir();
        std::env::set_var("AGENT_MOBILE_HOME", dir.path());
        // 极简 stub：POST /pairing-codes -> {"pairing_code":"483920"}
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                // 读满请求头 + 按 Content-Length 读完整 body 再响应：
                // 提前关闭会在接收缓冲区留未读数据触发 RST，客户端响应被丢弃
                // （ureq 报误导性的 header EINVAL，并行负载下间歇复现）。
                // 断言留给主线程（stub 内 panic 同样会 RST 连接）。
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut head_end = None;
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if head_end.is_none() {
                        head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4);
                    }
                    if let Some(he) = head_end {
                        let head = String::from_utf8_lossy(&buf[..he]);
                        let len: usize = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse().ok())
                            })
                            .unwrap_or(0);
                        if buf.len() >= he + len {
                            break;
                        }
                    }
                }
                let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
                let body = r#"{"pairing_code":"483920"}"#;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        let cfg = crate::config::Config {
            proxy: Some(crate::config::ProxyConfig {
                url: format!("http://127.0.0.1:{port}"),
                token: "tok".to_string(),
            }),
            ..Default::default()
        };
        let out = pair_proxy(&cfg, false).unwrap();
        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stub 应收到请求");
        assert!(req.starts_with("POST /pairing-codes"), "实际请求: {req}");
        let uri = out["uri"].as_str().unwrap();
        assert_eq!(
            uri,
            format!("agent-mobile://pair?host=127.0.0.1&port={port}&code=483920")
        );
        assert!(!uri.contains("scheme"), "http 部署不应携带 scheme: {uri}");
        std::env::remove_var("AGENT_MOBILE_HOME");
    }

    /// 起真实 HTTP 服务（配对状态用临时目录），返回端口与 shutdown。
    async fn start_http(dir: &std::path::Path) -> (u16, Arc<Notify>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("绑定随机端口");
        let port = listener.local_addr().expect("读取地址").port();
        let shutdown = Arc::new(Notify::new());
        let state = crate::exec::DaemonState::new_in(crate::config::Config::default(), dir);
        tokio::spawn(http::serve(listener, Arc::clone(&shutdown), state));
        (port, shutdown)
    }

    // pair() 内部是阻塞 HTTP 客户端：需多线程运行时，避免阻塞饿死 axum 服务
    #[tokio::test(flavor = "multi_thread")]
    async fn pair_outputs_code_ips_uri_and_qr() {
        let dir = temp_dir();
        let (port, shutdown) = start_http(dir.path()).await;
        let config = crate::config::Config {
            http_port: port,
            ..Default::default()
        };
        let v = pair(&config, false).expect("pair 应成功");
        let code = v["pairing_code"].as_str().expect("配对码");
        assert_eq!(code.len(), 6);
        assert_eq!(v["code_active"], true);
        assert!(v["ips"].is_array());
        let uri = v["uri"].as_str().expect("uri");
        assert!(
            uri.starts_with("agent-mobile://pair?host="),
            "uri 前缀: {uri}"
        );
        assert!(
            uri.contains(&format!("&port={}", config.bridge_port)),
            "uri 端口: {uri}"
        );
        assert!(uri.contains(&format!("&code={code}")), "uri 配对码: {uri}");
        let qr = v["qr_unicode"].as_str().expect("qr_unicode 为字符串字段");
        assert!(qr.lines().count() > 5, "二维码应多行: {qr}");
        assert!(
            qr.chars().any(|c| "█▄▀ ".contains(c) && c != ' '),
            "应含 unicode block"
        );

        // --reset：重置标记 + 配对码重新可用
        let v2 = pair(&config, true).expect("pair --reset 应成功");
        assert_eq!(v2["reset"], true);
        assert_eq!(v2["code_active"], true);
        assert_eq!(v2["pairing_code"].as_str().unwrap().len(), 6);
        shutdown.notify_one();
    }
    #[test]
    fn ensure_daemon_port_mismatch_returns_guidance() {
        let dir = temp_dir();
        let server = StubServer::start();
        // 配置端口被改：与锁内端口不一致，且锁内 daemon 健康存活
        let config = Config {
            http_port: unused_port(),
            ..Config::default()
        };
        write_lock(dir.path(), server.port);
        let err = ensure_daemon_in(&config, dir.path()).expect_err("端口不一致应直接报错");
        assert!(
            err.message.contains(&server.port.to_string()),
            "错误应包含锁内端口 {}: {}",
            server.port,
            err.message
        );
        assert!(
            err.message.contains(&config.http_port.to_string()),
            "错误应包含配置端口 {}: {}",
            config.http_port,
            err.message
        );
        assert!(
            err.message.contains("daemon-stop"),
            "错误应给出指引: {}",
            err.message
        );
    }

    #[test]
    fn status_prefers_lock_port_when_config_port_changed() {
        let dir = temp_dir();
        let server = StubServer::start();
        // 配置端口不通，但锁内端口上的 daemon 健康：status 应按锁内端口判定
        let config = Config {
            http_port: unused_port(),
            ..Config::default()
        };
        write_lock(dir.path(), server.port);
        let value = status_in(&config, dir.path()).expect("status 调用");
        assert_eq!(value["running"], true, "锁内端口健康应报运行中: {value}");
        assert_eq!(value["pid"], std::process::id());
        assert_eq!(value["http_port"], server.port);
        assert_eq!(value["started_at_epoch_secs"], 1);
    }

    #[test]
    fn status_lock_missing_but_healthy_reports_running() {
        let dir = temp_dir();
        let server = StubServer::start();
        // 锁文件缺失但配置端口健康（例如锁被手工删除）：(None, true) 不得误报未运行
        let config = Config {
            http_port: server.port,
            ..Config::default()
        };
        let value = status_in(&config, dir.path()).expect("status 调用");
        assert_eq!(value["running"], true, "健康检查通过应报运行中: {value}");
        assert!(value["pid"].is_null(), "锁缺失时 pid 应为 null: {value}");
        assert!(
            value["started_at_epoch_secs"].is_null(),
            "锁缺失时 started_at 应为 null: {value}"
        );
    }

    #[test]
    fn stop_uses_lock_port_when_config_port_unreachable() {
        let dir = temp_dir();
        let server = StubServer::start();
        // 配置端口不通，锁内端口在监听：stop 应按锁内端口 POST /shutdown
        let config = Config {
            http_port: unused_port(),
            ..Config::default()
        };
        write_lock(dir.path(), server.port);
        stop_in(&config, dir.path()).expect("stop 应按锁内端口关停");
        assert_eq!(
            server.shutdowns.load(Ordering::SeqCst),
            1,
            "应向锁内端口发起一次 /shutdown"
        );
    }
}
