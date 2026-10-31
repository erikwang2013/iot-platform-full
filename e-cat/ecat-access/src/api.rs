use crate::adapter::{adapter_for, VendorCreds};
use crate::breaker::{BreakerConfig, CircuitBreaker};
use crate::store::Store;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use ecat_data_redis::RedisCache;
use ecat_mq_kafka::KafkaMq;
use ecat_mq_mqtt::MqttMq;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{Arc, RwLock};

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<Store>,
    pub kafka: Arc<KafkaMq>,
    pub redis: Arc<RedisCache>,
    pub mqtt: Arc<MqttMq>,
    /// 厂商 API 熔断器（B-2）：按 vendor 隔离，open 时降级到缓存设备列表。
    pub breakers: Arc<RwLock<std::collections::HashMap<String, CircuitBreaker>>>,
}

impl ApiState {
    /// 取指定厂商的熔断器（不存在则惰性创建并共享同一实例）。
    pub fn breaker(&self, vendor: &str) -> CircuitBreaker {
        breaker_in(&self.breakers, vendor)
    }
}

fn breaker_in(
    map: &Arc<RwLock<std::collections::HashMap<String, CircuitBreaker>>>,
    vendor: &str,
) -> CircuitBreaker {
    // 锁内只做 map 操作，guard 不跨 await
    map.write()
        .unwrap_or_else(|e| e.into_inner())
        .entry(vendor.to_string())
        .or_insert_with(|| CircuitBreaker::new(BreakerConfig::default()))
        .clone()
}

/// 厂商 token 如即将过期先刷新；有变化则落库，返回本次调用应使用的凭据。
async fn ensure_fresh_creds(
    api: &ApiState,
    tenant_id: &str,
    vendor: &str,
    creds: &crate::adapter::VendorCreds,
    adapter: &dyn crate::adapter::VendorAdapter,
) -> Result<crate::adapter::VendorCreds, (StatusCode, String)> {
    let fresh = adapter
        .ensure_fresh(creds)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    if fresh.access_token != creds.access_token
        || fresh.refresh_token != creds.refresh_token
        || fresh.expires_at != creds.expires_at
    {
        api.store
            .save_creds(tenant_id, vendor, &serde_json::to_value(&fresh).unwrap())
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    }
    Ok(fresh)
}

/// 构造受保护 API 路由（挂载见 main.rs，路径前缀 /api/access）。
pub fn router(api: ApiState) -> axum::Router {
    axum::Router::new()
        .route("/vendors/{vendor}/creds", axum::routing::put(save_creds))
        .route("/vendors/{vendor}/import", axum::routing::post(import_devices))
        .route("/devices/{device_id}/command", axum::routing::post(send_command))
        .with_state(api)
}

/// PUT /api/access/vendors/{vendor}/creds（受保护）
/// 直填凭据（AK/SK 类厂商无 OAuth：huawei/aws/azure；涂鸦/小米也可用），AES 加密落库。
pub async fn save_creds(
    State(api): State<ApiState>,
    axum::Extension(tenant_id): axum::Extension<String>,
    Path(vendor): Path<String>,
    Json(creds): Json<VendorCreds>,
) -> Result<Json<Value>, (StatusCode, String)> {
    // 输入校验：系统边界，未知厂商/空凭据直接拒绝
    if adapter_for(&vendor).is_err() {
        return Err((StatusCode::BAD_REQUEST, format!("unknown vendor {vendor}")));
    }
    if creds.client_id.trim().is_empty() || creds.client_secret.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "client_id and client_secret required".into()));
    }
    api.store
        .save_creds(&tenant_id, &vendor, &serde_json::to_value(&creds).unwrap())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(json!({ "ok": true, "vendor": vendor })))
}

