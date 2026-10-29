use ecat_rule::models::{EventMessage, NewScene};
use ecat_rule::scenes::{
    conditions_match_at, cooldown_allows, extract_device_ids, time_window_matches, validate_scene,
};
use serde_json::{Value, json};
use std::collections::HashMap;

fn valid_scene() -> NewScene {
    NewScene {
        name: "回家模式".into(),
        enabled: true,
        conditions: json!([
            {"type": "property", "device_id": "123456789", "code": "temp", "operator": "gt", "value": 30},
            {"type": "time", "start": "18:00", "end": "23:00"}
        ]),
        actions: json!([
            {"type": "command", "device_id": "987654321", "code": "light", "value": true},
            {"type": "notify", "title": "场景触发", "message": "温度超过 30"}
        ]),
    }
}

fn event_for(tenant: &str, device: &str, code: &str, value: Value) -> EventMessage {
    EventMessage {
        device_id: device.into(),
        tenant_id: tenant.into(),
        kind: "property".into(),
        code: code.into(),
        value,
        ts: 1_690_000_000_000,
    }
}

fn event(device: &str, code: &str, value: Value) -> EventMessage {
    event_for("t1", device, code, value)
}

type Latest = HashMap<(String, String, String), Value>;

fn latest_with(tenant: &str, pairs: &[(&str, &str, Value)]) -> Latest {
    pairs
        .iter()
        .map(|(d, c, v)| {
            (
                (tenant.to_string(), d.to_string(), c.to_string()),
                v.clone(),
            )
        })
        .collect()
}

#[test]
fn valid_scene_passes() {
    assert!(validate_scene(&valid_scene()).is_ok());
}

#[test]
fn command_value_defaults_to_optional() {
    let mut s = valid_scene();
    s.actions = json!([{"type": "command", "device_id": "1", "code": "switch"}]);
    assert!(validate_scene(&s).is_ok());
}

#[test]
fn rejects_bad_name() {
    for name in ["", "   "] {
        let mut s = valid_scene();
        s.name = name.into();
        assert!(validate_scene(&s).is_err(), "name {name:?} 应被拒绝");
    }
    let mut s = valid_scene();
    s.name = "x".repeat(129);
    assert!(validate_scene(&s).is_err());
}

#[test]
fn rejects_bad_operator_and_value_type() {
    let mut s = valid_scene();
    s.conditions = json!([
        {"type": "property", "device_id": "1", "code": "temp", "operator": "between", "value": 1}
    ]);
    assert!(validate_scene(&s).is_err());

    let mut s = valid_scene();
    s.conditions = json!([
        {"type": "property", "device_id": "1", "code": "temp", "operator": "gt", "value": null}
    ]);
    assert!(validate_scene(&s).is_err());

    let mut s = valid_scene();
    s.conditions = json!([
        {"type": "property", "device_id": "not-a-number", "code": "temp", "operator": "gt", "value": 1}
    ]);
    assert!(validate_scene(&s).is_err());
}

#[test]
fn rejects_bad_time() {
    for (start, end) in [
        ("24:00", "23:00"),
        ("9:00", "10:00"),
        ("12:60", "13:00"),
        ("12:00", "25:00"),
    ] {
        let mut s = valid_scene();
        s.conditions = json!([{"type": "time", "start": start, "end": end}]);
        assert!(validate_scene(&s).is_err(), "{start}-{end} 应被拒绝");
    }
}

#[test]
fn rejects_unknown_type_and_extra_shape() {
    let mut s = valid_scene();
    s.conditions = json!([{"type": "weather", "value": 1}]);
    assert!(validate_scene(&s).is_err());

    let mut s = valid_scene();
    s.conditions = json!([
        {"type": "property", "device_id": "1", "code": "temp", "operator": "gt", "value": 1, "extra": 1}
    ]);
    assert!(validate_scene(&s).is_err());

    let mut s = valid_scene();
    s.actions = json!([{"type": "email", "title": "t"}]);
    assert!(validate_scene(&s).is_err());

    let mut s = valid_scene();
    s.actions = json!([{"type": "notify", "title": "", "message": "m"}]);
    assert!(validate_scene(&s).is_err());
}

