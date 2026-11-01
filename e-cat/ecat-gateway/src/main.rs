use axum::{
    Router,
    response::IntoResponse,
    routing::{delete, get, post, put},
};
use ecat_auth::JwtAuthCompat;
use ecat_gateway::{
    audit::MysqlAuditSink,
    proxy::{
        ProxyState, access_proxy, access_proxy_open, auth_proxy, cdn_proxy, console_proxy,
        data_proxy, device_proxy, ota_public_proxy, ota_upload_proxy, rule_proxy,
    },
};
use ecat_health::HealthRegistry;
use ecat_metrics::MetricsLayer;
use ecat_middleware::{
    AuditState, MemoryStore, RateLimitLayer, RateLimitStore, RedisRateLimitStore, audit_middleware,
};
use ecat_security::SecurityBodyCompatLayer;
use std::sync::Arc;
use std::time::Duration;

/// GET /api/v1/openapi.json：返回只读端点的 OpenAPI 3.0 文档。
async fn openapi_doc() -> axum::Json<serde_json::Value> {
    let spec = ecat_gateway::openapi::read_only_spec();
    axum::Json(serde_json::to_value(&spec).unwrap_or_else(|_| serde_json::json!({})))
}

/// 值级 RBAC 角色表（与登录签发的 role claim 对齐）：
/// 读操作（GET/HEAD/OPTIONS）三角色均可；写操作 admin/operator；
/// 租户/用户/固件管理仅 admin。
const ROLES_ALL: &[&str] = &["admin", "operator", "read-only"];
const ROLES_WRITE: &[&str] = &["admin", "operator"];
const ROLES_ADMIN: &[&str] = &["admin"];

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let secret = std::env::var("JWT_SECRET").map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "JWT_SECRET not set; refusing to start with a public default",
        )
    })?;
    // 转发上游时必须携带网关共享密钥；空值会让上游 gate 拒绝全部请求，
    // 启动即失败优于运行期 503。
    ecat::require_nonempty_env("IOT_GATEWAY_SECRET")?;
    // 限流租户提示中间件与 JWT 层共用同一 secret / 同一验签逻辑
    let hint_secret: Arc<str> = Arc::from(secret.as_str());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());

    let proxy_state = ProxyState {
        // 连接 5s 超时；read 60s 是"无数据"超时而非总超时——大固件上传/下载
        // 只要持续有字节流动就不会被掐断，上游卡死 60s 才失败。
        client: reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(Duration::from_secs(60))
            .build()?,
        resolver: ecat_gateway::service::ServiceResolver::new(),
    };

    // 全局限流与登录限流共享同一存储（Redis 优先，多实例计数一致；
    // 不可用时降级内存 fail-open，可用性优先，降级有日志告警）
    let rate_store = rate_limit_store(&redis_url).await;

    // 审计 sink：管理面写操作（POST/PUT/PATCH/DELETE）落库 audit_log。
    // MySQL 不可用时降级空实现（日志告警）——审计属可观测性增强，不阻塞启动。
    let audit_dsn = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into());
    // 检索 client：SEARCH_KIND=opensearch|elasticsearch + URL env 未设 → None（禁用，
    // 审计日志索引跳过、检索 API 503），不破坏无 OpenSearch 的本地环境。
    let search_client = ecat_search::connect_search();
    let audit_sink = MysqlAuditSink::connect(&audit_dsn)
        .await
        .with_search(search_client.clone());
    // 审计保留期清理（AUDIT_RETENTION_DAYS，默认 90 天）：启动 60s 后首次，
    // 此后每 6h；DB 不可用时方法直接跳过，失败仅 warn（不阻塞业务）。
    spawn_audit_cleanup(audit_sink.clone());
    let audit_state = AuditState(Arc::new(audit_sink));

    // /api/access/* 公开路径：OAuth 回调、开放 API 换 token
    // （无 JWT，浏览器/开放客户端直连）。Webhook 单独挂载：厂商事件量远高于
    // 普通匿名流量，不能和登录/探活共享 100 次/分钟桶。
    let access_public = Router::new()
        .route("/oauth/callback", get(access_proxy_open))
        .route("/open/token", post(access_proxy_open))
        .with_state(proxy_state.clone());
    let webhook_public = Router::new()
        .route("/webhook/tuya", post(access_proxy_open))
        // 独立限流不能牺牲原先的 1MiB 体上限/严格扫描；签名校验仍在 access 内。
        .layer(
            SecurityBodyCompatLayer::new()
                .strict()
                .body_limit(1024 * 1024),
        )
        .with_state(proxy_state.clone());
    // /api/access/* 受保护路径：JWT 校验后透传租户（AuthClaims.sub → x-tenant-id）
    let access_admin = Router::new()
        .route("/oauth/authorize-url", post(access_proxy))
        .route("/vendors/{vendor}/import", post(access_proxy))
        .route("/devices/{device_id}/command", post(access_proxy))
        // 全写操作：设备命令/厂商导入属于"设备操作"，operator 可做
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_WRITE))
        .with_state(proxy_state.clone());
    // 厂商凭据（AK/SK）写操作仅 admin：凭据可换取厂商全量设备访问权，
    // 权限高于设备命令/导入，故单独分组而不放宽 access_admin 的 operator 写策略。
    let access_creds = Router::new()
        .route("/vendors/{vendor}/creds", put(access_proxy))
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_ADMIN))
        .with_state(proxy_state.clone());
    // /api/data/* 受保护路径：历史曲线 / 导出（GET，query 透传）
    let data_admin = Router::new()
        .route("/data/history", get(data_proxy))
        .route("/data/export", get(data_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, &[]))
        .with_state(proxy_state.clone());
    // /api/rule/* 受保护路径：规则 CRUD / 告警记录（REST，WS 直连 8084 不走网关）
    let rule_admin = Router::new()
        .route("/rule/rules", get(rule_proxy).post(rule_proxy))
        .route("/rule/rules/{id}", put(rule_proxy).delete(rule_proxy))
        .route("/rule/alerts", get(rule_proxy))
        .route("/rule/alerts/{id}/ack", post(rule_proxy))
        .route("/rule/stats", get(rule_proxy))
        .route("/rule/reports", get(rule_proxy))
        .route("/rule/channels", get(rule_proxy))
        .route(
            "/rule/channels/{channel}",
            put(rule_proxy).delete(rule_proxy),
        )
        .route("/rule/scenes", get(rule_proxy).post(rule_proxy))
        .route("/rule/scenes/{id}", put(rule_proxy).delete(rule_proxy))
        .route("/rule/scenes/{id}/execute", post(rule_proxy))
        .route("/rule/notifications", get(rule_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_WRITE))
        .with_state(proxy_state.clone());
    // 标记通知已读：reviewer 要求 read-only 也可操作（写策略 ROLES_ALL），
    // 与其余 rule 写路由的 operator 策略不同，故单独分组。
    let rule_notify = Router::new()
        .route("/rule/notifications/{id}/read", post(rule_proxy))
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_ALL))
        .with_state(proxy_state.clone());
    // /api/search/* 受保护路径：跨 devices/alerts/logs 检索（GET 只读，
    // 直接持有 SearchClient 查询 OpenSearch，不转发上游）
    let search_admin = Router::new()
        .route("/search", get(ecat_gateway::search::search))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, &[]))
        .with_state(ecat_gateway::search::SearchState(
            ecat_search::connect_search(),
        ));
    // /api/cdn/* 受保护路径：供应商 CRUD / 启停 / 刷新预热 / 签名 URL
    let cdn_admin = Router::new()
        .route("/cdn/providers", get(cdn_proxy).post(cdn_proxy))
        .route(
            "/cdn/providers/{id}",
            get(cdn_proxy).put(cdn_proxy).delete(cdn_proxy),
        )
        .route("/cdn/providers/{id}/enable", post(cdn_proxy))
        .route("/cdn/providers/{id}/disable", post(cdn_proxy))
        .route("/cdn/providers/{id}/test", post(cdn_proxy))
        .route("/cdn/providers/{id}/signed-url", post(cdn_proxy))
        .route("/cdn/providers/{id}/purge", post(cdn_proxy))
        .route("/cdn/providers/{id}/prefetch", post(cdn_proxy))
        .route("/cdn/tasks", get(cdn_proxy))
        .route("/cdn/stats", get(cdn_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_WRITE))
        .with_state(proxy_state.clone());

    // 管理面（/api/*）：租户/用户/物模型 → iot-access；设备/OTA → iot-device。
    // 双面都要求 sub+role：客户端登录 token 也带 role（P5 401 根因），
    // 客户端设备列表走 /api/devices 同样受此校验。
    // 租户/用户管理仅 admin（读三角色均可）；物模型属设备能力定义，operator 可写
    let console_admin = Router::new()
        .route("/tenants", get(console_proxy).post(console_proxy))
        .route("/tenants/{id}", delete(console_proxy))
        .route("/users", get(console_proxy).post(console_proxy))
        .route("/users/{id}", delete(console_proxy))
        // MQTT 凭据签发：静态后缀比 device_admin 的 /devices/{id} 优先匹配，
        // 不与其冲突；沿用本组 admin-only 策略与审计（凭据等同设备接入权）。
        .route("/devices/{id}/mqtt-credential", post(console_proxy))
        .route("/audit", get(console_proxy))
        .route("/api-keys", get(console_proxy).post(console_proxy))
        .route("/api-keys/{id}", delete(console_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_ADMIN))
        .with_state(proxy_state.clone());
    let models_admin = Router::new()
        .route("/models/things", get(console_proxy).post(console_proxy))
        .route(
            "/models/things/{id}",
            get(console_proxy).delete(console_proxy),
        )
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_WRITE))
        .with_state(proxy_state.clone());
    // 设备生命周期 operator 可写；OTA 固件管理仅 admin
    let device_admin = Router::new()
        .route("/devices", get(device_proxy))
        .route("/devices/stats", get(device_proxy))
        .route("/devices/groups", get(device_proxy).post(device_proxy))
        .route("/devices/groups/{id}", delete(device_proxy))
        .route(
            "/devices/groups/{id}/members",
            post(device_proxy).delete(device_proxy),
        )
        .route("/devices/batch", post(device_proxy))
        .route("/devices/{id}", put(device_proxy).delete(device_proxy))
        .route("/devices/{id}/unbind", post(device_proxy))
        .route("/devices/{id}/tags", get(device_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_WRITE))
        .with_state(proxy_state.clone());
    let ota_admin = Router::new()
        .route("/ota/firmwares", get(device_proxy).post(device_proxy))
        .route("/ota/firmwares/{id}", delete(device_proxy))
        .route("/ota/firmwares/{id}/signed-url", post(device_proxy))
        .route("/ota/firmwares/{id}/downloads", get(device_proxy))
        .route("/ota/tasks", get(device_proxy).post(device_proxy))
        .route("/ota/tasks/batch", post(device_proxy))
        .route("/ota/tasks/{id}/rollback", post(device_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_ADMIN))
        .with_state(proxy_state.clone());
    // OTA 进度上报：设备侧 operator token 需可写（升级进度由设备上报），
    // 故从 admin-only 的 ota_admin 拆出；rollback 等其余 OTA 操作仍仅 admin。
    let ota_report = Router::new()
        .route("/ota/tasks/{id}/report", post(device_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_WRITE))
        .with_state(proxy_state.clone());
    // OTA 固件上传（管理端）：独立 Router 绕开 SecurityBody 的 1MiB 体扫描
    // （64MiB 固件会被直接拒绝）；仍受全局限流/指标覆盖（merge 后统一挂载）。
    let ota_upload = Router::new()
        .route("/ota/firmwares/upload", post(ota_upload_proxy))
        // 审计层在 JWT 内层：JWT 先注入 AuthClaims，写操作据此记租户/角色；
        // 401/403 短路（不产生业务事件），与"仅审计成功会话"语义一致。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .layer(JwtAuthCompat::new(&secret, &["sub", "role"])?.role_policy(ROLES_ALL, ROLES_ADMIN))
        // forward 的 64MiB 上限之外再声明提取器上限，避免默认 2MiB 影响后续改造
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(proxy_state.clone());
    // OTA 公开下载：设备凭 device 现签的 token 下载，无 JWT/租户；
    // 与上传一样在 security 之后 nest，不经请求体扫描。
    let ota_public = Router::new()
        .route("/ota/public/firmwares/{id}/download", get(ota_public_proxy))
        .with_state(proxy_state.clone());
    // 登录端点：管理端 /api/v1/auth/login 与客户端 /api/v1/admin/auth/login 同一
    // handler，独立更严的按 IP 限流（10 次/分钟）防爆破；全局 100 次/分钟仍叠加。
    // 与全局限流共享 Redis 存储：多实例下登录失败计数一致（防爆破不因扩容失效）
    let login_router = Router::new()
        .route("/auth/login", post(auth_proxy))
        .route("/admin/auth/login", post(auth_proxy))
        .layer(
            tower::ServiceBuilder::new()
                .layer(axum::error_handling::HandleErrorLayer::new(
                    rate_limit_error,
                ))
                .layer(login_rate_limit(rate_store.clone())),
        )
        // 登录没有 AuthClaims，审计记 anonymous；仍记录路径/状态用于爆破排查。
        .layer(axum::middleware::from_fn_with_state(
            audit_state.clone(),
            audit_middleware,
        ))
        .with_state(proxy_state);

    // 公共 REST 统一挂在 /api/v1，另套安全扫描（1MiB 请求体上限）。
    // OTA 上传路由不在此 Router 上：它在下面 security 层之后 merge，
    // 不经 SecurityBody 扫描，否则 64MiB 固件会被 1MiB 限制直接拒绝。
    let protected = Router::new()
        .route("/api/v1/ping", get(|| async { "pong" }))
        // OpenAPI 3.0 文档（A-4）：只读端点描述，公开只读，无鉴权（纯文档）
        .route("/api/v1/openapi.json", get(openapi_doc))
        .nest("/api/v1/access", access_public)
        .nest("/api/v1/access", access_admin)
        .nest("/api/v1/access", access_creds)
        .nest("/api/v1", data_admin)
        .nest("/api/v1", rule_admin)
        .nest("/api/v1", rule_notify)
        .nest("/api/v1", search_admin)
        .nest("/api/v1", cdn_admin)
        .nest("/api/v1", console_admin)
        .nest("/api/v1", models_admin)
        .nest("/api/v1", device_admin)
        .nest("/api/v1", ota_admin)
        .nest("/api/v1", ota_report)
        .nest("/api/v1", login_router)
        // 严格模式 + 1MB 体上限：与原 scan.rs 语义一致（任意 severity 即 403）
        .layer(
            SecurityBodyCompatLayer::new()
                .strict()
                .body_limit(1024 * 1024),
        );

    let router = protected
        // OTA 上传/公开下载在 security 之后 merge：上传不受 1MiB 扫描限制，
        // 公开下载也不经请求体扫描；两者仍被下面的全局限流/指标层覆盖。
        .merge(Router::new().nest("/api/v1", ota_upload))
        .merge(Router::new().nest("/api/v1", ota_public))
        // 限流：默认 100 次/分钟（RATE_LIMIT_MAX / RATE_LIMIT_WINDOW_SECS 可配），
        // Redis 不可用降级内存存储（fail-open，与 store 语义一致）；
        // HandleErrorLayer 包裹限流层，把超限错误映射为 429
        .layer(
            // ServiceBuilder：先加的层在外层，故 HandleError 包住限流层
            tower::ServiceBuilder::new()
                .layer(axum::error_handling::HandleErrorLayer::new(
                    rate_limit_error,
                ))
                .layer(rate_limit(rate_store.clone()))
                .into_inner(),
        )
        // 租户提示必须在限流外层（后加的 layer 先执行）：限流 key 依赖其验签后的
        // 租户，而不是可伪造的 x-tenant-id 请求头。
        .layer(axum::middleware::from_fn_with_state(
            hint_secret,
            tenant_hint,
        ))
        // 最外层：403/429 也计入错误率（MetricsLayer 在 SecurityBody/限流之外）
        .layer(MetricsLayer::new())
        // 最最外层：错误消息 i18n（C-线）——Accept-Language 以 zh 开头时，
        // 把公共 API 常见错误（401/403/404/422/429）响应体翻成中文；
        // 未命中映射保持原文，内部日志不受影响。
        .layer(axum::middleware::from_fn(localize_error_body))
        // 最外层：来源 IP 注入 extensions，供内层审计层（登录/写操作）记录。
        // 后加的 layer 先执行，必须挂在审计层之外。
        .layer(axum::middleware::from_fn(inject_client_ip));

    // 基础设施端点和云厂商 webhook 不参与公共 API 限流：
    // - /health /ready /metrics 需要稳定探活，不能被匿名流量挤占；
    // - Tuya webhook 是机器事件流，单独用更高的每 IP 限额。
    let router = router
        .merge(HealthRegistry::new().into_router())
        .merge(ecat_metrics::metrics_router())
        .merge(
            Router::new().nest("/api/v1/access", webhook_public).layer(
                tower::ServiceBuilder::new()
                    .layer(axum::error_handling::HandleErrorLayer::new(
                        rate_limit_error,
                    ))
                    .layer(webhook_rate_limit(rate_store)),
            ),
        );

    let srv = {
        let mut srv = ecat_transport_http::HttpServer::new("0.0.0.0:8080").router(router);
        // B-5 TLS：配置 TLS_CERT + TLS_KEY（PEM）时启用 HTTPS（ingress 前的本地终结也可用）
        if let (Ok(cert), Ok(key)) = (std::env::var("TLS_CERT"), std::env::var("TLS_KEY")) {
            let tls = ecat_transport::TlsConfig::new(cert, key);
            srv = srv.tls(tls);
            tracing::info!("gateway TLS enabled");
        }
        srv
    };
    let mut app = ecat::App::builder()
        .name("iot-gateway")
        .version("0.1.0")
        .server(srv)
        .build()?;
    app.run().await?;
    Ok(())
}

