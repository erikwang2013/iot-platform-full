use crate::store::Store;
use axum::{Json, extract::State, http::StatusCode};
use ecat_auth::issue_token_with_actor;
use ecat_security::crypto::verify_hmac_sha256_hex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub struct AuthState {
    pub store: Arc<Store>,
    pub jwt_secret: String,
    /// 旧 HMAC 口令兼容校验用 pepper；启动时必填（main fail-closed）。
    pub pepper: String,
    pub token_ttl_secs: u64,
    /// 未知用户也跑一次同成本 PBKDF2，响应时间不泄露用户名是否存在。
    dummy_password_hash: String,
}

impl AuthState {
    pub fn new(store: Arc<Store>, jwt_secret: String, pepper: String, token_ttl_secs: u64) -> Self {
        Self {
            store,
            jwt_secret,
            pepper,
            token_ttl_secs,
            dummy_password_hash: dummy_password_hash(),
        }
    }
}

/// 固定哑哈希：未知用户走与真实校验相同的 PBKDF2 迭代/输出长度。
fn dummy_password_hash() -> String {
    hash_password("dummy-password-for-timing-equalization")
}

#[derive(Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
pub struct OpenTokenReq {
    pub app_id: String,
    pub app_secret: String,
}

/// PBKDF2 迭代次数与口令哈希前缀；格式：
/// `pbkdf2-sha256$<iter>$<salt_hex>$<hash_hex>`。
const PBKDF2_ITERATIONS: u32 = 210_000;
const PBKDF2_PREFIX: &str = "pbkdf2-sha256$";
/// 校验侧迭代次数上限：防被篡改的库值导致校验卡死。
const PBKDF2_MAX_ITERATIONS: u32 = 1_000_000;

/// 新口令哈希：PBKDF2-HMAC-SHA256，16 字节随机盐，输出 32 字节。
pub fn hash_password(password: &str) -> String {
    let mut salt = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut salt);
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut out);
    format!(
        "{PBKDF2_PREFIX}{PBKDF2_ITERATIONS}${}${}",
        hex::encode(salt),
        hex::encode(out)
    )
}

/// 密码校验：PBKDF2 格式直接校验；旧 HMAC（pepper 为密钥）格式继续兼容
/// （登录成功后由 auth::login 原地升级）。
pub fn password_matches(stored_hash: &str, pepper: &str, password: &str) -> bool {
    if let Some(rest) = stored_hash.strip_prefix(PBKDF2_PREFIX) {
        return verify_pbkdf2(rest, password);
    }
    verify_hmac_sha256_hex(pepper, password.as_bytes(), stored_hash)
}

/// `<iter>$<salt_hex>$<hash_hex>` 校验；格式损坏/长度不符一律 false。
fn verify_pbkdf2(rest: &str, password: &str) -> bool {
    let mut parts = rest.split('$');
    let (Some(iter), Some(salt), Some(expected), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(iter) = iter.parse::<u32>() else {
        return false;
    };
    if iter == 0 || iter > PBKDF2_MAX_ITERATIONS {
        return false;
    }
    let (Ok(salt), Ok(expected)) = (hex::decode(salt), hex::decode(expected)) else {
        return false;
    };
    let mut out = vec![0u8; expected.len()];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), &salt, iter, &mut out);
    ct_eq(&out, &expected)
}

/// 常数时间字节比较（等长逐字节 XOR；长度差直接失败）。
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 阻塞线程池校验口令（PBKDF2 210k 次迭代，避免占住 tokio worker）。
async fn verify_password(hash: String, pepper: String, password: String) -> bool {
    tokio::task::spawn_blocking(move || password_matches(&hash, &pepper, &password))
        .await
        .unwrap_or(false)
}

/// POST /api/auth/login 与 /admin/auth/login（网关公开代理，无 JWT 前置）
/// 校验 users 表 → 签发 {sub: tenant_id, actor: username, role, exp, iat} 的 JWT。
/// 失败统一 401 通用文案，细节只进日志（防用户枚举）。
pub async fn login(
    State(auth): State<AuthState>,
    Json(req): Json<LoginReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if req.username.trim().is_empty() || req.password.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "username and password required".into(),
        ));
    }
    let user = match auth.store.find_user(req.username.trim()).await {
        Ok(Some(u)) => u,
        Ok(None) | Err(_) => {
            // 未知用户同样跑一次全成本 PBKDF2，响应时间与"密码错误"一致。
            let _ = verify_password(
                auth.dummy_password_hash.clone(),
                auth.pepper.clone(),
                req.password.clone(),
            )
            .await;
            tracing::warn!("login failed: user not found or lookup error");
            return Err((
                StatusCode::UNAUTHORIZED,
                "invalid username or password".into(),
            ));
        }
    };
    if !verify_password(
        user.password_hash.clone(),
        auth.pepper.clone(),
        req.password.clone(),
    )
    .await
    {
        tracing::warn!(user = %user.username, "login failed: bad password");
        return Err((
            StatusCode::UNAUTHORIZED,
            "invalid username or password".into(),
        ));
    }
    // 旧 HMAC 格式在登录成功时原地升级为 PBKDF2（明文口令仅此刻可用）。
    if !user.password_hash.starts_with(PBKDF2_PREFIX) {
        let password = req.password.clone();
        match tokio::task::spawn_blocking(move || hash_password(&password)).await {
            Ok(hash) => {
                if let Err(e) = auth.store.update_user_password(&user.id, &hash).await {
                    tracing::warn!(user = %user.username, error = %e, "password hash upgrade failed");
                }
            }
            Err(e) => {
                tracing::warn!(user = %user.username, error = %e, "password hash upgrade task failed")
            }
        }
    }
    let token = issue_token_with_actor(
        &auth.jwt_secret,
        &user.tenant_id,
        &user.role,
        Some(&user.username),
        auth.token_ttl_secs,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "token issue failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
    })?;
    Ok(Json(json!({
        "token": token,
        "user": { "id": user.id, "username": user.username, "role": user.role, "tenant_id": user.tenant_id },
    })))
}

