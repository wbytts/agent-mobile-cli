//! 桥接 WS 协议消息类型（单一来源，design.md 决策 3/11）。
//!
//! 本文件将被 App Rust core 以 path 直接引用：只允许依赖 serde/serde_json，
//! 不得引用 CLI 内部任何类型。字段与 docs/bridge-protocol.md 逐字段一致。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 设备能力集（hello 上报 / command method 取值，design.md 决策 6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Capability {
    Tap,
    Swipe,
    Input,
    Key,
    UiTree,
    Screenshot,
    Apps,
    Launch,
    Script,
}

/// 客户端（App）→ daemon 的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// 注册握手：首次配对提交 pairing_code，已配对凭 token；二者必居其一。
    Hello(Hello),
    /// 心跳：daemon 刷新 last_seen 并回 pong。
    Heartbeat,
    /// command/script 的执行回传（按 id 关联）。
    Result(ResultMessage),
}

/// 注册握手载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    /// 首次配对提交的 6 位一次性配对码（与 token 二选一）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_code: Option<String>,
    /// 已配对设备持有的长期 token（与 pairing_code 二选一）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// 设备名称（桥接设备 id 形如 `bridge:<device_name>`）。
    pub device_name: String,
    /// Android 版本（如 "12"）。
    pub android_version: String,
    /// 能力集。
    pub capabilities: Vec<Capability>,
}

/// command/script 执行回传。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultMessage {
    /// 与下发的 command/script id 一致。
    pub id: String,
    pub ok: bool,
    /// 成功时的结构化结果（截图为 base64 PNG 内联，design.md 决策 15）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// 失败时的错误描述。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// daemon → 客户端（App）的消息。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonMessage {
    /// hello 应答：ok=true 时首次配对附带新签发 token；ok=false 时附 error 并断开。
    HelloAck(HelloAck),
    /// 设备操作命令。
    Command(CommandMessage),
    /// 脚本下发（JS 源码，沙盒执行）。
    Script(ScriptMessage),
    /// result 接收确认。
    ResultAck(ResultAck),
    /// 心跳应答。
    Pong,
}

/// hello 应答载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelloAck {
    pub ok: bool,
    /// 配对码验证通过时新签发的长期 token（token 认证路径不重复下发）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// 拒绝原因（配对码错误/失效、缺少凭证等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 设备操作命令载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMessage {
    /// 请求标识（result 按此关联）。
    pub id: String,
    /// 动作名：tap/swipe/input/key/uiTree/screenshot/apps/launch。
    pub method: String,
    /// 动作参数（各 method 的 schema 见 docs/bridge-protocol.md）。
    pub params: Value,
}

/// 脚本下载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptMessage {
    /// 请求标识（result 按此关联）。
    pub id: String,
    /// JS 源码（QuickJS 沙盒执行，注入 mobile.* API）。
    pub source: String,
}

