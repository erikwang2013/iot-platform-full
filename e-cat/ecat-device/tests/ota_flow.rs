//! OTA 闭环集成测试：需 MySQL（`docker compose up -d mysql`）后运行：
//! `DATABASE_URL=mysql://iot:iot@127.0.0.1:3306/iot cargo test -p ecat-device --test ota_flow -- --ignored`
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use ecat_data::RdbmsClient;
use ecat_data_sqlx::SqlxClient;
use ecat_device::{
    Db, migrate,
    ota_rollout::{BatchOtaReq, create_ota_tasks_batch, rollback_ota_task},
    transition_allowed,
};
use serde_json::{Value, json};
use std::sync::Arc;

async fn db() -> Arc<SqlxClient> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into());
    let db = Arc::new(SqlxClient::connect(&url).await.unwrap());
    // 新库先建表，否则 tenants 尚不存在；随后重建被测试清空的数据表
    migrate(&db).await.expect("migrate");
    db.execute("INSERT IGNORE INTO tenants (id, name) VALUES ('ota-tenant', 'ota')")
        .await
        .expect("insert tenant");
    // 迁移可重复跑：先删引用方再删父表
    for t in [
        "ota_upgrade_tasks",
        "ota_firmwares",
        "device_links",
        "devices",
    ] {
        db.execute(&format!("DROP TABLE IF EXISTS {t}"))
            .await
            .expect("drop");
    }
    migrate(&db).await.expect("migrate");
    db
}

const TEST_DEVICE: i64 = 900_000_000_000_101;
const TEST_FW: i64 = 900_000_000_000_110;
const TEST_TASK: i64 = 900_000_000_000_120;

async fn insert_device(db: &Arc<SqlxClient>) {
    db.execute(&format!(
        "INSERT IGNORE INTO devices (id, tenant_id, name, vendor, status) \
         VALUES ({TEST_DEVICE}, 'ota-tenant', 'dev', 'mock', 'online')"
    ))
    .await
    .expect("insert device");
}

async fn insert_firmware(db: &Arc<SqlxClient>) {
    db.execute(&format!(
        "INSERT IGNORE INTO ota_firmwares (id, tenant_id, name, version, url, description) \
         VALUES ({TEST_FW}, 'ota-tenant', 'fw', '1.0.0', 'http://cdn/x.bin', '')"
    ))
    .await
    .expect("insert firmware");
}

async fn insert_task(db: &Arc<SqlxClient>) -> i64 {
    db.execute(&format!(
        "INSERT IGNORE INTO ota_upgrade_tasks (id, tenant_id, device_id, firmware_id, status) \
         VALUES ({TEST_TASK}, 'ota-tenant', {TEST_DEVICE}, {TEST_FW}, 'pending')"
    ))
    .await
    .expect("insert task");
    TEST_TASK
}

async fn task_status(db: &Arc<SqlxClient>, id: i64) -> (String, i64, String) {
    let rows = db
        .query_with(
            "SELECT status, progress, message FROM ota_upgrade_tasks WHERE id = ?",
            &[json!(id)],
        )
        .await
        .unwrap();
    let r = rows.first().unwrap();
    (
        r.get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
        r.get("progress")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(-1),
        r.get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string(),
    )
}

#[tokio::test]
#[ignore]
async fn ota_task_full_lifecycle_via_report() {
    let db = db().await;
    insert_device(&db).await;
    insert_firmware(&db).await;
    let task = insert_task(&db).await;

    // 初始 pending
    assert_eq!(task_status(&db, task).await.0, "pending");
    // 跳步被状态机拒绝（模拟设备不能直接报 success）
    assert!(!transition_allowed("pending", "success"));

    // pending → downloading（进度 30）
    db.execute_with(
        "UPDATE ota_upgrade_tasks SET status = 'downloading', progress = 30, message = '' \
         WHERE id = ?",
        &[json!(task)],
    )
    .await
    .unwrap();
    assert_eq!(
        task_status(&db, task).await,
        ("downloading".into(), 30, "".into())
    );

    // downloading → installing（进度 80 + 消息）
    db.execute_with(
        "UPDATE ota_upgrade_tasks SET status = 'installing', progress = 80, message = '烧写中' \
         WHERE id = ?",
        &[json!(task)],
    )
    .await
    .unwrap();

    // installing → success：progress 强制 100
    db.execute_with(
        "UPDATE ota_upgrade_tasks SET status = 'success', progress = 100 WHERE id = ?",
        &[json!(task)],
    )
    .await
    .unwrap();
    let (status, progress, msg) = task_status(&db, task).await;
    assert_eq!(status, "success");
    assert_eq!(progress, 100);
    assert_eq!(msg, "烧写中");
}

#[tokio::test]
#[ignore]
async fn ota_failed_task_frozen() {
    let db = db().await;
    insert_device(&db).await;
    insert_firmware(&db).await;
    let task = insert_task(&db).await;

    db.execute_with(
        "UPDATE ota_upgrade_tasks SET status = 'failed', progress = 50, message = '校验失败' \
         WHERE id = ?",
        &[json!(task)],
    )
    .await
    .unwrap();
    // 失败后不可再回 success（状态机冻结）
    assert!(!transition_allowed("failed", "success"));
    assert_eq!(task_status(&db, task).await.0, "failed");
}

