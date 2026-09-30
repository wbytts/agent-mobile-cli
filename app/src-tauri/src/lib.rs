//! Agent Mobile Bridge Rust core。
//! QuickJS 脚本沙盒与 Kotlin 插件桥在任务组 3 接入；WS 客户端在任务组 4 接入。

// 协议类型单一来源（design.md 决策 11）：path 引用 CLI 侧 src/bridge_proto.rs，
// 两端不各自维护消息定义；该文件仅依赖 serde/serde_json。
#[path = "../../../src/bridge_proto.rs"]
pub mod bridge_proto;

pub mod bridge_client;
pub mod mobile;
pub mod sandbox;

use std::sync::Arc;

use bridge_client::{BridgeController, ConnState, PairInfo};
use sandbox::MobileOps;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

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
/// 同时装配组 4 的连接控制器（WS 客户端 + 命令循环 + 配对存储/前台服务桥）。
fn bridge_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("bridge")
        .setup(|app, api| {
            #[cfg(target_os = "android")]
            let (ops, platform) = {
                let handle =
                    api.register_android_plugin("com.agentmobile.bridge", "BridgePlugin")?;
                let ops: Arc<dyn MobileOps> =
                    Arc::new(mobile::AndroidMobileOps::new(handle.clone()));
                let platform: Arc<dyn bridge_client::BridgePlatform> =
                    Arc::new(mobile::AndroidBridgePlatform::new(handle));
                (ops, platform)
            };
            #[cfg(not(target_os = "android"))]
            let (ops, platform) = {
                let _ = api;
                let ops: Arc<dyn MobileOps> = Arc::new(mobile::UnsupportedMobileOps);
                let platform: Arc<dyn bridge_client::BridgePlatform> =
                    Arc::new(mobile::UnsupportedBridgePlatform);
                (ops, platform)
            };
            app.manage(MobileBridge { ops: ops.clone() });
            app.manage(BridgeController::new(ops, platform));
            Ok(())
        })
        .build()
}

// ---------- 组 4：连接开关 / 状态查询 / 配对扫码 ----------

/// 发起桥接连接（WS 客户端 + 自动重连）；配对码可空（已有 token 时免配对）。
/// `url` 为完整 WS URL（代理服务器 ws(s):// 形式）；None 时按 host/port 走 legacy daemon 路径。
#[tauri::command]
fn bridge_connect(
    app: AppHandle,
    ctrl: State<'_, BridgeController>,
    host: String,
    port: u16,
    pairing_code: Option<String>,
    url: Option<String>,
) -> Result<(), String> {
    ctrl.connect(app, host, port, pairing_code, url)
}

/// 断开桥接连接并停止前台服务。
#[tauri::command]
fn bridge_disconnect(app: AppHandle, ctrl: State<'_, BridgeController>) {
    ctrl.disconnect(&app);
}

/// 当前连接状态（UI 初次渲染用，后续经 `bridge://state` 事件推送）。
#[tauri::command]
fn bridge_state(ctrl: State<'_, BridgeController>) -> ConnState {
    ctrl.state()
}

/// 解析扫码得到的配对 URI（`agent-mobile://pair?host=..&port=..&code=..`）。
#[tauri::command]
fn bridge_parse_pair_uri(uri: String) -> Result<PairInfo, String> {
    bridge_client::parse_pair_uri(&uri)
}

/// 触发相机扫码（Kotlin 扫码页），返回扫描内容文本。
/// 扫码页等待用户操作，须放到 blocking 线程避免占用异步运行时。
#[tauri::command]
async fn bridge_scan_pair_qr(ctrl: State<'_, BridgeController>) -> Result<String, String> {
    let platform = ctrl.platform();
    tauri::async_runtime::spawn_blocking(move || platform.scan_pair_qr())
        .await
        .map_err(|e| format!("扫码任务中断: {e}"))?
}

/// 上次成功连接的 daemon 地址（连接页回填）。
#[tauri::command]
fn bridge_last_address(ctrl: State<'_, BridgeController>) -> Option<bridge_client::SavedAddress> {
    ctrl.platform()
        .load_address()
        .map(|(host, port)| bridge_client::SavedAddress { host, port })
}

/// 冷启动自动连接：存在已保存地址即发起连接（有 token 免配对），返回是否已触发。
#[tauri::command]
fn bridge_auto_connect(app: AppHandle, ctrl: State<'_, BridgeController>) -> bool {
    match bridge_client::auto_connect_target(&ctrl.platform()) {
        Some((host, port)) => {
            // 已连接/连接中时不重复触发
            if matches!(
                ctrl.state(),
                ConnState::Connected { .. } | ConnState::Connecting
            ) {
                return false;
            }
            let _ = ctrl.connect(app, host, port, None, None);
            true
        }
        None => false,
    }
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
            bridge_connect,
            bridge_disconnect,
            bridge_state,
            bridge_parse_pair_uri,
            bridge_scan_pair_qr,
            bridge_last_address,
            bridge_auto_connect,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Agent Mobile Bridge");
}
