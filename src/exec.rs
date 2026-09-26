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
    /// 命令发起侧 CLI 的工作目录（相对路径输出以其为基准）
    cwd: Mutex<PathBuf>,
    backend: Mutex<Option<Arc<AdbBackend>>>,
    /// 快照引用缓存：设备 ID → 最近一次 snapshot 的元素引用表
    ref_cache: Mutex<HashMap<String, Vec<ElemRef>>>,
}

impl DaemonState {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config,
            cwd: Mutex::new(PathBuf::from(".")),
            backend: Mutex::new(None),
            ref_cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_cwd(&self, cwd: PathBuf) {
        *self.cwd.lock() = cwd;
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
    /// 把 CLI 相对输出路径解析为以发起侧 cwd 为基准的绝对路径。
    fn resolve_out(&self, out: &str) -> PathBuf {
        let p = Path::new(out);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.cwd.lock().join(p)
        }
    }

    pub fn execute(&self, command: &Command) -> Output {
        match self.run(command) {
            Ok(v) => Output::success(v),
            Err(e) => e.into(),
        }
    }

    fn run(&self, command: &Command) -> Result<Value, ErrorBody> {
        match command {
            Command::Devices => {
                let devices = self.backend()?.devices()?;
                Ok(json!({ "devices": devices }))
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
                let path = self.resolve_out(out);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_out_relative_uses_cwd() {
        let state = DaemonState::new(Config::default());
        state.set_cwd(PathBuf::from("/tmp/work"));
        assert_eq!(
            state.resolve_out("shot.png"),
            PathBuf::from("/tmp/work/shot.png")
        );
        assert_eq!(
            state.resolve_out("/abs/shot.png"),
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
        let out = state.execute(&Command::DaemonStatus);
        assert!(!out.ok);
        assert_eq!(out.error.unwrap().code, crate::output::ErrorCode::Usage);
    }
}
