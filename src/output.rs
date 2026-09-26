use serde::Serialize;

// 错误面由 design.md 决策 7 定义；组 2-4 逐命令接入后移除此 allow（Build 任务内清理）
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Output {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
}

#[allow(dead_code)]
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
