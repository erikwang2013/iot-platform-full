# 生产部署指南（docker compose 单机 / 小集群）

> **推荐路径（单机生产）**：直接使用独立的
> [`deploy/compose/docker-compose.yml`](single-machine.md)。`sudo scripts/init-prod.sh`
> 生成 `.env`、内部 CA/EMQX 证书与数据目录，只发布 nginx 80/443 与 EMQX 8883，数据
> bind mount 到 `deploy/compose/data/`；部署步骤见[单机部署指南](single-machine.md) /
> [English](single-machine.en.md)。该编排与根目录开发用 `docker-compose.yml` 相互独立，
> 不要混用。

本文面向其余场景：在仓库根目录的开发用 `docker-compose.yml` 上自行加固、接入外部 /
托管基础设施，或需要逐项调整 TLS、备份与监控。它把该文件生产化到单机或小集群可用
水平。集群/高可用部署见 [`k8s.md`](k8s.md)，多副本语义见
[`multi-instance.md`](multi-instance.md)。

> 前提说明：仓库默认 compose 是**开发形态**——默认口令、无 TLS、Redis 无持久化、
> 单 Kafka broker、OpenSearch 关闭安全插件。MQTT 认证/授权的加固已提供叠加文件
> `docker-compose.hardened.yml`；其余项仍需按本文改造，compose 不会替你改掉基础设施
> 默认口令等开发配置。

## 1. 架构假设与边界

- **部署单元**：compose 内六个业务服务均为单副本进程，无自动故障转移；单机故障 = 全站
  不可用。需要可用性时用 K8s 多副本（`deploy/k8s/`）或托管基础设施。
- **状态位置**：业务状态集中在 MySQL / Redis / Kafka / TDengine / OpenSearch。两个本地
  落盘点必须挂持久卷：
  - `iot-device`：固件目录 `FIRMWARE_DIR`（默认 `./firmwares`，单文件上限 64 MiB）。
  - `iot-access`：凭据密钥环文件 `IOT_CRED_KEYS_FILE`（默认 `./cred_keys.json`）。
  多副本时这两处需要共享存储或粘性路由；当前代码未内置共享存储。
- **基础设施**：compose 中的 MySQL / Redis / Kafka / EMQX / TDengine / OpenSearch 都是单
  节点。生产建议改用托管服务或独立集群；单 Kafka broker 故障会丢事件。
- **网络边界**：对外只开反向代理的 80/443（及按需的 MQTT TLS 端口）。MySQL 3306、
  Redis 6379、Kafka 9092、TDengine 6041、OpenSearch 9200、EMQX 1883、
  Prometheus 9090、Grafana 3000、Loki 3100 只允许内网/本机访问；EMQX 控制台 18083
  在默认 compose 中仅绑定 `127.0.0.1`。
- **服务清单与端口**：iot-gateway 8080、iot-device 8081、iot-access 8082、
  iot-data 8083、iot-rule 8084、iot-cdn 8085。

## 2. 密钥与配置

### 2.1 必填环境变量（从源码核实）

“fail-fast”指变量缺失时进程直接退出，不会带默认值启动。

