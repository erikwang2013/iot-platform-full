use crate::auth::password_matches;
use crate::events::{publish_event, shadow_apply, validate_event};
use crate::models::EventMessage;
use crate::store::Store;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use ecat_data_redis::RedisCache;
use ecat_iot::now_ms;
use ecat_mq::MessageQueue;
use ecat_mq_kafka::KafkaMq;
use ecat_mq_mqtt::MqttMq;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// 直连设备上报 topic 约定：iot/devices/{device_id}/properties。
pub fn report_topic(device_id: &str) -> String {
    format!("iot/devices/{device_id}/properties")
}

/// 指令下发 topic 约定：iot/devices/{device_id}/commands。
pub fn command_topic(device_id: &str) -> String {
    format!("iot/devices/{device_id}/commands")
}

/// 所有直连设备上报的单条通配订阅。
const REPORT_FILTER: &str = "iot/devices/+/properties";
/// 直连设备心跳通配订阅：只用于保活，不产生事件。
pub const HEARTBEAT_FILTER: &str = "iot/devices/+/heartbeat";
/// 设备 → 租户映射刷新间隔。
const TENANT_REFRESH: Duration = Duration::from_secs(30);

/// 严格解析设备 topic：仅 `iot/devices/{id}/{suffix}`，id 非空且不含 '/'。
/// 设备 id 从 broker 投递的实际 topic 取，payload 无法伪造归属。
fn parse_topic<'a>(topic: &'a str, suffix: &str) -> Option<&'a str> {
    let (device_id, actual) = topic.strip_prefix("iot/devices/")?.split_once('/')?;
    (actual == suffix && !device_id.is_empty()).then_some(device_id)
}

pub fn parse_report_topic(topic: &str) -> Option<&str> {
    parse_topic(topic, "properties")
}

fn parse_heartbeat_topic(topic: &str) -> Option<&str> {
    parse_topic(topic, "heartbeat")
}

/// 上报 payload：{"code","value","ts?"}；ts 缺省取当前毫秒。
/// 事件经 validate_event 校验：超长 code/value 直接丢弃，避免打挂时序写入。
pub fn parse_payload(
    platform_id: &str,
    tenant_id: &str,
    raw: &[u8],
) -> Result<EventMessage, String> {
    let v: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| format!("bad mqtt payload: {e}"))?;
    let ev = EventMessage {
        device_id: platform_id.to_string(),
        tenant_id: tenant_id.to_string(),
        kind: "property".into(),
        code: v["code"].as_str().unwrap_or("").to_string(),
        value: v["value"].clone(),
        ts: v["ts"].as_i64().unwrap_or_else(|| now_ms()),
    };
    validate_event(&ev)?;
    Ok(ev)
}

/// 指令下发（api.rs 直连分支调用）。
pub async fn publish_command(
    mqtt: &MqttMq,
    device_id: &str,
    code: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    let payload = json!({ "code": code, "value": value });
    mqtt.publish(
        &command_topic(device_id),
        serde_json::to_vec(&payload).unwrap().as_slice(),
    )
    .await
    .map_err(|e| format!("mqtt publish: {e}"))
}

