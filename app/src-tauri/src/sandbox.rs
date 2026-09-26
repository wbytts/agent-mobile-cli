//! QuickJS 脚本沙盒（design.md 决策 2/10）。
//!
//! 每次执行脚本新建 Runtime/Context；不注册 std/os 等宿主模块，
//! 脚本仅能通过显式注入的 `mobile.*` 全局对象触达设备能力。

use std::sync::Arc;

use rquickjs::{Ctx, Exception, Function, Object, Value};

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
    /// 返回 `{"xml": "<与 uiautomator dump 同构的 XML>"}`（桥接协议契约，CLI 侧 ui::simplify 复用）。
    fn ui_tree(&self) -> Result<String, String>;
    /// 返回 `{"png_base64": "..."}`。
    fn screenshot(&self) -> Result<String, String>;
    /// 返回 `{"packages": ["..."]}`；filter 为包名/标签子串（空为不过滤），all 含无启动入口应用。
    fn apps(&self, filter: Option<&str>, all: bool) -> Result<String, String>;
    fn launch(&self, package: &str) -> Result<String, String>;
    /// 返回 `{"enabled": bool}`，无障碍服务是否已启用。
    fn a11y_status(&self) -> Result<String, String>;
    /// 跳转系统无障碍设置页。
    fn open_a11y_settings(&self) -> Result<String, String>;
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
    pub fn run(&self, script: &str) -> Result<serde_json::Value, String> {
        // Runtime/Context 每脚本新建（design.md 决策 2）：脚本间无状态泄漏。
        let runtime =
            rquickjs::Runtime::new().map_err(|e| format!("创建 QuickJS Runtime 失败: {e}"))?;
        let context = rquickjs::Context::full(&runtime)
            .map_err(|e| format!("创建 QuickJS Context 失败: {e}"))?;
        context.with(|ctx| {
            register_mobile(&ctx, &self.ops)?;
            // 先试 eval（最后表达式语义）；命中顶层 return 语法限制时按 IIFE 重试（return 语义），
            // 两种脚本风格都可用——与 Node REPL 的回退策略一致。
            let value: Value = match ctx.eval(script) {
                Ok(v) => v,
                Err(e) => {
                    let msg = script_error(&ctx, e);
                    if msg.contains("return not in a function") {
                        let wrapped = format!("(function(){{\n{script}\n}})()");
                        ctx.eval(wrapped).map_err(|e2| script_error(&ctx, e2))?
                    } else {
                        return Err(msg);
                    }
                }
            };
            js_value_to_json(&ctx, value)
        })
    }
}

/// 向全局注入 `mobile.*` 八个设备操作函数；桥失败转为 JS 异常。
fn register_mobile(ctx: &Ctx<'_>, ops: &Arc<dyn MobileOps>) -> Result<(), String> {
    let mobile = Object::new(ctx.clone()).map_err(|e| format!("创建 mobile 对象失败: {e}"))?;

    macro_rules! bind {
        ($name:literal, $f:expr) => {
            mobile
                .set(
                    $name,
                    Function::new(ctx.clone(), $f)
                        .map_err(|e| format!("注册 {} 失败: {e}", $name))?,
                )
                .map_err(|e| format!("绑定 {} 失败: {e}", $name))?
        };
    }

    {
        let ops = ops.clone();
        bind!("tap", move |ctx: Ctx<'_>, x: f64, y: f64| {
            call_op(&ctx, ops.tap(x, y))
        });
    }
    {
        let ops = ops.clone();
        bind!("swipe", move |ctx: Ctx<'_>,
                             x1: f64,
                             y1: f64,
                             x2: f64,
                             y2: f64,
                             duration_ms: f64| {
            call_op(&ctx, ops.swipe(x1, y1, x2, y2, duration_ms))
        });
    }
    {
        let ops = ops.clone();
        bind!("input", move |ctx: Ctx<'_>, text: String| {
            call_op(&ctx, ops.input(&text))
        });
    }
    {
        let ops = ops.clone();
        bind!("key", move |ctx: Ctx<'_>, key: String| {
            call_op(&ctx, ops.key(&key))
        });
    }
    {
        let ops = ops.clone();
        bind!("uiTree", move |ctx: Ctx<'_>| call_op(&ctx, ops.ui_tree()));
    }
    {
        let ops = ops.clone();
        bind!("screenshot", move |ctx: Ctx<'_>| call_op(
            &ctx,
            ops.screenshot()
        ));
    }
    {
        let ops = ops.clone();
        bind!("apps", move |ctx: Ctx<'_>,
                            filter: Option<String>,
                            all: Option<bool>| {
            call_op(&ctx, ops.apps(filter.as_deref(), all.unwrap_or(false)))
        });
    }
    {
        let ops = ops.clone();
        bind!("launch", move |ctx: Ctx<'_>, package: String| {
            call_op(&ctx, ops.launch(&package))
        });
    }

    ctx.globals()
        .set("mobile", mobile)
        .map_err(|e| format!("注入 mobile 全局对象失败: {e}"))
}

