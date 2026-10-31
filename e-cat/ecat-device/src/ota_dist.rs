//! OTA 分发（T4.7）：可选 S3/MinIO 固件存储、CDN 签名下载 URL、下载统计。

use crate::{Db, db_err};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use ecat_data::StorageClient;
use ecat_data_s3::{S3Client, S3Config};
use ecat_security::crypto::{hmac_sha256_hex, verify_hmac_sha256_hex};
use serde_json::{Value, json};
use std::time::Duration;

/// OTA_S3_* 全有或全无；部分配置必须失败关闭，不能静默回落本地盘。
#[derive(Clone)]
struct S3Settings {
    endpoint: String,
    region: String,
    bucket: String,
    access_key: String,
    secret_key: String,
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn s3_settings_from(
    endpoint: Option<String>,
    region: Option<String>,
    bucket: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
) -> Result<Option<S3Settings>, String> {
    let n = [&endpoint, &bucket, &access_key, &secret_key]
        .iter()
        .filter(|v| v.is_some())
        .count();
    if n == 0 {
        return Ok(None);
    }
    let (Some(endpoint), Some(bucket), Some(access_key), Some(secret_key)) =
        (endpoint, bucket, access_key, secret_key)
    else {
        return Err("OTA_S3_* partially configured".into());
    };
    Ok(Some(S3Settings {
        endpoint,
        region: region.unwrap_or_else(|| "us-east-1".into()),
        bucket,
        access_key,
        secret_key,
    }))
}

fn s3_settings() -> Result<Option<S3Settings>, String> {
    s3_settings_from(
        non_empty_env("OTA_S3_ENDPOINT"),
        non_empty_env("OTA_S3_REGION"),
        non_empty_env("OTA_S3_BUCKET"),
        non_empty_env("OTA_S3_ACCESS_KEY"),
        non_empty_env("OTA_S3_SECRET_KEY"),
    )
}

fn s3_client(settings: &S3Settings) -> Result<S3Client, String> {
    S3Client::from_config(S3Config {
        endpoint: settings.endpoint.clone(),
        region: settings.region.clone(),
        access_key: settings.access_key.clone(),
        secret_key: settings.secret_key.clone(),
        tls: None,
        query_timeout_secs: None,
        breaker: None,
        max_concurrency: None,
    })
    .map_err(|e| format!("s3 client: {e}"))
}

/// 对象键：firmware/{tenant}/{id}.bin（tenant 已由中间件限制为 [A-Za-z0-9_-]）。
pub fn s3_key(tenant: &str, id: i64) -> String {
    format!("firmware/{tenant}/{id}.bin")
}

fn parse_s3_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("s3://")?;
    let (bucket, key) = rest.split_once('/')?;
    (!bucket.is_empty() && !key.is_empty()).then_some((bucket, key))
}

