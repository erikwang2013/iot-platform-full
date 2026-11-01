CREATE TABLE IF NOT EXISTS rules (
    id BIGINT PRIMARY KEY,
    tenant_id VARCHAR(36) NOT NULL,
    name VARCHAR(128) NOT NULL,
    device_id BIGINT NOT NULL,
    code VARCHAR(64) NOT NULL,
    operator VARCHAR(8) NOT NULL,
    threshold DOUBLE NOT NULL,
    -- 明文 URL 上限 512；enc:v1 信封（base64 + nonce/tag）约 730 字符
    webhook_url VARCHAR(1024) NULL,
    enabled INT NOT NULL DEFAULT 1,
    action_device_id BIGINT NULL,
    action_code VARCHAR(64) NULL,
    action_value JSON NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    KEY idx_rule_tenant (tenant_id),
    CONSTRAINT fk_rule_tenant FOREIGN KEY (tenant_id) REFERENCES tenants(id)
) ENGINE = InnoDB;

CREATE TABLE IF NOT EXISTS alert_records (
    id BIGINT PRIMARY KEY,
    -- 可空：异常检测告警无 rules 行；删规则时置 NULL 保留历史告警
    rule_id BIGINT NULL,
    tenant_id VARCHAR(36) NOT NULL,
    device_id BIGINT NOT NULL,
    code VARCHAR(64) NOT NULL,
    operator VARCHAR(8) NOT NULL,
    threshold DOUBLE NOT NULL,
    value TEXT NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'active',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    KEY idx_alert_tenant_ts (tenant_id, created_at),
    CONSTRAINT fk_alert_rule FOREIGN KEY (rule_id) REFERENCES rules(id) ON DELETE SET NULL,
    CONSTRAINT fk_alert_tenant FOREIGN KEY (tenant_id) REFERENCES tenants(id)
) ENGINE = InnoDB;
