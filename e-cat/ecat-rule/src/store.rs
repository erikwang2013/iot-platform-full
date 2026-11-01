use crate::cache::RuleCache;
use crate::engine::to_alert_record;
use crate::models::{AlertMessage, AlertRecord, NewNotifyChannel, NewRule, NotifyChannel, Rule};
use ecat_data::Row;
use ecat_data_sqlx::SqlxClient;
use ecat_security::crypto::{decrypt, derive_key, encrypt};
use serde_json::{Value, json};
use std::sync::Arc;

/// 加密 config 信封前缀：与历史明文行区分（读时两者都兼容）。
const ENC_PREFIX: &str = "enc:v1:";

#[derive(Clone)]
pub struct RuleStore {
    pub db: Arc<SqlxClient>,
    /// 规则/场景/渠道列表的进程内 TTL 缓存；多副本下写只失效本进程，
    /// 其他副本最多陈旧 RULE_CACHE_TTL_SECS（默认 5s）。
    pub(crate) cache: Arc<RuleCache>,
    /// IOT_CRED_ENCRYPT_KEY 派生的 AES 密钥；None = 明文落库（测试/未配置）。
    key: Option<[u8; 32]>,
    /// 轮换宽限期的上一把密钥，仅用于读取旧 enc:v1 行。
    old_key: Option<[u8; 32]>,
}

pub const OPERATORS: [&str; 6] = ["gt", "gte", "lt", "lte", "eq", "neq"];

pub const CHANNELS: [&str; 4] = ["email", "dingtalk", "wecom", "sms"];

fn config_str(c: &serde_json::Value, key: &str) -> Option<String> {
    c.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

fn validate_email_config(c: &serde_json::Value) -> Result<(), String> {
    let host = config_str(c, "smtp_host").ok_or("smtp_host required")?;
    if host.len() > 255 || host.contains(' ') {
        return Err("smtp_host invalid".into());
    }
    if let Some(p) = c.get("smtp_port").and_then(|v| v.as_i64()) {
        if !(1..=65535).contains(&p) {
            return Err("smtp_port must be 1..65535".into());
        }
    }
    for key in ["mail_from", "mail_to"] {
        let addr = config_str(c, key).ok_or(format!("{key} required"))?;
        if addr.len() > 255 || !addr.contains('@') {
            return Err(format!("{key} must be a valid email address"));
        }
    }
    Ok(())
}

fn validate_webhook_config(c: &serde_json::Value) -> Result<(), String> {
    let url = config_str(c, "webhook_url").ok_or("webhook_url required")?;
    if url.len() > 512 || !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("webhook_url must start with http(s):// and be <= 512 chars".into());
    }
    Ok(())
}

/// 短信渠道（A-1）：经 HTTP API 发送（阿里云/腾讯云短信等，POST JSON）。
/// config 字段：api_url（短信服务商 HTTP 端点）、phone（接收手机号）、
/// sign（短信签名）、template_id（模板 ID）。
fn validate_sms_config(c: &serde_json::Value) -> Result<(), String> {
    let url = config_str(c, "api_url").ok_or("api_url required")?;
    if url.len() > 512 || !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("api_url must start with http(s):// and be <= 512 chars".into());
    }
    let phone = config_str(c, "phone").ok_or("phone required")?;
    if phone.len() > 20 || !phone.chars().all(|ch| ch.is_ascii_digit() || ch == '+') {
        return Err("phone must be a valid phone number".into());
    }
    if config_str(c, "sign").is_none() {
        return Err("sign required".into());
    }
    if config_str(c, "template_id").is_none() {
        return Err("template_id required".into());
    }
    Ok(())
}

/// 通知渠道校验：渠道名白名单 + 按渠道校验 config 结构。
pub fn validate_channel(channel: &str, c: &serde_json::Value) -> Result<(), String> {
    if !c.is_object() {
        return Err("config must be a JSON object".into());
    }
    match channel {
        "email" => validate_email_config(c),
        "dingtalk" | "wecom" => validate_webhook_config(c),
        "sms" => validate_sms_config(c),
        _ => Err(format!("channel must be one of {CHANNELS:?}")),
    }
}

/// 迁移执行（复用 ecat-data-sqlx::execute_script 逐条执行）。
/// 编译期 include_str! 内联，无运行时文件依赖。
const MIGRATION_SQL: [&str; 4] = [
    include_str!("../migrations/0001_rule_tables.sql"),
    include_str!("../migrations/0002_notify_channels.sql"),
    include_str!("../migrations/0003_daily_reports.sql"),
    include_str!("../migrations/0004_scenes.sql"),
];

