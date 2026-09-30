//! JSON 数据文件持久化：owners / 设备记录 / token / 配对码状态（design.md 决策 4/10）。
//!
//! - 单 JSON 文件（默认 `./proxy-data.json`），parking_lot 保护并发，明文 token
//!   （与 daemon tokens.json 现状一致）；
//! - 首启无任何 owner 时自动生成一个 64 hex owner token 并落盘（由 bin 打印 stdout）；
//! - `--owner-token` 注入的 owner 追加合并（幂等去重）；v1 无 owner 管理 API。

use parking_lot::Mutex;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::bridge_proto::Capability;

/// 一台已配对设备的持久记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceRecord {
    /// 设备名称（owner 内唯一，hello.device_name）。
    pub name: String,
    /// 设备长期 token（64 hex）。
    pub token: String,
    #[serde(default)]
    pub android_version: String,
    #[serde(default)]
    pub capabilities: Vec<Capability>,
    /// 最近活跃时间（unix 秒；hello/heartbeat 刷新）。
    #[serde(default)]
    pub last_seen: u64,
}

/// 一个 owner 的持久记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnerRecord {
    /// owner token（64 hex，HTTP Bearer 凭证）。
    pub token: String,
    /// 当前有效的一次性配对码（未签发/已消费后为 None）。
    #[serde(default)]
    pub pairing_code: Option<String>,
    #[serde(default)]
    pub devices: Vec<DeviceRecord>,
}

