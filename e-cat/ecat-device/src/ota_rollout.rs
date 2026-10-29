//! OTA 灰度分批下发与失败回滚（P6）。
//! 灰度按设备 id 的 FNV-1a 哈希分桶：同一设备在任意比例下结果稳定，
//! 重复调用只会为新增的合格设备创建任务（已存在任务的设备跳过）。

use crate::{Db, db_err};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use ecat_data::RdbmsClient;
use serde_json::{Value, json};
use std::collections::HashSet;

/// FNV-1a 32 位：hash % 100 < percent。percent >= 100 恒真，0 恒假。
pub fn rollout_selected(device_id: &str, percent: u8) -> bool {
    let mut hash: u32 = 0x811c_9dc5;
    for b in device_id.as_bytes() {
        hash ^= u32::from(*b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    (hash % 100) < u32::from(percent)
}

#[derive(serde::Deserialize)]
pub struct BatchOtaReq {
    pub firmware_id: String,
    #[serde(default)]
    pub device_ids: Vec<String>,
    #[serde(default = "default_rollout_percent")]
    pub rollout_percent: u8,
}

fn default_rollout_percent() -> u8 {
    100
}

/// 单次批量上限：多行 INSERT 每行 4 个占位符，1000 行远低于 MySQL 65,535 参数上限。
const MAX_BATCH_DEVICES: usize = 1000;

fn over_batch_limit(n: usize) -> bool {
    n > MAX_BATCH_DEVICES
}

/// POST /api/ota/tasks/batch：按灰度比例给设备批量创建升级任务。
/// device_ids 为空表示全租户设备；已有本固件任务的设备不再重复创建。
pub async fn create_ota_tasks_batch(
    State(db): State<Db>,
    tenant: axum::Extension<String>,
    Json(req): Json<BatchOtaReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let firmware_id: i64 = req
        .firmware_id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid firmware id".into()))?;
    if !(1..=100).contains(&req.rollout_percent) {
        return Err((
            StatusCode::BAD_REQUEST,
            "rollout_percent must be 1..=100".into(),
        ));
    }
    let owned =
        db.0.query_with(
            "SELECT id FROM ota_firmwares WHERE id = ? AND tenant_id = ?",
            &[json!(firmware_id), json!(tenant.as_str())],
        )
        .await
        .map_err(db_err)?;
    if owned.is_empty() {
        return Err((StatusCode::FORBIDDEN, "firmware not in tenant".into()));
    }

    let candidates: Vec<i64> = if req.device_ids.is_empty() {
        // 多取 1 行即可判定超限，避免把整租户设备载入内存
        db.0.query_with(
            &format!(
                "SELECT id FROM devices WHERE tenant_id = ? LIMIT {}",
                MAX_BATCH_DEVICES + 1
            ),
            &[json!(tenant.as_str())],
        )
        .await
        .map_err(db_err)?
        .iter()
        .filter_map(|r| r.get("id").and_then(Value::as_i64))
        .collect()
    } else {
        let mut seen = HashSet::new();
        let mut ids = Vec::with_capacity(req.device_ids.len());
        for s in &req.device_ids {
            let id: i64 = s
                .parse()
                .map_err(|_| (StatusCode::BAD_REQUEST, "invalid device id".into()))?;
            if seen.insert(id) {
                ids.push(id);
            }
        }
        if over_batch_limit(ids.len()) {
            return Err((
                StatusCode::BAD_REQUEST,
                "at most 1000 device_ids per request".into(),
            ));
        }
        // 显式 id 必须全部属于本租户，防止跨租户下发
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut params: Vec<Value> = vec![json!(tenant.as_str())];
        params.extend(ids.iter().map(|id| json!(id)));
        let owned_devices =
            db.0.query_with(
                &format!("SELECT id FROM devices WHERE tenant_id = ? AND id IN ({placeholders})"),
                &params,
            )
            .await
            .map_err(db_err)?;
        if owned_devices.len() != ids.len() {
            return Err((StatusCode::FORBIDDEN, "device not in tenant".into()));
        }
        ids
    };
    if candidates.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no devices in tenant".into()));
    }
    // 租户全量路径同样限流，避免超出 MySQL 占位符上限或超大单条 INSERT
    if over_batch_limit(candidates.len()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "too many tenant devices; pass device_ids in pages".into(),
        ));
    }

    let selected: Vec<i64> = candidates
        .iter()
        .copied()
        .filter(|id| rollout_selected(&id.to_string(), req.rollout_percent))
        .collect();
    let selected_count = selected.len();

    // 已有本固件任务的设备视为已下发，跳过以支持比例上调后的增量下发
    let mut already: HashSet<i64> = HashSet::new();
    if !selected.is_empty() {
        let placeholders = vec!["?"; selected.len()].join(",");
        let mut params: Vec<Value> = vec![json!(tenant.as_str()), json!(firmware_id)];
        params.extend(selected.iter().map(|id| json!(id)));
        let rows =
            db.0.query_with(
                &format!(
                    "SELECT device_id FROM ota_upgrade_tasks \
                     WHERE tenant_id = ? AND firmware_id = ? AND device_id IN ({placeholders})"
                ),
                &params,
            )
            .await
            .map_err(db_err)?;
        already = rows
            .iter()
            .filter_map(|r| r.get("device_id").and_then(Value::as_i64))
            .collect();
    }
    // shortcut: 查重与插入非事务且无唯一键，并发同批次可能重复建任务；需强一致时加 (tenant_id, device_id, firmware_id) 唯一键
    let to_create: Vec<i64> = selected
        .into_iter()
        .filter(|id| !already.contains(id))
        .collect();
    let skipped = already.len();

    if !to_create.is_empty() {
        // 单条多行 INSERT：id 由 next_id 预生成，保持与原单条端点一致的形态
        let mut sql = String::from(
            "INSERT INTO ota_upgrade_tasks (id, tenant_id, device_id, firmware_id, status) VALUES ",
        );
        let mut params: Vec<Value> = Vec::with_capacity(to_create.len() * 4);
        for (i, device_id) in to_create.iter().enumerate() {
            if i > 0 {
                sql.push(',');
            }
            sql.push_str("(?, ?, ?, ?, 'pending')");
            params.push(json!(ecat::ids::next_id()));
            params.push(json!(tenant.as_str()));
            params.push(json!(device_id));
            params.push(json!(firmware_id));
        }
        db.0.execute_with(&sql, &params).await.map_err(db_err)?;
    }

    // 同一固件只签名一次，失败也不影响任务创建
    let download_url = crate::ota_dist::download_url_for(&db, tenant.as_str(), firmware_id).await;
    Ok(Json(json!({
        "created": to_create.len(),
        "skipped": skipped,
        "selected": selected_count,
        "total": candidates.len(),
        "download_url": download_url,
    })))
}

