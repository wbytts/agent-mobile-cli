mod cli;
mod config;
mod output;

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
    if let Err(e) = config::Config::load_or_create() {
        return Output::failure(ErrorCode::IoError, format!("配置文件初始化失败: {e}"), None);
    }
    match command {
        Daemon => Output::failure(ErrorCode::NotSupported, "daemon 将在任务 2.x 实现", None),
        _ => Output::failure(ErrorCode::NotSupported, "该命令将在后续任务实现", None),
    }
}
