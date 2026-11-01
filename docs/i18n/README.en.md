# IoT Platform

[中文](../../README.md) | [English](README.en.md)

<p align="center">
  <img src="../mascot.svg" width="120" height="120" alt="Project Mascot" />
</p>

A one-stop SaaS IoT platform that unifies access to major device vendors at home and abroad (Tuya, Xiaomi, Huawei, AWS IoT, Azure IoT, etc.), providing device management, thing models, rules & alerts, time-series data, CDN delivery, and multi-platform apps. The backend is a Rust microservice workspace (e-cat), the frontends are Flutter + HarmonyOS. The UI supports 13 languages; the documentation is provided in Chinese / English.

## Features

- **Multi-vendor access**: cloud-to-cloud OAuth (Tuya / Xiaomi / Huawei / AWS / Azure) + vendor AK/SK configuration in the admin console; direct MQTT with TLS / mTLS and username/password; QoS1 publishing with persistent sessions and automatic resubscription after reconnect
- **Thing model**: unified modeling of property / event / service — modeled in admin, rendered dynamically in client
- **Device lifecycle**: register → enable / disable → unbind → delete, with online / offline runtime status; device detail (thing model, alerts, commands, tags), groups and batch creation; direct-device creation API `POST /api/v1/devices`, server-time online freshness (120s) and heartbeat liveness
- **Rules & alerts**: threshold rules, scene automation (property / time conditions + command / notification actions, manual execution), AI anomaly detection with persisted alert history, notification inbox, notification channels, real-time WebSocket push
- **Data & reports**: TDengine time-series storage, history curves, CSV export
- **OTA upgrades**: firmware registration by URL or binary upload, S3 / MinIO storage, CDN signed download URLs, streaming firmware downloads (local/S3), download stats, grayscale batch rollout by device hash, rollback on failure
- **CDN management**: vendor config, enable/disable, refresh & prewarm, signed URLs
- **Multi-tenant SaaS**: tenant isolation, quotas, platform super-admin, roles (admin / operator / read-only)
- **Security**: security-rust inbound scanning, JWT + RBAC, PBKDF2 passwords, mutual TLS, AES-encrypted credentials/notification config/rule webhooks, per-IP/tenant and dedicated webhook rate limiting, audit actor and source IP (90-day retention, bounded queue); WebSocket JWTs travel via `Sec-WebSocket-Protocol` with a connection cap; MQTT device credentials are revoked on delete/disable/unbind, auth rejects disabled/unbound devices, and ACL rechecks existing sessions; MQTT QoS1 publishing with persistent sessions, command queueing when the online state is stale and replay on heartbeat/report, alert/notification/OTA-download retention; startup-required JWT/gateway/encryption/password-pepper secrets fail startup when missing or blank
- **Observability & benchmarking**: Prometheus `/metrics`, alert rules (service down / 5xx / P95 / Kafka lag + kafka-exporter), `ecat-bench` CLI and `scripts/bench.sh`
- **13-language UI**, across Flutter Web / Mobile and native HarmonyOS (4 apps)

## Architecture

![Architecture](../architecture.en.svg)

## Device Access Flows

![Flows](../flow.en.svg)

## Feature Map

![Feature Map](../features.en.svg)

## Device Lifecycle

![Lifecycle](../lifecycle.en.svg)

## Security Architecture

![Security](../security.en.svg)

## Tech Stack

| Layer | Technology |
|----|------|
| Frontend | Flutter (Web / Mobile) · HarmonyOS ArkTS |
| Backend | Rust (axum · tokio · gRPC) · e-cat 8.0.0 · security-rust scanning |
| Middleware | EMQX (MQTT) · Kafka (event bus) · gRPC (internal RPC) |
| Storage | MySQL 8 (metadata) · TDengine (time-series) · Redis (shadow / cache) · S3 / MinIO (objects) |
| Access | Cloud-to-cloud OAuth adapters · direct MQTT (mTLS) |

