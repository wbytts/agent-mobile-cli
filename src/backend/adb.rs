//! ADB 直连后端：实现 Backend trait，预留 app-bridge 路由（design.md 决策 10）。
// TODO(接线): CLI/daemon 命令接线后移除本行（参考 ui.rs 约定，避免组 2-4 接线前 dead_code 警告）。
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

    /// 执行设备 shell 命令并校验远端退出码为 0（组 4 内部辅助）。
    fn shell_checked(&self, device: &str, cmd: &str) -> BResult<()> {
        let res = self.adb.shell(device, cmd, crate::adb::DEFAULT_TIMEOUT)?;
        if res.exit_code == 0 {
            Ok(())
        } else {
            Err(ErrorBody::adb_error(
                format!("设备命令失败（退出码 {}）: {cmd}", res.exit_code),
                Some(serde_json::json!({
                    "stdout": res.stdout.trim(),
                    "stderr": res.stderr.trim(),
                })),
            ))
        }
    }
}

/// uiautomator dump 的设备端输出路径（design.md 决策 4）。
const UI_DUMP_REMOTE: &str = "/sdcard/am_ui.xml";

/// 转义 input text 载荷（纯函数，design.md 决策 5）：
/// 仅 ASCII；空格 → %s；字面 % → %25（input 自身的转义体系）；
/// 设备端 shell 元字符加反斜杠，避免被 /system/bin/sh 解释。
fn escape_input_text(text: &str) -> BResult<String> {
    if !text.is_ascii() {
        return Err(ErrorBody::not_supported(
            "input text 仅支持 ASCII 输入；非 ASCII（如中文）请改用其他输入方式",
        ));
    }
    const SHELL_META: &str = "\\\"'&|;<>()$`!*?[]{}~#^";
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            ' ' => out.push_str("%s"),
            '%' => out.push_str("%25"),
            c if SHELL_META.contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii_graphic() => out.push(c),
            c => {
                return Err(ErrorBody::not_supported(format!(
                    "input text 不支持控制字符 U+{:04X}",
                    c as u32
                )))
            }
        }
    }
    Ok(out)
}