pub fn validate_rule(r: &NewRule) -> Result<(), String> {
    if r.name.trim().is_empty() || r.name.len() > 128 {
        return Err("name must be 1..128 chars".into());
    }
    // 平台设备 id 为 snowflake i64（十进制字符串）；DB 列 BIGINT
    if r.device_id.trim().is_empty() || r.device_id.parse::<i64>().is_err() {
        return Err("device_id must be a platform device id".into());
    }
    if r.code.trim().is_empty() || r.code.len() > 64 {
        return Err("code must be 1..64 chars".into());
    }
    if !OPERATORS.contains(&r.operator.as_str()) {
        return Err(format!("operator must be one of {OPERATORS:?}"));
    }
    if !r.threshold.is_finite() {
        return Err("threshold must be finite".into());
    }
    if let Some(u) = &r.webhook_url {
        if u.len() > 512 || !u.starts_with("http") {
            return Err("webhook_url must start with http and be <= 512 chars".into());
        }
    }
    // D-3 联动：action 三件套必须齐全（device_id + code + value）或不配置
    let action_fields = [
        r.action_device_id.is_some(),
        r.action_code.is_some(),
        r.action_value.is_some(),
    ];
    let any_action = action_fields.iter().any(|&f| f);
    let all_action = action_fields.iter().all(|&f| f);
    if any_action && !all_action {
        return Err("action requires action_device_id, action_code, action_value together".into());
    }
    if let Some(id) = &r.action_device_id {
        if id.trim().is_empty() || id.parse::<i64>().is_err() {
            return Err("action_device_id must be a platform device id".into());
        }
    }
    if let Some(code) = &r.action_code {
        if code.len() > 64 {
            return Err("action_code must be <= 64 chars".into());
        }
    }
    Ok(())
}

