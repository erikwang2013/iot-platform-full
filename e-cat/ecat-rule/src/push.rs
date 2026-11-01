use crate::models::AlertMessage;
use futures_util::StreamExt;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast};

/// 单通道容量：慢消费者会丢最旧消息（broadcast 语义），WS 侧只是展示，可接受。
const CHANNEL_CAPACITY: usize = 256;

/// 默认 WS 连接上限（可用 WS_MAX_CONNECTIONS 覆盖）。
const DEFAULT_MAX_CONNECTIONS: usize = 1024;

/// 默认单租户 WS 连接上限（可用 WS_MAX_CONNECTIONS_PER_TENANT 覆盖）。
const DEFAULT_MAX_CONNECTIONS_PER_TENANT: usize = 128;

/// 每租户一个 broadcast channel：send/subscribe 均为同步操作，
/// 用 std::sync::Mutex 保护注册表即可（临界区极短），无需 async 锁。
#[derive(Clone)]
pub struct PushHub {
    tenants: Arc<Mutex<HashMap<String, broadcast::Sender<AlertMessage>>>>,
    /// 各租户当前连接数；独立锁，避免与 tenants 锁嵌套。
    tenant_connections: Arc<Mutex<HashMap<String, usize>>>,
    connections: Arc<Semaphore>,
    per_tenant: usize,
}

/// WS 连接名额：持有全局 Semaphore 许可，Drop 时归还租户计数（无 await）。
pub struct ConnectionGuard {
    tenant: String,
    tenant_connections: Arc<Mutex<HashMap<String, usize>>>,
    _permit: OwnedSemaphorePermit,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        // Drop 中不能 panic：锁中毒按正常取用处理，避免连接关闭时 abort。
        let mut counts = self
            .tenant_connections
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let remaining = counts.get_mut(&self.tenant).map(|n| {
            *n = n.saturating_sub(1);
            *n
        });
        if remaining == Some(0) {
            counts.remove(&self.tenant);
        }
    }
}

impl Default for PushHub {
    fn default() -> Self {
        Self::new()
    }
}

impl PushHub {
    pub fn new() -> Self {
        Self::with_limit(DEFAULT_MAX_CONNECTIONS)
    }

    /// global: 全局连接上限；per_tenant: 单租户上限。0 均按 1 处理
    /// （永久拒绝所有连接不是预期配置）。
    pub fn with_limits(global: usize, per_tenant: usize) -> Self {
        Self {
            tenants: Arc::new(Mutex::new(HashMap::new())),
            tenant_connections: Arc::new(Mutex::new(HashMap::new())),
            connections: Arc::new(Semaphore::new(global.max(1))),
            per_tenant: per_tenant.max(1),
        }
    }

    /// 兼容旧调用：单租户上限取默认值。
    pub fn with_limit(max: usize) -> Self {
        Self::with_limits(max, DEFAULT_MAX_CONNECTIONS_PER_TENANT)
    }

    /// 获取一个 WS 连接名额；全局或该租户达上限返回 None（调用方回 503）。
    /// 名额须活到连接结束：ConnectionGuard drop 即归还全局许可与租户计数。
    pub fn try_connection(&self, tenant: &str) -> Option<ConnectionGuard> {
        let permit = self.connections.clone().try_acquire_owned().ok()?;
        {
            let mut counts = self.tenant_connections.lock().unwrap();
            let count = counts.entry(tenant.to_string()).or_insert(0);
            if *count >= self.per_tenant {
                return None; // 提前返回时全局许可自动释放
            }
            *count += 1;
        }
        Some(ConnectionGuard {
            tenant: tenant.to_string(),
            tenant_connections: self.tenant_connections.clone(),
            _permit: permit,
        })
    }

    /// 向租户广播告警；无订阅者时 send 返回 Err，忽略（不保留历史）。
    /// 只投递到已有 sender，绝不新建（否则无订阅者的租户会永久驻留 map）。
    pub fn publish(&self, tenant: &str, msg: &AlertMessage) {
        let sender = self.tenants.lock().unwrap().get(tenant).cloned();
        if let Some(sender) = sender {
            let _ = sender.send(msg.clone());
        }
    }

