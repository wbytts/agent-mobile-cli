//! adb 可执行文件封装：探测链与子进程调用（design.md 决策 2/3）。
use crate::backend::{
    BResult, BackendKind, ConnectionKind, DeviceRecord, DeviceState, ShellResult,
};
use crate::config::Config;
use crate::output::ErrorBody;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Adb {
    pub path: PathBuf,
}

impl Adb {
    /// 探测链：配置 adb_path → ANDROID_HOME/ANDROID_SDK_ROOT → PATH → 平台常见路径。
    pub fn locate(config: &Config) -> BResult<Adb> {
        let env = |key: &str| std::env::var(key).ok();
        let exists = |p: &Path| is_executable(p);
        locate_with(config.adb_path.as_deref(), &env, &exists)
            .map(|path| Adb { path })
            .ok_or_else(|| {
                ErrorBody::adb_not_found(
                    "未找到 adb 可执行文件；请安装 Android platform-tools \
                     （https://developer.android.com/tools/releases/platform-tools），\
                     或在配置文件 config.json 中设置 adb_path 指向 adb 绝对路径",
                )
            })
    }

    /// 解析 `adb devices -l` 输出为设备记录。
    pub fn devices(&self) -> BResult<Vec<DeviceRecord>> {
        let stdout = self.run(&["devices", "-l"], DEFAULT_TIMEOUT)?;
        Ok(parse_devices(&stdout))
    }

    /// `adb connect <host:port>`：输出含 connected 判定成功。
    pub fn connect(&self, target: &str) -> BResult<String> {
        let stdout = self.run(&["connect", target], DEFAULT_TIMEOUT)?;
        if stdout.contains("connected to") || stdout.contains("already connected") {
            Ok(stdout.trim().to_string())
        } else {
            Err(ErrorBody::adb_error(
                format!("adb connect {target} 失败"),
                Some(serde_json::json!({ "output": stdout.trim() })),
            ))
        }
    }

    /// `adb -s <device> shell <cmd>`：cmd 单字符串透传到设备端 shell，
    /// 经退出码标记包装返回远端真实退出码（stdout/stderr 分离收集）。
    pub fn shell(&self, device: &str, cmd: &str, timeout: Duration) -> BResult<ShellResult> {
        let wrapped = format!("{cmd} ; echo {EXIT_MARKER}$?");
        let out = self.spawn_collect(&["-s", device, "shell", &wrapped], timeout)?;
        if out.code != Some(0) {
            return Err(classify_failure(
                &format!("adb -s {device} shell {cmd}"),
                &out,
            ));
        }
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let (body, code) = strip_exit_marker(&stdout);
        let exit_code = code.ok_or_else(|| {
            ErrorBody::adb_error(
                format!("adb -s {device} shell 输出缺少退出码标记"),
                Some(serde_json::json!({ "stdout": body })),
            )
        })?;
        Ok(ShellResult {
            stdout: body,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit_code,
        })
    }

    /// `adb -s <device> exec-out <args>`：返回原始 stdout 字节，
    /// 用于截图等二进制输出场景（exec-out 不经 PTY，不会损坏换行）。
    pub fn exec_out(&self, device: &str, args: &[&str], timeout: Duration) -> BResult<Vec<u8>> {
        let mut full: Vec<&str> = vec!["-s", device, "exec-out"];
        full.extend_from_slice(args);
        let out = self.spawn_collect(&full, timeout)?;
        if out.code != Some(0) {
            return Err(classify_failure(&format!("adb {}", full.join(" ")), &out));
        }
        Ok(out.stdout)
    }

    /// `adb -s <device> pull <remote> <local>`：拉取设备文件到本地。
    pub fn pull(&self, device: &str, remote: &str, local: &Path, timeout: Duration) -> BResult<()> {
        let local_str = local.to_string_lossy().into_owned();
        let out = self.spawn_collect(&["-s", device, "pull", remote, &local_str], timeout)?;
        if out.code != Some(0) {
            return Err(classify_failure(
                &format!("adb -s {device} pull {remote}"),
                &out,
            ));
        }
        Ok(())
    }