| 变量 | 使用服务 | fail-fast | 说明 |
|------|---------|-----------|------|
| `JWT_SECRET` | gateway / access / rule | 是 | 三处必须同值且至少 32 字节；缺省/为空时进程直接退出，禁止默认密钥 |
| `IOT_GATEWAY_SECRET` | gateway + 五个后端 | 是 | 六处必须同值且非空；网关经 `x-gateway-secret` 注入，后端租户门校验，缺失/空白时启动失败 |
| `IOT_CRED_ENCRYPT_KEY` | access / cdn / rule | 是 | 三处同值；用于厂商凭据加密、OAuth state 签名、通知渠道配置与规则 webhook 加密 |
| `IOT_CRED_ENCRYPT_KEY_OLD` | access / cdn / rule | 否 | 轮换后的上一把凭据密钥，仅用于解密 version=1 旧行；连续轮换只保留一代，见 2.5 |
| `IOT_PASSWORD_PEPPER` | access | 是 | 旧 HMAC 口令兼容校验；新口令为 PBKDF2-SHA256（自带随机盐，不依赖 pepper）。缺省/为空时 access 直接退出 |
| `ADMIN_INITIAL_PASSWORD` | access | 否 | 首次启动创建 `admin`（租户 `tenant-1`）的口令；未设置时随机生成并在启动日志打印一次，之后无法找回 |
| `PLATFORM_TENANT` | access | 否（`tenant-1`） | 平台超管所在租户：该租户的 `admin` 才能管理租户列表/创建/删除；普通管理员建用户只能建到本租户 |
| `EMQX_AUTH_SECRET` | access + hardened overlay | 加固模式下是 | EMQX HTTP 认证/授权请求头 `x-emqx-auth` 的共享密钥；`scripts/install.sh` 自动生成 |
| `DATABASE_URL` | gateway/device/access/rule/cdn | 否（localhost 默认） | 生产必须指向实际 MySQL |
| `REDIS_URL` | gateway/access/rule | 否 | 限流计数、影子缓存、告警广播桥 |
| `KAFKA_BROKERS` | access/rule/data | 否 | 事件总线 |
| `MQTT_URL` 及 `MQTT_TLS`/`MQTT_CA_FILE`/`MQTT_CERT_FILE`/`MQTT_KEY_FILE`/`MQTT_USERNAME`/`MQTT_PASSWORD`/`MQTT_CLIENT_ID` | access | 部分（成对变量只设一个即退出） | TLS/认证契约见 3.2；加固模式下平台账号为 `ecat-access`（可访问 `iot/#`），多副本时 `MQTT_CLIENT_ID` 必须逐副本唯一 |
| `TDENGINE_URL` / `TDENGINE_USER` / `TDENGINE_PASS` | data | 否 | 时序库 |
| `OPENSEARCH_URL`（或 `SEARCH_KIND=elasticsearch` + `ELASTICSEARCH_URL`） | gateway/device/access/rule | 否 | 未设置 = 检索/索引禁用（读 API 503），不影响启动 |
| `ACCESS_CALLBACK_BASE` | access | 否（localhost 默认） | 必须改为外部 HTTPS 回调：`https://<域名>/api/v1/access/oauth/callback` |
| `FIRMWARE_DIR` | device | 否（`./firmwares`） | 固件落盘目录，需持久卷 |
| `IOT_CRED_KEYS_FILE` | access | 否（`./cred_keys.json`） | 密钥轮换持久化文件，需持久卷 |
| `TUYA_CLIENT_ID` / `TUYA_CLIENT_SECRET` / `MIOT_CLIENT_ID` / `MIOT_CLIENT_SECRET` | access | 否 | 仅接入对应厂商时需要 |
| `SNOWFLAKE_WORKER_ID` | 全部（`ecat::ids`） | 否 | 0-1023；缺省按 HOSTNAME 哈希。多副本建议每副本显式唯一 |

可选调优（均有代码默认值）：`JWT_TTL_SECS`（86400）、`RATE_LIMIT_MAX`/`RATE_LIMIT_WINDOW_SECS`
（100/60s）、`LOGIN_RATE_LIMIT_MAX`/`LOGIN_RATE_LIMIT_WINDOW_SECS`（10/60s）、
`WEBHOOK_RATE_LIMIT_MAX`/`WEBHOOK_RATE_LIMIT_WINDOW_SECS`（6000/60s，Tuya webhook 独立桶）、
`TRUST_PROXY_HEADERS`（nginx/ingress 后设 1，信任其覆盖的 `X-Real-IP`）、
`SERVICE_DISCOVERY_TTL_SECS`（30）、`AUDIT_RETENTION_DAYS`（90）——gateway；
`OFFLINE_TIMEOUT_SECS`/`OFFLINE_PATROL_SECS`（300/60）——access；`REPORT_INTERVAL_SECS`（3600）、
`WS_MAX_CONNECTIONS`（1024）——rule；
`DATA_RETENTION_DAYS`/`DATA_RETENTION_INTERVAL_SECS`（90/21600，<=0 关闭清理）、
`TSDB_KIND`/`CLICKHOUSE_URL`/`CLICKHOUSE_DB`——data；`LOG_FORMAT=json`（Loki 采集建议）、
`RUST_LOG=info`——全部服务。

全局限流按 **JWT 解出的租户** 计数；未登录请求按来源 IP（可信反代后需
`TRUST_PROXY_HEADERS=1`，使用 nginx/ingress 覆盖的 `X-Real-IP`）。登录为 10 次/分钟/IP；
Tuya webhook 使用独立的 `WEBHOOK_RATE_LIMIT_MAX`（默认 6000/60s）；`/health`、`/ready`、
`/metrics` 不经过公共限流。网关上游代理超时固定为连接 5s、读取 60s；固件上传
（multipart ≤ 64 MiB）走 `POST /api/v1/ota/firmwares/upload`，网关流式放行。通知渠道的
webhook / 短信 URL 和 SMTP host 在发送前会解析 DNS 并拒绝私网、回环、链路本地等非公网
地址（SSRF 防护）；需要内网服务时不要绕过，改为公网入口或专用出口代理。

### 2.2 无默认值原则

所有必需安全变量（`JWT_SECRET`、`IOT_GATEWAY_SECRET`、`IOT_CRED_ENCRYPT_KEY`、`IOT_PASSWORD_PEPPER`）缺失或仅空白时启动失败；加固模式下 `EMQX_AUTH_SECRET` 为空会拒绝全部 MQTT 认证回调，必须显式生成。`DATABASE_URL` 等非密钥变量仍可能使用本机默认值，生产必须显式配置。

### 2.3 生成密钥

`scripts/install.sh` 首次运行会生成下列密钥并写入 `.env`；单机生产编排用
`scripts/init-prod.sh` 生成同一组应用密钥，另含基础设施口令与 `IOT_CRED_ENCRYPT_KEY_OLD`，
写入 `deploy/compose/.env`。手工部署时按同样方式生成：