fn public_base() -> Option<String> {
    non_empty_env("OTA_PUBLIC_BASE").map(|b| b.trim_end_matches('/').to_string())
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 本地固件下载 bearer token：{id}.{tenant}.{exp}.{hmac}
/// tenant 字符集与网关租户门一致（[A-Za-z0-9_-]），保证 '.' 分隔无歧义。
pub fn sign_download_token(secret: &str, id: i64, tenant: &str, expires_at: i64) -> String {
    let sig = hmac_sha256_hex(secret, format!("{id}|{tenant}|{expires_at}").as_bytes());
    format!("{id}.{tenant}.{expires_at}.{sig}")
}

/// 验签并校验有效期；返回 (firmware_id, tenant)。
pub fn verify_download_token(secret: &str, token: &str, now: i64) -> Result<(i64, String), String> {
    let invalid = || "invalid download token".to_string();
    let mut parts = token.split('.');
    let (Some(id), Some(tenant), Some(exp), Some(sig)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    if parts.next().is_some() {
        return Err(invalid());
    }
    let id: i64 = id.parse().map_err(|_| invalid())?;
    let exp: i64 = exp.parse().map_err(|_| invalid())?;
    let tenant_ok = !tenant.is_empty()
        && tenant.len() <= 64
        && tenant
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !tenant_ok {
        return Err(invalid());
    }
    if exp <= now {
        return Err("download token expired".into());
    }
    if !verify_hmac_sha256_hex(secret, format!("{id}|{tenant}|{exp}").as_bytes(), sig) {
        return Err(invalid());
    }
    Ok((id, tenant.to_string()))
}

/// 存储 url → CDN 可签的公开源站 URL。
/// http(s) 原样返回；s3://bucket/key → {base}/{bucket}/{key}；相对路径 → {base}{url}。
pub fn origin_url(stored: &str, public_base: Option<&str>) -> Result<String, String> {
    if stored.starts_with("http://") || stored.starts_with("https://") {
        return Ok(stored.to_string());
    }
    let base = public_base
        .map(|b| b.trim_end_matches('/'))
        .filter(|b| !b.is_empty())
        .ok_or("OTA_PUBLIC_BASE required for relative/s3 firmware url")?;
    if let Some((bucket, key)) = parse_s3_url(stored) {
        return Ok(format!("{base}/{bucket}/{key}"));
    }
    if stored.starts_with('/') {
        return Ok(format!("{base}{stored}"));
    }
    Err("firmware url must be http(s), s3://, or a path starting with /".into())
}

/// 签名有效期边界：60s ~ 24h（与 ecat-cdn 一致）。
pub fn validate_expires(secs: u64) -> Result<(), String> {
    if (60..=86400).contains(&secs) {
        Ok(())
    } else {
        Err("expires_secs must be 60..=86400".into())
    }
}

fn sign_expires_default() -> Result<u64, String> {
    match non_empty_env("OTA_SIGN_EXPIRES_SECS") {
        None => Ok(3600),
        Some(v) => {
            let secs: u64 = v
                .parse()
                .map_err(|_| "OTA_SIGN_EXPIRES_SECS must be a number".to_string())?;
            validate_expires(secs)?;
            Ok(secs)
        }
    }
}

fn cdn_provider() -> Option<String> {
    non_empty_env("OTA_CDN_PROVIDER_ID")
}

/// 调 ecat-cdn 签名：失败只回 Err（上层净化文案），错误信息不含密钥。
async fn sign_with_cdn(
    provider_id: &str,
    tenant: &str,
    url: &str,
    expires_secs: u64,
) -> Result<String, String> {
    let base = non_empty_env("CDN_BASE").unwrap_or_else(|| "http://localhost:8085".to_string());
    let endpoint = format!(
        "{}/api/cdn/providers/{provider_id}/signed-url",
        base.trim_end_matches('/')
    );
    let secret = std::env::var("IOT_GATEWAY_SECRET").unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| format!("cdn client: {e}"))?;
    let resp = client
        .post(&endpoint)
        .header("x-gateway-secret", secret)
        .header("x-tenant-id", tenant)
        .json(&json!({"url": url, "expires_secs": expires_secs}))
        .send()
        .await
        .map_err(|e| format!("cdn sign request: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("cdn sign: HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("cdn sign body: {e}"))?;
    body.get("url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "cdn sign: missing url".to_string())
}

/// 上传固件：S3 已配置时返回 Some(s3://...) 供入库，未配置返回 None（由调用方落本地盘）。
/// 部分配置返回 500，绝不静默回落。
pub async fn store_firmware(
    tenant: &str,
    id: i64,
    bytes: &[u8],
) -> Result<Option<String>, (StatusCode, String)> {
    let settings = s3_settings().map_err(|e| {
        tracing::warn!(error = %e, "ota s3 config invalid");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "ota s3 config incomplete".to_string(),
        )
    })?;
    let Some(settings) = settings else {
        return Ok(None);
    };
    let client = s3_client(&settings).map_err(|e| {
        tracing::warn!(error = %e, "ota s3 client failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "firmware storage failed".to_string(),
        )
    })?;
    let key = s3_key(tenant, id);
    client
        .put(&settings.bucket, &key, bytes)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "ota s3 put failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "firmware storage failed".to_string(),
            )
        })?;
    Ok(Some(format!("s3://{}/{key}", settings.bucket)))
}