    /// 设备相关命令透传（自动补 `-s <device>`），返回 stdout 文本。
    pub fn run_on(&self, device: &str, args: &[&str], timeout: Duration) -> BResult<String> {
        let mut full: Vec<&str> = vec!["-s", device];
        full.extend_from_slice(args);
        self.run(&full, timeout)
    }

    /// 基础子进程调用：收集 stdout/stderr，超时 kill，非零退出转结构化错误。
    fn run(&self, args: &[&str], timeout: Duration) -> BResult<String> {
        let out = self.spawn_collect(args, timeout)?;
        if out.code != Some(0) {
            return Err(classify_failure(&format!("adb {}", args.join(" ")), &out));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// 启动 adb 子进程并排空管道：超时 kill 转 Timeout，spawn/wait 失败转 IoError。
    fn spawn_collect(&self, args: &[&str], timeout: Duration) -> BResult<RawOutput> {
        let mut child = Command::new(&self.path)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                ErrorBody::io_error(format!("启动 adb（{}）失败: {e}", self.path.display()))
            })?;

        // 分别在读取线程中排空管道，避免子进程写满管道缓冲区后死锁。
        let mut out_pipe = child.stdout.take().expect("stdout 已 piped");
        let mut err_pipe = child.stderr.take().expect("stderr 已 piped");
        let out_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = out_pipe.read_to_end(&mut buf);
            buf
        });
        let err_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = err_pipe.read_to_end(&mut buf);
            buf
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(ErrorBody::timeout(format!(
                            "adb {} 执行超时（{}s）",
                            args.join(" "),
                            timeout.as_secs()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(ErrorBody::io_error(format!("等待 adb 进程退出失败: {e}"))),
            }
        };

        Ok(RawOutput {
            stdout: out_thread.join().unwrap_or_default(),
            stderr: err_thread.join().unwrap_or_default(),
            code: status.code(),
        })
    }
}

/// shell 退出码标记：追加 `; echo <标记>$?` 取回远端命令真实退出码
///（adb shell 自身退出码只反映传输层，不反映远端命令结果）。
const EXIT_MARKER: &str = "__AMC_EXIT__";

/// adb 子进程原始输出：保留字节以支持 exec-out 二进制场景。
struct RawOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// None 表示进程被信号终止。
    code: Option<i32>,
}

/// 非零退出分类（design.md 决策 7）：stderr/stdout 命中设备离线/未找到特征
/// 时转 DeviceOffline/DeviceNotFound 结构化错误，其余为 AdbError。
fn classify_failure(desc: &str, out: &RawOutput) -> ErrorBody {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let haystack = format!("{stdout}\n{stderr}").to_lowercase();
    if haystack.contains("device offline") {
        ErrorBody::device_offline(format!("{desc} 失败：设备离线"))
    } else if haystack.contains("not found") {
        ErrorBody::device_not_found(format!("{desc} 失败：设备不存在或未连接"))
    } else {
        ErrorBody::adb_error(
            format!("{desc} 退出码 {}", out.code.unwrap_or(-1)),
            Some(serde_json::json!({
                "stdout": stdout.trim(),
                "stderr": stderr.trim(),
            })),
        )
    }
}

/// 剥离 shell 输出尾部的退出码标记，返回（命令原始输出, 退出码）。
fn strip_exit_marker(stdout: &str) -> (String, Option<i32>) {
    match stdout.rfind(EXIT_MARKER) {
        Some(pos) => {
            let body = &stdout[..pos];
            let code = stdout[pos + EXIT_MARKER.len()..].trim().parse::<i32>().ok();
            (body.to_string(), code)
        }
        None => (stdout.to_string(), None),
    }
}

/// adb 可执行文件名（Windows 带 .exe）。
fn adb_exe_name() -> &'static str {
    if cfg!(windows) {
        "adb.exe"
    } else {
        "adb"
    }
}

