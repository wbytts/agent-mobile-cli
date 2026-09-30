//! 公网代理后端（mobile-debug-proxy change，design.md 决策 2/11）。
//!
//! CLI/daemon 经 HTTP API 接入 mobile-debug-proxy-server：`GET /devices` 实时枚举远端
//! 设备（不做轮询缓存，实现简化见 rulings「代理设备采用实时查询」），command/script 经
//! `POST /devices/:name/commands|scripts` 同步下发。ureq 同步客户端：Backend 方法在
//! daemon `spawn_blocking` 线程执行，不可用 tokio 阻塞原语（项目记忆
//! daemon-spawn-blocking-block-in-place）；CLI 短进程同样直接调用。
//!
//! HTTP 契约（与 proxy server 对齐）：中继完成统一 200 + `{"ok", "result"|"error"}`；
//! 401 认证失败 → PROXY_AUTH；404 设备不存在/离线 → DEVICE_NOT_FOUND；504 等待超时
//! → TIMEOUT；其余非 2xx 与传输错误 → PROXY_ERROR。

use super::{BResult, Backend, BackendKind, ConnectionKind, DeviceRecord, DeviceState, TapTarget};
use crate::config::ProxyConfig;
use crate::output::ErrorBody;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 代理 API 默认超时（命令中继与设备执行耗时叠加，对齐桥接 30s 契约）。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);

/// mobile-debug-proxy-server 的同步 HTTP 客户端。
pub struct ProxyClient {
    base: String,
    token: String,
    timeout: Duration,
}

/// 代理侧设备记录（GET /devices 响应元素）。
#[derive(Debug, Clone, Deserialize)]
pub struct ProxyDevice {
    pub name: String,
    pub online: bool,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[allow(dead_code)] // 保留字段：枚举输出与排障使用
    pub last_seen: Option<String>,
}

