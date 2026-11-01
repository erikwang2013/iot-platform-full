//! 集成测试：单台 DELETE /api/devices/{id} 必须级联清理标签/分组成员/厂商链接。
//! 运行：docker compose up -d mysql
//!       cargo test -p ecat-device --test device_delete -- --ignored --nocapture
use axum::Extension;
use axum::extract::{Path, State};
use ecat_data_sqlx::SqlxClient;
use ecat_device::{Db, delete_device};
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires docker compose up -d mysql"]
async fn delete_device_removes_tags_members_and_links() {
    let dsn = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into());
    let db = Arc::new(SqlxClient::connect(&dsn).await.unwrap());
    ecat_device::migrate(&db).await.unwrap();

    let (tenant, device) = ("t-del-a", 777001i64);
    db.execute_with(
        "REPLACE INTO tenants (id, name) VALUES (?, 'del-a'), ('t-del-b', 'del-b')",
        &[json!(tenant)],
    )
    .await
    .unwrap();
    db.execute_with("DELETE FROM devices WHERE id = ?", &[json!(device)])
        .await
        .unwrap();
    db.execute_with(
        "REPLACE INTO devices (id, tenant_id, name, vendor, status) \
         VALUES (?, ?, 'del-dev', 'tuya', 'online')",
        &[json!(device), json!(tenant)],
    )
    .await
    .unwrap();
    db.execute_with(
        "REPLACE INTO device_links (device_id, tenant_id, vendor, vendor_id, vendor_name, category) \
         VALUES (?, ?, 'tuya', 'tuya-del-1', 'del-dev', 'sensor')",
        &[json!(device), json!(tenant)],
    )
    .await
    .unwrap();
    db.execute_with(
        "INSERT IGNORE INTO device_tags (device_id, tag) VALUES (?, 'vip')",
        &[json!(device)],
    )
    .await
    .unwrap();
    let group: i64 = 7771001;
    db.execute_with(
        "REPLACE INTO device_groups (id, tenant_id, name) VALUES (?, ?, 'del-group')",
        &[json!(group), json!(tenant)],
    )
    .await
    .unwrap();
    db.execute_with(
        "INSERT IGNORE INTO device_group_members (group_id, device_id) VALUES (?, ?)",
        &[json!(group), json!(device)],
    )
    .await
    .unwrap();

    let resp = delete_device(
        State(Db(db.clone())),
        Extension(tenant.to_string()),
        Path(device.to_string()),
    )
    .await;
    assert!(resp.is_ok(), "delete failed: {resp:?}");

    for (table, sql) in [
        ("devices", "SELECT COUNT(*) AS n FROM devices WHERE id = ?"),
        (
            "device_links",
            "SELECT COUNT(*) AS n FROM device_links WHERE device_id = ?",
        ),
        (
            "device_tags",
            "SELECT COUNT(*) AS n FROM device_tags WHERE device_id = ?",
        ),
        (
            "device_group_members",
            "SELECT COUNT(*) AS n FROM device_group_members WHERE device_id = ?",
        ),
    ] {
        let rows = db.query_with(sql, &[json!(device)]).await.unwrap();
        let n = rows
            .first()
            .and_then(|r| r.get("n"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-1);
        assert_eq!(n, 0, "{table} 残留 {n} 行");
    }
}