    /// 订阅即创建 sender 条目（release 时清理空条目）。
    pub fn subscribe(&self, tenant: &str) -> broadcast::Receiver<AlertMessage> {
        let mut map = self.tenants.lock().unwrap();
        map.entry(tenant.to_string())
            .or_insert_with(|| broadcast::channel(CHANNEL_CAPACITY).0)
            .subscribe()
    }

    /// 连接结束后清理已无订阅者的 sender 条目。判断与删除都在锁内，
    /// 与 subscribe 的插入串行化：新订阅要么复用条目，要么在删除后重建。
    pub fn release(&self, tenant: &str) {
        let mut map = self.tenants.lock().unwrap();
        if map.get(tenant).is_some_and(|s| s.receiver_count() == 0) {
            map.remove(tenant);
        }
    }

    #[cfg(test)]
    fn sender_count(&self) -> usize {
        self.tenants.lock().unwrap().len()
    }
}

/// Redis pub/sub 桥接通道名（固定单通道，消息内携带租户；实例按租户分发到本地 hub）。
pub const ALERT_CHANNEL: &str = "iot:alerts";

/// 跨实例广播信封：instance 为发布实例 ID，订阅端据此跳过自己发的消息
/// （避免本地重复投递——本地直达已在 publish 完成）。
#[derive(Serialize, Deserialize)]
struct BridgeEnvelope {
    instance: String,
    alert: AlertMessage,
}

/// 告警跨实例广播桥：本地直达 + Redis pub/sub 扇出。
/// - Redis 可用：publish 本地直达并发布到 Redis；订阅端收到后分发给本地 WS。
/// - Redis 不可用：降级仅本地直达（fail-open，日志告警，与其他中间件降级策略一致）。
#[derive(Clone)]
pub struct AlertBridge {
    hub: PushHub,
    redis: Option<redis::aio::MultiplexedConnection>,
    instance: Arc<str>,
}

impl AlertBridge {
    pub async fn connect(hub: PushHub, redis_url: &str) -> Self {
        let redis = match redis::Client::open(redis_url) {
            Ok(client) => match client.get_multiplexed_async_connection().await {
                Ok(conn) => Some(conn),
                Err(e) => {
                    tracing::warn!(error = %e, "alert bridge redis unavailable; local-only push");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "alert bridge redis url invalid; local-only push");
                None
            }
        };
        Self {
            hub,
            redis,
            instance: Arc::from(uuid::Uuid::new_v4().to_string()),
        }
    }

    /// 本桥实例 ID：必须与 spawn_subscriber 传入的 ID 相同，
    /// 否则订阅端无法识别自己发布的 Redis 回声，WS 会重复推送。
    pub fn instance(&self) -> Arc<str> {
        self.instance.clone()
    }

    /// 发布告警：本地直达 + Redis 扇出（Redis 失败仅本地，下次重试自然恢复）。
    pub async fn publish(&self, tenant: &str, msg: &AlertMessage) {
        self.hub.publish(tenant, msg);
        let Some(redis) = &self.redis else { return };
        // 克隆连接句柄（Arc 共享流水线，开销可忽略）以满足 AsyncCommands 的 &mut
        let mut conn = redis.clone();
        let envelope = BridgeEnvelope {
            instance: self.instance.to_string(),
            alert: msg.clone(),
        };
        let payload = serde_json::to_vec(&envelope).unwrap_or_default();
        if let Err(e) = conn.publish::<_, _, i64>(ALERT_CHANNEL, payload).await {
            tracing::warn!(error = %e, "alert bridge redis publish failed; local-only this time");
        }
    }

    /// 订阅任务：接收其他实例的告警并分发到本地 hub。断线自动重连（1s 退避）。
    /// `instance` 须取发布端 [`AlertBridge::instance`]，用于跳过自己的回声。
    pub async fn spawn_subscriber(hub: PushHub, redis_url: String, instance: Arc<str>) {
        loop {
            let client = match redis::Client::open(redis_url.as_str()) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "alert bridge subscriber: bad redis url");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            let mut pubsub = match client.get_async_pubsub().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(error = %e, "alert bridge subscriber: connect failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            };
            if let Err(e) = pubsub.subscribe(ALERT_CHANNEL).await {
                tracing::warn!(error = %e, "alert bridge subscriber: subscribe failed");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                continue;
            }
            tracing::info!("alert bridge subscriber: listening on redis channel {ALERT_CHANNEL}");
            let mut stream = pubsub.into_on_message();
            while let Some(msg) = stream.next().await {
                let payload: Vec<u8> = match msg.get_payload() {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!(error = %e, "alert bridge subscriber: bad payload");
                        continue;
                    }
                };
                let envelope: BridgeEnvelope = match serde_json::from_slice(&payload) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(error = %e, "alert bridge subscriber: drop unparseable");
                        continue;
                    }
                };
                if is_own_echo(&envelope.instance, &instance) {
                    continue; // 本地直达已投递，跳过回声
                }
                hub.publish(&envelope.alert.tenant_id, &envelope.alert);
            }
            tracing::warn!("alert bridge subscriber: stream ended; reconnecting in 1s");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }
}

