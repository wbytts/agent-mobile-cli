//! 薄 bin：仅解析参数并启动 lib（design.md 决策 8）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

/// 命令行参数。
struct Cli {
    /// 监听地址（默认 0.0.0.0:28777）。
    bind: SocketAddr,
    /// 数据文件路径（默认 ./proxy-data.json）。
    data: PathBuf,
    /// 追加注入的 owner token（可重复）。
    owner_tokens: Vec<String>,
}

const DEFAULT_BIND: &str = "0.0.0.0:28777";
const DEFAULT_DATA: &str = "./proxy-data.json";

fn usage() -> String {
    format!(
        "mobile-debug-proxy-server — 公网代理中继服务\n\n\
         用法: mobile-debug-proxy-server [选项]\n\n\
         选项:\n\
         \x20 --bind <addr>         监听地址（默认 {DEFAULT_BIND}）\n\
         \x20 --data <path>         数据文件路径（默认 {DEFAULT_DATA}）\n\
         \x20 --owner-token <hex>   追加注入 owner token（可重复）\n\
         \x20 -h, --help            显示帮助"
    )
}

/// 解析参数（支持 `--key value` 与 `--key=value` 两种形式）。
fn parse_args<I: Iterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut bind = DEFAULT_BIND.to_string();
    let mut data = DEFAULT_DATA.to_string();
    let mut owner_tokens = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let (key, inline_val) = match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (arg, None),
        };
        let mut take = |key: &str| -> Result<String, String> {
            if let Some(v) = inline_val.clone() {
                return Ok(v);
            }
            args.next().ok_or_else(|| format!("选项 {key} 缺少参数值"))
        };
        match key.as_str() {
            "--bind" => bind = take("--bind")?,
            "--data" => data = take("--data")?,
            "--owner-token" => owner_tokens.push(take("--owner-token")?),
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("未知选项 {other}\n\n{}", usage())),
        }
    }
    let bind: SocketAddr = bind
        .parse()
        .map_err(|e| format!("--bind 地址无效: {bind} ({e})"))?;
    Ok(Cli {
        bind,
        data: PathBuf::from(data),
        owner_tokens,
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match parse_args(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::from(2);
        }
    };
    match mobile_debug_proxy_server::run(cli.bind, &cli.data, &cli.owner_tokens).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("服务异常退出: {e}");
            ExitCode::FAILURE
        }
    }
}
