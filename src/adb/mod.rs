//! adb 可执行文件封装：探测链与子进程调用（design.md 决策 2/3）。

use crate::backend::{BResult, DeviceRecord};
use crate::config::Config;
use crate::output::ErrorBody;
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Adb {
    pub path: PathBuf,
}

impl Adb {
    /// 探测链：配置 adb_path → ANDROID_HOME/ANDROID_SDK_ROOT → PATH → 平台常见路径。
    pub fn locate(config: &Config) -> BResult<Adb> {
        let _ = config;
        unimplemented!("任务 3.1 实现")
    }

    /// 解析 `adb devices -l` 输出为设备记录。
    pub fn devices(&self) -> BResult<Vec<DeviceRecord>> {
        let _ = self;
        unimplemented!("任务 3.2 实现")
    }
}

/// 解析 `adb devices -l` 文本（纯函数，便于单测）。
pub fn parse_devices(text: &str) -> Vec<DeviceRecord> {
    let _ = text;
    unimplemented!("任务 3.2 实现")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "List of devices attached\n127.0.0.1:5555\tdevice product:23116PN5BC model:23116PN5BC device:23116PN5BC transport_id:4\nemulator-5554\toffline transport_id:2\n0b3c1234\tunauthorized usb:1-2 transport_id:3\n";

    #[test]
    fn parses_devices_states_and_model() {
        let devices = parse_devices(SAMPLE);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[0].id, "127.0.0.1:5555");
        assert_eq!(devices[0].state, crate::backend::DeviceState::Online);
        assert_eq!(devices[0].model.as_deref(), Some("23116PN5BC"));
        assert_eq!(
            devices[0].connection,
            crate::backend::ConnectionKind::Network
        );
        assert_eq!(devices[1].state, crate::backend::DeviceState::Offline);
        assert_eq!(devices[2].state, crate::backend::DeviceState::Unauthorized);
        assert_eq!(devices[2].connection, crate::backend::ConnectionKind::Usb);
    }

    #[test]
    fn parses_empty_list() {
        let devices = parse_devices("List of devices attached\n\n");
        assert!(devices.is_empty());
    }
}