> **Current version**: v1.23.0.
>
> **API version**: since v1.19.0 all public REST APIs are served under `/api/v1/*`; the legacy `/api/*` and `/admin/*` mounts are removed. The client login is `/api/v1/admin/auth/login`.

## Repository Layout

```
├── apps/            # Frontend apps
│   ├── admin/       # Admin console (Flutter + HarmonyOS)
│   └── client/      # Client app (Flutter + HarmonyOS)
├── e-cat/           # Rust workspace (framework + business microservices)
│   └── ecat*/       # Shared framework crates + services (ecat-gateway · ecat-device · ecat-access · ecat-rule · ecat-data-service · ecat-cdn · ecat-edge …)
├── docs/            # Docs, diagrams, donation images
├── scripts/         # Build / validation / smoke / benchmark scripts (install.sh · bench.sh · loadtest.sh)
└── docker-compose.yml  # Infrastructure (MySQL / Redis / EMQX / Kafka / MinIO / TDengine)
```

## One-Click Install

```bash
git clone https://github.com/erikwang2013/iot-platform.git
cd iot-platform
./scripts/install.sh
```

The script will: start the infrastructure (MySQL / Redis / EMQX / Kafka / MinIO / TDengine) → build the 6 services → generate `.env` (32-byte random secrets for JWT / gateway / encryption / password pepper / EMQX auth, plus the initial admin password) → print per-service start commands that load `.env`. Safe to re-run; an existing `.env` is never overwritten.

## Single-Machine Docker Compose Deployment

A standalone stack for single-machine production / small deployments (it does not replace the development `docker-compose.yml` at the repository root): data, infrastructure logs and keys stay under `deploy/compose/data/`, while application logs are capped at 10 MiB × 3 by the Docker local driver; only nginx 80/443 and EMQX 8883 are published, and application containers run read-only and non-root.

```bash
# only manual step: place the domain certificate in deploy/certs/ (fullchain.pem / privkey.pem)
sudo scripts/init-prod.sh                 # generate .env, internal CA/EMQX certs and data dirs; prints the admin initial password
docker compose -f deploy/compose/docker-compose.yml up -d --build
# optional monitoring (Prometheus / Grafana; Grafana binds 127.0.0.1:3000 only)
docker compose -f deploy/compose/docker-compose.yml --profile monitoring up -d
```

`scripts/init-prod.sh --regenerate` rotates application keys: the old `.env` is backed up to `deploy/compose/backups/`, the previous encryption key is kept as `IOT_CRED_ENCRYPT_KEY_OLD`, and infrastructure passwords are not rotated automatically. See the [single-machine deployment guide](../deploy/single-machine.md) for full steps, ports, backups and unverified items.

## Installation Guide

### Prerequisites

- Docker 24+ and docker compose (or docker-compose)
- Rust 1.80+ (stable, for building the services; the script builds automatically if cargo is installed)
- Port availability: 8080-8085, 3306, 6379, 1883, 9092, 9000-9001, 6041 must be free

### Installation Steps

1. Start the infrastructure: `./scripts/install.sh` runs `docker compose up -d`
2. Build services: the script runs `cargo build --release` when cargo is detected, outputting binaries to `scripts/bin/` (or use the ones under `e-cat/target/release/`)
3. Start services: launch the 6 services one by one with the commands printed at the end of the script (they include `set -a; . <project>/.env; set +a`)
4. Database migration: runs automatically on service start; on first boot iot-access creates the default tenant and admin account. The initial password comes from `ADMIN_INITIAL_PASSWORD` in `.env`

### Environment Variables (selected)