/// POST /api/access/vendors/{vendor}/import（受保护）
/// 拉取厂商设备列表 → 入库 devices + device_links。
pub async fn import_devices(
    State(api): State<ApiState>,
    axum::Extension(tenant_id): axum::Extension<String>,
    Path(vendor): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let adapter = adapter_for(&vendor).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let creds = api
        .store
        .load_creds(&tenant_id, &vendor)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let creds: crate::adapter::VendorCreds =
        serde_json::from_value(creds).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // token 即将过期先刷新并落库（刷新后的 refresh_token 可能一次性）
    let creds = ensure_fresh_creds(&api, &tenant_id, &vendor, &creds, adapter.as_ref()).await?;
    // B-2 熔断：open 期间跳过上游调用，降级返回当前已入库设备列表（缓存）
    let breaker = api.breaker(&vendor);
    let (devices, degraded) = if breaker.allow() {
        match adapter.list_devices(&creds).await {
            Ok(devs) => {
                breaker.record(true);
                (devs, false)
            }
            Err(e) => {
                breaker.record(false);
                // 失败但未熔断：仍降级到缓存并标记（避免整批失败）
                let cached = api
                    .store
                    .list_vendor_devices(&tenant_id, &vendor)
                    .await
                    .unwrap_or_default();
                if cached.is_empty() {
                    return Err((StatusCode::BAD_GATEWAY, e.to_string()));
                }
                (cached, true)
            }
        }
    } else {
        // 熔断 open：直接返回缓存
        (api.store.list_vendor_devices(&tenant_id, &vendor).await.unwrap_or_default(), true)
    };
    // 配额校验（C-5）：仅统计"确实会新增"的设备（已存在复用不占配额），
    // 超限整批拒绝（409），防部分生效。
    let mut adding = 0i64;
    for d in &devices {
        if api
            .store
            .find_device_by_vendor_id(&vendor, &d.vendor_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            .is_none()
        {
            adding += 1;
        }
    }
    api.store
        .check_quota(&tenant_id, adding)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e))?;
    let mut imported = Vec::new();
    for d in &devices {
        let platform_id = api
            .store
            .upsert_device(&tenant_id, &vendor, &d.vendor_id, &d.name, &d.category, d.online)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        // T2.8: 设备写入 OpenSearch 索引（同步 index，失败仅 warn 不阻断导入）
        if let Some(search) = ecat_search::connect_search() {
            let status = if d.online { "online" } else { "offline" };
            let doc = json!({
                "tenant_id": tenant_id,
                "id": platform_id,
                "name": d.name,
                "vendor": vendor,
                "status": status,
            });
            if let Err(e) = search.index("devices", &platform_id, &doc).await {
                tracing::warn!(error = %e, "device index failed");
            }
        }
        imported.push(json!({ "platform_id": platform_id, "vendor_id": d.vendor_id, "name": d.name }));
    }
    // degraded=true 表示本次来自缓存（熔断或上游失败降级），供前端展示
    Ok(Json(json!({ "imported": imported, "count": imported.len(), "degraded": degraded })))
}

#[derive(Deserialize)]
pub struct CommandReq {
    pub code: String,
    pub value: Value,
}

/// POST /api/access/devices/{id}/command（受保护）
/// 查 device_links → 适配器 send_command → 厂商 OpenAPI / 直连 MQTT 下发。
pub async fn send_command(
    State(api): State<ApiState>,
    axum::Extension(tenant_id): axum::Extension<String>,
    Path(device_id): Path<String>,
    Json(req): Json<CommandReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let link = api
        .store
        .find_link(&tenant_id, &device_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let (vendor, vendor_id) = link
        .ok_or_else(|| (StatusCode::NOT_FOUND, "device not linked".to_string()))?;
    if vendor == "direct" {
        // 直连设备：在线则 MQTT 下发；离线则入队（D-2），上线后补发。
        if crate::command_queue::device_online(&api.redis, &device_id).await {
            crate::mqtt::publish_command(&api.mqtt, &device_id, &req.code, &req.value)
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
            return Ok(Json(json!({ "ok": true, "channel": "mqtt" })));
        }
        let id = api
            .store
            .enqueue_command(
                &tenant_id,
                &device_id,
                &req.code,
                &req.value,
                crate::command_queue::command_expire_secs(),
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
        return Ok(Json(
            json!({ "ok": true, "queued": true, "channel": "mqtt", "queue_id": id }),
        ));
    }
    let adapter = adapter_for(&vendor).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let creds = api
        .store
        .load_creds(&tenant_id, &vendor)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let creds: crate::adapter::VendorCreds =
        serde_json::from_value(creds).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let creds = ensure_fresh_creds(&api, &tenant_id, &vendor, &creds, adapter.as_ref()).await?;
    adapter
        .send_command(&creds, &vendor_id, &req.code, req.value.clone())
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    Ok(Json(json!({ "ok": true, "channel": vendor })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_state_is_shared_per_vendor() {
        let map = Arc::new(RwLock::new(std::collections::HashMap::new()));
        let first = breaker_in(&map, "tuya");
        let second = breaker_in(&map, "tuya");
        for _ in 0..5 {
            first.record(false);
        }
        assert!(second.is_open(), "同一厂商的 breaker 必须共享状态");
        assert!(!breaker_in(&map, "miot").is_open(), "厂商之间状态隔离");
    }
}
