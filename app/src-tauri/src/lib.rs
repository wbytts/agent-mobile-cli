//! Agent Mobile Bridge Rust core。
//! 本阶段仅承载 WebView UI；桥接协议/WS 客户端/QuickJS 沙盒在任务组 4 接入。

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
        .run(tauri::generate_context!())
        .expect("error while running Agent Mobile Bridge");
}
