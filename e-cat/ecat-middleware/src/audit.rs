// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
//! 审计日志层：对写方法（POST/PUT/PATCH/DELETE）提取 JWT 上下文记录审计事件。
//! 存储经 `AuditSink` 抽象，middleware 不持有任何 DB 依赖；
//! 网关侧用 MySQL sink 落库，测试用内存 sink 断言。

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use ecat_auth::AuthClaims;
use std::sync::Arc;

/// 审计事件：租户 + 操作者 + 角色 + 请求方法/路径 + 响应状态码 + 来源 IP。
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub tenant_id: String,
    /// 操作者（JWT `actor` claim：登录用户名 / 开放 API 密钥 id）；
    /// 无 claims（登录端点）时取自请求体的 username，旧 token 或解析失败为空串。
    pub actor: String,
    pub role: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// 来源 IP（网关 middleware 解析并注入；未注入时为空串）。
    pub ip: String,
}

/// 可信来源 IP 标记：由网关在请求扩展中注入，审计层只读不解析。
/// 解析（X-Real-IP / ConnectInfo）集中在网关侧，避免信任头配置散落各处。
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

/// 审计事件落库抽象。
#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn record(&self, event: AuditEvent);
}

/// 无操作 sink：审计未配置时静默跳过。
pub struct NullSink;
#[async_trait]
impl AuditSink for NullSink {
    async fn record(&self, _event: AuditEvent) {}
}

/// 内存 sink：测试用，收集事件以便断言。
#[derive(Default)]
pub struct MemorySink {
    pub events: tokio::sync::Mutex<Vec<AuditEvent>>,
}
#[async_trait]
impl AuditSink for MemorySink {
    async fn record(&self, event: AuditEvent) {
        self.events.lock().await.push(event);
    }
}

const WRITE_METHODS: [Method; 4] = [Method::POST, Method::PUT, Method::PATCH, Method::DELETE];

/// 审计中间件状态：注入 AuditSink。
#[derive(Clone)]
pub struct AuditState(pub Arc<dyn AuditSink>);

/// 登录请求体缓冲上限：只取 username，正常登录体远小于此。
const LOGIN_BODY_LIMIT: usize = 64 * 1024;

/// 从登录请求体提取 username，返回重建后的请求（body 原样保留，供后续转发）。
/// 超过上限返回 413 且不转发；空/非 JSON body 不报错，用户名留空。
/// 只解析 username，密码等字段绝不进入日志。
#[allow(clippy::result_large_err)] // Err 是 413 Response；直接返回避免再包一层错误类型
async fn extract_login_actor(req: Request) -> Result<(Request, Option<String>), Response> {
    let (parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, LOGIN_BODY_LIMIT).await else {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response());
    };
    // actor 列为 VARCHAR(64)：空/空白用户名记空串，超长按字符截断，
    // 避免恶意长串导致落库失败丢审计。
    let username = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| v.get("username")?.as_str().map(str::to_owned))
        .filter(|u| !u.trim().is_empty())
        .map(|u| u.chars().take(64).collect());
    Ok((Request::from_parts(parts, Body::from(bytes)), username))
}

