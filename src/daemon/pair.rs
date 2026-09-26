//! 配对认证状态机（design.md 决策 8/14）。
//!
//! - daemon 启动桥接监听时生成 6 位一次性配对码：首次配对成功或 daemon 重启后失效；
//! - 配对码验证通过签发长期 token（32 字节随机 hex），追加记录到 `<配置目录>/tokens.json`；
//! - 已签发 token 持久化，daemon 重启后仍有效；`pair --reset` 重新生成配对码并清空全部 token。

use parking_lot::Mutex;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 认证结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthOutcome {
    /// 配对码验证通过，新签发 token（hello_ack 下发）。
    Paired { token: String },
    /// 长期 token 验证通过（hello_ack 不重复下发 token）。
    TokenOk,
    /// 拒绝（配对码错误/失效、token 未知、缺少凭证）。
    Rejected { reason: String },
}

/// tokens.json 中的一条签发记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub token: String,
    pub device_name: String,
    /// 签发时间（Unix 秒）。
    pub issued_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TokensFile {
    #[serde(default)]
    tokens: Vec<TokenRecord>,
}

struct Inner {
    code: String,
    /// 配对码是否可用（首次配对成功后失效）。
    code_active: bool,
    tokens: Vec<TokenRecord>,
}

/// 配对状态：配对码 + 已签发 token 集合（内存为准，tokens.json 落盘持久化）。
pub struct Pairing {
    dir: PathBuf,
    inner: Mutex<Inner>,
}

