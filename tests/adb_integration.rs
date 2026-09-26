//! ADB 集成测试：依赖真实 adb 与在线设备。
//! 运行方式：AGENT_MOBILE_TEST_DEVICE=127.0.0.1:5555 cargo test --test adb_integration -- --ignored
//!
//! 通过 #[path] 引入库层模块树（crate:: 路径与二进制目标一致），无需新建 lib target。

#[path = "../src/adb/mod.rs"]
mod adb;
#[path = "../src/backend/mod.rs"]
mod backend;
#[path = "../src/bridge_proto.rs"]
mod bridge_proto;
#[path = "../src/config.rs"]
mod config;
#[allow(dead_code)]
#[path = "../src/output.rs"]
mod output;
#[path = "../src/ui.rs"]
mod ui;
// backend::bridge 依赖的注册表子树（crate:: 路径与二进制目标一致）
#[path = "daemon/mod.rs"]
mod daemon;

use backend::adb::AdbBackend;
use backend::{Backend, DeviceState};
use parking_lot::Mutex;

/// 集成测试共享锁：断连/重连与设备枚举必须互斥，避免并行用例互相影响。
static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// 读取目标设备；未设置环境变量时返回 None（测试直接跳过）。
fn target_device() -> Option<String> {
    std::env::var("AGENT_MOBILE_TEST_DEVICE").ok()
}

fn locate_backend() -> AdbBackend {
    let adb = adb::Adb::locate(&config::Config::default()).expect("本机应能探测到 adb");
    AdbBackend::new(adb)
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn devices_contains_target_online_and_resolves() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    // 先确保目标设备已连接（connect 幂等：已连接返回 already connected）。
    backend
        .connect(&target)
        .unwrap_or_else(|e| panic!("连接 {target} 应成功: {}", e.message));

    let devices = backend.devices().expect("adb devices -l 应成功");
    let record = devices
        .iter()
        .find(|d| d.id == target)
        .unwrap_or_else(|| panic!("设备列表应包含 {target}，实际: {devices:?}"));
    assert_eq!(record.state, DeviceState::Online, "{target} 应在线");
    let online: Vec<_> = devices
        .iter()
        .filter(|d| d.state == DeviceState::Online)
        .cloned()
        .collect();
    let resolved =
        backend::resolve_target(Some(&target), None, &online).expect("显式指定在线设备应解析成功");
    assert_eq!(resolved.id, target);
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn connect_after_disconnect_succeeds() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    // 断连与重连必须在本测试函数内串行完成，避免影响其他测试。
    let adb_path = adb::Adb::locate(&config::Config::default())
        .expect("本机应能探测到 adb")
        .path;
    let _ = std::process::Command::new(&adb_path)
        .args(["disconnect", &target])
        .output();

    let msg = backend
        .connect(&target)
        .unwrap_or_else(|e| panic!("重连 {target} 应成功: {}", e.message));
    assert!(
        msg.contains("connected"),
        "connect 输出应含 connected，实际: {msg}"
    );

    let devices = backend.devices().expect("重连后 adb devices -l 应成功");
    let record = devices
        .iter()
        .find(|d| d.id == target)
        .unwrap_or_else(|| panic!("重连后设备列表应包含 {target}"));
    assert_eq!(record.state, DeviceState::Online, "重连后 {target} 应在线");
}