/// 读取 s3:// 固件对象。
pub async fn fetch_firmware(url: &str) -> Result<Vec<u8>, (StatusCode, String)> {
    let (bucket, key) = parse_s3_url(url).ok_or((
        StatusCode::BAD_REQUEST,
        "invalid s3 firmware url".to_string(),
    ))?;
    let settings = s3_settings().map_err(|e| {
        tracing::warn!(error = %e, "ota s3 config invalid");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "ota s3 config incomplete".to_string(),
        )
    })?;
    let Some(settings) = settings else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "ota s3 config missing".to_string(),
        ));
    };
    let client = s3_client(&settings).map_err(|e| {
        tracing::warn!(error = %e, "ota s3 client failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "firmware storage failed".to_string(),
        )
    })?;
    client.get(bucket, key).await.map_err(|e| {
        tracing::warn!(error = %e, "ota s3 get failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "firmware storage read failed".to_string(),
        )
    })
}

/// 删除 s3:// 固件对象。
pub async fn delete_firmware_object(url: &str) -> Result<(), (StatusCode, String)> {
    let (bucket, key) = parse_s3_url(url).ok_or((
        StatusCode::BAD_REQUEST,
        "invalid s3 firmware url".to_string(),
    ))?;
    let settings = s3_settings().map_err(|e| {
        tracing::warn!(error = %e, "ota s3 config invalid");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "ota s3 config incomplete".to_string(),
        )
    })?;
    let Some(settings) = settings else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "ota s3 config missing".to_string(),
        ));
    };
    let client = s3_client(&settings).map_err(|e| {
        tracing::warn!(error = %e, "ota s3 client failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "firmware storage failed".to_string(),
        )
    })?;
    client.delete(bucket, key).await.map_err(|e| {
        tracing::warn!(error = %e, "ota s3 delete failed");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "firmware storage delete failed".to_string(),
        )
    })
}

/// 下载计数（发生在固件字节读取成功之后）。
pub async fn record_download(
    db: &Db,
    tenant: &str,
    firmware_id: i64,
) -> Result<(), (StatusCode, String)> {
    db.0.execute_with(
        "INSERT INTO ota_downloads (id, tenant_id, firmware_id) VALUES (?, ?, ?)",
        &[
            json!(ecat::ids::next_id()),
            json!(tenant),
            json!(firmware_id),
        ],
    )
    .await
    .map(|_| ())
    .map_err(db_err)
}

/// 本地固件（相对路径）→ 公开下载端点的短期 token URL；无 base/secret 则 None。
fn local_download_url(tenant: &str, firmware_id: i64, expires_secs: u64) -> Option<String> {
    let base = public_base()?;
    let secret = std::env::var("IOT_GATEWAY_SECRET").ok()?;
    let token = sign_download_token(
        &secret,
        firmware_id,
        tenant,
        now_secs() + expires_secs as i64,
    );
    Some(format!(
        "{base}/api/v1/ota/public/firmwares/{firmware_id}/download?token={token}"
    ))
}

/// 任务创建响应里的 download_url：签名失败退化为未签名公开 URL，构建不出来则省略。
/// 同一固件只查一次、只签一次（批量下发复用）。
pub async fn download_url_for(db: &Db, tenant: &str, firmware_id: i64) -> Option<Value> {
    let rows =
        db.0.query_with(
            "SELECT url FROM ota_firmwares WHERE id = ? AND tenant_id = ?",
            &[json!(firmware_id), json!(tenant)],
        )
        .await
        .ok()?;
    let stored = rows.first()?.get("url")?.as_str()?;
    // 本地落盘：token 即签名（设备没有网关 secret，不能再走受保护下载端点）
    if stored.starts_with('/') {
        let expires = sign_expires_default().ok()?;
        let url = local_download_url(tenant, firmware_id, expires)?;
        return Some(json!({"url": url, "signed": true}));
    }
    let origin = origin_url(stored, public_base().as_deref()).ok()?;
    let expires = sign_expires_default().ok()?;
    if let Some(provider) = cdn_provider()
        && let Ok(signed) = sign_with_cdn(&provider, tenant, &origin, expires).await
    {
        return Some(json!({"url": signed, "signed": true}));
    }
    Some(json!({"url": origin, "signed": false}))
}