/// Redis 限流存储，不可用时降级内存（fail-open，与 store 语义一致）。
/// 全局与登录限流共享同一 Arc<dyn RateLimitStore>，多实例计数一致。
async fn rate_limit_store(redis_url: &str) -> Arc<dyn RateLimitStore> {
    match RedisRateLimitStore::connect(redis_url).await {
        Ok(s) => {
            tracing::info!("rate limit store: redis");
            Arc::new(s)
        }
        Err(e) => {
            tracing::warn!(error = %e, "redis rate-limit store unavailable; fallback to in-memory");
            Arc::new(MemoryStore::new())
        }
    }
}

fn rate_limit(store: Arc<dyn RateLimitStore>) -> RateLimitLayer<axum::body::Body> {
    let max = env_u32("RATE_LIMIT_MAX", 100);
    let window_secs = env_u32("RATE_LIMIT_WINDOW_SECS", 60);
    RateLimitLayer::new(max, Duration::from_secs(window_secs as u64))
        .with_store(store)
        .with_key_fn(rate_limit_key)
}

/// 是否信任代理写入的 X-Real-IP。默认关闭：直连网关时该头可伪造。
/// 仅在 nginx/ingress 后且网关不直接暴露时设 TRUST_PROXY_HEADERS=1。
fn trust_proxy_headers() -> bool {
    static TRUST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRUST.get_or_init(|| {
        std::env::var("TRUST_PROXY_HEADERS")
            .map(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
            .unwrap_or(false)
    })
}

/// 客户端 IP：可信代理后的 X-Real-IP 优先，否则用 axum::serve 填充的
/// ConnectInfo（HttpServer 已启用 into_make_service_with_connect_info）。
fn client_ip(req: &axum::http::Request<axum::body::Body>) -> Option<std::net::IpAddr> {
    if trust_proxy_headers()
        && let Some(ip) = req
            .headers()
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<std::net::IpAddr>().ok())
    {
        return Some(ip);
    }
    if let Some(axum::extract::ConnectInfo(peer)) = req
        .extensions()
        .get::<axum::extract::ConnectInfo<ecat_transport_http::PeerAddr>>()
    {
        return Some(peer.0.ip());
    }
    req.extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|axum::extract::ConnectInfo(addr)| addr.ip())
}