impl OwnerRecord {
    fn new(token: String) -> Self {
        Self {
            token,
            pairing_code: None,
            devices: Vec::new(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DataFile {
    #[serde(default)]
    owners: Vec<OwnerRecord>,
}

/// 数据文件句柄：内存为准（Mutex 保护），每次变更原子落盘。
pub struct Store {
    inner: Mutex<DataFile>,
    path: PathBuf,
}

impl Store {
    /// 打开（不存在则创建）数据文件并合并 `extra_tokens` 注入的 owner。
    ///
    /// 返回 `(Store, Option<新生成的 owner token>)`：仅当文件中无任何 owner
    /// 且未注入时生成（首启场景，bin 负责打印）。
    pub fn open(path: &Path, extra_tokens: &[String]) -> io::Result<(Self, Option<String>)> {
        let mut file = match fs::read_to_string(path) {
            Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text).map_err(|e| {
                io::Error::new(io::ErrorKind::InvalidData, format!("数据文件损坏: {e}"))
            })?,
            Ok(_) => DataFile::default(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => DataFile::default(),
            Err(e) => return Err(e),
        };
        for token in extra_tokens {
            if !file.owners.iter().any(|o| &o.token == token) {
                file.owners.push(OwnerRecord::new(token.clone()));
            }
        }
        let mut generated = None;
        if file.owners.is_empty() {
            let token = generate_token();
            file.owners.push(OwnerRecord::new(token.clone()));
            generated = Some(token);
        }
        let store = Self {
            inner: Mutex::new(file),
            path: path.to_path_buf(),
        };
        store.save()?;
        Ok((store, generated))
    }

    /// owner 数量（测试用）。
    pub fn owner_count(&self) -> usize {
        self.inner.lock().owners.len()
    }

    /// 判断是否为有效 owner token。
    pub fn is_owner_token(&self, token: &str) -> bool {
        self.inner.lock().owners.iter().any(|o| o.token == token)
    }

    /// 为 owner 签发 6 位一次性配对码并落盘；owner 不存在返回 None。
    /// 持锁重摇直至全部 owner 的待消费配对码中无重复（FixReview SUGGESTION：
    /// 跨 owner 碰撞会把设备静默配入靠前 owner）。
    pub fn issue_pairing_code(&self, owner: &str) -> Option<String> {
        let mut file = self.inner.lock();
        if !file.owners.iter().any(|o| o.token == owner) {
            return None;
        }
        let code = loop {
            let candidate = generate_code();
            if !file
                .owners
                .iter()
                .any(|o| o.pairing_code.as_deref() == Some(candidate.as_str()))
            {
                break candidate;
            }
        };
        let record = file.owners.iter_mut().find(|o| o.token == owner)?;
        record.pairing_code = Some(code.clone());
        drop(file);
        self.save().ok()?;
        Some(code)
    }

    /// 一次性消费配对码：命中即失效，返回所属 owner token。
    pub fn consume_pairing_code(&self, code: &str) -> Option<String> {
        let mut file = self.inner.lock();
        let record = file
            .owners
            .iter_mut()
            .find(|o| o.pairing_code.as_deref() == Some(code))?;
        record.pairing_code = None;
        let owner = record.token.clone();
        drop(file);
        self.save().ok()?;
        Some(owner)
    }

    /// 配对码验证通过后登记设备并签发设备 token；同名设备重新配对则轮换 token。
    pub fn pair_device(
        &self,
        owner: &str,
        name: &str,
        android_version: &str,
        capabilities: &[Capability],
    ) -> Option<String> {
        let mut file = self.inner.lock();
        let record = file.owners.iter_mut().find(|o| o.token == owner)?;
        let token = generate_token();
        let now = now_secs();
        if let Some(dev) = record.devices.iter_mut().find(|d| d.name == name) {
            dev.token = token.clone();
            dev.android_version = android_version.to_string();
            dev.capabilities = capabilities.to_vec();
            dev.last_seen = now;
        } else {
            record.devices.push(DeviceRecord {
                name: name.to_string(),
                token: token.clone(),
                android_version: android_version.to_string(),
                capabilities: capabilities.to_vec(),
                last_seen: now,
            });
        }
        drop(file);
        self.save().ok()?;
        Some(token)
    }

    /// 凭设备 token 反查 `(owner token, 设备记录)`。
    pub fn verify_device_token(&self, token: &str) -> Option<(String, DeviceRecord)> {
        let file = self.inner.lock();
        file.owners.iter().find_map(|o| {
            o.devices
                .iter()
                .find(|d| d.token == token)
                .map(|d| (o.token.clone(), d.clone()))
        })
    }

    /// 刷新设备 last_seen（hello/heartbeat 时调用）。
    pub fn touch_device(&self, owner: &str, name: &str) {
        let mut file = self.inner.lock();
        if let Some(record) = file.owners.iter_mut().find(|o| o.token == owner) {
            if let Some(dev) = record.devices.iter_mut().find(|d| d.name == name) {
                dev.last_seen = now_secs();
            }
        }
        drop(file);
        let _ = self.save();
    }

    /// 更新设备元数据（重连 hello 时能力集/系统版本可能变化）。
    pub fn update_device_meta(
        &self,
        owner: &str,
        name: &str,
        android_version: &str,
        capabilities: &[Capability],
    ) {
        let mut file = self.inner.lock();
        if let Some(record) = file.owners.iter_mut().find(|o| o.token == owner) {
            if let Some(dev) = record.devices.iter_mut().find(|d| d.name == name) {
                dev.android_version = android_version.to_string();
                dev.capabilities = capabilities.to_vec();
                dev.last_seen = now_secs();
            }
        }
        drop(file);
        let _ = self.save();
    }

    /// 枚举 owner 名下全部设备记录（含离线）。
    pub fn devices_of(&self, owner: &str) -> Vec<DeviceRecord> {
        let file = self.inner.lock();
        file.owners
            .iter()
            .find(|o| o.token == owner)
            .map(|o| o.devices.clone())
            .unwrap_or_default()
    }

    /// 重置配对：吊销该 owner 全部设备 token、清空配对码，返回被吊销的设备名列表
    ///（调用方据此断开已连接设备）。
    pub fn reset_owner(&self, owner: &str) -> Option<Vec<String>> {
        let mut file = self.inner.lock();
        let record = file.owners.iter_mut().find(|o| o.token == owner)?;
        record.pairing_code = None;
        let revoked: Vec<String> = record.devices.iter().map(|d| d.name.clone()).collect();
        record.devices.clear();
        drop(file);
        self.save().ok()?;
        Some(revoked)
    }

    /// 原子落盘：tmp + rename；unix 上 0600（token 等价长期凭证）。
    fn save(&self) -> io::Result<()> {
        let file = self.inner.lock();
        let text = serde_json::to_string_pretty(&*file)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        drop(file);
        let tmp = self.path.with_extension("json.tmp");
        {
            #[cfg(unix)]
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let mut f = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&tmp)?;
                f.write_all(text.as_bytes())?;
            }
            #[cfg(not(unix))]
            {
                fs::write(&tmp, text.as_bytes())?;
            }
        }
        fs::rename(&tmp, &self.path)
    }
}

/// 6 位数字配对码（允许前导零）。
fn generate_code() -> String {
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

/// 32 字节随机 token 的 hex 编码（64 字符）。
fn generate_token() -> String {
    let bytes: [u8; 32] = rand::thread_rng().gen();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 当前 unix 秒。
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
#[cfg(test)]
mod tests {
    use crate::bridge_proto::Capability;
    use crate::store::Store;

    fn temp_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join("proxy-data.json")
    }

    #[test]
    fn 首启生成_owner_token_并落盘() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_path(&dir);
        let (store, generated) = Store::open(&path, &[]).unwrap();
        let token = generated.expect("首启必须生成 owner token");
        assert_eq!(token.len(), 64, "owner token 为 64 hex");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(store.is_owner_token(&token));
        assert!(path.exists(), "首启后数据文件已写盘");

        // 重启加载：同一 token 保留，不重复生成
        let (store2, generated2) = Store::open(&path, &[]).unwrap();
        assert!(generated2.is_none(), "二次启动不再生成");
        assert!(store2.is_owner_token(&token));
    }

    #[test]
    fn 注入_owner_token_追加且幂等() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_path(&dir);
        let injected = "ab".repeat(32);
        let (store, generated) = Store::open(&path, std::slice::from_ref(&injected)).unwrap();
        assert!(generated.is_none(), "注入后不再自动生成");
        assert!(store.is_owner_token(&injected));

        // 重复注入不重复记录
        let (store2, _) = Store::open(&path, std::slice::from_ref(&injected)).unwrap();
        assert!(store2.is_owner_token(&injected));
        assert_eq!(store2.owner_count(), 1);
    }

    #[test]
    fn 配对码签发与一次性消费() {
        let dir = tempfile::tempdir().unwrap();
        let (store, gen) = Store::open(&temp_path(&dir), &[]).unwrap();
        let owner = gen.unwrap();
        let code = store.issue_pairing_code(&owner).unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));