/// 解析 `pm list packages` 输出（纯函数）：提取每行 `package:` 前缀后的包名。
fn parse_packages(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// 校验 PNG 魔数（纯函数）：screencap 输出损坏/非 PNG 时转 AdbError。
fn validate_png(bytes: &[u8]) -> BResult<()> {
    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() >= PNG_MAGIC.len() && &bytes[..8] == PNG_MAGIC {
        Ok(())
    } else {
        Err(ErrorBody::adb_error(
            "screencap 输出不是有效 PNG（魔数不匹配或为空）",
            Some(serde_json::json!({ "bytes": bytes.len() })),
        ))
    }
}

/// 校验包名/组件名字符集（防 shell 注入）：字母数字与 ._-。
fn validate_package(package: &str) -> BResult<()> {
    if !package.is_empty()
        && package
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        Ok(())
    } else {
        Err(ErrorBody::not_supported(format!(
            "非法包名（仅允许字母数字与 ._-）: {package}"
        )))
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

    /// UI 快照（design.md 决策 4）：uiautomator dump → pull 到临时文件 → ui::simplify。
    /// full=true 时 tree 直接放原始 XML 文本、refs 为空（ui.rs 的 Snapshot 无 raw_xml
    /// 字段，取最简方案；调用方需要完整结构时以 tree 为原始 XML 处理）。
    fn snapshot(&self, device: &str, full: bool) -> BResult<crate::ui::Snapshot> {
        let dump = self.adb.shell(
            device,
            &format!("uiautomator dump {UI_DUMP_REMOTE}"),
            crate::adb::TRANSFER_TIMEOUT,
        )?;
        if dump.exit_code != 0 {
            return Err(ErrorBody::adb_error(
                "uiautomator dump 失败",
                Some(serde_json::json!({
                    "exit_code": dump.exit_code,
                    "stdout": dump.stdout.trim(),
                    "stderr": dump.stderr.trim(),
                })),
            ));
        }
        let tmp = std::env::temp_dir().join(format!(
            "agent-mobile-ui-{}-{}.xml",
            std::process::id(),
            device.replace([':', '/', '\\'], "_")
        ));
        let result = (|| {
            self.adb
                .pull(device, UI_DUMP_REMOTE, &tmp, crate::adb::TRANSFER_TIMEOUT)?;
            let xml = std::fs::read_to_string(&tmp)
                .map_err(|e| ErrorBody::io_error(format!("读取快照临时文件失败: {e}")))?;
            if full {
                Ok(crate::ui::Snapshot {
                    tree: xml,
                    refs: Vec::new(),
                })
            } else {
                crate::ui::simplify(&xml).map_err(|e| {
                    ErrorBody::adb_error(
                        "UI 快照 XML 解析失败",
                        Some(serde_json::json!({ "error": e })),
                    )
                })
            }
        })();
        let _ = std::fs::remove_file(&tmp);
        result
    }

    fn tap(&self, device: &str, target: TapTarget) -> BResult<()> {
        match target {
            TapTarget::Coord(x, y) => self.shell_checked(device, &format!("input tap {x} {y}")),
            // Ref 解引用由 daemon executor 层完成（查最近快照的 refs 表取中心点），
            // backend 不持有快照状态，直接拒绝。
            TapTarget::Ref(_) => Err(ErrorBody::not_supported(
                "Ref 目标由 executor 层解引用为坐标后下发，backend 不直接处理",
            )),
        }
    }

    fn swipe(
        &self,
        device: &str,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        duration_ms: u32,
    ) -> BResult<()> {
        self.shell_checked(
            device,
            &format!("input swipe {x1} {y1} {x2} {y2} {duration_ms}"),
        )
    }

    fn input_text(&self, device: &str, text: &str) -> BResult<()> {
        let escaped = escape_input_text(text)?;
        self.shell_checked(device, &format!("input text {escaped}"))
    }

    fn key(&self, device: &str, key: &str) -> BResult<()> {
        // keyevent 接受名称（KEYCODE_HOME）或数字码（design.md 决策 5），
        // 限定字符集防 shell 注入。
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(ErrorBody::not_supported(format!(
                "非法 keyevent（仅允许字母数字与 _）: {key}"
            )));
        }
        self.shell_checked(device, &format!("input keyevent {key}"))
    }

    /// 截图（design.md 决策 6）：exec-out 避免 PTY 损坏二进制，写文件前校验 PNG 魔数。
    fn screenshot(&self, device: &str, out: &Path) -> BResult<PathBuf> {
        let bytes =
            self.adb
                .exec_out(device, &["screencap", "-p"], crate::adb::TRANSFER_TIMEOUT)?;
        validate_png(&bytes)?;
        std::fs::write(out, &bytes)
            .map_err(|e| ErrorBody::io_error(format!("写入截图 {} 失败: {e}", out.display())))?;
        Ok(out.to_path_buf())
    }

    fn apps(&self, device: &str, filter: Option<&str>, all: bool) -> BResult<Vec<String>> {
        let cmd = if all {
            "pm list packages"
        } else {
            "pm list packages -3"
        };
        let res = self.adb.shell(device, cmd, crate::adb::DEFAULT_TIMEOUT)?;
        if res.exit_code != 0 {
            return Err(ErrorBody::adb_error(
                format!("pm list packages 失败（退出码 {}）", res.exit_code),
                Some(serde_json::json!({
                    "stdout": res.stdout.trim(),
                    "stderr": res.stderr.trim(),
                })),
            ));
        }
        let mut pkgs = parse_packages(&res.stdout);
        if let Some(f) = filter {
            pkgs.retain(|p| p.contains(f));
        }
        Ok(pkgs)
    }

    /// 启动应用（design.md 决策 6）：monkey 免查 launcher activity。
    fn launch(&self, device: &str, package: &str) -> BResult<()> {
        validate_package(package)?;
        self.shell_checked(
            device,
            &format!("monkey -p {package} -c android.intent.category.LAUNCHER 1"),
        )
    }

    fn stop(&self, device: &str, package: &str) -> BResult<()> {
        validate_package(package)?;
        self.shell_checked(device, &format!("am force-stop {package}"))
    }

    /// 日志（design.md 决策 6）：dump 模式非阻塞；tag → -s，level → *:<level>。
    fn logcat(
        &self,
        device: &str,
        lines: u32,
        tag: Option<&str>,
        level: Option<&str>,
    ) -> BResult<String> {
        let mut args: Vec<String> =
            vec!["logcat".into(), "-d".into(), "-t".into(), lines.to_string()];
        if let Some(tag) = tag {
            args.push("-s".into());
            args.push(tag.to_string());
        }
        if let Some(level) = level {
            args.push(format!("*:{level}"));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.adb.run_on(device, &refs, crate::adb::DEFAULT_TIMEOUT)
    }

    /// shell 透传：显式 -s，返回远端真实退出码（经退出码标记包装）。
    fn shell(&self, device: &str, cmd: &[String]) -> BResult<ShellResult> {
        self.adb
            .shell(device, &cmd.join(" "), crate::adb::DEFAULT_TIMEOUT)
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
    fn ops_with_missing_adb_binary_surface_io_error() {
        // 组 4 实现后：伪 adb 路径下各操作在 spawn 阶段失败为 IoError，而非 not_supported。
        let b = backend();
        let cases: Vec<ErrorBody> = vec![
            b.snapshot("dev", false).unwrap_err(),
            b.tap("dev", TapTarget::Coord(1, 2)).unwrap_err(),
            b.swipe("dev", 0, 0, 1, 1, 100).unwrap_err(),
            b.input_text("dev", "hi").unwrap_err(),
            b.key("dev", "HOME").unwrap_err(),
            b.screenshot("dev", Path::new("/tmp/x.png")).unwrap_err(),
            b.apps("dev", None, false).unwrap_err(),
            b.launch("dev", "com.x.y").unwrap_err(),
            b.stop("dev", "com.x.y").unwrap_err(),
            b.logcat("dev", 10, None, None).unwrap_err(),
            b.shell("dev", &["ls".to_string()]).unwrap_err(),
        ];
        for e in cases {
            assert_eq!(e.code, crate::output::ErrorCode::IoError, "{e:?}");
        }
    }

    #[test]
    fn tap_ref_returns_not_supported_without_spawn() {
        // Ref 解引用由 daemon executor 层完成，backend 直接拒绝且不触发 adb 子进程。
        let e = backend()
            .tap("dev", TapTarget::Ref("@e1".into()))
            .unwrap_err();
        assert_eq!(e.code, crate::output::ErrorCode::NotSupported);
    }

    // ---- input_text ASCII 校验与转义（纯函数） ----

    #[test]
    fn escape_input_text_converts_space_to_percent_s() {
        assert_eq!(escape_input_text("hello world").unwrap(), "hello%sworld");
    }

    #[test]
    fn escape_input_text_rejects_non_ascii() {
        let e = escape_input_text("你好").unwrap_err();
        assert_eq!(e.code, crate::output::ErrorCode::NotSupported);
        assert!(e.message.contains("ASCII"));
    }

    #[test]
    fn escape_input_text_escapes_shell_metachars() {
        assert_eq!(escape_input_text("a&b;c").unwrap(), "a\\&b\\;c");
        assert_eq!(escape_input_text("$(x)").unwrap(), "\\$\\(x\\)");
    }

    #[test]
    fn escape_input_text_escapes_literal_percent() {
        // input text 自身的 % 转义体系：字面 % 必须转 %25，避免被误解析。
        assert_eq!(escape_input_text("100%").unwrap(), "100%25");
    }

    #[test]
    fn escape_input_text_rejects_control_chars() {
        let e = escape_input_text("a\nb").unwrap_err();
        assert_eq!(e.code, crate::output::ErrorCode::NotSupported);
    }

    // ---- pm list packages 解析（纯函数） ----

    #[test]
    fn parse_packages_extracts_names() {
        let text = "package:com.android.settings\npackage:com.mumu.launcher\npackage:com.google.android.gms\n";
        assert_eq!(
            parse_packages(text),
            vec![
                "com.android.settings".to_string(),
                "com.mumu.launcher".to_string(),
                "com.google.android.gms".to_string()
            ]
        );
    }

    #[test]
    fn parse_packages_skips_malformed_and_empty() {
        let text = "package:com.a\n\nerror line\npackage:\npackage:com.b\r\n";
        assert_eq!(
            parse_packages(text),
            vec!["com.a".to_string(), "com.b".to_string()]
        );
    }

    // ---- PNG magic 校验（纯函数） ----

    #[test]
    fn validate_png_accepts_magic_header() {
        let bytes = b"\x89PNG\r\n\x1a\nrest-of-payload";
        assert!(validate_png(bytes).is_ok());
    }

    #[test]
    fn validate_png_rejects_non_png_and_empty() {
        assert!(validate_png(b"not a png at all").is_err());
        assert!(validate_png(b"").is_err());
        assert!(validate_png(b"\x89PNG").is_err());
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
