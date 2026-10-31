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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let claims = AuthClaims {
        sub: sub.to_string(),
        exp: Some(now + ttl_secs),
        iat: Some(now),
        role: Some(role.to_string()),
        extra: Default::default(),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}