/// axum 中间件：写操作（POST/PUT/PATCH/DELETE）在响应完成后异步记录审计事件。
/// 挂载在 JWT 层内层（extensions 已注入 AuthClaims），缺 claims 记 anonymous
/// （如登录端点防爆破审计）；登录路径从请求体 username 补全 actor。
pub async fn audit_middleware(
    State(state): State<AuditState>,
    mut req: Request,
    next: Next,
) -> Response {
    let is_write = WRITE_METHODS.contains(req.method());
    let claims = req.extensions().get::<AuthClaims>().cloned();
    let ip = req
        .extensions()
        .get::<ClientIp>()
        .map(|c| c.0.clone())
        .unwrap_or_default();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    // 登录没有 AuthClaims：从 body 提取 username 作为 actor（不记录密码）。
    let mut login_actor = None;
    if is_write && claims.is_none() && path.ends_with("/auth/login") {
        match extract_login_actor(req).await {
            Ok((rebuilt, username)) => {
                req = rebuilt;
                login_actor = username;
            }
            Err(resp) => return resp,
        }
    }
    let resp = next.run(req).await;
    if is_write {
        let status = resp.status().as_u16();
        let (tenant_id, actor, role) = match &claims {
            Some(c) => (
                c.sub.clone(),
                c.extra
                    .get("actor")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                c.role.clone().unwrap_or_default(),
            ),
            None => (
                "anonymous".into(),
                login_actor.unwrap_or_default(),
                String::new(),
            ),
        };
        let sink = state.0.clone();
        let event = AuditEvent {
            tenant_id,
            actor,
            role,
            method,
            path,
            status,
            ip,
        };
        // 非阻塞落库：失败仅 sink 内 warn，不阻塞业务响应。
        tokio::spawn(async move { sink.record(event).await });
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware::from_fn_with_state;
    use axum::routing::post;
    use tower::ServiceExt;

    fn claims_ext(tenant: &str, role: &str) -> AuthClaims {
        let mut extra = std::collections::HashMap::new();
        extra.insert("actor".into(), serde_json::json!("alice"));
        AuthClaims {
            sub: tenant.into(),
            exp: None,
            iat: None,
            role: Some(role.into()),
            extra,
        }
    }

    /// 登录端点回显请求体：断言审计层缓冲后原样转发。
    async fn echo(req: Request<Body>) -> axum::response::Response {
        let bytes = axum::body::to_bytes(req.into_body(), usize::MAX)
            .await
            .unwrap_or_default();
        axum::response::Response::new(Body::from(bytes))
    }

    fn app_with(sink: Arc<MemorySink>) -> Router {
        Router::new()
            .route("/devices", post(|| async { "ok" }).get(|| async { "ok" }))
            .route("/auth/login", post(echo))
            .layer(from_fn_with_state(AuditState(sink), audit_middleware))
    }

    async fn post_json(sink: &Arc<MemorySink>, uri: &str, body: &str) -> (u16, String) {
        let app = app_with(sink.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn write_requests_are_audited_with_claims() {
        let sink = Arc::new(MemorySink::default());
        let app = app_with(sink.clone());

        let mut req = Request::builder()
            .method(Method::POST)
            .uri("/devices")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(claims_ext("tenant-1", "admin"));
        req.extensions_mut().insert(ClientIp("10.1.2.3".into()));

        let _ = app.oneshot(req).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = sink.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tenant_id, "tenant-1");
        assert_eq!(events[0].actor, "alice");
        assert_eq!(events[0].role, "admin");
        assert_eq!(events[0].path, "/devices");
        assert_eq!(events[0].status, 200);
        assert_eq!(events[0].ip, "10.1.2.3");
    }

    #[tokio::test]
    async fn login_extracts_username_and_preserves_body() {
        let sink = Arc::new(MemorySink::default());
        let body = r#"{"username":"bob","password":"s3cret"}"#;
        let (status, echoed) = post_json(&sink, "/auth/login", body).await;
        assert_eq!(status, 200);
        assert_eq!(echoed, body, "buffered body must be forwarded unchanged");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = sink.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tenant_id, "anonymous");
        assert_eq!(events[0].actor, "bob");
        assert_eq!(events[0].path, "/auth/login");
        assert_eq!(events[0].status, 200);
    }

    #[tokio::test]
    async fn empty_and_malformed_login_bodies_still_forward() {
        let sink = Arc::new(MemorySink::default());
        let (status, echoed) = post_json(&sink, "/auth/login", "").await;
        assert_eq!((status, echoed.as_str()), (200, ""));
        let (status, echoed) = post_json(&sink, "/auth/login", "{not json").await;
        assert_eq!((status, echoed.as_str()), (200, "{not json"));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = sink.events.lock().await;
        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.actor.is_empty()));
    }

    #[tokio::test]
    async fn login_actor_is_bounded_and_ignores_blank_usernames() {
        let sink = Arc::new(MemorySink::default());
        let long = "u".repeat(200);
        let body = format!(r#"{{"username":"{long}","password":"p"}}"#);
        assert_eq!(post_json(&sink, "/auth/login", &body).await.0, 200);
        assert_eq!(
            post_json(&sink, "/auth/login", r#"{"username":"   "}"#)
                .await
                .0,
            200
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = sink.events.lock().await;
        assert_eq!(events.len(), 2);
        assert_eq!(
            events.iter().filter(|e| e.actor == "u".repeat(64)).count(),
            1,
            "long username must be truncated to 64 chars"
        );
        assert_eq!(
            events.iter().filter(|e| e.actor.is_empty()).count(),
            1,
            "blank username must not become actor"
        );
    }

    #[tokio::test]
    async fn oversized_login_body_is_rejected_with_413() {
        let sink = Arc::new(MemorySink::default());
        let body = "x".repeat(LOGIN_BODY_LIMIT + 1);
        let (status, echoed) = post_json(&sink, "/auth/login", &body).await;
        assert_eq!(status, 413);
        assert!(
            echoed.contains("too large"),
            "oversized body must be rejected, not forwarded: {echoed}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(sink.events.lock().await.is_empty());
    }

    #[tokio::test]
    async fn read_requests_are_not_audited() {
        let sink = Arc::new(MemorySink::default());
        let app = app_with(sink.clone());

        let req = Request::builder()
            .uri("/devices")
            .body(Body::empty())
            .unwrap();
        let _ = app.oneshot(req).await.unwrap();
        assert!(sink.events.lock().await.is_empty());
    }

    #[tokio::test]
    async fn anonymous_write_is_audited() {
        let sink = Arc::new(MemorySink::default());
        let app = app_with(sink.clone());

        let req = Request::builder()
            .method(Method::POST)
            .uri("/auth/login")
            .body(Body::empty())
            .unwrap();
        let _ = app.oneshot(req).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let events = sink.events.lock().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tenant_id, "anonymous");
        assert_eq!(events[0].actor, "");
        assert_eq!(events[0].ip, "", "no ClientIp extension => empty ip");
    }
}
