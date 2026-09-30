//! mobile-debug-proxy-server：公网代理中继服务（lib 承载全部逻辑，design.md 决策 8）。
//!
//! App 出站 WS 绑定到 `WS /ws/device`；CLI 侧（经本机 daemon uplink）调用 HTTP API
//! 间接调试设备。帧协议单一来源为根 crate 的 `src/bridge_proto.rs`（#[path] 引用）。

// 注意：#[path] 相对本文件所在目录解析，仓库根 src/ 需上溯两级。
pub mod auth;
#[path = "../../src/bridge_proto.rs"]
pub mod bridge_proto;
pub mod http;
pub mod relay;
pub mod store;
pub mod ws;

use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

/// 超时配置（生产默认 10s/30s/30s；测试注入毫秒级，design.md 决策 13）。
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// hello 首帧等待超时。
    pub hello_timeout: Duration,
    /// 心跳（任意消息）静默超时：超时断开并标离线。
    pub heartbeat_timeout: Duration,
    /// command/script 等待设备 result 的超时。
    pub command_timeout: Duration,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            hello_timeout: Duration::from_secs(10),
            heartbeat_timeout: Duration::from_secs(30),
            command_timeout: Duration::from_secs(30),
        }
    }
}

/// 共享状态：数据文件 + 中继注册表 + 配置。
pub struct AppState {
    pub store: store::Store,
    pub relay: relay::Relay,
    pub config: ProxyConfig,
    /// 认证注册与配对重置互斥锁（FixReview IMPORTANT-1）：
    /// WS hello 的 authenticate+register 与 pairing_reset 的 reset_owner+disconnect_owner
    /// 必须各自原子完成，防止「认证通过后 reset 落空、设备以已吊销 token 注册上线」的
    /// TOCTOU 僵尸连接。
    pub auth_lock: tokio::sync::Mutex<()>,
}

impl AppState {
    pub fn new(store: store::Store, config: ProxyConfig) -> Arc<Self> {
        Arc::new(Self {
            store,
            relay: relay::Relay::default(),
            config,
            auth_lock: tokio::sync::Mutex::new(()),
        })
    }
}

/// 在已绑定的 listener 上提供全部服务（HTTP + WS 同端口）。
pub async fn serve(listener: TcpListener, state: Arc<AppState>) -> io::Result<()> {
    axum::serve(listener, http::router(state)).await
}

/// 绑定地址并后台启动服务，返回实际地址与任务句柄（测试绑 `127.0.0.1:0` 取随机端口）。
pub async fn start(
    bind: SocketAddr,
    state: Arc<AppState>,
) -> io::Result<(SocketAddr, tokio::task::JoinHandle<io::Result<()>>)> {
    let listener = TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(serve(listener, state));
    Ok((addr, handle))
}

/// bin 入口：加载数据文件（首启生成并打印 owner token），启动服务直至退出。
pub async fn run(bind: SocketAddr, data: &Path, owner_tokens: &[String]) -> io::Result<()> {
    let (store, generated) = store::Store::open(data, owner_tokens)?;
    if let Some(token) = &generated {
        println!("首启生成 owner token（请妥善保管）: {token}");
    }
    let state = AppState::new(store, ProxyConfig::default());
    let listener = TcpListener::bind(bind).await?;
    println!("监听地址: {}", listener.local_addr()?);
    serve(listener, state).await
}

/// 进程内测试工具：随机端口起服务 + ureq HTTP 客户端封装。
#[cfg(test)]
pub(crate) mod testutil {
    use super::*;
    use serde_json::Value;
    use std::future::Future;

    /// 测试服务句柄。
    pub struct TestHandle {
        pub owner_token: String,
        pub state: Arc<AppState>,
    }

    /// 毫秒级超时，保证套件快速确定（design.md 决策 13）。
    /// 层级约束：command < heartbeat（命令超时先于静默踢线），hello 独立。
    pub fn test_config() -> ProxyConfig {
        ProxyConfig {
            hello_timeout: Duration::from_millis(500),
            heartbeat_timeout: Duration::from_millis(1500),
            command_timeout: Duration::from_millis(300),
        }
    }

