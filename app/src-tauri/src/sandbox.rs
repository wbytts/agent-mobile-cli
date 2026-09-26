//! QuickJS 脚本沙盒（design.md 决策 2/10）。
//!
//! 每次执行脚本新建 Runtime/Context；不注册 std/os 等宿主模块，
//! 脚本仅能通过显式注入的 `mobile.*` 全局对象触达设备能力。

use std::sync::Arc;

/// 设备能力桥：生产实现经 Tauri 插件调到 Kotlin（Android），单测用 mock。
///
/// 各方法返回 JSON 文本（与 Kotlin 插件 JSObject 序列化结果一致），
/// 沙盒层负责解析回 JS 值；错误为面向脚本/调用方的可读消息。
pub trait MobileOps: Send + Sync {
    fn tap(&self, x: f64, y: f64) -> Result<String, String>;
    fn swipe(&self, x1: f64, y1: f64, x2: f64, y2: f64, duration_ms: f64)
        -> Result<String, String>;
    fn input(&self, text: &str) -> Result<String, String>;
    fn key(&self, key: &str) -> Result<String, String>;
    fn ui_tree(&self) -> Result<String, String>;
    /// 返回 JSON 文本，如 `{"png_base64":"..."}`。
    fn screenshot(&self) -> Result<String, String>;
    fn apps(&self) -> Result<String, String>;
    fn launch(&self, package: &str) -> Result<String, String>;
}

/// QuickJS 沙盒执行器。
pub struct Sandbox {
    ops: Arc<dyn MobileOps>,
}

impl Sandbox {
    pub fn new(ops: Arc<dyn MobileOps>) -> Self {
        Self { ops }
    }