/// 存在且可执行（Unix 校验可执行位，其余平台校验为文件）。
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// 探测链纯函数：env / exists 全部注入，便于单测桩。
fn locate_with(
    config_path: Option<&Path>,
    env: &dyn Fn(&str) -> Option<String>,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    // 1. 配置 adb_path
    if let Some(p) = config_path {
        if exists(p) {
            return Some(p.to_path_buf());
        }
    }
    // 2/3. ANDROID_HOME / ANDROID_SDK_ROOT 下的 platform-tools
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = env(key) {
            let p = Path::new(&root).join("platform-tools").join(adb_exe_name());
            if exists(&p) {
                return Some(p);
            }
        }
    }
    // 4. PATH（等价 which("adb")）
    if let Some(path_var) = env("PATH") {
        let found = std::env::split_paths(&path_var)
            .map(|dir| dir.join(adb_exe_name()))
            .find(|p| exists(p));
        if found.is_some() {
            return found;
        }
    }
    // 5. 平台常见安装路径
    platform_candidates(env).into_iter().find(|p| exists(p))
}

/// 各平台常见 SDK 安装路径。
fn platform_candidates(env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(target_os = "macos")]
    if let Some(home) = env("HOME") {
        candidates.push(Path::new(&home).join("Library/Android/sdk/platform-tools/adb"));
    }
    #[cfg(target_os = "linux")]
    if let Some(home) = env("HOME") {
        candidates.push(Path::new(&home).join("Android/Sdk/platform-tools/adb"));
    }
    #[cfg(target_os = "windows")]
    if let Some(local) = env("LOCALAPPDATA") {
        candidates.push(Path::new(&local).join("Android/Sdk/platform-tools/adb.exe"));
    }
    let _ = env;
    candidates
}