/// URL 路径段百分编码（FixReview IMPORTANT-2）：设备名可含空格/中文，
/// 未编码插值会破坏请求行与路由匹配。保留 unreserved 字符，其余按字节 %XX。
fn encode_path_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl ProxyClient {
    pub fn new(url: &str, token: &str) -> Self {
        Self {
            base: url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            timeout: REQUEST_TIMEOUT,
        }
    }

    #[allow(dead_code)] // bin 使用；tests 的 #[path] 模块树不经过此入口
    pub fn from_config(cfg: &ProxyConfig) -> Self {
        Self::new(&cfg.url, &cfg.token)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(self.timeout).build()
    }

    /// 统一请求与错误映射：非 2xx 按状态码归类，传输失败 → PROXY_ERROR。
    fn request(&self, method: &str, path: &str, body: Option<Value>) -> BResult<Value> {
        let url = format!("{}{}", self.base, path);
        let req = self
            .agent()
            .request(method, &url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .timeout(self.timeout);
        let result = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match result {
            Ok(resp) => resp.into_json::<Value>().map_err(|e| {
                ErrorBody::proxy_error(format!("代理服务响应解析失败: {e}（{method} {path}）"))
            }),
            Err(ureq::Error::Status(status, resp)) => {
                let detail = resp
                    .into_json::<Value>()
                    .ok()
                    .and_then(|v| v["error"].as_str().map(str::to_owned))
                    .unwrap_or_else(|| format!("HTTP {status}"));
                Err(match status {
                    401 | 403 => ErrorBody::proxy_auth(format!(
                        "代理服务认证失败：检查配置 proxy.token（{detail}）"
                    )),
                    404 => {
                        ErrorBody::device_not_found(format!("代理设备不存在或已离线（{detail}）"))
                    }
                    504 => ErrorBody::timeout(format!("等待代理设备执行结果超时（{detail}）")),
                    _ => ErrorBody::proxy_error(format!("代理服务返回 HTTP {status}: {detail}")),
                })
            }
            Err(ureq::Error::Transport(t)) => {
                if t.kind() == ureq::ErrorKind::Io && t.to_string().contains("timed out") {
                    Err(ErrorBody::timeout(format!("代理服务请求超时: {t}")))
                } else {
                    Err(ErrorBody::proxy_error(format!(
                        "代理服务不可达（{}）: {t}",
                        self.base
                    )))
                }
            }
        }
    }

    pub fn list_devices(&self) -> BResult<Vec<ProxyDevice>> {
        let v = self.request("GET", "/devices", None)?;
        let devices = v
            .get("devices")
            .cloned()
            .ok_or_else(|| ErrorBody::proxy_error("代理 /devices 响应缺少 devices 字段"))?;
        serde_json::from_value(devices)
            .map_err(|e| ErrorBody::proxy_error(format!("代理 /devices 设备记录解析失败: {e}")))
    }

    /// 下发 command 并返回设备结果值；ok:false → AdbError（对齐桥接 map_result 语义）。
    pub fn command(&self, name: &str, method: &str, params: Value) -> BResult<Value> {
        let v = self.request(
            "POST",
            &format!("/devices/{}/commands", encode_path_segment(name)),
            Some(json!({ "method": method, "params": params })),
        )?;
        map_relay_result(name, method, v)
    }

    pub fn script(&self, name: &str, source: &str) -> BResult<Value> {
        let v = self.request(
            "POST",
            &format!("/devices/{}/scripts", encode_path_segment(name)),
            Some(json!({ "source": source })),
        )?;
        map_relay_result(name, "script", v)
    }

    pub fn create_pairing_code(&self) -> BResult<String> {
        let v = self.request("POST", "/pairing-codes", Some(json!({})))?;
        v["pairing_code"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ErrorBody::proxy_error("代理 /pairing-codes 响应缺少 pairing_code"))
    }

    pub fn pairing_reset(&self) -> BResult<()> {
        self.request("POST", "/pairing-reset", Some(json!({})))?;
        Ok(())
    }
}

/// 中继响应映射：ok → result（缺省 Null）；!ok → AdbError（含方法/设备/错误描述）。
fn map_relay_result(device: &str, method: &str, v: Value) -> BResult<Value> {
    match v["ok"].as_bool() {
        Some(true) => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
        Some(false) => Err(ErrorBody::adb_error(
            format!(
                "代理命令 {method} 设备 {device} 执行失败: {}",
                v["error"].as_str().unwrap_or("未知错误")
            ),
            Some(json!({ "device": device, "method": method })),
        )),
        None => Err(ErrorBody::proxy_error(format!(
            "代理中继响应缺少 ok 字段（设备 {device} 方法 {method}）"
        ))),
    }
}

/// 代理后端：Backend trait 的 proxy 通路实现。能力集与桥接一致
/// （stop/logcat/shell/connect 不支持），命令经 ProxyClient 下发。
pub struct ProxyBackend {
    client: Option<ProxyClient>,
}

impl ProxyBackend {
    #[allow(dead_code)] // bin 使用；adb_integration 测试树不实例化
    pub fn new(config: Option<&ProxyConfig>) -> Self {
        Self {
            client: config.map(ProxyClient::from_config),
        }
    }

    /// 测试用：直接注入客户端。（bin target 视角为 dead code：经 tests 的 #[path] 模块树使用）
    #[allow(dead_code)]
    pub fn from_client(client: ProxyClient) -> Self {
        Self {
            client: Some(client),
        }
    }

    /// 未配置代理服务的后端实例（全部命令报 PROXY_ERROR）。同 #[path] 测试树使用。
    #[allow(dead_code)]
    pub fn unconfigured() -> Self {
        Self { client: None }
    }

    fn client(&self) -> BResult<&ProxyClient> {
        self.client.as_ref().ok_or_else(|| {
            ErrorBody::proxy_error("未配置代理服务：请在 config.json 配置 proxy.url 与 proxy.token")
        })
    }

    /// 设备名解引用：proxy:<name> → name；非代理 id → 用法错误。
    fn device_name(device: &str) -> BResult<&str> {
        super::proxy_device_name(device)
            .ok_or_else(|| ErrorBody::usage(format!("代理后端收到非代理设备 id: {device}")))
    }

    /// 能力校验：实时查询设备记录，未上报能力 → NOT_SUPPORTED，不下发命令。
    fn require(&self, device: &str, cap: &str) -> BResult<()> {
        let name = Self::device_name(device)?;
        let devices = self.client()?.list_devices()?;
        let dev = devices
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| ErrorBody::device_not_found(format!("代理设备 {device} 未注册")))?;
        if !dev.online {
            return Err(ErrorBody::device_offline(format!("代理设备 {device} 离线")));
        }
        if dev.capabilities.iter().any(|c| c == cap) {
            Ok(())
        } else {
            Err(ErrorBody::not_supported(format!(
                "代理设备 {device} 未上报 {cap} 能力，命令不可用"
            )))
        }
    }

    fn call(&self, device: &str, method: &str, params: Value) -> BResult<Value> {
        let name = Self::device_name(device)?;
        self.client()?.command(name, method, params)
    }

    /// 取 result Value 的字符串字段（与桥接 result_str 同契约）。
    fn result_str<'a>(v: &'a Value, field: &str, method: &str, device: &str) -> BResult<&'a str> {
        v.get(field).and_then(Value::as_str).ok_or_else(|| {
            ErrorBody::adb_error(
                format!("代理命令 {method} 设备 {device} 返回缺少 {field} 字符串字段"),
                Some(json!({ "result": v })),
            )
        })
    }

    fn not_supported(capability: &str, device: &str) -> ErrorBody {
        ErrorBody::not_supported(format!("代理后端不支持 {capability} 能力（设备 {device}）"))
    }
}

