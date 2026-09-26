//! daemon 启动锁：锁文件 + pid 存活检测 + 端口探测双重判定，过期锁可回收（design.md 决策 8）。
//!
//! 锁文件 `<配置目录>/daemon.lock` 为 JSON：`{pid, port, started_at_epoch_secs}`。
//! 获取语义：
//! - 锁文件不存在 → `create_new` 原子创建，竞争失败者进入存量判定；
//! - 锁内 pid 存活且端口在听 → `AlreadyRunning`；
//! - pid 已死或端口不通（含锁文件损坏）→ 视为过期锁，删除后重新竞争。

use crate::backend::BResult;
use crate::output::ErrorBody;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const LOCK_FILE_NAME: &str = "daemon.lock";

/// 竞争重试上限：过期锁删除与创建之间允许其他实例抢先，有限次重试后报错。
const MAX_ATTEMPTS: u32 = 8;

/// 锁文件内容。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LockInfo {
    pub pid: u32,
    pub port: u16,
    pub started_at_epoch_secs: u64,
}

/// 获取结果：要么拿到新锁，要么发现已有 daemon 在运行。
#[derive(Debug)]
pub enum LockOutcome {
    Acquired(DaemonLock),
    AlreadyRunning { pid: u32, port: u16 },
}

/// 已持有的启动锁；Drop 时删除锁文件。
#[derive(Debug)]
pub struct DaemonLock {
    pub info: LockInfo,
    path: PathBuf,
}

