//! 场景自动化（T3.4）：条件校验、事件条件匹配、动作执行（指令/通知）。
//! 存储访问见 scene_store.rs。
//!
//! 时间条件按 UTC 分钟数计算（项目无 chrono 依赖）；跨时区语义由上层折算，
//! 当前 MVP 按 UTC 解释 HH:MM。

use crate::engine::compare;
use crate::models::{EventMessage, NewScene, Scene};
use crate::store::OPERATORS;
use ecat_data_sqlx::SqlxClient;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

// ---------- 校验 ----------

fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max
}

fn i64_str(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_some_and(|s| s.parse::<i64>().is_ok())
}

fn code_ok(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| text_ok(s, 64))
}

/// 对象只允许 allowed 中的键，且 required 必须齐全（拒绝多字段/缺字段）。
fn only_keys(obj: &Map<String, Value>, allowed: &[&str], required: &[&str]) -> bool {
    obj.keys().all(|k| allowed.contains(&k.as_str()))
        && required.iter().all(|k| obj.contains_key(*k))
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    if b.len() != 5
        || b[2] != b':'
        || !b[..2].iter().all(u8::is_ascii_digit)
        || !b[3..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let h = u32::from(b[0] - b'0') * 10 + u32::from(b[1] - b'0');
    let m = u32::from(b[3] - b'0') * 10 + u32::from(b[4] - b'0');
    (h < 24 && m < 60).then_some(h * 60 + m)
}

fn validate_condition(c: &Value, i: usize) -> Result<(), String> {
    let obj = c
        .as_object()
        .ok_or_else(|| format!("conditions[{i}] must be an object"))?;
    let typ = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("conditions[{i}].type required"))?;
    match typ {
        "property" => {
            let keys = ["type", "device_id", "code", "operator", "value"];
            if !only_keys(obj, &keys, &keys) {
                return Err(format!(
                    "conditions[{i}] property requires exactly {keys:?}"
                ));
            }
            if !i64_str(obj.get("device_id")) {
                return Err(format!(
                    "conditions[{i}].device_id must be a platform device id"
                ));
            }
            if !code_ok(obj.get("code")) {
                return Err(format!("conditions[{i}].code must be 1..64 chars"));
            }
            let op = obj
                .get("operator")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !OPERATORS.contains(&op) {
                return Err(format!(
                    "conditions[{i}].operator must be one of {OPERATORS:?}"
                ));
            }
            if !matches!(
                obj.get("value"),
                Some(Value::Number(_) | Value::String(_) | Value::Bool(_))
            ) {
                return Err(format!(
                    "conditions[{i}].value must be number, string or bool"
                ));
            }
        }
        "time" => {
            let keys = ["type", "start", "end"];
            if !only_keys(obj, &keys, &keys) {
                return Err(format!("conditions[{i}] time requires exactly {keys:?}"));
            }
            for key in ["start", "end"] {
                let v = obj.get(key).and_then(Value::as_str).unwrap_or_default();
                if parse_hhmm(v).is_none() {
                    return Err(format!(
                        "conditions[{i}].{key} must be HH:MM (00:00..23:59)"
                    ));
                }
            }
        }
        _ => return Err(format!("conditions[{i}].type must be property|time")),
    }
    Ok(())
}

fn validate_action(a: &Value, i: usize) -> Result<(), String> {
    let obj = a
        .as_object()
        .ok_or_else(|| format!("actions[{i}] must be an object"))?;
    let typ = obj
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("actions[{i}].type required"))?;
    match typ {
        "command" => {
            let allowed = ["type", "device_id", "code", "value"];
            if !only_keys(obj, &allowed, &allowed[..3]) {
                return Err(format!(
                    "actions[{i}] command requires type, device_id, code and optional value"
                ));
            }
            if !i64_str(obj.get("device_id")) {
                return Err(format!(
                    "actions[{i}].device_id must be a platform device id"
                ));
            }
            if !code_ok(obj.get("code")) {
                return Err(format!("actions[{i}].code must be 1..64 chars"));
            }
        }
        "notify" => {
            let keys = ["type", "title", "message"];
            if !only_keys(obj, &keys, &keys) {
                return Err(format!("actions[{i}] notify requires exactly {keys:?}"));
            }
            for (key, max) in [("title", 128), ("message", 1024)] {
                let v = obj.get(key).and_then(Value::as_str).unwrap_or_default();
                if !text_ok(v, max) {
                    return Err(format!("actions[{i}].{key} must be 1..{max} chars"));
                }
            }
        }
        _ => return Err(format!("actions[{i}].type must be command|notify")),
    }
    Ok(())
}

