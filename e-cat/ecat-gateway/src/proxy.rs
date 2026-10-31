use crate::service::ServiceResolver;
use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use ecat_auth::AuthClaims;

/// iot-access 内部地址（生产走服务发现，P1 直连；ACCESS_BASE 环境变量可覆盖，
/// 供端口冲突的本地环境用）。
const ACCESS_BASE: &str = "http://localhost:8082";
/// iot-data 内部地址（DATA_BASE 环境变量可覆盖）。
const DATA_BASE: &str = "http://localhost:8083";
/// iot-rule 内部地址（RULE_BASE 环境变量可覆盖）。
const RULE_BASE: &str = "http://localhost:8084";
/// iot-cdn 内部地址（CDN_BASE 环境变量可覆盖）。
const CDN_BASE: &str = "http://localhost:8085";
/// iot-device 内部地址（DEVICE_BASE 环境变量可覆盖）。
const DEVICE_BASE: &str = "http://localhost:8081";
/// 转发请求体上限 64MiB，与 device 固件上传上限（MAX_FIRMWARE_SIZE）对齐。
/// 普通路由仍由外层 SecurityBodyCompatLayer 的 1MiB 先兜底，
/// 该上限只对绕过安全扫描的 OTA 上传路由真正生效。
const MAX_FORWARD_BODY: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub struct ProxyState {
    pub client: reqwest::Client,
    /// 服务发现（B-3）：按服务名解析上游 base URL，env/默认兜底。
    pub resolver: ServiceResolver,
}

fn claims_of(req: &Request) -> Option<AuthClaims> {
    req.extensions().get::<AuthClaims>().cloned()
}

/// 受保护转发：JwtAuthCompat 已把 AuthClaims 放入 extensions（ecat-auth jwt.rs），
/// 取其 sub 作为租户注入 x-tenant-id、role 注入 x-tenant-role。
/// claims 缺失时租户为空，上游会以 401 拒绝（fail-closed）。
pub async fn access_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api/access",
        "ACCESS_BASE",
        "iot-access",
        ACCESS_BASE,
    )
    .await
}

/// 公开转发（OAuth 回调 / 涂鸦 webhook / 登录）：不注入租户。
pub async fn access_proxy_open(State(ps): State<ProxyState>, req: Request) -> Response {
    forward(
        &ps,
        req,
        None,
        "/api/access",
        "ACCESS_BASE",
        "iot-access",
        ACCESS_BASE,
    )
    .await
}

/// 登录公开代理：/api/v1/auth/login 与 /api/v1/admin/auth/login 分别转到
/// iot-access 的 /api/auth/login 或 /admin/auth/login。
pub async fn auth_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    // nest 只剥掉 /api/v1；handler 看到的剩余路径以 /admin 开头即管理端登录。
    let prefix = if req.uri().path().starts_with("/admin") {
        "/admin"
    } else {
        "/api"
    };
    forward(
        &ps,
        req,
        None,
        prefix,
        "ACCESS_BASE",
        "iot-access",
        ACCESS_BASE,
    )
    .await
}

/// /api/data/* 受保护转发（GET）：JWT sub → x-tenant-id + x-gateway-secret，
/// query 原样透传（history/export 由 iot-data 从 Query 读取）。
pub async fn data_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api",
        "DATA_BASE",
        "iot-data",
        DATA_BASE,
    )
    .await
}

/// /api/rule/* 受保护转发：JWT sub → x-tenant-id + x-gateway-secret，
/// 路径/query/body 原样透传（rules/alerts CRUD 由 iot-rule 处理）。
pub async fn rule_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    // 与 data_proxy 同构：nest 在 /api 下、路由为 /rule/...，handler 看到的
    // 路径是 /rule/rules 等剩余段，prefix "/api" 补回后即 /api/rule/rules
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api",
        "RULE_BASE",
        "iot-rule",
        RULE_BASE,
    )
    .await
}

/// /api/cdn/* 受保护转发：JWT sub → x-tenant-id + x-gateway-secret，
/// 供应商 CRUD/启停/刷新预热/签名 URL 由 iot-cdn 处理。
pub async fn cdn_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api",
        "CDN_BASE",
        "iot-cdn",
        CDN_BASE,
    )
    .await
}

/// /api/tenants、/api/users、/api/models/* 受保护转发 → iot-access。
/// iot-access 的控制台路由挂在 /api/access 下（main.rs nest），因此这里补的是
/// /api/access 而不是 /api；否则上游会 404（2026-08-31 起的历史不一致）。
pub async fn console_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api/access",
        "ACCESS_BASE",
        "iot-access",
        ACCESS_BASE,
    )
    .await
}

/// /api/devices/*、/api/ota/* 受保护转发 → iot-device。
pub async fn device_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api",
        "DEVICE_BASE",
        "iot-device",
        DEVICE_BASE,
    )
    .await
}

/// OTA 固件上传（管理端）：nest 在 /api/v1 下，handler 看到 /ota/firmwares/upload，
/// 补 /api 后转发到 device 的 /api/ota/firmwares/upload。
pub async fn ota_upload_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    let claims = claims_of(&req);
    forward(
        &ps,
        req,
        claims.as_ref(),
        "/api",
        "DEVICE_BASE",
        "iot-device",
        DEVICE_BASE,
    )
    .await
}

