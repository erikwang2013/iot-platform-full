use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, post, put},
};
use ecat_data_sqlx::SqlxClient;
use ecat_device::{
    Db, MAX_FIRMWARE_SIZE, create_firmware, create_ota_task, delete_device, delete_firmware,
    device_stats, download_firmware, download_firmware_public,
    groups::{
        add_members, batch_devices, create_group, delete_group, list_device_tags, list_groups,
        remove_members,
    },
    list_devices, list_firmwares, list_ota_tasks, migrate,
    ota_dist::{firmware_downloads, signed_firmware_url},
    ota_rollout::{create_ota_tasks_batch, rollback_ota_task},
    report_ota_progress, unbind_device, update_device, upload_firmware,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 网关租户门与公开下载 token 都依赖该 secret：缺失直接启动失败，禁止静默降级
    std::env::var("IOT_GATEWAY_SECRET").map_err(|_| "IOT_GATEWAY_SECRET not set".to_string())?;

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into());
    let db = SqlxClient::connect(&db_url).await?;
    migrate(&db).await?;
    let db = Arc::new(db);

    let health_router = ecat_health::HealthRegistry::new()
        .with_check(ecat_health::db_check(db.clone()))
        .into_router();

    // 受保护路由：需网关 secret + x-tenant-id（与 ecat-access/rule/data 一致）
    let devices = Router::new()
        .route("/", get(list_devices))
        .route("/stats", get(device_stats))
        .route("/groups", get(list_groups).post(create_group))
        .route("/groups/{id}", delete(delete_group))
        .route("/groups/{id}/members", post(add_members).delete(remove_members))
        .route("/batch", post(batch_devices))
        .route("/{id}", put(update_device).delete(delete_device))
        .route("/{id}/unbind", post(unbind_device))
        .route("/{id}/tags", get(list_device_tags))
        .with_state(Db(db.clone()));
    let ota = Router::new()
        .route("/firmwares", get(list_firmwares).post(create_firmware))
        .route(
            "/firmwares/upload",
            post(upload_firmware).layer(DefaultBodyLimit::max(MAX_FIRMWARE_SIZE)),
        )
        .route("/firmwares/{id}", delete(delete_firmware))
        .route("/firmwares/{id}/download", get(download_firmware))
        .route("/firmwares/{id}/signed-url", post(signed_firmware_url))
        .route("/firmwares/{id}/downloads", get(firmware_downloads))
        .route("/tasks", get(list_ota_tasks).post(create_ota_task))
        .route("/tasks/batch", post(create_ota_tasks_batch))
        .route("/tasks/{id}/report", post(report_ota_progress))
        .route("/tasks/{id}/rollback", post(rollback_ota_task))
        .with_state(Db(db.clone()));
    // 公开下载：token 即凭据（设备不持有网关 secret），不套租户门
    let ota_public = Router::new()
        .route(
            "/api/ota/public/firmwares/{id}/download",
            get(download_firmware_public),
        )
        .with_state(Db(db));
    let protected = Router::new()
        .nest("/api/devices", devices)
        .nest("/api/ota", ota)
        .layer(middleware::from_fn(ecat_middleware::tenant_from_header));

    // C-3 Prometheus：/metrics 公开（scrape 端点），MetricsLayer 记请求数/时延/状态码
    let router = Router::new()
        .merge(health_router)
        .merge(protected)
        .merge(ota_public)
        .merge(ecat_metrics::metrics_router())
        .layer(ecat_metrics::MetricsLayer::new());

    let srv = ecat_transport_http::HttpServer::new("0.0.0.0:8081").router(router);
    let mut app = ecat::App::builder()
        .name("iot-device")
        .version("0.1.0")
        .server(srv)
        .build()?;
    app.run().await?;
    Ok(())
}
