//! App 侧桥接 WS 客户端（design.md 决策 7/8/10/11，任务组 4）。
//!
//! 分层：
//! - 纯协议核心 [`Session`]：hello 构造（token 优先）、hello_ack 处理、
//!   command/script 分发回传、状态机——无 I/O，host 可单测。
//! - 异步传输循环 [`run`]：tokio-tungstenite 连接、心跳、指数退避重连、
//!   Tauri 事件推送（`bridge://state` / `bridge://log` / `bridge://heartbeat`）。
//! - [`BridgeController`]：Tauri 命令侧的连接开关（connect/disconnect）。

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::bridge_proto::{
    Capability, CommandMessage, Hello, HelloAck, ResultMessage, ScriptMessage,
};
use crate::sandbox::{MobileOps, Sandbox};

/// 心跳间隔。daemon 端 HEARTBEAT_TIMEOUT 为 30s（src/daemon/registry.rs），
/// 心跳周期必须低于该值留出网络余量，否则空闲连接会被误标 offline。
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);
/// 断线重连退避上限（1s 起指数翻倍）。
pub const BACKOFF_MAX_SECS: u64 = 30;

/// hello 上报的能力集（design.md 决策 6/12；与协议 Capability 枚举一一对应）。
pub const CAPABILITIES: [Capability; 9] = [
    Capability::Tap,
    Capability::Swipe,
    Capability::Input,
    Capability::Key,
    Capability::UiTree,
    Capability::Screenshot,
    Capability::Apps,
    Capability::Launch,
    Capability::Script,
];

/// 连接状态机，经 `bridge://state` 事件推给 UI。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected {
        device_id: String,
    },
    /// 配对/认证失败（终态：daemon 已断开，等待用户修正配对码或重新扫码）。
    Pairing {
        reason: String,
    },
}

/// 连接/命令日志事件载荷（`bridge://log`）。
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub level: String,
    pub message: String,
}

/// 扫码配对 URI 解析结果（`agent-mobile://pair?host=..&port=..&code=..`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairInfo {
    pub host: String,
    pub port: u16,
    pub code: String,
}

/// 已保存的 daemon 地址（连接页回填 / 冷启动自动连接）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavedAddress {
    pub host: String,
    pub port: u16,
}

/// 冷启动自动连接触发条件：存在上次成功连接的地址 → Some((host, port))，否则 None。
/// 无 token 时连接会被 daemon 拒入 Pairing 态，等用户输配对码——符合预期。
pub fn auto_connect_target(platform: &Arc<dyn BridgePlatform>) -> Option<(String, u16)> {
    let (host, port) = platform.load_address()?;
    (!host.is_empty() && port > 0).then_some((host, port))
}

/// App 侧平台桥：设备信息、token 私有存储（SharedPreferences）、前台服务保活、扫码。
/// Android 实现经 Tauri 插件调到 Kotlin；host 与单测用内存实现。
pub trait BridgePlatform: Send + Sync {
    fn device_name(&self) -> String;
    fn android_version(&self) -> String;
    /// 按 host:port 读取已配对 token；未配对返回 None。
    fn load_token(&self, host: &str, port: u16) -> Option<String>;
    fn save_token(&self, host: &str, port: u16, token: &str);
    /// 前台服务保活开关：连接开启时启动，断开/配对失败时停止。
    fn set_foreground(&self, running: bool);
    /// 上次成功连接的 daemon 地址（冷启动自动连接 + 连接页回填用）。
    fn load_address(&self) -> Option<(String, u16)>;
    fn save_address(&self, host: &str, port: u16);
    /// 清除该 host:port 的已存 token（认证被拒=失效证据，清除后下次 hello 走配对码）。
    fn delete_token(&self, host: &str, port: u16);
    /// 触发相机扫码（Kotlin 扫码页），返回扫描内容；用户取消/权限被拒返回 Err。
    fn scan_pair_qr(&self) -> Result<String, String>;
}

/// 一次桥接会话的纯协议核心（无 I/O，可单测）。
pub struct Session {
    platform: Arc<dyn BridgePlatform>,
    ops: Arc<dyn MobileOps>,
    host: String,
    port: u16,
    pairing_code: Option<String>,
    state: ConnState,
}

impl Session {
    pub fn new(
        platform: Arc<dyn BridgePlatform>,
        ops: Arc<dyn MobileOps>,
        host: String,
        port: u16,
        pairing_code: Option<String>,
    ) -> Self {
        Self {
            platform,
            ops,
            host,
            port,
            pairing_code,
            state: ConnState::Disconnected,
        }
    }

