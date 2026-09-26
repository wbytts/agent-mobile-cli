//! MobileOps 的生产实现（Android 经 Tauri 插件调到 Kotlin）与非 Android 占位实现。

use crate::sandbox::MobileOps;

#[cfg(target_os = "android")]
mod imp {
    use super::*;
    use serde_json::{json, Value};
    use tauri::{plugin::PluginHandle, Wry};

    /// 经 `PluginHandle::run_mobile_plugin`（blocking）调用 Kotlin BridgePlugin。
    /// 供 Tauri 命令线程与 QuickJS 沙盒线程使用；调用方不得在 UI 主线程阻塞等待。
    pub struct AndroidMobileOps {
        handle: PluginHandle<Wry>,
    }

    impl AndroidMobileOps {
        pub fn new(handle: PluginHandle<Wry>) -> Self {
            Self { handle }
        }

        fn call(&self, command: &str, payload: Value) -> Result<String, String> {
            let result: Value = self
                .handle
                .run_mobile_plugin(command, payload)
                .map_err(|e| e.to_string())?;
            Ok(result.to_string())
        }
    }

    /// BridgePlatform 的 Android 实现：设备信息、token SharedPreferences 存储、
    /// 前台服务保活、扫码——全部经同一 PluginHandle 调到 Kotlin BridgePlugin。
    pub struct AndroidBridgePlatform {
        handle: PluginHandle<Wry>,
    }

    impl AndroidBridgePlatform {
        pub fn new(handle: PluginHandle<Wry>) -> Self {
            Self { handle }
        }

        fn call(&self, command: &str, payload: Value) -> Result<Value, String> {
            self.handle
                .run_mobile_plugin(command, payload)
                .map_err(|e| e.to_string())
        }

        fn device_info_str(&self, key: &str) -> Option<String> {
            self.call("deviceInfo", json!({})).ok().and_then(|v| {
                v.get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|s| !s.is_empty())
            })
        }
    }

    impl crate::bridge_client::BridgePlatform for AndroidBridgePlatform {
        fn device_name(&self) -> String {
            self.device_info_str("device_name")
                .unwrap_or_else(|| "android".to_string())
        }

        fn android_version(&self) -> String {
            self.device_info_str("android_version")
                .unwrap_or_else(|| "unknown".to_string())
        }

        fn load_token(&self, host: &str, port: u16) -> Option<String> {
            self.call("getBridgeToken", json!({ "host": host, "port": port }))
                .ok()
                .and_then(|v| {
                    v.get("token")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .filter(|s| !s.is_empty())
                })
        }

        fn save_token(&self, host: &str, port: u16, token: &str) {
            let _ = self.call(
                "setBridgeToken",
                json!({ "host": host, "port": port, "token": token }),
            );
        }

        fn set_foreground(&self, running: bool) {
            let command = if running {
                "startForegroundService"
            } else {
                "stopForegroundService"
            };
            let _ = self.call(command, json!({}));
        }

        fn load_address(&self) -> Option<(String, u16)> {
            let v = self.call("getLastAddress", json!({})).ok()?;
            let host = v.get("host").and_then(Value::as_str)?.to_string();
            let port = v.get("port").and_then(Value::as_u64)? as u16;
            (!host.is_empty() && port > 0).then_some((host, port))
        }

        fn save_address(&self, host: &str, port: u16) {
            let _ = self.call("setLastAddress", json!({ "host": host, "port": port }));
        }

        fn scan_pair_qr(&self) -> Result<String, String> {
            let v = self.call("scanPairQr", json!({}))?;
            v.get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "扫码结果为空".to_string())
        }
    }
    impl MobileOps for AndroidMobileOps {
        fn tap(&self, x: f64, y: f64) -> Result<String, String> {
            self.call("tap", json!({ "x": x, "y": y }))
        }

        fn swipe(
            &self,
            x1: f64,
            y1: f64,
            x2: f64,
            y2: f64,
            duration_ms: f64,
        ) -> Result<String, String> {
            self.call(
                "swipe",
                json!({
                    "x1": x1, "y1": y1, "x2": x2, "y2": y2,
                    "duration_ms": duration_ms as u64,
                }),
            )
        }

        fn input(&self, text: &str) -> Result<String, String> {
            self.call("input", json!({ "text": text }))
        }

        fn key(&self, key: &str) -> Result<String, String> {
            self.call("key", json!({ "key": key }))
        }

        fn ui_tree(&self) -> Result<String, String> {
            self.call("uiTree", json!({}))
        }

        fn screenshot(&self) -> Result<String, String> {
            self.call("screenshot", json!({}))
        }

        fn apps(&self, filter: Option<&str>, all: bool) -> Result<String, String> {
            self.call("apps", json!({ "filter": filter, "all": all }))
        }

        fn launch(&self, package: &str) -> Result<String, String> {
            self.call("launch", json!({ "package": package }))
        }

        fn a11y_status(&self) -> Result<String, String> {
            self.call("a11yStatus", json!({}))
        }

        fn open_a11y_settings(&self) -> Result<String, String> {
            self.call("openA11ySettings", json!({}))
        }
    }
}
#[cfg(not(target_os = "android"))]
mod imp {
    use super::*;

    const MSG: &str = "设备能力仅 Android 平台可用";

    /// 非 Android 平台占位：所有能力直接报错（保证 host 侧可编译可单测）。
    pub struct UnsupportedMobileOps;

    impl MobileOps for UnsupportedMobileOps {
        fn tap(&self, _x: f64, _y: f64) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn swipe(
            &self,
            _x1: f64,
            _y1: f64,
            _x2: f64,
            _y2: f64,
            _duration_ms: f64,
        ) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn input(&self, _text: &str) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn key(&self, _key: &str) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn ui_tree(&self) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn screenshot(&self) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn apps(&self, _filter: Option<&str>, _all: bool) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn launch(&self, _package: &str) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn a11y_status(&self) -> Result<String, String> {
            Err(MSG.to_string())
        }

        fn open_a11y_settings(&self) -> Result<String, String> {
            Err(MSG.to_string())
        }
    }

    /// 非 Android 平台占位 BridgePlatform：连接不可用，仅保证 host 可编译。
    pub struct UnsupportedBridgePlatform;

    impl crate::bridge_client::BridgePlatform for UnsupportedBridgePlatform {
        fn device_name(&self) -> String {
            "host-device".to_string()
        }

        fn android_version(&self) -> String {
            "unknown".to_string()
        }

        fn load_token(&self, _host: &str, _port: u16) -> Option<String> {
            None
        }

        fn save_token(&self, _host: &str, _port: u16, _token: &str) {}

        fn set_foreground(&self, _running: bool) {}

        fn scan_pair_qr(&self) -> Result<String, String> {
            Err("扫码仅 Android 平台可用".to_string())
        }

        fn load_address(&self) -> Option<(String, u16)> {
            None
        }

        fn save_address(&self, _host: &str, _port: u16) {}
    }
}

pub use imp::*;
