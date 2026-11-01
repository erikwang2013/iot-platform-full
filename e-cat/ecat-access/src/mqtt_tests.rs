use super::*;

fn auth_req(username: &str, clientid: &str, password: &str) -> MqttAuthReq {
    MqttAuthReq {
        clientid: clientid.into(),
        username: username.into(),
        password: password.into(),
    }
}

fn acl_req(username: &str, topic: &str, action: &str) -> MqttAclReq {
    MqttAclReq {
        username: username.into(),
        topic: topic.into(),
        action: action.into(),
        clientid: username.into(),
    }
}

#[test]
fn report_topic_parses_strictly() {
    assert_eq!(parse_report_topic("iot/devices/42/properties"), Some("42"));
    assert_eq!(parse_report_topic("iot/devices/1/2/properties"), None);
    assert_eq!(parse_report_topic("iot/devices//properties"), None);
    assert_eq!(parse_report_topic("iot/devices/42/heartbeat"), None);
    assert_eq!(parse_report_topic("iot/devices/42/properties/x"), None);
    assert_eq!(parse_report_topic("other/42/properties"), None);
    // 解析出的 id 可直接查 设备→租户 映射
    let mut map = HashMap::new();
    map.insert("42".to_string(), "tenant-1".to_string());
    let topic = "iot/devices/42/properties";
    assert_eq!(
        map.get(parse_report_topic(topic).unwrap())
            .map(String::as_str),
        Some("tenant-1")
    );
}

#[tokio::test]
async fn auth_allows_platform_and_device_credentials() {
    // 平台账号（env 配置）
    let req = auth_req("broker-user", "any-client", "broker-pass");
    assert!(auth_allowed(Some("broker-user"), Some("broker-pass"), None, "p", &req).await);
    assert!(!auth_allowed(Some("broker-user"), Some("other-pass"), None, "p", &req).await);
    // 设备凭据：clientid 必须等于用户名
    let hash = crate::auth::hash_password("device-pass");
    assert!(
        auth_allowed(
            None,
            None,
            Some(&hash),
            "p",
            &auth_req("42", "42", "device-pass")
        )
        .await
    );
    assert!(
        !auth_allowed(
            None,
            None,
            Some(&hash),
            "p",
            &auth_req("42", "other", "device-pass")
        )
        .await
    );
    assert!(!auth_allowed(None, None, Some(&hash), "p", &auth_req("42", "42", "wrong")).await);
    // 未知用户名（无哈希）拒绝
    assert!(!auth_allowed(None, None, None, "p", &auth_req("42", "42", "device-pass")).await);
}

#[test]
fn acl_cache_roundtrip_expiry_and_disable() {
    let cache = AclCache::default();
    assert_eq!(cached_active(&cache, "42"), None);
    cache_active(&cache, "42", true, Duration::from_secs(30));
    assert_eq!(cached_active(&cache, "42"), Some(true));
    cache_active(&cache, "42", false, Duration::from_secs(30));
    assert_eq!(cached_active(&cache, "42"), Some(false));
    // 过期条目视为未命中（禁用状态因此最多残留 TTL）
    cache
        .lock()
        .unwrap()
        .insert("42".into(), (Instant::now(), true));
    assert_eq!(cached_active(&cache, "42"), None);
    // MQTT_ACL_CACHE_SECS=0 关闭缓存
    let off = AclCache::default();
    cache_active(&off, "42", true, Duration::ZERO);
    assert_eq!(cached_active(&off, "42"), None);
}

#[test]
fn acl_cache_is_bounded() {
    let cache = AclCache::default();
    for i in 0..ACL_CACHE_MAX {
        cache_active(&cache, &i.to_string(), true, Duration::from_secs(30));
    }
    cache_active(&cache, "overflow", true, Duration::from_secs(30));
    assert_eq!(cache.lock().unwrap().len(), 1);
}

#[test]
fn acl_limits_devices_to_own_topics() {
    // 平台账号只限 iot/ 命名空间
    assert!(acl_allowed(
        Some("broker-user"),
        &acl_req("broker-user", "iot/devices/1/properties", "publish")
    ));
    assert!(!acl_allowed(
        Some("broker-user"),
        &acl_req("broker-user", "other/topic", "publish")
    ));
    // 设备自身 topic 放行
    assert!(acl_allowed(
        None,
        &acl_req("42", "iot/devices/42/properties", "publish")
    ));
    assert!(acl_allowed(
        None,
        &acl_req("42", "iot/devices/42/heartbeat", "publish")
    ));
    assert!(acl_allowed(
        None,
        &acl_req("42", "iot/devices/42/commands", "subscribe")
    ));
    // 跨设备 / 越权动作拒绝
    assert!(!acl_allowed(
        None,
        &acl_req("42", "iot/devices/43/properties", "publish")
    ));
    assert!(!acl_allowed(
        None,
        &acl_req("42", "iot/devices/42/commands", "publish")
    ));
    assert!(!acl_allowed(
        None,
        &acl_req("42", "iot/devices/42/properties", "subscribe")
    ));
    assert!(!acl_allowed(
        None,
        &acl_req("", "iot/devices//properties", "publish")
    ));
}