        // 非 owner 无法签发
        assert!(store.issue_pairing_code("not-an-owner").is_none());

        // 一次性：首次消费成功，再次消费失败
        assert_eq!(store.consume_pairing_code(&code), Some(owner.clone()));
        assert_eq!(store.consume_pairing_code(&code), None);
        assert_eq!(store.consume_pairing_code("000000"), None);
    }

    #[test]
    fn 设备配对_加载保存_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_path(&dir);
        let (store, gen) = Store::open(&path, &[]).unwrap();
        let owner = gen.unwrap();
        let caps = vec![Capability::Tap, Capability::Screenshot];
        let dev_token = store
            .pair_device(&owner, "pixel-7", "14", &caps)
            .expect("owner 存在");
        assert_eq!(dev_token.len(), 64, "设备 token 为 64 hex");

        // 重新加载后设备记录与 token 仍在
        let (store2, _) = Store::open(&path, &[]).unwrap();
        let (owner2, rec) = store2.verify_device_token(&dev_token).unwrap();
        assert_eq!(owner2, owner);
        assert_eq!(rec.name, "pixel-7");
        assert_eq!(rec.android_version, "14");
        assert_eq!(rec.capabilities, caps);

        // 同名重新配对：token 轮换，旧 token 失效
        let new_token = store2.pair_device(&owner, "pixel-7", "14", &caps).unwrap();
        assert_ne!(new_token, dev_token);
        assert!(store2.verify_device_token(&dev_token).is_none());
        assert!(store2.verify_device_token(&new_token).is_some());
    }

    #[test]
    fn 配对重置_吊销全部设备token_并重新出码() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_path(&dir);
        let (store, gen) = Store::open(&path, &[]).unwrap();
        let owner = gen.unwrap();
        let caps = vec![Capability::Tap];
        let t1 = store.pair_device(&owner, "dev-a", "12", &caps).unwrap();
        let t2 = store.pair_device(&owner, "dev-b", "13", &caps).unwrap();
        let old_code = store.issue_pairing_code(&owner).unwrap();

        let revoked = store.reset_owner(&owner).expect("owner 存在");
        assert_eq!(revoked.len(), 2);
        assert!(store.verify_device_token(&t1).is_none(), "旧 token 已吊销");
        assert!(store.verify_device_token(&t2).is_none());
        assert_eq!(store.consume_pairing_code(&old_code), None, "旧配对码失效");
        assert!(store.devices_of(&owner).is_empty());

        // reset 后可重新签发配对码
        let new_code = store.issue_pairing_code(&owner).unwrap();
        assert_eq!(store.consume_pairing_code(&new_code), Some(owner.clone()));
    }

    #[test]
    fn 重置不波及其他_owner() {
        let dir = tempfile::tempdir().unwrap();
        let path = temp_path(&dir);
        let other = "cd".repeat(32);
        let (_store, gen) = Store::open(&path, &[]).unwrap();
        let owner = gen.unwrap();
        // 二次启动注入另一个 owner
        let (store, _) = Store::open(&path, std::slice::from_ref(&other)).unwrap();
        let caps = vec![Capability::Tap];
        let other_dev = store.pair_device(&other, "other-dev", "12", &caps).unwrap();
        let my_dev = store.pair_device(&owner, "my-dev", "12", &caps).unwrap();

        store.reset_owner(&owner).unwrap();
        assert!(store.verify_device_token(&my_dev).is_none());
        assert!(
            store.verify_device_token(&other_dev).is_some(),
            "其他 owner 不受影响"
        );
    }

    #[test]
    fn touch_刷新_last_seen() {
        let dir = tempfile::tempdir().unwrap();
        let (store, gen) = Store::open(&temp_path(&dir), &[]).unwrap();
        let owner = gen.unwrap();
        store.pair_device(&owner, "dev", "12", &[]).unwrap();
        let before = store.devices_of(&owner)[0].last_seen;
        std::thread::sleep(std::time::Duration::from_millis(1100));
        store.touch_device(&owner, "dev");
        let after = store.devices_of(&owner)[0].last_seen;
        assert!(after > before, "last_seen 随 touch 刷新");
    }
}