    pub fn state(&self) -> &ConnState {
        &self.state
    }

    pub fn ops(&self) -> Arc<dyn MobileOps> {
        self.ops.clone()
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// 桥接设备 id：`bridge:<device_name>`（协议约定）。
    pub fn device_id(&self) -> String {
        format!("bridge:{}", self.platform.device_name())
    }

    /// 构造 hello：已存 token 优先；无 token 用配对码（design.md 决策 8）。
    pub fn hello(&self) -> Hello {
        let token = self.platform.load_token(&self.host, self.port);
        let pairing_code = if token.is_some() {
            None
        } else {
            self.pairing_code.clone()
        };
        Hello {
            pairing_code,
            token,
            device_name: self.platform.device_name(),
            android_version: self.platform.android_version(),
            capabilities: CAPABILITIES.to_vec(),
        }
    }

    /// 处理 hello_ack：成功→Connected 并持久化 token 与地址；失败→Pairing{reason}。
    pub fn on_hello_ack(&mut self, ack: &HelloAck) -> ConnState {
        if ack.ok {
            if let Some(token) = &ack.token {
                self.platform.save_token(&self.host, self.port, token);
            }
            // 连接成功才持久化地址（避免保存用户输错的地址）
            self.platform.save_address(&self.host, self.port);
            self.state = ConnState::Connected {
                device_id: self.device_id(),
            };
        } else {
            // 协议上 hello 的拒绝原因均为认证类（配对码错误/失效、token 无效、缺少凭证），
            // 见 docs/bridge-protocol.md hello_ack error 字段说明。token 被服务端明确拒绝
            // 即失效证据：清除已存 token，下次 hello 自然走配对码（修复 pair --reset 后
            // 旧 token 反复抢占 hello 导致配对码永远轮不到的死锁）。
            // 若本次 hello 本就未带 token（配对码路径被拒），delete_token 是无害 no-op。
            self.platform.delete_token(&self.host, self.port);
            self.state = ConnState::Pairing {
                reason: ack
                    .error
                    .clone()
                    .unwrap_or_else(|| "认证失败（无错误描述）".to_string()),
            };
        }
        self.state.clone()
    }

    /// command 分发（同步阻塞 Kotlin 桥；异步上下文调用方须 spawn_blocking）。
    pub fn handle_command(&self, cmd: &CommandMessage) -> ResultMessage {
        handle_command(&self.ops, cmd)
    }

    /// script 下发：QuickJS 沙盒执行（同样阻塞）。
    pub fn handle_script(&self, msg: &ScriptMessage) -> ResultMessage {
        handle_script(&self.ops, msg)
    }
}

/// command{id,method,params} → 按 method 分发到设备能力 → result{id,ok,...}。
pub fn handle_command(ops: &Arc<dyn MobileOps>, cmd: &CommandMessage) -> ResultMessage {
    match dispatch(ops, &cmd.method, &cmd.params) {
        Ok(value) => ResultMessage {
            id: cmd.id.clone(),
            ok: true,
            result: Some(value),
            error: None,
        },
        Err(error) => ResultMessage {
            id: cmd.id.clone(),
            ok: false,
            result: None,
            error: Some(error),
        },
    }
}

/// script{id,source} → Sandbox 执行 → script_result（ok/异常回传）。
/// Sandbox 默认 25s 执行超时 + 64MB 内存上限（sandbox.rs），死循环/爆内存脚本
/// 会在 daemon 心跳超时（30s）前中断并回传结构化错误，spawn_blocking 不会永久卡住。
pub fn handle_script(ops: &Arc<dyn MobileOps>, msg: &ScriptMessage) -> ResultMessage {
    match Sandbox::new(ops.clone()).run(&msg.source) {
        Ok(value) => ResultMessage {
            id: msg.id.clone(),
            ok: true,
            result: Some(value),
            error: None,
        },
        Err(error) => ResultMessage {
            id: msg.id.clone(),
            ok: false,
            result: None,
            error: Some(error),
        },
    }
}

/// 按 method 分发到 MobileOps（契约见 docs/bridge-protocol.md「command method 参数契约」）。
fn dispatch(ops: &Arc<dyn MobileOps>, method: &str, params: &Value) -> Result<Value, String> {
    let text = match method {
        "tap" => ops.tap(num(params, "x")?, num(params, "y")?),
        "swipe" => ops.swipe(
            num(params, "x1")?,
            num(params, "y1")?,
            num(params, "x2")?,
            num(params, "y2")?,
            num(params, "duration_ms")?,
        ),
        "input" => ops.input(text(params, "text")?),
        "key" => ops.key(text(params, "key")?),
        "uiTree" => ops.ui_tree(),
        "screenshot" => ops.screenshot(),
        "apps" => ops.apps(
            params
                .get("filter")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty()),
            params.get("all").and_then(Value::as_bool).unwrap_or(false),
        ),
        "launch" => ops.launch(text(params, "package")?),
        other => return Err(format!("不支持的命令 method: {other}")),
    }?;
    serde_json::from_str(&text).map_err(|e| format!("插件返回非法 JSON: {e}"))
}