```bash
# 每个密钥独立生成；保存到部署机的 .env（chmod 600），不要提交、不要进镜像
for k in JWT_SECRET IOT_GATEWAY_SECRET IOT_CRED_ENCRYPT_KEY IOT_PASSWORD_PEPPER \
         ADMIN_INITIAL_PASSWORD EMQX_AUTH_SECRET MQTT_PASSWORD; do
  printf '%s=%s\n' "$k" "$(openssl rand -hex 32)"
done
printf 'MQTT_USERNAME=ecat-access\n'
```

`ADMIN_INITIAL_PASSWORD` 只在首次建号时使用；留空则由 iot-access 启动时随机生成并在日志
打印一次。

MySQL / MinIO / Grafana / TDengine 的初始口令也要改掉，建议同样用 `openssl rand -hex 24`
生成（十六进制可避免 URL 编码问题）。注意 `DATABASE_URL` 中的口令需要 URL 编码。

### 2.4 注入方式

**docker compose**：基础文件已声明 `env_file: .env`（`required: false`），生产把
`.env` 放项目根目录并 `chmod 600`。注意基础 compose 在 `environment:` 中硬编码了
`DATABASE_URL`、`REDIS_URL`、`KAFKA_BROKERS` 等为服务名，**它们的优先级高于 `.env`**；
接外部托管数据库时需要修改基础文件或叠加 override。

**K8s**：非敏感项进 `deploy/k8s/configmap.yaml`，密钥进 `deploy/k8s/secret.yaml`（替换占位值，
或用 sealed-secrets / External Secrets）；两者均以 `envFrom` 整组注入，变量名与上表一致，
secret 生成产物不要提交仓库。

**MQTT 加固**：仓库自带 `docker-compose.hardened.yml`，为 EMQX 挂载
`deploy/emqx/emqx.conf` 并把 HTTP 认证/授权的 `x-emqx-auth` 密钥注入 EMQX 与
iot-access。与生产 override 一起叠加：

```bash
docker compose -f docker-compose.yml -f docker-compose.hardened.yml \
  -f docker-compose.prod.yml --profile app up -d
```

需要 `.env` 中已有 `EMQX_AUTH_SECRET` 与 `MQTT_PASSWORD`（`scripts/install.sh` 自动生成）。
默认 `docker-compose.yml` 不加载该 overlay，仍是开发形态。

**生产化最小 override**（与基础文件叠加）：

```bash
docker compose -f docker-compose.yml -f docker-compose.prod.yml --profile app up -d
```

```yaml
# docker-compose.prod.yml（示例，按需增删）
services:
  mysql:
    restart: unless-stopped
    environment:
      MYSQL_ROOT_PASSWORD: ${MYSQL_ROOT_PASSWORD:?required}
      MYSQL_PASSWORD: ${MYSQL_PASSWORD:?required}
    healthcheck:
      test: ["CMD", "mysqladmin", "ping", "-h", "localhost", "-uiot", "-p${MYSQL_PASSWORD}"]
  redis:
    restart: unless-stopped
    command: ["redis-server", "--appendonly", "yes"]
    volumes: [redisdata:/data]
  tdengine:
    restart: unless-stopped
    volumes: [tddata:/var/lib/taos]
  emqx:
    restart: unless-stopped
    volumes: [emqxdata:/opt/emqx/data]
volumes:
  redisdata:
  tddata:
  emqxdata:
```

六个业务服务同样加 `restart: unless-stopped`（连不上基础设施时会直接退出）。**端口不能用
override 删除**（`ports` 是合并语义）：要把基础设施收进本机/内网，请直接改基础文件的 `ports`
为 `127.0.0.1:<port>:<port>`，或用防火墙/安全组阻断公网。

### 2.5 轮换

- `JWT_SECRET`：无新旧双密钥支持。access（签发）、gateway / rule（校验）必须同时切换；
  轮换后所有已签发 token 立即失效，用户需重新登录。建议在维护窗口执行。
- `IOT_GATEWAY_SECRET`：同样无双密钥。gateway 与五个后端必须同一批重建；滚动过程中会
  出现短暂 401（旧 secret 的网关请求打到新 secret 的后端）。内网维护窗口内一次切换。
- `IOT_CRED_ENCRYPT_KEY`：在线轮换接口为 iot-access 的 `POST /api/access/keys/rotate`
  （直接访问 8082，携带 `x-gateway-secret` / `x-tenant-id` / `x-tenant-role: admin`；网关
  当前未代理该路径）。新值写 `IOT_CRED_ENCRYPT_KEY`，旧值留 `IOT_CRED_ENCRYPT_KEY_OLD` 解
  version=1 旧行；当前无批量重加密接口，旧值要保留到凭据重新保存或不再需要。
  `IOT_CRED_KEYS_FILE` 必须挂持久卷，连续轮换两次会让最早一代数据无法解密（只保留上一把）。