const OWN_DEVICE: i64 = 900_000_000_000_001;
const FOREIGN_DEVICE: i64 = 900_000_000_000_002;
const OLD_FW: i64 = 900_000_000_000_010;
const NEW_FW: i64 = 900_000_000_000_011;

async fn batch(db: &Arc<SqlxClient>, req: BatchOtaReq) -> Result<Value, (StatusCode, String)> {
    create_ota_tasks_batch(
        State(Db(db.clone())),
        Extension("ota-tenant".to_string()),
        Json(req),
    )
    .await
    .map(|Json(v)| v)
}

fn batch_req(device_ids: Vec<i64>) -> BatchOtaReq {
    BatchOtaReq {
        firmware_id: NEW_FW.to_string(),
        device_ids: device_ids.into_iter().map(|id| id.to_string()).collect(),
        rollout_percent: 100,
    }
}

#[tokio::test]
#[ignore]
async fn ota_batch_gray_rollout_and_rollback() {
    let db = db().await;
    db.execute("INSERT IGNORE INTO tenants (id, name) VALUES ('ota-other', 'other')")
        .await
        .unwrap();
    for (id, tenant, name) in [
        (OWN_DEVICE, "ota-tenant", "own"),
        (FOREIGN_DEVICE, "ota-other", "foreign"),
    ] {
        db.execute(&format!(
            "INSERT IGNORE INTO devices (id, tenant_id, name, vendor, status) \
             VALUES ({id}, '{tenant}', '{name}', 'mock', 'online')"
        ))
        .await
        .unwrap();
    }
    for (id, version) in [(OLD_FW, "1.0.0"), (NEW_FW, "2.0.0")] {
        db.execute(&format!(
            "INSERT IGNORE INTO ota_firmwares (id, tenant_id, name, version, url, description) \
             VALUES ({id}, 'ota-tenant', 'fw', '{version}', 'http://cdn/{id}.bin', '')"
        ))
        .await
        .unwrap();
    }
    // 设备当前成功版本（回滚目标）
    db.execute(&format!(
        "INSERT IGNORE INTO ota_upgrade_tasks (id, tenant_id, device_id, firmware_id, status) \
         VALUES (900000000000020, 'ota-tenant', {OWN_DEVICE}, {OLD_FW}, 'success')"
    ))
    .await
    .unwrap();

    // 显式 id 跨租户 → 403
    let err = batch(&db, batch_req(vec![FOREIGN_DEVICE]))
        .await
        .unwrap_err();
    assert_eq!(
        err,
        (StatusCode::FORBIDDEN, "device not in tenant".to_string())
    );

    // 本租户设备 → 创建 1 条
    let first = batch(&db, batch_req(vec![OWN_DEVICE])).await.unwrap();
    assert_eq!(first["created"], json!(1));
    assert_eq!(first["skipped"], json!(0));
    assert_eq!(first["selected"], json!(1));
    assert_eq!(first["total"], json!(1));

    // 重复同批次 → 幂等，不重复创建
    let again = batch(&db, batch_req(vec![OWN_DEVICE])).await.unwrap();
    assert_eq!(again["created"], json!(0));
    assert_eq!(again["skipped"], json!(1));
    assert_eq!(again["selected"], json!(1));
    assert_eq!(again["total"], json!(1));

    // 刚创建的任务置为 failed 后回滚 → 新 pending 任务指向上一个成功版本
    let rows = db
        .query_with(
            "SELECT id FROM ota_upgrade_tasks WHERE tenant_id = ? AND device_id = ? \
             AND firmware_id = ? AND status = 'pending'",
            &[json!("ota-tenant"), json!(OWN_DEVICE), json!(NEW_FW)],
        )
        .await
        .unwrap();
    let failed_id = rows
        .first()
        .and_then(|r| r.get("id"))
        .and_then(Value::as_i64)
        .expect("pending task created by batch");
    db.execute_with(
        "UPDATE ota_upgrade_tasks SET status = 'failed' WHERE id = ?",
        &[json!(failed_id)],
    )
    .await
    .unwrap();

    let rolled = rollback_ota_task(
        State(Db(db.clone())),
        Extension("ota-tenant".to_string()),
        Path(failed_id.to_string()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(rolled["status"], json!("pending"));
    assert_eq!(rolled["device_id"], json!(OWN_DEVICE.to_string()));
    assert_eq!(rolled["firmware_id"], json!(OLD_FW.to_string()));
    let new_task_id = rolled["id"].as_str().unwrap().parse::<i64>().unwrap();

    let rows = db
        .query_with(
            "SELECT status FROM ota_upgrade_tasks WHERE id = ? AND device_id = ? AND firmware_id = ?",
            &[json!(new_task_id), json!(OWN_DEVICE), json!(OLD_FW)],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.first()
            .and_then(|r| r.get("status"))
            .and_then(Value::as_str),
        Some("pending")
    );
}
