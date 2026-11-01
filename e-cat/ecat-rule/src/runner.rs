use crate::engine::{TOPIC_EVENTS, evaluate};
use crate::models::{AlertMessage, EventMessage, Scene};
use crate::push::AlertBridge;
use crate::scenes::{SceneCooldown, conditions_match, execute_scene, extract_device_ids};
use crate::store::RuleStore;
use ecat_iot::CommandEvent;
use ecat_mq::MessageQueue;
use ecat_mq_kafka::KafkaMq;
use futures_util::{StreamExt, stream::poll_fn};
use std::sync::Arc;

/// 每事件最多评估的场景数（单租户大量场景会拖慢共享消费循环）。
const MAX_SCENES_PER_EVENT: usize = 100;
/// 同时在跑的场景动作任务上限（每个任务最多 20 个 5s 的 HTTP 动作）。
const MAX_INFLIGHT_SCENES: usize = 16;

/// 冷却一律用墙钟毫秒：事件 ts 由设备提供，未来时间戳会长期压制场景。
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn select_scenes(scenes: Vec<Scene>) -> Vec<Scene> {
    if scenes.len() > MAX_SCENES_PER_EVENT {
        tracing::warn!(
            total = scenes.len(),
            cap = MAX_SCENES_PER_EVENT,
            "scene list capped per event"
        );
    }
    scenes.into_iter().take(MAX_SCENES_PER_EVENT).collect()
}

enum SceneClaim {
    Skip,
    Busy,
    Run(tokio::sync::OwnedSemaphorePermit),
}

/// 条件/冷却通过且非阻塞拿到执行许可后，才登记冷却时间；
/// 许可饱和时返回 Busy，不消耗冷却窗口（下个事件可重试）。
fn claim_scene(
    scene: &Scene,
    ev: &EventMessage,
    latest: &mut std::collections::HashMap<(String, String, String), serde_json::Value>,
    cooldown: &mut SceneCooldown,
    limit: &Arc<tokio::sync::Semaphore>,
    now_ms: i64,
) -> SceneClaim {
    if !conditions_match(&scene.conditions, ev, latest, &scene.tenant_id)
        || !cooldown.allows(&scene.id, now_ms)
    {
        return SceneClaim::Skip;
    }
    let Ok(permit) = limit.clone().try_acquire_owned() else {
        return SceneClaim::Busy;
    };
    cooldown.mark_triggered(&scene.id, now_ms);
    SceneClaim::Run(permit)
}

/// 告警 → webhook 请求体（AlertMessage JSON，与 WS 推送同一形状）。
pub fn webhook_payload(msg: &AlertMessage) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_default()
}