fn num(params: &Value, key: &str) -> Result<f64, String> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("参数 {key} 缺失或不是数字"))
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("参数 {key} 缺失或不是字符串"))
}

/// 断线重连退避：1s 起指数翻倍，30s 封顶（attempt 为已失败次数，从 0 起）。
pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64 << attempt.min(5);
    Duration::from_secs(secs.min(BACKOFF_MAX_SECS))
}

/// 解析扫码得到的配对 URI（design.md 决策 8/13）。
pub fn parse_pair_uri(uri: &str) -> Result<PairInfo, String> {
    let parsed = url::Url::parse(uri.trim()).map_err(|e| format!("配对 URI 解析失败: {e}"))?;
    if parsed.scheme() != "agent-mobile" {
        return Err(format!("不是配对 URI（scheme={}）", parsed.scheme()));
    }
    if parsed.host_str() != Some("pair") {
        return Err("不是配对 URI（缺少 pair 路径）".to_string());
    }
    let param = |key: &str| -> Result<String, String> {
        parsed
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .ok_or_else(|| format!("配对 URI 缺少参数 {key}"))
    };
    let host = param("host")?;
    if host.is_empty() {
        return Err("配对 URI 的 host 为空".to_string());
    }
    let port: u16 = param("port")?
        .parse()
        .map_err(|_| "配对 URI 的 port 不是有效端口".to_string())?;
    let code = param("code")?;
    if code.is_empty() {
        return Err("配对 URI 的 code 为空".to_string());
    }
    Ok(PairInfo { host, port, code })
}

// ---------- 异步传输循环（连接/心跳/重连；Android 运行时使用，核心逻辑由上方单测覆盖） ----------

use futures_util::{SinkExt, StreamExt};
use tauri::{AppHandle, Emitter};
use tokio_tungstenite::tungstenite::Message;

use crate::bridge_proto::ClientMessage;
use crate::bridge_proto::DaemonMessage;

/// `bridge://state` 事件名（连接状态机推送）。
pub const EVENT_STATE: &str = "bridge://state";
/// `bridge://log` 事件名（连接事件与命令执行记录）。
pub const EVENT_LOG: &str = "bridge://log";
/// `bridge://heartbeat` 事件名（心跳应答，UI 刷新「最近心跳」）。
pub const EVENT_HEARTBEAT: &str = "bridge://heartbeat";