/// 把可信来源 IP 写入请求扩展：审计层只读扩展，IP 解析/代理信任策略
/// 只在 client_ip 一处。必须挂在审计层外层（后加的 layer 先执行）。
async fn inject_client_ip(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some(ip) = client_ip(&req) {
        req.extensions_mut()
            .insert(ecat_middleware::ClientIp(ip.to_string()));
    }
    next.run(req).await
}

/// 限流 key：优先验签后的租户（tenant_hint 注入），否则按来源 IP。
/// 绝不读 x-tenant-id 请求头——该头客户端可伪造，会把他人租户额度耗尽。
fn rate_limit_key(req: &axum::http::Request<axum::body::Body>) -> String {
    if let Some(TenantHint(tenant)) = req.extensions().get::<TenantHint>() {
        return format!("tenant:{tenant}");
    }
    client_ip(req)
        .map(|ip| format!("ip:{ip}"))
        .unwrap_or_else(|| "global".into())
}

/// 验签后的租户提示（仅用于限流分桶，不参与鉴权）。
#[derive(Clone)]
struct TenantHint(String);

/// 外层租户提示中间件：从 Bearer token 解出并验签租户，写入 extensions 供限流
/// key 使用。必须挂在限流层外层（`.layer(rate_limit)` 之后再加该层）。
/// 验签失败不拦截、不注入——401/403 由内层 JwtAuthCompat 决定。
async fn tenant_hint(
    axum::extract::State(secret): axum::extract::State<Arc<str>>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if let Some(token) = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        && let Ok(claims) = ecat_auth::verify_token(token, &secret)
    {
        req.extensions_mut().insert(TenantHint(claims.sub));
    }
    next.run(req).await
}