#[test]
fn rejects_limit_violations() {
    let cond = json!({"type": "time", "start": "00:00", "end": "00:00"});
    let mut s = valid_scene();
    s.conditions = Value::Array(vec![cond; 21]);
    assert!(validate_scene(&s).is_err());
    s.conditions = json!([]);
    assert!(validate_scene(&s).is_err());

    let act = json!({"type": "notify", "title": "t", "message": "m"});
    let mut s = valid_scene();
    s.actions = Value::Array(vec![act; 21]);
    assert!(validate_scene(&s).is_err());
    s.actions = json!([]);
    assert!(validate_scene(&s).is_err());
}

#[test]
fn extract_device_ids_collects_and_dedupes() {
    let conditions = json!([
        {"type": "property", "device_id": "11", "code": "temp", "operator": "gt", "value": 30},
        {"type": "property", "device_id": "22", "code": "hum", "operator": "lt", "value": 40},
        {"type": "time", "start": "00:00", "end": "00:00"}
    ]);
    let actions = json!([
        {"type": "command", "device_id": "22", "code": "light"},
        {"type": "notify", "title": "t", "message": "m"}
    ]);
    assert_eq!(extract_device_ids(&conditions, &actions), vec![11, 22]);
}

#[test]
fn extract_device_ids_empty_inputs() {
    assert!(extract_device_ids(&json!([]), &json!([])).is_empty());
    // 非数组/非数字 id 一律跳过（边界由 validate_scene 拦截）
    assert!(
        extract_device_ids(&json!({}), &json!([{"type": "command", "device_id": "x"}])).is_empty()
    );
}

#[test]
fn numeric_property_conditions_compare() {
    let conds = json!([
        {"type": "property", "device_id": "d1", "code": "temp", "operator": "gt", "value": 30},
        {"type": "property", "device_id": "d1", "code": "temp", "operator": "eq", "value": 35}
    ]);
    let mut latest = HashMap::new();
    assert!(conditions_match_at(
        &conds,
        &event("d1", "temp", json!(35)),
        &mut latest,
        "t1",
        0
    ));
    let mut latest = HashMap::new();
    assert!(!conditions_match_at(
        &conds,
        &event("d1", "temp", json!(30)),
        &mut latest,
        "t1",
        0
    ));
}

#[test]
fn string_and_bool_conditions_use_eq_neq() {
    let conds = json!([
        {"type": "property", "device_id": "d1", "code": "mode", "operator": "eq", "value": "on"}
    ]);
    let mut latest = HashMap::new();
    assert!(conditions_match_at(
        &conds,
        &event("d1", "mode", json!("on")),
        &mut latest,
        "t1",
        0
    ));
    let mut latest = HashMap::new();
    assert!(!conditions_match_at(
        &conds,
        &event("d1", "mode", json!("off")),
        &mut latest,
        "t1",
        0
    ));
    // 字符串不支持大小比较
    let gt = json!([
        {"type": "property", "device_id": "d1", "code": "mode", "operator": "gt", "value": "a"}
    ]);
    let mut latest = HashMap::new();
    assert!(!conditions_match_at(
        &gt,
        &event("d1", "mode", json!("b")),
        &mut latest,
        "t1",
        0
    ));

    let flag = json!([
        {"type": "property", "device_id": "d1", "code": "armed", "operator": "eq", "value": true}
    ]);
    let mut latest = HashMap::new();
    assert!(conditions_match_at(
        &flag,
        &event("d1", "armed", json!(true)),
        &mut latest,
        "t1",
        0
    ));
    let mut latest = HashMap::new();
    assert!(!conditions_match_at(
        &flag,
        &event("d1", "armed", json!(false)),
        &mut latest,
        "t1",
        0
    ));
}

