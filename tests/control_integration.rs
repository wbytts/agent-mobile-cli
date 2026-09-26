//! 组 4 设备控制命令集成测试：依赖真实 adb 与在线设备。
//! 运行方式：AGENT_MOBILE_TEST_DEVICE=127.0.0.1:5555 cargo test --test control_integration -- --ignored
//!
//! 通过 #[path] 引入库层模块树（crate:: 路径与二进制目标一致），无需新建 lib target。

// 测试目标只引用模块树子集：CLI/daemon 专用 API（如 Output::print）在本树内未用，
// 与 tests/adb_integration.rs 的既有 #[path] 模式一致，对无内层 allow 的模块放行 dead_code。
#[path = "../src/adb/mod.rs"]
mod adb;
#[allow(dead_code)]
#[path = "../src/backend/mod.rs"]
mod backend;
#[allow(dead_code)]
#[path = "../src/config.rs"]
mod config;
#[allow(dead_code)]
#[path = "../src/output.rs"]
mod output;
#[path = "../src/ui.rs"]
mod ui;
use backend::adb::AdbBackend;
use backend::{Backend, TapTarget};
use output::ErrorCode;
use parking_lot::Mutex;

/// 集成测试共享锁：所有用例互斥串行，避免并行操作互相干扰设备状态。
static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// 读取目标设备；未设置环境变量时返回 None（测试直接跳过）。
fn target_device() -> Option<String> {
    std::env::var("AGENT_MOBILE_TEST_DEVICE").ok()
}

fn locate_backend() -> AdbBackend {
    let adb = adb::Adb::locate(&config::Config::default()).expect("本机应能探测到 adb");
    AdbBackend::new(adb)
}

