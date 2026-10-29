CREATE TABLE IF NOT EXISTS ota_downloads (
    id BIGINT PRIMARY KEY,
    tenant_id VARCHAR(64) NOT NULL,
    firmware_id BIGINT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    INDEX idx_ota_downloads_firmware (firmware_id, created_at)
) ENGINE = InnoDB;