/// result 接收确认载荷。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultAck {
    pub id: String,
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hello_json() -> serde_json::Value {
        json!({
            "type": "hello",
            "pairing_code": "123456",
            "device_name": "MuMu",
            "android_version": "12",
            "capabilities": ["tap", "swipe", "uiTree", "screenshot", "script"]
        })
    }

    #[test]
    fn hello_with_pairing_code_roundtrip() {
        let msg: ClientMessage = serde_json::from_value(hello_json()).expect("解析 hello");
        let ClientMessage::Hello(h) = msg else {
            panic!("应解析为 hello")
        };
        assert_eq!(h.pairing_code.as_deref(), Some("123456"));
        assert_eq!(h.token, None);
        assert_eq!(h.device_name, "MuMu");
        assert_eq!(h.android_version, "12");
        assert_eq!(
            h.capabilities,
            vec![
                Capability::Tap,
                Capability::Swipe,
                Capability::UiTree,
                Capability::Screenshot,
                Capability::Script,
            ]
        );
        // 序列化回 JSON 后 type 标签与字段名一致
        let v = serde_json::to_value(ClientMessage::Hello(h)).expect("序列化 hello");
        assert_eq!(v["type"], "hello");
        assert_eq!(v["pairing_code"], "123456");
        assert!(v.get("token").is_none(), "None 字段不序列化");
    }

    #[test]
    fn hello_with_token_path() {
        let msg: ClientMessage = serde_json::from_value(json!({
            "type": "hello",
            "token": "ab".repeat(32),
            "device_name": "phone",
            "android_version": "14",
            "capabilities": []
        }))
        .expect("解析 token hello");
        let ClientMessage::Hello(h) = msg else {
            panic!("应解析为 hello")
        };
        assert_eq!(h.token.as_deref().map(str::len), Some(64));
        assert_eq!(h.pairing_code, None);
    }

    #[test]
    fn all_capabilities_parse() {
        for (s, cap) in [
            ("tap", Capability::Tap),
            ("swipe", Capability::Swipe),
            ("input", Capability::Input),
            ("key", Capability::Key),
            ("uiTree", Capability::UiTree),
            ("screenshot", Capability::Screenshot),
            ("apps", Capability::Apps),
            ("launch", Capability::Launch),
            ("script", Capability::Script),
        ] {
            let parsed: Capability = serde_json::from_value(json!(s)).expect("解析能力");
            assert_eq!(parsed, cap);
            assert_eq!(serde_json::to_value(cap).unwrap(), json!(s));
        }
    }

    #[test]
    fn heartbeat_and_result_parse() {
        let msg: ClientMessage = serde_json::from_value(json!({"type": "heartbeat"})).unwrap();
        assert!(matches!(msg, ClientMessage::Heartbeat));

        let msg: ClientMessage = serde_json::from_value(json!({
            "type": "result",
            "id": "cmd-1",
            "ok": true,
            "result": {"tapped": [100, 200]}
        }))
        .unwrap();
        let ClientMessage::Result(r) = msg else {
            panic!("应解析为 result")
        };
        assert_eq!(r.id, "cmd-1");
        assert!(r.ok);
        assert_eq!(r.result.unwrap()["tapped"], json!([100, 200]));
        assert!(r.error.is_none());

        let msg: ClientMessage = serde_json::from_value(json!({
            "type": "result",
            "id": "cmd-2",
            "ok": false,
            "error": "element not interactable"
        }))
        .unwrap();
        let ClientMessage::Result(r) = msg else {
            panic!("应解析为失败 result")
        };
        assert!(!r.ok);
        assert_eq!(r.error.as_deref(), Some("element not interactable"));
    }

    #[test]
    fn daemon_messages_serialize_with_type_tag() {
        let ack = DaemonMessage::HelloAck(HelloAck {
            ok: true,
            token: Some("t".into()),
            error: None,
        });
        let v = serde_json::to_value(&ack).unwrap();
        assert_eq!(v["type"], "hello_ack");
        assert_eq!(v["ok"], true);
        assert_eq!(v["token"], "t");
        assert!(v.get("error").is_none());

        let cmd = DaemonMessage::Command(CommandMessage {
            id: "cmd-1".into(),
            method: "tap".into(),
            params: json!({"x": 1, "y": 2}),
        });
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["type"], "command");
        assert_eq!(v["id"], "cmd-1");
        assert_eq!(v["method"], "tap");
        assert_eq!(v["params"], json!({"x": 1, "y": 2}));

        let script = DaemonMessage::Script(ScriptMessage {
            id: "cmd-2".into(),
            source: "mobile.tap(1,2)".into(),
        });
        let v = serde_json::to_value(&script).unwrap();
        assert_eq!(v["type"], "script");
        assert_eq!(v["source"], "mobile.tap(1,2)");

        let rack = DaemonMessage::ResultAck(ResultAck { id: "cmd-1".into() });
        let v = serde_json::to_value(&rack).unwrap();
        assert_eq!(v["type"], "result_ack");
        assert_eq!(v["id"], "cmd-1");

        let pong = DaemonMessage::Pong;
        let v = serde_json::to_value(&pong).unwrap();
        assert_eq!(v["type"], "pong");
    }

    #[test]
    fn daemon_messages_roundtrip_via_deserialize() {
        // App 侧反序列化路径：两端共用同一类型定义
        let v = json!({"type": "command", "id": "c1", "method": "uiTree", "params": {}});
        let msg: DaemonMessage = serde_json::from_value(v).unwrap();
        let DaemonMessage::Command(c) = msg else {
            panic!("应解析为 command")
        };
        assert_eq!(c.method, "uiTree");

        let v = json!({"type": "hello_ack", "ok": false, "error": "配对码错误或已失效"});
        let msg: DaemonMessage = serde_json::from_value(v).unwrap();
        let DaemonMessage::HelloAck(a) = msg else {
            panic!("应解析为 hello_ack")
        };
        assert!(!a.ok);
        assert_eq!(a.error.as_deref(), Some("配对码错误或已失效"));
    }

    #[test]
    fn unknown_type_rejected() {
        assert!(serde_json::from_value::<ClientMessage>(json!({"type": "bogus"})).is_err());
        assert!(serde_json::from_value::<DaemonMessage>(json!({"type": "bogus"})).is_err());
        // 未知能力名拒绝（协议契约固定枚举）
        assert!(serde_json::from_value::<Capability>(json!("reboot")).is_err());
    }
}
