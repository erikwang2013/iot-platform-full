use axum::{Router, middleware};
use ecat_access::{
    api::{self, ApiState},
    auth::{self, AuthState},
    console::{self, ConsoleState},
    oauth::{self, OauthState},
    store::Store,
    webhook::{self, WebhookState},
};
use ecat_data::RdbmsClient;
use ecat_data_redis::RedisCache;
use ecat_data_sqlx::SqlxClient;
use ecat_mq_kafka::{KafkaConfig, KafkaMq};
use ecat_mq_mqtt::{MqttConfig, MqttMq};
use std::sync::Arc;

async fn migrate(db: &SqlxClient) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 编译期 include_str! 内联，无运行时文件依赖（iot-device 亦经此读取单一副本）
    for sql in [
        include_str!("../migrations/0001_init.sql"),
        include_str!("../migrations/0002_vendor_auth.sql"),
        include_str!("../migrations/0003_platform.sql"),
        include_str!("../migrations/0004_audit.sql"),
        include_str!("../migrations/0005_groups.sql"),
        include_str!("../migrations/0006_api_keys.sql"),
        include_str!("../migrations/0007_command_queue.sql"),
        include_str!("../migrations/0008_ota_dist.sql"),
        include_str!("../migrations/0009_mqtt_credentials.sql"),
    ] {
        db.execute_script(sql).await?;
    }
    // 角色拼写统一：历史只读角色 readonly → read-only（JWT 签发/校验用后者）。
    db.execute_with("UPDATE users SET role = 'read-only' WHERE role = 'readonly'", &[])
        .await?;
    // api_keys.scope（C-5 开放写）：老库补列（MySQL 8 无 ADD COLUMN IF NOT
    // EXISTS，先查 information_schema；新库由 0006 CREATE TABLE 直接建）。
    let exists = db
        .query_with(
            "SELECT COUNT(*) AS n FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'api_keys' AND COLUMN_NAME = 'scope'",
            &[],
        )
        .await?;
    let n = exists
        .first()
        .and_then(|r| r.get("n"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    if n == 0 {
        db.execute_with(
            "ALTER TABLE api_keys ADD COLUMN scope VARCHAR(16) NOT NULL DEFAULT 'read'",
            &[],
        )
        .await?;
    }
    // vendor_credentials.key_version（密钥轮换，见 store.rs KeyRing）：老库补列
    let exists = db
        .query_with(
            "SELECT COUNT(*) AS n FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'vendor_credentials' \
             AND COLUMN_NAME = 'key_version'",
            &[],
        )
        .await?;
    let n = exists
        .first()
        .and_then(|r| r.get("n"))
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    if n == 0 {
        db.execute_with(
            "ALTER TABLE vendor_credentials ADD COLUMN key_version INT NOT NULL DEFAULT 1",
            &[],
        )
        .await?;
    }
    Ok(())
}

/// 首次启动播种初始账号：租户 tenant-1 + 管理员 admin。
/// 密码取 env ADMIN_INITIAL_PASSWORD；未设置则生成 16 位随机字母数字，
/// 用 tracing::warn! 打印一次（此后无法找回，需登录后修改）。
async fn seed_admin(store: &Store) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    store.ensure_tenant("tenant-1", "默认租户").await?;
    if store.find_user("admin").await?.is_some() {
        return Ok(());
    }
    let password = match std::env::var("ADMIN_INITIAL_PASSWORD") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            use rand::Rng;
            use rand::distributions::Alphanumeric;
            let generated: String = rand::thread_rng()
                .sample_iter(&Alphanumeric)
                .take(16)
                .map(char::from)
                .collect();
            tracing::warn!(
                password = %generated,
                "ADMIN_INITIAL_PASSWORD not set; generated initial admin password"
            );
            generated
        }
    };
    store
        .create_user(
            "tenant-1",
            "admin",
            &ecat_access::auth::hash_password(&password),
            "admin",
        )
        .await?;
    tracing::info!("seeded initial admin account: admin (tenant-1)");
    Ok(())
}