/// 桥返回的 JSON 文本包装：经 `IntoJs` 在调用现场解析为 JS 值，
/// 规避闭包返回 `Value<'js>` 的生命周期约束。
struct JsonValue(String);

impl<'js> rquickjs::IntoJs<'js> for JsonValue {
    fn into_js(self, ctx: &Ctx<'js>) -> rquickjs::Result<Value<'js>> {
        json_parse(ctx, &self.0)
    }
}

/// 桥调用结果：成功返回待解析 JSON；失败抛带消息的 JS 异常。
fn call_op(ctx: &Ctx<'_>, result: Result<String, String>) -> rquickjs::Result<JsonValue> {
    match result {
        Ok(text) => Ok(JsonValue(text)),
        Err(message) => Err(throw_message(ctx, &message)),
    }
}

/// JSON.parse；桥返回的 null 解析为 JS null。
fn json_parse<'js>(ctx: &Ctx<'js>, text: &str) -> rquickjs::Result<Value<'js>> {
    let json: Object<'js> = ctx.globals().get("JSON")?;
    let parse: Function<'js> = json.get("parse")?;
    parse.call((text,))
}

/// 构造带消息的 JS Error 异常。
fn throw_message(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    match Exception::from_message(ctx.clone(), message) {
        Ok(exception) => ctx.throw(exception.into_object().into_value()),
        Err(e) => e,
    }
}

/// eval 失败时提取脚本异常消息（含栈）。
fn script_error(ctx: &Ctx<'_>, error: rquickjs::Error) -> String {
    if matches!(error, rquickjs::Error::Exception) {
        let caught = ctx.catch();
        if let Some(obj) = caught.as_object().cloned() {
            if let Some(exception) = Exception::from_object(obj) {
                let message = exception
                    .message()
                    .unwrap_or_else(|| "未知异常".to_string());
                return match exception.stack() {
                    Some(stack) if !stack.is_empty() => format!("脚本异常: {message}\n{stack}"),
                    _ => format!("脚本异常: {message}"),
                };
            }
        }
        return "脚本异常（非 Error 值）".to_string();
    }
    format!("脚本执行失败: {error}")
}

/// JS 值经 JSON.stringify 转 serde_json；undefined 归为 null。
fn js_value_to_json<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<serde_json::Value, String> {
    if value.is_undefined() {
        return Ok(serde_json::Value::Null);
    }
    let json: Object = ctx
        .globals()
        .get("JSON")
        .map_err(|e| format!("访问 JSON 全局失败: {e}"))?;
    let stringify: Function = json
        .get("stringify")
        .map_err(|e| format!("访问 JSON.stringify 失败: {e}"))?;
    let text: Option<String> = stringify.call((value,)).map_err(|e| script_error(ctx, e))?;
    match text {
        None => Ok(serde_json::Value::Null),
        Some(text) => serde_json::from_str(&text).map_err(|e| format!("脚本结果序列化失败: {e}")),
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
                ui_tree_json: r#"{"xml":"<?xml version=\"1.0\" encoding=\"UTF-8\"?><hierarchy><node text=\"设置\" class=\"TextView\" bounds=\"[0,0][10,10]\" clickable=\"true\"/></hierarchy>"}"#.to_string(),
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
        fn apps(&self, filter: Option<&str>, all: bool) -> Result<String, String> {
            self.record(
                "apps",
                vec![filter.unwrap_or("").to_string(), all.to_string()],
            )?;
            Ok(r#"{"packages":["com.android.settings"]}"#.to_string())
        }
        fn launch(&self, package: &str) -> Result<String, String> {
            self.record("launch", vec![package.to_string()])
        }
        fn a11y_status(&self) -> Result<String, String> {
            self.record("a11yStatus", vec![])?;
            Ok(r#"{"enabled":true}"#.to_string())
        }
        fn open_a11y_settings(&self) -> Result<String, String> {
            self.record("openA11ySettings", vec![])
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
        // 顶层 return 写法经 IIFE 回退同样可用
        assert_eq!(
            sandbox.run("const x = 41; return x + 1;").unwrap(),
            serde_json::json!(42)
        );
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
            .run("mobile.uiTree().xml.includes(\"设置\")")
            .expect("脚本应成功");
        assert_eq!(result, serde_json::json!(true));
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
        assert_eq!(result, serde_json::json!(vec!["function"; 8]));
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
                (
                    "launch".to_string(),
                    vec!["com.android.settings".to_string()]
                ),
            ]
        );
    }
}