/// POST /api/ota/tasks/{id}/rollback：失败任务回滚到该设备上一个成功版本。
pub async fn rollback_ota_task(
    State(db): State<Db>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let id_n: i64 = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid ota task id".into()))?;
    let rows =
        db.0.query_with(
            "SELECT device_id, firmware_id, status FROM ota_upgrade_tasks \
             WHERE id = ? AND tenant_id = ?",
            &[json!(id_n), json!(tenant.as_str())],
        )
        .await
        .map_err(db_err)?;
    let row = rows
        .first()
        .ok_or((StatusCode::NOT_FOUND, "ota task not found".to_string()))?;
    let device_id = row
        .get("device_id")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let firmware_id = row
        .get("firmware_id")
        .and_then(Value::as_i64)
        .unwrap_or_default();
    if row.get("status").and_then(Value::as_str) != Some("failed") {
        return Err((
            StatusCode::BAD_REQUEST,
            "only failed tasks can be rolled back".into(),
        ));
    }

    let prev =
        db.0.query_with(
            "SELECT firmware_id FROM ota_upgrade_tasks \
             WHERE tenant_id = ? AND device_id = ? AND status = 'success' AND firmware_id <> ? \
             ORDER BY updated_at DESC, id DESC LIMIT 1",
            &[json!(tenant.as_str()), json!(device_id), json!(firmware_id)],
        )
        .await
        .map_err(db_err)?;
    let Some(prev_id) = prev
        .first()
        .and_then(|r| r.get("firmware_id"))
        .and_then(Value::as_i64)
    else {
        return Err((
            StatusCode::BAD_REQUEST,
            "no previous successful firmware".into(),
        ));
    };

    let exists =
        db.0.query_with(
            "SELECT id FROM ota_firmwares WHERE id = ? AND tenant_id = ?",
            &[json!(prev_id), json!(tenant.as_str())],
        )
        .await
        .map_err(db_err)?;
    if exists.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "previous firmware no longer exists".into(),
        ));
    }

    let active =
        db.0.query_with(
            "SELECT id FROM ota_upgrade_tasks WHERE tenant_id = ? AND device_id = ? \
             AND firmware_id = ? AND status IN ('pending','downloading','installing') LIMIT 1",
            &[json!(tenant.as_str()), json!(device_id), json!(prev_id)],
        )
        .await
        .map_err(db_err)?;
    if !active.is_empty() {
        return Err((StatusCode::CONFLICT, "rollback already in progress".into()));
    }

    // shortcut: 活跃回滚检查与插入非事务，并发回滚可能建出两条 pending；需强一致时加唯一键或事务
    let new_id = ecat::ids::next_id();
    db.0.execute_with(
        "INSERT INTO ota_upgrade_tasks (id, tenant_id, device_id, firmware_id, status) \
             VALUES (?, ?, ?, ?, 'pending')",
        &[
            json!(new_id),
            json!(tenant.as_str()),
            json!(device_id),
            json!(prev_id),
        ],
    )
    .await
    .map_err(db_err)?;
    Ok(Json(json!({
        "id": new_id.to_string(),
        "device_id": device_id.to_string(),
        "firmware_id": prev_id.to_string(),
        "status": "pending",
    })))
}

#[cfg(test)]
mod tests {
    use super::{MAX_BATCH_DEVICES, over_batch_limit, rollout_selected};

    #[test]
    fn batch_size_cap() {
        assert!(!over_batch_limit(0));
        assert!(!over_batch_limit(MAX_BATCH_DEVICES));
        assert!(over_batch_limit(MAX_BATCH_DEVICES + 1));
    }

    #[test]
    fn rollout_percent_extremes() {
        for id in ["1", "42", "device-42", "999999999999999999"] {
            assert!(rollout_selected(id, 100));
            assert!(!rollout_selected(id, 0));
        }
    }

    #[test]
    fn rollout_deterministic() {
        for id in ["1", "42", "device-42"] {
            assert_eq!(rollout_selected(id, 37), rollout_selected(id, 37));
        }
    }

    #[test]
    fn rollout_distribution_sanity() {
        let count = (1..=10_000)
            .filter(|n| rollout_selected(&n.to_string(), 10))
            .count();
        assert!(
            (500..=1500).contains(&count),
            "selected {count}/10000 at 10%"
        );
    }
}
