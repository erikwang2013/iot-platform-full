use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 统一事件消息：契约定义在 ecat-iot（跨服务共享）。
pub use ecat_iot::EventMessage;

/// 阈值规则（rules 表行）。operator：gt|gte|lt|lte|eq|neq。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub device_id: String,
    pub code: String,
    pub operator: String,
    pub threshold: f64,
    /// 命中时额外 POST 告警 JSON 到该 URL；None = 仅推送
    pub webhook_url: Option<String>,
    /// D-3 联动：命中时对另一设备下发指令（device_id + code + value）；全 None = 仅告警
    pub action_device_id: Option<String>,
    pub action_code: Option<String>,
    pub action_value: Option<serde_json::Value>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建/更新规则的请求体（id/tenant_id/时间戳由服务端生成）。
#[derive(Deserialize, Clone, Debug)]
pub struct NewRule {
    pub name: String,
    pub device_id: String,
    pub code: String,
    pub operator: String,
    pub threshold: f64,
    pub webhook_url: Option<String>,
    /// D-3 联动：目标设备指令（需三件套齐全才生效）
    pub action_device_id: Option<String>,
    pub action_code: Option<String>,
    pub action_value: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

/// WebSocket 推送与 webhook 载荷（引擎产出物；Redis 跨实例桥需 Deserialize）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AlertMessage {
    pub rule_id: String,
    pub rule_name: String,
    pub tenant_id: String,
    pub device_id: String,
    pub code: String,
    pub operator: String,
    pub threshold: f64,
    pub value: serde_json::Value,
    pub ts: i64,
}

/// 通知渠道（notify_channels 表行）。channel：email|dingtalk|wecom|sms。
/// config 为渠道私有 JSON：email 用 smtp_host/port/user/pass/mail_from/mail_to，
/// dingtalk/wecom 用 webhook_url，sms 用 api_url/phone/sign/template_id（A-1）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NotifyChannel {
    pub id: String,
    pub tenant_id: String,
    pub channel: String,
    pub config: serde_json::Value,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建/更新通知渠道的请求体（id/tenant_id/时间戳由服务端生成）。
#[derive(Deserialize, Clone, Debug)]
pub struct NewNotifyChannel {
    pub config: serde_json::Value,
    pub enabled: Option<bool>,
}

/// 非管理员可见的渠道 config：仅非敏感字段 + 是否已配置密钥。
/// 凭据（smtp_pass / webhook_url / 短信 api_url）不下发。
pub fn masked_channel_config(channel: &str, c: &Value) -> Value {
    let mut out = serde_json::Map::new();
    let visible: &[&str] = match channel {
        "email" => &[
            "smtp_host",
            "smtp_port",
            "smtp_tls",
            "smtp_user",
            "mail_from",
            "mail_to",
        ],
        "sms" => &["phone", "sign", "template_id"],
        _ => &[],
    };
    for key in visible {
        if let Some(v) = c.get(*key) {
            out.insert((*key).to_string(), v.clone());
        }
    }
    let flag: &[(&str, &str)] = match channel {
        "email" => &[("has_password", "smtp_pass")],
        "dingtalk" | "wecom" => &[("has_webhook", "webhook_url")],
        "sms" => &[("has_api_url", "api_url")],
        _ => &[],
    };
    for (name, key) in flag {
        let set = c
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
        out.insert((*name).to_string(), Value::Bool(set));
    }
    Value::Object(out)
}

/// 非管理员可见的规则：隐藏 webhook_url（常为携带 token 的 bearer URL）。
pub fn masked_rule(mut r: Rule) -> Rule {
    r.webhook_url = None;
    r
}

fn default_true() -> bool {
    true
}

/// 场景（scenes 表行）。conditions/actions 为校验后的 JSON 数组（见 scenes::validate_scene）。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Scene {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub enabled: bool,
    pub conditions: Value,
    pub actions: Value,
    pub last_triggered_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 创建/更新场景的请求体（id/tenant_id 时间戳由服务端生成）。
#[derive(Deserialize, Clone, Debug)]
pub struct NewScene {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub conditions: Value,
    pub actions: Value,
}

/// 站内通知（notifications 表行）。source：scene 等。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Notification {
    pub id: String,
    pub tenant_id: String,
    pub title: String,
    pub message: String,
    pub source: String,
    pub read_at: Option<String>,
    pub created_at: String,
}

/// 告警记录（alert_records 表行）。status：active|acknowledged。
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AlertRecord {
    pub id: String,
    pub rule_id: String,
    pub tenant_id: String,
    pub device_id: String,
    pub code: String,
    pub operator: String,
    pub threshold: f64,
    pub value: serde_json::Value,
    pub status: String,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masked_channel_config_hides_credentials() {
        let email = masked_channel_config(
            "email",
            &json!({
                "smtp_host": "smtp.example.com",
                "smtp_user": "u",
                "smtp_pass": "p",
                "mail_from": "a@example.com",
                "mail_to": "b@example.com",
            }),
        );
        assert_eq!(email["smtp_host"], "smtp.example.com");
        assert_eq!(email["has_password"], true);
        assert!(email.get("smtp_pass").is_none());

        let hook = masked_channel_config(
            "wecom",
            &json!({ "webhook_url": "https://qyapi.weixin.qq.com/x?key=k" }),
        );
        assert_eq!(hook["has_webhook"], true);
        assert!(!hook.to_string().contains("key=k"));

        let sms = masked_channel_config(
            "sms",
            &json!({ "api_url": "https://sms.example.com/send", "phone": "13800138000" }),
        );
        assert_eq!(sms["phone"], "13800138000");
        assert_eq!(sms["has_api_url"], true);
        assert!(sms.get("api_url").is_none());
        assert!(!sms.to_string().contains("sms.example.com"));
    }

    #[test]
    fn masked_rule_drops_webhook_url() {
        let rule = Rule {
            id: "1".into(),
            tenant_id: "t1".into(),
            name: "r".into(),
            device_id: "2".into(),
            code: "temp".into(),
            operator: "gt".into(),
            threshold: 1.0,
            webhook_url: Some("https://example.com/hook?token=x".into()),
            action_device_id: None,
            action_code: None,
            action_value: None,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        };
        assert!(masked_rule(rule).webhook_url.is_none());
    }
}
