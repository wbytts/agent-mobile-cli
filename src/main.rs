mod adb;
mod backend;
mod cli;
mod config;
mod daemon;
mod exec;
mod output;
mod ui;

use clap::Parser;
use output::{ErrorCode, Output};

fn main() {
    let cli = cli::Cli::parse();
    let out = run(cli.command);
    out.print();
    std::process::exit(out.exit_code());
}

fn run(command: cli::Command) -> Output {
    use cli::Command::*;
    let cfg = match config::Config::load_or_create() {
        Ok(cfg) => cfg,
        Err(e) => {
            return Output::failure(ErrorCode::IoError, format!("配置文件初始化失败: {e}"), None)
        }
    };
    match command {
        Daemon => match daemon::run(&cfg) {
            Ok(()) => Output::success(serde_json::json!({ "stopped": true })),
            Err(e) => err_output(e),
        },
        DaemonStatus => match daemon::status(&cfg) {
            Ok(status) => Output::success(status),
            Err(e) => err_output(e),
        },
        DaemonStop => match daemon::stop(&cfg) {
            Ok(()) => Output::success(serde_json::json!({ "stopped": true })),
            Err(e) => err_output(e),
        },
        DaemonRestart => {
            if daemon::health_check(cfg.http_port) {
                if let Err(e) = daemon::stop(&cfg) {
                    return err_output(e);
                }
                // 等待旧 daemon 退出：以「锁文件消失或锁内 pid 退出」为准（每 100ms 轮询，≤10s）。
                // 健康检查在 /cmd 有在飞长命令时会持续通过，不能作为退出判据。
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                loop {
                    let exited = match daemon::lock::read_lock(&config::Config::dir()) {
                        None => true,
                        Some(info) => !daemon::lock::pid_alive(info.pid),
                    };
                    if exited {
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        return Output::failure(
                            ErrorCode::Timeout,
                            "旧 daemon 未在 10 秒内退出（可能有长命令在飞，可稍后重试 daemon-status 确认）",
                            None,
                        );
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            match daemon::ensure_daemon(&cfg).and_then(|()| daemon::status(&cfg)) {
                Ok(status) => Output::success(status),
                Err(e) => err_output(e),
            }
        }
        _ => {
            // 设备命令：确保 daemon 后把原始参数转发给 daemon 执行（design.md 决策 1/2）
            if let Err(e) = daemon::ensure_daemon(&cfg) {
                return err_output(e);
            }
            let args: Vec<String> = std::env::args().skip(1).collect();
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match daemon::post_cmd(cfg.http_port, &args, &cwd) {
                Ok(out) => out,
                Err(e) => err_output(e),
            }
        }
    }
}

fn err_output(e: output::ErrorBody) -> Output {
    Output::failure(e.code, e.message, e.details)
}
