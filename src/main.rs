mod adb;
mod backend;
mod bridge_proto;
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
        Pair { reset } => {
            // 配对信息管理命令：确保 daemon 后读本机管理端点（design.md 决策 8）
            if let Err(e) = daemon::ensure_daemon(&cfg) {
                return err_output(e);
            }
            match daemon::pair(&cfg, reset) {
                Ok(info) => Output::success(info),
                Err(e) => err_output(e),
            }
        }
        DaemonRestart => {
            if daemon::health_check(cfg.http_port) {
                if let Err(e) = daemon::stop(&cfg) {
                    return err_output(e);
                }
                if let Err(e) = daemon::wait_daemon_gone_in(
                    &config::Config::dir(),
                    std::time::Duration::from_secs(10),
                ) {
                    return err_output(e);
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
