//! 需 docker：`docker compose up -d mysql` 后运行：
//! `DATABASE_URL=mysql://iot:iot@127.0.0.1:3306/iot \
//!  cargo test -p ecat-rule --test rule_flow -- --ignored --test-threads=1`
use ecat_data_sqlx::SqlxClient;
use ecat_rule::models::{EventMessage, NewNotifyChannel, NewRule, NewScene};
use ecat_rule::push::PushHub;
use ecat_rule::store::{RuleStore, migrate};
use serde_json::json;
use std::sync::Arc;

fn rule_with_action(action_value: serde_json::Value) -> NewRule {
    NewRule {
        name: "itest-temp".into(),
        device_id: "6755399441055744".into(),
        code: "temp".into(),
        operator: "gt".into(),
        threshold: 30.0,
        webhook_url: None,
        action_device_id: Some("6755399441055744".into()),
        action_code: Some("switch".into()),
        action_value: Some(action_value),
        enabled: Some(true),
    }
}

#[tokio::test]
#[ignore]
async fn engine_fires_rule_and_alert_reaches_store_and_hub() {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "mysql://iot:iot@localhost:3306/iot".into());
    let db = Arc::new(SqlxClient::connect(&url).await.unwrap());
    // rules/alert_records 外键引用 tenants（iot-access 迁移建表）；独立库先行建好父表
    db.execute(
        "CREATE TABLE IF NOT EXISTS tenants (
            id VARCHAR(36) PRIMARY KEY,
            name VARCHAR(255) NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        ) ENGINE = InnoDB",
    )
    .await
    .expect("create tenants");
    // rules.tenant_id 外键指向 tenants.id，插入规则前租户记录必须在库
    db.execute("INSERT IGNORE INTO tenants (id, name) VALUES ('itest-tenant', 'itest')")
        .await
        .expect("insert tenant");
    // 迁移可重复跑（先删引用方再删父表）
    for t in [
        "alert_records",
        "rules",
        "notify_channels",
        "scenes",
        "notifications",
        "daily_reports",
    ] {
        db.execute(&format!("DROP TABLE IF EXISTS {t}"))
            .await
            .expect("drop");
    }
    migrate(&db).await.expect("migrate");
    let store = Arc::new(RuleStore::new(db.clone()));
    let hub = PushHub::new();
    let mut rx = hub.subscribe("itest-tenant");

    // 建规则（temp > 30），action_value 为 JSON 列
    let rule = store
        .insert_rule("itest-tenant", &rule_with_action(json!({"power": true})))
        .await
        .expect("insert rule");

    // 不匹配事件 → 无告警
    let rules = store.list_rules("itest-tenant").await.unwrap();
    // MySQL JSON 列经文本写入 + CAST 读取后完整 round-trip
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].action_value, Some(json!({"power": true})));
    // 时间列 CAST 无别名时 header 是表达式文本，mapper 会读成空串
    assert!(!rules[0].created_at.is_empty());
    assert!(!rules[0].updated_at.is_empty());
    let low = EventMessage {
        device_id: "6755399441055744".into(),
        tenant_id: "itest-tenant".into(),
        kind: "property".into(),
        code: "temp".into(),
        value: json!(25.0),
        ts: 1_690_000_000_000,
    };
    assert!(ecat_rule::engine::evaluate(&low, &rules).is_empty());

    // 匹配事件 → evaluate 命中 → 推送 hub + 落告警记录
    let high = EventMessage {
        device_id: "6755399441055744".into(),
        tenant_id: "itest-tenant".into(),
        kind: "property".into(),
        code: "temp".into(),
        value: json!(35.0),
        ts: 1_690_000_000_001,
    };
    let msgs = ecat_rule::engine::evaluate(&high, &rules);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].rule_id, rule.id);
    hub.publish("itest-tenant", &msgs[0]);
    store.insert_alert(&msgs[0]).await.expect("insert alert");

    let got = rx.try_recv().unwrap();
    assert_eq!(got.value, json!(35.0));

    let alerts = store.list_alerts("itest-tenant", None).await.unwrap();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].status, "active");
    assert!(!alerts[0].created_at.is_empty());
    assert!(
        store
            .ack_alert("itest-tenant", &alerts[0].id)
            .await
            .unwrap()
    );
    let acked = store
        .list_alerts("itest-tenant", Some("acknowledged"))
        .await
        .unwrap();
    assert_eq!(acked.len(), 1);

    // update 走同一 JSON 文本绑定；标量 JSON 也要能读回
    assert!(
        store
            .update_rule("itest-tenant", &rule.id, &rule_with_action(json!("off")))
            .await
            .unwrap()
    );
    let rules = store.list_rules("itest-tenant").await.unwrap();
    assert_eq!(rules[0].action_value, Some(json!("off")));

    // notify_channels.config 同为 JSON 列
    store
        .upsert_channel(
            "itest-tenant",
            "dingtalk",
            &NewNotifyChannel {
                config: json!({"webhook_url": "https://example.com/hook"}),
                enabled: Some(true),
            },
        )
        .await
        .expect("upsert channel");
    let channels = store.list_channels("itest-tenant").await.unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(
        channels[0].config,
        json!({"webhook_url": "https://example.com/hook"})
    );
    assert!(!channels[0].created_at.is_empty());
    assert!(!channels[0].updated_at.is_empty());

    // notifications: 未读分支 -> 标记已读 -> 已读分支
    store
        .insert_notification("itest-tenant", "itest-title", "itest-message", "scene")
        .await
        .expect("insert notification");
    let unread = store
        .list_notifications("itest-tenant", true)
        .await
        .unwrap();
    assert_eq!(unread.len(), 1);
    assert!(unread[0].read_at.is_none());
    assert!(!unread[0].created_at.is_empty());
    assert!(
        store
            .mark_notification_read("itest-tenant", &unread[0].id)
            .await
            .unwrap()
    );
    assert!(
        store
            .list_notifications("itest-tenant", true)
            .await
            .unwrap()
            .is_empty()
    );
    let all = store
        .list_notifications("itest-tenant", false)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert!(!all[0].read_at.as_deref().unwrap_or_default().is_empty());

    // scenes.conditions_json / actions_json
    let scene = store
        .insert_scene(
            "itest-tenant",
            &NewScene {
                name: "itest-scene".into(),
                enabled: true,
                conditions: json!([{
                    "type": "property",
                    "device_id": "6755399441055744",
                    "code": "temp",
                    "operator": "gt",
                    "value": 30
                }]),
                actions: json!([{"type": "notify", "title": "itest", "message": "hot"}]),
            },
        )
        .await
        .expect("insert scene");
    let scenes = store.list_scenes("itest-tenant", false).await.unwrap();
    assert_eq!(scenes.len(), 1);
    assert_eq!(scenes[0].id, scene.id);
    assert_eq!(scenes[0].conditions, scene.conditions);
    assert_eq!(scenes[0].actions, scene.actions);

    // daily_reports.summary 为 JSON 列
    db.execute_with(
        "INSERT INTO daily_reports (id, tenant_id, report_date, summary) VALUES (?, ?, ?, ?)",
        &[
            json!(ecat::ids::next_id()),
            json!("itest-tenant"),
            json!("2000-01-01"),
            json!(json!({"devices_total": 1}).to_string()),
        ],
    )
    .await
    .expect("insert report");
    let reports = ecat_rule::report::list_reports(&db, "itest-tenant", None)
        .await
        .unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].summary, json!({"devices_total": 1}));
}

