//! 桥接设备注册表（design.md 决策 1/3）：daemon 内存中的桥接设备状态与命令路由。
//!
//! - `register`/`unregister`/`touch` 由 WS 连接循环驱动；
//! - `device_records` 供 executor devices 合并枚举（在线判定 = 有连接且 last_seen 未超心跳超时）；
//! - `command`/`script` 下发帧并登记 oneshot 等待；`complete` 按 id 配对回传 result（组 5 后端路由使用）。

use crate::backend::{BackendKind, ConnectionKind, DeviceRecord, DeviceState};
use crate::bridge_proto::{
    Capability, CommandMessage, DaemonMessage, Hello, ResultMessage, ScriptMessage,
};
use crate::output::ErrorBody;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

/// 心跳超时：超过该时长未收到任何消息即判离线（design.md：30s）。
pub const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);

#[allow(dead_code)] // tx 由组 5 命令路由（dispatch）读取
struct Connection {
    /// 连接代际：重连后旧连接的断线事件不得误标新连接离线。
    id: u64,
    tx: mpsc::UnboundedSender<String>,
    last_seen: Instant,
}

#[allow(dead_code)] // android_version/capabilities 由组 5 设备枚举与能力校验读取
struct BridgeDevice {
    name: String,
    android_version: String,
    capabilities: Vec<Capability>,
    conn: Option<Connection>,
}

/// daemon 级共享的桥接设备注册表。
pub struct BridgeRegistry {
    devices: Mutex<HashMap<String, BridgeDevice>>,
    /// 在飞请求：command/script id → 结果等待方。
    pending: Mutex<HashMap<String, oneshot::Sender<ResultMessage>>>,
    next_id: AtomicU64,
}

impl Default for BridgeRegistry {
    fn default() -> Self {
        Self {
            devices: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }
}

impl BridgeRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// hello 认证通过后注册设备；返回（设备 id, 连接代际）。同名设备重连即替换旧连接。
    pub fn register(&self, hello: &Hello, tx: mpsc::UnboundedSender<String>) -> (String, u64) {
        let id = format!("bridge:{}", hello.device_name);
        let conn_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let device = BridgeDevice {
            name: hello.device_name.clone(),
            android_version: hello.android_version.clone(),
            capabilities: hello.capabilities.clone(),
            conn: Some(Connection {
                id: conn_id,
                tx,
                last_seen: Instant::now(),
            }),
        };
        self.devices.lock().insert(id.clone(), device);
        (id, conn_id)
    }

    /// 连接断开：仅当代际匹配（仍是当前连接）时标离线；设备记录保留。
    pub fn unregister(&self, device_id: &str, conn_id: u64) {
        let mut devices = self.devices.lock();
        if let Some(dev) = devices.get_mut(device_id) {
            if dev.conn.as_ref().is_some_and(|c| c.id == conn_id) {
                dev.conn = None;
            }
        }
    }

    /// 心跳/任意消息刷新 last_seen（代际匹配时）。
    pub fn touch(&self, device_id: &str, conn_id: u64) {
        let mut devices = self.devices.lock();
        if let Some(dev) = devices.get_mut(device_id) {
            if let Some(conn) = dev.conn.as_mut() {
                if conn.id == conn_id {
                    conn.last_seen = Instant::now();
                }
            }
        }
    }

    /// 设备枚举快照：在线 = 有连接且 last_seen 未超心跳超时。
    pub fn device_records(&self) -> Vec<DeviceRecord> {
        self.devices
            .lock()
            .values()
            .map(|dev| {
                let online = dev
                    .conn
                    .as_ref()
                    .is_some_and(|c| c.last_seen.elapsed() <= HEARTBEAT_TIMEOUT);
                DeviceRecord {
                    id: format!("bridge:{}", dev.name),
                    kind: BackendKind::AppBridge,
                    model: Some(dev.name.clone()),
                    state: if online {
                        DeviceState::Online
                    } else {
                        DeviceState::Offline
                    },
                    connection: ConnectionKind::Bridge,
                }
            })
            .collect()
    }

    /// 设备能力集（组 5 能力校验使用）。
    #[allow(dead_code)] // 组 5 app-bridge 后端启用
    pub fn capabilities(&self, device_id: &str) -> Option<Vec<Capability>> {
        self.devices
            .lock()
            .get(device_id)
            .map(|d| d.capabilities.clone())
    }