/// 订阅端跳过自己发布的 Redis 回声（本地直达已在 publish 完成）。
fn is_own_echo(published_by: &str, instance: &str) -> bool {
    published_by == instance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_detected_only_for_same_instance() {
        assert!(is_own_echo("bridge-a", "bridge-a"));
        assert!(!is_own_echo("bridge-b", "bridge-a"));
    }

    fn alert(tenant: &str) -> AlertMessage {
        AlertMessage {
            rule_id: "rule".into(),
            rule_name: "rule".into(),
            tenant_id: tenant.into(),
            device_id: "dev".into(),
            code: "temp".into(),
            operator: ">".into(),
            threshold: 1.0,
            value: serde_json::json!(2.0),
            ts: 0,
        }
    }

    #[test]
    fn connection_permit_released_on_drop() {
        let hub = PushHub::with_limit(1);
        let first = hub.try_connection("t").expect("首个连接应获许可");
        assert!(hub.try_connection("t").is_none(), "超限应拒绝");
        drop(first);
        assert!(hub.try_connection("t").is_some(), "断开后名额应释放");
    }

    #[test]
    fn connection_limit_is_at_least_one() {
        let hub = PushHub::with_limit(0);
        assert!(hub.try_connection("t").is_some());
    }

    #[test]
    fn per_tenant_limit_rejects_only_that_tenant() {
        let hub = PushHub::with_limits(10, 1);
        let a = hub.try_connection("a").expect("租户 a 首个连接");
        assert!(hub.try_connection("a").is_none(), "租户 a 超限");
        assert!(hub.try_connection("b").is_some(), "其他租户不受影响");
        drop(a);
        assert!(hub.try_connection("a").is_some(), "断开后租户名额应释放");
    }

    #[test]
    fn tenant_rejection_does_not_leak_global_permit() {
        let hub = PushHub::with_limits(2, 1);
        let a = hub.try_connection("a").expect("a 首个连接");
        assert!(hub.try_connection("a").is_none(), "租户超限");
        drop(a);
        // 若租户拒绝路径泄漏全局许可，全局只剩 1 个名额，第二次会失败
        let b = hub.try_connection("b").expect("b 应获许可");
        assert!(hub.try_connection("c").is_some(), "全局名额应已全部归还");
        drop(b);
    }

    #[test]
    fn publish_does_not_insert_tenant() {
        let hub = PushHub::new();
        hub.publish("t", &alert("t"));
        assert_eq!(hub.sender_count(), 0, "publish 不应创建 sender");
        let _rx = hub.subscribe("t");
        assert_eq!(hub.sender_count(), 1, "subscribe 应创建 sender");
    }

    #[test]
    fn release_removes_sender_only_after_receivers_drop() {
        let hub = PushHub::new();
        let rx1 = hub.subscribe("t");
        let rx2 = hub.subscribe("t");
        drop(rx1);
        hub.release("t");
        assert_eq!(hub.sender_count(), 1, "仍有订阅者时不应删除");
        drop(rx2);
        hub.release("t");
        assert_eq!(hub.sender_count(), 0, "无订阅者后应删除");
    }

    #[test]
    fn publish_after_resubscribe_still_delivers() {
        let hub = PushHub::new();
        let rx = hub.subscribe("t");
        drop(rx);
        hub.release("t");
        let mut rx = hub.subscribe("t");
        hub.publish("t", &alert("t"));
        assert_eq!(rx.try_recv().expect("重新订阅后应收到告警").tenant_id, "t");
    }
}
