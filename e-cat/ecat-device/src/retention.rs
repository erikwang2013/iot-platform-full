//! OTA 下载记录保留期：周期删除过期的 ota_downloads 行（finding 13 设备侧）。
//! 只保留最近 OTA_DOWNLOAD_RETENTION_DAYS（默认 180）天的下载统计。

use crate::Db;
use std::time::Duration;

/// 保留天数：env OTA_DOWNLOAD_RETENTION_DAYS，默认 180；0/非法值按 180。
/// 0 不能解释为“立即删除”，否则一次误配会清空统计。
fn retention_days() -> u32 {
    retention_days_from(std::env::var("OTA_DOWNLOAD_RETENTION_DAYS").ok().as_deref())
}

fn retention_days_from(raw: Option<&str>) -> u32 {
    raw.and_then(|v| v.trim().parse().ok())
        .filter(|v| *v > 0)
        .unwrap_or(180)
}

/// 注册周期清理任务：每 `interval` 删一次过期下载记录，失败仅 warn 不 panic。
pub fn register(scheduler: &mut ecat_scheduler::Scheduler, db: Db, interval: Duration) {
    scheduler.every(interval, move || {
        let db = db.clone();
        async move {
            let days = retention_days() as i64;
            match db
                .0
                .execute_with(
                    "DELETE FROM ota_downloads WHERE created_at < NOW() - INTERVAL ? DAY",
                    &[serde_json::json!(days)],
                )
                .await
            {
                Ok(n) if n > 0 => {
                    tracing::info!(deleted = n, days, "ota download retention cleanup");
                }
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "ota download retention cleanup failed"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::retention_days_from;

    #[test]
    fn zero_and_invalid_fall_back_to_default() {
        assert_eq!(retention_days_from(None), 180);
        assert_eq!(retention_days_from(Some("0")), 180);
        assert_eq!(retention_days_from(Some("-3")), 180);
        assert_eq!(retention_days_from(Some("abc")), 180);
        assert_eq!(retention_days_from(Some(" 30 ")), 30);
    }
}