- `IOT_PASSWORD_PEPPER`：只影响尚未迁移的旧 HMAC 口令哈希；新口令为 PBKDF2-SHA256，
  自带随机盐、不读取 pepper。旧用户在下一次成功登录时会自动改写为 PBKDF2，全部迁移后
  该变量可移除。改动 pepper 会让尚未迁移的旧用户无法登录（需管理员重置口令）。
- MySQL / MinIO / Grafana / TDengine：在上线前改掉默认口令，并同步更新
  `DATABASE_URL` / `.env` / `GF_SECURITY_ADMIN_PASSWORD` / `TDENGINE_PASS`。

**初始管理员**：iot-access 首次启动创建 `admin`（租户 `tenant-1`），口令取
`ADMIN_INITIAL_PASSWORD`；未设置时随机生成 16 位口令并在启动日志
（`ADMIN_INITIAL_PASSWORD not set; generated initial admin password`）打印一次。
**不存在 `admin/admin123` 默认账号**。首次启动前就把 `ADMIN_INITIAL_PASSWORD` 设为
强随机值（`scripts/install.sh` 生成的即 64 位十六进制）；当前没有在线改密接口，需要
更换时按 PBKDF2 格式更新 `users.password_hash` 或重置该用户。

不要删除 `admin` 用户：下次 access 启动会因找不到同名用户而重新播种（使用当前
`ADMIN_INITIAL_PASSWORD` 或再次随机生成）。

## 3. TLS

### 3.1 反向代理终止（推荐）

网关直连 TLS 也可用：给 iot-gateway 设置 `TLS_CERT` / `TLS_KEY`（PEM 文件路径）即启用 HTTPS；
生产更建议在反代终止。**网关本身不代理 `/ws`**：WebSocket 直连 iot-rule 的 `/ws`，反代必须
单独转发 `/ws` 到 `127.0.0.1:8084` 并透传 Upgrade。前端生产环境（https）连接
`wss://<同域>/ws`，本地开发仍为 `ws://localhost:8084`。鉴权 JWT 通过
`Sec-WebSocket-Protocol: bearer, <JWT>` 传递，**不得再放进 URL query**；反代不要把该 header
写入访问日志。旧 `?token=` 客户端会收到 401，需要升级前端。iot-rule 单进程默认最多 1024 条 WS 连接
（`WS_MAX_CONNECTIONS`），compose 的 nginx `/ws` 另按 IP 限 20 条；多副本总上限为副本数相乘。写操作与
登录审计会记录 actor 与来源 IP。

Caddy 最小配置（自动 ACME 证书）：

```caddyfile
iot.example.com {
    @ws path /ws*
    reverse_proxy @ws 127.0.0.1:8084
    reverse_proxy 127.0.0.1:8080
}
```

Nginx 最小配置（证书可用 `scripts/renew-cert.sh` 续期）：

```nginx
server {
    listen 80;
    server_name iot.example.com;
    return 301 https://$host$request_uri;
}
server {
    listen 443 ssl http2;
    server_name iot.example.com;
    ssl_certificate     /etc/letsencrypt/live/iot.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/iot.example.com/privkey.pem;

    client_max_body_size 64m;   # 对齐 MAX_FIRMWARE_SIZE（64 MiB）

    location /ws {
        proxy_pass http://127.0.0.1:8084;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 3600s;
    }
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 60s;
    }
}
```

`/metrics`、`/ready`、`/health` 是基础设施端点，不属于 `/api/v1` 公共 API，服务端
无鉴权。不需要外部探活时，在反代层对 `/metrics`、`/ready` 返回 404。

### 3.2 MQTT 认证、授权与 TLS

**默认 compose 没有任何 MQTT 认证，只适用于本机开发。** 生产使用仓库自带的加固叠加：

```bash
docker compose -f docker-compose.yml -f docker-compose.hardened.yml --profile app up -d
```

- `deploy/emqx/emqx.conf` 配置 HTTP 认证与 HTTP ACL，指向 iot-access：
  `POST /api/access/mqtt/auth`（body `clientid/username/password`）与
  `POST /api/access/mqtt/acl`（body `username/topic/action/clientid`），两者都要求
  请求头 `x-emqx-auth` 等于 `EMQX_AUTH_SECRET`。
- 平台账号 `MQTT_USERNAME=ecat-access`（配 `MQTT_PASSWORD`）可发布/订阅 `iot/#`；
  设备账号由 admin 调用 `POST /api/v1/devices/{id}/mqtt-credential` 签发，用户名=十进制
  设备 id，密码只返回一次。设备连接时 **MQTT client id 必须等于用户名（设备 id）**，
  否则认证拒绝；设备只能发布 `iot/devices/{id}/properties`、
  `iot/devices/{id}/heartbeat`，只能订阅 `iot/devices/{id}/commands`。
