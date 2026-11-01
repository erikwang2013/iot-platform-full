use crate::auth::password_matches;
use crate::events::{publish_event, shadow_apply};
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
use std::sync::{Arc, RwLock};
use std::time::Duration;

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
/// 设备 → 租户映射刷新间隔。
const TENANT_REFRESH: Duration = Duration::from_secs(30);

/// 严格解析上报 topic：仅 `iot/devices/{id}/properties`，id 非空且不含 '/'。
/// 设备 id 从 broker 投递的实际 topic 取，payload 无法伪造归属。
pub fn parse_report_topic(topic: &str) -> Option<&str> {
    let (device_id, suffix) = topic.strip_prefix("iot/devices/")?.split_once('/')?;
    (suffix == "properties" && !device_id.is_empty()).then_some(device_id)
}

/// 上报 payload：{"code","value","ts?"}；ts 缺省取当前毫秒。
pub fn parse_payload(
    platform_id: &str,
    tenant_id: &str,
    raw: &[u8],
) -> Result<EventMessage, String> {
    let v: serde_json::Value =
        serde_json::from_slice(raw).map_err(|e| format!("bad mqtt payload: {e}"))?;
    Ok(EventMessage {
        device_id: platform_id.to_string(),
        tenant_id: tenant_id.to_string(),
        kind: "property".into(),
        code: v["code"].as_str().unwrap_or("").to_string(),
        value: v["value"].clone(),
        ts: v["ts"].as_i64().unwrap_or_else(|| now_ms()),
    })
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
        crate::command_queue::flush_for_device(
            store.clone(),
            redis.clone(),
            mqtt.clone(),
            device_id,
        )
        .await;
    }
}

// ---- EMQX HTTP 认证 / ACL 回调（公开路由，无 JWT）----

#[derive(Clone)]
pub struct MqttAuthState {
    pub store: Arc<Store>,
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
/// 设备禁用/删除在认证阶段拦截（凭据随租户删除），此处不再查库（每消息热路径）。
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
        env_nonempty("MQTT_USERNAME").as_deref(),
        env_nonempty("MQTT_PASSWORD").as_deref(),
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
    State(_s): State<MqttAuthState>,
    headers: HeaderMap,
    Json(req): Json<MqttAclReq>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if !emqx_secret_ok(&headers) {
        return Err((StatusCode::UNAUTHORIZED, "unauthorized".into()));
    }
    let allowed = acl_allowed(env_nonempty("MQTT_USERNAME").as_deref(), &req);
    Ok(Json(
        json!({ "result": if allowed { "allow" } else { "deny" } }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth_req(username: &str, clientid: &str, password: &str) -> MqttAuthReq {
        MqttAuthReq {
            clientid: clientid.into(),
            username: username.into(),
            password: password.into(),
        }
    }

    fn acl_req(username: &str, topic: &str, action: &str) -> MqttAclReq {
        MqttAclReq {
            username: username.into(),
            topic: topic.into(),
            action: action.into(),
            clientid: username.into(),
        }
    }

    #[test]
    fn report_topic_parses_strictly() {
        assert_eq!(parse_report_topic("iot/devices/42/properties"), Some("42"));
        assert_eq!(parse_report_topic("iot/devices/1/2/properties"), None);
        assert_eq!(parse_report_topic("iot/devices//properties"), None);
        assert_eq!(parse_report_topic("iot/devices/42/heartbeat"), None);
        assert_eq!(parse_report_topic("iot/devices/42/properties/x"), None);
        assert_eq!(parse_report_topic("other/42/properties"), None);
        // 解析出的 id 可直接查 设备→租户 映射
        let mut map = HashMap::new();
        map.insert("42".to_string(), "tenant-1".to_string());
        let topic = "iot/devices/42/properties";
        assert_eq!(
            map.get(parse_report_topic(topic).unwrap())
                .map(String::as_str),
            Some("tenant-1")
        );
    }

    #[tokio::test]
    async fn auth_allows_platform_and_device_credentials() {
        // 平台账号（env 配置）
        let req = auth_req("broker-user", "any-client", "broker-pass");
        assert!(auth_allowed(Some("broker-user"), Some("broker-pass"), None, "p", &req).await);
        assert!(!auth_allowed(Some("broker-user"), Some("other-pass"), None, "p", &req).await);
        // 设备凭据：clientid 必须等于用户名
        let hash = crate::auth::hash_password("device-pass");
        assert!(
            auth_allowed(
                None,
                None,
                Some(&hash),
                "p",
                &auth_req("42", "42", "device-pass")
            )
            .await
        );
        assert!(
            !auth_allowed(
                None,
                None,
                Some(&hash),
                "p",
                &auth_req("42", "other", "device-pass")
            )
            .await
        );
        assert!(!auth_allowed(None, None, Some(&hash), "p", &auth_req("42", "42", "wrong")).await);
        // 未知用户名（无哈希）拒绝
        assert!(!auth_allowed(None, None, None, "p", &auth_req("42", "42", "device-pass")).await);
    }

    #[test]
    fn acl_limits_devices_to_own_topics() {
        // 平台账号只限 iot/ 命名空间
        assert!(acl_allowed(
            Some("broker-user"),
            &acl_req("broker-user", "iot/devices/1/properties", "publish")
        ));
        assert!(!acl_allowed(
            Some("broker-user"),
            &acl_req("broker-user", "other/topic", "publish")
        ));
        // 设备自身 topic 放行
        assert!(acl_allowed(
            None,
            &acl_req("42", "iot/devices/42/properties", "publish")
        ));
        assert!(acl_allowed(
            None,
            &acl_req("42", "iot/devices/42/heartbeat", "publish")
        ));
        assert!(acl_allowed(
            None,
            &acl_req("42", "iot/devices/42/commands", "subscribe")
        ));
        // 跨设备 / 越权动作拒绝
        assert!(!acl_allowed(
            None,
            &acl_req("42", "iot/devices/43/properties", "publish")
        ));
        assert!(!acl_allowed(
            None,
            &acl_req("42", "iot/devices/42/commands", "publish")
        ));
        assert!(!acl_allowed(
            None,
            &acl_req("42", "iot/devices/42/properties", "subscribe")
        ));
        assert!(!acl_allowed(
            None,
            &acl_req("", "iot/devices//properties", "publish")
        ));
    }
}
