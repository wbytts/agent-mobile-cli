//! App 桥接后端：Backend trait 的桥接设备实现（design.md 决策 6/12）。
//!
//! 八个可用方法（snapshot/tap/swipe/input/key/screenshot/apps/launch）与 script 经
//! [`BridgeRegistry`] 的 WS 通道下发 command/script 帧并同步等待 result（超时 30s → TIMEOUT）；
//! snapshot 的 tree/refs 复用 [`crate::ui::simplify`] 同一 @eN 分配逻辑（桥接 uiTree 返回
//! uiautomator dump 同构 XML，design.md 决策 5）。stop/logcat/shell/connect 为桥接不支持能力，
//! 返回 NOT_SUPPORTED 结构化错误，不静默路由到其他后端。
//!
//! 调用上下文：Backend 方法在 daemon 的 `spawn_blocking` 线程执行（HTTP /cmd 转发），
//! 故等待实现不依赖 tokio 运行时（`block_in_place` 在 blocking 线程会 panic），
//! 采用 try_recv 轮询 + 截止期限。

use super::adb::validate_png;
use super::{BResult, Backend, BackendKind, DeviceRecord, ShellResult, TapTarget};
use crate::bridge_proto::{Capability, ResultMessage};
use crate::daemon::registry::BridgeRegistry;
use crate::output::ErrorBody;
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// 桥接命令 result 等待超时（组 5 契约：30s → TIMEOUT）。
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 等待轮询间隔：设备命令时延为毫秒~秒级，2ms 轮询的附加延迟可忽略。
const POLL_INTERVAL: Duration = Duration::from_millis(2);

pub struct AppBridgeBackend {
    registry: Arc<BridgeRegistry>,
    timeout: Duration,
}

impl AppBridgeBackend {
    pub fn new(registry: Arc<BridgeRegistry>) -> Self {
        Self {
            registry,
            timeout: COMMAND_TIMEOUT,
        }
    }

    /// 测试用：自定义 result 等待超时（生产固定 [`COMMAND_TIMEOUT`]）。
    #[cfg(test)]
    fn with_timeout(registry: Arc<BridgeRegistry>, timeout: Duration) -> Self {
        Self { registry, timeout }
    }

    /// 能力校验（design.md 决策 6）：hello 上报能力集缺失即 NOT_SUPPORTED
    /// （消息含能力名与设备名），不下发帧。
    fn require(&self, device: &str, cap: Capability) -> BResult<()> {
        let caps = self
            .registry
            .capabilities(device)
            .ok_or_else(|| ErrorBody::device_not_found(format!("桥接设备 {device} 未注册")))?;
        if caps.contains(&cap) {
            Ok(())
        } else {
            Err(ErrorBody::not_supported(format!(
                "桥接设备 {device} 未上报 {} 能力，命令不可用",
                cap_name(cap)
            )))
        }
    }

    /// 下发 command 帧并同步等待 result，映射为结果 Value。
    fn call(&self, device: &str, method: &str, params: Value) -> BResult<Value> {
        let rx = self.registry.command(device, method, params)?;
        let msg = wait_result(device, method, rx, self.timeout)?;
        map_result(device, method, msg)
    }
}

/// 能力名（协议 camelCase 取值，与 hello capabilities 一致）。
fn cap_name(cap: Capability) -> String {
    serde_json::to_value(cap)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{cap:?}"))
}