/// 密钥 scope → JWT role 映射：read=read-only（仅读端点）；write/command=
/// operator（写端点经网关 RBAC 放行）。OTA/租户/用户管理等 admin-only 端点
/// 不在 operator 范围内，开放密钥一律不可达。
fn role_for_scope(scope: &str) -> &'static str {
    match scope {
        "write" | "command" => "operator",
        _ => "read-only",
    }
}

/// POST /api/access/open/token：开放 API 客户端凭 app_id/app_secret 换 JWT。
/// role 由密钥 scope 决定（read→read-only 只读；write/command→operator 可写；
/// 默认 read，向后兼容）。失败统一 401 通用文案（防枚举）。
pub async fn open_token(
    State(auth): State<AuthState>,
    Json(req): Json<OpenTokenReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if req.app_id.trim().is_empty() || req.app_secret.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "app_id and app_secret required".into(),
        ));
    }
    let (tenant_id, scope) = match auth
        .store
        .verify_api_key(req.app_id.trim(), &req.app_secret)
        .await
    {
        Ok(Some(v)) => v,
        Ok(None) | Err(_) => {
            tracing::warn!("open token failed: invalid or revoked api key");
            return Err((
                StatusCode::UNAUTHORIZED,
                "invalid app_id or app_secret".into(),
            ));
        }
    };
    let role = role_for_scope(&scope);
    // actor 用稳定的密钥 id（app_id），便于审计区分同一租户下的不同开放客户端。
    let token = issue_token_with_actor(
        &auth.jwt_secret,
        &tenant_id,
        role,
        Some(req.app_id.trim()),
        auth.token_ttl_secs,
    )
    .map_err(|e| {
        tracing::error!(error = %e, "token issue failed");
        (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
    })?;
    Ok(Json(
        json!({ "token": token, "tenant_id": tenant_id, "role": role, "scope": scope }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecat_security::crypto::hmac_sha256_hex;

    #[test]
    fn password_hash_matches_and_rejects() {
        // 旧 HMAC 格式继续可验证（存量用户）
        let pepper = "test-pepper";
        let hash = hmac_sha256_hex(pepper, b"admin123");
        assert!(password_matches(&hash, pepper, "admin123"));
        assert!(!password_matches(&hash, pepper, "wrong"));
        assert!(!password_matches(&hash, "other-pepper", "admin123"));
    }

    #[test]
    fn pbkdf2_hash_roundtrip_and_corruption() {
        let hash = hash_password("s3cret");
        assert!(hash.starts_with("pbkdf2-sha256$210000$"));
        // PBKDF2 不依赖 pepper（salt 已随机内置）
        assert!(password_matches(&hash, "any-pepper", "s3cret"));
        assert!(!password_matches(&hash, "any-pepper", "wrong"));
        // 同一口令两次哈希盐不同
        assert_ne!(hash, hash_password("s3cret"));
        // 格式损坏一律拒绝
        assert!(!password_matches("pbkdf2-sha256$", "p", "x"));
        assert!(!password_matches("pbkdf2-sha256$bad", "p", "x"));
        assert!(!password_matches("pbkdf2-sha256$210000$zz$zz", "p", "x"));
        assert!(!password_matches("pbkdf2-sha256$0$00$00", "p", "x"));
    }

    #[test]
    fn dummy_hash_costs_same_pbkdf2_and_never_matches() {
        let dummy = dummy_password_hash();
        assert!(dummy.starts_with(PBKDF2_PREFIX));
        // 未知用户也执行完整 PBKDF2（迭代数来自哑哈希本身）
        assert!(!password_matches(&dummy, "any-pepper", "guess"));
        assert!(!password_matches(&dummy, "any-pepper", ""));
    }

    #[test]
    fn role_for_scope_maps_write_scopes_to_operator() {
        assert_eq!(role_for_scope("read"), "read-only");
        assert_eq!(role_for_scope("write"), "operator");
        assert_eq!(role_for_scope("command"), "operator");
        // 未知/缺失 scope 回退只读（向后兼容）
        assert_eq!(role_for_scope(""), "read-only");
        assert_eq!(role_for_scope("admin"), "read-only");
    }
}
