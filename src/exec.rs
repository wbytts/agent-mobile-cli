//! 命令执行器：daemon 内把 CLI 命令解析结果路由到设备后端执行（design.md 架构节、决策 2/10）。
//!
//! daemon 侧持有配置、惰性初始化的 ADB 后端与快照引用缓存（决策 4：引用表存 daemon 内存，
//! 每次 snapshot 全量刷新，daemon 重启失效）。

use crate::backend::adb::AdbBackend;
use crate::backend::bridge::AppBridgeBackend;
use crate::backend::{BResult, Backend, BackendKind, DeviceRecord, DeviceState, TapTarget};
use crate::cli::Command;
use crate::config::Config;
use crate::output::{ErrorBody, Output};
use crate::ui::ElemRef;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct DaemonState {
    config: Config,
    backend: Mutex<Option<Arc<AdbBackend>>>,
    /// 快照引用缓存：设备 ID → 最近一次 snapshot 的元素引用表
    ref_cache: Mutex<HashMap<String, Vec<ElemRef>>>,
    /// 桥接设备注册表（WS 服务与 executor 共享，design.md 决策 1）
    bridge: Arc<crate::daemon::registry::BridgeRegistry>,
    /// 桥接后端实例（持有 BridgeRegistry 句柄，按设备记录路由，design.md 决策 10）
    bridge_backend: Arc<AppBridgeBackend>,
    /// 公网代理后端（配置 proxy 段时实时查询代理服务，mobile-debug-proxy 决策 2/11）
    proxy_backend: Arc<crate::backend::proxy::ProxyBackend>,
    /// 配对认证状态（WS hello 与 HTTP 管理端点共享，design.md 决策 8/14）
    pairing: Arc<crate::daemon::pair::Pairing>,
}

impl DaemonState {
    pub fn new(config: Config) -> Arc<Self> {
        Self::new_in(config, &Config::dir())
    }

    /// 可注入配置目录的构造（测试用临时目录，避免污染用户 tokens.json）。
    pub fn new_in(config: Config, dir: &Path) -> Arc<Self> {
        let bridge = crate::daemon::registry::BridgeRegistry::new();
        Arc::new(Self {
            backend: Mutex::new(None),
            ref_cache: Mutex::new(HashMap::new()),
            bridge_backend: Arc::new(AppBridgeBackend::new(Arc::clone(&bridge))),
            proxy_backend: Arc::new(crate::backend::proxy::ProxyBackend::new(
                config.proxy.as_ref(),
            )),
            bridge,
            pairing: Arc::new(crate::daemon::pair::Pairing::new(dir)),
            config,
        })
    }

    pub fn bridge(&self) -> &Arc<crate::daemon::registry::BridgeRegistry> {
        &self.bridge
    }

