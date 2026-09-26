mod adb;
mod backend;
mod cli;
mod config;
mod daemon;
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
                // 等待旧 daemon 退出（健康检查转为失败），避免 ensure 复用到正在关闭的实例
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while daemon::health_check(cfg.http_port) {
                    if std::time::Instant::now() >= deadline {
                        return Output::failure(
                            ErrorCode::Timeout,
                            "旧 daemon 未在 5 秒内退出",
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
            // 其他命令先自动确保 daemon（任务 2.2）；实际执行由组 3/4 接入
            if let Err(e) = daemon::ensure_daemon(&cfg) {
                return err_output(e);
            }
            Output::failure(ErrorCode::NotSupported, "该命令将在后续任务实现", None)
        }
    }
}

fn err_output(e: output::ErrorBody) -> Output {
    Output::failure(e.code, e.message, e.details)
}