- overlay 通过 `EMQX_AUTHENTICATION__1__HEADERS` /
  `EMQX_AUTHORIZATION__SOURCES__1__HEADERS` 注入真实密钥（emqx.conf 不能引用宿主机
  环境变量）。该配置按 EMQX 5.8 文档编写，**仓库内未在真实 EMQX 上联调**；上线前执行
  `docker compose exec emqx emqx_ctl conf show authentication` 与
  `... conf show authorization` 确认 url/headers 生效，并用真实设备验证 allow/deny。
- 多副本 iot-access 的 `MQTT_CLIENT_ID` 必须逐副本唯一，否则同 client id 会互相踢线。
- 设备删除/停用/解绑会删除 `mqtt_credentials`，认证查询联表 `devices` 并拒绝 `disabled`/`unbound`。
  已连接会话不会被 access 主动踢下线，需要等待重连，或接入 EMQX API 主动踢线。

TLS/mTLS 监听器（EMQX 5.x `emqx.conf`）：加固 overlay 会以 `deploy/emqx/emqx.conf`
整体替换镜像内的同名文件（仅该文件只读，其余 `etc` 不受影响），TLS 段落应追加到该文件；
node cookie 等自定义项同理，否则使用 EMQX schema 默认值。

```hocon
listeners.ssl.default {
  bind = "0.0.0.0:8883"
  ssl_options {
    cacertfile = "/opt/emqx/etc/certs/ca.pem"
    certfile   = "/opt/emqx/etc/certs/server.pem"
    keyfile    = "/opt/emqx/etc/certs/server.key"
    verify     = verify_peer
    fail_if_no_peer_cert = true
  }
}
```

- 证书 SAN 必须包含服务连接使用的主机名；`ca.pem` 为签发客户端/服务端证书的 CA。

iot-access 的 MQTT 客户端由以下环境变量配置（`mqtt_config_from_env`，fail-closed）：

| 变量 | 说明 |
|------|------|
| `MQTT_URL` | `tcp://host:1883` 或 `ssl://host:8883` / `mqtts://host`；缺省 `tcp://localhost:1883` |
| `MQTT_TLS` | `1/true/yes/on` 或 `0/false/no/off`；非法值启动失败。缺省由 `ssl://`/`mqtts://` 推断；TLS scheme + `0` 启动失败 |
| `MQTT_CA_FILE` | 自定义 CA（PEM）；设置即强制 TLS |
| `MQTT_CERT_FILE` / `MQTT_KEY_FILE` | 客户端证书/私钥（mTLS），必须成对设置 |
| `MQTT_USERNAME` / `MQTT_PASSWORD` / `MQTT_CLIENT_ID` | 前两者必须成对；client id 可选，缺省 `ecat-mqtt` |

- 明文连接要求 `MQTT_URL=tcp://...` 且未设 CA/客户端证书。`ssl://`/`mqtts://` 与 `MQTT_TLS=0` 同时出现时启动报错 `MQTT_TLS=0 conflicts with TLS URL scheme`，不会降级为明文。
- `MQTT_TLS=1` 可与 `tcp://` 搭配（TLS 强制启用，端口取自 URL，缺省 8883）；设置 `MQTT_CA_FILE` 或客户端证书同样强制 TLS。
- 证书/口令只配一半、或证书文件不可读时，进程启动即失败。

示例（EMQX 8883 双向认证）：

```bash
MQTT_URL=ssl://emqx:8883
MQTT_CA_FILE=/etc/iot/certs/ca.pem
MQTT_CERT_FILE=/etc/iot/certs/access.pem
MQTT_KEY_FILE=/etc/iot/certs/access.key
MQTT_USERNAME=ecat-access
MQTT_PASSWORD=<强口令>
EMQX_AUTH_SECRET=<与 iot-access 相同的随机密钥>
```

证书文件需挂载进 iot-access 容器（compose 加只读 volume）；`MQTT_PASSWORD` 走 `.env`/Secret。

### 3.3 内网明文边界

compose 内部网络中的 MySQL / Redis / Kafka / TDengine / OpenSearch / EMQX / 服务间 HTTP
均为明文。安全边界依赖：只让反代入口对外、基础设施端口不发布到公网、宿主机防火墙限制
来源网段。跨主机部署时，基础设施应使用各自的原生 TLS（MySQL TLS、Redis TLS、Kafka
SASL_SSL、OpenSearch TLS）或置于私有网络/服务网格内。

## 4. 数据持久化与备份/恢复

| 组件 | compose 卷 | 数据重要性 | 备份方式 |
|------|-----------|-----------|---------|
| MySQL | `mysqldata` | 主数据（设备/用户/规则/审计） | `mysqldump --single-transaction` + binlog |
| TDengine | 无（需补卷） | 时序历史 | `taosdump` |
| Redis | 无（需补卷 + AOF） | 限流/影子/广播，可降级重建 | RDB/AOF 文件 |
| MinIO | `miniodata` | 对象存储（六服务当前未依赖） | `mc mirror` |
| Kafka | `kafkadata` | 事件总线 | 无逻辑备份，靠保留策略 + 多副本 |
| OpenSearch | `opensearchdata` | 检索索引（可重建） | 快照（可选） |