    pub fn pairing(&self) -> &Arc<crate::daemon::pair::Pairing> {
        &self.pairing
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// 惰性初始化 ADB 后端；每次未初始化时重试探测（adb 可能后装）。
    fn backend(&self) -> Result<Arc<AdbBackend>, ErrorBody> {
        let mut guard = self.backend.lock();
        if let Some(b) = guard.as_ref() {
            return Ok(b.clone());
        }
        let adb = crate::adb::Adb::locate(&self.config)?;
        let backend = Arc::new(AdbBackend::new(adb));
        *guard = Some(backend.clone());
        Ok(backend)
    }

    /// 目标设备解析：合并 adb、桥接与代理设备的在线集合统一解析。
    /// 代理查询失败时：目标显式指向 proxy: 前缀则返回原错误，否则降级为空集
    /// （代理服务临时不可达不阻断本地设备解析）。
    fn resolve(&self, selector: Option<&str>) -> Result<DeviceRecord, ErrorBody> {
        let adb_devices = self.backend().and_then(|b| b.devices());
        let mut proxy_error = None;
        let proxy_devices = match self.proxy_devices() {
            Ok(devices) => devices,
            Err(e) => {
                let target = selector.or(self.config.default_device.as_deref());
                if target.is_some_and(|t| t.starts_with(crate::backend::PROXY_ID_PREFIX)) {
                    return Err(e);
                }
                proxy_error = Some(e);
                Vec::new()
            }
        };
        let mut non_adb = self.bridge.device_records();
        non_adb.extend(proxy_devices);
        resolve_merged(
            selector,
            self.config.default_device.as_deref(),
            adb_devices,
            non_adb,
            proxy_error,
        )
    }

    /// 代理设备实时查询；未配置代理服务时返回空集（不视为错误）。
    fn proxy_devices(&self) -> BResult<Vec<DeviceRecord>> {
        if self.config.proxy.is_none() {
            return Ok(Vec::new());
        }
        self.proxy_backend.devices()
    }

    /// 按设备记录的后端类型选择后端实例（多后端路由，design.md 决策 10）。
    fn backend_for(&self, dev: &DeviceRecord) -> Result<Arc<dyn Backend>, ErrorBody> {
        let backend: Arc<dyn Backend> = match dev.kind {
            BackendKind::Adb => self.backend()?,
            BackendKind::AppBridge => Arc::clone(&self.bridge_backend) as Arc<dyn Backend>,
            BackendKind::Proxy => Arc::clone(&self.proxy_backend) as Arc<dyn Backend>,
        };
        debug_assert_eq!(backend.kind(), dev.kind, "后端实例与设备记录的后端类型一致");
        Ok(backend)
    }

    pub fn execute(&self, command: &Command, cwd: &Path) -> Output {
        match self.run(command, cwd) {
            Ok(v) => Output::success(v),
            Err(e) => e.into(),
        }
    }

    fn run(&self, command: &Command, cwd: &Path) -> Result<Value, ErrorBody> {
        match command {
            Command::Devices => {
                let mut non_adb = self.bridge.device_records();
                // 代理设备实时查询；失败时降级列出其余设备并附错误说明（spec：认证失败须显式可见）
                let proxy_error = match self.proxy_devices() {
                    Ok(devices) => {
                        non_adb.extend(devices);
                        None
                    }
                    Err(e) => Some(e.message),
                };
                match self.backend().and_then(|b| b.devices()) {
                    Ok(mut devices) => {
                        devices.extend(non_adb);
                        let mut out = json!({ "devices": devices });
                        if let Some(msg) = proxy_error {
                            out["proxy_error"] = json!(msg);
                        }
                        Ok(out)
                    }
                    // adb 不可用且无其他设备时保持原错误；有其他设备时降级列出并附 adb 错误
                    Err(e) if non_adb.is_empty() && proxy_error.is_none() => Err(e),
                    Err(e) => {
                        let mut out = json!({
                            "devices": non_adb,
                            "adb_error": e.message,
                        });
                        if let Some(msg) = proxy_error {
                            out["proxy_error"] = json!(msg);
                        }
                        Ok(out)
                    }
                }
            }
            Command::Connect { target } => {
                let msg = self.backend()?.connect(target)?;
                Ok(json!({ "connected": target, "message": msg }))
            }
            Command::Snapshot { device, full } => {
                let dev = self.resolve(device.as_deref())?;
                let snap = self.backend_for(&dev)?.snapshot(&dev.id, *full)?;
                self.ref_cache
                    .lock()
                    .insert(dev.id.clone(), snap.refs.clone());
                Ok(json!({
                    "device": dev.id,
                    "tree": snap.tree,
                    "refs": snap.refs,
                }))
            }
            Command::Tap { target, y, device } => {
                let dev = self.resolve(device.as_deref())?;
                let coord = self.tap_coord(&dev.id, target, *y)?;
                self.backend_for(&dev)?
                    .tap(&dev.id, TapTarget::Coord(coord.0, coord.1))?;
                Ok(json!({ "device": dev.id, "tapped": coord }))
            }
            Command::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration,
                device,
            } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend_for(&dev)?
                    .swipe(&dev.id, *x1, *y1, *x2, *y2, *duration)?;
                Ok(json!({ "device": dev.id }))
            }
            Command::Input { text, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend_for(&dev)?.input_text(&dev.id, text)?;
                Ok(json!({ "device": dev.id }))
            }
            Command::Key { key, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend_for(&dev)?.key(&dev.id, key)?;
                Ok(json!({ "device": dev.id }))
            }
            Command::Screenshot { out, device } => {
                let dev = self.resolve(device.as_deref())?;
                let path = resolve_out(cwd, out);
                let saved = self.backend_for(&dev)?.screenshot(&dev.id, &path)?;
                Ok(json!({ "device": dev.id, "path": saved }))
            }
            Command::Apps {
                filter,
                all,
                device,
            } => {
                let dev = self.resolve(device.as_deref())?;
                let apps = self
                    .backend_for(&dev)?
                    .apps(&dev.id, filter.as_deref(), *all)?;
                Ok(json!({ "device": dev.id, "apps": apps }))
            }
            Command::Launch { package, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend_for(&dev)?.launch(&dev.id, package)?;
                Ok(json!({ "device": dev.id, "launched": package }))
            }
            Command::Stop { package, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend_for(&dev)?.stop(&dev.id, package)?;
                Ok(json!({ "device": dev.id, "stopped": package }))
            }
            Command::Logcat {
                lines,
                tag,
                level,
                device,
            } => {
                let dev = self.resolve(device.as_deref())?;
                let out = self.backend_for(&dev)?.logcat(
                    &dev.id,
                    *lines,
                    tag.as_deref(),
                    level.as_deref(),
                )?;
                Ok(json!({ "device": dev.id, "logcat": out }))
            }
            Command::Shell { cmd, device } => {
                if cmd
                    .iter()
                    .any(|a| a == "--device" || a.starts_with("--device="))
                {
                    return Err(ErrorBody::usage(
                        "shell 的 --device 等选项需写在命令之前：agent-mobile-cli shell --device <serial> <cmd...>；如需向设备命令传递字面 --device 参数，请用 sh -c '...' 包裹",
                    ));
                }
                let dev = self.resolve(device.as_deref())?;
                let out = self.backend_for(&dev)?.shell(&dev.id, cmd)?;
                Ok(json!({
                    "device": dev.id,
                    "stdout": out.stdout,
                    "stderr": out.stderr,
                    "exit_code": out.exit_code,
                }))
            }
            Command::Script {
                source,
                stdin,
                device,
            } => {
                let code = match stdin {
                    Some(s) => s.clone(),
                    None if source == "-" => {
                        return Err(ErrorBody::usage(
                            "script - 需从 stdin 读取脚本内容（经 CLI 发起时自动随请求携带）",
                        ));
                    }
                    None => std::fs::read_to_string(resolve_out(cwd, source)).map_err(|e| {
                        ErrorBody::io_error(format!("读取脚本文件 {source} 失败: {e}"))
                    })?,
                };
                let dev = self.resolve(device.as_deref())?;
                let value = self.backend_for(&dev)?.script(&dev.id, &code)?;
                Ok(json!({ "device": dev.id, "result": value }))
            }
            Command::Daemon
            | Command::DaemonStatus
            | Command::DaemonRestart
            | Command::DaemonStop => Err(ErrorBody::usage(
                "daemon 生命周期命令只能在本机直接执行，不经 /cmd 转发",
            )),
            Command::Pair { .. } => Err(ErrorBody::usage(
                "pair 是本机管理命令，不经 /cmd 转发（请在 CLI 侧直接执行）",
            )),
        }
    }

    /// tap 目标解析：`@eN` 查引用缓存取中心点；数字 + `--y` 为坐标。
    fn tap_coord(
        &self,
        device: &str,
        target: &str,
        y: Option<i32>,
    ) -> Result<(i32, i32), ErrorBody> {
        if let Some(id) = target.strip_prefix('@') {
            let cache = self.ref_cache.lock();
            let refs = cache.get(device).ok_or_else(|| {
                ErrorBody::usage(format!(
                    "设备 {device} 没有可用快照引用，请先执行 snapshot（引用仅对最近一次快照有效）"
                ))
            })?;
            let needle = format!("@{}", id);
            let r = refs.iter().find(|r| r.id == needle).ok_or_else(|| {
                ErrorBody::usage(format!(
                    "引用 {needle} 不在最近一次快照中（共 {} 个引用）",
                    refs.len()
                ))
            })?;
            return Ok(r.center);
        }
        let x: i32 = target
            .parse()
            .map_err(|_| ErrorBody::usage("tap 目标必须是 @eN 引用或 X 坐标（配合 --y 使用）"))?;
        let y = y.ok_or_else(|| ErrorBody::usage("tap 坐标形式需要同时提供 --y"))?;
        Ok((x, y))
    }
}

