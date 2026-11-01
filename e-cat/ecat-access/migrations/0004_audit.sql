CREATE TABLE IF NOT EXISTS audit_log (
    id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    tenant_id VARCHAR(64) NOT NULL DEFAULT 'anonymous',
    -- 操作者（JWT actor claim：登录用户名 / 开放 API 密钥 id）；旧 token 为空串
    actor VARCHAR(64) NOT NULL DEFAULT '',
    role VARCHAR(32) NOT NULL DEFAULT '',
    method VARCHAR(8) NOT NULL,
    path VARCHAR(255) NOT NULL,
    status INT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id),
    KEY idx_audit_tenant_created (tenant_id, created_at)
) ENGINE = InnoDB;
