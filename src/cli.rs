use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "agent-mobile-cli",
    version,
    about = "面向 Agent 的 Android 设备感知与控制 CLI"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// 枚举当前可达的 Android 设备
    Devices,
    /// 连接网络 Android 设备（host:port）
    Connect { target: String },
    /// 获取当前屏幕 UI 快照（简化树 + 元素引用）
    Snapshot {
        #[arg(long)]
        device: Option<String>,
        #[arg(long)]
        full: bool,
    },
    /// 点击（坐标或元素引用 @eN）
    Tap {
        target: String,
        #[arg(long)]
        y: Option<i32>,
        #[arg(long)]
        device: Option<String>,
    },
    /// 滑动
    Swipe {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        #[arg(long, default_value_t = 300)]
        duration: u32,
        #[arg(long)]
        device: Option<String>,
    },
    /// 输入文本（仅 ASCII）
    Input {
        text: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// 发送按键事件
    Key {
        key: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// 截取当前屏幕保存为 PNG
    Screenshot {
        #[arg(long, default_value = "screenshot.png")]
        out: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// 列出已安装应用
    Apps {
        #[arg(long)]
        filter: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        device: Option<String>,
    },
    /// 启动应用
    Launch {
        package: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// 停止应用
    Stop {
        package: String,
        #[arg(long)]
        device: Option<String>,
    },
    /// 读取 logcat
    Logcat {
        #[arg(long, default_value_t = 100)]
        lines: u32,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        level: Option<String>,
        #[arg(long)]
        device: Option<String>,
    },
    /// 在设备上执行 shell 命令
    Shell {
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,
        #[arg(long)]
        device: Option<String>,
    },
    /// 查看 daemon 状态
    DaemonStatus,
    /// 重启 daemon
    DaemonRestart,
    /// 停止 daemon
    DaemonStop,
    /// （内部）启动常驻 daemon
    #[command(hide = true)]
    Daemon,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devices() {
        let cli = Cli::try_parse_from(["agent-mobile-cli", "devices"]).unwrap();
        assert_eq!(cli.command, Command::Devices);
    }

    #[test]
    fn parses_connect() {
        let cli = Cli::try_parse_from(["agent-mobile-cli", "connect", "127.0.0.1:5555"]).unwrap();
        assert_eq!(
            cli.command,
            Command::Connect {
                target: "127.0.0.1:5555".into()
            }
        );
    }

    #[test]
    fn parses_snapshot_with_device() {
        let cli =
            Cli::try_parse_from(["agent-mobile-cli", "snapshot", "--device", "emu-1"]).unwrap();
        match cli.command {
            Command::Snapshot { device, full } => {
                assert_eq!(device.as_deref(), Some("emu-1"));
                assert!(!full);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_tap_coordinate_and_ref() {
        let cli = Cli::try_parse_from(["agent-mobile-cli", "tap", "100", "--y", "200"]).unwrap();
        match cli.command {
            Command::Tap { target, y, .. } => {
                assert_eq!(target, "100");
                assert_eq!(y, Some(200));
            }
            other => panic!("unexpected: {other:?}"),
        }
        let cli = Cli::try_parse_from(["agent-mobile-cli", "tap", "@e3"]).unwrap();
        match cli.command {
            Command::Tap { target, y, .. } => {
                assert_eq!(target, "@e3");
                assert_eq!(y, None);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_swipe() {
        let cli =
            Cli::try_parse_from(["agent-mobile-cli", "swipe", "0", "100", "200", "300"]).unwrap();
        match cli.command {
            Command::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration,
                ..
            } => {
                assert_eq!((x1, y1, x2, y2), (0, 100, 200, 300));
                assert_eq!(duration, 300);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_logcat_options() {
        let cli = Cli::try_parse_from([
            "agent-mobile-cli",
            "logcat",
            "--lines",
            "50",
            "--tag",
            "App",
            "--level",
            "E",
        ])
        .unwrap();
        match cli.command {
            Command::Logcat {
                lines, tag, level, ..
            } => {
                assert_eq!(lines, 50);
                assert_eq!(tag.as_deref(), Some("App"));
                assert_eq!(level.as_deref(), Some("E"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_shell_trailing() {
        let cli = Cli::try_parse_from(["agent-mobile-cli", "shell", "getprop", "ro.product.model"])
            .unwrap();
        match cli.command {
            Command::Shell { cmd, .. } => {
                assert_eq!(cmd, vec!["getprop", "ro.product.model"]);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_daemon_lifecycle() {
        for (sub, expected) in [
            ("daemon-status", Command::DaemonStatus),
            ("daemon-restart", Command::DaemonRestart),
            ("daemon-stop", Command::DaemonStop),
        ] {
            let cli = Cli::try_parse_from(["agent-mobile-cli", sub]).unwrap();
            assert_eq!(cli.command, expected);
        }
    }

    #[test]
    fn shell_requires_cmd() {
        assert!(Cli::try_parse_from(["agent-mobile-cli", "shell"]).is_err());
    }
}