/// 代理错误替换决策（纯函数，便于确定性单测；FixReview IMPORTANT-3）：
/// 代理查询失败被降级为空集后，若全部来源均无在线设备且为隐式目标
/// （无 selector、无默认设备），返回代理原错误而非误导性的
/// 「没有在线设备」或 adb 探测失败；显式目标保留原解析错误（NOT_FOUND/候选列表更有用）。
fn resolve_merged(
    selector: Option<&str>,
    default: Option<&str>,
    adb_devices: Result<Vec<DeviceRecord>, ErrorBody>,
    non_adb: Vec<DeviceRecord>,
    proxy_error: Option<ErrorBody>,
) -> Result<DeviceRecord, ErrorBody> {
    let any_adb_online = adb_devices
        .as_ref()
        .map(|ds| ds.iter().any(|d| d.state == DeviceState::Online))
        .unwrap_or(false);
    let nothing_online = !any_adb_online && !non_adb.iter().any(|d| d.state == DeviceState::Online);
    let implicit = selector.is_none() && default.is_none();
    match resolve_with_bridge(selector, default, adb_devices, non_adb) {
        Err(e) if nothing_online && implicit => Err(proxy_error.unwrap_or(e)),
        other => other,
    }
}

/// 合并 adb 与桥接设备的在线集合做目标解析（纯函数，便于确定性单测）：
/// adb 不可用不阻断桥接设备解析；两侧均无在线设备时回传 adb 原始错误（保持既有行为）。
fn resolve_with_bridge(
    selector: Option<&str>,
    default: Option<&str>,
    adb_devices: Result<Vec<DeviceRecord>, ErrorBody>,
    bridge_devices: Vec<DeviceRecord>,
) -> Result<DeviceRecord, ErrorBody> {
    let mut online: Vec<DeviceRecord> = bridge_devices
        .into_iter()
        .filter(|d| d.state == DeviceState::Online)
        .collect();
    let adb_err = match adb_devices {
        Ok(devices) => {
            online.extend(
                devices
                    .into_iter()
                    .filter(|d| d.state == DeviceState::Online),
            );
            None
        }
        Err(e) => Some(e),
    };
    if online.is_empty() {
        if let Some(e) = adb_err {
            return Err(e);
        }
    }
    crate::backend::resolve_target(selector, default, &online)
}