#[derive(serde::Deserialize)]
pub struct SignedUrlReq {
    #[serde(default)]
    pub expires_secs: Option<u64>,
}

/// POST /api/ota/firmwares/{id}/signed-url：返回 CDN 签名下载 URL（未配 provider 则未签名）。
pub async fn signed_firmware_url(
    State(db): State<Db>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
    Json(req): Json<SignedUrlReq>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let id_n: i64 = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid firmware id".into()))?;
    let expires = match req.expires_secs {
        Some(secs) => {
            validate_expires(secs).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
            secs
        }
        None => sign_expires_default().map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?,
    };
    let rows =
        db.0.query_with(
            "SELECT url FROM ota_firmwares WHERE id = ? AND tenant_id = ?",
            &[json!(id_n), json!(tenant.as_str())],
        )
        .await
        .map_err(db_err)?;
    let Some(stored) = rows
        .first()
        .and_then(|r| r.get("url"))
        .and_then(Value::as_str)
    else {
        return Err((StatusCode::NOT_FOUND, "firmware not found".into()));
    };
    // 本地落盘：token 即签名（CDN provider 只适用于 s3/绝对源站 URL）
    if stored.starts_with('/') {
        let url = local_download_url(tenant.as_str(), id_n, expires).ok_or((
            StatusCode::BAD_REQUEST,
            "OTA_PUBLIC_BASE required for local firmware url".to_string(),
        ))?;
        return Ok(Json(
            json!({"url": url, "expires_secs": expires, "signed": true}),
        ));
    }
    let origin =
        origin_url(stored, public_base().as_deref()).map_err(|e| (StatusCode::BAD_REQUEST, e))?;
    let Some(provider) = cdn_provider() else {
        return Ok(Json(
            json!({"url": origin, "expires_secs": expires, "signed": false}),
        ));
    };
    let signed = sign_with_cdn(&provider, tenant.as_str(), &origin, expires)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "cdn signed-url failed");
            (StatusCode::BAD_GATEWAY, "cdn signing failed".to_string())
        })?;
    Ok(Json(
        json!({"url": signed, "expires_secs": expires, "signed": true}),
    ))
}