| Variable | Service | Notes |
|------|------|------|
| `OTA_S3_ENDPOINT` / `OTA_S3_REGION` / `OTA_S3_BUCKET` / `OTA_S3_ACCESS_KEY` / `OTA_S3_SECRET_KEY` | iot-device | Store firmware in S3 / MinIO; endpoint, bucket and keys must be set together (partial configuration makes upload return 500 instead of silently falling back to local disk); `OTA_S3_REGION` defaults to `us-east-1` |
| `OTA_PUBLIC_BASE` | iot-device | Public origin prefix used to turn `s3://bucket/key` or relative paths into CDN-signable URLs |
| `OTA_CDN_PROVIDER_ID` | iot-device | iot-cdn vendor used to sign firmware download URLs; when unset, task URLs fall back to the unsigned origin URL |
| `OTA_SIGN_EXPIRES_SECS` | iot-device | Signed URL TTL, 60–86400 seconds, default 3600 |
| `OTA_DOWNLOAD_RETENTION_DAYS` | iot-device | OTA download-record retention days, default 180; 0/invalid falls back |
| `CDN_BASE` | iot-gateway / iot-device | Internal iot-cdn address, default `http://localhost:8085` |
| `MQTT_URL` | iot-access | `tcp://` / `ssl://` / `mqtts://`, default `tcp://localhost:1883` |
| `MQTT_TLS` | iot-access | `1/0`; a TLS URL scheme combined with `0` fails at startup instead of downgrading to plaintext |
| `MQTT_CA_FILE` / `MQTT_CERT_FILE` / `MQTT_KEY_FILE` | iot-access | Custom CA / client certificate and key (mTLS; certificate and key must be set together) |
| `MQTT_USERNAME` / `MQTT_PASSWORD` | iot-access | Must be set together; in hardened mode the platform account is `ecat-access` (full `iot/#` access) |
| `MQTT_CLIENT_ID` | iot-access | Optional, default `ecat-mqtt`; the access publisher uses a persistent session, so every instance must be unique; the edge runtime uses `edge-{EDGE_DEVICE_ID}` |
| `ONLINE_FRESHNESS_SECS` | iot-access | Online freshness in seconds, default 120; reports older than this mark the device offline and queue commands in MySQL for replay |
| `COMMAND_QUEUE_CLEANUP_SECS` | iot-access | Expired offline-command cleanup interval in seconds, default 3600; 0/invalid falls back to the default |
| `ADMIN_INITIAL_PASSWORD` | iot-access | Initial `admin` password; when unset, a random one is printed once at first startup |
| `PLATFORM_TENANT` | iot-access | Tenant that holds the platform super-admin, default `tenant-1`; tenant management is restricted to that tenant admin |
| `JWT_SECRET` | iot-gateway / iot-access / iot-rule | Required, same value in all three and at least 32 bytes; services refuse to start when missing/empty (no development fallback) |
| `IOT_GATEWAY_SECRET` | iot-gateway + five backends | Required and non-empty, same value in all six; the gateway injects `x-gateway-secret`, backends verify it at the tenant gate |
| `IOT_CRED_ENCRYPT_KEY` | iot-access / iot-cdn / iot-rule | Required, same value in all three; encrypts vendor credentials, OAuth state, notification-channel config, and rule webhooks |
| `WS_MAX_CONNECTIONS` / `WS_MAX_CONNECTIONS_PER_TENANT` | iot-rule | WebSocket cap per process (default 1024) and per tenant (default 128); production nginx `/ws` also limits 20 per IP |
| `RULE_HTTP_TIMEOUT_SECS` | iot-rule | Total HTTP timeout for notifications/webhooks, default 10s; slow webhooks no longer block rule consumption |
| `RULE_CACHE_TTL_SECS` / `RULE_CACHE_MAX_TENANTS` | iot-rule | In-process rule/scene/channel cache TTL (default 5, 0 disables) and tenant cap (default 10000) |
| `NOTIFY_CONCURRENCY` | iot-rule | Notification-channel concurrency cap, default 16; saturated sends are skipped and logged |
| `ALERT_RETENTION_DAYS` / `NOTIFICATION_RETENTION_DAYS` / `RETENTION_INTERVAL_SECS` | iot-rule | Alert/notification retention days (default 180/90) and cleanup interval (default 21600s; 0 falls back) |
| `SCENE_UTC_OFFSET_MINUTES` | iot-rule | Timezone offset for scene HH:MM conditions, default 0 (UTC); use 480 for UTC+8 |
| `MQTT_AUTH_CONCURRENCY` / `MQTT_ACL_CACHE_SECS` | iot-access | Device PBKDF2 concurrency (default 8) and ACL device-status cache TTL (default 2s, 0 disables) |
| `AUDIT_QUEUE_CAPACITY` | iot-gateway | Bounded audit-event queue capacity, default 4096; drops are counted |
| `IOT_PASSWORD_PEPPER` | iot-access | Required; used only for legacy HMAC password hash compatibility/migration |
| `TRUST_PROXY_HEADERS` | iot-gateway | Set `1` behind nginx/ingress to trust its `X-Real-IP`; keep off when the gateway is directly exposed |
| `EMQX_AUTH_SECRET` | iot-access / EMQX | Shared `x-emqx-auth` secret for the MQTT HTTP auth/ACL endpoints in hardened mode |