/// OTA 公开下载：token 由 device 验签，网关不做 JWT、不注入租户；
/// 固件最大 64MiB，响应必须流式回传（不能整体读入内存）。
pub async fn ota_public_proxy(State(ps): State<ProxyState>, req: Request) -> Response {
    forward_streaming(&ps, req, "/api", "DEVICE_BASE", "iot-device", DEVICE_BASE).await
}

/// 原样转发到上游服务：方法/路径/query/body 透传，注入 x-gateway-secret
/// （上游 tenant_from_header 门禁校验）+ 受保护路径的 x-tenant-id / x-tenant-role。
/// 上游 base 由服务发现（B-3）解析，env/默认兜底。上游失败 → 502。
async fn forward(
    ps: &ProxyState,
    req: Request,
    claims: Option<&AuthClaims>,
    path_prefix: &str,
    base_env: &str,
    service_name: &str,
    default_base: &str,
) -> Response {
    // axum nest 会剥掉挂载前缀，handler 里看到的路径是剩余部分，转发时补回；
    // query 也要透传（OAuth callback 的 ?code=&state= 由上游从 Query 读取）
    let mut path = format!("{path_prefix}{}", req.uri().path());
    if let Some(q) = req.uri().query() {
        path.push('?');
        path.push_str(q);
    }
    let method = req.method().clone();
    let content_type = req.headers().get(header::CONTENT_TYPE).cloned();
    // 只透传白名单头（x-tuya-signature 供涂鸦 webhook 验签用）；
    // 不复制全部请求头，避免客户端伪造 x-tenant-id / x-gateway-secret。
    let tuya_sig = req.headers().get("x-tuya-signature").cloned();
    let raw = match to_bytes(req.into_body(), MAX_FORWARD_BODY).await {
        Ok(b) => b,
        Err(_) => {
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                axum::Json(serde_json::json!({"error": "body too large"})),
            )
                .into_response();
        }
    };
    // 服务发现解析（env → Consul → 默认），见 ServiceResolver::resolve
    let base = ps
        .resolver
        .resolve(base_env, service_name)
        .await
        .unwrap_or_else(|| default_base.into());
    let mut rb = ps.client.request(method, format!("{base}{path}"));
    if let Some(ct) = content_type {
        rb = rb.header(header::CONTENT_TYPE, ct);
    }
    if let Some(sig) = tuya_sig {
        rb = rb.header("x-tuya-signature", sig);
    }
    if let Ok(secret) = std::env::var("IOT_GATEWAY_SECRET") {
        rb = rb.header("x-gateway-secret", secret);
    }
    if let Some(claims) = claims {
        rb = rb.header("x-tenant-id", &claims.sub);
        if let Some(role) = &claims.role {
            rb = rb.header("x-tenant-role", role);
        }
    }
    let resp = match rb.body(raw).send().await {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": format!("upstream: {e}")})),
            )
                .into_response();
        }
    };
    let status = resp.status();
    let content_type = resp.headers().get(header::CONTENT_TYPE).cloned();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": format!("upstream read: {e}")})),
            )
                .into_response();
        }
    };
    let mut rb = Response::builder().status(status);
    if let Some(ct) = content_type {
        rb = rb.header(header::CONTENT_TYPE, ct);
    }
    rb.body(Body::from(bytes.to_vec())).unwrap()
}

/// 流式转发（OTA 公开下载）：不缓冲响应体，逐块回传（固件最大 64MiB）。
/// 只透传内容相关响应头；请求仅注入 x-gateway-secret，不注入租户。
async fn forward_streaming(
    ps: &ProxyState,
    req: Request,
    path_prefix: &str,
    base_env: &str,
    service_name: &str,
    default_base: &str,
) -> Response {
    let mut path = format!("{path_prefix}{}", req.uri().path());
    if let Some(q) = req.uri().query() {
        path.push('?');
        path.push_str(q);
    }
    let method = req.method().clone();
    // 服务发现解析（env → Consul → 默认），见 ServiceResolver::resolve
    let base = ps
        .resolver
        .resolve(base_env, service_name)
        .await
        .unwrap_or_else(|| default_base.into());
    let mut rb = ps.client.request(method, format!("{base}{path}"));
    if let Ok(secret) = std::env::var("IOT_GATEWAY_SECRET") {
        rb = rb.header("x-gateway-secret", secret);
    }
    let resp = match rb.send().await {
        Ok(r) => r,
        Err(e) => return bad_gateway(format!("upstream: {e}")),
    };
    let status = resp.status();
    let mut builder = Response::builder().status(status);
    for name in [
        header::CONTENT_TYPE,
        header::CONTENT_LENGTH,
        header::CONTENT_DISPOSITION,
    ] {
        if let Some(v) = resp.headers().get(&name) {
            builder = builder.header(name, v.clone());
        }
    }
    builder
        .body(Body::from_stream(resp.bytes_stream()))
        .unwrap_or_else(|_| bad_gateway("upstream: invalid response".into()))
}

fn bad_gateway(msg: String) -> Response {
    (
        StatusCode::BAD_GATEWAY,
        axum::Json(serde_json::json!({"error": msg})),
    )
        .into_response()
}