先载入 `.env`：

```bash
set -a; . ./.env; set +a
mkdir -p backups
```

### 4.1 MySQL

```bash
# 逻辑备份（含 binlog 位点，便于 PITR；MySQL 8.0.26+ 用 --source-data=2）
docker compose exec -T mysql sh -c \
  'exec mysqldump -uroot -p"$MYSQL_ROOT_PASSWORD" --single-transaction --quick \
   --routines --triggers --events --source-data=2 --set-gtid-purged=OFF iot' \
  > "backups/iot-$(date +%F).sql"

# 确认 binlog 开启（MySQL 8 默认开启；位点写在 dump 头部注释）
docker compose exec mysql mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -e "SHOW VARIABLES LIKE 'log_bin';"
```

恢复：

```bash
docker compose exec -T mysql sh -c 'exec mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -e "CREATE DATABASE IF NOT EXISTS iot"'
docker compose exec -T mysql sh -c \
  'exec mysql -uroot -p"$MYSQL_ROOT_PASSWORD" iot' < backups/iot-YYYY-MM-DD.sql
# PITR：从 dump 头部记录的 binlog 文件/位点继续回放
docker compose exec mysql sh -c 'exec mysqlbinlog --start-position=<POS> /var/lib/mysql/<binlog.000001>' \
  | docker compose exec -T mysql sh -c 'exec mysql -uroot -p"$MYSQL_ROOT_PASSWORD" iot'
```

旧版 `mysqldump` 用 `--master-data=2`；binlog 默认在数据目录，随 `mysqldata` 卷持久化，
跨机备份时一并复制。

### 4.2 TDengine

`taosdump` 走原生端口 6030，compose 只发布了 REST 6041，因此在容器内执行：

```bash
docker compose exec tdengine taosdump -h 127.0.0.1 -u root -p "${TDENGINE_PASS:-taosdata}" \
  -D iot -o /var/lib/taos/backup
docker compose cp tdengine:/var/lib/taos/backup "backups/taos-$(date +%F)"
```

恢复（先停 iot-data，避免边恢复边写入）：

```bash
docker compose cp "backups/taos-YYYY-MM-DD" tdengine:/var/lib/taos/restore
docker compose exec tdengine taosdump -h 127.0.0.1 -u root -p "${TDENGINE_PASS:-taosdata}" \
  -i /var/lib/taos/restore
```

`taosdump` 版本需与 TDengine 3.3 服务端匹配；基础 compose 的 tdengine 没有卷，必须先按 2.4 补 `tddata:/var/lib/taos`，否则容器重建即丢全部时序数据。

### 4.3 Redis

Redis 只保存可重建/可降级数据（限流计数、影子缓存、告警广播），丢失不会破坏主数据，但
会丢失限流计数与缓存。生产开启 AOF（2.4 的 override）并挂卷。备份：

```bash
docker compose exec redis redis-cli BGSAVE
docker compose cp redis:/data/dump.rdb "backups/redis-$(date +%F).rdb"
```

恢复：停 redis → 把 `dump.rdb` 放回 `/data`（覆盖）→ 启动；AOF 模式同时保留 `appendonly.aof` 与 `appendonlydir/`（Redis 7）。

### 4.4 MinIO

```bash
mc alias set iot-minio http://127.0.0.1:9000 "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD"
mc mirror --overwrite iot-minio/<bucket> "backups/minio/<bucket>"
# 恢复
mc mirror --overwrite "backups/minio/<bucket>" iot-minio/<bucket>
```

注意：生产 compose 使用 `quay.io/minio/minio:RELEASE.2025-04-22T22-12-26Z` 与固定的
`minio/mc` 版本（官方 Docker Hub 镜像已下架），升级前先备份 `data/minio`。只有配置
`OTA_S3_*` 时 OTA 固件才落 MinIO；未配置则落本地 `FIRMWARE_DIR`，但仍建议备份
MinIO 数据（宿主机需安装 `mc`）。

### 4.5 Kafka 保留策略

Kafka 是事件总线（topic `iot.events`，消息以 `tenant_id` 为分区 key；消费组
`iot-rule-rules`、`iot-data-ingest`、`iot-anomaly`），没有“备份”，只有保留与多副本：

```bash
# 按业务保留期与磁盘容量调整（示例：7 天 / 10 GiB）
docker compose exec kafka kafka-configs.sh --bootstrap-server kafka:9092 \
  --alter --entity-type topics --entity-name iot.events \
  --add-config retention.ms=604800,retention.bytes=10737418240

# 显式建 topic（分区数按消费并行度设置）
docker compose exec kafka kafka-topics.sh --bootstrap-server kafka:9092 \
  --create --if-not-exists --topic iot.events --partitions 12 --replication-factor 1
```

单机单 broker 无法容忍磁盘/进程故障；需要事件不丢时改用多 broker 集群
（`replication.factor>=2`、`min.insync.replicas>=1`，由 broker 侧配置）。