See the [production deployment guide](../deploy/production.md) for the full variable list, key generation and hardening.

For production, enable EMQX HTTP authentication/authorization with `docker compose -f docker-compose.yml -f docker-compose.hardened.yml --profile app up -d` (the default compose remains a development stack). Device MQTT credentials are issued by an admin through `POST /api/v1/devices/{id}/mqtt-credential` (the password is returned once); a device must connect with client id equal to its username (the device id). Deleting, disabling, or unbinding a device revokes its MQTT credentials, and the auth endpoint rejects disabled/unbound devices; already-open connections are rejected on the next ACL check (device-status cache TTL defaults to 2s; set `MQTT_ACL_CACHE_SECS=0` to disable caching). MQTT publishing is QoS1; the access publisher uses a persistent session and subscribers resubscribe automatically after reconnect. Devices send heartbeats every 30s; a device silent for more than 120s is treated as offline, and commands are queued first and replayed when a heartbeat/report returns. Direct devices can be created with `POST /api/v1/devices` (admin/operator), then issued MQTT credentials.

### Site Publishing

`site/` is a static site source (Chinese/English pages, search index, and images).
Use `scripts/deploy-site.sh` to mirror it to the target directory over rsync/SSH:

```bash
SITE_USER=www SITE_PATH=/path/to/iot-platform \
  scripts/deploy-site.sh
# optional: SITE_HOST (default package.erik.xyz), SITE_PORT (default 22000), SITE_SSH_KEY
```

The script uses `--delete` to mirror the target; verify `SITE_PATH` before running.

## Usage

### Login

- Admin console: account `admin` (tenant `tenant-1`); the initial password comes from `ADMIN_INITIAL_PASSWORD` in `.env`, or is printed once by iot-access at first startup. There is no `admin/admin123` default.
- Platform super-admin: the `admin` of the default tenant (`PLATFORM_TENANT`, default `tenant-1`). Tenant list/create/delete is restricted to that admin; a regular admin can only create users in their own tenant.
- Roles: `admin` / `operator` / `read-only` (the legacy `readonly` spelling is migrated on startup).
- Passwords use PBKDF2-SHA256; legacy HMAC hashes are upgraded automatically on the next successful login.

### Service Ports

| Service | Port |
|------|------|
| iot-gateway (gateway / public API) | 8080 |
| iot-device (device service) | 8081 |
| iot-access (ingress / auth) | 8082 |
| iot-data (data service) | 8083 |
| iot-rule (rule engine) | 8084 |
| iot-cdn (CDN management) | 8085 |
| MySQL / Redis / EMQX / Kafka / MinIO / TDengine | 3306 / 6379 / 1883 / 9092 / 9000 / 6041 |

### Module Usage

