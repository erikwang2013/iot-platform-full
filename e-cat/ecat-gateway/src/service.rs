//! 服务发现（B-3）：网关按服务名解析上游 base URL。
//! 解析优先级：环境变量（ACCESS_BASE 等）→ Consul 服务发现（CONSUL_URL 配置时）
//! → 内置默认。Consul 命中结果按服务名缓存，TTL 后重新查询——实例下线/迁移时
//! 永不过期的缓存会把请求持续打到失效地址。
use ecat_registry::Registry;
use ecat_registry_consul::ConsulRegistry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Consul 缓存默认 TTL（SERVICE_DISCOVERY_TTL_SECS 覆盖；0 = 每次请求都重查）。
const DEFAULT_DISCOVERY_TTL_SECS: u64 = 30;

/// 服务发现解析器。`CONSUL_URL` 未配置时退化为纯 env/默认（不联网）。
#[derive(Clone)]
pub struct ServiceResolver {
    consul: Option<ConsulRegistry>,
    /// service_name → (base_url, 写入时刻)
    cache: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    ttl: Duration,
}

impl ServiceResolver {
    /// 读取 CONSUL_URL 构建解析器；未配置时为 None（纯 env/默认直连）。
    pub fn new() -> Self {
        let consul = match std::env::var("CONSUL_URL") {
            Ok(url) => match ConsulRegistry::new(url) {
                Ok(r) => {
                    tracing::info!("service discovery: consul enabled");
                    Some(r)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "service discovery: invalid CONSUL_URL; fallback to env/direct");
                    None
                }
            },
            Err(_) => None,
        };
        let ttl = std::env::var("SERVICE_DISCOVERY_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DISCOVERY_TTL_SECS);
        Self {
            consul,
            cache: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(ttl),
        }
    }

    /// 解析服务 base URL。优先环境变量（兼容现状），否则 Consul 发现。
    /// 返回 Some(base)；None 表示无发现且无覆盖 → 调用方用内置默认。
    pub async fn resolve(&self, env_var: &str, service_name: &str) -> Option<String> {
        // 1) 环境变量优先（显式覆盖/本地开发，兼容旧行为）
        if let Ok(v) = std::env::var(env_var) {
            return Some(v);
        }
        // 2) Consul 服务发现（未过期缓存命中直接返回）
        let Some(consul) = &self.consul else {
            return None;
        };
        if let Some(v) = self.cached(service_name) {
            return Some(v);
        }
        let resolved = match consul.discover(service_name).await {
            Ok(svcs) => {
                // 取第一个实例的首个端点；地址可能为 host:port，规范化为 http://
                svcs.first()
                    .and_then(|s| s.endpoints.first().cloned())
                    .map(|e| normalize_endpoint(&e))
            }
            Err(e) => {
                tracing::warn!(service = %service_name, error = %e, "service discovery failed; using env/default");
                // 可用性优先：Consul 暂时不可达时沿用过期缓存，避免整个转发链 502
                if let Some(stale) = self.stale(service_name) {
                    tracing::warn!(service = %service_name, base = %stale, "using stale cached endpoint");
                    return Some(stale);
                }
                None
            }
        };
        if let Some(base) = resolved.clone() {
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(service_name.to_string(), (base.clone(), Instant::now()));
        }
        resolved
    }

    /// 未过期的缓存值。
    fn cached(&self, service_name: &str) -> Option<String> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .get(service_name)
            .filter(|(_, at)| at.elapsed() < self.ttl)
            .map(|(base, _)| base.clone())
    }

    /// 过期缓存值：仅 Consul 查询失败时兜底。
    fn stale(&self, service_name: &str) -> Option<String> {
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(service_name)
            .map(|(base, _)| base.clone())
    }
}

impl Default for ServiceResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// 将 Consul 端点规范化为 base URL：无 scheme 时补 http://，去掉尾部斜杠。
fn normalize_endpoint(e: &str) -> String {
    let trimmed = e.trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{trimmed}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_consul_when_env_absent() {
        // 未配置 CONSUL_URL → resolver 不启用 Consul（纯 env/默认）
        let r = ServiceResolver::new();
        assert!(r.consul.is_none());
    }

    #[test]
    fn normalize_endpoint_adds_scheme() {
        assert_eq!(normalize_endpoint("10.0.0.1:8082"), "http://10.0.0.1:8082");
        assert_eq!(normalize_endpoint("http://svc:8082/"), "http://svc:8082");
        assert_eq!(normalize_endpoint("https://svc:8082"), "https://svc:8082");
    }

    /// 缓存 TTL：未过期命中、过期不再作为新解析结果，但 Consul 失败时可兜底。
    #[test]
    fn cache_expires_after_ttl_but_remains_stale_fallback() {
        let r = ServiceResolver {
            consul: None,
            cache: Arc::new(Mutex::new(HashMap::new())),
            ttl: Duration::from_secs(30),
        };
        r.cache.lock().unwrap().insert(
            "iot-device".to_string(),
            ("http://dev:8081".to_string(), Instant::now()),
        );
        assert_eq!(r.cached("iot-device").as_deref(), Some("http://dev:8081"));

        r.cache.lock().unwrap().get_mut("iot-device").unwrap().1 =
            Instant::now() - Duration::from_secs(31);
        assert_eq!(r.cached("iot-device"), None, "expired entry must be re-resolved");
        assert_eq!(
            r.stale("iot-device").as_deref(),
            Some("http://dev:8081"),
            "stale entry stays available as availability fallback"
        );
    }
}
