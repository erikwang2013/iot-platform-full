CREATE TABLE IF NOT EXISTS scenes (
    id BIGINT PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL,
    name VARCHAR(128) NOT NULL,
    enabled TINYINT(1) NOT NULL DEFAULT 1,
    conditions_json JSON NOT NULL,
    actions_json JSON NOT NULL,
    last_triggered_at TIMESTAMP NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    INDEX idx_scenes_tenant (tenant_id, enabled)
) ENGINE = InnoDB;

CREATE TABLE IF NOT EXISTS notifications (
    id BIGINT PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL,
    title VARCHAR(128) NOT NULL,
    message VARCHAR(1024) NOT NULL,
    source VARCHAR(32) NOT NULL DEFAULT 'scene',
    read_at TIMESTAMP NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_notifications_tenant (tenant_id, created_at)
) ENGINE = InnoDB;