- **Device management**: add devices in the admin console → choose vendor OAuth or direct MQTT (direct devices can also be created with `POST /api/v1/devices`); device details show online status, thing model, alerts and tags and support sending commands; groups, batch creation and batch group membership are supported
- **Thing model**: define property / event / service for device categories; the client dashboard renders automatically
- **Rules & alerts**: configure threshold rules and scene automation; scenes support property / time conditions and command / notification actions and can run manually; alerts and scene notifications go to the notification inbox, with real-time WebSocket push on trigger
- **History data**: iot-data stores time-series data; view history curves in admin / client and export CSV
- **Reports & statistics**: multi-dimensional reports on devices / data / CDN / alerts / tenants
- **OTA upgrades**: register a firmware URL in the admin console (or upload a binary through iot-device) → create a single-device or grayscale batch task → devices report progress; signed URLs, download stats, streaming downloads (local/S3) and rollback on failure are supported
- **CDN management**: configure multi-vendor CDN with refresh / prewarm and signed URLs
- **Multi-vendor access**: cloud-to-cloud OAuth adapters for Tuya / Xiaomi / Huawei / AWS / Azure, with vendor AK/SK configuration in the admin console

### Admin Endpoints

See [docs/open-api.md](../open-api.md) for the open API. The following OTA / scene / notification / device-credential endpoints are available to admin-console roles only; open API keys cannot reach them. Firmware public download does not require JWT; the HMAC token in the URL is the credential:

| Method / Path | Description |
|------|------|
| `GET/POST /api/v1/ota/firmwares` | List / register firmware (URL) |
| `POST /api/v1/ota/firmwares/upload` | Multipart binary upload (up to 64 MiB; the gateway allows this path) |
| `DELETE /api/v1/ota/firmwares/{id}` | Delete firmware (S3 object is deleted too) |
| `POST /api/v1/ota/firmwares/{id}/signed-url` | Create a CDN signed download URL (`expires_secs` optional, 60–86400) |
| `GET /api/v1/ota/public/firmwares/{id}/download?token=...` | Public firmware download (HMAC token, TTL from `OTA_SIGN_EXPIRES_SECS`) |
| `GET /api/v1/ota/firmwares/{id}/downloads` | Download count and last download time |
| `POST /api/v1/devices/{id}/mqtt-credential` | Issue device MQTT username/password (admin; password returned once) |
| `GET/POST /api/v1/ota/tasks` | List / create upgrade tasks (response includes `download_url`) |
| `POST /api/v1/ota/tasks/batch` | Grayscale batch creation by `rollout_percent` (1–100), at most 1000 devices per request |
| `POST /api/v1/ota/tasks/{id}/report` | Device reports `downloading` / `installing` / `success` / `failed` |
| `POST /api/v1/ota/tasks/{id}/rollback` | Roll back an upgrade task |
| `GET/POST /api/v1/rule/scenes`, `PUT/DELETE /api/v1/rule/scenes/{id}` | Scene CRUD |
| `POST /api/v1/rule/scenes/{id}/execute` | Execute a scene immediately (works even when disabled) |
| `GET /api/v1/rule/notifications?unread=1` | Notification list (newest first, at most 100) |
| `POST /api/v1/rule/notifications/{id}/read` | Mark a notification as read |

### Benchmarking

```bash
./scripts/bench.sh                              # 3 /health targets, 1000 requests / concurrency 10 / warmup 100
N=5000 C=50 TOKEN=<jwt> ./scripts/bench.sh      # custom request count, concurrency and auth
GATEWAY_URL=http://host:8080/health ./scripts/bench.sh
```

Exit codes: `0` no failed requests on any target; `1` any target had failed requests; `2` usage error. The script is a smoke / load tool, not a CI performance gate; you can also run `cargo run -p ecat-bench --bin bench -- --url ... --json` directly.

## Implementation Phases

| Phase | Scope |
|------|------|
| P0 Skeleton | Repo structure, iot-gateway + iot-device + Docker orchestration (MySQL/Redis/EMQX/Kafka/MinIO) |
| P1 Access | iot-access + Tuya adapter + direct MQTT |
| P2 Data | iot-data + TDengine + history curves |
| P3 Rules | iot-rule alerts + scene automation |
| P4 Multi-vendor + CDN | Xiaomi/Huawei/AWS/Azure adapters + iot-cdn |
| P5 Frontend | apps/admin + apps/client full-stack |
| P6 Launch | Security hardening, load testing, OTA |

## Open Source and Commercial Editions

