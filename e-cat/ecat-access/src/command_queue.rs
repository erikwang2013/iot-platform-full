//! 离线指令队列（D-2）：设备离线时指令入队，上线后自动补发。
//! 补发触发点：send_command 检测到设备离线时入队；设备上线（shadow online）
//! 时由 flush_for_device 补发。补发按入队时间升序，带过期时间。
use crate::events::{shadow_key, shadow_seen_ms};
use crate::store::Store;
use ecat_data::Cache;
use ecat_data_redis::RedisCache;
use ecat_mq_mqtt::MqttMq;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// 指令过期时间（秒）：env COMMAND_EXPIRE_SECS 默认 3600（1 小时）。
pub fn command_expire_secs() -> i64 {
    std::env::var("COMMAND_EXPIRE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3600)
}

/// 在线新鲜度（毫秒）：env ONLINE_FRESHNESS_SECS 默认 120s。
/// 外部直连设备可能 60s 才上报一次；心跳设备 30s 一次，120s 内都算在线。
pub fn online_freshness_ms() -> i64 {
    std::env::var("ONLINE_FRESHNESS_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(|s: i64| s.max(1) * 1000)
        .unwrap_or(120 * 1000)
}

/// 在线判定（纯函数）：online 标记为真且最近一次上报仍在新鲜窗口内。
/// 只看 online 标记不够：短暂断连时旧标记可能仍为 true，指令会 QoS0
/// 发出却无人接收；此时应入队等设备真正上线补发。
/// 心跳边沿判定（events::shadow_mark_online）复用本函数。
pub(crate) fn shadow_online(shadow: &Value, now_ms: i64, freshness_ms: i64) -> bool {
    shadow
        .get("online")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && now_ms.saturating_sub(shadow_seen_ms(shadow)) < freshness_ms
}

/// 设备是否在线（Redis shadow 的 online 标记 + 最近上报新鲜度）。
pub async fn device_online(redis: &RedisCache, device_id: &str) -> bool {
    redis
        .get(&shadow_key(device_id))
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
        .is_some_and(|v| shadow_online(&v, ecat_iot::now_ms(), online_freshness_ms()))
}

/// 注册过期指令清理任务：每 `interval` 删除 command_queue 过期行。
pub fn register_cleanup(
    scheduler: &mut ecat_scheduler::Scheduler,
    store: Arc<Store>,
    interval: Duration,
) {
    scheduler.every(interval, move || {
        let store = store.clone();
        async move {
            match store.delete_expired_commands().await {
                Ok(n) if n > 0 => tracing::info!(deleted = n, "expired commands cleaned"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "expired command cleanup failed"),
            }
        }
    });
}

/// 进程内正在补发的设备集合：心跳与属性上报可能同时触发同一设备的补发，
/// 不串行会导致同一队列行被两条路径各下发一次（进程内互斥；多副本 out of scope）。
static FLUSH_IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// 尝试占用设备补发权；已有同设备补发在进行时返回 None。
fn flush_guard(device_id: &str) -> Option<FlushGuard> {
    let mut in_flight = FLUSH_IN_FLIGHT
        .get_or_init(|| Mutex::new(HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !in_flight.insert(device_id.to_string()) {
        return None;
    }
    Some(FlushGuard {
        device_id: device_id.to_string(),
    })
}

/// Drop 时释放补发权（覆盖提前 return / panic 路径）。
struct FlushGuard {
    device_id: String,
}

impl Drop for FlushGuard {
    fn drop(&mut self) {
        if let Some(in_flight) = FLUSH_IN_FLIGHT.get() {
            let mut in_flight = in_flight.lock().unwrap_or_else(|e| e.into_inner());
            in_flight.remove(&self.device_id);
        }
    }
}

/// 设备上线后补发排队指令（按入队顺序）。逐条 MQTT 下发，成功即删。
/// 某条失败仅记日志、保留在队列，下次上线再补发。
pub async fn flush_for_device(
    store: Arc<Store>,
    redis: Arc<RedisCache>,
    mqtt: Arc<MqttMq>,
    device_id: &str,
) {
    // 同一设备已有补发在进行：直接跳过，避免竞态下重复下发
    let Some(_guard) = flush_guard(device_id) else {
        tracing::debug!(device = %device_id, "flush already in progress; skip");
        return;
    };
    let pending = match store.pending_commands(device_id).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(device = %device_id, error = %e, "flush commands: load pending failed");
            return;
        }
    };
    for (id, code, value) in pending {
        if let Err(e) = crate::mqtt::publish_command(&mqtt, device_id, &code, &value).await {
            tracing::warn!(device = %device_id, cmd = %id, error = %e, "flush command failed; kept in queue");
            continue;
        }
        if let Err(e) = store.delete_command(&id).await {
            tracing::warn!(device = %device_id, cmd = %id, error = %e, "flush command: delete failed");
        }
        tracing::info!(device = %device_id, cmd = %id, code = %code, "offline command replayed");
    }
    let _ = redis; // 保留引用签名（补发不依赖 shadow 写回）
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_expire_secs_env_behavior() {
        // env 为进程全局，同变量只允许一个测试触碰：并行线程下多写者会竞态，合并串行断言。
        unsafe { std::env::remove_var("COMMAND_EXPIRE_SECS") };
        assert_eq!(command_expire_secs(), 3600);
        unsafe { std::env::set_var("COMMAND_EXPIRE_SECS", "600") };
        assert_eq!(command_expire_secs(), 600);
        unsafe { std::env::remove_var("COMMAND_EXPIRE_SECS") };
    }

    #[test]
    fn online_freshness_env_behavior() {
        unsafe { std::env::remove_var("ONLINE_FRESHNESS_SECS") };
        assert_eq!(online_freshness_ms(), 120 * 1000);
        unsafe { std::env::set_var("ONLINE_FRESHNESS_SECS", "30") };
        assert_eq!(online_freshness_ms(), 30 * 1000);
        // 0/负数兜底 1s，避免所有设备永远被判离线
        unsafe { std::env::set_var("ONLINE_FRESHNESS_SECS", "0") };
        assert_eq!(online_freshness_ms(), 1000);
        unsafe { std::env::remove_var("ONLINE_FRESHNESS_SECS") };
    }

    #[test]
    fn shadow_online_requires_flag_and_fresh_seen() {
        let now = 1_000_000;
        let fresh = serde_json::json!({"online": true, "seen": now - 1_000});
        assert!(shadow_online(&fresh, now, 120_000));
        // 超过新鲜窗口：视为离线
        let stale = serde_json::json!({"online": true, "seen": now - 120_000});
        assert!(!shadow_online(&stale, now, 120_000));
        // 标记离线：即使刚上报也不算在线
        let offline = serde_json::json!({"online": false, "seen": now - 1_000});
        assert!(!shadow_online(&offline, now, 120_000));
        // 旧影子无 seen：回退 ts
        let legacy = serde_json::json!({"online": true, "ts": now - 1_000});
        assert!(shadow_online(&legacy, now, 120_000));
        // 未来时间（设备 ts 超前）不能溢出
        let future = serde_json::json!({"online": true, "ts": now + 10_000});
        assert!(shadow_online(&future, now, 120_000));
    }

    #[test]
    fn flush_guard_serializes_same_device() {
        let guard = flush_guard("dev-guard-test");
        assert!(guard.is_some());
        // 同一设备第二次进入被拒（心跳与属性上报竞态）
        assert!(flush_guard("dev-guard-test").is_none());
        // 其他设备不受影响
        assert!(flush_guard("dev-guard-other").is_some());
        // 释放后可再次进入
        drop(guard);
        assert!(flush_guard("dev-guard-test").is_some());
    }
}