pub async fn migrate(db: &SqlxClient) -> Result<(), String> {
    for sql in MIGRATION_SQL {
        db.execute_script(sql)
            .await
            .map_err(|e| format!("migrate: {e}"))?;
    }
    // D-3 联动规则：老库补列（MySQL 8 无 ADD COLUMN IF NOT EXISTS，先查
    // information_schema，不存在才 ALTER；新装库由 0001 CREATE TABLE 直接建全列）。
    for (column, ddl) in RULE_ACTION_EXTRA_COLUMNS {
        let exists = db
            .query_with(
                "SELECT COUNT(*) AS n FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'rules' AND COLUMN_NAME = ?",
                &[json!(column)],
            )
            .await
            .map_err(|e| format!("check column {column}: {e}"))?;
        let n = exists
            .first()
            .and_then(|r| r.get("n"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if n == 0 {
            db.execute_with(&format!("ALTER TABLE rules ADD COLUMN {ddl}"), &[])
                .await
                .map_err(|e| format!("alter rules add {column}: {e}"))?;
        }
    }
    // webhook_url 加密后为 enc:v1 信封（512 字符明文≈730 字符密文），旧库 VARCHAR(512)
    // 不够；查到长度不足才改，避免每次启动重建表。
    let len = db
        .query_with(
            "SELECT CHARACTER_MAXIMUM_LENGTH AS n FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = 'rules' AND COLUMN_NAME = 'webhook_url'",
            &[],
        )
        .await
        .map_err(|e| format!("check rules.webhook_url length: {e}"))?
        .first()
        .and_then(|r| r.get("n"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if (1..1024).contains(&len) {
        db.execute_with(
            "ALTER TABLE rules MODIFY COLUMN webhook_url VARCHAR(1024) NULL",
            &[],
        )
        .await
        .map_err(|e| format!("widen rules.webhook_url: {e}"))?;
    }
    // alert_records.rule_id 须可空且规则删除时置 NULL：删规则要保留历史告警，
    // 异常检测告警（anomaly-detector）没有对应 rules 行。老库外键为 RESTRICT
    // 且列为 NOT NULL，按 information_schema 检出后重建。
    let delete_rule = db
        .query_with(
            "SELECT DELETE_RULE AS r FROM information_schema.REFERENTIAL_CONSTRAINTS \
             WHERE CONSTRAINT_SCHEMA = DATABASE() AND CONSTRAINT_NAME = 'fk_alert_rule'",
            &[],
        )
        .await
        .map_err(|e| format!("check fk_alert_rule: {e}"))?
        .first()
        .and_then(|r| r.get("r"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if delete_rule.as_deref() != Some("SET NULL") {
        // 手工建表可能没有该外键：不存在时跳过 DROP，避免启动直接失败。
        if delete_rule.is_some() {
            db.execute_with(
                "ALTER TABLE alert_records DROP FOREIGN KEY fk_alert_rule",
                &[],
            )
            .await
            .map_err(|e| format!("drop fk_alert_rule: {e}"))?;
        }
        db.execute_with(
            "ALTER TABLE alert_records MODIFY COLUMN rule_id BIGINT NULL",
            &[],
        )
        .await
        .map_err(|e| format!("make alert_records.rule_id nullable: {e}"))?;
        db.execute_with(
            "ALTER TABLE alert_records ADD CONSTRAINT fk_alert_rule \
             FOREIGN KEY (rule_id) REFERENCES rules(id) ON DELETE SET NULL",
            &[],
        )
        .await
        .map_err(|e| format!("recreate fk_alert_rule: {e}"))?;
    }
    Ok(())
}

/// D-3 联动规则新增列（老库补列；新库由 0001 CREATE TABLE 直接建）。
const RULE_ACTION_EXTRA_COLUMNS: [(&str, &str); 3] = [
    ("action_device_id", "action_device_id BIGINT NULL"),
    ("action_code", "action_code VARCHAR(64) NULL"),
    ("action_value", "action_value JSON NULL"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_sql_non_empty() {
        for sql in MIGRATION_SQL {
            assert!(!sql.trim().is_empty());
        }
    }

    #[test]
    fn alert_rule_id_keeps_numeric_and_nulls_anomaly() {
        assert_eq!(alert_rule_id("42"), Some(42));
        assert_eq!(alert_rule_id("anomaly-detector"), None);
        assert_eq!(alert_rule_id(""), None);
    }

    #[test]
    fn stats_from_counts_totals() {
        let c = vec![("active".to_string(), 3), ("acknowledged".to_string(), 2)];
        assert_eq!(stats_from_counts(&c), (5, 3));
        assert_eq!(stats_from_counts(&[]), (0, 0));
    }

    #[test]
    fn channel_config_encrypts_at_rest_and_reads_legacy_plaintext() {
        let key = derive_key("test-env-value");
        let cfg = json!({"smtp_pass": "p", "smtp_host": "h"});
        let enc = encode_config(Some(&key), &cfg).unwrap();
        assert!(enc.starts_with(ENC_PREFIX));
        assert!(!enc.contains("smtp_pass"));
        assert_eq!(decode_config(Some(&key), &enc), Some(cfg.clone()));
        // 启用加密后仍能读历史明文行
        assert_eq!(
            decode_config(Some(&key), &cfg.to_string()),
            Some(cfg.clone())
        );
        // RuleStore::new（无密钥）保持明文写、明文读
        let plain = encode_config(None, &cfg).unwrap();
        assert_eq!(plain, cfg.to_string());
        assert_eq!(decode_config(None, &plain), Some(cfg.clone()));
        // 无密钥/错误密钥读密文 → 不可读（不会误当明文解析）
        assert_eq!(decode_config(None, &enc), None);
        assert_eq!(decode_config(Some(&derive_key("other")), &enc), None);
    }

    fn action_row(v: Value) -> Row {
        Row::new(vec!["action_value".into()], vec![v])
    }

    #[test]
    fn rule_from_row_parses_json_text() {
        // MySQL 的 JSON 列经 CAST 返回文本
        assert_eq!(
            rule_from_row(&action_row(json!(r#"{"code":"on"}"#)), None, None).action_value,
            Some(json!({"code": "on"}))
        );
        // 其他驱动可能直接返回 Object，保持原有行为
        assert_eq!(
            rule_from_row(&action_row(json!({"code": "on"})), None, None).action_value,
            Some(json!({"code": "on"}))
        );
        // JSON 标量（如指令值 true）不应丢失
        assert_eq!(
            rule_from_row(&action_row(json!("true")), None, None).action_value,
            Some(json!(true))
        );
        assert_eq!(
            rule_from_row(&action_row(Value::Null), None, None).action_value,
            None
        );
        assert_eq!(
            rule_from_row(&action_row(json!(5)), None, None).action_value,
            None
        );
    }

    #[test]
    fn rule_webhook_encrypts_at_rest_and_reads_legacy_plaintext() {
        let key = derive_key("test-env-value");
        let url = "https://example.com/hook?token=s3cret-value";
        let enc = encode_webhook(Some(&key), Some(url)).unwrap().unwrap();
        assert!(enc.starts_with(ENC_PREFIX));
        assert!(!enc.contains("s3cret-value"));
        assert_eq!(
            decode_webhook(Some(&key), None, Some(&enc)).as_deref(),
            Some(url)
        );
        // 轮换宽限期：旧密钥密文仍可读
        assert_eq!(
            decode_webhook(Some(&derive_key("new")), Some(&key), Some(&enc)).as_deref(),
            Some(url)
        );
        // 历史明文行原样读出
        assert_eq!(
            decode_webhook(Some(&key), None, Some(url)).as_deref(),
            Some(url)
        );
        // 无密钥：明文写、明文读
        assert_eq!(
            encode_webhook(None, Some(url)).unwrap().as_deref(),
            Some(url)
        );
        assert_eq!(decode_webhook(None, None, Some(url)).as_deref(), Some(url));
        // 无密钥/错误密钥读密文 → None（不会把密文当 URL 使用）
        assert_eq!(decode_webhook(None, None, Some(&enc)), None);
        assert_eq!(
            decode_webhook(Some(&derive_key("other")), None, Some(&enc)),
            None
        );
        // NULL 保持 NULL
        assert_eq!(encode_webhook(Some(&key), None).unwrap(), None);
        assert_eq!(decode_webhook(Some(&key), None, None), None);
    }
}

fn encode_config(key: Option<&[u8; 32]>, config: &Value) -> Result<String, String> {
    let plain = config.to_string();
    match key {
        None => Ok(plain),
        Some(key) => encrypt(key, plain.as_bytes()).map(|enc| format!("{ENC_PREFIX}{enc}")),
    }
}

/// 规则 webhook 加密落库（enc:v1 信封）；无密钥保持明文（测试/本地）。
fn encode_webhook(key: Option<&[u8; 32]>, url: Option<&str>) -> Result<Option<String>, String> {
    let Some(url) = url else { return Ok(None) };
    match key {
        None => Ok(Some(url.to_string())),
        Some(key) => encrypt(key, url.as_bytes()).map(|enc| Some(format!("{ENC_PREFIX}{enc}"))),
    }
}

/// 解出规则 webhook：先当前密钥、再轮换旧密钥；历史明文原样返回。
/// 密文解不开时返回 None，绝不把密文当 URL 传给发送端。
fn decode_webhook(
    key: Option<&[u8; 32]>,
    old_key: Option<&[u8; 32]>,
    raw: Option<&str>,
) -> Option<String> {
    let raw = raw?;
    let Some(enc) = raw.strip_prefix(ENC_PREFIX) else {
        return Some(raw.to_string());
    };
    for k in [key, old_key].into_iter().flatten() {
        if let Ok(plain) = decrypt(k, enc) {
            if let Ok(url) = String::from_utf8(plain) {
                return Some(url);
            }
        }
    }
    if key.is_none() {
        tracing::warn!("encrypted rule webhook but IOT_CRED_ENCRYPT_KEY is not configured");
    } else {
        tracing::warn!("encrypted rule webhook cannot be decrypted with current/old key");
    }
    None
}

/// MySQL JSON 列的 CAST 可能把 JSON 字符串输出成带引号的文本；统一还原。
fn config_text(raw: &str) -> String {
    if raw.starts_with('"') {
        serde_json::from_str::<String>(raw).unwrap_or_else(|_| raw.to_string())
    } else {
        raw.to_string()
    }
}

/// 解出 config：识别 enc:v1: 信封，其余按历史明文处理。
fn decode_config(key: Option<&[u8; 32]>, raw: &str) -> Option<Value> {
    let raw = config_text(raw);
    let Some(enc) = raw.strip_prefix(ENC_PREFIX) else {
        return serde_json::from_str(&raw).ok();
    };
    let Some(key) = key else {
        tracing::warn!("encrypted notify config but IOT_CRED_ENCRYPT_KEY is not configured");
        return None;
    };
    decrypt(key, enc)
        .ok()
        .and_then(|plain| serde_json::from_slice(&plain).ok())
}

impl RuleStore {
    pub fn new(db: Arc<SqlxClient>) -> Self {
        Self {
            db,
            cache: Arc::new(RuleCache::from_env()),
            key: None,
            old_key: None,
        }
    }

    /// 生产构造：notify_channels.config 以 enc:v1: 信封加密落库。
    /// env_value 为 IOT_CRED_ENCRYPT_KEY 原值（derive_key 派生 AES 密钥）；
    /// IOT_CRED_ENCRYPT_KEY_OLD 存在时用于读取轮换前的旧密文。
    pub fn with_encryption(db: Arc<SqlxClient>, env_value: &str) -> Self {
        let old_key = std::env::var("IOT_CRED_ENCRYPT_KEY_OLD")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .map(|v| derive_key(&v));
        Self {
            db,
            cache: Arc::new(RuleCache::from_env()),
            key: Some(derive_key(env_value)),
            old_key,
        }
    }

    pub async fn list_rules(&self, tenant_id: &str) -> Result<Arc<Vec<Rule>>, String> {
        if let Some(rules) = self.cache.get_rules(tenant_id) {
            return Ok(rules);
        }
        let rows = self
            .db
            .query_with(
                // sqlx Any 不支持 Timestamp/Json 类型，时间列与 JSON 列须 CAST 成文本
                "SELECT id, tenant_id, name, device_id, code, operator, threshold, webhook_url, \
                 action_device_id, action_code, CAST(action_value AS CHAR) AS action_value, enabled, \
                 CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at \
                 FROM rules WHERE tenant_id = ? ORDER BY created_at DESC",
                &[json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("list rules: {e}"))?;
        let rules: Vec<Rule> = rows
            .iter()
            .map(|r| rule_from_row(r, self.key.as_ref(), self.old_key.as_ref()))
            .collect();
        self.cache.put_rules(tenant_id, &rules);
        Ok(Arc::new(rules))
    }

    pub async fn insert_rule(&self, tenant_id: &str, r: &NewRule) -> Result<Rule, String> {
        validate_rule(r)?;
        // DB 列 BIGINT：绑定数字；Rule/JSON 层保持十进制字符串
        let id = ecat::ids::next_id();
        let device_id: i64 = r.device_id.parse().map_err(|_| "device_id invalid")?;
        // MySQL JSON 列：sqlx Any 只能编码字符串，JSON 文本入库，NULL 保持 NULL
        let action_value = r.action_value.as_ref().map(Value::to_string);
        let action_device_id: Option<i64> = r
            .action_device_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "action_device_id invalid")?;
        let rule = Rule {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            name: r.name.clone(),
            device_id: device_id.to_string(),
            code: r.code.clone(),
            operator: r.operator.clone(),
            threshold: r.threshold,
            webhook_url: r.webhook_url.clone(),
            action_device_id: r.action_device_id.clone(),
            action_code: r.action_code.clone(),
            action_value: r.action_value.clone(),
            enabled: r.enabled.unwrap_or(true),
            created_at: String::new(),
            updated_at: String::new(),
        };
        self.db
            .execute_with(
                "INSERT INTO rules (id, tenant_id, name, device_id, code, operator, threshold, \
                 webhook_url, action_device_id, action_code, action_value, enabled) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                &[
                    json!(id),
                    json!(rule.tenant_id),
                    json!(rule.name),
                    json!(device_id),
                    json!(rule.code),
                    json!(rule.operator),
                    json!(rule.threshold),
                    json!(encode_webhook(self.key.as_ref(), r.webhook_url.as_deref())?),
                    json!(action_device_id),
                    json!(rule.action_code),
                    json!(action_value),
                    json!(rule.enabled as i64),
                ],
            )
            .await
            .map_err(|e| format!("insert rule: {e}"))?;
        self.cache.invalidate_rules(tenant_id);
        Ok(rule)
    }

    pub async fn update_rule(
        &self,
        tenant_id: &str,
        id: &str,
        r: &NewRule,
    ) -> Result<bool, String> {
        validate_rule(r)?;
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        let device_id: i64 = r.device_id.parse().map_err(|_| "device_id invalid")?;
        let action_value = r.action_value.as_ref().map(Value::to_string);
        let action_device_id: Option<i64> = r
            .action_device_id
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| "action_device_id invalid")?;
        let n = self
            .db
            .execute_with(
                "UPDATE rules SET name = ?, device_id = ?, code = ?, operator = ?, threshold = ?, \
                 webhook_url = ?, action_device_id = ?, action_code = ?, action_value = ?, \
                 enabled = ? WHERE id = ? AND tenant_id = ?",
                &[
                    json!(r.name),
                    json!(device_id),
                    json!(r.code),
                    json!(r.operator),
                    json!(r.threshold),
                    json!(encode_webhook(self.key.as_ref(), r.webhook_url.as_deref())?),
                    json!(action_device_id),
                    json!(r.action_code),
                    json!(action_value),
                    json!(r.enabled.unwrap_or(true) as i64),
                    json!(id),
                    json!(tenant_id),
                ],
            )
            .await
            .map_err(|e| format!("update rule: {e}"))?;
        self.cache.invalidate_rules(tenant_id);
        Ok(n > 0)
    }

    /// 删规则不删历史告警：alert_records.rule_id 外键为 ON DELETE SET NULL。
    pub async fn delete_rule(&self, tenant_id: &str, id: &str) -> Result<bool, String> {
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        let n = self
            .db
            .execute_with(
                "DELETE FROM rules WHERE id = ? AND tenant_id = ?",
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("delete rule: {e}"))?;
        self.cache.invalidate_rules(tenant_id);
        Ok(n > 0)
    }

    pub async fn insert_alert(&self, msg: &AlertMessage) -> Result<(), String> {
        let rec = to_alert_record(msg);
        // alert_records 的 id/rule_id/device_id 为 BIGINT 列：绑定数字。
        // rule_id 可空：异常检测告警的 "anomaly-detector" 没有 rules 行。
        let id: i64 = rec.id.parse().map_err(|_| "alert id invalid")?;
        let rule_id: Option<i64> = alert_rule_id(&rec.rule_id);
        let device_id: i64 = rec
            .device_id
            .parse()
            .map_err(|_| "device_id must be a platform id")?;
        self.db
            .execute_with(
                "INSERT INTO alert_records (id, rule_id, tenant_id, device_id, code, operator, threshold, value, status) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active')",
                &[
                    json!(id),
                    json!(rule_id),
                    json!(rec.tenant_id),
                    json!(device_id),
                    json!(rec.code),
                    json!(rec.operator),
                    json!(rec.threshold),
                    json!(rec.value.to_string()),
                ],
            )
            .await
            .map_err(|e| format!("insert alert: {e}"))?;
        Ok(())
    }

    pub async fn list_alerts(
        &self,
        tenant_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<AlertRecord>, String> {
        let (sql, params) = match status {
            Some(s) => (
                "SELECT id, rule_id, tenant_id, device_id, code, operator, threshold, value, status, \
                 CAST(created_at AS CHAR) AS created_at \
                 FROM alert_records WHERE tenant_id = ? AND status = ? ORDER BY created_at DESC LIMIT 100",
                vec![json!(tenant_id), json!(s)],
            ),
            None => (
                "SELECT id, rule_id, tenant_id, device_id, code, operator, threshold, value, status, \
                 CAST(created_at AS CHAR) AS created_at \
                 FROM alert_records WHERE tenant_id = ? ORDER BY created_at DESC LIMIT 100",
                vec![json!(tenant_id)],
            ),
        };
        let rows = self
            .db
            .query_with(sql, &params)
            .await
            .map_err(|e| format!("list alerts: {e}"))?;
        Ok(rows.iter().map(alert_from_row).collect())
    }

    /// 告警统计：总数 + 未处理（active）数。
    pub async fn stats(&self, tenant_id: &str) -> Result<(i64, i64), String> {
        let rows = self
            .db
            .query_with(
                "SELECT status, COUNT(*) AS n FROM alert_records WHERE tenant_id = ? GROUP BY status",
                &[json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("alert stats: {e}"))?;
        let counts: Vec<(String, i64)> = rows
            .iter()
            .map(|r| {
                (
                    r.get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    r.get("n").and_then(Value::as_i64).unwrap_or(0),
                )
            })
            .collect();
        Ok(stats_from_counts(&counts))
    }

    pub async fn ack_alert(&self, tenant_id: &str, id: &str) -> Result<bool, String> {
        let id: i64 = id.parse().map_err(|_| "id must be a platform id")?;
        let n = self
            .db
            .execute_with(
                "UPDATE alert_records SET status = 'acknowledged' WHERE id = ? AND tenant_id = ?",
                &[json!(id), json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("ack alert: {e}"))?;
        Ok(n > 0)
    }

    /// 把历史上以明文 JSON 落库的通知渠道配置批量迁移为 enc:v1 信封。
    /// 幂等：已加密行跳过；无密钥时不做任何事（测试/本地）。
    pub async fn encrypt_channels_at_rest(&self) -> Result<u64, String> {
        if self.key.is_none() {
            return Ok(0);
        }
        let rows = self
            .db
            .query_with(
                "SELECT id, tenant_id, CAST(config AS CHAR) AS config FROM notify_channels",
                &[],
            )
            .await
            .map_err(|e| format!("list channels for encryption: {e}"))?;
        let mut migrated = 0u64;
        for r in &rows {
            let Some(id) = r.get("id").and_then(Value::as_i64) else {
                continue;
            };
            let Some(tenant) = r.get("tenant_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(raw) = r.get("config").and_then(Value::as_str) else {
                continue;
            };
            let raw = config_text(raw);
            if raw.starts_with(ENC_PREFIX) {
                continue;
            }
            let config: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(channel_id = id, error = %e, "skip malformed legacy channel config");
                    continue;
                }
            };
            let encrypted = encode_config(self.key.as_ref(), &config)?;
            self.db
                .execute_with(
                    "UPDATE notify_channels SET config = ? WHERE id = ? AND tenant_id = ?",
                    &[json!(encrypted), json!(id), json!(tenant)],
                )
                .await
                .map_err(|e| format!("encrypt channel {id}: {e}"))?;
            migrated += 1;
        }
        Ok(migrated)
    }

    /// 把历史上以明文落库的规则 webhook 批量迁移为 enc:v1 信封。
    /// 幂等：已加密行（含旧密钥密文）与空值跳过；无密钥时不做任何事。
    pub async fn encrypt_rules_at_rest(&self) -> Result<u64, String> {
        if self.key.is_none() {
            return Ok(0);
        }
        let rows = self
            .db
            .query_with(
                "SELECT id, tenant_id, webhook_url FROM rules \
                 WHERE webhook_url IS NOT NULL AND webhook_url <> ''",
                &[],
            )
            .await
            .map_err(|e| format!("list rules for encryption: {e}"))?;
        let mut migrated = 0u64;
        for r in &rows {
            let Some(id) = r.get("id").and_then(Value::as_i64) else {
                continue;
            };
            let Some(tenant) = r.get("tenant_id").and_then(Value::as_str) else {
                continue;
            };
            let Some(raw) = r.get("webhook_url").and_then(Value::as_str) else {
                continue;
            };
            if raw.starts_with(ENC_PREFIX) {
                continue;
            }
            let encrypted = encode_webhook(self.key.as_ref(), Some(raw))?.unwrap_or_default();
            self.db
                .execute_with(
                    "UPDATE rules SET webhook_url = ? WHERE id = ? AND tenant_id = ?",
                    &[json!(encrypted), json!(id), json!(tenant)],
                )
                .await
                .map_err(|e| format!("encrypt rule {id}: {e}"))?;
            migrated += 1;
        }
        Ok(migrated)
    }

    pub async fn list_channels(&self, tenant_id: &str) -> Result<Arc<Vec<NotifyChannel>>, String> {
        if let Some(channels) = self.cache.get_channels(tenant_id) {
            return Ok(channels);
        }
        let rows = self
            .db
            .query_with(
                "SELECT id, tenant_id, channel, CAST(config AS CHAR) AS config, enabled, \
                 CAST(created_at AS CHAR) AS created_at, CAST(updated_at AS CHAR) AS updated_at \
                 FROM notify_channels WHERE tenant_id = ? ORDER BY created_at",
                &[json!(tenant_id)],
            )
            .await
            .map_err(|e| format!("list channels: {e}"))?;
        let channels: Vec<NotifyChannel> = rows.iter().map(|r| self.channel_from_row(r)).collect();
        self.cache.put_channels(tenant_id, &channels);
        Ok(Arc::new(channels))
    }

    /// 单租户单渠道唯一（UNIQUE KEY），重复 PUT 即更新。
    pub async fn upsert_channel(
        &self,
        tenant_id: &str,
        channel: &str,
        req: &NewNotifyChannel,
    ) -> Result<NotifyChannel, String> {
        validate_channel(channel, &req.config)?;
        let config = encode_config(self.key.as_ref(), &req.config)?;
        let id = ecat::ids::next_id();
        self.db
            .execute_with(
                "INSERT INTO notify_channels (id, tenant_id, channel, config, enabled) \
                 VALUES (?, ?, ?, ?, ?) \
                 ON DUPLICATE KEY UPDATE config = VALUES(config), enabled = VALUES(enabled)",
                &[
                    json!(id),
                    json!(tenant_id),
                    json!(channel),
                    json!(config),
                    json!(req.enabled.unwrap_or(true) as i64),
                ],
            )
            .await
            .map_err(|e| format!("upsert channel: {e}"))?;
        self.cache.invalidate_channels(tenant_id);
        Ok(NotifyChannel {
            id: id.to_string(),
            tenant_id: tenant_id.to_string(),
            channel: channel.to_string(),
            config: req.config.clone(),
            enabled: req.enabled.unwrap_or(true),
            created_at: String::new(),
            updated_at: String::new(),
        })
    }

    pub async fn delete_channel(&self, tenant_id: &str, channel: &str) -> Result<bool, String> {
        let n = self
            .db
            .execute_with(
                "DELETE FROM notify_channels WHERE tenant_id = ? AND channel = ?",
                &[json!(tenant_id), json!(channel)],
            )
            .await
            .map_err(|e| format!("delete channel: {e}"))?;
        self.cache.invalidate_channels(tenant_id);
        Ok(n > 0)
    }
}

/// 告警的规则 id：平台规则为十进制主键，异常检测告警为 "anomaly-detector"
/// （无 rules 行，落库 NULL）。
fn alert_rule_id(s: &str) -> Option<i64> {
    s.parse().ok()
}

/// status 计数 → (总数, 未处理数)。acknowledged 只计入总数。
fn stats_from_counts(counts: &[(String, i64)]) -> (i64, i64) {
    let mut total = 0;
    let mut active = 0;
    for (status, n) in counts {
        total += n;
        if status == "active" {
            active += n;
        }
    }
    (total, active)
}

/// BIGINT 列（snowflake id）读取 → 十进制字符串。JSON 侧该列是 Number，
/// 严禁 as_str（会静默得空串）。
pub(crate) fn id_str(r: &Row, col: &str) -> String {
    r.get(col)
        .and_then(Value::as_i64)
        .map(|n| n.to_string())
        .unwrap_or_default()
}

fn rule_from_row(r: &Row, key: Option<&[u8; 32]>, old_key: Option<&[u8; 32]>) -> Rule {
    Rule {
        id: id_str(r, "id"),
        tenant_id: r
            .get("tenant_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        name: r
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        device_id: id_str(r, "device_id"),
        code: r
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        operator: r
            .get("operator")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        threshold: r.get("threshold").and_then(Value::as_f64).unwrap_or(0.0),
        webhook_url: decode_webhook(key, old_key, r.get("webhook_url").and_then(Value::as_str)),
        action_device_id: r
            .get("action_device_id")
            .and_then(Value::as_i64)
            .map(|n| n.to_string()),
        action_code: r
            .get("action_code")
            .and_then(Value::as_str)
            .map(str::to_string),
        // MySQL JSON 列经 CAST 返回文本；兼容驱动直接返回 Object 的情况
        action_value: r.get("action_value").and_then(|v| match v {
            Value::String(s) => serde_json::from_str(s).ok(),
            Value::Object(_) => Some(v.clone()),
            _ => None,
        }),
        enabled: r.get("enabled").and_then(Value::as_i64).unwrap_or(0) != 0,
        created_at: r
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        updated_at: r
            .get("updated_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

impl RuleStore {
    fn channel_from_row(&self, r: &Row) -> NotifyChannel {
        NotifyChannel {
            id: id_str(r, "id"),
            tenant_id: r
                .get("tenant_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            channel: r
                .get("channel")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            config: r
                .get("config")
                .and_then(Value::as_str)
                .and_then(|s| {
                    decode_config(self.key.as_ref(), s)
                        .or_else(|| decode_config(self.old_key.as_ref(), s))
                })
                .unwrap_or(Value::Null),
            enabled: r.get("enabled").and_then(Value::as_i64).unwrap_or(0) != 0,
            created_at: r
                .get("created_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            updated_at: r
                .get("updated_at")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        }
    }
}

fn alert_from_row(r: &Row) -> AlertRecord {
    AlertRecord {
        id: id_str(r, "id"),
        rule_id: id_str(r, "rule_id"),
        tenant_id: r
            .get("tenant_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        device_id: id_str(r, "device_id"),
        code: r
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        operator: r
            .get("operator")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        threshold: r.get("threshold").and_then(Value::as_f64).unwrap_or(0.0),
        value: r
            .get("value")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null),
        status: r
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        created_at: r
            .get("created_at")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}