impl DaemonLock {
    /// 在 `dir` 下以 `port` 获取启动锁。
    pub fn acquire(dir: &Path, port: u16) -> BResult<LockOutcome> {
        fs::create_dir_all(dir)
            .map_err(|e| ErrorBody::io_error(format!("创建配置目录失败 {}: {e}", dir.display())))?;
        let path = dir.join(LOCK_FILE_NAME);
        let info = LockInfo {
            pid: std::process::id(),
            port,
            started_at_epoch_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let body = serde_json::to_vec(&info)
            .map_err(|e| ErrorBody::io_error(format!("序列化锁信息失败: {e}")))?;
        for _ in 0..MAX_ATTEMPTS {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(&body).map_err(|e| {
                        ErrorBody::io_error(format!("写入锁文件失败 {}: {e}", path.display()))
                    })?;
                    return Ok(LockOutcome::Acquired(DaemonLock { info, path }));
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists => match read_lock_with_grace(dir) {
                    Some(old) if pid_alive(old.pid) && port_listening(old.port) => {
                        return Ok(LockOutcome::AlreadyRunning {
                            pid: old.pid,
                            port: old.port,
                        });
                    }
                    // 过期或损坏的锁：删除后重新竞争
                    _ => {
                        let _ = fs::remove_file(&path);
                    }
                },
                Err(e) => {
                    return Err(ErrorBody::io_error(format!(
                        "创建锁文件失败 {}: {e}",
                        path.display()
                    )))
                }
            }
        }
        Err(ErrorBody::io_error(format!(
            "获取 daemon 启动锁失败：竞争重试超过 {MAX_ATTEMPTS} 次"
        )))
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// 读取并解析锁文件；文件缺失或内容损坏时返回 None。
pub fn read_lock(dir: &Path) -> Option<LockInfo> {
    let content = fs::read_to_string(dir.join(LOCK_FILE_NAME)).ok()?;
    serde_json::from_str(&content).ok()
}

/// 读取锁文件并带写入竞态宽限：`create_new` 成功到内容写完之间存在空文件窗口，
/// 此时短暂重试而不是立即按「损坏」回收，避免并发竞争方误删有效锁。
fn read_lock_with_grace(dir: &Path) -> Option<LockInfo> {
    let path = dir.join(LOCK_FILE_NAME);
    for _ in 0..20 {
        match fs::read_to_string(&path) {
            Ok(content) => {
                if let Ok(info) = serde_json::from_str(&content) {
                    return Some(info);
                }
            }
            // 文件已消失（持有方退出），无需宽限
            Err(_) => return None,
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// pid 是否存活（EPERM 视为存活：进程存在但无权发信号）。
#[cfg(unix)]
pub fn pid_alive(pid: u32) -> bool {
    if unsafe { libc::kill(pid as i32, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// 非 Unix 平台占位：首版 daemon 仅面向 macOS/Linux，保守视为存活以避免误删锁。
#[cfg(not(unix))]
pub fn pid_alive(_pid: u32) -> bool {
    true
}

/// 127.0.0.1:port 是否有服务在监听。
pub fn port_listening(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// 健康检查失败后清理过期锁；运行中的锁不动。返回是否实际删除。
pub fn remove_stale_lock(dir: &Path) -> BResult<bool> {
    let path = dir.join(LOCK_FILE_NAME);
    if let Some(info) = read_lock(dir) {
        if pid_alive(info.pid) && port_listening(info.port) {
            return Ok(false);
        }
    }
    match fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ErrorBody::io_error(format!(
            "清理过期锁文件失败 {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::{Arc, Barrier};

    fn temp_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("创建临时目录")
    }

    /// 绑定 :0 取一个空闲端口，listener 由调用方持有以保持「端口在听」。
    fn listening_port() -> (TcpListener, u16) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定随机端口");
        let port = listener.local_addr().expect("读取本地地址").port();
        (listener, port)
    }

    /// 取一个已退出进程的 pid（spawn `true` 并 wait，保证 kill(pid,0) 返回 ESRCH）。
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn true");
        let pid = child.id();
        child.wait().expect("wait true");
        pid
    }

    #[test]
    fn acquire_creates_lock_and_drop_releases() {
        let dir = temp_dir();
        let (_l, port) = listening_port();
        let outcome = DaemonLock::acquire(dir.path(), port).expect("首次获取锁");
        match outcome {
            LockOutcome::Acquired(lock) => {
                let info = read_lock(dir.path()).expect("读取锁文件");
                assert_eq!(info.pid, std::process::id());
                assert_eq!(info.port, port);
                assert!(info.started_at_epoch_secs > 0);
                assert!(dir.path().join(LOCK_FILE_NAME).exists());
                drop(lock);
                assert!(
                    !dir.path().join(LOCK_FILE_NAME).exists(),
                    "Drop 后锁文件应被删除"
                );
            }
            LockOutcome::AlreadyRunning { .. } => panic!("首次获取应为 Acquired"),
        }
    }

    #[test]
    fn concurrent_scopes_single_acquired_then_reacquire() {
        let dir = temp_dir();
        let (_l, port) = listening_port();
        let first = DaemonLock::acquire(dir.path(), port).expect("第一次获取");
        let second = DaemonLock::acquire(dir.path(), port).expect("第二次获取");
        match (&first, &second) {
            (LockOutcome::Acquired(_), LockOutcome::AlreadyRunning { pid, port: p }) => {
                assert_eq!(*pid, std::process::id());
                assert_eq!(*p, port);
            }
            _ => panic!("应为一个 Acquired 一个 AlreadyRunning"),
        }
        drop(first);
        let third = DaemonLock::acquire(dir.path(), port).expect("释放后再次获取");
        assert!(
            matches!(third, LockOutcome::Acquired(_)),
            "释放后另一个应能获取"
        );
    }

    #[test]
    fn threaded_race_single_acquired() {
        let dir = temp_dir();
        let (_l, port) = listening_port();
        let path = Arc::new(dir.path().to_path_buf());
        let threads = 8;
        // Barrier 保证所有线程完成判定前没有锁被提前 Drop，避免时间窗抖动。
        let barrier = Arc::new(Barrier::new(threads));
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let path = Arc::clone(&path);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let outcome = DaemonLock::acquire(&path, port).expect("竞争获取锁");
                    let acquired = matches!(outcome, LockOutcome::Acquired(_));
                    barrier.wait();
                    acquired
                })
            })
            .collect();
        let acquired = handles
            .into_iter()
            .map(|h| h.join().expect("join 竞争线程"))
            .filter(|ok| *ok)
            .count();
        assert_eq!(acquired, 1, "并发竞争只允许一个 Acquired");
    }

    #[test]
    fn stale_lock_recycled() {
        let dir = temp_dir();
        // 死 pid + 未监听端口 → 过期锁
        let (_l, port) = listening_port();
        drop(_l);
        let stale = LockInfo {
            pid: dead_pid(),
            port,
            started_at_epoch_secs: 1,
        };
        fs::write(
            dir.path().join(LOCK_FILE_NAME),
            serde_json::to_string(&stale).expect("序列化过期锁"),
        )
        .expect("写入过期锁");
        let outcome = DaemonLock::acquire(dir.path(), port).expect("回收过期锁");
        match outcome {
            LockOutcome::Acquired(lock) => {
                let info = read_lock(dir.path()).expect("读取新锁");
                assert_eq!(info.pid, std::process::id(), "锁应被当前进程重写");
                drop(lock);
            }
            LockOutcome::AlreadyRunning { .. } => panic!("过期锁应被回收而非 AlreadyRunning"),
        }
    }

    #[test]
    fn corrupted_lock_recycled() {
        let dir = temp_dir();
        fs::write(dir.path().join(LOCK_FILE_NAME), "not-json").expect("写入损坏锁");
        let outcome = DaemonLock::acquire(dir.path(), 9).expect("回收损坏锁");
        assert!(matches!(outcome, LockOutcome::Acquired(_)));
    }

    #[test]
    fn remove_stale_lock_only_when_not_running() {
        let dir = temp_dir();
        let (_l, port) = listening_port();
        // 运行中（当前进程 pid + 在听端口）→ 不删除
        let running = LockInfo {
            pid: std::process::id(),
            port,
            started_at_epoch_secs: 1,
        };
        fs::write(
            dir.path().join(LOCK_FILE_NAME),
            serde_json::to_string(&running).expect("序列化锁"),
        )
        .expect("写入锁");
        assert!(!remove_stale_lock(dir.path()).expect("清理判定"));
        assert!(dir.path().join(LOCK_FILE_NAME).exists());
        // 死 pid → 删除
        let stale = LockInfo {
            pid: dead_pid(),
            port,
            started_at_epoch_secs: 1,
        };
        fs::write(
            dir.path().join(LOCK_FILE_NAME),
            serde_json::to_string(&stale).expect("序列化锁"),
        )
        .expect("写入锁");
        assert!(remove_stale_lock(dir.path()).expect("清理过期锁"));
        assert!(!dir.path().join(LOCK_FILE_NAME).exists());
    }
}
