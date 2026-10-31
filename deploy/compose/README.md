# 单机生产 Docker Compose

独立于仓库根目录的开发用 `docker-compose.yml`。所有服务各自一个容器；对外只开 nginx
80/443 与 EMQX 8883（MQTT over TLS）。数据、日志、密钥全部落在宿主机目录。

## 1. 首次部署

前置：Docker 24+ / Compose v2、openssl、宿主机域名已解析到本机。

```bash
# 1) 放置唯一需要人工提供的证书（Let's Encrypt / 商业证书）
#    deploy/certs/fullchain.pem、deploy/certs/privkey.pem
#    （scripts/renew-cert.sh 只负责 certbot 续期；续期后需把这两个文件复制到 deploy/certs/，
#      再 docker compose ... restart nginx）

# 2) 生成 .env、内部 CA/EMQX 证书、数据目录（会打印 admin 初始口令）
#    首次生成时用 DOMAIN 覆盖示例域名；之后修改 .env 的 DOMAIN 需再跑 --regenerate
#    （会重新签发内部证书并轮换应用密钥）
sudo DOMAIN=iot.example.com scripts/init-prod.sh

# 3) 构建并启动
docker compose -f deploy/compose/docker-compose.yml up -d --build

# 可选：Prometheus + Grafana（Grafana 仅绑 127.0.0.1:3000）
docker compose -f deploy/compose/docker-compose.yml --profile monitoring up -d
```

仅想校验 compose 语法、尚无域名证书时：

```bash
SKIP_CERT_CHECK=1 scripts/init-prod.sh
docker compose -f deploy/compose/docker-compose.yml config -q
```

启动后逐服务确认 `/ready`（在宿主机上通过容器网络或 `docker compose exec` 检查）：

```bash
docker compose -f deploy/compose/docker-compose.yml ps
docker compose -f deploy/compose/docker-compose.yml logs -f --tail=100 iot-access nginx
```

首次启动顺序由 healthcheck 保证（MySQL/Redis/Kafka/EMQX/OpenSearch/TDengine 健康后业务
服务才启动）；`iot-access` 建议最先就绪，它内联的迁移最全。

## 2. 端口与入口

| 入口 | 说明 |
|------|------|
| `80` | 仅 `/healthz` 返回 200，其余 301 到 HTTPS |
| `443` | `/` → iot-gateway:8080；`/ws` → iot-rule:8084（WebSocket Upgrade） |
| `443` `/iot-ota/` | OTA 固件对象（MinIO `iot-ota` bucket 的 `firmware/` 前缀，只读） |
| `8883` | 设备 MQTT over TLS（EMQX） |
| `127.0.0.1:3000` | Grafana（仅 monitoring profile） |

MySQL / Redis / Kafka / MinIO / TDengine / OpenSearch / EMQX 1883 与 dashboard / 六个业务
服务端口一律只在 compose 内网 `expose`，不发布到宿主机。`/metrics`、`/health`、`/ready`
在 nginx 上直接返回 404。

## 3. 数据、日志与备份

所有持久数据是 bind mount，位于 `deploy/compose/data/`（整个目录已 gitignore）：

| 目录 | 内容 |
|------|------|
| `mysql/` | MySQL 数据 + binlog + `error.log` |
| `redis/` | Redis AOF（`appendonly yes`） |
| `kafka/` | Kafka KRaft 数据 |
| `minio/` | MinIO 数据（OTA bucket） |
| `tdengine/`、`tdengine-log/` | TDengine 数据/日志 |
| `opensearch/`、`opensearch-logs/` | OpenSearch 数据/日志 |
| `emqx/`、`emqx-log/` | EMQX 数据/日志 |
| `firmware/` | iot-device 本地固件兜底目录 |
| `keys/` | iot-access 凭据密钥环 `cred_keys.json` |
| `certs/` | 内部 CA 与 EMQX 服务端证书（`ca.key` 不挂载进容器） |
| `prometheus/`、`grafana/` | 监控数据 |

容器 stdout 由 Docker `local` 日志驱动限制为 10 MiB × 3；nginx 的 access/error log
直接写 stdout/stderr；MySQL/EMQX/OpenSearch/TDengine 的文件日志在上述数据目录内。

备份/恢复命令（mysqldump、taosdump、mc mirror 等）沿用
[`docs/deploy/production.md`](../../docs/deploy/production.md) 第 4 节，把 `docker compose`
命令加上 `-f deploy/compose/docker-compose.yml` 即可。

## 4. 密钥与轮换

`scripts/init-prod.sh` 生成并写入 `deploy/compose/.env`（600）：

- 应用密钥：`JWT_SECRET`、`IOT_GATEWAY_SECRET`、`IOT_CRED_ENCRYPT_KEY`、
  `IOT_PASSWORD_PEPPER`、`ADMIN_INITIAL_PASSWORD`、`EMQX_AUTH_SECRET`、
  `MQTT_USERNAME`/`MQTT_PASSWORD`；