    /// 下发 command 帧并返回结果等待句柄。
    #[allow(dead_code)] // 组 5 app-bridge 后端启用
    pub fn command(
        self: &Arc<Self>,
        device_id: &str,
        method: &str,
        params: Value,
    ) -> crate::backend::BResult<oneshot::Receiver<ResultMessage>> {
        let id = self.next_request_id();
        self.dispatch(
            device_id,
            id.clone(),
            DaemonMessage::Command(CommandMessage {
                id,
                method: method.to_string(),
                params,
            }),
        )
    }

    /// 下发 script 帧并返回结果等待句柄。
    #[allow(dead_code)] // 组 5 app-bridge 后端启用
    pub fn script(
        self: &Arc<Self>,
        device_id: &str,
        source: &str,
    ) -> crate::backend::BResult<oneshot::Receiver<ResultMessage>> {
        let id = self.next_request_id();
        self.dispatch(
            device_id,
            id.clone(),
            DaemonMessage::Script(ScriptMessage {
                id,
                source: source.to_string(),
            }),
        )
    }

    /// result 回传配对：按 id 唤醒等待方（未知 id 静默丢弃，如超时后的迟到回传）。
    pub fn complete(&self, result: ResultMessage) {
        if let Some(tx) = self.pending.lock().remove(&result.id) {
            let _ = tx.send(result);
        }
    }