/// 解析 `adb devices -l` 文本（纯函数，便于单测）。
pub fn parse_devices(text: &str) -> Vec<DeviceRecord> {
    text.lines()
        .skip(1) // 跳过表头 "List of devices attached"
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('*') {
                return None;
            }
            let mut parts = line.split_whitespace();
            let serial = parts.next()?;
            let state = match parts.next()? {
                "device" => DeviceState::Online,
                "offline" => DeviceState::Offline,
                "unauthorized" => DeviceState::Unauthorized,
                // authorizing / connecting 等瞬态按离线处理
                _ => DeviceState::Offline,
            };
            let model = parts
                .find_map(|token| token.strip_prefix("model:"))
                .map(str::to_string);
            let connection = if serial.contains(':') || serial.starts_with("emulator-") {
                ConnectionKind::Network
            } else {
                ConnectionKind::Usb
            };
            Some(DeviceRecord {
                id: serial.to_string(),
                kind: BackendKind::Adb,
                model,
                state,
                connection,
            })
        })
        .collect()
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

    #[test]
    fn skips_daemon_messages_and_classifies_connection() {
        let text = "List of devices attached\n* daemon not running; starting now at tcp:5037\nemulator-5554\tdevice model:sdk_gphone\n";
        let devices = parse_devices(text);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].connection, ConnectionKind::Network);
        assert_eq!(devices[0].kind, BackendKind::Adb);
    }

    // ---- locate_with 探测链 ----

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn none_exists(_: &Path) -> bool {
        false
    }

    #[test]
    fn locate_prefers_config_path() {
        let cfg = PathBuf::from("/opt/adb");
        let found = locate_with(Some(&cfg), &no_env, &|p| p == Path::new("/opt/adb"));
        assert_eq!(found, Some(cfg));
    }

    #[test]
    fn locate_falls_back_to_android_home_when_config_missing() {
        let cfg = PathBuf::from("/missing/adb");
        let env = |key: &str| (key == "ANDROID_HOME").then(|| "/sdk".to_string());
        let exists = |p: &Path| p == Path::new("/sdk/platform-tools/adb");
        let found = locate_with(Some(&cfg), &env, &exists);
        assert_eq!(found, Some(PathBuf::from("/sdk/platform-tools/adb")));
    }

    #[test]
    fn locate_prefers_android_home_over_sdk_root() {
        let env = |key: &str| match key {
            "ANDROID_HOME" => Some("/home-sdk".to_string()),
            "ANDROID_SDK_ROOT" => Some("/root-sdk".to_string()),
            _ => None,
        };
        let exists = |p: &Path| {
            p == Path::new("/home-sdk/platform-tools/adb")
                || p == Path::new("/root-sdk/platform-tools/adb")
        };
        let found = locate_with(None, &env, &exists);
        assert_eq!(found, Some(PathBuf::from("/home-sdk/platform-tools/adb")));
    }

    #[test]
    fn locate_falls_back_to_path() {
        let env = |key: &str| (key == "PATH").then(|| "/usr/bin:/usr/local/bin".to_string());
        let exists = |p: &Path| p == Path::new("/usr/local/bin/adb");
        let found = locate_with(None, &env, &exists);
        assert_eq!(found, Some(PathBuf::from("/usr/local/bin/adb")));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn locate_falls_back_to_platform_path() {
        let env = |key: &str| (key == "HOME").then(|| "/Users/test".to_string());
        let exists =
            |p: &Path| p == Path::new("/Users/test/Library/Android/sdk/platform-tools/adb");
        let found = locate_with(None, &env, &exists);
        assert_eq!(
            found,
            Some(PathBuf::from(
                "/Users/test/Library/Android/sdk/platform-tools/adb"
            ))
        );
    }

    #[test]
    fn locate_returns_none_when_all_fail() {
        assert_eq!(locate_with(None, &no_env, &none_exists), None);
    }

    #[test]
    fn locate_error_contains_install_hint() {
        let config = Config {
            adb_path: Some(PathBuf::from("/definitely/missing/adb")),
            ..Config::default()
        };
        // 真实环境下可能从 PATH/常见路径找到 adb；此处仅在校验失败分支时断言文案。
        if let Err(e) = Adb::locate(&config) {
            assert_eq!(e.code, crate::output::ErrorCode::AdbNotFound);
            assert!(e.message.contains("platform-tools"));
            assert!(e.message.contains("adb_path"));
        }
    }

    // ---- 组 4：离线/未找到错误分类 ----

    #[test]
    fn classify_device_offline_from_stderr() {
        let out = RawOutput {
            stdout: Vec::new(),
            stderr: b"error: device offline".to_vec(),
            code: Some(1),
        };
        let e = classify_failure("adb -s d shell x", &out);
        assert_eq!(e.code, crate::output::ErrorCode::DeviceOffline);
    }

    #[test]
    fn classify_device_not_found_from_stderr() {
        let out = RawOutput {
            stdout: Vec::new(),
            stderr: b"adb: device 'fake-serial-9999' not found".to_vec(),
            code: Some(1),
        };
        let e = classify_failure("adb -s d shell x", &out);
        assert_eq!(e.code, crate::output::ErrorCode::DeviceNotFound);
    }

    #[test]
    fn classify_generic_failure_as_adb_error() {
        let out = RawOutput {
            stdout: b"some output".to_vec(),
            stderr: b"permission denied".to_vec(),
            code: Some(2),
        };
        let e = classify_failure("adb -s d shell x", &out);
        assert_eq!(e.code, crate::output::ErrorCode::AdbError);
    }

    // ---- 组 4：shell 退出码标记解析 ----

    #[test]
    fn strip_exit_marker_parses_trailing_code() {
        let (body, code) = strip_exit_marker("hello\n__AMC_EXIT__0\n");
        assert_eq!(body, "hello\n");
        assert_eq!(code, Some(0));
    }

    #[test]
    fn strip_exit_marker_parses_nonzero_code() {
        let (body, code) = strip_exit_marker("ls: x: No such file or directory\n__AMC_EXIT__1\n");
        assert_eq!(code, Some(1));
        assert!(body.contains("No such file"));
    }

    #[test]
    fn strip_exit_marker_handles_missing_marker() {
        let (body, code) = strip_exit_marker("no marker here");
        assert_eq!(body, "no marker here");
        assert_eq!(code, None);
    }

    #[test]
    fn strip_exit_marker_glued_to_output_without_newline() {
        // 命令输出不以换行结尾时标记与其粘连，仍应正确拆分。
        let (body, code) = strip_exit_marker("hi__AMC_EXIT__42\n");
        assert_eq!(body, "hi");
        assert_eq!(code, Some(42));
    }
}