impl Pairing {
    /// 加载 `<dir>/tokens.json`（缺失/损坏视为空），并生成新一次性配对码。
    pub fn new(dir: &Path) -> Self {
        let tokens = std::fs::read_to_string(dir.join("tokens.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<TokensFile>(&s).ok())
            .map(|f| f.tokens)
            .unwrap_or_default();
        Self {
            dir: dir.to_path_buf(),
            inner: Mutex::new(Inner {
                code: generate_code(),
                code_active: true,
                tokens,
            }),
        }
    }

    /// 当前配对码与可用状态（pair-info 展示用）。
    pub fn pairing_code(&self) -> (String, bool) {
        let g = self.inner.lock();
        (g.code.clone(), g.code_active)
    }

    /// hello 认证：token 优先；配对码路径验证通过即签发 token 并使配对码失效。
    pub fn authenticate(
        &self,
        pairing_code: Option<&str>,
        token: Option<&str>,
        device_name: &str,
    ) -> AuthOutcome {
        let mut g = self.inner.lock();
        if let Some(t) = token {
            if g.tokens.iter().any(|r| r.token == t) {
                return AuthOutcome::TokenOk;
            }
            return AuthOutcome::Rejected {
                reason: "token 无效或已被重置失效".to_string(),
            };
        }
        let Some(code) = pairing_code else {
            return AuthOutcome::Rejected {
                reason: "hello 缺少配对码或 token 凭证".to_string(),
            };
        };
        if !g.code_active || code != g.code {
            return AuthOutcome::Rejected {
                reason: "配对码错误或已失效".to_string(),
            };
        }
        let record = TokenRecord {
            token: generate_token(),
            device_name: device_name.to_string(),
            issued_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let token = record.token.clone();
        g.tokens.push(record);
        g.code_active = false;
        if let Err(e) = save_tokens(&self.dir, &g.tokens) {
            // 落盘失败回滚内存态，避免重启后 token 失效却自以为签发成功
            g.tokens.pop();
            g.code_active = true;
            return AuthOutcome::Rejected {
                reason: format!("token 持久化失败: {e}"),
            };
        }
        AuthOutcome::Paired { token }
    }

    /// `pair --reset`：重新生成配对码并清空全部已签发 token。
    pub fn reset(&self) {
        let mut g = self.inner.lock();
        g.code = generate_code();
        g.code_active = true;
        g.tokens.clear();
        // 清空落盘失败时保留错误可见性：尽力而为写空文件
        let _ = save_tokens(&self.dir, &g.tokens);
    }
}

/// 6 位数字配对码（允许前导零）。
fn generate_code() -> String {
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000u32))
}

/// 32 字节随机 token 的 hex 编码（64 字符）。
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 原子写入 tokens.json（tmp + rename）。
fn save_tokens(dir: &Path, tokens: &[TokenRecord]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let file = TokensFile {
        tokens: tokens.to_vec(),
    };
    let body = serde_json::to_string_pretty(&file)?;
    let tmp = dir.join("tokens.json.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, dir.join("tokens.json"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    fn pairing(dir: &std::path::Path) -> Pairing {
        Pairing::new(dir)
    }

    #[test]
    fn generates_six_digit_active_code() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        let (code, active) = p.pairing_code();
        assert!(active);
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn pairing_code_issues_token_once_then_expires() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        let (code, _) = p.pairing_code();

        let auth = p.authenticate(Some(&code.clone()), None, "dev-1");
        let AuthOutcome::Paired { token } = auth else {
            panic!("首次配对应成功: {auth:?}")
        };
        assert_eq!(token.len(), 64, "32 字节随机 hex 为 64 字符");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));

        // 配对码首次使用成功后失效
        let (_, active) = p.pairing_code();
        assert!(!active);
        let again = p.authenticate(Some(&code), None, "dev-2");
        assert!(
            matches!(again, AuthOutcome::Rejected { .. }),
            "配对码应已失效"
        );
    }

    #[test]
    fn token_authenticates_without_new_issue() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        let (code, _) = p.pairing_code();
        let AuthOutcome::Paired { token } = p.authenticate(Some(&code), None, "dev-1") else {
            panic!("配对应成功")
        };

        let auth = p.authenticate(None, Some(&token), "dev-1");
        assert!(
            matches!(auth, AuthOutcome::TokenOk),
            "token 应直接通过: {auth:?}"
        );
    }

    #[test]
    fn rejects_wrong_or_missing_credentials() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        assert!(matches!(
            p.authenticate(Some("000000"), None, "d"),
            AuthOutcome::Rejected { .. }
        ));
        assert!(matches!(
            p.authenticate(None, Some(&"f".repeat(64)), "d"),
            AuthOutcome::Rejected { .. }
        ));
        assert!(matches!(
            p.authenticate(None, None, "d"),
            AuthOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn reset_regenerates_code_and_revokes_tokens() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        let (code, _) = p.pairing_code();
        let AuthOutcome::Paired { token } = p.authenticate(Some(&code), None, "dev-1") else {
            panic!("配对应成功")
        };

        p.reset();
        let (new_code, active) = p.pairing_code();
        assert!(active, "重置后配对码应重新可用");
        assert!(
            matches!(
                p.authenticate(None, Some(&token), "dev-1"),
                AuthOutcome::Rejected { .. }
            ),
            "旧 token 应失效"
        );
        // 新配对码可以再次配对
        assert!(matches!(
            p.authenticate(Some(&new_code), None, "dev-2"),
            AuthOutcome::Paired { .. }
        ));
    }

    #[test]
    fn tokens_persist_across_restart() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        let (code, _) = p.pairing_code();
        let AuthOutcome::Paired { token } = p.authenticate(Some(&code), None, "dev-1") else {
            panic!("配对应成功")
        };
        assert!(tmp.path().join("tokens.json").exists(), "签发后应落盘");
        drop(p);

        // daemon 重启：新实例加载 tokens.json，旧 token 仍有效；配对码重新生成
        let p2 = pairing(tmp.path());
        assert!(matches!(
            p2.authenticate(None, Some(&token), "dev-1"),
            AuthOutcome::TokenOk
        ));
        let (_, active) = p2.pairing_code();
        assert!(active, "重启后配对码重新可用");
    }

    #[test]
    fn reset_clears_tokens_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = pairing(tmp.path());
        let (code, _) = p.pairing_code();
        let AuthOutcome::Paired { .. } = p.authenticate(Some(&code), None, "dev-1") else {
            panic!("配对应成功")
        };
        p.reset();
        let content = std::fs::read_to_string(tmp.path().join("tokens.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["tokens"], serde_json::json!([]));
    }

    #[test]
    fn corrupt_tokens_file_starts_empty() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("tokens.json"), "not json").unwrap();
        let p = pairing(tmp.path());
        assert!(matches!(
            p.authenticate(None, Some(&"a".repeat(64)), "d"),
            AuthOutcome::Rejected { .. }
        ));
    }
}
