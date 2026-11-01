//! 集成测试：find_link 必须按租户隔离——否则知道他租户平台设备 id 即可下发指令。
//! 运行：docker compose up -d mysql redis kafka
//!       cargo test -p ecat-access --test store_tenant_scope -- --ignored --nocapture
use ecat_access::store::Store;
use ecat_data_sqlx::SqlxClient;
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires docker compose up -d mysql"]
async fn find_link_is_scoped_to_tenant() {
    let db = Arc::new(
        SqlxClient::connect(
            &std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into()),
        )
        .await
        .unwrap(),
    );
    for file in [
        "migrations/0001_init.sql",
        "migrations/0002_vendor_auth.sql",
    ] {
        let sql = std::fs::read_to_string(file).unwrap();
        for stmt in sql.split(';').filter(|s| !s.trim().is_empty()) {
            db.execute(stmt).await.unwrap();
        }
    }
    let store = Store::new(db.clone(), "test-encrypt-key-0123456789");
    // REPLACE 保证覆盖历史残留行（固定 id）
    db.execute_with(
        "REPLACE INTO tenants (id, name) VALUES ('t-scope-a', 'tenant-a'), ('t-scope-b', 'tenant-b')",
        &[],
    )
    .await
    .unwrap();
    db.execute_with(
        "REPLACE INTO devices (id, tenant_id, name, vendor, status) \
         VALUES (900001, 't-scope-a', 'scope-dev', 'tuya', 'online')",
        &[],
    )
    .await
    .unwrap();
    db.execute_with(
        "REPLACE INTO device_links (device_id, tenant_id, vendor, vendor_id, vendor_name, category) \
         VALUES (900001, 't-scope-a', 'tuya', 'tuya-scope-1', 'scope-dev', 'temp_sensor')",
        &[],
    )
    .await
    .unwrap();

    // 设备属于 t-scope-a：本租户可查。
    assert_eq!(
        store.find_link("t-scope-a", "900001").await.unwrap(),
        Some(("tuya".to_string(), "tuya-scope-1".to_string()))
    );
    // 他租户即使知道平台设备 id，也必须查不到（不泄漏链接信息）。
    assert_eq!(store.find_link("t-scope-b", "900001").await.unwrap(), None);
}