/// 同步等待 result：try_recv 轮询至超时；发送端消失（pending 丢弃/连接断开）判离线。
fn wait_result(
    device: &str,
    method: &str,
    mut rx: oneshot::Receiver<ResultMessage>,
    timeout: Duration,
) -> BResult<ResultMessage> {
    let deadline = Instant::now() + timeout;
    loop {
        match rx.try_recv() {
            Ok(msg) => return Ok(msg),
            Err(oneshot::error::TryRecvError::Closed) => {
                return Err(ErrorBody::device_offline(format!(
                    "桥接命令 {method} 等待期间设备 {device} 连接断开"
                )));
            }
            Err(oneshot::error::TryRecvError::Empty) => {
                if Instant::now() >= deadline {
                    return Err(ErrorBody::timeout(format!(
                        "桥接命令 {method} 等待设备 {device} 回传超时（{} 秒）",
                        timeout.as_secs()
                    )));
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

/// result 消息映射：ok → result Value（缺省 Null）；!ok → AdbError（含方法/设备/错误描述）。
fn map_result(device: &str, method: &str, msg: ResultMessage) -> BResult<Value> {
    if msg.ok {
        Ok(msg.result.unwrap_or(Value::Null))
    } else {
        Err(ErrorBody::adb_error(
            format!(
                "桥接命令 {method} 在设备 {device} 执行失败: {}",
                msg.error.as_deref().unwrap_or("未知错误")
            ),
            None,
        ))
    }
}

/// 取 result Value 的字符串字段；桥接返回结构不符合契约 → AdbError。
fn result_str<'a>(v: &'a Value, field: &str, method: &str, device: &str) -> BResult<&'a str> {
    v.get(field).and_then(Value::as_str).ok_or_else(|| {
        ErrorBody::adb_error(
            format!("桥接命令 {method} 设备 {device} 返回缺少字符串字段 {field}"),
            Some(json!({ "result": v })),
        )
    })
}

/// 桥接不支持能力的统一错误（消息含能力名与设备名，design.md 决策 12）。
fn not_supported(capability: &str, device: &str) -> ErrorBody {
    ErrorBody::not_supported(format!("桥接后端不支持 {capability} 能力（设备 {device}）"))
}

impl Backend for AppBridgeBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::AppBridge
    }

    fn devices(&self) -> BResult<Vec<DeviceRecord>> {
        Ok(self.registry.device_records())
    }

    /// 桥接设备由 App 主动连接注册，无 connect 语义。
    fn connect(&self, target: &str) -> BResult<String> {
        Err(ErrorBody::not_supported(format!(
            "桥接后端不支持 connect 能力（目标 {target}）：桥接设备由 App 主动连接注册"
        )))
    }

    /// UI 快照：uiTree 返回 uiautomator 同构 XML；full → 原始 XML 直出，
    /// 否则复用 ui::simplify 的同一简化与 @eN 分配逻辑（design.md 决策 5）。
    fn snapshot(&self, device: &str, full: bool) -> BResult<crate::ui::Snapshot> {
        self.require(device, Capability::UiTree)?;
        let v = self.call(device, "uiTree", json!({}))?;
        let xml = result_str(&v, "xml", "uiTree", device)?;
        if full {
            Ok(crate::ui::Snapshot {
                tree: xml.to_string(),
                refs: Vec::new(),
            })
        } else {
            crate::ui::simplify(xml).map_err(|e| {
                ErrorBody::adb_error(
                    "桥接 uiTree 返回的 XML 解析失败",
                    Some(json!({ "device": device, "error": e })),
                )
            })
        }
    }

    fn tap(&self, device: &str, target: TapTarget) -> BResult<()> {
        match target {
            TapTarget::Coord(x, y) => {
                self.require(device, Capability::Tap)?;
                self.call(device, "tap", json!({ "x": x, "y": y }))?;
                Ok(())
            }
            // Ref 解引用由 daemon executor 层完成（查最近快照 refs 取中心点），backend 不处理。
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
        self.require(device, Capability::Swipe)?;
        self.call(
            device,
            "swipe",
            json!({ "x1": x1, "y1": y1, "x2": x2, "y2": y2, "duration_ms": duration_ms }),
        )?;
        Ok(())
    }

    /// 文本输入：JSON 传输无 shell 转义约束，任意 Unicode 文本原样下发
    /// （ADB 后端的 ASCII 限制源于 `input text` 转义体系，桥接路径不适用）。
    fn input_text(&self, device: &str, text: &str) -> BResult<()> {
        self.require(device, Capability::Input)?;
        self.call(device, "input", json!({ "text": text }))?;
        Ok(())
    }

    fn key(&self, device: &str, key: &str) -> BResult<()> {
        self.require(device, Capability::Key)?;
        self.call(device, "key", json!({ "key": key }))?;
        Ok(())
    }

    /// 截图：result 携带 base64 PNG（design.md 决策 15），解码校验魔数后写文件。
    fn screenshot(&self, device: &str, out: &Path) -> BResult<PathBuf> {
        self.require(device, Capability::Screenshot)?;
        let v = self.call(device, "screenshot", json!({}))?;
        let b64 = result_str(&v, "png_base64", "screenshot", device)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| {
                ErrorBody::adb_error(
                    format!("桥接 screenshot 设备 {device} 返回的 png_base64 解码失败: {e}"),
                    None,
                )
            })?;
        validate_png(&bytes)?;
        std::fs::write(out, &bytes)
            .map_err(|e| ErrorBody::io_error(format!("写入截图 {} 失败: {e}", out.display())))?;
        Ok(out.to_path_buf())
    }

    /// 应用列表：filter/all 由设备侧过滤（与 ADB 后端 pm list packages 语义一致）。
    fn apps(&self, device: &str, filter: Option<&str>, all: bool) -> BResult<Vec<String>> {
        self.require(device, Capability::Apps)?;
        let v = self.call(device, "apps", json!({ "filter": filter, "all": all }))?;
        let arr = v.get("packages").and_then(Value::as_array).ok_or_else(|| {
            ErrorBody::adb_error(
                format!("桥接命令 apps 设备 {device} 返回缺少 packages 数组"),
                Some(json!({ "result": v })),
            )
        })?;
        arr.iter()
            .map(|p| {
                p.as_str().map(str::to_owned).ok_or_else(|| {
                    ErrorBody::adb_error(
                        format!("桥接命令 apps 设备 {device} 返回非字符串包名"),
                        Some(json!({ "item": p })),
                    )
                })
            })
            .collect()
    }

    fn launch(&self, device: &str, package: &str) -> BResult<()> {
        self.require(device, Capability::Launch)?;
        self.call(device, "launch", json!({ "package": package }))?;
        Ok(())
    }

    /// 脚本执行：script 帧下发 QuickJS 沙盒，result 原样返回（design.md 决策 2/3）。
    fn script(&self, device: &str, source: &str) -> BResult<Value> {
        self.require(device, Capability::Script)?;
        let rx = self.registry.script(device, source)?;
        let msg = wait_result(device, "script", rx, self.timeout)?;
        map_result(device, "script", msg)
    }

    fn stop(&self, device: &str, _package: &str) -> BResult<()> {
        Err(not_supported("stop", device))
    }

    fn logcat(
        &self,
        device: &str,
        _lines: u32,
        _tag: Option<&str>,
        _level: Option<&str>,
    ) -> BResult<String> {
        Err(not_supported("logcat", device))
    }

    fn shell(&self, device: &str, _cmd: &[String]) -> BResult<ShellResult> {
        Err(not_supported("shell", device))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_proto::Hello;
    use crate::output::ErrorCode;
    use tokio::sync::mpsc;

    const TEST_XML: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>
<hierarchy rotation="0">
  <node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="com.x" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][100,200]">
    <node index="1" text="确定" resource-id="" class="android.widget.Button" package="com.x" content-desc="" checkable="false" checked="false" clickable="true" enabled="true" focusable="true" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[10,20][50,60]"/>
  </node>
</hierarchy>"#;

    fn full_caps() -> Vec<Capability> {
        vec![
            Capability::Tap,
            Capability::Swipe,
            Capability::Input,
            Capability::Key,
            Capability::UiTree,
            Capability::Screenshot,
            Capability::Apps,
            Capability::Launch,
            Capability::Script,
        ]
    }

    fn setup(
        caps: Vec<Capability>,
    ) -> (
        Arc<BridgeRegistry>,
        AppBridgeBackend,
        String,
        mpsc::UnboundedReceiver<String>,
    ) {
        let reg = BridgeRegistry::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let hello = Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: "MuMu".into(),
            android_version: "12".into(),
            capabilities: caps,
        };
        let (id, _conn, _close) = reg.register(&hello, tx);
        let backend = AppBridgeBackend::new(Arc::clone(&reg));
        (reg, backend, id, rx)
    }

    /// 模拟设备侧（独立线程）：收一帧 → check 断言 → 按 id 回传 result。
    fn device_side(
        reg: &Arc<BridgeRegistry>,
        mut rx: mpsc::UnboundedReceiver<String>,
        check: impl FnOnce(&Value) + Send + 'static,
        ok: bool,
        result: Option<Value>,
        error: Option<String>,
    ) -> std::thread::JoinHandle<()> {
        let reg = Arc::clone(reg);
        std::thread::spawn(move || {
            let frame = rx.blocking_recv().expect("应收到下发帧");
            let v: Value = serde_json::from_str(&frame).expect("帧为 JSON");
            check(&v);
            let id = v["id"].as_str().expect("帧含 id").to_string();
            reg.complete(ResultMessage {
                id,
                ok,
                result,
                error,
            });
        })
    }

    /// command 帧断言闭包：type/method/params 与 id 前缀。
    fn expect_command(method: &'static str, params: Value) -> impl FnOnce(&Value) + Send + 'static {
        move |v: &Value| {
            assert_eq!(v["type"], "command");
            assert_eq!(v["method"], method);
            assert_eq!(v["params"], params);
            assert!(v["id"].as_str().unwrap().starts_with("cmd-"));
        }
    }

    // ---- 八方法参数序列化与 result 映射 ----

    #[test]
    fn tap_command_serialized_and_result_mapped() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("tap", json!({"x": 3, "y": 4})),
            true,
            None,
            None,
        );
        backend.tap(&id, TapTarget::Coord(3, 4)).unwrap();
        dev.join().unwrap();
    }

    #[test]
    fn swipe_command_serialized() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command(
                "swipe",
                json!({"x1": 1, "y1": 2, "x2": 3, "y2": 4, "duration_ms": 250}),
            ),
            true,
            None,
            None,
        );
        backend.swipe(&id, 1, 2, 3, 4, 250).unwrap();
        dev.join().unwrap();
    }

    #[test]
    fn input_passes_text_verbatim() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("input", json!({"text": "hello 世界 %s"})),
            true,
            None,
            None,
        );
        // 桥接为 JSON 传输：空格/Unicode/% 原样下发（无 adb input text 转义体系）
        backend.input_text(&id, "hello 世界 %s").unwrap();
        dev.join().unwrap();
    }

    #[test]
    fn key_command_serialized() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("key", json!({"key": "KEYCODE_HOME"})),
            true,
            None,
            None,
        );
        backend.key(&id, "KEYCODE_HOME").unwrap();
        dev.join().unwrap();
    }

    #[test]
    fn snapshot_simplified_reuses_ui_model() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("uiTree", json!({})),
            true,
            Some(json!({"xml": TEST_XML})),
            None,
        );
        let snap = backend.snapshot(&id, false).unwrap();
        let expected = crate::ui::simplify(TEST_XML).unwrap();
        assert_eq!(snap.tree, expected.tree, "与 ADB 后端同一简化模型");
        assert_eq!(snap.refs.len(), 1);
        assert_eq!(snap.refs[0].id, "@e1");
        assert_eq!(snap.refs[0].center, (30, 40), "@eN 引用分配逻辑一致");
        dev.join().unwrap();
    }

    #[test]
    fn snapshot_full_returns_raw_xml() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("uiTree", json!({})),
            true,
            Some(json!({"xml": TEST_XML})),
            None,
        );
        let snap = backend.snapshot(&id, true).unwrap();
        assert_eq!(snap.tree, TEST_XML);
        assert!(snap.refs.is_empty());
        dev.join().unwrap();
    }

    #[test]
    fn screenshot_decodes_base64_and_writes_png() {
        let (reg, backend, id, rx) = setup(full_caps());
        let png = b"\x89PNG\r\n\x1a\nfake-payload";
        let b64 = base64::engine::general_purpose::STANDARD.encode(png);
        let dev = device_side(
            &reg,
            rx,
            expect_command("screenshot", json!({})),
            true,
            Some(json!({"png_base64": b64})),
            None,
        );
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("shot.png");
        let saved = backend.screenshot(&id, &out).unwrap();
        assert_eq!(saved, out);
        assert_eq!(std::fs::read(&out).unwrap(), png);
        dev.join().unwrap();
    }

    #[test]
    fn screenshot_rejects_non_png_payload() {
        let (reg, backend, id, rx) = setup(full_caps());
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"not a png");
        let dev = device_side(
            &reg,
            rx,
            expect_command("screenshot", json!({})),
            true,
            Some(json!({"png_base64": b64})),
            None,
        );
        let dir = tempfile::tempdir().unwrap();
        let err = backend
            .screenshot(&id, &dir.path().join("x.png"))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::AdbError);
        dev.join().unwrap();
    }

    #[test]
    fn apps_maps_params_and_packages() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("apps", json!({"filter": "com.x", "all": true})),
            true,
            Some(json!({"packages": ["com.x.a", "com.x.b"]})),
            None,
        );
        let apps = backend.apps(&id, Some("com.x"), true).unwrap();
        assert_eq!(apps, vec!["com.x.a", "com.x.b"]);
        dev.join().unwrap();
    }

    #[test]
    fn launch_command_serialized() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("launch", json!({"package": "com.x"})),
            true,
            None,
            None,
        );
        backend.launch(&id, "com.x").unwrap();
        dev.join().unwrap();
    }

    // ---- script 下发 ----

    #[test]
    fn script_frame_dispatched_and_result_returned() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            |v| {
                assert_eq!(v["type"], "script");
                assert_eq!(v["source"], "mobile.tap(1,2)");
            },
            true,
            Some(json!({"value": 42})),
            None,
        );
        let out = backend.script(&id, "mobile.tap(1,2)").unwrap();
        assert_eq!(out, json!({"value": 42}));
        dev.join().unwrap();
    }

    // ---- result 错误与超时映射 ----

    #[test]
    fn result_error_mapped_with_context() {
        let (reg, backend, id, rx) = setup(full_caps());
        let dev = device_side(
            &reg,
            rx,
            expect_command("tap", json!({"x": 1, "y": 2})),
            false,
            None,
            Some("element not interactable".into()),
        );
        let err = backend.tap(&id, TapTarget::Coord(1, 2)).unwrap_err();
        assert_eq!(err.code, ErrorCode::AdbError);
        assert!(err.message.contains("tap"));
        assert!(err.message.contains("bridge:MuMu"));
        assert!(err.message.contains("element not interactable"));
        dev.join().unwrap();
    }

    #[test]
    fn result_wait_timeout_maps_to_timeout_error() {
        let reg = BridgeRegistry::new();
        let (tx, _rx) = mpsc::unbounded_channel();
        let hello = Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: "MuMu".into(),
            android_version: "12".into(),
            capabilities: full_caps(),
        };
        let (id, _conn, _close) = reg.register(&hello, tx);
        let backend = AppBridgeBackend::with_timeout(Arc::clone(&reg), Duration::from_millis(50));
        let err = backend.tap(&id, TapTarget::Coord(1, 2)).unwrap_err();
        assert_eq!(err.code, ErrorCode::Timeout);
        assert!(err.message.contains("tap") && err.message.contains("bridge:MuMu"));
    }

    #[test]
    fn offline_or_unknown_device_errors() {
        let (reg, backend, id, _rx) = setup(full_caps());
        let (_id, conn, _close) = {
            // 重新注册拿代际再注销 → 离线
            let hello = Hello {
                pairing_code: None,
                token: Some("t".into()),
                device_name: "MuMu".into(),
                android_version: "12".into(),
                capabilities: full_caps(),
            };
            let (tx2, _rx2) = mpsc::unbounded_channel();
            reg.register(&hello, tx2)
        };
        reg.unregister(&id, conn);
        let err = backend.tap(&id, TapTarget::Coord(1, 2)).unwrap_err();
        assert_eq!(err.code, ErrorCode::DeviceOffline);

        let err = backend
            .tap("bridge:ghost", TapTarget::Coord(1, 2))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeviceNotFound);
    }

    // ---- 能力降级（5.2） ----

    #[test]
    fn missing_capability_rejected_before_dispatch() {
        // 未上报 screenshot 能力：NOT_SUPPORTED 且不下发帧
        let caps = full_caps()
            .into_iter()
            .filter(|c| *c != Capability::Screenshot)
            .collect();
        let (_reg, backend, id, mut rx) = setup(caps);
        let dir = tempfile::tempdir().unwrap();
        let err = backend
            .screenshot(&id, &dir.path().join("x.png"))
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(
            err.message.contains("screenshot"),
            "消息含能力名: {}",
            err.message
        );
        assert!(
            err.message.contains("bridge:MuMu"),
            "消息含设备名: {}",
            err.message
        );
        assert!(rx.try_recv().is_err(), "能力缺失不得下发帧");
    }

    #[test]
    fn missing_script_capability_rejected() {
        let caps = full_caps()
            .into_iter()
            .filter(|c| *c != Capability::Script)
            .collect();
        let (_reg, backend, id, mut rx) = setup(caps);
        let err = backend.script(&id, "mobile.tap(1,2)").unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains("script") && err.message.contains("bridge:MuMu"));
        assert!(rx.try_recv().is_err(), "能力缺失不得下发帧");
    }

    #[test]
    fn unsupported_methods_return_structured_not_supported() {
        let (_reg, backend, id, mut rx) = setup(full_caps());
        let cases: Vec<(ErrorBody, &str)> = vec![
            (backend.stop(&id, "com.x").unwrap_err(), "stop"),
            (backend.logcat(&id, 10, None, None).unwrap_err(), "logcat"),
            (
                backend.shell(&id, &["getprop".into()]).unwrap_err(),
                "shell",
            ),
            (backend.connect("192.168.1.2:5555").unwrap_err(), "connect"),
        ];
        for (err, cap) in cases {
            assert_eq!(err.code, ErrorCode::NotSupported, "{cap}");
            assert!(
                err.message.contains(cap),
                "{cap} 消息含能力名: {}",
                err.message
            );
        }
        assert!(rx.try_recv().is_err(), "不支持的能力不得下发帧");
    }

    #[test]
    fn tap_ref_target_not_supported_at_backend() {
        let (_reg, backend, id, _rx) = setup(full_caps());
        let err = backend.tap(&id, TapTarget::Ref("@e1".into())).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotSupported);
    }

    #[test]
    fn kind_and_devices_from_registry() {
        let (_reg, backend, id, _rx) = setup(full_caps());
        assert_eq!(backend.kind(), BackendKind::AppBridge);
        let records = backend.devices().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, id);
        assert_eq!(records[0].kind, BackendKind::AppBridge);
    }
}
