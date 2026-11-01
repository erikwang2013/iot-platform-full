//! 场景与站内通知的存储访问（RuleStore 扩展 impl）。
//! 所有查询按 tenant_id 隔离；时间列与 JSON 列 CAST 成文本
//! （sqlx Any 不支持 Timestamp/Json）。

use crate::models::{NewScene, Notification, Scene};
use crate::scenes::validate_scene;
use ecat_data::Row;
use ecat_data_sqlx::SqlxClient;
use serde_json::{Value, json};

pub async fn insert_notification_row(
    db: &SqlxClient,
    tenant_id: &str,
    title: &str,
    message: &str,
    source: &str,
) -> Result<(), String> {
    let id = ecat::ids::next_id();
    db.execute_with(
        "INSERT INTO notifications (id, tenant_id, title, message, source) VALUES (?, ?, ?, ?, ?)",
        &[
            json!(id),
            json!(tenant_id),
            json!(title),
            json!(message),
            json!(source),
        ],
    )
    .await
    .map_err(|e| format!("insert notification: {e}"))?;
    Ok(())
}

const SCENE_COLUMNS: &str = "id, tenant_id, name, \
     CAST(enabled AS SIGNED) AS enabled, \
     CAST(conditions_json AS CHAR) AS conditions_json, \
     CAST(actions_json AS CHAR) AS actions_json, \
     CAST(last_triggered_at AS CHAR) AS last_triggered_at, \
     CAST(created_at AS CHAR) AS created_at, \
     CAST(updated_at AS CHAR) AS updated_at";

impl crate::store::RuleStore {
    pub async fn list_scenes(
        &self,
        tenant_id: &str,
        enabled_only: bool,
    ) -> Result<std::sync::Arc<Vec<Scene>>, String> {
        if let Some(scenes) = self.cache.get_scenes(tenant_id, enabled_only) {
            return Ok(scenes);
        }
        let (sql, params) = if enabled_only {
            (
                format!(
                    "SELECT {SCENE_COLUMNS} FROM scenes WHERE tenant_id = ? AND enabled = 1 ORDER BY created_at DESC"
                ),
                vec![json!(tenant_id)],
            )
        } else {
            (
                format!(
                    "SELECT {SCENE_COLUMNS} FROM scenes WHERE tenant_id = ? ORDER BY created_at DESC"
                ),
                vec![json!(tenant_id)],
            )
        };
        let rows = self
            .db
            .query_with(&sql, &params)
            .await
            .map_err(|e| format!("list scenes: {e}"))?;
        let scenes: Vec<Scene> = rows.iter().map(scene_from_row).collect();
        self.cache.put_scenes(tenant_id, enabled_only, &scenes);
        Ok(std::sync::Arc::new(scenes))
    }

    /// id 非平台主键时视为不存在（避免把解析错误暴露成 500）。
    pub async fn get_scene(&self, tenant_id: &str, id: &str) -> Result<Option<Scene>, String> {
        let Ok(id) = id.parse::<i64>() else {
            return Ok(None);
        };
        let rows = self
            .db
            .query_with(
                &format!("SELECT {SCENE_COLUMNS} FROM scenes WHERE id = ? AND tenant_id = ?"),
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("get scene: {e}"))?;
        Ok(rows.first().map(scene_from_row))
    }

    /// 场景引用的设备必须全部属于该租户（create/update/execute 共用）。
    /// Ok(false) = 至少一个设备不属于该租户；Err = DB 失败。
    pub async fn ensure_devices_owned(&self, tenant_id: &str, ids: &[i64]) -> Result<bool, String> {
        if ids.is_empty() {
            return Ok(true);
        }
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        let placeholders = vec!["?"; ids.len()].join(",");
        let mut params = Vec::with_capacity(ids.len() + 1);
        params.push(json!(tenant_id));
        params.extend(ids.iter().map(|id| json!(id)));
        let rows = self
            .db
            .query_with(
                &format!("SELECT id FROM devices WHERE tenant_id = ? AND id IN ({placeholders})"),
                &params,
            )
            .await
            .map_err(|e| format!("check device ownership: {e}"))?;
        Ok(rows.len() == ids.len())
    }