/// Tauri 命令侧的连接控制器：持有共享状态与运行中的连接任务。
pub struct BridgeController {
    ops: Arc<dyn MobileOps>,
    platform: Arc<dyn BridgePlatform>,
    state: Arc<parking_lot::Mutex<ConnState>>,
    task: parking_lot::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl BridgeController {
    pub fn new(ops: Arc<dyn MobileOps>, platform: Arc<dyn BridgePlatform>) -> Self {
        Self {
            ops,
            platform,
            state: Arc::new(parking_lot::Mutex::new(ConnState::Disconnected)),
            task: parking_lot::Mutex::new(None),
        }
    }

    pub fn platform(&self) -> Arc<dyn BridgePlatform> {
        self.platform.clone()
    }

    pub fn state(&self) -> ConnState {
        self.state.lock().clone()
    }

    /// 启动连接任务；已有任务先中止。连接参数校验失败返回 Err。
    pub fn connect(
        &self,
        app: AppHandle,
        host: String,
        port: u16,
        pairing_code: Option<String>,
    ) -> Result<(), String> {
        let host = host.trim().to_string();
        if host.is_empty() {
            return Err("daemon 地址不能为空".to_string());
        }
        if port == 0 {
            return Err("端口无效".to_string());
        }
        let pairing_code = pairing_code.and_then(|c| {
            let c = c.trim().to_string();
            (!c.is_empty()).then_some(c)
        });
        self.abort_task();
        emit_log(&app, "info", format!("连接 ws://{host}:{port}/ws"));
        let handle = tauri::async_runtime::spawn(run(
            app,
            self.platform.clone(),
            self.ops.clone(),
            host,
            port,
            pairing_code,
            self.state.clone(),
        ));
        *self.task.lock() = Some(handle);
        Ok(())
    }

    /// 断开连接：中止任务、停止前台服务、状态归位 Disconnected。
    pub fn disconnect(&self, app: &AppHandle) {
        self.abort_task();
        self.platform.set_foreground(false);
        set_state(app, &self.state, ConnState::Disconnected);
        emit_log(app, "info", "已断开连接");
    }

    fn abort_task(&self) {
        if let Some(handle) = self.task.lock().take() {
            handle.abort();
        }
    }
}

fn set_state(app: &AppHandle, shared: &Arc<parking_lot::Mutex<ConnState>>, state: ConnState) {
    *shared.lock() = state.clone();
    let _ = app.emit(EVENT_STATE, state);
}

fn emit_log(app: &AppHandle, level: &str, message: impl Into<String>) {
    let _ = app.emit(
        EVENT_LOG,
        LogEntry {
            level: level.to_string(),
            message: message.into(),
        },
    );
}

/// 单次连接会话的结果。
enum ServeOutcome {
    /// 认证被拒（hello_ack ok=false）：终态，不再重试。
    AuthFailed,
    /// 连接失败或断开：按退避重连。
    Disconnected { reason: String, was_connected: bool },
}

/// 连接+自动重连主循环：前台服务保活、hello、心跳、命令循环。
/// 仅被外层 abort（用户断开）或认证失败（终态）结束。
async fn run(
    app: AppHandle,
    platform: Arc<dyn BridgePlatform>,
    ops: Arc<dyn MobileOps>,
    host: String,
    port: u16,
    pairing_code: Option<String>,
    shared_state: Arc<parking_lot::Mutex<ConnState>>,
) {
    platform.set_foreground(true);
    let mut session = Session::new(platform.clone(), ops, host, port, pairing_code);
    let mut attempt = 0u32;
    loop {
        set_state(&app, &shared_state, ConnState::Connecting);
        match serve_once(&app, &shared_state, &mut session).await {
            ServeOutcome::AuthFailed => {
                platform.set_foreground(false);
                return;
            }
            ServeOutcome::Disconnected {
                reason,
                was_connected,
            } => {
                emit_log(&app, "err", format!("连接断开：{reason}"));
                if was_connected {
                    attempt = 0;
                }
                let delay = backoff_delay(attempt);
                attempt = attempt.saturating_add(1);
                emit_log(&app, "info", format!("{} 秒后自动重连", delay.as_secs()));
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn send_json<S>(ws: &mut S, msg: &ClientMessage) -> Result<(), String>
where
    S: SinkExt<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    let text = serde_json::to_string(msg).map_err(|e| format!("序列化失败: {e}"))?;
    ws.send(Message::Text(text))
        .await
        .map_err(|e| format!("发送失败: {e}"))
}

/// 单次连接生命周期：WS 握手 → hello → hello_ack → 心跳+命令循环 → 断开。
async fn serve_once(
    app: &AppHandle,
    shared_state: &Arc<parking_lot::Mutex<ConnState>>,
    session: &mut Session,
) -> ServeOutcome {
    let url = format!("ws://{}:{}/ws", session.host(), session.port());
    let (mut ws, _) = match tokio_tungstenite::connect_async(&url).await {
        Ok(v) => v,
        Err(e) => {
            return ServeOutcome::Disconnected {
                reason: format!("WS 握手失败: {e}"),
                was_connected: false,
            }
        }
    };
    let hello = ClientMessage::Hello(session.hello());
    if let Err(e) = send_json(&mut ws, &hello).await {
        return ServeOutcome::Disconnected {
            reason: e,
            was_connected: false,
        };
    }
    let mut was_connected = false;
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await; // interval 首跳立即完成，丢弃以对齐周期
    loop {
        tokio::select! {
            inbound = ws.next() => { match inbound {
                Some(Ok(Message::Text(text))) => {
                    match serde_json::from_str::<DaemonMessage>(&text) {
                        Ok(DaemonMessage::HelloAck(ack)) => {
                            let state = session.on_hello_ack(&ack);
                            set_state(app, shared_state, state.clone());
                            match state {
                                ConnState::Connected { device_id } => {
                                    was_connected = true;
                                    emit_log(app, "ok", format!("已连接并注册：{device_id}"));
                                }
                                ConnState::Pairing { reason } => {
                                    emit_log(app, "err", format!("认证失败：{reason}"));
                                    return ServeOutcome::AuthFailed;
                                }
                                _ => {}
                            }
                        }
                        Ok(DaemonMessage::Command(cmd)) => {
                            emit_log(app, "info", format!("收到命令 {}（{}）", cmd.id, cmd.method));
                            let ops = session.ops();
                            let result = tauri::async_runtime::spawn_blocking(move || {
                                handle_command(&ops, &cmd)
                            })
                            .await;
                            was_connected = true;
                            match result {
                                Ok(result) => {
                                    log_result(app, &result);
                                    if let Err(e) = send_json(&mut ws, &ClientMessage::Result(result)).await {
                                        return ServeOutcome::Disconnected { reason: e, was_connected };
                                    }
                                }
                                Err(e) => emit_log(app, "err", format!("命令执行中断: {e}")),
                            }
                        }
                        Ok(DaemonMessage::Script(script)) => {
                            emit_log(app, "info", format!("收到脚本 {}", script.id));
                            let ops = session.ops();
                            let result = tauri::async_runtime::spawn_blocking(move || {
                                handle_script(&ops, &script)
                            })
                            .await;
                            was_connected = true;
                            match result {
                                Ok(result) => {
                                    log_result(app, &result);
                                    if let Err(e) = send_json(&mut ws, &ClientMessage::Result(result)).await {
                                        return ServeOutcome::Disconnected { reason: e, was_connected };
                                    }
                                }
                                Err(e) => emit_log(app, "err", format!("脚本执行中断: {e}")),
                            }
                        }
                        Ok(DaemonMessage::Pong) => {
                            let _ = app.emit(EVENT_HEARTBEAT, ());
                        }
                        Ok(DaemonMessage::ResultAck(_)) => {}
                        Err(e) => emit_log(app, "err", format!("收到无法解析的帧: {e}")),
                    }
                }
                Some(Ok(_)) => {} // ping/binary 等由 tungstenite 处理（ping 自动回 pong）
                Some(Err(e)) => {
                    return ServeOutcome::Disconnected {
                        reason: format!("WS 读取失败: {e}"),
                        was_connected,
                    }
                }
                None => {
                    return ServeOutcome::Disconnected {
                        reason: "对端关闭连接".to_string(),
                        was_connected,
                    }
                }
            } }
            _tick = heartbeat.tick() => {
                if let Err(e) = send_json(&mut ws, &ClientMessage::Heartbeat).await {
                    return ServeOutcome::Disconnected { reason: e, was_connected };
                }
            }
        }
    }
}

fn log_result(app: &AppHandle, result: &ResultMessage) {
    if result.ok {
        emit_log(app, "ok", format!("{} 执行成功", result.id));
    } else {
        emit_log(
            app,
            "err",
            format!(
                "{} 执行失败：{}",
                result.id,
                result.error.as_deref().unwrap_or("未知错误")
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use serde_json::json;

    /// 记录调用、可注入失败的 MobileOps mock。
    struct MockOps {
        calls: Mutex<Vec<String>>,
        fail: Option<String>,
    }

    impl MockOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail: None,
            }
        }

        fn failing(msg: &str) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                fail: Some(msg.to_string()),
            }
        }

        fn record(&self, entry: String) -> Result<String, String> {
            self.calls.lock().push(entry);
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok("{}".to_string()),
            }
        }
    }

    impl MobileOps for MockOps {
        fn tap(&self, x: f64, y: f64) -> Result<String, String> {
            self.record(format!("tap:{x}:{y}"))
        }
        fn swipe(
            &self,
            x1: f64,
            y1: f64,
            x2: f64,
            y2: f64,
            duration_ms: f64,
        ) -> Result<String, String> {
            self.record(format!("swipe:{x1}:{y1}:{x2}:{y2}:{duration_ms}"))
        }
        fn input(&self, text: &str) -> Result<String, String> {
            self.record(format!("input:{text}"))
        }
        fn key(&self, key: &str) -> Result<String, String> {
            self.record(format!("key:{key}"))
        }
        fn ui_tree(&self) -> Result<String, String> {
            self.calls.lock().push("uiTree".to_string());
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(r#"{"xml":"<hierarchy/>"}"#.to_string()),
            }
        }
        fn screenshot(&self) -> Result<String, String> {
            self.calls.lock().push("screenshot".to_string());
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(r#"{"png_base64":"QUJD"}"#.to_string()),
            }
        }
        fn apps(&self, filter: Option<&str>, all: bool) -> Result<String, String> {
            self.calls.lock().push(format!("apps:{filter:?}:{all}"));
            match &self.fail {
                Some(e) => Err(e.clone()),
                None => Ok(r#"{"packages":["a.b","c.d"]}"#.to_string()),
            }
        }
        fn launch(&self, package: &str) -> Result<String, String> {
            self.record(format!("launch:{package}"))
        }
        fn a11y_status(&self) -> Result<String, String> {
            Ok(r#"{"enabled":true}"#.to_string())
        }
        fn open_a11y_settings(&self) -> Result<String, String> {
            Ok("{}".to_string())
        }
    }

    /// 内存 BridgePlatform（token/地址存储与前台服务开关均可断言）。
    struct MemoryPlatform {
        tokens: Mutex<std::collections::HashMap<String, String>>,
        address: Mutex<Option<(String, u16)>>,
        foreground: Mutex<Vec<bool>>,
    }

    impl MemoryPlatform {
        fn new() -> Self {
            Self {
                tokens: Mutex::new(std::collections::HashMap::new()),
                address: Mutex::new(None),
                foreground: Mutex::new(Vec::new()),
            }
        }

        fn with_token(host: &str, port: u16, token: &str) -> Self {
            let p = Self::new();
            p.save_token(host, port, token);
            p
        }
    }

    impl BridgePlatform for MemoryPlatform {
        fn device_name(&self) -> String {
            "MuMu".to_string()
        }
        fn android_version(&self) -> String {
            "12".to_string()
        }
        fn load_token(&self, host: &str, port: u16) -> Option<String> {
            self.tokens.lock().get(&format!("{host}:{port}")).cloned()
        }
        fn save_token(&self, host: &str, port: u16, token: &str) {
            self.tokens
                .lock()
                .insert(format!("{host}:{port}"), token.to_string());
        }
        fn delete_token(&self, host: &str, port: u16) {
            self.tokens.lock().remove(&format!("{host}:{port}"));
        }
        fn load_address(&self) -> Option<(String, u16)> {
            self.address.lock().clone()
        }
        fn save_address(&self, host: &str, port: u16) {
            *self.address.lock() = Some((host.to_string(), port));
        }
        fn set_foreground(&self, running: bool) {
            self.foreground.lock().push(running);
        }
        fn scan_pair_qr(&self) -> Result<String, String> {
            Err("扫码仅 Android 平台可用".to_string())
        }
    }

    fn session(platform: Arc<MemoryPlatform>, ops: Arc<MockOps>, code: Option<&str>) -> Session {
        Session::new(
            platform,
            ops,
            "192.168.1.10".to_string(),
            18777,
            code.map(str::to_string),
        )
    }

    #[test]
    fn hello_prefers_stored_token_over_pairing_code() {
        let platform = Arc::new(MemoryPlatform::with_token(
            "192.168.1.10",
            18777,
            "ab".repeat(32).as_str(),
        ));
        let s = session(platform, Arc::new(MockOps::new()), Some("483920"));
        let hello = s.hello();
        assert_eq!(hello.token.as_deref(), Some("ab".repeat(32).as_str()));
        assert_eq!(hello.pairing_code, None, "有 token 时不得提交配对码");
        assert_eq!(hello.device_name, "MuMu");
        assert_eq!(hello.android_version, "12");
        assert_eq!(hello.capabilities.len(), 9);
    }

    #[test]
    fn hello_uses_pairing_code_when_no_token() {
        let s = session(
            Arc::new(MemoryPlatform::new()),
            Arc::new(MockOps::new()),
            Some("483920"),
        );
        let hello = s.hello();
        assert_eq!(hello.pairing_code.as_deref(), Some("483920"));
        assert_eq!(hello.token, None);
    }

    #[test]
    fn hello_ack_ok_stores_token_and_enters_connected() {
        let platform = Arc::new(MemoryPlatform::new());
        let mut s = session(platform.clone(), Arc::new(MockOps::new()), Some("483920"));
        let state = s.on_hello_ack(&HelloAck {
            ok: true,
            token: Some("cd".repeat(32)),
            error: None,
        });
        assert_eq!(
            state,
            ConnState::Connected {
                device_id: "bridge:MuMu".to_string()
            }
        );
        assert_eq!(
            platform.load_token("192.168.1.10", 18777).as_deref(),
            Some("cd".repeat(32).as_str()),
            "新签发 token 必须持久化"
        );
        assert_eq!(
            platform.load_address(),
            Some(("192.168.1.10".to_string(), 18777)),
            "连接成功必须持久化 daemon 地址（冷启动自动连接用）"
        );
    }

    #[test]
    fn address_persistence_roundtrip() {
        let platform = MemoryPlatform::new();
        assert_eq!(platform.load_address(), None, "初始无保存地址");
        platform.save_address("10.0.2.2", 18888);
        assert_eq!(
            platform.load_address(),
            Some(("10.0.2.2".to_string(), 18888))
        );
        platform.save_address("192.168.2.38", 18777);
        assert_eq!(
            platform.load_address(),
            Some(("192.168.2.38".to_string(), 18777)),
            "重复保存应覆盖"
        );
    }

    #[test]
    fn auto_connect_triggers_only_with_saved_address() {
        let empty: Arc<dyn BridgePlatform> = Arc::new(MemoryPlatform::new());
        assert_eq!(auto_connect_target(&empty), None, "无保存地址不自动连接");

        let saved: Arc<dyn BridgePlatform> = Arc::new(MemoryPlatform::new());
        saved.save_address("127.0.0.1", 18777);
        assert_eq!(
            auto_connect_target(&saved),
            Some(("127.0.0.1".to_string(), 18777)),
            "有保存地址应自动连接"
        );

        // 坏数据（空 host / 0 端口）不触发
        let bad: Arc<dyn BridgePlatform> = Arc::new(MemoryPlatform::new());
        bad.save_address("", 0);
        assert_eq!(auto_connect_target(&bad), None);
    }

    /// 回归：pair --reset 后旧 token 被拒 → 必须清除已存 token，
    /// 下次 hello 走配对码（否则 token 优先导致配对码永远轮不到，死锁）。
    #[test]
    fn hello_ack_reject_clears_stored_token_and_falls_back_to_code() {
        let platform = Arc::new(MemoryPlatform::with_token(
            "192.168.1.10",
            18777,
            "ef".repeat(32).as_str(),
        ));
        let mut s = session(platform.clone(), Arc::new(MockOps::new()), Some("248266"));
        // 首连 hello 带失效 token
        assert_eq!(s.hello().token.as_deref(), Some("ef".repeat(32).as_str()));
        let state = s.on_hello_ack(&HelloAck {
            ok: false,
            token: None,
            error: Some("token 无效".to_string()),
        });
        assert!(matches!(state, ConnState::Pairing { .. }));
        assert_eq!(
            platform.load_token("192.168.1.10", 18777),
            None,
            "认证被拒必须清除已存 token"
        );
        // 用户输新配对码重连：hello 带配对码而非失效 token
        let hello = s.hello();
        assert_eq!(hello.token, None);
        assert_eq!(hello.pairing_code.as_deref(), Some("248266"));
    }

    #[test]
    fn hello_ack_reject_enters_pairing_state_with_reason() {
        let mut s = session(
            Arc::new(MemoryPlatform::new()),
            Arc::new(MockOps::new()),
            Some("000000"),
        );
        let state = s.on_hello_ack(&HelloAck {
            ok: false,
            token: None,
            error: Some("配对码错误或已失效".to_string()),
        });
        assert_eq!(
            state,
            ConnState::Pairing {
                reason: "配对码错误或已失效".to_string()
            }
        );
    }

    #[test]
    fn command_tap_dispatches_to_ops_and_returns_ok_result() {
        let ops = Arc::new(MockOps::new());
        let s = session(Arc::new(MemoryPlatform::new()), ops.clone(), None);
        let r = s.handle_command(&CommandMessage {
            id: "cmd-1".to_string(),
            method: "tap".to_string(),
            params: json!({ "x": 100, "y": 200 }),
        });
        assert!(r.ok);
        assert_eq!(r.id, "cmd-1");
        assert_eq!(ops.calls.lock().as_slice(), ["tap:100:200"]);
    }

    #[test]
    fn command_all_methods_dispatch() {
        let ops = Arc::new(MockOps::new());
        let s = session(Arc::new(MemoryPlatform::new()), ops.clone(), None);
        for (method, params) in [
            (
                "swipe",
                json!({ "x1": 1, "y1": 2, "x2": 3, "y2": 4, "duration_ms": 300 }),
            ),
            ("input", json!({ "text": "你好" })),
            ("key", json!({ "key": "KEYCODE_HOME" })),
            ("uiTree", json!({})),
            ("screenshot", json!({})),
            ("apps", json!({ "filter": "agent", "all": true })),
            ("launch", json!({ "package": "a.b" })),
        ] {
            let r = s.handle_command(&CommandMessage {
                id: "x".to_string(),
                method: method.to_string(),
                params,
            });
            assert!(r.ok, "{method} 应成功: {:?}", r.error);
        }
        assert_eq!(
            ops.calls.lock().as_slice(),
            [
                "swipe:1:2:3:4:300",
                "input:你好",
                "key:KEYCODE_HOME",
                "uiTree",
                "screenshot",
                r#"apps:Some("agent"):true"#,
                "launch:a.b",
            ]
        );
        // uiTree/screenshot/apps 的结构化结果原样回传
        let r = s.handle_command(&CommandMessage {
            id: "x".to_string(),
            method: "screenshot".to_string(),
            params: json!({}),
        });
        assert_eq!(r.result.unwrap(), json!({ "png_base64": "QUJD" }));
    }

    #[test]
    fn command_unknown_method_returns_error_result() {
        let s = session(
            Arc::new(MemoryPlatform::new()),
            Arc::new(MockOps::new()),
            None,
        );
        let r = s.handle_command(&CommandMessage {
            id: "cmd-9".to_string(),
            method: "shell".to_string(),
            params: json!({}),
        });
        assert!(!r.ok);
        assert_eq!(r.id, "cmd-9");
        assert!(r.error.unwrap().contains("不支持的命令 method"));
    }

    #[test]
    fn command_plugin_error_becomes_error_result() {
        let s = session(
            Arc::new(MemoryPlatform::new()),
            Arc::new(MockOps::failing("无障碍服务未开启")),
            None,
        );
        let r = s.handle_command(&CommandMessage {
            id: "cmd-2".to_string(),
            method: "tap".to_string(),
            params: json!({ "x": 1, "y": 2 }),
        });
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("无障碍服务未开启"));
    }

    #[test]
    fn script_success_returns_value_result() {
        let ops = Arc::new(MockOps::new());
        let s = session(Arc::new(MemoryPlatform::new()), ops.clone(), None);
        let r = s.handle_script(&ScriptMessage {
            id: "cmd-3".to_string(),
            source: "mobile.tap(1, 2); 40 + 2".to_string(),
        });
        assert!(r.ok, "脚本应成功: {:?}", r.error);
        assert_eq!(r.result.unwrap(), json!(42));
        assert_eq!(ops.calls.lock().as_slice(), ["tap:1:2"]);
    }

    #[test]
    fn script_exception_becomes_error_result() {
        let s = session(
            Arc::new(MemoryPlatform::new()),
            Arc::new(MockOps::failing("服务未就绪")),
            None,
        );
        let r = s.handle_script(&ScriptMessage {
            id: "cmd-4".to_string(),
            source: "mobile.tap(1, 2)".to_string(),
        });
        assert!(!r.ok);
        assert!(r.error.unwrap().contains("服务未就绪"));
    }

    #[test]
    fn backoff_doubles_and_caps_at_30s() {
        let seq: Vec<u64> = (0..8).map(|i| backoff_delay(i).as_secs()).collect();
        assert_eq!(seq, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn parse_pair_uri_valid() {
        let info =
            parse_pair_uri("agent-mobile://pair?host=192.168.1.10&port=18777&code=483920").unwrap();
        assert_eq!(
            info,
            PairInfo {
                host: "192.168.1.10".to_string(),
                port: 18777,
                code: "483920".to_string(),
            }
        );
    }

    #[test]
    fn parse_pair_uri_rejects_bad_input() {
        assert!(parse_pair_uri("https://pair?host=a&port=1&code=2").is_err());
        assert!(parse_pair_uri("agent-mobile://pair?host=192.168.1.10&port=18777").is_err());
        assert!(
            parse_pair_uri("agent-mobile://pair?host=192.168.1.10&port=abc&code=483920").is_err()
        );
        assert!(parse_pair_uri("agent-mobile://pair?port=18777&code=483920").is_err());
        assert!(parse_pair_uri("not a uri").is_err());
    }
}
