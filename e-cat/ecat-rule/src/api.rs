use crate::models::{NewNotifyChannel, NewRule, NewScene, Scene};
use crate::store::RuleStore;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiState {
    pub store: Arc<RuleStore>,
    /// 场景动作（指令）需要回源 iot-access；Client 克隆廉价
    pub http: reqwest::Client,
}

#[derive(Deserialize)]
pub struct AlertQuery {
    pub status: Option<String>,
}

/// 受保护路由（挂载见 main.rs，路径前缀 /api/rule，租户经 tenant_from_header 写入 extensions）。
pub fn router(api: ApiState) -> axum::Router {
    axum::Router::new()
        .route("/rules", axum::routing::get(list_rules).post(create_rule))
        .route("/rules/{id}", axum::routing::put(update_rule).delete(delete_rule))
        .route("/alerts", axum::routing::get(list_alerts))
        .route("/alerts/{id}/ack", axum::routing::post(ack_alert))
        .route("/stats", axum::routing::get(alert_stats))
        .route(
            "/channels/{channel}",
            axum::routing::put(upsert_channel).delete(delete_channel),
        )
        .route("/channels", axum::routing::get(list_channels))
        .route("/reports", axum::routing::get(list_reports))
        .route(
            "/scenes",
            axum::routing::get(list_scenes).post(create_scene),
        )
        .route(
            "/scenes/{id}",
            axum::routing::put(update_scene).delete(delete_scene),
        )
        .route("/scenes/{id}/execute", axum::routing::post(execute_scene))
        .route("/notifications", axum::routing::get(list_notifications))
        .route(
            "/notifications/{id}/read",
            axum::routing::post(mark_notification_read),
        )
        .with_state(api)
}

#[derive(Deserialize)]
pub struct ReportQuery {
    pub date: Option<String>,
}

/// GET /api/rule/reports?date=YYYY-MM-DD：本租户每日汇总报表列表
/// （倒序最多 30 条；date 可选过滤）。报表由定时任务生成（见 report.rs）。
pub async fn list_reports(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Query(q): Query<ReportQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let date = q.date.as_deref().filter(|d| !d.is_empty());
    let reports = crate::report::list_reports(&api.store.db, &tenant, date)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "reports": reports })))
}

/// GET /api/rule/rules
pub async fn list_rules(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
) -> Result<Json<Vec<crate::models::Rule>>, (StatusCode, String)> {
    let rules = api
        .store
        .list_rules(&tenant)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(rules))
}

/// POST /api/rule/rules
pub async fn create_rule(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Json(body): Json<NewRule>,
) -> Result<(StatusCode, Json<crate::models::Rule>), (StatusCode, String)> {
    let rule = api
        .store
        .insert_rule(&tenant, &body)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok((StatusCode::CREATED, Json(rule)))
}

/// PUT /api/rule/rules/{id}
pub async fn update_rule(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
    Json(body): Json<NewRule>,
) -> Result<Json<crate::models::Rule>, (StatusCode, String)> {
    let ok = api
        .store
        .update_rule(&tenant, &id, &body)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    if !ok {
        return Err((StatusCode::NOT_FOUND, "rule not found".into()));
    }
    let rules = api
        .store
        .list_rules(&tenant)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    rules
        .into_iter()
        .find(|r| r.id == id)
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "rule not found".into()))
}

/// DELETE /api/rule/rules/{id}
pub async fn delete_rule(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ok = api
        .store
        .delete_rule(&tenant, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err((StatusCode::NOT_FOUND, "rule not found".into())) }
}

/// GET /api/rule/channels：通知渠道列表。
pub async fn list_channels(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
) -> Result<Json<Vec<crate::models::NotifyChannel>>, (StatusCode, String)> {
    let channels = api
        .store
        .list_channels(&tenant)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(channels))
}

/// PUT /api/rule/channels/{channel}：创建或更新（单租户单渠道唯一）。
pub async fn upsert_channel(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(channel): Path<String>,
    Json(body): Json<NewNotifyChannel>,
) -> Result<Json<crate::models::NotifyChannel>, (StatusCode, String)> {
    let ch = api
        .store
        .upsert_channel(&tenant, &channel, &body)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    Ok(Json(ch))
}

/// DELETE /api/rule/channels/{channel}
pub async fn delete_channel(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(channel): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ok = api
        .store
        .delete_channel(&tenant, &channel)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err((StatusCode::NOT_FOUND, "channel not found".into())) }
}

