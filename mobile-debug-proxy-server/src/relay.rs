//! 命令中继注册表：per-device mpsc 串行 + 全局 AtomicU64 请求 id（design.md 决策 9）。
//!
//! WS 连接任务即设备的串行工作协程：HTTP 侧 `dispatch` 入队 [`Job`]，
//! 连接任务逐条下发并挂起等待 result，超时/断线经 oneshot 唤醒 HTTP 等待方。

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Notify};

use crate::bridge_proto::{CommandMessage, DaemonMessage, ResultMessage, ScriptMessage};

/// 单设备命令队列容量（串行语义下多余请求排队）。
const JOB_QUEUE: usize = 32;

/// 注册表键：owner 隔离 + 设备名。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DeviceKey {
    owner: String,
    name: String,
}

/// 一条待下发命令。
pub struct Job {
    /// 服务侧分配的请求 id（`cmd-<n>`）。
    pub id: String,
    /// 待下发帧（command 或 script）。
    pub frame: DaemonMessage,
    /// 结果回传：设备 result、超时或连接中断（Sender 被 drop 则 RecvError）。
    pub reply: oneshot::Sender<JobOutcome>,
}

pub type JobOutcome = Result<ResultMessage, JobFail>;

/// 工作协程侧的失败（仅超时；断线经 drop reply 表达）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobFail {
    Timeout,
}

/// 已连接设备句柄。
pub struct DeviceHandle {
    /// 连接代际：同设备重连替换旧连接后，旧连接退出不得误删新句柄。
    pub conn_id: u64,
    pub job_tx: mpsc::Sender<Job>,
    /// 要求连接任务断开（pairing-reset 吊销）。
    pub close: Arc<Notify>,
}

/// dispatch 失败原因（HTTP 层映射为结构化错误）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchError {
    /// 设备不在线（未连接或刚断开）。
    Offline,
    /// 等待设备 result 超时。
    Timeout,
}

/// 待中继命令体。
pub enum CommandBody {
    Command {
        method: String,
        params: serde_json::Value,
    },
    Script {
        source: String,
    },
}

/// 在线设备注册表 + 请求 id 分配。
#[derive(Default)]
pub struct Relay {
    inner: Mutex<HashMap<DeviceKey, DeviceHandle>>,
    conn_seq: AtomicU64,
    cmd_seq: AtomicU64,
}

impl Relay {
    /// 注册连接：同设备已有旧连接时要求其断开并替换。
    pub fn register(&self, owner: &str, name: &str) -> (u64, mpsc::Receiver<Job>, Arc<Notify>) {
        let (tx, rx) = mpsc::channel(JOB_QUEUE);
        let close = Arc::new(Notify::new());
        let conn_id = self.conn_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let key = DeviceKey {
            owner: owner.to_string(),
            name: name.to_string(),
        };
        let mut map = self.inner.lock();
        if let Some(old) = map.insert(
            key,
            DeviceHandle {
                conn_id,
                job_tx: tx,
                close: close.clone(),
            },
        ) {
            old.close.notify_one();
        }
        (conn_id, rx, close)
    }

    /// 连接退出时注销：仅当句柄仍是本代连接才移除。
    pub fn unregister(&self, owner: &str, name: &str, conn_id: u64) {
        let mut map = self.inner.lock();
        let key = DeviceKey {
            owner: owner.to_string(),
            name: name.to_string(),
        };
        if map.get(&key).map(|h| h.conn_id) == Some(conn_id) {
            map.remove(&key);
        }
    }

    /// 设备当前是否在线。
    pub fn is_online(&self, owner: &str, name: &str) -> bool {
        let key = DeviceKey {
            owner: owner.to_string(),
            name: name.to_string(),
        };
        self.inner.lock().contains_key(&key)
    }

    /// 断开某 owner 全部已连接设备并移除句柄（pairing-reset）。
    pub fn disconnect_owner(&self, owner: &str) {
        let mut map = self.inner.lock();
        let keys: Vec<DeviceKey> = map.keys().filter(|k| k.owner == owner).cloned().collect();
        for key in keys {
            if let Some(handle) = map.remove(&key) {
                handle.close.notify_one();
            }
        }
    }

    /// 分配全局递增请求 id（`cmd-<n>`，与 daemon 语义一致）。
    pub fn next_command_id(&self) -> String {
        format!("cmd-{}", self.cmd_seq.fetch_add(1, Ordering::Relaxed) + 1)
    }

    /// 中继一条命令到设备并同步等待 result（design.md 决策 3/9）。
    ///
    /// 设备未连接立即返回 `Offline`；等待超过 command_timeout（由 ws 连接任务
    /// 强制执行）返回 `Timeout`；连接中断（reply 通道被 drop）按 `Offline` 处理。
    pub async fn dispatch(
        &self,
        owner: &str,
        name: &str,
        body: CommandBody,
    ) -> Result<ResultMessage, DispatchError> {
        let key = DeviceKey {
            owner: owner.to_string(),
            name: name.to_string(),
        };
        let job_tx = self
            .inner
            .lock()
            .get(&key)
            .map(|h| h.job_tx.clone())
            .ok_or(DispatchError::Offline)?;
        let id = self.next_command_id();
        let frame = match body {
            CommandBody::Command { method, params } => DaemonMessage::Command(CommandMessage {
                id: id.clone(),
                method,
                params,
            }),
            CommandBody::Script { source } => DaemonMessage::Script(ScriptMessage {
                id: id.clone(),
                source,
            }),
        };
        let (tx, rx) = oneshot::channel();
        job_tx
            .send(Job {
                id,
                frame,
                reply: tx,
            })
            .await
            .map_err(|_| DispatchError::Offline)?;
        match rx.await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(JobFail::Timeout)) => Err(DispatchError::Timeout),
            // 连接任务退出时 drop reply：按离线处理
            Err(_) => Err(DispatchError::Offline),
        }
    }
}