/// 场景请求体校验：名称/条件/动作的数量、形状、取值白名单。
pub fn validate_scene(s: &NewScene) -> Result<(), String> {
    if !text_ok(&s.name, 128) {
        return Err("name must be 1..128 chars".into());
    }
    let conditions = s
        .conditions
        .as_array()
        .filter(|a| (1..=20).contains(&a.len()))
        .ok_or("conditions must be an array of 1..=20 items")?;
    for (i, c) in conditions.iter().enumerate() {
        validate_condition(c, i)?;
    }
    let actions = s
        .actions
        .as_array()
        .filter(|a| (1..=20).contains(&a.len()))
        .ok_or("actions must be an array of 1..=20 items")?;
    for (i, a) in actions.iter().enumerate() {
        validate_action(a, i)?;
    }
    Ok(())
}

/// 收集场景引用的设备 id（property 条件 + command 动作），去重保序。
pub fn extract_device_ids(conditions: &Value, actions: &Value) -> Vec<i64> {
    let mut ids = Vec::new();
    for (list, kind) in [(conditions, "property"), (actions, "command")] {
        let Some(items) = list.as_array() else {
            continue;
        };
        for item in items {
            if item.get("type").and_then(Value::as_str) != Some(kind) {
                continue;
            }
            if let Some(id) = item
                .get("device_id")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<i64>().ok())
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    ids
}

// ---------- 条件匹配 ----------

/// 当天 UTC 分钟数（0..1440）。
pub fn utc_minute_of_day() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    ((secs % 86_400) / 60) as u32
}

/// 时间窗口 [start,end)；start==end 为全天，start>end 跨午夜。
pub fn time_window_matches(start: u32, end: u32, now: u32) -> bool {
    if start == end {
        true
    } else if start < end {
        start <= now && now < end
    } else {
        now >= start || now < end
    }
}

fn eq_neq(equal: bool, op: &str) -> bool {
    match op {
        "eq" => equal,
        "neq" => !equal,
        _ => false,
    }
}

fn property_matches(
    c: &Value,
    latest: &HashMap<(String, String, String), Value>,
    tenant: &str,
) -> bool {
    let (Some(device), Some(code), Some(op)) = (
        c.get("device_id").and_then(Value::as_str),
        c.get("code").and_then(Value::as_str),
        c.get("operator").and_then(Value::as_str),
    ) else {
        return false;
    };
    let Some(expected) = c.get("value") else {
        return false;
    };
    let Some(actual) = latest.get(&(tenant.to_string(), device.to_string(), code.to_string()))
    else {
        return false;
    };
    match (actual, expected) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => compare(a, op, b),
            _ => false,
        },
        (Value::String(a), Value::String(b)) => eq_neq(a == b, op),
        (Value::Bool(a), Value::Bool(b)) => eq_neq(a == b, op),
        _ => false,
    }
}

/// 事件条件匹配（全部 AND）。事件为 property 时先更新本租户的 latest 快照；
/// 快照按 (tenant, device, code) 隔离，property 条件只能看到本租户的历史值。
pub fn conditions_match(
    conditions: &Value,
    event: &EventMessage,
    latest: &mut HashMap<(String, String, String), Value>,
    tenant: &str,
) -> bool {
    conditions_match_at(conditions, event, latest, tenant, utc_minute_of_day())
}

/// now_minute 由调用方注入，便于确定性单测。
pub fn conditions_match_at(
    conditions: &Value,
    event: &EventMessage,
    latest: &mut HashMap<(String, String, String), Value>,
    tenant: &str,
    now_minute: u32,
) -> bool {
    if event.kind == "property" {
        latest.insert(
            (
                tenant.to_string(),
                event.device_id.clone(),
                event.code.clone(),
            ),
            event.value.clone(),
        );
    }
    let Some(list) = conditions.as_array() else {
        return false;
    };
    list.iter()
        .all(|c| match c.get("type").and_then(Value::as_str) {
            Some("property") => property_matches(c, latest, tenant),
            Some("time") => {
                let start = c.get("start").and_then(Value::as_str).and_then(parse_hhmm);
                let end = c.get("end").and_then(Value::as_str).and_then(parse_hhmm);
                matches!((start, end), (Some(s), Some(e)) if time_window_matches(s, e, now_minute))
            }
            _ => false,
        })
}

// ---------- 冷却 ----------

/// 冷却判定（纯函数）：window<=0 关闭冷却。
pub fn cooldown_allows(last_ms: Option<i64>, now_ms: i64, window_ms: i64) -> bool {
    window_ms <= 0 || last_ms.is_none_or(|last| now_ms - last >= window_ms)
}

/// 冷却表条目阈值：达到即触发一次过期清理（长时间运行也不会无界增长）。
const COOLDOWN_EVICT_THRESHOLD: usize = 4096;

/// 每场景触发冷却（进程内，多实例各自计算；MVP 级语义）。
pub struct SceneCooldown {
    last: HashMap<String, i64>,
    window_ms: i64,
    /// 上次清理时间：每过一个窗口清理过期键。
    last_evict_ms: i64,
}

