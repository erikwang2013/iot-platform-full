//! 简易 JWT 签发/校验（业务服务登录与 WebSocket 共用）。
use crate::AuthClaims;

/// 校验 HS256 JWT 并返回 claims。校验签名与 exp（若存在）。
pub fn verify_token(token: &str, secret: &str) -> Result<AuthClaims, String> {
    let mut validation = jsonwebtoken::Validation::default();
    validation.required_spec_claims = std::collections::HashSet::new();
    jsonwebtoken::decode::<AuthClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|d| d.claims)
    .map_err(|e| format!("bad token: {e}"))
}

/// 签发 HS256 JWT：sub=租户 ID、role=用户角色，带 exp/iat。
pub fn issue_token(secret: &str, sub: &str, role: &str, ttl_secs: u64) -> Result<String, String> {
    issue_token_with_actor(secret, sub, role, None, ttl_secs)
}

/// 同 [`issue_token`]，额外落 `actor` claim（登录用户 / 开放 API 密钥 id）。
/// sub 保持租户语义不变；actor 进 flatten 的 extra，审计层据此识别操作者。
pub fn issue_token_with_actor(
    secret: &str,
    sub: &str,
    role: &str,
    actor: Option<&str>,
    ttl_secs: u64,
) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let mut extra = std::collections::HashMap::new();
    if let Some(actor) = actor {
        extra.insert("actor".to_string(), serde_json::json!(actor));
    }
    let claims = AuthClaims {
        sub: sub.to_string(),
        exp: Some(now + ttl_secs),
        iat: Some(now),
        role: Some(role.to_string()),
        extra,
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-0123456789abcdefghijklmnop";

    #[test]
    fn actor_roundtrips_as_top_level_claim() {
        let token = issue_token_with_actor(SECRET, "tenant-1", "admin", Some("alice"), 60).unwrap();
        let claims = verify_token(&token, SECRET).unwrap();
        assert_eq!(claims.sub, "tenant-1", "sub 仍是租户");
        assert_eq!(claims.role.as_deref(), Some("admin"));
        assert_eq!(
            claims.extra.get("actor").and_then(|v| v.as_str()),
            Some("alice")
        );
    }

    #[test]
    fn token_without_actor_has_no_actor_claim() {
        let token = issue_token(SECRET, "tenant-1", "admin", 60).unwrap();
        let claims = verify_token(&token, SECRET).unwrap();
        assert!(!claims.extra.contains_key("actor"));
    }
}