### 4.6 恢复演练

上线前至少完整演练一次：

1. 在隔离环境用同一批镜像启动空基础设施。
2. 恢复 MySQL dump（必要时回放 binlog），核对关键表行数与最大时间戳。
3. 按第 5 节顺序启动服务，等待全部 `/ready` 返回 200。
4. 用管理员登录、拉设备列表、查询历史曲线、确认 `/metrics` 有数据；记录实际恢复耗时（RTO）
   与可接受的数据丢失窗口（RPO），写进运维手册（不要照搬其他环境的数字）。

## 5. 发布与升级

### 5.1 构建与标签

生产镜像用不可变 tag（如 git sha），不要用 `:local`/`:latest`：

```bash
TAG=$(git rev-parse --short HEAD)
for bin in iot-gateway iot-device iot-access iot-rule iot-data iot-cdn; do
  docker build -f deploy/docker/Dockerfile --build-arg BIN="$bin" \
    -t "registry.example.com/$bin:$TAG" .
  docker push "registry.example.com/$bin:$TAG"
done
```

生产 compose 的 `image:` 指向仓库 tag；`build:` 只保留给本地开发。

### 5.2 启动顺序

基础设施与可观测性 → 数据库就绪 → 业务服务（启动时自动执行内联迁移）。
使用 MQTT 加固时，下面所有 `docker compose` 命令都要叠加
`-f docker-compose.hardened.yml`。

```bash
docker compose up -d mysql redis kafka emqx tdengine opensearch \
  prometheus grafana loki promtail kafka-exporter
# 先载入 .env（set -a; . ./.env; set +a），再等 MySQL 可连接（其余按 READY 状态/日志确认）
until docker compose exec -T mysql mysqladmin ping -h localhost -uiot -p"$MYSQL_PASSWORD" --silent; do sleep 2; done

# 先起迁移集最全的 iot-access（0001~0007），其余服务顺序不限
docker compose --profile app up -d iot-access
docker compose --profile app up -d iot-device iot-rule iot-data iot-cdn iot-gateway
```

迁移在服务启动时执行，均为幂等脚本（`CREATE TABLE IF NOT EXISTS` + `information_schema`
补列），无需手工跑 SQL。iot-device 只内联 0001~0005，iot-access 内联 0001~0007，因此
建议 access 先起。

`depends_on` 只保证容器启动顺序，不等待健康状态；必须用上面的探测/重试或
`restart: unless-stopped` 兜底。

### 5.3 健康检查

| 服务 | 端口 | `/health` | `/ready` 检查内容 |
|------|------|-----------|------------------|
| iot-gateway | 8080 | 进程存活即 200 | 未注册依赖检查，进程存活即 200 |
| iot-device | 8081 | 200 | MySQL `SELECT 1` |
| iot-access | 8082 | 200 | MySQL `SELECT 1` |
| iot-data | 8083 | 200 | TDengine `server_status()`（ClickHouse 为 `SELECT 1`） |
| iot-rule | 8084 | 200 | MySQL `SELECT 1` |
| iot-cdn | 8085 | 200 | MySQL `SELECT 1` |

`/health` 是存活探针，不反映依赖；编排/负载均衡的就绪判断必须用 `/ready`。注意 gateway
的 `/ready` 不检查下游，网关进程活着但后端全挂时仍返回 200——请依赖 ServiceDown/5xx 告警。

```bash
for p in 8080 8081 8082 8083 8084 8085; do
  printf '%s: ' "$p"; curl -fsS "http://127.0.0.1:$p/ready" >/dev/null && echo ready || echo NOT-READY
done
```

### 5.4 滚动升级与回滚

- **单副本 compose**：逐服务 `docker compose --profile app up -d --no-deps <service>`，每次等
  `/ready`；切换期间该服务短暂不可用，Kafka 消费组在 iot-rule / iot-data 重启时短暂 rebalance。
- **多副本**：用 K8s Deployment 滚动更新（`deploy/k8s/services.yaml`）或自行蓝绿/灰度；不要在同一 compose 项目内起第二套（端口冲突）。
- **配置变更**：环境变量在容器启动时注入，改 `.env` 后必须 `docker compose --profile app up -d --force-recreate <service>` 才生效。
- **回滚**：把 `image:` 指回上一个 tag 并重建即可。迁移是向前兼容的加列/建表，旧二进制通常可继续运行；但执行过 uuid → snowflake 主键迁移（见 [`upgrade-legacy-ids.md`](upgrade-legacy-ids.md)）后，旧版本二进制**禁止**再连库。有破坏性迁移时，回滚前先按第 4 节恢复备份。
- **升级前必做**：MySQL 全量备份 + binlog 位点记录；确认上一个镜像 tag 仍可用且回滚步骤已在演练环境验证。

## 6. 监控与告警

### 6.1 端点与采集