/// 告警去重窗口（毫秒）：同一 (rule_id, device_id, code) 在该窗口内不重复
/// 投递（事件风暴防护，D-1）。env ALERT_DEDUP_WINDOW_SECS 默认 300s。
fn dedup_window_ms() -> i64 {
    std::env::var("ALERT_DEDUP_WINDOW_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(|s: i64| s * 1000)
        .unwrap_or(300 * 1000)
}

/// 去重表条目阈值：达到即触发一次过期清理（长时间运行也不会无界增长）。
const EVICT_THRESHOLD: usize = 4096;

/// 告警去重器：记录每个 (rule_id, device_id, code) 最近投递时间。
/// 非线程安全——仅 runner 单任务使用（std::collections::HashMap + Mutex 亦可，
/// 此处简化：runner 内单一消费循环串行访问）。
pub struct AlertDedup {
    last: std::collections::HashMap<(String, String, String), i64>,
    window_ms: i64,
    /// 上次清理时间：每过一个窗口清理过期键。
    last_evict_ms: i64,
}

impl AlertDedup {
    pub fn new() -> Self {
        Self {
            last: std::collections::HashMap::new(),
            window_ms: dedup_window_ms(),
            last_evict_ms: 0,
        }
    }

    /// 判断该告警是否应在窗口内抑制。返回 true 表示应投递。
    /// now_ms 为墙钟毫秒（不用 msg.ts：设备时间戳可伪造到未来，会长期抑制）。
    /// 窗口 <= 0 时全部投递（关闭去重）。
    pub fn should_deliver(&mut self, msg: &AlertMessage, now_ms: i64) -> bool {
        if self.window_ms <= 0 {
            return true;
        }
        if now_ms.saturating_sub(self.last_evict_ms) >= self.window_ms
            || self.last.len() >= EVICT_THRESHOLD
        {
            self.evict(now_ms);
        }
        let key = (msg.rule_id.clone(), msg.device_id.clone(), msg.code.clone());
        let suppress = self
            .last
            .get(&key)
            .map(|last_ts| now_ms - *last_ts < self.window_ms)
            .unwrap_or(false);
        if !suppress {
            self.last.insert(key, now_ms);
        }
        !suppress
    }

    /// 移除已过窗口的键；窗口内的活跃键保留，抑制语义不变。
    fn evict(&mut self, now_ms: i64) {
        let window = self.window_ms;
        self.last
            .retain(|_, ts| now_ms.saturating_sub(*ts) < window);
        self.last_evict_ms = now_ms;
    }
}

impl Default for AlertDedup {
    fn default() -> Self {
        Self::new()
    }
}

/// 后台任务：消费 iot.events → 按事件租户加载规则 → 纯函数 evaluate →
/// 命中则 WS 推送 + 落告警记录 + 可选 webhook；kind=anomaly（AI 异常检测
/// 产出）直接入告警流（检测器侧已做 z-score 阈值 + 冷却，无需规则匹配）。
/// ponytail: 每事件查一次规则（MVP 量级足够；量大时改为定时重载 + 内存缓存）。
pub async fn run(
    kafka: Arc<KafkaMq>,
    store: Arc<RuleStore>,
    bridge: AlertBridge,
    http: reqwest::Client,
) {
    let mut stream = match kafka.subscribe(TOPIC_EVENTS).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "kafka subscribe failed, rule engine exits");
            return;
        }
    };
    let mut stream = poll_fn(move |cx| stream.poll_recv(cx)).boxed();
    // 告警去重（D-1）：同一告警在窗口内只投递一次，防事件风暴重复通知
    let mut dedup = AlertDedup::new();
    // 场景自动化（T3.4）：按 (tenant, device, code) 隔离的最新属性值快照 + 每场景触发冷却
    let mut latest: std::collections::HashMap<(String, String, String), serde_json::Value> =
        std::collections::HashMap::new();
    let mut scene_cooldown = SceneCooldown::new();
    let action_limit = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_SCENES));
    while let Some(Ok(raw)) = stream.next().await {
        let ev: EventMessage = match serde_json::from_slice(&raw) {
            Ok(ev) => ev,
            Err(e) => {
                tracing::warn!(error = %e, "drop unparseable event");
                continue;
            }
        };
        let rules = match store.list_rules(&ev.tenant_id).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "list rules failed");
                continue;
            }
        };
        for msg in evaluate(&ev, &rules) {
            if dedup.should_deliver(&msg, now_ms()) {
                deliver_alert(&store, &bridge, &http, &msg, &rules, &kafka).await;
            }
        }
        if ev.kind == "anomaly" {
            let z_threshold = std::env::var("ANOMALY_Z_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(6.0);
            let msg = AlertMessage {
                rule_id: "anomaly-detector".into(),
                rule_name: "AI 异常检测".into(),
                tenant_id: ev.tenant_id.clone(),
                device_id: ev.device_id.clone(),
                code: ev.code.clone(),
                operator: "z-score".into(),
                threshold: z_threshold,
                value: ev.value.clone(),
                ts: ev.ts,
            };
            if dedup.should_deliver(&msg, now_ms()) {
                deliver_alert(&store, &bridge, &http, &msg, &rules, &kafka).await;
            }
        }
        // 场景自动化（T3.4）：告警送达后评估启用场景。条件评估在消费循环内
        // 完成（纯内存、很快），动作执行 spawn 出去——access 单次超时 5s，
        // 不能拖慢后续事件的告警路径。
        let scenes = match store.list_scenes(&ev.tenant_id, true).await {
            Ok(s) => select_scenes(s),
            Err(e) => {
                tracing::warn!(error = %e, "list scenes failed");
                Vec::new()
            }
        };
        // 快照按 (tenant, device, code) 隔离：场景只能看到本租户设备的历史值
        latest.insert(
            (ev.tenant_id.clone(), ev.device_id.clone(), ev.code.clone()),
            ev.value.clone(),
        );
        // shortcut: MVP 无淘汰策略，超过 10 万条整体清空；量大时改为 LRU/定时清理
        if latest.len() > 100_000 {
            latest.clear();
        }
        // 冷却用墙钟而非 ev.ts（设备时间戳可伪造到未来，会长期压制场景）
        let now = now_ms();
        for scene in scenes {
            let permit = match claim_scene(
                &scene,
                &ev,
                &mut latest,
                &mut scene_cooldown,
                &action_limit,
                now,
            ) {
                SceneClaim::Skip => continue,
                SceneClaim::Busy => {
                    tracing::warn!(scene = %scene.id, "scene action concurrency cap reached, skip");
                    continue;
                }
                SceneClaim::Run(permit) => permit,
            };
            let (store, http, tenant) = (store.clone(), http.clone(), ev.tenant_id.clone());
            tokio::spawn(async move {
                let _permit = permit;
                // 自动执行同样校验设备归属（写入时已校验，这里覆盖存量行/归属变更）
                let ids = extract_device_ids(&scene.conditions, &scene.actions);
                match store.ensure_devices_owned(&tenant, &ids).await {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::warn!(
                            scene = %scene.id,
                            tenant = %tenant,
                            "scene device not in tenant, skip"
                        );
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, scene = %scene.id, "scene ownership check failed, skip");
                        return;
                    }
                }
                let results = execute_scene(&scene, &http, &store.db, &tenant).await;
                for r in &results {
                    if !r.ok {
                        tracing::warn!(
                            scene = %scene.id,
                            action = %r.kind,
                            detail = ?r.detail,
                            "scene action failed"
                        );
                    }
                }
                if let Err(e) = store.touch_scene_triggered(&tenant, &scene.id).await {
                    tracing::warn!(error = %e, "touch scene failed");
                }
                tracing::info!(scene = %scene.id, actions = results.len(), "scene executed");
            });
        }
    }
}

