use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AdbNotFound,
    DeviceOffline,
    DeviceAmbiguous,
    DeviceNotFound,
    NotSupported,
    Timeout,
    AdbError,
    IoError,
    /// 命令用法/参数语义错误（对应退出码 2）
    Usage,
    /// 公网代理服务认证失败（owner token 无效/缺失）
    ProxyAuth,
    /// 公网代理服务通用错误（未配置、不可达、非预期响应）
    ProxyError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}
impl ErrorBody {
    pub fn new(
        code: ErrorCode,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            details,
        }
    }

    pub fn device_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DeviceNotFound, message, None)
    }

    pub fn device_offline(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DeviceOffline, message, None)
    }

    pub fn device_ambiguous(
        message: impl Into<String>,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self::new(ErrorCode::DeviceAmbiguous, message, details)
    }

    pub fn not_supported(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotSupported, message, None)
    }

    pub fn adb_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::AdbNotFound, message, None)
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Timeout, message, None)
    }

    pub fn adb_error(message: impl Into<String>, details: Option<serde_json::Value>) -> Self {
        Self::new(ErrorCode::AdbError, message, details)
    }

    pub fn io_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::IoError, message, None)
    }
    pub fn proxy_auth(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ProxyAuth, message, None)
    }
    pub fn proxy_error(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ProxyError, message, None)
    }
    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Usage, message, None)
    }
}

impl From<ErrorBody> for Output {
    fn from(e: ErrorBody) -> Self {
        Output::failure(e.code, e.message, e.details)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

impl Output {
    pub fn success(result: serde_json::Value) -> Self {
        Self {
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(
        code: ErrorCode,
        message: impl Into<String>,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self {
            ok: false,
            result: None,
            error: Some(ErrorBody {
                code,
                message: message.into(),
                details,
            }),
        }
    }

    pub fn exit_code(&self) -> i32 {
        if self.ok {
            0
        } else if self.error.as_ref().map(|e| e.code) == Some(ErrorCode::Usage) {
            2
        } else {
            1
        }
    }

    pub fn print(&self) {
        println!(
            "{}",
            serde_json::to_string_pretty(self).expect("输出序列化")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_json_shape() {
        let out = Output::success(serde_json::json!({"n": 1}));
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"]["n"], 1);
        assert!(v.get("error").is_none());
    }

    #[test]
    fn failure_json_shape_and_code() {
        let out = Output::failure(ErrorCode::DeviceOffline, "device offline", None);
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["code"], "DEVICE_OFFLINE");
        assert!(v.get("result").is_none());
        assert_eq!(out.exit_code(), 1);
    }

    #[test]
    fn error_code_serialization() {
        assert_eq!(
            serde_json::to_value(ErrorCode::AdbNotFound).unwrap(),
            "ADB_NOT_FOUND"
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::DeviceAmbiguous).unwrap(),
            "DEVICE_AMBIGUOUS"
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::NotSupported).unwrap(),
            "NOT_SUPPORTED"
        );
    }

    #[test]
    fn success_exit_code_zero() {
        assert_eq!(Output::success(serde_json::json!(null)).exit_code(), 0);
    }
}
