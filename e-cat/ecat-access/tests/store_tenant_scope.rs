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

/// 直连设备创建必须同时写 devices 与 device_links(direct)，并遵守租户配额；
/// 否则 send_command 的 find_link 分支永远走不到 MQTT。
#[tokio::test]
#[ignore = "requires docker compose up -d mysql"]
async fn create_direct_device_links_and_enforces_quota() {
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
    db.execute_with(
        "REPLACE INTO tenants (id, name, quota) \
         VALUES ('t-direct', 'direct', 100), ('t-direct-quota', 'quota', 1)",
        &[],
    )
    .await
    .unwrap();

    let id = store
        .create_direct_device("t-direct", "direct-1")
        .await
        .expect("create direct device");
    assert_eq!(
        store.find_link("t-direct", &id).await.unwrap(),
        Some(("direct".to_string(), id.clone())),
        "直连设备必须有 vendor=direct 的 device_links 行"
    );
    // 设备行 vendor=direct，mqtt/offline 扫描可见
    let rows = db
        .query_with(
            "SELECT vendor FROM devices WHERE id = ? AND tenant_id = 't-direct'",
            &[serde_json::json!(id.parse::<i64>().unwrap())],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.first()
            .and_then(|r| r.get("vendor"))
            .and_then(|v| v.as_str()),
        Some("direct")
    );

    // quota=1：第一台成功，第二台 409 语义错误
    store
        .create_direct_device("t-direct-quota", "first")
        .await
        .expect("first within quota");
    let err = store
        .create_direct_device("t-direct-quota", "second")
        .await
        .unwrap_err();
    assert!(
        err.starts_with("device quota exceeded"),
        "unexpected error: {err}"
    );
}