#[tokio::test]
#[ignore]
async fn ws_handshake_verifies_jwt() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // SAFETY: 测试进程内设置，无并发读 env 的其他线程
    unsafe { std::env::set_var("JWT_SECRET", "dev-secret-key-0123456789abcdefghijklmn") };

    // tower oneshot 缺 hyper OnUpgrade extension，WS extractor 永远 426；
    // 起真实 TCP 服务器做真握手（verify_token 在 on_upgrade 前执行，401/101 在 handler 内判定）
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            axum::Router::new()
                .route("/ws", axum::routing::get(ecat_rule::ws::ws_handler))
                .with_state(PushHub::new()),
        )
        .await
        .unwrap();
    });

    async fn handshake(addr: std::net::SocketAddr, uri: &str, protocols: Option<&str>) -> String {
        let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
        let proto_header = protocols
            .map(|p| format!("Sec-WebSocket-Protocol: {p}\r\n"))
            .unwrap_or_default();
        s.write_all(
            format!(
                "GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\n\
                 Upgrade: websocket\r\nSec-WebSocket-Version: 13\r\n\
                 Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n{proto_header}\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
        let mut buf = [0u8; 1024];
        let n = s.read(&mut buf).await.unwrap();
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    fn status(resp: &str) -> &str {
        resp.lines().next().unwrap_or("")
    }

    // 坏 token（子协议）→ 401（handler 拒绝，未升级）
    let resp = handshake(addr, "/ws", Some("bearer, garbage")).await;
    assert!(status(&resp).contains("401"), "bad token: {resp}");

    // 缺少 bearer / 缺少 token → 401
    let resp = handshake(addr, "/ws", Some("garbage")).await;
    assert!(status(&resp).contains("401"), "missing bearer: {resp}");
    let resp = handshake(addr, "/ws", None).await;
    assert!(status(&resp).contains("401"), "missing protocol: {resp}");

    // 合法 token（sub=itest-tenant，与网关同一 JWT_SECRET）→ 101 升级
    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        // 注意：AuthClaims.role 是 Option<String>，必须 Some(...)
        &ecat_auth::AuthClaims {
            sub: "itest-tenant".into(),
            exp: None,
            iat: None,
            role: Some("admin".into()),
            extra: Default::default(),
        },
        &jsonwebtoken::EncodingKey::from_secret(b"dev-secret-key-0123456789abcdefghijklmn"),
    )
    .unwrap();
    // token 只走子协议；响应回选 bearer，不回显 token
    let resp = handshake(addr, "/ws", Some(&format!("bearer, {token}"))).await;
    assert!(resp.starts_with("HTTP/1.1 101"), "good token: {resp}");
    assert!(
        resp.to_ascii_lowercase()
            .contains("sec-websocket-protocol: bearer"),
        "selected protocol: {resp}"
    );
    assert!(
        !resp.contains(token.as_str()),
        "token must not be echoed: {resp}"
    );

    // 旧方式 query token 不再认证
    let resp = handshake(addr, &format!("/ws?token={token}"), None).await;
    assert!(status(&resp).contains("401"), "query token: {resp}");
}