    /// 执行脚本并返回其结果（JSON 值）；脚本异常或 `mobile.*` 失败返回 Err。
    pub fn run(&self, _script: &str) -> Result<serde_json::Value, String> {
        todo!("沙盒执行未实现")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    #[derive(Default)]
    struct RecordedCall {
        method: &'static str,
        args: Vec<String>,
    }

    /// 记录调用并按方法名返回预置结果的 mock 桥。
    struct MockOps {
        calls: Mutex<Vec<RecordedCall>>,
        ui_tree_json: String,
        screenshot_json: String,
        fail_on: Option<&'static str>,
    }

    impl MockOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                ui_tree_json: r#"{"text":"设置","class":"TextView","bounds":[0,0,10,10],"clickable":true,"children":[]}"#.to_string(),
                screenshot_json: r#"{"png_base64":"aGVsbG8="}"#.to_string(),
                fail_on: None,
            }
        }

        fn record(&self, method: &'static str, args: Vec<String>) -> Result<String, String> {
            self.calls.lock().push(RecordedCall { method, args });
            if self.fail_on == Some(method) {
                return Err(format!("{method} 被 mock 拒绝"));
            }
            Ok("null".to_string())
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls
                .lock()
                .iter()
                .map(|c| (c.method.to_string(), c.args.clone()))
                .collect()
        }
    }

    impl MobileOps for MockOps {
        fn tap(&self, x: f64, y: f64) -> Result<String, String> {
            self.record("tap", vec![x.to_string(), y.to_string()])
        }
        fn swipe(
            &self,
            x1: f64,
            y1: f64,
            x2: f64,
            y2: f64,
            duration_ms: f64,
        ) -> Result<String, String> {
            self.record(
                "swipe",
                vec![
                    x1.to_string(),
                    y1.to_string(),
                    x2.to_string(),
                    y2.to_string(),
                    duration_ms.to_string(),
                ],
            )
        }
        fn input(&self, text: &str) -> Result<String, String> {
            self.record("input", vec![text.to_string()])
        }
        fn key(&self, key: &str) -> Result<String, String> {
            self.record("key", vec![key.to_string()])
        }
        fn ui_tree(&self) -> Result<String, String> {
            self.record("uiTree", vec![])?;
            Ok(self.ui_tree_json.clone())
        }
        fn screenshot(&self) -> Result<String, String> {
            self.record("screenshot", vec![])?;
            Ok(self.screenshot_json.clone())
        }
        fn apps(&self) -> Result<String, String> {
            self.record("apps", vec![])?;
            Ok(r#"{"apps":[{"label":"设置","package":"com.android.settings"}]}"#.to_string())
        }
        fn launch(&self, package: &str) -> Result<String, String> {
            self.record("launch", vec![package.to_string()])
        }
    }

    fn sandbox_with(ops: Arc<MockOps>) -> Sandbox {
        Sandbox::new(ops)
    }

    #[test]
    fn tap_reaches_bridge_with_coords() {
        let ops = Arc::new(MockOps::new());
        let result = sandbox_with(ops.clone())
            .run(r#"mobile.tap(100, 250); "done""#)
            .expect("脚本应成功");
        assert_eq!(result, serde_json::json!("done"));
        let calls = ops.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "tap");
        assert_eq!(calls[0].1, vec!["100".to_string(), "250".to_string()]);
    }

    #[test]
    fn swipe_reaches_bridge_with_all_args() {
        let ops = Arc::new(MockOps::new());
        sandbox_with(ops.clone())
            .run("mobile.swipe(10, 20, 300, 400, 500)")
            .expect("脚本应成功");
        let calls = ops.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "swipe");
        assert_eq!(
            calls[0].1,
            vec!["10", "20", "300", "400", "500"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn script_return_value_roundtrip() {
        let sandbox = sandbox_with(Arc::new(MockOps::new()));
        assert_eq!(sandbox.run("1 + 2").unwrap(), serde_json::json!(3));
        assert_eq!(
            sandbox.run(r#"({a: 1, b: "x"})"#).unwrap(),
            serde_json::json!({"a": 1, "b": "x"})
        );
        assert_eq!(sandbox.run("undefined").unwrap(), serde_json::Value::Null);
    }

    #[test]
    fn script_exception_propagates() {
        let err = sandbox_with(Arc::new(MockOps::new()))
            .run(r#"throw new Error("boom")"#)
            .expect_err("应返回脚本异常");
        assert!(err.contains("boom"), "错误信息应含脚本异常: {err}");
    }

    #[test]
    fn sandbox_has_no_host_modules() {
        let sandbox = sandbox_with(Arc::new(MockOps::new()));
        let result = sandbox
            .run("[typeof require, typeof std, typeof os, typeof process]")
            .expect("脚本应成功");
        assert_eq!(
            result,
            serde_json::json!(["undefined", "undefined", "undefined", "undefined"])
        );
    }

    #[test]
    fn ui_tree_result_available_as_object() {
        let ops = Arc::new(MockOps::new());
        let result = sandbox_with(ops)
            .run("mobile.uiTree().text")
            .expect("脚本应成功");
        assert_eq!(result, serde_json::json!("设置"));
    }

    #[test]
    fn screenshot_result_available_as_object() {
        let ops = Arc::new(MockOps::new());
        let result = sandbox_with(ops)
            .run("mobile.screenshot().png_base64")
            .expect("脚本应成功");
        assert_eq!(result, serde_json::json!("aGVsbG8="));
    }

    #[test]
    fn all_eight_mobile_functions_exist() {
        let result = sandbox_with(Arc::new(MockOps::new()))
            .run(
                r#"["tap","swipe","input","key","uiTree","screenshot","apps","launch"]
                    .map(n => typeof mobile[n])"#,
            )
            .expect("脚本应成功");
        assert_eq!(
            result,
            serde_json::json!(vec!["function"; 8])
        );
    }

    #[test]
    fn bridge_failure_becomes_script_error() {
        let mut ops = MockOps::new();
        ops.fail_on = Some("tap");
        let err = sandbox_with(Arc::new(ops))
            .run("mobile.tap(1, 2)")
            .expect_err("桥失败应使脚本失败");
        assert!(err.contains("tap 被 mock 拒绝"), "错误应含桥消息: {err}");
    }

    #[test]
    fn input_key_launch_forward_arguments() {
        let ops = Arc::new(MockOps::new());
        sandbox_with(ops.clone())
            .run(r#"mobile.input("你好"); mobile.key("back"); mobile.launch("com.android.settings")"#)
            .expect("脚本应成功");
        let calls = ops.calls();
        assert_eq!(
            calls,
            vec![
                ("input".to_string(), vec!["你好".to_string()]),
                ("key".to_string(), vec!["back".to_string()]),
                ("launch".to_string(), vec!["com.android.settings".to_string()]),
            ]
        );
    }
}