    #[allow(dead_code)] // 经 command/script 启用（组 5）
    fn next_request_id(&self) -> String {
        format!("cmd-{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    #[allow(dead_code)] // 经 command/script 启用（组 5）
    fn dispatch(
        self: &Arc<Self>,
        device_id: &str,
        req_id: String,
        msg: DaemonMessage,
    ) -> crate::backend::BResult<oneshot::Receiver<ResultMessage>> {
        let frame = serde_json::to_string(&msg)
            .map_err(|e| ErrorBody::io_error(format!("桥接消息序列化失败: {e}")))?;
        let tx = {
            let devices = self.devices.lock();
            let Some(dev) = devices.get(device_id) else {
                return Err(ErrorBody::device_not_found(format!(
                    "桥接设备 {device_id} 未注册"
                )));
            };
            let Some(conn) = dev.conn.as_ref() else {
                return Err(ErrorBody::device_offline(format!(
                    "桥接设备 {device_id} 离线"
                )));
            };
            if conn.last_seen.elapsed() > HEARTBEAT_TIMEOUT {
                return Err(ErrorBody::device_offline(format!(
                    "桥接设备 {device_id} 心跳超时离线"
                )));
            }
            conn.tx.clone()
        };
        let (wait_tx, wait_rx) = oneshot::channel();
        if tx.send(frame).is_err() {
            return Err(ErrorBody::device_offline(format!(
                "桥接设备 {device_id} 连接已断开"
            )));
        }
        self.pending.lock().insert(req_id, wait_tx);
        Ok(wait_rx)
    }

    /// 测试辅助：把连接 last_seen 回拨 `age`，模拟心跳超时。
    #[cfg(test)]
    fn age_connection_for_test(&self, device_id: &str, conn_id: u64, age: Duration) {
        let mut devices = self.devices.lock();
        if let Some(dev) = devices.get_mut(device_id) {
            if let Some(conn) = dev.conn.as_mut() {
                if conn.id == conn_id {
                    conn.last_seen = Instant::now() - age;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendKind, ConnectionKind, DeviceState};
    use crate::bridge_proto::{Capability, ClientMessage, DaemonMessage, Hello, ResultMessage};
    use serde_json::json;

    fn hello(name: &str) -> Hello {
        Hello {
            pairing_code: None,
            token: Some("t".into()),
            device_name: name.into(),
            android_version: "12".into(),
            capabilities: vec![Capability::Tap, Capability::UiTree],
        }
    }

    #[test]
    fn register_shows_online_bridge_device() {
        let reg = BridgeRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (id, _conn) = reg.register(&hello("MuMu"), tx);
        assert_eq!(id, "bridge:MuMu");

        let records = reg.device_records();
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.id, "bridge:MuMu");
        assert_eq!(r.kind, BackendKind::AppBridge);
        assert_eq!(r.connection, ConnectionKind::Bridge);
        assert_eq!(r.state, DeviceState::Online);
        assert_eq!(r.model.as_deref(), Some("MuMu"));
    }

    #[test]
    fn disconnect_marks_offline_but_keeps_record() {
        let reg = BridgeRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (id, conn) = reg.register(&hello("MuMu"), tx);
        reg.unregister(&id, conn);
        let r = &reg.device_records()[0];
        assert_eq!(r.state, DeviceState::Offline, "断线后保留记录并标离线");
    }

    #[test]
    fn stale_conn_id_cannot_unregister_new_connection() {
        let reg = BridgeRegistry::new();
        let (tx1, _rx1) = tokio::sync::mpsc::unbounded_channel();
        let (id, old) = reg.register(&hello("MuMu"), tx1);
        let (tx2, _rx2) = tokio::sync::mpsc::unbounded_channel();
        let (_id2, new_conn) = reg.register(&hello("MuMu"), tx2);
        // 旧连接断开事件晚到：不得误标新连接离线
        reg.unregister(&id, old);
        let r = &reg.device_records()[0];
        assert_eq!(r.state, DeviceState::Online);
        reg.unregister(&id, new_conn);
        assert_eq!(reg.device_records()[0].state, DeviceState::Offline);
    }

    #[test]
    fn heartbeat_timeout_marks_offline() {
        let reg = BridgeRegistry::new();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (id, conn) = reg.register(&hello("MuMu"), tx);
        reg.age_connection_for_test(&id, conn, HEARTBEAT_TIMEOUT + Duration::from_secs(1));
        assert_eq!(reg.device_records()[0].state, DeviceState::Offline);
        // 心跳刷新恢复在线
        reg.touch(&id, conn);
        assert_eq!(reg.device_records()[0].state, DeviceState::Online);
    }

    #[tokio::test]
    async fn command_frames_routed_and_result_correlated() {
        let reg = BridgeRegistry::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (id, _conn) = reg.register(&hello("MuMu"), tx);

        let wait = reg
            .command(&id, "tap", json!({"x": 100, "y": 200}))
            .expect("在线设备应可下发");

        // 设备侧收到 command 帧
        let frame = rx.recv().await.expect("应收到下发帧");
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["type"], "command");
        assert_eq!(v["method"], "tap");
        assert_eq!(v["params"], json!({"x": 100, "y": 200}));
        let cmd_id = v["id"].as_str().expect("应有 id").to_string();

        // 设备回传 result → 等待方收到
        reg.complete(ResultMessage {
            id: cmd_id,
            ok: true,
            result: Some(json!({"tapped": [100, 200]})),
            error: None,
        });
        let r = wait.await.expect("result 应回传配对");
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["tapped"], json!([100, 200]));
    }

    #[tokio::test]
    async fn script_frame_routed() {
        let reg = BridgeRegistry::new();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let (id, _conn) = reg.register(&hello("MuMu"), tx);
        let wait = reg
            .script(&id, "mobile.tap(1,2)")
            .expect("在线设备应可下发");
        let frame = rx.recv().await.unwrap();
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["type"], "script");
        assert_eq!(v["source"], "mobile.tap(1,2)");
        reg.complete(ResultMessage {
            id: v["id"].as_str().unwrap().to_string(),
            ok: true,
            result: Some(json!(42)),
            error: None,
        });
        assert_eq!(wait.await.unwrap().result, Some(json!(42)));
    }

    #[test]
    fn command_to_unknown_or_offline_device_errors() {
        let reg = BridgeRegistry::new();
        let err = reg.command("bridge:ghost", "tap", json!({})).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::DeviceNotFound);

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (id, conn) = reg.register(&hello("MuMu"), tx);
        reg.unregister(&id, conn);
        let err = reg.command(&id, "tap", json!({})).unwrap_err();
        assert_eq!(err.code, crate::output::ErrorCode::DeviceOffline);
    }

    #[test]
    fn result_ack_and_pong_frames_serialize() {
        // 连接层回包契约：pong / result_ack 可序列化为文本帧
        let pong = serde_json::to_string(&DaemonMessage::Pong).unwrap();
        assert_eq!(pong, r#"{"type":"pong"}"#);
        let ack =
            serde_json::to_string(&DaemonMessage::ResultAck(crate::bridge_proto::ResultAck {
                id: "cmd-1".into(),
            }))
            .unwrap();
        assert!(ack.contains("result_ack"));
        // ClientMessage 反序列化路径（ws 连接循环使用）
        let hb: ClientMessage = serde_json::from_str(r#"{"type":"heartbeat"}"#).unwrap();
        assert!(matches!(hb, ClientMessage::Heartbeat));
    }
}