/// 把 CLI 相对输出路径解析为以发起侧 cwd 为基准的绝对路径（自由函数，避免跨请求共享状态）。
fn resolve_out(cwd: &Path, out: &str) -> PathBuf {
    let p = Path::new(out);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_out_relative_uses_cwd() {
        let cwd = Path::new("/tmp/work");
        assert_eq!(
            resolve_out(cwd, "shot.png"),
            PathBuf::from("/tmp/work/shot.png")
        );
        assert_eq!(
            resolve_out(cwd, "/abs/shot.png"),
            PathBuf::from("/abs/shot.png")
        );
    }

    #[test]
    fn tap_ref_requires_snapshot() {
        let state = DaemonState::new(Config::default());
        let err = state.tap_coord("dev-1", "@e1", None).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::Usage);
        assert!(err.message.contains("snapshot"));
    }

    #[test]
    fn tap_coord_parse() {
        let state = DaemonState::new(Config::default());
        assert_eq!(state.tap_coord("d", "100", Some(200)).unwrap(), (100, 200));
        assert!(state.tap_coord("d", "100", None).is_err());
        assert!(state.tap_coord("d", "abc", None).is_err());
    }

    #[test]
    fn daemon_commands_rejected() {
        let state = DaemonState::new(Config::default());
        let out = state.execute(&Command::DaemonStatus, Path::new("/tmp"));
        assert!(!out.ok);
        assert_eq!(out.error.unwrap().code, crate::output::ErrorCode::Usage);
    }

    #[test]
    fn shell_rejects_trailing_device_flag() {
        // 防护在 backend 解析前触发，无需设备/adb
        let state = DaemonState::new(Config::default());
        for cmd in [
            vec!["getprop".to_owned(), "--device".to_owned(), "x".to_owned()],
            vec!["getprop".to_owned(), "--device=emu64a".to_owned()],
        ] {
            let out = state.execute(&Command::Shell { cmd, device: None }, Path::new("/tmp"));
            assert!(!out.ok);
            let err = out.error.unwrap();
            assert_eq!(err.code, crate::output::ErrorCode::Usage, "cmd 应被拦截");
            assert!(err.message.contains("sh -c"), "消息应含绕过指引");
        }
    }

    #[test]
    fn shell_allows_quoted_device_literal() {
        // 单 token（引号包裹）不触发防护；此时失败于 backend 不可用而非 USAGE
        let state = DaemonState::new(Config::default());
        let out = state.execute(
            &Command::Shell {
                cmd: vec!["sh -c 'echo --device'".to_owned()],
                device: None,
            },
            Path::new("/tmp"),
        );
        // 只对 error 存在时断言非 Usage（单设备环境下命令可能真实成功，不耦合 backend 结果）
        if let Some(e) = out.error {
            assert_ne!(e.code, crate::output::ErrorCode::Usage);
        }
    }

    #[test]
    fn devices_merges_bridge_registry_entries() {
        // 桥接设备已注册但 adb 后端不可用：仍应列出桥接设备（组 5 将完善合并语义）
        let state = DaemonState::new(Config::default());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let hello = crate::bridge_proto::Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: "MuMu".into(),
            android_version: "12".into(),
            capabilities: vec![crate::bridge_proto::Capability::Tap],
        };
        state.bridge().register(&hello, tx);
        let out = state.execute(&Command::Devices, Path::new("/tmp"));
        assert!(out.ok, "桥接设备在线时 devices 应成功: {out:?}");
        let devices = out.result.unwrap()["devices"].clone();
        let arr = devices.as_array().expect("devices 为数组");
        let bridge = arr
            .iter()
            .find(|d| d["id"] == "bridge:MuMu")
            .expect("应包含桥接设备");
        assert_eq!(bridge["connection"], "bridge");
        assert_eq!(bridge["state"], "online");
    }

    // ---- FixReview IMPORTANT-3：代理失败 + 无在线设备时暴露代理原错误 ----
    // （resolve_merged 纯函数测试，不依赖本机 adb/模拟器状态）

    fn proxy_err() -> ErrorBody {
        ErrorBody::proxy_error("代理服务认证失败")
    }

    #[test]
    fn resolve_隐式目标代理失败_返回代理错误() {
        // adb 不可用、无任何在线设备、隐式目标 → 暴露代理错误
        let adb_err = ErrorBody::io_error("adb not found");
        let err =
            resolve_merged(None, None, Err(adb_err), Vec::new(), Some(proxy_err())).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::ProxyError,
            "无任何在线设备且代理失败时应暴露代理错误，实际: {err:?}"
        );
    }

    #[test]
    fn resolve_显式目标代理失败_保留原解析错误() {
        // 显式 selector：即使代理失败也保留原解析错误（NOT_FOUND/候选列表更有用）
        let adb_err = ErrorBody::io_error("adb not found");
        let err = resolve_merged(
            Some("some-serial"),
            None,
            Err(adb_err),
            Vec::new(),
            Some(proxy_err()),
        )
        .unwrap_err();
        assert_ne!(
            err.code,
            ErrorCode::ProxyError,
            "显式目标不应被代理错误替换，实际: {err:?}"
        );
    }

    #[test]
    fn resolve_有在线设备时代理失败不影响解析() {
        // adb 有在线设备：正常解析该设备，代理错误被忽略
        let rec = resolve_merged(
            None,
            None,
            Ok(vec![adb_rec("dev-online")]),
            Vec::new(),
            Some(proxy_err()),
        )
        .unwrap();
        assert_eq!(rec.id, "dev-online");
    }

    // ---- 组 5：合并解析（resolve_with_bridge 纯函数） ----

    use crate::backend::{BackendKind, ConnectionKind, DeviceState};
    use crate::bridge_proto::{Capability, Hello, ResultMessage};
    use crate::output::ErrorCode;

    fn adb_rec(id: &str) -> DeviceRecord {
        DeviceRecord {
            id: id.into(),
            kind: BackendKind::Adb,
            model: None,
            state: DeviceState::Online,
            connection: ConnectionKind::Network,
        }
    }

    fn bridge_rec(id: &str, state: DeviceState) -> DeviceRecord {
        DeviceRecord {
            id: id.into(),
            kind: BackendKind::AppBridge,
            model: None,
            state,
            connection: ConnectionKind::Bridge,
        }
    }

    #[test]
    fn resolve_bridge_selector_when_adb_unavailable() {
        let got = resolve_with_bridge(
            Some("bridge:MuMu"),
            None,
            Err(ErrorBody::adb_not_found("no adb")),
            vec![bridge_rec("bridge:MuMu", DeviceState::Online)],
        )
        .unwrap();
        assert_eq!(got.id, "bridge:MuMu");
        assert_eq!(got.kind, BackendKind::AppBridge);
    }

    #[test]
    fn resolve_single_online_bridge_without_selector_when_adb_down() {
        let got = resolve_with_bridge(
            None,
            None,
            Err(ErrorBody::adb_not_found("no adb")),
            vec![bridge_rec("bridge:MuMu", DeviceState::Online)],
        )
        .unwrap();
        assert_eq!(got.id, "bridge:MuMu");
    }

    #[test]
    fn resolve_ambiguous_when_adb_and_bridge_online() {
        let err = resolve_with_bridge(
            None,
            None,
            Ok(vec![adb_rec("a:5555")]),
            vec![bridge_rec("bridge:MuMu", DeviceState::Online)],
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeviceAmbiguous);
    }

    #[test]
    fn resolve_selector_miss_lists_bridge_candidates() {
        let err = resolve_with_bridge(
            Some("bridge:ghost"),
            None,
            Err(ErrorBody::adb_not_found("no adb")),
            vec![bridge_rec("bridge:MuMu", DeviceState::Online)],
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeviceNotFound);
        assert!(err.message.contains("bridge:MuMu"));
    }

    #[test]
    fn resolve_adb_error_passthrough_when_nothing_online() {
        let err = resolve_with_bridge(None, None, Err(ErrorBody::adb_not_found("no adb")), vec![])
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::AdbNotFound);
        // 桥接设备离线同样视为无在线设备 → 回传 adb 原始错误
        let err = resolve_with_bridge(
            None,
            None,
            Err(ErrorBody::adb_not_found("no adb")),
            vec![bridge_rec("bridge:MuMu", DeviceState::Offline)],
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::AdbNotFound);
    }

    #[test]
    fn resolve_explicit_selector_requires_full_bridge_id() {
        // 与 adb 设备歧义规则一致：--device 须写完整 id（bridge:<name>），裸名不匹配
        let err = resolve_with_bridge(
            Some("MuMu"),
            None,
            Err(ErrorBody::adb_not_found("no adb")),
            vec![bridge_rec("bridge:MuMu", DeviceState::Online)],
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::DeviceNotFound);
    }

    // ---- 组 5：executor 多后端路由 ----

    fn full_bridge_caps() -> Vec<Capability> {
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

    fn register_bridge(
        state: &DaemonState,
        name: &str,
    ) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let hello = Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: name.into(),
            android_version: "12".into(),
            capabilities: full_bridge_caps(),
        };
        let (id, _conn, _close) = state.bridge().register(&hello, tx);
        (id, rx)
    }

    /// 模拟桥接设备侧：收一帧交给 check，再按 id 回传 result。
    fn bridge_device_side(
        state: &Arc<DaemonState>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<String>,
        check: impl FnOnce(&Value) + Send + 'static,
        result: Option<Value>,
    ) -> std::thread::JoinHandle<()> {
        let reg = Arc::clone(state.bridge());
        std::thread::spawn(move || {
            let frame = rx.blocking_recv().expect("应收到下发帧");
            let v: Value = serde_json::from_str(&frame).unwrap();
            check(&v);
            reg.complete(ResultMessage {
                id: v["id"].as_str().unwrap().to_string(),
                ok: true,
                result,
                error: None,
            });
        })
    }

    #[test]
    fn tap_routes_to_bridge_backend() {
        let state = DaemonState::new(Config::default());
        let (id, rx) = register_bridge(&state, "MuMu");
        let dev = bridge_device_side(
            &state,
            rx,
            |v| {
                assert_eq!(v["method"], "tap");
                assert_eq!(v["params"], json!({"x": 10, "y": 20}));
            },
            None,
        );
        let out = state.execute(
            &Command::Tap {
                target: "10".into(),
                y: Some(20),
                device: Some(id),
            },
            Path::new("/tmp"),
        );
        assert!(out.ok, "桥接 tap 应成功: {out:?}");
        let r = out.result.unwrap();
        assert_eq!(r["device"], "bridge:MuMu");
        assert_eq!(r["tapped"], json!([10, 20]));
        dev.join().unwrap();
    }

    #[test]
    fn shell_and_stop_on_bridge_device_not_supported() {
        let state = DaemonState::new(Config::default());
        let (id, _rx) = register_bridge(&state, "MuMu");
        let out = state.execute(
            &Command::Shell {
                cmd: vec!["getprop".into()],
                device: Some(id.clone()),
            },
            Path::new("/tmp"),
        );
        let err = out.error.unwrap();
        assert_eq!(err.code, ErrorCode::NotSupported);
        assert!(err.message.contains("shell") && err.message.contains("bridge:MuMu"));
        let out = state.execute(
            &Command::Stop {
                package: "com.x".into(),
                device: Some(id),
            },
            Path::new("/tmp"),
        );
        assert_eq!(out.error.unwrap().code, ErrorCode::NotSupported);
    }

    #[test]
    fn script_routes_stdin_content_to_bridge() {
        let state = DaemonState::new(Config::default());
        let (id, rx) = register_bridge(&state, "MuMu");
        let dev = bridge_device_side(
            &state,
            rx,
            |v| {
                assert_eq!(v["type"], "script");
                assert_eq!(v["source"], "mobile.tap(1,2)");
            },
            Some(json!({"tapped": true})),
        );
        let out = state.execute(
            &Command::Script {
                source: "-".into(),
                stdin: Some("mobile.tap(1,2)".into()),
                device: Some(id),
            },
            Path::new("/tmp"),
        );
        assert!(out.ok, "桥接 script 应成功: {out:?}");
        let r = out.result.unwrap();
        assert_eq!(r["device"], "bridge:MuMu");
        assert_eq!(r["result"], json!({"tapped": true}));
        dev.join().unwrap();
    }

    #[test]
    fn script_reads_file_relative_to_cwd() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s.js"), "mobile.key('home')").unwrap();
        let state = DaemonState::new(Config::default());
        let (id, rx) = register_bridge(&state, "MuMu");
        let dev = bridge_device_side(
            &state,
            rx,
            |v| assert_eq!(v["source"], "mobile.key('home')"),
            None,
        );
        let out = state.execute(
            &Command::Script {
                source: "s.js".into(),
                stdin: None,
                device: Some(id),
            },
            dir.path(),
        );
        assert!(out.ok, "脚本文件应按 cwd 解析读取: {out:?}");
        dev.join().unwrap();
    }

    #[test]
    fn script_source_errors() {
        let state = DaemonState::new(Config::default());
        let (id, _rx) = register_bridge(&state, "MuMu");
        // "-" 但无 stdin 内容（转发层未注入）→ Usage
        let out = state.execute(
            &Command::Script {
                source: "-".into(),
                stdin: None,
                device: Some(id.clone()),
            },
            Path::new("/tmp"),
        );
        assert_eq!(out.error.unwrap().code, ErrorCode::Usage);
        // 文件不存在 → IoError
        let out = state.execute(
            &Command::Script {
                source: "no-such-file.js".into(),
                stdin: None,
                device: Some(id),
            },
            Path::new("/tmp"),
        );
        assert_eq!(out.error.unwrap().code, ErrorCode::IoError);
    }
}
