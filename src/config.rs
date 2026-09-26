use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

pub const DEFAULT_HTTP_PORT: u16 = 18775;
pub const DEFAULT_BRIDGE_PORT: u16 = 18777;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub http_port: u16,
    pub bridge_port: u16,
    pub adb_path: Option<PathBuf>,
    pub default_device: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            http_port: DEFAULT_HTTP_PORT,
            bridge_port: DEFAULT_BRIDGE_PORT,
            adb_path: None,
            default_device: None,
        }
    }
}

impl Config {
    /// 用户级配置目录：~/.agent-mobile-cli（可用 AGENT_MOBILE_HOME 覆盖，便于测试）
    pub fn dir() -> PathBuf {
        if let Ok(dir) = std::env::var("AGENT_MOBILE_HOME") {
            return PathBuf::from(dir);
        }
        dirs_home().join(".agent-mobile-cli")
    }

    pub fn path() -> PathBuf {
        Self::dir().join("config.json")
    }

    /// 读取配置；文件缺失时自动生成默认配置后返回。
    pub fn load_or_create() -> std::io::Result<Self> {
        let path = Self::path();
        if path.exists() {
            let text = fs::read_to_string(&path)?;
            let cfg: Config = serde_json::from_str(&text).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("配置解析失败: {e}"),
                )
            })?;
            return Ok(cfg);
        }
        let cfg = Config::default();
        cfg.save()?;
        Ok(cfg)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let dir = Self::dir();
        fs::create_dir_all(&dir)?;
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(Self::path(), text)
    }
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|_| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_values() {
        let cfg = Config::default();
        assert_eq!(cfg.http_port, 18775);
        assert_eq!(cfg.bridge_port, 18777);
        assert!(cfg.adb_path.is_none());
        assert!(cfg.default_device.is_none());
    }

    #[test]
    fn creates_default_when_missing() {
        let _g = ENV_LOCK.lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT_MOBILE_HOME", tmp.path());
        let cfg = Config::load_or_create().unwrap();
        assert_eq!(cfg, Config::default());
        assert!(Config::path().exists());
        let text = fs::read_to_string(Config::path()).unwrap();
        assert!(text.contains("18775"));
        std::env::remove_var("AGENT_MOBILE_HOME");
    }

    #[test]
    fn loads_existing() {
        let _g = ENV_LOCK.lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT_MOBILE_HOME", tmp.path());
        let cfg = Config {
            http_port: 19000,
            default_device: Some("127.0.0.1:5555".into()),
            ..Config::default()
        };
        cfg.save().unwrap();
        let loaded = Config::load_or_create().unwrap();
        assert_eq!(loaded.http_port, 19000);
        assert_eq!(loaded.default_device.as_deref(), Some("127.0.0.1:5555"));
        std::env::remove_var("AGENT_MOBILE_HOME");
    }

    #[test]
    fn rejects_invalid_json() {
        let _g = ENV_LOCK.lock();
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("AGENT_MOBILE_HOME", tmp.path());
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(tmp.path().join("config.json"), "{ not json").unwrap();
        assert!(Config::load_or_create().is_err());
        std::env::remove_var("AGENT_MOBILE_HOME");
    }
}
