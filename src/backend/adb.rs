//! ADB 直连后端：实现 Backend trait，预留 app-bridge 路由（design.md 决策 10）。
// TODO(接线): CLI/daemon 命令接线后移除本行（参考 ui.rs 约定，避免组 2-4 接线前 dead_code 警告）。
#![allow(dead_code)]
use super::{resolve_target, BResult, Backend, BackendKind, DeviceRecord, ShellResult, TapTarget};
use crate::adb::Adb;
use crate::config::Config;
use crate::output::ErrorBody;
use std::path::{Path, PathBuf};

/// ADB 后端：所有设备操作经 adb 子进程下发，显式 `-s <serial>` 选定设备。
pub struct AdbBackend {
    adb: Adb,
}

impl AdbBackend {
    pub fn new(adb: Adb) -> Self {
        Self { adb }
    }

    /// 目标设备解析接线：--device 显式指定 → 配置默认 → 仅一台在线 → 歧义错误。
    pub fn resolve_device(&self, selector: Option<&str>, config: &Config) -> BResult<DeviceRecord> {
        let online: Vec<DeviceRecord> = self
            .devices()?
            .into_iter()
            .filter(|d| d.state == super::DeviceState::Online)
            .collect();
        resolve_target(selector, config.default_device.as_deref(), &online)
    }

    /// 组 4 统一占位：未实现的设备操作。
    fn pending() -> ErrorBody {
        ErrorBody::not_supported("组 4 实现")
    }
}

impl Backend for AdbBackend {
    fn kind(&self) -> BackendKind {
        BackendKind::Adb
    }

    fn devices(&self) -> BResult<Vec<DeviceRecord>> {
        self.adb.devices()
    }

    fn connect(&self, target: &str) -> BResult<String> {
        self.adb.connect(target)
    }

    // TODO(组4): snapshot 经 uiautomator dump / app-bridge 实现
    fn snapshot(&self, _device: &str, _full: bool) -> BResult<crate::ui::Snapshot> {
        Err(Self::pending())
    }

    // TODO(组4): tap 经 input tap / 坐标或 ref 定位实现
    fn tap(&self, _device: &str, _target: TapTarget) -> BResult<()> {
        Err(Self::pending())
    }

    // TODO(组4): swipe 经 input swipe 实现
    fn swipe(
        &self,
        _device: &str,
        _x1: i32,
        _y1: i32,
        _x2: i32,
        _y2: i32,
        _duration_ms: u32,
    ) -> BResult<()> {
        Err(Self::pending())
    }

    // TODO(组4): input_text 经 input text / 剪贴板方案实现
    fn input_text(&self, _device: &str, _text: &str) -> BResult<()> {
        Err(Self::pending())
    }

    // TODO(组4): key 经 input keyevent 实现
    fn key(&self, _device: &str, _key: &str) -> BResult<()> {
        Err(Self::pending())
    }

    // TODO(组4): screenshot 经 screencap + adb pull 实现
    fn screenshot(&self, _device: &str, _out: &Path) -> BResult<PathBuf> {
        Err(Self::pending())
    }

    // TODO(组4): apps 经 pm list packages 实现
    fn apps(&self, _device: &str, _filter: Option<&str>, _all: bool) -> BResult<Vec<String>> {
        Err(Self::pending())
    }

    // TODO(组4): launch 经 monkey / am start 实现
    fn launch(&self, _device: &str, _package: &str) -> BResult<()> {
        Err(Self::pending())
    }

    // TODO(组4): stop 经 am force-stop 实现
    fn stop(&self, _device: &str, _package: &str) -> BResult<()> {
        Err(Self::pending())
    }

    // TODO(组4): logcat 经 adb logcat -d 实现
    fn logcat(
        &self,
        _device: &str,
        _lines: u32,
        _tag: Option<&str>,
        _level: Option<&str>,
    ) -> BResult<String> {
        Err(Self::pending())
    }

    // TODO(组4): shell 经 adb -s <serial> shell 实现（显式 -s）
    fn shell(&self, _device: &str, _cmd: &[String]) -> BResult<ShellResult> {
        Err(Self::pending())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn backend() -> AdbBackend {
        // 伪路径：类型层测试不触发进程执行。
        AdbBackend::new(Adb {
            path: PathBuf::from("/nonexistent/adb"),
        })
    }

    #[test]
    fn kind_is_adb() {
        assert_eq!(backend().kind(), BackendKind::Adb);
    }

    #[test]
    fn unimplemented_ops_return_not_supported() {
        let b = backend();
        let cases: Vec<ErrorBody> = vec![
            b.snapshot("dev", false).unwrap_err(),
            b.tap("dev", TapTarget::Coord(1, 2)).unwrap_err(),
            b.swipe("dev", 0, 0, 1, 1, 100).unwrap_err(),
            b.input_text("dev", "hi").unwrap_err(),
            b.key("dev", "HOME").unwrap_err(),
            b.screenshot("dev", Path::new("/tmp/x.png")).unwrap_err(),
            b.apps("dev", None, false).unwrap_err(),
            b.launch("dev", "pkg").unwrap_err(),
            b.stop("dev", "pkg").unwrap_err(),
            b.logcat("dev", 10, None, None).unwrap_err(),
            b.shell("dev", &["ls".to_string()]).unwrap_err(),
        ];
        for e in cases {
            assert_eq!(e.code, crate::output::ErrorCode::NotSupported);
            assert!(e.message.contains("组 4"));
        }
    }

    #[test]
    fn missing_adb_binary_surfaces_io_error() {
        // 伪路径下 devices/connect 应在 spawn 阶段失败为 IoError，而非 panic。
        let b = backend();
        let e = b.devices().unwrap_err();
        assert_eq!(e.code, crate::output::ErrorCode::IoError);
        let e = b.connect("127.0.0.1:5555").unwrap_err();
        assert_eq!(e.code, crate::output::ErrorCode::IoError);
    }
}