    pub async fn insert_scene(&self, tenant_id: &str, s: &NewScene) -> Result<Scene, String> {
        validate_scene(s)?;
        let id = ecat::ids::next_id();
        self.db
            .execute_with(
                "INSERT INTO scenes (id, tenant_id, name, enabled, conditions_json, actions_json) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                &[
                    json!(id),
                    json!(tenant_id),
                    json!(s.name),
                    json!(s.enabled as i64),
                    json!(s.conditions.to_string()),
                    json!(s.actions.to_string()),
                ],
            )
            .await
            .map_err(|e| format!("insert scene: {e}"))?;
        self.cache.invalidate_scenes(tenant_id);
        self.get_scene(tenant_id, &id.to_string())
            .await?
            .ok_or_else(|| "scene vanished after insert".into())
    }

    pub async fn update_scene(
        &self,
        tenant_id: &str,
        id: &str,
        s: &NewScene,
    ) -> Result<bool, String> {
        validate_scene(s)?;
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        let n = self
            .db
            .execute_with(
                "UPDATE scenes SET name = ?, enabled = ?, conditions_json = ?, actions_json = ? \
                 WHERE id = ? AND tenant_id = ?",
                &[
                    json!(s.name),
                    json!(s.enabled as i64),
                    json!(s.conditions.to_string()),
                    json!(s.actions.to_string()),
                    json!(id),
                    json!(tenant_id),
                ],
            )
            .await
            .map_err(|e| format!("update scene: {e}"))?;
        self.cache.invalidate_scenes(tenant_id);
        if n > 0 {
            return Ok(true);
        }
        // MySQL 对“值未变化”返回 0 affected rows，需要再查存在性区分 404
        self.get_scene(tenant_id, &id.to_string())
            .await
            .map(|s| s.is_some())
    }

    pub async fn delete_scene(&self, tenant_id: &str, id: &str) -> Result<bool, String> {
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        let n = self
            .db
            .execute_with(
                "DELETE FROM scenes WHERE id = ? AND tenant_id = ?",
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("delete scene: {e}"))?;
        self.cache.invalidate_scenes(tenant_id);
        Ok(n > 0)
    }

    /// 触发时间戳为尽力而为（失败不影响动作执行结果）。
    pub async fn touch_scene_triggered(&self, tenant_id: &str, id: &str) -> Result<(), String> {
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        self.db
            .execute_with(
                "UPDATE scenes SET last_triggered_at = NOW() WHERE id = ? AND tenant_id = ?",
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("touch scene: {e}"))?;
        Ok(())
    }

    pub async fn list_notifications(
        &self,
        tenant_id: &str,
        unread: bool,
    ) -> Result<Vec<Notification>, String> {
        let (sql, params) = if unread {
            (
                "SELECT id, tenant_id, title, message, source, CAST(read_at AS CHAR) AS read_at, \
                 CAST(created_at AS CHAR) AS created_at FROM notifications \
                 WHERE tenant_id = ? AND read_at IS NULL ORDER BY created_at DESC LIMIT 100",
                vec![json!(tenant_id)],
            )
        } else {
            (
                "SELECT id, tenant_id, title, message, source, CAST(read_at AS CHAR) AS read_at, \
                 CAST(created_at AS CHAR) AS created_at FROM notifications \
                 WHERE tenant_id = ? ORDER BY created_at DESC LIMIT 100",
                vec![json!(tenant_id)],
            )
        };
        let rows = self
            .db
            .query_with(sql, &params)
            .await
            .map_err(|e| format!("list notifications: {e}"))?;
        Ok(rows.iter().map(notification_from_row).collect())
    }

    /// 已读幂等：已读记录仍返回 true（区分“已读”与“不存在”）。
    pub async fn mark_notification_read(&self, tenant_id: &str, id: &str) -> Result<bool, String> {
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        let n = self
            .db
            .execute_with(
                "UPDATE notifications SET read_at = NOW() WHERE id = ? AND tenant_id = ? AND read_at IS NULL",
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("mark notification read: {e}"))?;
        if n > 0 {
            return Ok(true);
        }
        let rows = self
            .db
            .query_with(
                "SELECT id FROM notifications WHERE id = ? AND tenant_id = ?",
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("get notification: {e}"))?;
        Ok(!rows.is_empty())
    }

    pub async fn insert_notification(
        &self,
        tenant_id: &str,
        title: &str,
        message: &str,
        source: &str,
    ) -> Result<(), String> {
        insert_notification_row(&self.db, tenant_id, title, message, source).await
    }
}

fn str_col(r: &Row, col: &str) -> String {
    r.get(col)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn opt_str(r: &Row, col: &str) -> Option<String> {
    r.get(col).and_then(Value::as_str).map(str::to_string)
}

fn json_col(r: &Row, col: &str) -> Value {
    r.get(col)
        .and_then(Value::as_str)
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}

fn bool_col(r: &Row, col: &str) -> bool {
    match r.get(col) {
        Some(Value::Bool(b)) => *b,
        Some(v) => v.as_i64().unwrap_or(0) != 0,
        None => false,
    }
}

fn scene_from_row(r: &Row) -> Scene {
    Scene {
        id: crate::store::id_str(r, "id"),
        tenant_id: str_col(r, "tenant_id"),
        name: str_col(r, "name"),
        enabled: bool_col(r, "enabled"),
        conditions: json_col(r, "conditions_json"),
        actions: json_col(r, "actions_json"),
        last_triggered_at: opt_str(r, "last_triggered_at"),
        created_at: str_col(r, "created_at"),
        updated_at: str_col(r, "updated_at"),
    }
}

fn notification_from_row(r: &Row) -> Notification {
    Notification {
        id: crate::store::id_str(r, "id"),
        tenant_id: str_col(r, "tenant_id"),
        title: str_col(r, "title"),
        message: str_col(r, "message"),
        source: str_col(r, "source"),
        read_at: opt_str(r, "read_at"),
        created_at: str_col(r, "created_at"),
    }
}