/// 回到桌面，减少测试对设备前台状态的残留。
fn go_home(backend: &AdbBackend, device: &str) {
    let _ = backend.key(device, "KEYCODE_HOME");
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn snapshot_returns_tree_with_refs() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    let snap = backend
        .snapshot(&target, false)
        .unwrap_or_else(|e| panic!("snapshot 应成功: {}", e.message));
    assert!(!snap.tree.is_empty(), "简化树应非空");
    assert!(!snap.refs.is_empty(), "应存在可交互元素引用");
    assert_eq!(snap.refs[0].id, "@e1", "首个引用应为 @e1");
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn concurrent_snapshots_both_succeed() {
    // 并发安全回归：两个线程对同设备并发 snapshot，
    // backend 以 pid+原子序号生成唯一临时路径并加进程内临界区锁，均应成功且树非空。
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    std::thread::scope(|s| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                s.spawn(|| {
                    backend
                        .snapshot(&target, false)
                        .unwrap_or_else(|e| panic!("并发 snapshot 应成功: {}", e.message))
                })
            })
            .collect();
        for h in handles {
            let snap = h.join().expect("snapshot 线程不应 panic");
            assert!(!snap.tree.is_empty(), "并发快照简化树应非空");
        }
    });
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn tap_coord_changes_ui() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();
    go_home(&backend, &target);
    std::thread::sleep(std::time::Duration::from_millis(800));

    let before = backend
        .snapshot(&target, false)
        .unwrap_or_else(|e| panic!("tap 前 snapshot 应成功: {}", e.message));
    // 选叶子图标（优先桌面「设置」），避免全屏 ScrollView 容器中心点落在空白处。
    let icon = before
        .refs
        .iter()
        .find(|r| r.text.as_deref() == Some("设置") || r.content_desc.as_deref() == Some("设置"))
        .or_else(|| {
            before
                .refs
                .iter()
                .find(|r| r.text.is_some() || r.content_desc.is_some())
        });
    let Some(icon) = icon else {
        eprintln!("跳过：当前界面无带文本的可交互元素");
        return;
    };
    let (x, y) = icon.center;
    backend
        .tap(&target, TapTarget::Coord(x, y))
        .unwrap_or_else(|e| panic!("tap ({x},{y}) 应成功: {}", e.message));
    std::thread::sleep(std::time::Duration::from_millis(2000));

    let after = backend
        .snapshot(&target, false)
        .unwrap_or_else(|e| panic!("tap 后 snapshot 应成功: {}", e.message));
    // 恢复原状：停止可能拉起的设置并返回桌面。
    let _ = backend.stop(&target, "com.android.settings");
    go_home(&backend, &target);
    assert_ne!(before.tree, after.tree, "点击桌面图标后界面应变化");
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn input_text_ascii_succeeds_and_non_ascii_rejected() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    // ASCII 输入封装可用（无焦点输入框时 input text 仍为合法命令，退出码 0）。
    backend
        .input_text(&target, "abc 123")
        .unwrap_or_else(|e| panic!("ASCII input_text 应成功: {}", e.message));
    // 非 ASCII 校验在下发前完成，返回 NOT_SUPPORTED。
    let err = backend.input_text(&target, "你好").unwrap_err();
    assert_eq!(err.code, ErrorCode::NotSupported);
    assert!(err.message.contains("ASCII"));
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn screenshot_writes_valid_png() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    let dir = tempfile::tempdir().expect("临时目录");
    let out = dir.path().join("shot.png");
    let path = backend
        .screenshot(&target, &out)
        .unwrap_or_else(|e| panic!("screenshot 应成功: {}", e.message));
    assert_eq!(path, out);
    let bytes = std::fs::read(&out).expect("截图文件应可读");
    assert!(bytes.len() > 8, "截图文件应非空");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "应为 PNG 魔数");
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn apps_contains_settings_and_filter_works() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    let all = backend
        .apps(&target, None, true)
        .unwrap_or_else(|e| panic!("apps --all 应成功: {}", e.message));
    assert!(
        all.iter().any(|p| p == "com.android.settings"),
        "全部包列表应含 com.android.settings"
    );
    let filtered = backend
        .apps(&target, Some("settings"), true)
        .unwrap_or_else(|e| panic!("apps 带过滤应成功: {}", e.message));
    assert!(!filtered.is_empty(), "过滤 settings 应有结果");
    assert!(
        filtered.iter().all(|p| p.contains("settings")),
        "过滤结果应全部含 settings 子串"
    );
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn launch_and_stop_settings() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    let pid_of = |backend: &AdbBackend| -> Option<String> {
        let res = backend
            .shell(&target, &["pidof com.android.settings".to_string()])
            .expect("pidof 调用本身应成功");
        (res.exit_code == 0).then(|| res.stdout.trim().to_string())
    };

    // 先停止，确保前置状态干净。
    let _ = backend.stop(&target, "com.android.settings");
    std::thread::sleep(std::time::Duration::from_millis(500));

    backend
        .launch(&target, "com.android.settings")
        .unwrap_or_else(|e| panic!("launch 应成功: {}", e.message));
    std::thread::sleep(std::time::Duration::from_millis(2000));
    let pid = pid_of(&backend);
    assert!(pid.is_some(), "launch 后设置进程应存在");

    backend
        .stop(&target, "com.android.settings")
        .unwrap_or_else(|e| panic!("stop 应成功: {}", e.message));
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert!(pid_of(&backend).is_none(), "stop 后设置进程应消失");

    // 恢复原状：返回桌面。
    go_home(&backend, &target);
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn logcat_respects_line_limit() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    let text = backend
        .logcat(&target, 20, None, None)
        .unwrap_or_else(|e| panic!("logcat 应成功: {}", e.message));
    assert!(!text.trim().is_empty(), "logcat 输出应非空");
    // 硬契约：输出行数严格 ≤ 请求行数（backend 已去缓冲区块头并截取末尾 N 行）。
    let lines = text.lines().count();
    assert!(lines <= 20, "logcat 行数 {lines} 应严格 ≤ 20");
    assert!(
        !text
            .lines()
            .any(|l| l.starts_with("--------- beginning of")),
        "logcat 输出不应含缓冲区块头"
    );
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn shell_getprop_returns_model_with_exit_zero() {
    let Some(target) = target_device() else {
        eprintln!("跳过：未设置 AGENT_MOBILE_TEST_DEVICE");
        return;
    };
    let _guard = DEVICE_LOCK.lock();
    let backend = locate_backend();

    let res = backend
        .shell(
            &target,
            &["getprop".to_string(), "ro.product.model".to_string()],
        )
        .unwrap_or_else(|e| panic!("shell getprop 应成功: {}", e.message));
    assert_eq!(res.exit_code, 0, "getprop 退出码应为 0: {res:?}");
    assert!(!res.stdout.trim().is_empty(), "型号应非空");

    // 远端退出码透传：失败命令应取回非零码而非传输层 0。
    let fail = backend
        .shell(&target, &["ls /definitely/missing".to_string()])
        .unwrap_or_else(|e| panic!("shell ls 传输层应成功: {}", e.message));
    assert_ne!(fail.exit_code, 0, "失败命令应透传非零退出码");
}

#[test]
#[ignore = "需要真实 adb 与在线设备"]
fn offline_device_yields_structured_error() {
    let _guard = DEVICE_LOCK.lock();
    let _ = target_device(); // 仅保持跳过语义一致；本用例不需要在线设备
    let backend = locate_backend();
    let fake = "am-fake-serial-9999";

    let err = backend
        .shell(fake, &["echo hi".to_string()])
        .expect_err("伪造 serial 的 shell 应失败");
    assert!(
        matches!(
            err.code,
            ErrorCode::DeviceNotFound | ErrorCode::DeviceOffline
        ),
        "应为离线/未找到结构化错误，实际: {err:?}"
    );

    let err = backend
        .snapshot(fake, false)
        .expect_err("伪造 serial 的 snapshot 应失败");
    assert!(
        matches!(
            err.code,
            ErrorCode::DeviceNotFound | ErrorCode::DeviceOffline
        ),
        "应为离线/未找到结构化错误，实际: {err:?}"
    );

    let dir = tempfile::tempdir().expect("临时目录");
    let err = backend
        .screenshot(fake, &dir.path().join("x.png"))
        .expect_err("伪造 serial 的 screenshot 应失败");
    assert!(
        matches!(
            err.code,
            ErrorCode::DeviceNotFound | ErrorCode::DeviceOffline
        ),
        "应为离线/未找到结构化错误，实际: {err:?}"
    );
}