/// 告警送达流水线：WS 推送 + 落告警记录 + 规则 webhook + 联动动作（D-3）
/// + 多渠道通知。（规则 webhook 仅规则触发的告警有；anomaly-detector 无。）
async fn deliver_alert(
    store: &Arc<RuleStore>,
    bridge: &AlertBridge,
    http: &reqwest::Client,
    msg: &AlertMessage,
    rules: &[crate::models::Rule],
    kafka: &KafkaMq,
) {
    bridge.publish(&msg.tenant_id, msg).await;
    if let Err(e) = store.insert_alert(msg).await {
        tracing::warn!(error = %e, "alert record insert failed");
    }
    // T2.8: 告警同步写入 OpenSearch 索引（doc id 与 alert_records 同源；
    // 失败仅 warn，不阻断送达流水线）
    if let Some(search) = ecat_search::connect_search() {
        let id = crate::engine::to_alert_record(msg).id;
        let doc = serde_json::json!({
            "id": id,
            "rule_id": msg.rule_id,
            "tenant_id": msg.tenant_id,
            "device_id": msg.device_id,
            "code": msg.code,
            "operator": msg.operator,
            "threshold": msg.threshold,
            "value": msg.value,
            "ts": msg.ts,
            "status": "active",
        });
        if let Err(e) = search.index("alerts", &id, &doc).await {
            tracing::warn!(error = %e, "alert index failed");
        }
    }
    let rule = rules.iter().find(|r| r.id == msg.rule_id);
    // D-3 联动：命中规则配置了动作 → 发布指令事件到 iot.commands，access 消费后下发
    if let Some(dev) = rule.and_then(|r| r.action_device_id.clone()) {
        if let (Some(code), Some(value)) = (
            rule.and_then(|r| r.action_code.clone()),
            rule.and_then(|r| r.action_value.clone()),
        ) {
            let cmd = CommandEvent {
                device_id: dev,
                tenant_id: msg.tenant_id.clone(),
                code,
                value,
                ts: msg.ts,
            };
            let payload = serde_json::to_vec(&cmd).unwrap_or_default();
            // 分区键用 tenant_id：同租户联动指令固定同一分区，access 多副本顺序稳定
            if let Err(e) = kafka
                .publish_keyed(ecat_iot::TOPIC_COMMANDS, &msg.tenant_id, &payload)
                .await
            {
                tracing::warn!(error = %e, "linkage action publish failed");
            }
        }
    }
    let webhook = rule.and_then(|r| r.webhook_url.clone());
    if let Some(url) = webhook {
        notify_webhook(http, &url, msg).await;
    }
    // 多渠道通知：加载租户渠道配置，异步发送，失败只记日志
    match store.list_channels(&msg.tenant_id).await {
        Ok(channels) if !channels.is_empty() => {
            let http = http.clone();
            let msg = msg.clone();
            tokio::spawn(async move {
                crate::notify::dispatch(channels, msg, http).await;
            });
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "list notify channels failed"),
    }
}

