use crate::models::EventMessage;
use ecat_data::Cache;
use ecat_data_redis::RedisCache;
use ecat_mq::{MessageQueue, MqError};
use ecat_mq_kafka::KafkaMq;
use serde_json::{Value, json};

pub use ecat_iot::TOPIC_EVENTS;

/// TDengine 列宽上限：code NCHAR(64) / value_str NCHAR(255)。
/// 超限事件会让整批写入失败或截断成相同 tag，必须在入口拒绝。
pub const MAX_CODE_CHARS: usize = 64;
pub const MAX_VALUE_TEXT_CHARS: usize = 255;

/// 影子键前缀：`shadow:{device_id}`。
pub fn shadow_key(device_id: &str) -> String {
    format!("shadow:{device_id}")
}

pub async fn publish_event(mq: &KafkaMq, ev: &EventMessage) -> Result<(), MqError> {
    let payload = serde_json::to_vec(ev).map_err(|e| MqError::Other(format!("encode: {e}")))?;
    // 分区键用 tenant_id：同租户事件固定同一分区，多副本时下游顺序一致
    mq.publish_keyed(TOPIC_EVENTS, &ev.tenant_id, &payload)
        .await
}

/// 影子结构：{"online":bool,"properties":{code:value},"ts":ms,"seen":ms}。
/// property 事件隐含设备在线；online/offline 只改在线标记。
pub async fn shadow_apply(redis: &RedisCache, ev: &EventMessage) -> Result<(), String> {
    let key = shadow_key(&ev.device_id);
    let mut shadow: Value = redis
        .get(&key)
        .await
        .map_err(|e| format!("shadow get: {e}"))?
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_else(|| json!({ "online": false, "properties": {} }));
    match ev.kind.as_str() {
        "property" => {
            shadow["online"] = json!(true);
            shadow["properties"][ev.code.clone()] = ev.value.clone();
        }
        "online" => shadow["online"] = json!(true),
        "offline" => shadow["online"] = json!(false),
        _ => {}
    }
    shadow["ts"] = json!(ev.ts);
    // ts 是设备数据点时间（可能超前/滞后），seen 是平台收到消息的时间：
    // 离线判定只能信 seen，否则设备时钟偏差会让在线状态误判。
    shadow["seen"] = json!(ecat_iot::now_ms());
    let raw = serde_json::to_vec(&shadow).map_err(|e| format!("shadow encode: {e}"))?;
    // ttl = 0 → 无过期时间（Cache::set 内部走 SET 而非 PSETEX）
    redis
        .set(&key, &raw, std::time::Duration::ZERO)
        .await
        .map_err(|e| format!("shadow set: {e}"))
}

/// 影子最近一次被平台看到的时间：优先 seen（服务端接收时间），
/// 旧影子无 seen 时回退设备 ts；都没有返回 0（从未收到上报）。
pub fn shadow_seen_ms(shadow: &Value) -> i64 {
    shadow
        .get("seen")
        .and_then(Value::as_i64)
        .or_else(|| shadow.get("ts").and_then(Value::as_i64))
        .unwrap_or(0)
}

/// 心跳只刷新在线标记与 seen，不产生事件、不写时序库。
/// 返回心跳前的在线状态：false 表示离线→在线边沿，调用方需补发离线指令。
pub async fn shadow_mark_online(redis: &RedisCache, device_id: &str) -> Result<bool, String> {
    let key = shadow_key(device_id);
    let mut shadow: Value = redis
        .get(&key)
        .await
        .map_err(|e| format!("shadow get: {e}"))?
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_else(|| json!({ "online": false, "properties": {} }));
    let now = ecat_iot::now_ms();
    // 先按旧影子判定：无影子时默认影子 online=false，was_online 自然为 false
    let was_online = crate::command_queue::shadow_online(
        &shadow,
        now,
        crate::command_queue::online_freshness_ms(),
    );
    shadow["online"] = json!(true);
    shadow["seen"] = json!(now);
    let raw = serde_json::to_vec(&shadow).map_err(|e| format!("shadow encode: {e}"))?;
    // ttl = 0 → 无过期时间（同 shadow_apply）
    redis
        .set(&key, &raw, std::time::Duration::ZERO)
        .await
        .map_err(|e| format!("shadow set: {e}"))?;
    Ok(was_online)
}

/// 设备事件校验：code 非空且不超列宽；非数值 value 的文本不超列宽。
/// 数值不受文本长度限制（写入 value 列）。
pub fn validate_event(ev: &EventMessage) -> Result<(), String> {
    if ev.code.trim().is_empty() {
        return Err("event rejected: empty code".into());
    }
    if ev.code.chars().count() > MAX_CODE_CHARS {
        return Err(format!(
            "event rejected: code longer than {MAX_CODE_CHARS} chars"
        ));
    }
    if !ev.value.is_number() && ev.value.to_string().chars().count() > MAX_VALUE_TEXT_CHARS {
        return Err(format!(
            "event rejected: value longer than {MAX_VALUE_TEXT_CHARS} chars"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: &str, value: Value) -> EventMessage {
        EventMessage {
            device_id: "1".into(),
            tenant_id: "tenant-1".into(),
            kind: "property".into(),
            code: code.into(),
            value,
            ts: 1,
        }
    }

    #[test]
    fn shadow_seen_prefers_server_time_and_falls_back_to_ts() {
        assert_eq!(shadow_seen_ms(&json!({"seen": 200, "ts": 100})), 200);
        // 旧影子无 seen：回退设备 ts
        assert_eq!(shadow_seen_ms(&json!({"ts": 100})), 100);
        assert_eq!(shadow_seen_ms(&json!({})), 0);
    }

    #[test]
    fn validate_event_accepts_numeric_and_short_text_values() {
        assert!(validate_event(&ev("temp", json!(21.5))).is_ok());
        assert!(validate_event(&ev("switch", json!("on"))).is_ok());
        // 对象值按 JSON 文本计长，短对象放行
        assert!(validate_event(&ev("cfg", json!({"k": "v"}))).is_ok());
    }

    #[test]
    fn validate_event_rejects_empty_and_long_codes() {
        let err = validate_event(&ev("  ", json!(1))).unwrap_err();
        assert!(err.starts_with("event rejected: ") && err.contains("code"));
        let err = validate_event(&ev(&"a".repeat(MAX_CODE_CHARS + 1), json!(1))).unwrap_err();
        assert!(err.contains("code"));
        // 恰好 64 字符放行
        assert!(validate_event(&ev(&"a".repeat(MAX_CODE_CHARS), json!(1))).is_ok());
    }

    #[test]
    fn validate_event_limits_text_values_only() {
        // 300 字符字符串超 value_str 列宽
        let err = validate_event(&ev("msg", json!("x".repeat(300)))).unwrap_err();
        assert!(err.starts_with("event rejected: ") && err.contains("value"));
        // 长对象序列化后同样超限，不能靠对象绕过
        let err = validate_event(&ev("cfg", json!({"k": "x".repeat(300)}))).unwrap_err();
        assert!(err.contains("value"));
        // 数字不受文本长度限制
        assert!(validate_event(&ev("n", json!(1234567890123456789i64))).is_ok());
    }
}
