//! 设备离线巡检（B-1）：周期扫描直连设备，心跳超时（默认 5 分钟）未上报
//! 则标记 offline 并产生 offline 事件。
//!
//! 依据：Redis 影子（shadow:{device_id}）的 ts 字段记录最近一次上报时间。
//! 巡检间隔 / 超时阈值均可配；状态翻转幂等（已是 offline 不再重复告警）。
use crate::events::{publish_event, shadow_apply, shadow_key};
use crate::models::EventMessage;
use crate::store::Store;
use ecat_data::Cache;
use ecat_data_redis::{RedisCache, RedisLock};
use ecat_iot::now_ms;
use ecat_lock::DistributedLock;
use ecat_mq_kafka::KafkaMq;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

/// 轮次锁键名：同一时刻只允许一个副本执行巡检。
const ROUND_LOCK_KEY: &str = "offline-patrol-lock";
/// 设备级翻转锁 TTL：远小于离线判定窗口，靠过期自愈。
const DEVICE_LOCK_TTL: Duration = Duration::from_secs(120);

/// 心跳超时（毫秒）：env OFFLINE_TIMEOUT_SECS 默认 300s（5 分钟）。
pub fn offline_timeout() -> i64 {
    std::env::var("OFFLINE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(|s: i64| s * 1000)
        .unwrap_or(300 * 1000)
}

/// 轮次锁 TTL：略小于巡检间隔（留 5s 余量），保证下一轮能重新抢到；
/// 间隔异常小时兜底 1s。
fn round_lock_ttl(interval: Duration) -> Duration {
    let ttl = interval.saturating_sub(Duration::from_secs(5));
    if ttl.is_zero() {
        Duration::from_secs(1)
    } else {
        ttl
    }
}

/// 单次巡检：扫描直连设备，超时未上报的标记 offline。
/// 返回本次新翻转为 offline 的设备数。失败仅记日志，不 panic。
pub async fn run_once(
    store: Arc<Store>,
    redis: Arc<RedisCache>,
    kafka: Arc<KafkaMq>,
    patrol_lock: &RedisLock,
    round_ttl: Duration,
) -> u64 {
    let timeout = offline_timeout();
    if timeout <= 0 {
        return 0;
    }
    // 多副本互斥：抢不到本轮锁（别的副本正在巡检）即整轮跳过。
    match patrol_lock.acquire(ROUND_LOCK_KEY, round_ttl).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            tracing::debug!("offline patrol: round lock held by another replica; skip");
            return 0;
        }
        Err(e) => {
            tracing::warn!(error = %e, "offline patrol: round lock acquire failed");
            return 0;
        }
    }
    let devices = match store.list_direct_devices().await {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "offline patrol: list direct devices failed");
            return 0;
        }
    };
    let now = now_ms();
    let mut flipped = 0u64;
    for (device_id, tenant_id) in devices {
        // 读影子最近上报时间
        let key = shadow_key(&device_id);
        let last_ts: i64 = match redis.get(&key).await {
            Ok(Some(raw)) => serde_json::from_slice::<Value>(&raw)
                .ok()
                .and_then(|v| v.get("ts").and_then(Value::as_i64).map(|t| t as i64))
                .unwrap_or(0),
            Ok(None) => 0,
            Err(e) => {
                tracing::warn!(device = %device_id, error = %e, "offline patrol: shadow read failed");
                continue;
            }
        };
        // 无历史上报（last_ts==0）：视为从未在线，跳过（避免冷启动误标）。
        if last_ts == 0 {
            continue;
        }
        if now - last_ts < timeout {
            continue; // 仍在心跳窗口内
        }
        // 已离线则跳过（幂等），否则标记 offline + 发事件
        let already_offline = redis
            .get(&key)
            .await
            .ok()
            .flatten()
            .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
            .and_then(|v| v.get("online").and_then(Value::as_bool))
            .unwrap_or(false)
            == false;
        if already_offline {
            continue;
        }
        // 设备级锁：轮次锁 TTL 短于巡检间隔，慢轮次仍可能与其他副本重叠，
        // 翻转前再按设备 NX 去重（不释放，靠 TTL 过期自愈）。
        match patrol_lock
            .acquire(&format!("offline-lock:{device_id}"), DEVICE_LOCK_TTL)
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!(device = %device_id, error = %e, "offline patrol: device lock acquire failed");
                continue;
            }
        }
        // 更新影子 online=false，落库状态，发布 offline 事件
        let ev = EventMessage {
            device_id: device_id.clone(),
            tenant_id: tenant_id.clone(),
            kind: "offline".into(),
            code: String::new(),
            value: Value::Null,
            ts: now,
        };
        if let Err(e) = shadow_apply(&redis, &ev).await {
            tracing::warn!(device = %device_id, error = %e, "offline patrol: shadow apply failed");
            continue;
        }
        if let Err(e) = publish_event(&kafka, &ev).await {
            tracing::warn!(device = %device_id, error = %e, "offline patrol: publish offline event failed");
        }
        if let Err(e) = store.set_device_offline(&device_id).await {
            tracing::warn!(device = %device_id, error = %e, "offline patrol: db status update failed");
        }
        flipped += 1;
        tracing::info!(device = %device_id, "device marked offline (heartbeat timeout)");
    }
    flipped
}

/// 注册周期巡检任务：每 `interval` 运行一次 run_once。
pub fn register(
    scheduler: &mut ecat_scheduler::Scheduler,
    store: Arc<Store>,
    redis: Arc<RedisCache>,
    kafka: Arc<KafkaMq>,
    lock: Arc<RedisLock>,
    interval: Duration,
) {
    // 轮次锁 TTL 稍短于巡检间隔：正常不互抢，副本异常退出也能下一轮自愈。
    let round_ttl = round_lock_ttl(interval);
    scheduler.every(interval, move || {
        let (store, redis, kafka, lock) =
            (store.clone(), redis.clone(), kafka.clone(), lock.clone());
        async move {
            run_once(store, redis, kafka, &lock, round_ttl).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_lock_ttl_stays_below_interval() {
        assert_eq!(
            round_lock_ttl(Duration::from_secs(60)),
            Duration::from_secs(55)
        );
        // 间隔不足 5s 时兜底 1s，避免 PX 0 非法过期时间
        assert_eq!(
            round_lock_ttl(Duration::from_secs(3)),
            Duration::from_secs(1)
        );
        assert_eq!(round_lock_ttl(Duration::ZERO), Duration::from_secs(1));
    }

    #[test]
    fn offline_timeout_env_behavior() {
        // env 为进程全局，同变量只允许一个测试触碰：同 binary 内并行线程
        // 若分属多个测试会相互 set_var/remove_var 竞态，故合并串行断言。
        unsafe { std::env::remove_var("OFFLINE_TIMEOUT_SECS") };
        assert_eq!(offline_timeout(), 300 * 1000);
        unsafe { std::env::set_var("OFFLINE_TIMEOUT_SECS", "10") };
        assert_eq!(offline_timeout(), 10 * 1000);
        unsafe { std::env::set_var("OFFLINE_TIMEOUT_SECS", "0") };
        assert_eq!(offline_timeout(), 0);
        unsafe { std::env::remove_var("OFFLINE_TIMEOUT_SECS") };
    }
}