/// 审计日志保留期清理：启动 60s 后首次执行，此后每 6h 一次。
/// AUDIT_RETENTION_DAYS 默认 90；DB 不可用时 cleanup 直接跳过，失败仅 warn。
fn spawn_audit_cleanup(sink: MysqlAuditSink) {
    let days = env_u32("AUDIT_RETENTION_DAYS", 90);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            match sink.cleanup_older_than(days).await {
                Ok(n) if n > 0 => tracing::info!(rows = n, days, "audit log cleanup done"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, days, "audit log cleanup failed"),
            }
            tokio::time::sleep(Duration::from_secs(6 * 60 * 60)).await;
        }
    });
}

/// 登录防爆破：按来源 IP 10 次/分钟（LOGIN_RATE_LIMIT_MAX 可配）。
/// 与全局限流共享 Redis 存储（多实例登录失败计数一致，防爆破不因扩容失效）；
/// Redis 不可用时降级内存 fail-open——登录可用性优先，爆破防护降级由日志告警。
fn login_rate_limit(store: Arc<dyn RateLimitStore>) -> RateLimitLayer<axum::body::Body> {
    let max = env_u32("LOGIN_RATE_LIMIT_MAX", 10);
    let window_secs = env_u32("LOGIN_RATE_LIMIT_WINDOW_SECS", 60);
    RateLimitLayer::new(max, Duration::from_secs(window_secs as u64))
        .with_store(store)
        .with_key_fn(|req: &axum::http::Request<axum::body::Body>| {
            client_ip(req)
                .map(|ip| format!("login:{ip}"))
                .unwrap_or_else(|| "login:global".into())
        })
}

