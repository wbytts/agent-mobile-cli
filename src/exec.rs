//! 命令执行器：daemon 内把 CLI 命令解析结果路由到设备后端执行（design.md 架构节、决策 2/10）。
//!
//! daemon 侧持有配置、惰性初始化的 ADB 后端与快照引用缓存（决策 4：引用表存 daemon 内存，
//! 每次 snapshot 全量刷新，daemon 重启失效）。

use crate::backend::adb::AdbBackend;
use crate::backend::{Backend, DeviceRecord, TapTarget};
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
    /// 配对认证状态（WS hello 与 HTTP 管理端点共享，design.md 决策 8/14）
    pairing: Arc<crate::daemon::pair::Pairing>,
}

impl DaemonState {
    pub fn new(config: Config) -> Arc<Self> {
        Self::new_in(config, &Config::dir())
    }

    /// 可注入配置目录的构造（测试用临时目录，避免污染用户 tokens.json）。
    pub fn new_in(config: Config, dir: &Path) -> Arc<Self> {
        Arc::new(Self {
            config,
            backend: Mutex::new(None),
            ref_cache: Mutex::new(HashMap::new()),
            bridge: crate::daemon::registry::BridgeRegistry::new(),
            pairing: Arc::new(crate::daemon::pair::Pairing::new(dir)),
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

    fn resolve(&self, selector: Option<&str>) -> Result<DeviceRecord, ErrorBody> {
        self.backend()?.resolve_device(selector, &self.config)
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
                let bridge_devices = self.bridge.device_records();
                match self.backend().and_then(|b| b.devices()) {
                    Ok(mut devices) => {
                        devices.extend(bridge_devices);
                        Ok(json!({ "devices": devices }))
                    }
                    // adb 不可用且无桥接设备时保持原错误；有桥接设备时降级列出并附 adb 错误
                    Err(e) if bridge_devices.is_empty() => Err(e),
                    Err(e) => Ok(json!({
                        "devices": bridge_devices,
                        "adb_error": e.message,
                    })),
                }
            }
            Command::Connect { target } => {
                let msg = self.backend()?.connect(target)?;
                Ok(json!({ "connected": target, "message": msg }))
            }
            Command::Snapshot { device, full } => {
                let dev = self.resolve(device.as_deref())?;
                let snap = self.backend()?.snapshot(&dev.id, *full)?;
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
                self.backend()?
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
                self.backend()?
                    .swipe(&dev.id, *x1, *y1, *x2, *y2, *duration)?;
                Ok(json!({ "device": dev.id }))
            }
            Command::Input { text, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend()?.input_text(&dev.id, text)?;
                Ok(json!({ "device": dev.id }))
            }
            Command::Key { key, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend()?.key(&dev.id, key)?;
                Ok(json!({ "device": dev.id }))
            }
            Command::Screenshot { out, device } => {
                let dev = self.resolve(device.as_deref())?;
                let path = resolve_out(cwd, out);
                let saved = self.backend()?.screenshot(&dev.id, &path)?;
                Ok(json!({ "device": dev.id, "path": saved }))
            }
            Command::Apps {
                filter,
                all,
                device,
            } => {
                let dev = self.resolve(device.as_deref())?;
                let apps = self.backend()?.apps(&dev.id, filter.as_deref(), *all)?;
                Ok(json!({ "device": dev.id, "apps": apps }))
            }
            Command::Launch { package, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend()?.launch(&dev.id, package)?;
                Ok(json!({ "device": dev.id, "launched": package }))
            }
            Command::Stop { package, device } => {
                let dev = self.resolve(device.as_deref())?;
                self.backend()?.stop(&dev.id, package)?;
                Ok(json!({ "device": dev.id, "stopped": package }))
            }
            Command::Logcat {
                lines,
                tag,
                level,
                device,
            } => {
                let dev = self.resolve(device.as_deref())?;
                let out =
                    self.backend()?
                        .logcat(&dev.id, *lines, tag.as_deref(), level.as_deref())?;
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
                let out = self.backend()?.shell(&dev.id, cmd)?;
                Ok(json!({
                    "device": dev.id,
                    "stdout": out.stdout,
                    "stderr": out.stderr,
                    "exit_code": out.exit_code,
                }))
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
}