/// 后台任务：一条连接通配订阅全部直连设备上报，设备 id 由 topic 解析后
/// 查内存映射得租户；映射每 30s 从 DB 刷新，未知设备丢弃。
/// 待发指令补发为边沿触发：设备由离线转在线的那条上报才查 MySQL 补发。
pub async fn run(
    mqtt: Arc<MqttMq>,
    store: Arc<Store>,
    redis: Arc<RedisCache>,
    kafka: Arc<KafkaMq>,
) {
    let tenants: Arc<RwLock<HashMap<String, String>>> = Arc::new(RwLock::new(HashMap::new()));
    loop {
        // 先刷新映射再订阅，避免启动窗口内丢弃事件
        refresh_tenants(&store, &tenants).await;
        let mut stream = match mqtt.subscribe_with_topic(REPORT_FILTER).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "mqtt topic subscribe failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        let mut tick = tokio::time::interval(TENANT_REFRESH);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await; // interval 首次立即完成；刚刷新过，跳过
        loop {
            tokio::select! {
                msg = stream.recv() => match msg {
                    Some((topic, raw)) => {
                        handle_report(&mqtt, &store, &redis, &kafka, &tenants, &topic, &raw).await;
                    }
                    None => {
                        tracing::warn!("mqtt topic stream ended; resubscribing");
                        break;
                    }
                },
                _ = tick.tick() => refresh_tenants(&store, &tenants).await,
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// 刷新 设备 id → 租户 映射。
async fn refresh_tenants(store: &Store, tenants: &RwLock<HashMap<String, String>>) {
    match store.list_direct_devices().await {
        Ok(devices) => {
            let mut map = tenants.write().unwrap_or_else(|e| e.into_inner());
            map.clear();
            map.extend(devices);
        }
        Err(e) => tracing::warn!(error = %e, "list direct devices failed"),
    }
}

/// 单条上报：topic → 设备 → 租户 → 事件/影子；离线转在线时补发排队指令。
async fn handle_report(
    mqtt: &Arc<MqttMq>,
    store: &Arc<Store>,
    redis: &Arc<RedisCache>,
    kafka: &Arc<KafkaMq>,
    tenants: &RwLock<HashMap<String, String>>,
    topic: &str,
    raw: &[u8],
) {
    let Some(device_id) = parse_report_topic(topic) else {
        tracing::warn!(topic, "drop mqtt message with unexpected topic");
        return;
    };
    // 锁内只做查表，guard 不跨 await
    let tenant_id = tenants
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(device_id)
        .cloned();
    let Some(tenant_id) = tenant_id else {
        tracing::debug!(device = %device_id, "drop mqtt message from unknown device");
        return;
    };
    let ev = match parse_payload(device_id, &tenant_id, raw) {
        Ok(ev) => ev,
        Err(e) => {
            tracing::warn!(device = %device_id, error = %e, "drop mqtt payload");
            return;
        }
    };
    // shadow_apply 会把设备置为在线：先读旧状态，判断是否离线→在线边沿
    let was_online = crate::command_queue::device_online(redis, device_id).await;
    if let Err(e) = publish_event(kafka, &ev).await {
        tracing::error!(error = %e, "kafka publish failed");
    }
    if let Err(e) = shadow_apply(redis, &ev).await {
        tracing::warn!(error = %e, "shadow update failed");
    }
    if !was_online {
        // 补发是侧路：QoS1 下 broker 慢时 publish 可能阻塞，不能拖住后续设备的
        // 上报消费；flush_for_device 内的进程内 guard 负责去重。
        let (store, redis, mqtt) = (store.clone(), redis.clone(), mqtt.clone());
        let device_id = device_id.to_string();
        tokio::spawn(async move {
            crate::command_queue::flush_for_device(store, redis, mqtt, &device_id).await;
        });
    }
}

/// 后台任务：通配订阅心跳 topic，只刷新影子 online/seen。
/// 属性上报间隔可能大于离线阈值，心跳是设备保活的唯一信号；
/// 心跳不产生事件、不写时序库。离线→在线边沿补发断连期间入队的指令。
pub async fn run_heartbeats(mqtt: Arc<MqttMq>, redis: Arc<RedisCache>, store: Arc<Store>) {
    loop {
        let mut stream = match mqtt.subscribe_with_topic(HEARTBEAT_FILTER).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "mqtt heartbeat subscribe failed");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };
        while let Some((topic, _payload)) = stream.recv().await {
            let Some(device_id) = parse_heartbeat_topic(&topic) else {
                tracing::warn!(topic, "drop mqtt heartbeat with unexpected topic");
                continue;
            };
            match crate::events::shadow_mark_online(&redis, device_id).await {
                // 心跳前已在线：只刷新鲜度，不查队列（避免每次心跳打库）
                Ok(true) => {}
                // 离线→在线边沿：补发断连期间入队的指令（侧路 spawn，不阻塞
                // 后续设备心跳的 seen 刷新；guard 防止与属性上报重复补发）
                Ok(false) => {
                    let (store, redis, mqtt) = (store.clone(), redis.clone(), mqtt.clone());
                    let device_id = device_id.to_string();
                    tokio::spawn(async move {
                        crate::command_queue::flush_for_device(store, redis, mqtt, &device_id)
                            .await;
                    });
                }
                Err(e) => {
                    tracing::warn!(device = %device_id, error = %e, "heartbeat shadow update failed");
                }
            }
        }
        tracing::warn!("mqtt heartbeat stream ended; resubscribing");
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

// ---- EMQX HTTP 认证 / ACL 回调（公开路由，无 JWT）----

/// ACL 缓存条目上限：超过即整体清空，防止随机用户名撑爆内存。
const ACL_CACHE_MAX: usize = 10_000;

#[derive(Clone)]
pub struct MqttAuthState {
    pub store: Arc<Store>,
    /// 设备密码 PBKDF2 并发上限（MQTT_AUTH_CONCURRENCY，默认 8）。
    auth_limit: Arc<tokio::sync::Semaphore>,
    /// username → (过期时刻, 设备会话是否有效)（MQTT_ACL_CACHE_SECS，0 关闭）。
    acl_cache: Arc<Mutex<HashMap<String, (Instant, bool)>>>,
    acl_cache_ttl: Duration,
}

impl MqttAuthState {
    pub fn new(store: Arc<Store>) -> Self {
        let concurrency = std::env::var("MQTT_AUTH_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(8)
            .max(1);
        let ttl_secs = std::env::var("MQTT_ACL_CACHE_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(2);
        Self {
            store,
            auth_limit: Arc::new(tokio::sync::Semaphore::new(concurrency)),
            acl_cache: Arc::new(Mutex::new(HashMap::new())),
            acl_cache_ttl: Duration::from_secs(ttl_secs),
        }
    }
}

type AclCache = Mutex<HashMap<String, (Instant, bool)>>;

fn cached_active(cache: &AclCache, username: &str) -> Option<bool> {
    let cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    let (expires, active) = cache.get(username)?;
    (*expires > Instant::now()).then_some(*active)
}

/// 写入并缓存 `ttl`；ttl 为 0 时关闭缓存。
fn cache_active(cache: &AclCache, username: &str, active: bool, ttl: Duration) {
    if ttl.is_zero() {
        return;
    }
    let Some(expires) = Instant::now().checked_add(ttl) else {
        return;
    };
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= ACL_CACHE_MAX {
        cache.clear();
    }
    cache.insert(username.to_string(), (expires, active));
}

/// 挂在 /api/access 公开 nest 下：/api/access/mqtt/auth、/api/access/mqtt/acl。
pub fn auth_router(state: MqttAuthState) -> Router {
    Router::new()
        .route("/mqtt/auth", post(mqtt_auth))
        .route("/mqtt/acl", post(mqtt_acl))
        .with_state(state)
}

#[derive(Deserialize)]
pub struct MqttAuthReq {
    pub clientid: String,
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct MqttAclReq {
    pub username: String,
    pub topic: String,
    pub action: String,
    #[allow(dead_code)] // EMQX 请求契约字段（判定不使用）
    pub clientid: String,
}

/// 回调门禁：x-emqx-auth 必须等于 env EMQX_AUTH_SECRET；未配置即拒绝。
fn emqx_secret_ok(headers: &HeaderMap) -> bool {
    let expected = std::env::var("EMQX_AUTH_SECRET").unwrap_or_default();
    !expected.is_empty()
        && headers
            .get("x-emqx-auth")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| ecat_middleware::secret_eq(v, &expected))
}

/// 认证判定：平台账号（env MQTT_USERNAME/PASSWORD）或设备凭据
/// （clientid 必须等于用户名，密码比对 PBKDF2/旧 HMAC 哈希）。
/// 设备哈希校验（PBKDF2，CPU 密集）放 spawn_blocking，避免阻塞 Tokio worker。
async fn auth_allowed(
    platform_username: Option<&str>,
    platform_password: Option<&str>,
    device_hash: Option<&str>,
    pepper: &str,
    req: &MqttAuthReq,
) -> bool {
    if let (Some(user), Some(pass)) = (platform_username, platform_password)
        && req.username.as_str() == user
        && ecat_middleware::secret_eq(&req.password, pass)
    {
        return true;
    }
    if req.clientid != req.username {
        return false;
    }
    let Some(hash) = device_hash else {
        return false;
    };
    let (hash, pepper, password) = (hash.to_owned(), pepper.to_owned(), req.password.clone());
    tokio::task::spawn_blocking(move || password_matches(&hash, &pepper, &password))
        .await
        .unwrap_or(false)
}

/// 读取环境变量并过滤空白值（`VAR=` 视为未配置）。
fn env_nonempty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// ACL 判定（纯函数）：平台账号可访问 iot/ 命名空间；设备只能访问自身
/// properties/heartbeat（发布）与 commands（订阅）；其他一律拒绝。
/// 设备是否仍有效由 mqtt_acl 查库（带缓存）判定，本函数只管 topic 归属。
fn acl_allowed(platform_username: Option<&str>, req: &MqttAclReq) -> bool {
    if platform_username.is_some_and(|u| u == req.username.as_str()) {
        return req.topic.starts_with("iot/");
    }
    if req.username.is_empty() {
        return false;
    }
    match req.action.as_str() {
        "publish" => {
            req.topic == format!("iot/devices/{}/properties", req.username)
                || req.topic == format!("iot/devices/{}/heartbeat", req.username)
        }
        "subscribe" => req.topic == format!("iot/devices/{}/commands", req.username),
        _ => false,
    }
}

async fn mqtt_auth(
    State(s): State<MqttAuthState>,
    headers: HeaderMap,
    Json(req): Json<MqttAuthReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !emqx_secret_ok(&headers) {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".into()));
    }
    let (platform_user, platform_pass) =
        (env_nonempty("MQTT_USERNAME"), env_nonempty("MQTT_PASSWORD"));
    // PBKDF2 是 CPU 密集操作：并发上限外的设备认证直接拒绝（平台账号不占额度）。
    // permit 绑定到函数作用域，认证结束才释放。
    let _permit = if platform_user.as_deref() != Some(req.username.as_str()) {
        match s.auth_limit.clone().try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => {
                tracing::warn!(user = %req.username, "mqtt auth concurrency limit reached; deny");
                return Ok(Json(json!({ "result": "deny" })));
            }
        }
    } else {
        None
    };
    let device_hash = s
        .store
        .mqtt_credential_hash(&req.username)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "mqtt auth credential lookup failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
        })?;
    let pepper =
        std::env::var("IOT_PASSWORD_PEPPER").unwrap_or_else(|_| "iot-password-pepper-v1".into());
    let allowed = auth_allowed(
        platform_user.as_deref(),
        platform_pass.as_deref(),
        device_hash.as_deref(),
        &pepper,
        &req,
    )
    .await;
    Ok(Json(
        json!({ "result": if allowed { "allow" } else { "deny" } }),
    ))
}

async fn mqtt_acl(
    State(s): State<MqttAuthState>,
    headers: HeaderMap,
    Json(req): Json<MqttAclReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !emqx_secret_ok(&headers) {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".into()));
    }
    let platform_user = env_nonempty("MQTT_USERNAME");
    let allowed = if platform_user.as_deref() == Some(req.username.as_str()) {
        acl_allowed(platform_user.as_deref(), &req)
    } else if req.username.is_empty() {
        false
    } else {
        // 已有会话的 ACL 复查：设备被 disabled/unbound/删除后，认证不会重跑，
        // 只能靠这里拒绝。缓存 TTL 界定状态变更生效延迟。
        let active = match cached_active(&s.acl_cache, &req.username) {
            Some(active) => active,
            None => match s.store.mqtt_device_active(&req.username).await {
                Ok(active) => {
                    cache_active(&s.acl_cache, &req.username, active, s.acl_cache_ttl);
                    active
                }
                Err(e) => {
                    tracing::warn!(error = %e, user = %req.username, "mqtt acl device lookup failed; deny");
                    // 故障态短缓存，避免每消息打库
                    let ttl = s.acl_cache_ttl.min(Duration::from_secs(1));
                    cache_active(&s.acl_cache, &req.username, false, ttl);
                    false
                }
            },
        };
        active && acl_allowed(platform_user.as_deref(), &req)
    };
    Ok(Json(
        json!({ "result": if allowed { "allow" } else { "deny" } }),
    ))
}

#[cfg(test)]
#[path = "mqtt_tests.rs"]
mod tests;