/// 从环境变量构造 MQTT 配置；安全配置不完整直接报错（fail-closed）。
/// `get` 注入变量读取，main 传 `std::env::var`，便于纯函数单测。
fn mqtt_config_from_env(get: impl Fn(&str) -> Option<String>) -> Result<MqttConfig, String> {
    let url = get("MQTT_URL").unwrap_or_else(|| "tcp://localhost:1883".into());
    // ssl:// / mqtts:// 隐含 TLS。
    let scheme_tls = url.starts_with("ssl://") || url.starts_with("mqtts://");
    let tls = match get("MQTT_TLS") {
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" if scheme_tls => {
                return Err("MQTT_TLS=0 conflicts with TLS URL scheme".into());
            }
            "0" | "false" | "no" | "off" => false,
            _ => return Err(format!("invalid MQTT_TLS value: {v}")),
        },
        None => scheme_tls,
    };
    let (username, password) = (get("MQTT_USERNAME"), get("MQTT_PASSWORD"));
    if username.is_some() != password.is_some() {
        return Err("MQTT_USERNAME and MQTT_PASSWORD must be set together".into());
    }
    let (cert_file, key_file) = (get("MQTT_CERT_FILE"), get("MQTT_KEY_FILE"));
    if cert_file.is_some() != key_file.is_some() {
        return Err("MQTT_CERT_FILE and MQTT_KEY_FILE must be set together".into());
    }
    let ca_file = get("MQTT_CA_FILE");
    Ok(MqttConfig {
        url,
        client_id: get("MQTT_CLIENT_ID"),
        username,
        password,
        // 配了 CA / 客户端证书即按 TLS 连接，避免证书被静默忽略走明文。
        tls: tls || ca_file.is_some() || cert_file.is_some(),
        ca_file,
        cert_file,
        key_file,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 安全相关变量缺失直接启动失败，禁止静默降级（oauth state 签名密钥 / 租户门）
    let enc_key = std::env::var("IOT_CRED_ENCRYPT_KEY")
        .map_err(|_| "IOT_CRED_ENCRYPT_KEY not set".to_string())?;
    std::env::var("IOT_GATEWAY_SECRET").map_err(|_| "IOT_GATEWAY_SECRET not set".to_string())?;

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into());
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());
    let kafka_brokers = std::env::var("KAFKA_BROKERS").unwrap_or_else(|_| "localhost:9092".into());
    let mqtt_cfg = mqtt_config_from_env(|k| std::env::var(k).ok())?;
    let tuya_client_id =
        std::env::var("TUYA_CLIENT_ID").unwrap_or_else(|_| "dev-tuya-client-id".into());
    let miot_client_id =
        std::env::var("MIOT_CLIENT_ID").unwrap_or_else(|_| "dev-miot-client-id".into());
    // 测试/演示指向 mock：export TUYA_OPENAPI_BASE=http://127.0.0.1:18084
    // TUYA_CLIENT_SECRET / MIOT_CLIENT_SECRET 由各 exchange 函数读取

    let db = SqlxClient::connect(&db_url).await?;
    migrate(&db).await?;
    let db = Arc::new(db);
    let redis = Arc::new(RedisCache::connect(&redis_url).await?);
    // 离线巡检多副本互斥锁：独立 Redis 连接（SET NX PX），与缓存同用 REDIS_URL。
    let patrol_lock = Arc::new(ecat_data_redis::RedisLock::connect(&redis_url).await?);
    let kafka = Arc::new(KafkaMq::connect(&kafka_brokers).await?);
    // 指令消费独立固定组：多副本共享分区（不重复下发），auto_commit 支持重启续传。
    let command_kafka = Arc::new(
        KafkaMq::from_config(KafkaConfig {
            brokers: kafka_brokers.clone(),
            group_id: Some("iot-commands".into()),
            auto_commit: true,
            security_protocol: None,
            sasl_mechanism: None,
            sasl_username: None,
            sasl_password: None,
        })
        .await?,
    );
    let mqtt = Arc::new(MqttMq::from_config(mqtt_cfg).await?);
    let store = Arc::new(Store::new(db.clone(), &enc_key));
    seed_admin(&store).await?;

    let callback_base = std::env::var("ACCESS_CALLBACK_BASE")
        .unwrap_or_else(|_| "http://localhost:8080/api/access/oauth/callback".into());

    let oauth_state = OauthState {
        store: store.clone(),
        tuya_client_id,
        miot_client_id,
        callback_base,
    };
    let api_state = ApiState {
        store: store.clone(),
        kafka: kafka.clone(),
        redis: redis.clone(),
        mqtt: mqtt.clone(),
        // B-2 熔断器：每厂商一个实例（惰性创建并共享；空映射起步）
        breakers: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
    };
    let webhook_state = WebhookState {
        store: store.clone(),
        kafka: kafka.clone(),
        redis: redis.clone(),
    };
    let auth_state = AuthState {
        store: store.clone(),
        jwt_secret: std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "dev-secret-key-0123456789abcdefghijklmn".into()),
        token_ttl_secs: std::env::var("JWT_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24 * 3600),
    };
    let console_state = ConsoleState {
        store: store.clone(),
    };

    // 后台任务：直连 MQTT 订阅
    let (mqtt_run_mqtt, mqtt_run_store, mqtt_run_redis, mqtt_run_kafka) =
        (mqtt.clone(), store.clone(), redis.clone(), kafka.clone());
    tokio::spawn(async move {
        ecat_access::mqtt::run(
            mqtt_run_mqtt,
            mqtt_run_store,
            mqtt_run_redis,
            mqtt_run_kafka,
        )
        .await;
    });

    // 定时任务：设备离线巡检（B-1）——心跳超时（默认 5 分钟）未上报则标记
    // offline + 发 offline 事件。每 60s 巡检一次（OFFLINE_PATROL_SECS 可配）。
    let mut scheduler = ecat_scheduler::Scheduler::new();
    let patrol_interval = std::env::var("OFFLINE_PATROL_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60u64);
    ecat_access::offline::register(
        &mut scheduler,
        store.clone(),
        redis.clone(),
        kafka.clone(),
        patrol_lock,
        std::time::Duration::from_secs(patrol_interval),
    );
    tokio::spawn(async move { scheduler.run().await });

    // 后台任务：消费指令事件（D-3 联动）→ MQTT 下发目标设备
    let (cmd_kafka, cmd_mqtt, cmd_store) =
        (command_kafka.clone(), mqtt.clone(), store.clone());
    tokio::spawn(async move {
        ecat_access::command_consumer::run(cmd_kafka, cmd_mqtt, cmd_store).await;
    });

    // 公开路由：涂鸦 webhook、OAuth 回调、登录、开放 API 换 token
    // （浏览器跳回/登录前置/开放客户端，无 JWT/租户）
    let login = Router::new()
        .route("/login", axum::routing::post(auth::login))
        .with_state(auth_state.clone());
    let open = Router::new()
        .route("/open/token", axum::routing::post(auth::open_token))
        .with_state(auth_state);
    let public = Router::new()
        .merge(webhook::router(webhook_state))
        .merge(oauth::router_public(oauth_state.clone()))
        .merge(ecat_access::mqtt::auth_router(ecat_access::mqtt::MqttAuthState {
            store: store.clone(),
        }))
        .merge(open);
    // 受保护路由：需网关 secret + x-tenant-id
    let protected = Router::new()
        .merge(oauth::router(oauth_state))
        .merge(api::router(api_state))
        .merge(console::router(console_state))
        .layer(middleware::from_fn(ecat_middleware::tenant_from_header));

    let health_router = ecat_health::HealthRegistry::new()
        .with_check(ecat_health::db_check(db.clone()))
        .into_router();

    let router = Router::new()
        .merge(health_router)
        .nest("/api/access", public)
        .nest("/api/access", protected)
        // 登录（管理端 /api/auth/login 与客户端 /admin/auth/login）走网关节流，
        // 顶层挂载避开 /api/access 前缀
        .nest("/api/auth", login.clone())
        .nest("/admin/auth", login)
        // C-3 Prometheus：/metrics 公开（scrape 端点），MetricsLayer 记请求数/时延/状态码
        .merge(ecat_metrics::metrics_router())
        .layer(ecat_metrics::MetricsLayer::new());

    let bind = std::env::var("HTTP_BIND").unwrap_or_else(|_| "0.0.0.0:8082".into());
    let srv = ecat_transport_http::HttpServer::new(bind).router(router);
    let mut app = ecat::App::builder()
        .name("iot-access")
        .version("0.1.0")
        .server(srv)
        .build()?;
    app.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pairs: &[(&str, &str)]) -> Result<MqttConfig, String> {
        mqtt_config_from_env(|key| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_string())
        })
    }

    #[test]
    fn default_is_plaintext_tcp() {
        let cfg = parse(&[]).unwrap();
        assert_eq!(cfg.url, "tcp://localhost:1883");
        assert!(!cfg.tls);
        assert!(cfg.username.is_none() && cfg.password.is_none());
        assert!(cfg.ca_file.is_none() && cfg.cert_file.is_none() && cfg.key_file.is_none());
    }

    #[test]
    fn ssl_scheme_implies_tls() {
        assert!(parse(&[("MQTT_URL", "ssl://broker:8883")]).unwrap().tls);
        assert!(parse(&[("MQTT_URL", "mqtts://broker")]).unwrap().tls);
    }

    #[test]
    fn mqtt_tls_overrides_scheme() {
        assert!(parse(&[("MQTT_TLS", "1")]).unwrap().tls);
        // 显式关闭 TLS 不能覆盖 TLS scheme：拒绝启动而非静默明文。
        assert!(parse(&[("MQTT_URL", "ssl://broker:8883"), ("MQTT_TLS", "0")]).is_err());
        assert!(
            !parse(&[("MQTT_URL", "tcp://broker:1883"), ("MQTT_TLS", "0")])
                .unwrap()
                .tls
        );
        assert!(parse(&[("MQTT_TLS", "maybe")]).is_err());
    }

    #[test]
    fn username_without_password_fails() {
        assert!(parse(&[("MQTT_USERNAME", "u")]).is_err());
        assert!(parse(&[("MQTT_PASSWORD", "p")]).is_err());
        let cfg = parse(&[("MQTT_USERNAME", "u"), ("MQTT_PASSWORD", "p")]).unwrap();
        assert_eq!(cfg.username.as_deref(), Some("u"));
        assert_eq!(cfg.password.as_deref(), Some("p"));
    }

    #[test]
    fn cert_without_key_fails() {
        assert!(parse(&[("MQTT_CERT_FILE", "client.pem")]).is_err());
        assert!(parse(&[("MQTT_KEY_FILE", "client.key")]).is_err());
    }

    #[test]
    fn cert_files_imply_tls_without_mqtt_tls() {
        let cfg = parse(&[
            ("MQTT_CERT_FILE", "client.pem"),
            ("MQTT_KEY_FILE", "client.key"),
        ])
        .unwrap();
        assert!(cfg.tls);
    }

    #[test]
    fn full_mtls_config_parses_paths() {
        let cfg = parse(&[
            ("MQTT_URL", "ssl://broker:8883"),
            ("MQTT_CA_FILE", "/etc/iot/ca.pem"),
            ("MQTT_CERT_FILE", "/etc/iot/client.pem"),
            ("MQTT_KEY_FILE", "/etc/iot/client.key"),
            ("MQTT_CLIENT_ID", "iot-access-1"),
        ])
        .unwrap();
        assert!(cfg.tls);
        assert_eq!(cfg.ca_file.as_deref(), Some("/etc/iot/ca.pem"));
        assert_eq!(cfg.cert_file.as_deref(), Some("/etc/iot/client.pem"));
        assert_eq!(cfg.key_file.as_deref(), Some("/etc/iot/client.key"));
        assert_eq!(cfg.client_id.as_deref(), Some("iot-access-1"));
    }
}