/// GET /api/ota/firmwares/{id}/downloads：下载次数与最近一次时间（租户隔离）。
pub async fn firmware_downloads(
    State(db): State<Db>,
    tenant: axum::Extension<String>,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let id_n: i64 = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "invalid firmware id".into()))?;
    let owned =
        db.0.query_with(
            "SELECT id FROM ota_firmwares WHERE id = ? AND tenant_id = ?",
            &[json!(id_n), json!(tenant.as_str())],
        )
        .await
        .map_err(db_err)?;
    if owned.is_empty() {
        return Err((StatusCode::NOT_FOUND, "firmware not found".into()));
    }
    let rows =
        db.0.query_with(
            "SELECT COUNT(*) AS n, CAST(MAX(created_at) AS CHAR) AS last_at \
             FROM ota_downloads WHERE tenant_id = ? AND firmware_id = ?",
            &[json!(tenant.as_str()), json!(id_n)],
        )
        .await
        .map_err(db_err)?;
    let row = rows.first();
    let count = row
        .and_then(|r| r.get("n"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let last_at = row
        .and_then(|r| r.get("last_at"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(json!({
        "firmware_id": id_n.to_string(),
        "count": count,
        "last_at": last_at,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s3_key_builder() {
        assert_eq!(s3_key("acme", 42), "firmware/acme/42.bin");
        assert_eq!(
            parse_s3_url("s3://bucket/firmware/acme/42.bin"),
            Some(("bucket", "firmware/acme/42.bin"))
        );
        assert_eq!(parse_s3_url("s3://bucket"), None);
        assert_eq!(parse_s3_url("/local/path"), None);
    }

    #[test]
    fn origin_url_forms() {
        // 绝对 URL 原样返回，无需 public base
        assert_eq!(
            origin_url("https://cdn.example/fw.bin", None).unwrap(),
            "https://cdn.example/fw.bin"
        );
        // s3 对象 → base/bucket/key
        assert_eq!(
            origin_url("s3://bucket/firmware/acme/42.bin", Some("http://origin/")).unwrap(),
            "http://origin/bucket/firmware/acme/42.bin"
        );
        // 相对路径 → base + url
        assert_eq!(
            origin_url("/api/ota/firmwares/42/download", Some("http://origin")).unwrap(),
            "http://origin/api/ota/firmwares/42/download"
        );
        // 缺少 public base 的 s3/相对路径必须报错
        assert!(origin_url("s3://bucket/key.bin", None).is_err());
        assert!(origin_url("/api/ota/firmwares/42/download", None).is_err());
        assert!(origin_url("ftp://host/fw.bin", Some("http://origin")).is_err());
    }
    #[test]
    fn expires_bounds() {
        assert!(validate_expires(60).is_ok());
        assert!(validate_expires(3600).is_ok());
        assert!(validate_expires(86400).is_ok());
        assert!(validate_expires(59).is_err());
        assert!(validate_expires(86401).is_err());
    }

    #[test]
    fn download_token_roundtrip() {
        let token = sign_download_token("s3cr3t", 42, "acme-1", 2_000_000_000);
        assert_eq!(
            verify_download_token("s3cr3t", &token, 1_000_000_000).unwrap(),
            (42, "acme-1".to_string())
        );
        // 过期（exp == now 也算过期）
        assert!(verify_download_token("s3cr3t", &token, 2_000_000_000).is_err());
        // 密钥不符 / 篡改 id / 篡改 tenant
        assert!(verify_download_token("other", &token, 1_000_000_000).is_err());
        let parts: Vec<&str> = token.split('.').collect();
        let tampered_id = format!("43.{}.{}.{}", parts[1], parts[2], parts[3]);
        assert!(verify_download_token("s3cr3t", &tampered_id, 1_000_000_000).is_err());
        let tampered_tenant = format!("{}.other.{}.{}", parts[0], parts[2], parts[3]);
        assert!(verify_download_token("s3cr3t", &tampered_tenant, 1_000_000_000).is_err());
    }

    #[test]
    fn download_token_rejects_malformed() {
        for bad in [
            "",
            "1.2.3",
            "1.acme.2000000000.deadbeef.extra",
            "x.acme.2000000000.deadbeef",
            "42.acme.not-a-number.deadbeef",
            "42.ac me.2000000000.deadbeef",
            "42..2000000000.deadbeef",
        ] {
            assert!(
                verify_download_token("s3cr3t", bad, 0).is_err(),
                "should reject {bad:?}"
            );
        }
    }
    #[test]
    fn s3_partial_config_detection() {
        let full = || {
            (
                Some("http://minio:9000".to_string()),
                None,
                Some("bucket".to_string()),
                Some("ak".to_string()),
                Some("sk".to_string()),
            )
        };
        // 全空 → 未启用（未配置时上传落本地盘）
        assert!(
            s3_settings_from(None, None, None, None, None)
                .unwrap()
                .is_none()
        );
        // 只给 region（有默认值）→ 未启用
        let only_region =
            s3_settings_from(None, Some("eu-west-1".into()), None, None, None).unwrap();
        assert!(only_region.is_none());
        // 四项齐全 → 启用，region 默认 us-east-1
        let (e, r, b, a, s) = full();
        let cfg = s3_settings_from(e, r, b, a, s).unwrap().unwrap();
        assert_eq!(cfg.region, "us-east-1");
        assert_eq!(cfg.bucket, "bucket");
        // 缺任一必填项 → Err（失败关闭）
        let (e, r, b, a, _) = full();
        assert!(s3_settings_from(e, r, b, a, None).is_err());
    }
}
