//! Bearer 认证中间件（design.md 决策 3/4）。
//!
//! 除 `GET /healthz` 外全部 HTTP API 需 `Authorization: Bearer <owner-token>`；
//! 缺失或无效凭证返回 401 `{"error":"unauthorized"}`。

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::sync::Arc;

use crate::AppState;

/// 认证通过后注入请求扩展的 owner 身份（owner token）。
#[derive(Debug, Clone)]
pub struct OwnerIdentity(pub String);

/// 401 结构化错误（契约固定形状）。
pub fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": "unauthorized"})),
    )
        .into_response()
}

/// Bearer 认证中间件：校验 owner token，注入 [`OwnerIdentity`]。
pub async fn require_owner(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let token: Option<String> = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string);
    match token {
        Some(t) if state.store.is_owner_token(&t) => {
            req.extensions_mut().insert(OwnerIdentity(t));
            next.run(req).await
        }
        _ => unauthorized(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    /// 无凭证访问受保护接口 → 401 {"error":"unauthorized"}。
    #[tokio::test]
    async fn 缺失凭证_401() {
        let (addr, _state, _dir) = crate::testutil::spawn_test_server().await;
        let (status, body) = crate::testutil::http_get(addr, "/devices", None).await;
        assert_eq!(status, 401);
        assert_eq!(body, serde_json::json!({"error": "unauthorized"}));
    }

    /// 错误 token → 401；合法 owner token → 200。
    #[tokio::test]
    async fn 错误凭证拒绝_合法放行() {
        let (addr, state, _dir) = crate::testutil::spawn_test_server().await;

        let (status, body) = crate::testutil::http_get(addr, "/devices", Some("deadbeef")).await;
        assert_eq!(status, 401);
        assert_eq!(body, serde_json::json!({"error": "unauthorized"}));

        let (status, body) =
            crate::testutil::http_get(addr, "/devices", Some(&state.owner_token)).await;
        assert_eq!(status, 200, "合法 owner token 放行: {body}");
        let devices: &Value = &body["devices"];
        assert!(devices.is_array(), "返回 devices 数组: {body}");
    }

    /// /healthz 无认证可访问。
    #[tokio::test]
    async fn healthz_免认证() {
        let (addr, _state, _dir) = crate::testutil::spawn_test_server().await;
        let (status, body) = crate::testutil::http_get(addr, "/healthz", None).await;
        assert_eq!(status, 200);
        assert_eq!(body["ok"], serde_json::json!(true));
    }
}
