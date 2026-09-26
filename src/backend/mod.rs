//! 后端抽象层契约：统一设备模型与后端接口（design.md 决策 10）。
//! 本文件只定义类型与接口，实现由 adb / app-bridge 后端模块提供。

use crate::output::ErrorBody;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

pub type BResult<T> = Result<T, ErrorBody>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    Adb,
    AppBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceState {
    Online,
    Offline,
    Unauthorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionKind {
    Usb,
    Network,
    Bridge,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceRecord {
    pub id: String,
    pub kind: BackendKind,
    pub model: Option<String>,
    pub state: DeviceState,
    pub connection: ConnectionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TapTarget {
    Coord(i32, i32),
    Ref(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct ShellResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// 统一后端接口：控制命令经该接口路由到目标设备所属后端。
pub trait Backend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn devices(&self) -> BResult<Vec<DeviceRecord>>;
    fn connect(&self, target: &str) -> BResult<String>;
    fn snapshot(&self, device: &str, full: bool) -> BResult<crate::ui::Snapshot>;
    fn tap(&self, device: &str, target: TapTarget) -> BResult<()>;
    fn swipe(
        &self,
        device: &str,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        duration_ms: u32,
    ) -> BResult<()>;
    fn input_text(&self, device: &str, text: &str) -> BResult<()>;
    fn key(&self, device: &str, key: &str) -> BResult<()>;
    fn screenshot(&self, device: &str, out: &Path) -> BResult<PathBuf>;
    fn apps(&self, device: &str, filter: Option<&str>, all: bool) -> BResult<Vec<String>>;
    fn launch(&self, device: &str, package: &str) -> BResult<()>;
    fn stop(&self, device: &str, package: &str) -> BResult<()>;
    fn logcat(
        &self,
        device: &str,
        lines: u32,
        tag: Option<&str>,
        level: Option<&str>,
    ) -> BResult<String>;
    fn shell(&self, device: &str, cmd: &[String]) -> BResult<ShellResult>;
}

/// 目标设备解析：--device 显式指定 → 配置默认 → 仅一台在线 → 歧义错误。
pub fn resolve_target(
    selector: Option<&str>,
    default: Option<&str>,
    online: &[DeviceRecord],
) -> BResult<DeviceRecord> {
    let find = |id: &str| online.iter().find(|d| d.id == id).cloned();
    match selector.or(default) {
        Some(id) => find(id).ok_or_else(|| {
            ErrorBody::device_not_found(format!("设备 {id} 不在线；在线设备: {}", list_ids(online)))
        }),
        None => match online {
            [single] => Ok(single.clone()),
            [] => Err(ErrorBody::device_offline(
                "没有在线设备；请先连接设备（adb connect 或桥接 App）",
            )),
            many => Err(ErrorBody::device_ambiguous(
                "存在多个在线设备，请用 --device 指定",
                Some(serde_json::json!({ "candidates": list_ids(many) })),
            )),
        },
    }
}

fn list_ids(devices: &[DeviceRecord]) -> String {
    devices
        .iter()
        .map(|d| d.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str) -> DeviceRecord {
        DeviceRecord {
            id: id.into(),
            kind: BackendKind::Adb,
            model: Some("m".into()),
            state: DeviceState::Online,
            connection: ConnectionKind::Network,
        }
    }

    #[test]
    fn resolve_single_online() {
        let devices = vec![rec("a:5555")];
        let got = resolve_target(None, None, &devices).unwrap();
        assert_eq!(got.id, "a:5555");
    }

    #[test]
    fn resolve_none_online() {
        let err = resolve_target(None, None, &[]).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::DeviceOffline);
    }

    #[test]
    fn resolve_ambiguous_lists_candidates() {
        let devices = vec![rec("a:5555"), rec("b:5555")];
        let err = resolve_target(None, None, &devices).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::DeviceAmbiguous);
        let details = err.details.unwrap();
        assert!(details["candidates"].as_str().unwrap().contains("a:5555"));
    }

    #[test]
    fn resolve_selector_hit_and_miss() {
        let devices = vec![rec("a:5555"), rec("b:5555")];
        assert_eq!(
            resolve_target(Some("b:5555"), None, &devices).unwrap().id,
            "b:5555"
        );
        let err = resolve_target(Some("zzz"), None, &devices).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::DeviceNotFound);
    }

    #[test]
    fn default_used_when_no_selector() {
        let devices = vec![rec("a:5555"), rec("b:5555")];
        let got = resolve_target(None, Some("b:5555"), &devices).unwrap();
        assert_eq!(got.id, "b:5555");
    }
}