The same codebase is delivered as open-source and commercial editions: the open-source edition targets personal study and research, while the commercial edition targets production delivery with commercial licensing, deployment and support. See the [edition comparison](https://package.erik.xyz/iot-platform/en.html#editions); for commercial licensing, contact **erik@erik.xyz**.

## Support & Donations

Your support keeps this project going. Donations are greatly appreciated!

### Scan to Donate (WeChat Pay / Alipay)

<p>
  <img src="../weixinpay.png" width="130" height="130" alt="WeChat Pay QR code" title="WeChat Pay" />
  <img src="../alipay.png" width="130" height="130" alt="Alipay QR code" title="Alipay" />
</p>

WeChat Pay · Alipay

## Project License

Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz

- This project is provided for personal study and research only.
- Commercial use of any kind is prohibited without prior written authorization from the author (including but not limited to commercial deployment, paid services, resale, SaaS operation, and providing commercial services based on this project).
- For commercial licensing, contact **erik@erik.xyz**.
- Unauthorized commercial use will be pursued for legal liability worldwide, and the author reserves the right to pursue such liability for life.
- Third-party components remain under their original licenses; visual assets are governed by ASSETS-LICENSE.md where applicable.



## Project Pet Expressions · IoT Bot

<table align="center">
<tr><td align="center"><img src="../pet-expressions/smile.svg" width="88" alt="Smile"><br><b>Smile</b><br><code>:emoji-smile:</code></td><td align="center"><img src="../pet-expressions/joy.svg" width="88" alt="Joy"><br><b>Joy</b><br><code>:emoji-joy:</code></td><td align="center"><img src="../pet-expressions/cry.svg" width="88" alt="Cry"><br><b>Cry</b><br><code>:emoji-cry:</code></td><td align="center"><img src="../pet-expressions/depressed.svg" width="88" alt="Depressed"><br><b>Depressed</b><br><code>:emoji-depressed:</code></td><td align="center"><img src="../pet-expressions/terrified.svg" width="88" alt="Terrified"><br><b>Terrified</b><br><code>:emoji-terrified:</code></td><td align="center"><img src="../pet-expressions/angry.svg" width="88" alt="Angry"><br><b>Angry</b><br><code>:emoji-angry:</code></td></tr>
<tr><td align="center"><img src="../pet-expressions/coquettish.svg" width="88" alt="Coquettish"><br><b>Coquettish</b><br><code>:emoji-coquettish:</code></td><td align="center"><img src="../pet-expressions/cute.svg" width="88" alt="Cute"><br><b>Cute</b><br><code>:emoji-cute:</code></td><td align="center"><img src="../pet-expressions/pleasing.svg" width="88" alt="Pleasing"><br><b>Pleasing</b><br><code>:emoji-pleasing:</code></td><td align="center"><img src="../pet-expressions/knocking.svg" width="88" alt="Knocking"><br><b>Knocking</b><br><code>:emoji-knocking:</code></td><td align="center"><img src="../pet-expressions/facepalm.svg" width="88" alt="Facepalm"><br><b>Facepalm</b><br><code>:emoji-facepalm:</code></td><td align="center"><img src="../pet-expressions/tongue.svg" width="88" alt="Tongue"><br><b>Tongue</b><br><code>:emoji-tongue:</code></td></tr>
</table>

> 12 colorful animated expressions: Smile / Joy / Cry / Depressed / Terrified / Angry / Coquettish / Cute / Pleasing / Knocking / Facepalm / Tongue. Each SVG includes a facial expression, mood effects and overall animation, and respects `prefers-reduced-motion`.
<!-- pet-expressions:end -->

<!-- assets-license:start -->
## Visual Assets License

The project pet, images, icons and expressions in this repository (including the
SVG / PNG files under `docs/pet-expressions/`) **are not open-source assets**; copyright remains with the author. Copying, modification, redistribution or
commercial use requires written permission; for commercial licensing contact **erik@erik.xyz**. See
[ASSETS-LICENSE.md](../../ASSETS-LICENSE.md).

> The code remains under the repository `LICENSE`; third-party assets (such as the clearly marked Go Gopher) remain under their original licenses.
<!-- assets-license:end -->