impl SceneCooldown {
    pub fn new() -> Self {
        let secs = std::env::var("SCENE_COOLDOWN_SECS")
            .ok()
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(60);
        Self::with_window_ms(secs * 1000)
    }

    /// 显式指定冷却窗口（单测用，避免依赖环境变量）。
    pub fn with_window_ms(window_ms: i64) -> Self {
        Self {
            last: HashMap::new(),
            window_ms,
            last_evict_ms: 0,
        }
    }

    /// 冷却窗口是否已过（只读，不登记时间）。
    pub fn allows(&self, scene_id: &str, now_ms: i64) -> bool {
        cooldown_allows(self.last.get(scene_id).copied(), now_ms, self.window_ms)
    }

    /// 条目数（单测断言清理效果）。
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.last.len()
    }

    /// 登记本次触发时间（拿到执行许可后再调用，饱和丢弃不消耗冷却）。
    pub fn mark_triggered(&mut self, scene_id: &str, now_ms: i64) {
        // 关闭冷却时无需登记：allows 恒为 true，登记只会无界增长
        if self.window_ms <= 0 {
            return;
        }
        if now_ms.saturating_sub(self.last_evict_ms) >= self.window_ms
            || self.last.len() >= COOLDOWN_EVICT_THRESHOLD
        {
            let window = self.window_ms;
            self.last
                .retain(|_, ts| now_ms.saturating_sub(*ts) < window);
            self.last_evict_ms = now_ms;
        }
        self.last.insert(scene_id.to_string(), now_ms);
    }
}

impl Default for SceneCooldown {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- 动作执行 ----------

/// 单个动作执行结果（API 原样返回）。
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ActionResult {
    #[serde(rename = "type")]
    pub kind: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ActionResult {
    fn fail(kind: &str, detail: String) -> Self {
        Self {
            kind: kind.into(),
            ok: false,
            detail: Some(detail),
        }
    }
}

/// 立即执行场景动作（手动执行允许 disabled 场景）。
pub async fn execute_scene(
    scene: &Scene,
    http: &reqwest::Client,
    db: &SqlxClient,
    tenant: &str,
) -> Vec<ActionResult> {
    let Some(actions) = scene.actions.as_array() else {
        return Vec::new();
    };
    let base = std::env::var("ACCESS_BASE").unwrap_or_else(|_| "http://localhost:8082".into());
    let secret = std::env::var("IOT_GATEWAY_SECRET").unwrap_or_default();
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        let kind = action
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        results.push(match kind {
            "command" => run_command(action, http, &base, &secret, tenant).await,
            "notify" => run_notify(action, db, tenant).await,
            _ => ActionResult::fail("unknown", "unknown action type".into()),
        });
    }
    results
}

async fn run_command(
    action: &Value,
    http: &reqwest::Client,
    base: &str,
    secret: &str,
    tenant: &str,
) -> ActionResult {
    let (Some(device), Some(code)) = (
        action.get("device_id").and_then(Value::as_str),
        action.get("code").and_then(Value::as_str),
    ) else {
        return ActionResult::fail("command", "invalid command action".into());
    };
    let value = action.get("value").cloned().unwrap_or(Value::Null);
    let url = format!("{base}/api/access/devices/{device}/command");
    let resp = http
        .post(url)
        .timeout(std::time::Duration::from_secs(5))
        .header("x-gateway-secret", secret)
        .header("x-tenant-id", tenant)
        .json(&json!({ "code": code, "value": value }))
        .send()
        .await;
    match resp {
        Ok(r) if r.status().is_success() => {
            let body: Value = r.json().await.unwrap_or(Value::Null);
            let detail = if body.get("queued").and_then(Value::as_bool) == Some(true) {
                "queued".to_string()
            } else {
                body.get("channel")
                    .and_then(Value::as_str)
                    .unwrap_or("ok")
                    .to_string()
            };
            ActionResult {
                kind: "command".into(),
                ok: true,
                detail: Some(detail),
            }
        }
        // 仅回状态码，不回上游响应体/请求头，避免泄漏 secret
        Ok(r) => ActionResult::fail("command", format!("access status {}", r.status().as_u16())),
        Err(e) => ActionResult::fail("command", format!("transport: {e}")),
    }
}

async fn run_notify(action: &Value, db: &SqlxClient, tenant: &str) -> ActionResult {
    let (Some(title), Some(message)) = (
        action.get("title").and_then(Value::as_str),
        action.get("message").and_then(Value::as_str),
    ) else {
        return ActionResult::fail("notify", "invalid notify action".into());
    };
    match crate::scene_store::insert_notification_row(db, tenant, title, message, "scene").await {
        Ok(()) => ActionResult {
            kind: "notify".into(),
            ok: true,
            detail: None,
        },
        Err(e) => ActionResult::fail("notify", e),
    }
}