- 基础设施口令：MySQL root/iot、Redis、MinIO root、TDengine root、Grafana、EMQX dashboard；
- 地址与 OTA：`DOMAIN`、`PLATFORM_TENANT`、`PUID`/`PGID`、`ACCESS_CALLBACK_BASE`、
  `OTA_PUBLIC_BASE`、`OTA_S3_*`。

轮换应用密钥：

```bash
sudo scripts/init-prod.sh --regenerate
docker compose -f deploy/compose/docker-compose.yml up -d --force-recreate
```

- 旧 `.env` 备份到 `deploy/compose/backups/.env.<时间戳>`，永不删除；
- `IOT_CRED_ENCRYPT_KEY` 轮换后旧值写入 `IOT_CRED_ENCRYPT_KEY_OLD`（代码支持宽限解密）。
  代码只保留上一把旧钥：连续执行两次 `--regenerate` 后，最早一代密钥只能从备份 `.env` 找回；
  **注意**：若曾调用在线轮换接口生成了 `data/keys/cred_keys.json`，该文件优先于环境变量；
  此时继续用接口轮换，或先重加密再换，避免只改 `.env` 不生效；
- 基础设施口令**不会**被自动轮换（改了会连不上已落盘数据），需人工在数据库/MinIO/
  TDengine/Grafana 内修改后同步 `.env`；旧值仍在备份里；
- JWT 轮换后所有已签发 token 失效，用户需重新登录；`IOT_GATEWAY_SECRET` 轮换需六个业务
  容器同批重建；`MQTT_PASSWORD` 变更需同步更新平台侧 MQTT 客户端。

内部 CA 也会在 `--regenerate` 时轮换（旧文件备份为 `*.old`）。已信任旧 CA 的设备需要
重新安装 `data/certs/ca.pem`。域名证书始终由用户维护，脚本只检查存在性。

设备侧：MQTT 客户端需信任 `data/certs/ca.pem`（内部 CA），或由运维改用公网域名证书；
连接地址 `ssl://<DOMAIN>:8883`，平台的 iot-access 用 `ssl://emqx:8883` + 内部 CA。

## 5. 已知未验证项 / 上线前必查

本仓库无法在不启动重型镜像的情况下完成实机联调，以下按官方文档编写，**上线前必须核对**：

1. EMQX：`deploy/compose/emqx.conf` 的 8883 listener、dashboard 绑定、HTTP auth/ACL
   是否被镜像接受。执行
   `docker compose -f deploy/compose/docker-compose.yml exec emqx emqx ctl conf show listeners`
   与 `conf show authentication` / `conf show authorization`；用真实设备验证 allow/deny。
2. TDengine：`TAOS_ROOT_PASSWORD` 需要镜像 ≥ 3.3.6.6；若 `tdengine/tdengine:3.3` 当前指向
   更旧补丁，root 口令仍为 `taosdata`，需手工 `ALTER USER root PASS '...'` 并同步 `.env`。
3. MinIO：`minio-init` 用 `mc anonymous set download <bucket>/firmware` 给固件前缀匿名读
   权限（OTA 下载 URL 不带签名）。若需要租户隔离，改为走 CDN 签名或不启用 `OTA_S3_*`
   （改用本地 `data/firmware`）。`nginx.conf` 的 `/iot-ota/` 路由按默认 bucket 名
   `iot-ota` 写死；修改 `OTA_S3_BUCKET` 时需同步修改该 location。
4. 数据目录属主：脚本按各官方镜像默认 UID chown（EMQX/MinIO/OpenSearch 1000、Kafka
   1001、Prometheus 65534、Grafana 472）。以 root 运行脚本；若镜像版本变更 UID，按容器
   日志修正属主。healthcheck 依赖镜像内工具：MinIO/OpenSearch 的 `curl`、Kafka 的
   `kafka-topics.sh`、Prometheus 的 `wget`、TDengine 的 `taos`、EMQX 的 `emqx ctl`；
   若某个镜像版本裁剪了这些工具，`docker compose ps` 会显示 unhealthy，需改用等价探针。
5. 单机单副本：任一基础设施容器故障即全站不可用；`mem_limit`/`cpus` 只是上限，按实际
   容量压测后调整。Kafka 单 broker、OpenSearch 单节点，无高可用。

## 6. 日志与排障

```bash
docker compose -f deploy/compose/docker-compose.yml logs -f --tail=200 <service>
docker compose -f deploy/compose/docker-compose.yml exec mysql \
  mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -e 'SHOW DATABASES;'   # 先 source deploy/compose/.env
```

修改 `.env` 后必须 `up -d --force-recreate <service>` 才生效；修改 `nginx.conf` /
`emqx.conf` 后 `docker compose ... restart nginx|emqx`。
