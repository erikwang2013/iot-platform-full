-- 设备直连 MQTT 凭据：EMQX HTTP 认证端点按 username 取 PBKDF2 哈希校验。
CREATE TABLE IF NOT EXISTS mqtt_credentials (
    device_id BIGINT PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL,
    username VARCHAR(64) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uq_mqtt_username (username),
    INDEX idx_mqtt_tenant (tenant_id)
) ENGINE = InnoDB;

-- 口令哈希由 HMAC-SHA256（64 字符）升级为 PBKDF2（约 120 字符），原列宽放不下。
-- MODIFY 幂等：列宽已足够时 MySQL 8 不做数据重建。
ALTER TABLE users MODIFY COLUMN password_hash VARCHAR(255) NOT NULL;
