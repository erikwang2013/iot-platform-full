//! 规则/场景/渠道热路径读缓存：进程内 TTL 缓存，TTL 0 关闭。
//! 多副本部署时写操作只失效本进程缓存，其他副本最多陈旧一个 TTL（默认 5s）。
//! 查询失败不入缓存；空结果（合法）照常缓存。

use crate::models::{NotifyChannel, Rule, Scene};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct Entry<T> {
    expires: Instant,
    value: T,
}

// 列表按 Arc 共享：热路径每次事件只克隆指针，不再克隆整个规则/场景/渠道列表。
type RuleMap = HashMap<String, Entry<Arc<Vec<Rule>>>>;
type SceneKey = (String, bool);
type SceneMap = HashMap<SceneKey, Entry<Arc<Vec<Scene>>>>;
type ChannelMap = HashMap<String, Entry<Arc<Vec<NotifyChannel>>>>;

pub struct RuleCache {
    ttl: Duration,
    max_tenants: usize,
    rules: Mutex<RuleMap>,
    scenes: Mutex<SceneMap>,
    channels: Mutex<ChannelMap>,
}

impl RuleCache {
    pub fn new(ttl_secs: u64, max_tenants: usize) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            max_tenants: max_tenants.max(1),
            rules: Mutex::new(HashMap::new()),
            scenes: Mutex::new(HashMap::new()),
            channels: Mutex::new(HashMap::new()),
        }
    }

    /// env：RULE_CACHE_TTL_SECS（默认 5，0 关闭）、RULE_CACHE_MAX_TENANTS（默认 10000）。
    pub fn from_env() -> Self {
        let ttl = std::env::var("RULE_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);
        let max = std::env::var("RULE_CACHE_MAX_TENANTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10_000);
        Self::new(ttl, max)
    }

    pub fn get_rules(&self, tenant_id: &str) -> Option<Arc<Vec<Rule>>> {
        let now = Instant::now();
        let map = lock(&self.rules);
        map.get(tenant_id)
            .filter(|e| e.expires > now)
            .map(|e| Arc::clone(&e.value))
    }

    pub fn put_rules(&self, tenant_id: &str, rules: &[Rule]) {
        put(
            &self.rules,
            self.max_tenants,
            self.ttl,
            tenant_id.to_string(),
            Arc::new(rules.to_vec()),
            Instant::now(),
        );
    }

    pub fn invalidate_rules(&self, tenant_id: &str) {
        lock(&self.rules).remove(tenant_id);
    }

    pub fn get_scenes(&self, tenant_id: &str, enabled_only: bool) -> Option<Arc<Vec<Scene>>> {
        let now = Instant::now();
        let key = (tenant_id.to_string(), enabled_only);
        let map = lock(&self.scenes);
        map.get(&key)
            .filter(|e| e.expires > now)
            .map(|e| Arc::clone(&e.value))
    }

    pub fn put_scenes(&self, tenant_id: &str, enabled_only: bool, scenes: &[Scene]) {
        put(
            &self.scenes,
            self.max_tenants,
            self.ttl,
            (tenant_id.to_string(), enabled_only),
            Arc::new(scenes.to_vec()),
            Instant::now(),
        );
    }

    /// 两种 enabled_only 变体一起失效（写场景会影响两个列表）。
    pub fn invalidate_scenes(&self, tenant_id: &str) {
        lock(&self.scenes).retain(|(t, _), _| t != tenant_id);
    }

    pub fn get_channels(&self, tenant_id: &str) -> Option<Arc<Vec<NotifyChannel>>> {
        let now = Instant::now();
        let map = lock(&self.channels);
        map.get(tenant_id)
            .filter(|e| e.expires > now)
            .map(|e| Arc::clone(&e.value))
    }

    pub fn put_channels(&self, tenant_id: &str, channels: &[NotifyChannel]) {
        put(
            &self.channels,
            self.max_tenants,
            self.ttl,
            tenant_id.to_string(),
            Arc::new(channels.to_vec()),
            Instant::now(),
        );
    }

    pub fn invalidate_channels(&self, tenant_id: &str) {
        lock(&self.channels).remove(tenant_id);
    }
}

/// 锁中毒按正常取用处理：缓存内容无一致性约束，单个 panic 不应拖垮热路径。
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn put<K: Eq + std::hash::Hash, T>(
    map: &Mutex<HashMap<K, Entry<T>>>,
    max: usize,
    ttl: Duration,
    key: K,
    value: T,
    now: Instant,
) {
    if ttl.is_zero() {
        return;
    }
    let mut map = lock(map);
    // shortcut: 超容量整体清空该 map，写少读多场景够用；需要时改 LRU/分片。
    if map.len() >= max && !map.contains_key(&key) {
        map.clear();
    }
    map.insert(
        key,
        Entry {
            expires: now + ttl,
            value,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Rule;
    use serde_json::json;

    fn sample_rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            tenant_id: "t1".into(),
            name: "r".into(),
            device_id: "1".into(),
            code: "temp".into(),
            operator: "gt".into(),
            threshold: 1.0,
            webhook_url: None,
            action_device_id: None,
            action_code: None,
            action_value: None,
            enabled: true,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn rules_cache_hit_and_invalidate() {
        let c = RuleCache::new(60, 10);
        assert!(c.get_rules("t1").is_none());
        c.put_rules("t1", &[sample_rule("r1")]);
        assert_eq!(c.get_rules("t1").unwrap().len(), 1);
        // 其他租户互不影响
        assert!(c.get_rules("t2").is_none());
        c.invalidate_rules("t1");
        assert!(c.get_rules("t1").is_none());
    }

    #[test]
    fn cached_lists_are_shared_by_arc() {
        let c = RuleCache::new(60, 10);
        c.put_rules("t1", &[sample_rule("r1")]);
        let a = c.get_rules("t1").unwrap();
        let b = c.get_rules("t1").unwrap();
        // 热路径只克隆 Arc，不复制底层列表
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn ttl_zero_disables_cache() {
        let c = RuleCache::new(0, 10);
        c.put_rules("t1", &[sample_rule("r1")]);
        assert!(c.get_rules("t1").is_none());
    }

    #[test]
    fn entry_expires_after_ttl() {
        let c = RuleCache::new(1, 10);
        c.put_rules("t1", &[sample_rule("r1")]);
        assert!(c.get_rules("t1").is_some());
        std::thread::sleep(Duration::from_millis(1_200));
        assert!(c.get_rules("t1").is_none());
    }

    #[test]
    fn scenes_keyed_by_enabled_only_and_invalidated_together() {
        let c = RuleCache::new(60, 10);
        let scene = Scene {
            id: "1".into(),
            tenant_id: "t1".into(),
            name: "s".into(),
            enabled: true,
            conditions: json!([]),
            actions: json!([]),
            last_triggered_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        c.put_scenes("t1", true, std::slice::from_ref(&scene));
        assert!(c.get_scenes("t1", true).is_some());
        assert!(c.get_scenes("t1", false).is_none());
        c.put_scenes("t2", false, &[scene]);
        c.invalidate_scenes("t1");
        assert!(c.get_scenes("t1", true).is_none());
        // 其他租户的缓存保留
        assert!(c.get_scenes("t2", false).is_some());
    }

    #[test]
    fn capacity_overflow_clears_map() {
        let c = RuleCache::new(60, 2);
        c.put_rules("t1", &[sample_rule("r1")]);
        c.put_rules("t2", &[sample_rule("r2")]);
        assert!(c.get_rules("t1").is_some());
        // 第三个租户触发整体清空，只剩新键
        c.put_rules("t3", &[sample_rule("r3")]);
        assert!(c.get_rules("t1").is_none());
        assert!(c.get_rules("t2").is_none());
        assert!(c.get_rules("t3").is_some());
        // 覆写已有键不触发清空
        c.put_rules("t3", &[sample_rule("r4")]);
        assert_eq!(c.get_rules("t3").unwrap()[0].id, "r4");
    }
}