async fn notify_webhook(http: &reqwest::Client, url: &str, msg: &AlertMessage) {
    // 规则 webhook 目标由租户配置：发送前拒绝内网/回环地址（SSRF）
    if let Err(e) = crate::url_guard::ensure_safe_http_url(url).await {
        tracing::warn!(url = %url, error = %e, "rule webhook target rejected");
        return;
    }
    match http.post(url).body(webhook_payload(msg)).send().await {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => tracing::warn!(status = %r.status().as_u16(), url = %url, "webhook non-2xx"),
        Err(e) => tracing::warn!(error = %e, url = %url, "webhook delivery failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn msg(rule: &str, dev: &str, code: &str, ts: i64) -> AlertMessage {
        AlertMessage {
            rule_id: rule.into(),
            rule_name: "t".into(),
            tenant_id: "t1".into(),
            device_id: dev.into(),
            code: code.into(),
            operator: "gt".into(),
            threshold: 1.0,
            value: json!(2),
            ts,
        }
    }

    #[test]
    fn dedup_suppresses_within_window() {
        let mut d = AlertDedup::new();
        d.window_ms = 5000;
        // 首次投递
        assert!(d.should_deliver(&msg("r1", "d1", "temp", 1000), 1000));
        // 窗口内重复 → 抑制
        assert!(!d.should_deliver(&msg("r1", "d1", "temp", 2000), 2000));
        // 不同 code → 放行
        assert!(d.should_deliver(&msg("r1", "d1", "hum", 2000), 2000));
        // 不同设备 → 放行
        assert!(d.should_deliver(&msg("r1", "d2", "temp", 2000), 2000));
        // 窗口过后 → 放行
        assert!(d.should_deliver(&msg("r1", "d1", "temp", 7000), 7000));
        // 事件 ts 在未来：去重只按墙钟判定，未来 ts 不会导致长期抑制
        let future = i64::MAX / 2;
        assert!(d.should_deliver(&msg("r2", "d1", "temp", future), 1000));
        assert!(!d.should_deliver(&msg("r2", "d1", "temp", future), 2000));
        assert!(d.should_deliver(&msg("r2", "d1", "temp", future), 7000));
    }

    #[test]
    fn dedup_disabled_when_window_zero() {
        let mut d = AlertDedup::new();
        d.window_ms = 0;
        assert!(d.should_deliver(&msg("r1", "d1", "temp", 1000), 1000));
        assert!(d.should_deliver(&msg("r1", "d1", "temp", 1000), 1000));
        // 关闭去重不登记键，避免无界增长
        assert_eq!(d.last.len(), 0);
    }

    #[test]
    fn dedup_evicts_stale_entries() {
        let mut d = AlertDedup::new();
        d.window_ms = 5_000;
        assert!(d.should_deliver(&msg("r1", "d1", "temp", 0), 1_000));
        assert!(d.should_deliver(&msg("r2", "d2", "hum", 0), 1_000));
        assert_eq!(d.last.len(), 2);
        // 窗口过后下一次判定清理过期键，只保留当前活跃条目
        assert!(d.should_deliver(&msg("r3", "d3", "x", 0), 10_000));
        assert_eq!(d.last.len(), 1);
        assert!(d.last.contains_key(&("r3".into(), "d3".into(), "x".into())));
    }

    fn event(device: &str, ts: i64) -> EventMessage {
        EventMessage {
            device_id: device.into(),
            tenant_id: "t1".into(),
            kind: "property".into(),
            code: "temp".into(),
            value: json!(1),
            ts,
        }
    }

    fn scene(id: i64, conditions: serde_json::Value) -> Scene {
        Scene {
            id: id.to_string(),
            tenant_id: "t1".into(),
            name: "s".into(),
            enabled: true,
            conditions,
            actions: json!([]),
            last_triggered_at: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn run_claim(claim: SceneClaim) -> bool {
        matches!(claim, SceneClaim::Run(_))
    }

    #[test]
    fn future_event_ts_does_not_suppress_scene() {
        let s = scene(
            1,
            json!([{"type": "time", "start": "00:00", "end": "00:00"}]),
        );
        let limit = Arc::new(tokio::sync::Semaphore::new(1));
        let mut cooldown = SceneCooldown::with_window_ms(5_000);
        let mut latest = std::collections::HashMap::new();
        let future = i64::MAX / 2;
        let now = 1_000_000;
        assert!(run_claim(claim_scene(
            &s,
            &event("d1", future),
            &mut latest,
            &mut cooldown,
            &limit,
            now
        )));
        assert!(!run_claim(claim_scene(
            &s,
            &event("d1", future),
            &mut latest,
            &mut cooldown,
            &limit,
            now + 1_000
        )));
        // 墙钟走过冷却窗口后再次触发；若用未来 ev.ts 作冷却时间，这里会被压制
        assert!(run_claim(claim_scene(
            &s,
            &event("d1", future),
            &mut latest,
            &mut cooldown,
            &limit,
            now + 5_000
        )));
    }

    #[test]
    fn cooldown_not_consumed_when_permits_saturated() {
        let s = scene(
            1,
            json!([{"type": "time", "start": "00:00", "end": "00:00"}]),
        );
        let limit = Arc::new(tokio::sync::Semaphore::new(1));
        let held = limit.clone().try_acquire_owned().unwrap();
        let mut cooldown = SceneCooldown::with_window_ms(5_000);
        let mut latest = std::collections::HashMap::new();
        let ev = event("d1", 1);
        assert!(matches!(
            claim_scene(&s, &ev, &mut latest, &mut cooldown, &limit, 1_000),
            SceneClaim::Busy
        ));
        // 没拿到许可不 stamp：释放许可后下个事件仍可执行
        drop(held);
        assert!(run_claim(claim_scene(
            &s,
            &ev,
            &mut latest,
            &mut cooldown,
            &limit,
            1_001
        )));
        assert!(matches!(
            claim_scene(&s, &ev, &mut latest, &mut cooldown, &limit, 2_000),
            SceneClaim::Skip
        ));
    }

    #[test]
    fn cooldown_evicts_stale_entries() {
        let mut c = SceneCooldown::with_window_ms(5_000);
        c.mark_triggered("s1", 1_000);
        c.mark_triggered("s2", 1_000);
        assert_eq!(c.len(), 2);
        // 窗口过后下一次触发清理过期键，只保留本次
        c.mark_triggered("s3", 10_000);
        assert_eq!(c.len(), 1);
        assert!(c.allows("s1", 10_000));
    }

    #[test]
    fn disabled_cooldown_does_not_grow() {
        let mut c = SceneCooldown::with_window_ms(0);
        c.mark_triggered("s1", 1_000);
        c.mark_triggered("s2", 1_000);
        assert_eq!(c.len(), 0);
        assert!(c.allows("s1", 1_000));
    }

    #[test]
    fn scene_selection_is_capped_per_event() {
        let many: Vec<_> = (0..=MAX_SCENES_PER_EVENT as i64)
            .map(|i| scene(i, json!([])))
            .collect();
        assert_eq!(many.len(), MAX_SCENES_PER_EVENT + 1);
        assert_eq!(select_scenes(many).len(), MAX_SCENES_PER_EVENT);
        let few: Vec<_> = (0..5).map(|i| scene(i, json!([]))).collect();
        assert_eq!(select_scenes(few).len(), 5);
    }
}