impl Backend for ProxyBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Proxy
    }

    fn devices(&self) -> BResult<Vec<DeviceRecord>> {
        let devices = self.client()?.list_devices()?;
        Ok(devices
            .into_iter()
            .map(|d| DeviceRecord {
                id: super::proxy_device_id(&d.name),
                kind: BackendKind::Proxy,
                model: None,
                state: if d.online {
                    DeviceState::Online
                } else {
                    DeviceState::Offline
                },
                connection: ConnectionKind::Proxy,
            })
            .collect())
    }

    /// 代理设备由 App 绑定代理服务注册，无 connect 语义。
    fn connect(&self, target: &str) -> BResult<String> {
        Err(ErrorBody::not_supported(format!(
            "代理后端不支持 connect 能力（目标 {target}）：代理设备由 App 绑定代理服务注册"
        )))
    }

    fn snapshot(&self, device: &str, full: bool) -> BResult<crate::ui::Snapshot> {
        self.require(device, "uiTree")?;
        let v = self.call(device, "uiTree", json!({}))?;
        let xml = Self::result_str(&v, "xml", "uiTree", device)?;
        if full {
            Ok(crate::ui::Snapshot {
                tree: xml.to_string(),
                refs: Vec::new(),
            })
        } else {
            crate::ui::simplify(xml).map_err(|e| {
                ErrorBody::adb_error(
                    "代理 uiTree 返回的 XML 解析失败",
                    Some(json!({ "device": device, "error": e })),
                )
            })
        }
    }

    fn tap(&self, device: &str, target: TapTarget) -> BResult<()> {
        match target {
            TapTarget::Coord(x, y) => {
                self.require(device, "tap")?;
                self.call(device, "tap", json!({ "x": x, "y": y }))?;
                Ok(())
            }
            TapTarget::Ref(_) => Err(ErrorBody::not_supported(
                "Ref 目标由 executor 层解引用为坐标后下发，backend 不直接处理",
            )),
        }
    }

    fn swipe(
        &self,
        device: &str,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        duration_ms: u32,
    ) -> BResult<()> {
        self.require(device, "swipe")?;
        self.call(
            device,
            "swipe",
            json!({ "x1": x1, "y1": y1, "x2": x2, "y2": y2, "duration_ms": duration_ms }),
        )?;
        Ok(())
    }

    fn input_text(&self, device: &str, text: &str) -> BResult<()> {
        self.require(device, "input")?;
        self.call(device, "input", json!({ "text": text }))?;
        Ok(())
    }

    fn key(&self, device: &str, key: &str) -> BResult<()> {
        self.require(device, "key")?;
        self.call(device, "key", json!({ "key": key }))?;
        Ok(())
    }

    /// 截图：result 携带 base64 PNG（同桥接契约），解码校验魔数后写文件。
    fn screenshot(&self, device: &str, out: &Path) -> BResult<PathBuf> {
        use base64::Engine;
        self.require(device, "screenshot")?;
        let v = self.call(device, "screenshot", json!({}))?;
        let b64 = Self::result_str(&v, "png_base64", "screenshot", device)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| {
                ErrorBody::adb_error(
                    format!("代理 screenshot 设备 {device} 返回的 png_base64 解码失败: {e}"),
                    None,
                )
            })?;
        super::adb::validate_png(&bytes)?;
        std::fs::write(out, &bytes)
            .map_err(|e| ErrorBody::io_error(format!("写入截图 {} 失败: {e}", out.display())))?;
        Ok(out.to_path_buf())
    }

    fn apps(&self, device: &str, filter: Option<&str>, all: bool) -> BResult<Vec<String>> {
        self.require(device, "apps")?;
        let v = self.call(device, "apps", json!({ "filter": filter, "all": all }))?;
        let arr = v.get("packages").and_then(Value::as_array).ok_or_else(|| {
            ErrorBody::adb_error(
                format!("代理命令 apps 设备 {device} 返回缺少 packages 数组"),
                Some(json!({ "result": v })),
            )
        })?;
        arr.iter()
            .map(|p| {
                p.as_str().map(str::to_owned).ok_or_else(|| {
                    ErrorBody::adb_error(
                        format!("代理命令 apps 设备 {device} 返回非字符串包名"),
                        Some(json!({ "item": p })),
                    )
                })
            })
            .collect()
    }

    fn launch(&self, device: &str, package: &str) -> BResult<()> {
        self.require(device, "launch")?;
        self.call(device, "launch", json!({ "package": package }))?;
        Ok(())
    }

    fn script(&self, device: &str, source: &str) -> BResult<Value> {
        self.require(device, "script")?;
        let name = Self::device_name(device)?;
        self.client()?.script(name, source)
    }

    fn stop(&self, device: &str, _package: &str) -> BResult<()> {
        Err(Self::not_supported("stop", device))
    }

    fn logcat(
        &self,
        device: &str,
        _lines: u32,
        _tag: Option<&str>,
        _level: Option<&str>,
    ) -> BResult<String> {
        Err(Self::not_supported("logcat", device))
    }

    fn shell(&self, device: &str, _cmd: &[String]) -> BResult<crate::backend::ShellResult> {
        Err(Self::not_supported("shell", device))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// 极简 HTTP stub：接收一个请求后返回预置响应（Connection: close）。
    /// 返回 (base_url, 请求记录接收通道)。
    fn stub_server(respond: impl Fn(&str) -> (u16, String) + Send + 'static) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            // 服务多个顺序请求（list_devices + command 等）
            for stream in listener.incoming() {
                let mut stream = match stream {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // 读到头结束 + 按 Content-Length 读 body
                let mut head_end = None;
                loop {
                    let n = match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    buf.extend_from_slice(&chunk[..n]);
                    if head_end.is_none() {
                        head_end = find(&buf, b"\r\n\r\n").map(|p| p + 4);
                    }
                    if let Some(he) = head_end {
                        let head = String::from_utf8_lossy(&buf[..he]).to_string();
                        let len = content_length(&head);
                        if buf.len() >= he + len {
                            break;
                        }
                    }
                }
                let raw = String::from_utf8_lossy(&buf).to_string();
                let (status, body) = respond(&raw);
                let reason = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    504 => "Gateway Timeout",
                    _ => "Error",
                };
                let resp = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    fn content_length(head: &str) -> usize {
        head.lines()
            .find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse().ok())
            })
            .unwrap_or(0)
    }

    fn client(base: &str) -> ProxyClient {
        ProxyClient::new(base, "tok123")
    }

    #[test]
    fn list_devices_maps_records() {
        let base = stub_server(|req| {
            assert!(req.starts_with("GET /devices "));
            assert!(req.contains("Authorization: Bearer tok123"));
            (
                200,
                json!({"devices":[{"name":"MuMu","online":true,"capabilities":["tap","uiTree"],"last_seen":"2026-09-29T12:00:00Z"}]}).to_string(),
            )
        });
        let devices = client(&base).list_devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "MuMu");
        assert!(devices[0].online);
        assert!(devices[0].capabilities.contains(&"uiTree".to_string()));
    }

    /// FixReview IMPORTANT-2：含空格/中文的设备名在请求路径中必须百分编码，
    /// 否则请求行非法或路由段错误（服务端 axum 解码后匹配原名）。
    #[test]
    fn command_设备名含空格中文_路径百分编码() {
        let base = stub_server(|req| {
            assert!(
                req.starts_with("POST /devices/Pixel%207%20%E6%B5%8B%E8%AF%95/commands "),
                "请求行路径段应百分编码: {}",
                req.lines().next().unwrap_or("")
            );
            (200, json!({"ok":true,"result":{}}).to_string())
        });
        client(&base)
            .command("Pixel 7 测试", "uiTree", json!({}))
            .unwrap();
    }

    #[test]
    fn encode_path_segment_保留unreserved() {
        assert_eq!(encode_path_segment("MuMu-12_x.z~"), "MuMu-12_x.z~");
        assert_eq!(encode_path_segment("a b"), "a%20b");
        assert_eq!(encode_path_segment("a/b?c#d%e"), "a%2Fb%3Fc%23d%25e");
    }

    #[test]
    fn auth_failure_maps_proxy_auth() {
        let base = stub_server(|_| (401, json!({"error":"unauthorized"}).to_string()));
        let err = client(&base).list_devices().unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::ProxyAuth);
    }

    #[test]
    fn command_success_returns_result() {
        let base = stub_server(|req| {
            assert!(req.starts_with("POST /devices/MuMu/commands "));
            assert!(req.contains(r#""method":"tap""#));
            (
                200,
                json!({"ok":true,"result":{"tapped":[100,200]}}).to_string(),
            )
        });
        let v = client(&base)
            .command("MuMu", "tap", json!({"x":100,"y":200}))
            .unwrap();
        assert_eq!(v["tapped"], json!([100, 200]));
    }

    #[test]
    fn command_device_failure_maps_adb_error() {
        let base = stub_server(|_| {
            (
                200,
                json!({"ok":false,"error":"element not interactable"}).to_string(),
            )
        });
        let err = client(&base)
            .command("MuMu", "tap", json!({"x":1,"y":2}))
            .unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::AdbError);
        assert!(err.message.contains("element not interactable"));
    }

    #[test]
    fn command_offline_maps_device_not_found() {
        let base = stub_server(|_| {
            (
                404,
                json!({"error":"device not found or offline"}).to_string(),
            )
        });
        let err = client(&base)
            .command("MuMu", "tap", json!({"x":1,"y":2}))
            .unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::DeviceNotFound);
    }

    #[test]
    fn command_timeout_maps_timeout() {
        let base = stub_server(|_| {
            (
                504,
                json!({"error":"timeout waiting for device result"}).to_string(),
            )
        });
        let err = client(&base)
            .command("MuMu", "tap", json!({"x":1,"y":2}))
            .unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::Timeout);
    }

    #[test]
    fn unreachable_maps_proxy_error() {
        // 绑定后立即释放端口，连接必失败
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let err = client(&format!("http://127.0.0.1:{port}"))
            .list_devices()
            .unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::ProxyError);
    }

    #[test]
    fn pairing_code_created() {
        let base = stub_server(|req| {
            assert!(req.starts_with("POST /pairing-codes "));
            (200, json!({"pairing_code":"483920"}).to_string())
        });
        let code = client(&base).create_pairing_code().unwrap();
        assert_eq!(code, "483920");
    }

    #[test]
    fn pairing_reset_calls_endpoint() {
        let base = stub_server(|req| {
            assert!(req.starts_with("POST /pairing-reset "));
            (200, json!({"reset":true}).to_string())
        });
        client(&base).pairing_reset().unwrap();
    }

    #[test]
    fn script_posts_source() {
        let base = stub_server(|req| {
            assert!(req.starts_with("POST /devices/MuMu/scripts "));
            assert!(req.contains("mobile.tap"));
            (200, json!({"ok":true,"result":{"value":42}}).to_string())
        });
        let v = client(&base).script("MuMu", "mobile.tap(1,2)").unwrap();
        assert_eq!(v["value"], json!(42));
    }

    fn backend_with(base: &str) -> ProxyBackend {
        ProxyBackend::from_client(client(base))
    }

    #[test]
    fn backend_unconfigured_rejects() {
        let b = ProxyBackend::unconfigured();
        let err = b.tap("proxy:MuMu", TapTarget::Coord(1, 2)).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::ProxyError);
        assert!(err.message.contains("未配置代理服务"));
    }

    #[test]
    fn backend_tap_strips_prefix_and_checks_capability() {
        let base = stub_server(|req| {
            if req.starts_with("GET /devices ") {
                (
                    200,
                    json!({"devices":[{"name":"MuMu","online":true,"capabilities":["tap"],"last_seen":"2026-09-29T12:00:00Z"}]}).to_string(),
                )
            } else {
                assert!(req.starts_with("POST /devices/MuMu/commands "));
                (200, json!({"ok":true,"result":null}).to_string())
            }
        });
        backend_with(&base)
            .tap("proxy:MuMu", TapTarget::Coord(10, 20))
            .unwrap();
    }

    #[test]
    fn backend_missing_capability_rejects_without_command() {
        let base = stub_server(|req| {
            assert!(req.starts_with("GET /devices "));
            assert!(!req.starts_with("POST /devices"), "能力缺失时不得下发命令");
            (
                200,
                json!({"devices":[{"name":"MuMu","online":true,"capabilities":["uiTree"],"last_seen":"2026-09-29T12:00:00Z"}]}).to_string(),
            )
        });
        let err = backend_with(&base)
            .tap("proxy:MuMu", TapTarget::Coord(10, 20))
            .unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::NotSupported);
    }

    #[test]
    fn backend_devices_records() {
        let base = stub_server(|_| {
            (
                200,
                json!({"devices":[{"name":"MuMu","online":true,"capabilities":[],"last_seen":"2026-09-29T12:00:00Z"},{"name":"Old","online":false,"capabilities":[],"last_seen":"2026-09-28T12:00:00Z"}]}).to_string(),
            )
        });
        let records = backend_with(&base).devices().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].id, "proxy:MuMu");
        assert_eq!(records[0].kind, BackendKind::Proxy);
        assert_eq!(records[0].connection, ConnectionKind::Proxy);
        assert_eq!(records[0].state, DeviceState::Online);
        assert_eq!(records[1].state, DeviceState::Offline);
    }

    #[test]
    fn backend_stop_not_supported() {
        let b = ProxyBackend::unconfigured();
        // 未配置时 stop 也是 not_supported？——未配置优先报未配置
        let err = b.stop("proxy:MuMu", "pkg").unwrap_err();
        assert!(matches!(
            err.code,
            crate::output::ErrorCode::NotSupported | crate::output::ErrorCode::ProxyError
        ));
    }
}