/// GET /api/rule/stats：告警总数 + 未处理数。
pub async fn alert_stats(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let (total, active) = api
        .store
        .stats(&tenant)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "total": total, "active": active })))
}

/// GET /api/rule/alerts?status=active|acknowledged
pub async fn list_alerts(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Query(q): Query<AlertQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let status = q.status.as_deref().filter(|s| !s.is_empty());
    if let Some(s) = status {
        if !["active", "acknowledged"].contains(&s) {
            return Err((StatusCode::BAD_REQUEST, "status must be active|acknowledged".into()));
        }
    }
    let alerts = api
        .store
        .list_alerts(&tenant, status)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "alerts": alerts })))
}

/// POST /api/rule/alerts/{id}/ack
pub async fn ack_alert(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let ok = api
        .store
        .ack_alert(&tenant, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if ok { Ok(StatusCode::NO_CONTENT) } else { Err((StatusCode::NOT_FOUND, "alert not found".into())) }
}

/// 路径 id 必须是平台主键：格式错误 → 400（其余错误才走 500）。
fn validate_id(id: &str) -> Result<(), (StatusCode, String)> {
    id.parse::<i64>()
        .map(|_| ())
        .map_err(|_| (StatusCode::BAD_REQUEST, "id must be a platform id".into()))
}

/// 场景引用的设备必须属于当前租户：越权 403，DB 失败 500。
async fn check_scene_devices(
    api: &ApiState,
    tenant: &str,
    conditions: &Value,
    actions: &Value,
) -> Result<(), (StatusCode, String)> {
    let ids = crate::scenes::extract_device_ids(conditions, actions);
    match api.store.ensure_devices_owned(tenant, &ids).await {
        Ok(true) => Ok(()),
        Ok(false) => Err((StatusCode::FORBIDDEN, "device not in tenant".into())),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// GET /api/rule/scenes
pub async fn list_scenes(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let scenes = api
        .store
        .list_scenes(&tenant, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "scenes": scenes })))
}

/// POST /api/rule/scenes
pub async fn create_scene(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Json(body): Json<NewScene>,
) -> Result<(StatusCode, Json<Scene>), (StatusCode, String)> {
    crate::scenes::validate_scene(&body).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    check_scene_devices(&api, &tenant, &body.conditions, &body.actions).await?;
    let scene = api
        .store
        .insert_scene(&tenant, &body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok((StatusCode::CREATED, Json(scene)))
}

/// PUT /api/rule/scenes/{id}
pub async fn update_scene(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
    Json(body): Json<NewScene>,
) -> Result<Json<Scene>, (StatusCode, String)> {
    validate_id(&id)?;
    crate::scenes::validate_scene(&body).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    check_scene_devices(&api, &tenant, &body.conditions, &body.actions).await?;
    let ok = api
        .store
        .update_scene(&tenant, &id, &body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if !ok {
        return Err((StatusCode::NOT_FOUND, "scene not found".into()));
    }
    api.store
        .get_scene(&tenant, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "scene not found".into()))
}

/// DELETE /api/rule/scenes/{id}
pub async fn delete_scene(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_id(&id)?;
    let ok = api
        .store
        .delete_scene(&tenant, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "scene not found".into()))
    }
}

/// POST /api/rule/scenes/{id}/execute：立即执行动作（disabled 场景也可手动执行）。
pub async fn execute_scene(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    validate_id(&id)?;
    let scene = api
        .store
        .get_scene(&tenant, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
        .ok_or((StatusCode::NOT_FOUND, "scene not found".into()))?;
    check_scene_devices(&api, &tenant, &scene.conditions, &scene.actions).await?;
    let results = crate::scenes::execute_scene(&scene, &api.http, &api.store.db, &tenant).await;
    Ok(Json(
        serde_json::json!({ "executed": results.len(), "results": results }),
    ))
}

#[derive(Deserialize)]
pub struct NotificationQuery {
    pub unread: Option<i64>,
}

/// GET /api/rule/notifications?unread=1：本租户通知，倒序最多 100 条。
pub async fn list_notifications(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Query(q): Query<NotificationQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let notifications = api
        .store
        .list_notifications(&tenant, q.unread == Some(1))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    Ok(Json(serde_json::json!({ "notifications": notifications })))
}

/// POST /api/rule/notifications/{id}/read
pub async fn mark_notification_read(
    State(api): State<ApiState>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    validate_id(&id)?;
    let ok = api
        .store
        .mark_notification_read(&tenant, &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;
    if ok {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "notification not found".into()))
    }
}