    /// 起服务：临时目录数据文件 + 随机端口；返回 (地址, 句柄, 临时目录守卫)。
    pub async fn spawn_test_server() -> (SocketAddr, Arc<TestHandle>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (store, generated) =
            store::Store::open(&dir.path().join("proxy-data.json"), &[]).unwrap();
        let owner_token = generated.unwrap();
        let state = AppState::new(store, test_config());
        let (addr, _task) = start("127.0.0.1:0".parse().unwrap(), state.clone())
            .await
            .unwrap();
        let handle = Arc::new(TestHandle { owner_token, state });
        (addr, handle, dir)
    }

    /// 阻塞 ureq 调用放进 blocking 池，避免卡住 current_thread runtime。
    async fn http_blocking(f: impl FnOnce() -> (u16, Value) + Send + 'static) -> (u16, Value) {
        tokio::task::spawn_blocking(f).await.unwrap()
    }

    /// ureq 响应统一处理：非 2xx 也读取 body；解析失败暴露原始文本与 URL 便于诊断。
    fn ureq_result_for(url: &str, result: Result<ureq::Response, ureq::Error>) -> (u16, Value) {
        fn parse(url: &str, resp: ureq::Response) -> (u16, Value) {
            let status = resp.status();
            let text = resp.into_string().unwrap_or_default();
            let body = serde_json::from_str(&text).unwrap_or_else(|e| {
                panic!("响应体解析失败: {url} status={status} raw={text:?} err={e}")
            });
            (status, body)
        }
        match result {
            Ok(resp) => parse(url, resp),
            Err(ureq::Error::Status(_, resp)) => parse(url, resp),
            Err(e) => panic!("HTTP 请求失败: {url} {e}"),
        }
    }

    /// GET（返回 'static future，可 tokio::spawn 并发驱动）。
    pub fn http_get(
        addr: SocketAddr,
        path: &str,
        token: Option<&str>,
    ) -> impl Future<Output = (u16, Value)> + Send + 'static {
        let url = format!("http://{addr}{path}");
        let token = token.map(str::to_string);
        async move {
            http_blocking(move || {
                // 每次新建 Agent：ureq 顶层函数共享进程级全局 Agent（连接池），
                // 测试服务器随用例结束销毁、临时端口被 OS 复用后，池化 keep-alive
                // 连接会命中新服务器返回空响应（并行压测间歇复现：200 + 空 body）。
                let mut req = ureq::AgentBuilder::new().build().get(&url);
                if let Some(t) = token {
                    req = req.set("Authorization", &format!("Bearer {t}"));
                }
                ureq_result_for(&url, req.call())
            })
            .await
        }
    }

    /// POST JSON（返回 'static future，可 tokio::spawn 并发驱动）。
    pub fn http_post(
        addr: SocketAddr,
        path: &str,
        token: Option<&str>,
        body: Value,
    ) -> impl Future<Output = (u16, Value)> + Send + 'static {
        let url = format!("http://{addr}{path}");
        let token = token.map(str::to_string);
        async move {
            http_blocking(move || {
                let mut req = ureq::AgentBuilder::new().build().post(&url);
                if let Some(t) = token {
                    req = req.set("Authorization", &format!("Bearer {t}"));
                }
                ureq_result_for(&url, req.send_json(body))
            })
            .await
        }
    }

    /// WS 客户端（模拟 App）。
    pub type WsClient = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    /// 连接 /ws/device。
    pub async fn ws_connect(addr: SocketAddr) -> WsClient {
        let (ws, _resp) = tokio_tungstenite::connect_async(format!("ws://{addr}/ws/device"))
            .await
            .expect("WS 连接失败");
        ws
    }

    /// 发送 JSON 文本帧。
    pub async fn ws_send<T: serde::Serialize>(ws: &mut WsClient, msg: &T) {
        use futures_util::SinkExt;
        ws.send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(msg).unwrap(),
        ))
        .await
        .expect("WS 发送失败");
    }

    /// 在超时内等待一条 JSON 文本帧；连接关闭或超时返回 None。
    pub async fn ws_recv<T: serde::de::DeserializeOwned>(
        ws: &mut WsClient,
        wait: Duration,
    ) -> Option<T> {
        use futures_util::StreamExt;
        match tokio::time::timeout(wait, ws.next()).await {
            Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) => {
                serde_json::from_str(&text).ok()
            }
            _ => None,
        }
    }
}