/// 云厂商 webhook 的独立限额：事件量远高于交互请求，默认 6000 次/分钟/IP；
/// 仅按来源 IP 分桶（webhook 没有 JWT），签名校验仍是主要信任边界。
fn webhook_rate_limit(store: Arc<dyn RateLimitStore>) -> RateLimitLayer<axum::body::Body> {
    let max = env_u32("WEBHOOK_RATE_LIMIT_MAX", 6000);
    let window_secs = env_u32("WEBHOOK_RATE_LIMIT_WINDOW_SECS", 60);
    RateLimitLayer::new(max, Duration::from_secs(window_secs as u64))
        .with_store(store)
        .with_key_fn(|req: &axum::http::Request<axum::body::Body>| {
            client_ip(req)
                .map(|ip| format!("webhook:{ip}"))
                .unwrap_or_else(|| "webhook:global".into())
        })
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// 限流超限 → 429；其余服务错误 → 500（详情只进日志）。
async fn rate_limit_error(e: Box<dyn std::error::Error + Send + Sync>) -> axum::response::Response {
    if e.is::<ecat_middleware::RateLimitError>() {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        )
            .into_response();
    }
    tracing::warn!(error = %e, "rate limit service error");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "internal error",
    )
        .into_response()
}

/// 错误消息本地化中间件（C-线 i18n）：Accept-Language 以 zh 开头且响应为
/// 公共 API 常见错误码（401/403/404/422/429）时，把错误体翻成中文。
/// 兼容两种错误体：`{"error": "..."}` JSON（JWT 层）与纯文本（handler 元组、
/// 限流层）。映射只做精确匹配，未命中保持原文。
async fn localize_error_body(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let locale = ecat_errors::locale_from_accept_language(
        request
            .headers()
            .get("accept-language")
            .and_then(|v| v.to_str().ok()),
    );
    let mut response = next.run(request).await;
    let status = response.status().as_u16();
    if locale.is_none() || ![401, 403, 404, 422, 429].contains(&status) {
        return response;
    }
    // 错误体都很小；超过 16KB 直接跳过（不可能是常见错误消息）
    let bytes = match axum::body::to_bytes(std::mem::take(response.body_mut()), 16 * 1024).await {
        Ok(b) => b,
        Err(_) => return response,
    };
    let is_json = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("application/json"));
    let body: axum::body::Bytes = if is_json {
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(mut v) => {
                if let Some(msg) = v.get("error").and_then(serde_json::Value::as_str) {
                    v["error"] = serde_json::json!(ecat_errors::localize(locale, msg));
                }
                serde_json::to_vec(&v)
                    .unwrap_or_else(|_| bytes.to_vec())
                    .into()
            }
            Err(_) => bytes,
        }
    } else if let Ok(text) = std::str::from_utf8(&bytes) {
        ecat_errors::localize(locale, text).into_bytes().into()
    } else {
        bytes
    };
    *response.body_mut() = axum::body::Body::from(body);
    response.headers_mut().remove("content-length");
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const TEST_SECRET: &str = "tenant-hint-secret-0123456789abcdefghijklmn";

    fn make_token(sub: &str) -> String {
        let claims = serde_json::json!({"sub": sub, "role": "admin", "exp": 4_102_444_800u64});
        jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .unwrap()
    }

    fn hinted_app() -> Router {
        Router::new()
            .route(
                "/whoami",
                get(|req: axum::extract::Request| async move {
                    match req.extensions().get::<TenantHint>() {
                        Some(TenantHint(t)) => format!("tenant:{t}"),
                        None => "none".to_string(),
                    }
                }),
            )
            .layer(axum::middleware::from_fn_with_state(
                Arc::<str>::from(TEST_SECRET),
                tenant_hint,
            ))
    }

    async fn whoami(app: Router, headers: &[(&str, &str)]) -> (StatusCode, String) {
        let mut req = Request::builder().uri("/whoami");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn tenant_hint_injects_verified_tenant() {
        let token = format!("Bearer {}", make_token("t1"));
        let (status, body) = whoami(hinted_app(), &[("authorization", token.as_str())]).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "tenant:t1");
    }

    #[tokio::test]
    async fn tenant_hint_ignores_invalid_token_and_tenant_header() {
        // 坏 token + 伪造 x-tenant-id：不拦截、不注入，交给内层 JWT 层处理
        let (status, body) = whoami(
            hinted_app(),
            &[
                ("authorization", "Bearer not-a-jwt"),
                ("x-tenant-id", "victim"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "none");
    }

    #[test]
    fn rate_limit_key_ignores_spoofed_tenant_header() {
        let mut req = Request::builder()
            .header("x-tenant-id", "victim")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [10, 0, 0, 7],
                1234,
            ))));
        assert_eq!(rate_limit_key(&req), "ip:10.0.0.7");

        // 验签后的 tenant hint 优先于 IP（伪造头始终被忽略）
        req.extensions_mut()
            .insert(TenantHint("real-tenant".into()));
        assert_eq!(rate_limit_key(&req), "tenant:real-tenant");
    }

    #[tokio::test]
    async fn inject_client_ip_sets_extension_from_peer_addr() {
        let app = Router::new()
            .route(
                "/ip",
                get(|req: axum::extract::Request| async move {
                    req.extensions()
                        .get::<ecat_middleware::ClientIp>()
                        .map(|c| c.0.clone())
                        .unwrap_or_else(|| "none".into())
                }),
            )
            .layer(axum::middleware::from_fn(inject_client_ip));
        let mut req = Request::builder().uri("/ip").body(Body::empty()).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(ecat_transport_http::PeerAddr(
                std::net::SocketAddr::from(([10, 0, 0, 9], 1234)),
            )));
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&*body, b"10.0.0.9");
    }
}
