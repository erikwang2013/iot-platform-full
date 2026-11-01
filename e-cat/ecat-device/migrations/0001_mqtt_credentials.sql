-- 设备直连 MQTT 凭据：EMQX HTTP 认证端点按 username 取 PBKDF2 哈希校验。
-- 与 ecat-access/migrations/0009_mqtt_credentials.sql 表定义一致；
-- 设备服务可独立部署，故自带建表 SQL，不依赖 access 迁移（也不做 users ALTER）。
CREATE TABLE IF NOT EXISTS mqtt_credentials (
    device_id BIGINT PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL,
    username VARCHAR(64) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE KEY uq_mqtt_username (username),
    INDEX idx_mqtt_tenant (tenant_id)
) ENGINE = InnoDB;
