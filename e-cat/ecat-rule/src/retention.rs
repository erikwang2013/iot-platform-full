//! 告警/通知保留期清理：alert_records 与 notifications 只增不减，
//! 由定时任务按天数删除历史行（报表/审计各有自己的策略）。

use crate::store::RuleStore;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// 注册周期任务：env ALERT_RETENTION_DAYS（默认 180）、
/// NOTIFICATION_RETENTION_DAYS（默认 90）；失败只 warn，不 panic。
pub fn register(
    scheduler: &mut ecat_scheduler::Scheduler,
    store: Arc<RuleStore>,
    interval: Duration,
) {
    let alert_days = env_days("ALERT_RETENTION_DAYS", 180);
    let notification_days = env_days("NOTIFICATION_RETENTION_DAYS", 90);
    scheduler.every(interval, move || {
        let store = store.clone();
        async move {
            match store.cleanup_retention(alert_days, notification_days).await {
                Ok((alerts, notifications)) if alerts + notifications > 0 => {
                    tracing::info!(alerts, notifications, "retention cleanup");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "retention cleanup failed"),
            }
        }
    });
}

/// 保留天数：非法值用默认值，0 视为 1（避免误配置清空整表）。
fn env_days(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
        .max(1)
}

impl RuleStore {
    /// 删除超过保留期的告警/通知行，返回 (告警, 通知) 删除数。
    /// 每表一条 DELETE；低频任务不做分批。
    pub async fn cleanup_retention(
        &self,
        alert_days: u32,
        notification_days: u32,
    ) -> Result<(u64, u64), String> {
        let alerts = self
            .db
            .execute_with(
                "DELETE FROM alert_records WHERE created_at < NOW() - INTERVAL ? DAY",
                &[json!(alert_days)],
            )
            .await
            .map_err(|e| format!("cleanup alert records: {e}"))?;
        let notifications = self
            .db
            .execute_with(
                "DELETE FROM notifications WHERE created_at < NOW() - INTERVAL ? DAY",
                &[json!(notification_days)],
            )
            .await
            .map_err(|e| format!("cleanup notifications: {e}"))?;
        Ok((alerts, notifications))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_days_fall_back_and_clamp() {
        // 未设置 / 非法值 → 默认；0 → 1（避免清空整表）
        assert_eq!(env_days("ECAT_RULE_RETENTION_TEST_UNSET", 180), 180);
        // SAFETY: 测试专用变量名，无并发读写同一变量的其他线程
        unsafe { std::env::set_var("ECAT_RULE_RETENTION_TEST_ZERO", "0") };
        assert_eq!(env_days("ECAT_RULE_RETENTION_TEST_ZERO", 90), 1);
        // SAFETY: 同上
        unsafe { std::env::set_var("ECAT_RULE_RETENTION_TEST_BAD", "abc") };
        assert_eq!(env_days("ECAT_RULE_RETENTION_TEST_BAD", 90), 90);
    }
}
