//! Agent Mobile Bridge Rust core。
//! QuickJS 脚本沙盒与 Kotlin 插件桥在任务组 3 接入；WS 客户端在任务组 4 接入。

pub mod mobile;
pub mod sandbox;

use std::sync::Arc;

use sandbox::MobileOps;
use serde_json::Value;
use tauri::{Manager, State};

/// Kotlin 桥命令入口：前端自检页命令与组 4 WS 命令循环共用的设备能力。
pub struct MobileBridge {
    ops: Arc<dyn MobileOps>,
}

impl MobileBridge {
    /// 供组 4 脚本沙盒复用同一能力桥。
    pub fn ops(&self) -> Arc<dyn MobileOps> {
        self.ops.clone()
    }
}

/// 桥返回 JSON 文本 → serde_json::Value。
fn json_result(result: Result<String, String>) -> Result<Value, String> {
    result
        .and_then(|text| serde_json::from_str(&text).map_err(|e| format!("插件返回非法 JSON: {e}")))
}

#[tauri::command]
fn bridge_tap(state: State<'_, MobileBridge>, x: f64, y: f64) -> Result<Value, String> {
    json_result(state.ops.tap(x, y))
}

#[tauri::command]
fn bridge_swipe(
    state: State<'_, MobileBridge>,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    duration_ms: f64,
) -> Result<Value, String> {
    json_result(state.ops.swipe(x1, y1, x2, y2, duration_ms))
}

#[tauri::command]
fn bridge_input(state: State<'_, MobileBridge>, text: String) -> Result<Value, String> {
    json_result(state.ops.input(&text))
}

#[tauri::command]
fn bridge_key(state: State<'_, MobileBridge>, key: String) -> Result<Value, String> {
    json_result(state.ops.key(&key))
}

#[tauri::command]
fn bridge_ui_tree(state: State<'_, MobileBridge>) -> Result<Value, String> {
    json_result(state.ops.ui_tree())
}

#[tauri::command]
fn bridge_screenshot(state: State<'_, MobileBridge>) -> Result<Value, String> {
    json_result(state.ops.screenshot())
}

#[tauri::command]
fn bridge_apps(
    state: State<'_, MobileBridge>,
    filter: Option<String>,
    all: Option<bool>,
) -> Result<Value, String> {
    json_result(state.ops.apps(filter.as_deref(), all.unwrap_or(false)))
}

#[tauri::command]
fn bridge_launch(state: State<'_, MobileBridge>, package: String) -> Result<Value, String> {
    json_result(state.ops.launch(&package))
}

#[tauri::command]
fn bridge_a11y_status(state: State<'_, MobileBridge>) -> Result<Value, String> {
    json_result(state.ops.a11y_status())
}

#[tauri::command]
fn bridge_open_a11y_settings(state: State<'_, MobileBridge>) -> Result<Value, String> {
    json_result(state.ops.open_a11y_settings())
}

/// 注册 bridge 插件：Android 侧绑定 Kotlin BridgePlugin，其他平台用占位实现。
fn bridge_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("bridge")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let ops: Arc<dyn MobileOps> = {
                let handle =
                    api.register_android_plugin("com.agentmobile.bridge", "BridgePlugin")?;
                Arc::new(mobile::AndroidMobileOps::new(handle))
            };
            #[cfg(not(target_os = "android"))]
            let ops: Arc<dyn MobileOps> = {
                let _ = api;
                Arc::new(mobile::UnsupportedMobileOps)
            };
            app.manage(MobileBridge { ops });
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
fn init_android_logger() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("AgentMobileBridge"),
    );
}

#[cfg(not(target_os = "android"))]
fn init_android_logger() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_android_logger();
    tauri::Builder::default()
        .plugin(bridge_plugin())
        .invoke_handler(tauri::generate_handler![
            bridge_tap,
            bridge_swipe,
            bridge_input,
            bridge_key,
            bridge_ui_tree,
            bridge_screenshot,
            bridge_apps,
            bridge_launch,
            bridge_a11y_status,
            bridge_open_a11y_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agent Mobile Bridge");
}