#[test]
fn multi_property_uses_latest_snapshot_map() {
    let conds = json!([
        {"type": "property", "device_id": "d1", "code": "temp", "operator": "gt", "value": 30},
        {"type": "property", "device_id": "d2", "code": "hum", "operator": "lt", "value": 40}
    ]);
    // d2 的历史值来自 latest 快照，当前事件只带 d1
    let mut latest = latest_with("t1", &[("d2", "hum", json!(35))]);
    assert!(conditions_match_at(
        &conds,
        &event("d1", "temp", json!(31)),
        &mut latest,
        "t1",
        0
    ));

    let mut latest = latest_with("t1", &[("d2", "hum", json!(45))]);
    assert!(!conditions_match_at(
        &conds,
        &event("d1", "temp", json!(31)),
        &mut latest,
        "t1",
        0
    ));

    // 缺少该设备/属性 → 不匹配
    let mut latest = HashMap::new();
    let only_d2 = json!([
        {"type": "property", "device_id": "d2", "code": "hum", "operator": "lt", "value": 40}
    ]);
    assert!(!conditions_match_at(
        &only_d2,
        &event("d1", "temp", json!(31)),
        &mut latest,
        "t1",
        0
    ));
}

#[test]
fn tenants_with_same_device_id_code_do_not_see_each_other() {
    let conds = json!([
        {"type": "property", "device_id": "d1", "code": "temp", "operator": "gt", "value": 30}
    ]);
    let mut latest = HashMap::new();
    // tenant-a 上报 d1.temp=35
    assert!(conditions_match_at(
        &conds,
        &event_for("tenant-a", "d1", "temp", json!(35)),
        &mut latest,
        "tenant-a",
        0
    ));
    // tenant-b 的同名设备/属性没有历史值：不能读到 tenant-a 的 35
    assert!(!conditions_match_at(
        &conds,
        &event_for("tenant-b", "d1", "mode", json!("on")),
        &mut latest,
        "tenant-b",
        0
    ));
    // tenant-b 上报自己的 d1.temp=20：只能看到自己的值
    assert!(!conditions_match_at(
        &conds,
        &event_for("tenant-b", "d1", "temp", json!(20)),
        &mut latest,
        "tenant-b",
        0
    ));
    // 各自的快照互不覆盖
    assert_eq!(
        latest.get(&("tenant-a".into(), "d1".into(), "temp".into())),
        Some(&json!(35))
    );
    assert_eq!(
        latest.get(&("tenant-b".into(), "d1".into(), "temp".into())),
        Some(&json!(20))
    );
}

#[test]
fn time_window_normal_wrap_and_all_day() {
    let (nine, eighteen, twenty_two, six) = (9 * 60, 18 * 60, 22 * 60, 6 * 60);
    // 常规窗口 [09:00,18:00)
    assert!(time_window_matches(nine, eighteen, 12 * 60));
    assert!(time_window_matches(nine, eighteen, nine));
    assert!(!time_window_matches(nine, eighteen, 8 * 60 + 59));
    assert!(!time_window_matches(nine, eighteen, eighteen));
    // 跨午夜 [22:00,06:00)
    assert!(time_window_matches(twenty_two, six, 23 * 60));
    assert!(time_window_matches(twenty_two, six, 5 * 60 + 59));
    assert!(!time_window_matches(twenty_two, six, 12 * 60));
    // start == end → 全天
    assert!(time_window_matches(twenty_two, twenty_two, 0));
    assert!(time_window_matches(twenty_two, twenty_two, 23 * 60 + 59));
}

#[test]
fn time_condition_routes_through_conditions_match() {
    let all_day = json!([{"type": "time", "start": "00:00", "end": "00:00"}]);
    let mut latest = HashMap::new();
    assert!(conditions_match_at(
        &all_day,
        &event("d1", "temp", json!(1)),
        &mut latest,
        "t1",
        777
    ));

    let night = json!([{"type": "time", "start": "22:00", "end": "06:00"}]);
    let mut latest = HashMap::new();
    assert!(conditions_match_at(
        &night,
        &event("d1", "temp", json!(1)),
        &mut latest,
        "t1",
        23 * 60
    ));
    let mut latest = HashMap::new();
    assert!(!conditions_match_at(
        &night,
        &event("d1", "temp", json!(1)),
        &mut latest,
        "t1",
        12 * 60
    ));
}

#[test]
fn cooldown_suppresses_within_window() {
    assert!(cooldown_allows(None, 1_000, 5_000));
    assert!(!cooldown_allows(Some(1_000), 2_000, 5_000));
    assert!(cooldown_allows(Some(1_000), 6_000, 5_000));
    // window <= 0 → 关闭冷却
    assert!(cooldown_allows(Some(1_000), 1_000, 0));
}