六个服务都在各自端口暴露 `GET /metrics`（Prometheus 文本格式，无鉴权，仅内网可达）。
`deploy/prometheus/prometheus.yml` 已静态抓取 6 个业务服务 + Prometheus 自身 + `kafka-exporter`；
Grafana 已自动配置 Prometheus 与 Loki 数据源（`deploy/grafana/datasources.yml`）。
仓库**没有预置 Grafana dashboard**，需要自行导入/创建面板。

- Grafana：`http://<内网>:3000`，用户 `admin`；上线前设置 `GF_SECURITY_ADMIN_PASSWORD`（默认 `admin`）。
- Loki：`http://<内网>:3100`，promtail 通过 `docker.sock` 采集容器 stdout；`LOG_FORMAT=json`
  便于结构化检索。rootless Docker 的容器日志目录不在 `/var/lib/docker/containers`，需按
  `deploy/promtail/promtail.yml` 注释调整。`docker.sock` 等价宿主机 root 权限，只部署在受信节点。
- Prometheus：`http://<内网>:9090`。compose 未开 `--web.enable-lifecycle`，改规则后用
  `docker compose restart prometheus` 生效（不要用 `/-/reload`）。

### 6.2 告警规则

`deploy/prometheus/alerts.yml`（已通过 `promtool check rules`）包含 4 条规则：

| 告警 | 表达式 | for | 级别 |
|------|--------|-----|------|
| `ServiceDown` | `up == 0` | 2m | critical |
| `Http5xxRate` | 各 job 5xx 占比 > 5% | 10m | warning |
| `HttpP95Latency` | P95 请求时延 > 1s | 10m | warning |
| `KafkaConsumerLag` | `kafka_consumergroup_lag > 10000`（按分区） | 10m | warning |

注意：

- `ServiceDown` 对未启动的 app profile 服务同样会触发（抓取目标不存在）。
- `Http5xxRate` 只统计 `status=~"5.."`；传输层错误记 `status="error"`，不计入该比值，
  需要覆盖时把表达式改为 `status=~"5..|error"`。
- `KafkaConsumerLag` 的数据源是随 compose 内置的 `kafka-exporter`（抓取 job
  `kafka-exporter:9308`），默认即可用；exporter 未运行时该告警无数据，先查该容器与 targets。

- **未部署 Alertmanager**：规则只在 Prometheus 的 `/alerts` 页面可见，不发送任何通知。
  需要邮件/IM 通知时另行部署 Alertmanager，并在 `prometheus.yml` 增加 `alerting:` 段与
  `alertmanagers:` 配置；这是本仓库尚未覆盖的运维组件。

## 7. 上线检查清单

- [ ] `.env` 权限 600，未提交仓库；`JWT_SECRET`、`IOT_GATEWAY_SECRET`、
      `IOT_CRED_ENCRYPT_KEY`、`IOT_PASSWORD_PEPPER`、`ADMIN_INITIAL_PASSWORD`、
      `EMQX_AUTH_SECRET`、`MQTT_PASSWORD` 均由 `openssl rand` 生成且各环境不同
- [ ] `JWT_SECRET` 在 gateway / access / rule 三处一致；`IOT_GATEWAY_SECRET` 在六处一致；
      `IOT_CRED_ENCRYPT_KEY` 在 access / cdn 一致
- [ ] MySQL root/iot、MinIO、Grafana、TDengine 默认口令已更换，连接串同步更新；初始管理员
      使用强随机 `ADMIN_INITIAL_PASSWORD`（而非日志随机值）；旧 HMAC 用户已完成登录迁移
- [ ] MQTT 使用 `docker-compose.hardened.yml` 启动，`emqx_ctl conf show authentication`
      与 `show authorization` 显示 HTTP 端点与 `x-emqx-auth` 已生效；设备凭据已逐个签发
- [ ] `ACCESS_CALLBACK_BASE` 指向外部 HTTPS 域名
- [ ] 反代 80/443 就绪，证书自动续期（`scripts/renew-cert.sh` 或 ACME）验证通过
- [ ] `/ws` 反代透传 Upgrade 且长连接超时已调大；固件上传体积上限 ≥ 64 MiB
- [ ] 基础设施端口未暴露公网；`/metrics`、`/ready` 未对外
- [ ] MySQL 定时备份 + binlog 保留已配置；TDengine 已补持久卷并配置 taosdump；
      Redis 已开 AOF 并挂卷；Kafka 保留策略与副本数符合业务 RPO
- [ ] 备份恢复与回滚演练完成：上一镜像 tag 可启动、备份可恢复，RTO/RPO 已记录
- [ ] 六服务 `restart: unless-stopped`；`/ready` 全部 200
- [ ] Prometheus 抓取目标全 UP（含 kafka-exporter），4 条告警规则已加载；Grafana 可登录且
      数据源正常；Loki 能查到日志
- [ ] 压测已按真实业务模型完成，容量结论来自本环境实测（不含本仓库之外的臆测数字）
- [ ] 旧库（uuid 主键）已按 [`upgrade-legacy-ids.md`](upgrade-legacy-ids.md) 迁移并校验
