use serde::{Deserialize, Serialize};
use serde_json::Value;

/// CDN 供应商（config 为解密后的厂商配置 JSON）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub tenant_id: String,
    pub name: String,
    pub vendor: String,
    pub domain: String,
    pub config: Value,
    pub status: String,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct NewProvider {
    pub name: String,
    pub vendor: String,
    pub domain: Option<String>,
    pub config: Value,
}

#[derive(Deserialize, Default)]
pub struct UpdateProvider {
    pub name: Option<String>,
    pub domain: Option<String>,
    pub config: Option<Value>,
}

#[derive(Deserialize)]
pub struct UrlReq {
    pub url: String,
    pub expires_secs: u64,
}

#[derive(Deserialize)]
pub struct TaskReq {
    pub urls: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CdnTask {
    pub id: String,
    pub tenant_id: String,
    pub provider_id: String,
    pub kind: String,
    pub urls: Vec<String>,
    pub status: String,
    pub error: String,
    pub created_at: String,
}

/// 边界校验：URL 必须是 http/https 且长度受限。
pub fn validate_url(url: &str) -> Result<(), String> {
    if url.len() > 2048 || !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("url must be http(s) and within 2048 chars".into());
    }
    Ok(())
}

/// 边界校验：签名 URL 有效期 60s ~ 24h。
pub fn validate_expires(secs: u64) -> Result<(), String> {
    if !(60..=86400).contains(&secs) {
        return Err("expires_secs must be 60..=86400".into());
    }
    Ok(())
}

/// 非管理员可见的供应商 config：保留字段名，值统一掩码（凭据不下发）。
pub fn masked_provider_config(c: &Value) -> Value {
    let Some(obj) = c.as_object() else {
        return Value::Object(serde_json::Map::new());
    };
    Value::Object(
        obj.keys()
            .map(|k| (k.clone(), Value::String("***".into())))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masked_provider_config_hides_secret_values() {
        let masked = masked_provider_config(&json!({
            "api_token": "t0ken",
            "zone_id": "z1",
        }));
        assert_eq!(masked["api_token"], "***");
        assert_eq!(masked["zone_id"], "***");
        assert!(!masked.to_string().contains("t0ken"));
        assert!(!masked.to_string().contains("z1"));
    }
}
