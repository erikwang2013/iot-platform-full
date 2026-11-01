# 开放 API 使用指南

面向第三方集成的开放 API：设备列表/状态/物模型/历史数据等读端点，以及
设备命令/规则管理/厂商导入等写端点，均可用 app_id/app_secret 换取 JWT 后调用。

密钥带 **scope**（创建时指定）：`read`（默认）只能调读端点，`write`/`command`
可调写端点（映射为 operator 角色，经网关 RBAC 放行）。

> 自 v1.19.0 起所有公开 REST 端点统一使用 `/api/v1` 前缀；旧的 `/api` 与
> `/admin` 前缀已移除。

## 1. 注册密钥（管理端，admin 角色）

```bash
# 创建只读密钥（scope 省略即 read）
curl -s -X POST http://localhost:8080/api/v1/api-keys \
  -H "Authorization: Bearer $ADMIN_JWT" -H "Content-Type: application/json" \
  -d '{"name": "partner-bi"}'
# → { "app_id": "<uuid>", "app_secret": "<64-hex>", "scope": "read" }

# 创建可写密钥（设备命令 / 规则管理 / 厂商导入）
curl -s -X POST http://localhost:8080/api/v1/api-keys \
  -H "Authorization: Bearer $ADMIN_JWT" -H "Content-Type: application/json" \
  -d '{"name": "partner-ops", "scope": "write"}'

# 列表（含 scope，secret 不回显）
curl -s http://localhost:8080/api/v1/api-keys \
  -H "Authorization: Bearer $ADMIN_JWT"

# 吊销
curl -s -X DELETE http://localhost:8080/api/v1/api-keys/{app_id} \
  -H "Authorization: Bearer $ADMIN_JWT"
```

scope 取值：`read`（仅读端点）| `write`（全部写端点）| `command`（当前 RBAC
角色模型下与 write 等价，为设备命令类密钥预留语义）。密钥创建后 scope 不可改，
需吊销重建。

## 2. 换 token（公开端点，无 JWT）

```bash
curl -s -X POST http://localhost:8080/api/v1/access/open/token \
  -H "Content-Type: application/json" \
  -d '{"app_id": "175921860444160", "app_secret": "<64-hex>"}'
# → { "token": "<jwt>", "tenant_id": "...", "role": "operator", "scope": "write" }
```

失败统一 401 `invalid app_id or app_secret`（已吊销/不存在的密钥同样处理，防枚举）。
token 有效期同登录（JWT_TTL_SECS，默认 24h）。scope=read 的密钥换出的
role=read-only，写端点一律 403。

## 3. 调用端点

所有请求带 `Authorization: Bearer <token>`；版本由路径前缀 `/api/v1` 固定。

### 读端点（全部 scope 可用）

| 端点 | 说明 |
|------|------|
| `GET /api/v1/devices` | 设备列表（支持 `?group_id=` / `?tag=` 过滤） |
| `GET /api/v1/devices/stats` | 设备统计 |
| `GET /api/v1/devices/{id}/tags` | 设备标签 |
| `GET /api/v1/models/things` | 物模型条目 |
| `GET /api/v1/models/things/{device_id}` | 设备物模型（属性/事件/服务分组） |
| `GET /api/v1/data/history?device_id=&code=&start=&end=` | 历史曲线（start/end 为 epoch 毫秒，可选） |
| `GET /api/v1/data/export?...` | 历史导出 |
| `GET /api/v1/rule/alerts` | 告警记录 |
| `GET /api/v1/rule/stats` | 规则统计 |
| `GET /api/v1/rule/reports?date=YYYY-MM-DD` | 每日汇总报表（可选按日期过滤） |

### 写端点（scope=write/command 可用）

| 端点 | 说明 |
|------|------|
| `POST /api/v1/access/devices/{id}/command` | 下发设备指令（body: `{"code":"...","value":...}`） |
| `POST /api/v1/access/vendors/{vendor}/import` | 拉取厂商设备列表入库 |
| `POST /api/v1/rule/rules` | 创建规则 |
| `PUT /api/v1/rule/rules/{id}` | 更新规则 |
| `DELETE /api/v1/rule/rules/{id}` | 删除规则 |
| `POST /api/v1/rule/alerts/{id}/ack` | 确认告警 |
| `POST /api/v1/devices/batch` | 批量创建设备 |
| `PUT /api/v1/devices/{id}` / `DELETE /api/v1/devices/{id}` | 更新/删除设备 |
| `POST /api/v1/devices/groups` / `DELETE /api/v1/devices/groups/{id}` | 设备分组管理 |

OTA 固件/任务、场景与通知、通知渠道写操作、租户/用户管理、API 密钥管理、CDN provider
配置为 **admin-only** 或管理端专用；开放密钥（含 write scope）不可达。非 admin 读取
CDN/通知渠道配置时只返回脱敏字段，详见第 4 节。

```bash
# 下发指令示例
curl -s -X POST http://localhost:8080/api/v1/access/devices/{id}/command \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" -d '{"code": "power", "value": "on"}'
```

## 4. 管理端专用端点（开放密钥不可达）

以下端点仅管理端登录/JWT 可达，开放密钥调用返回 403：

| 端点 | 说明 |
|------|------|
| `GET/POST /api/v1/ota/firmwares` | 固件列表 / 登记（URL 直填） |
| `POST /api/v1/ota/firmwares/upload` | multipart 二进制上传（≤64 MiB，网关放行该路径） |
| `DELETE /api/v1/ota/firmwares/{id}` | 删除固件（S3 对象一并删除） |
| `POST /api/v1/ota/firmwares/{id}/signed-url` | 生成 CDN 签名下载 URL（`expires_secs` 60–86400，可选） |
| `GET /api/v1/ota/firmwares/{id}/downloads` | 下载次数与最近下载时间 |
| `POST /api/v1/devices/{id}/mqtt-credential` | 签发设备 MQTT 用户名/口令（密码只返回一次） |
| `GET/POST /api/v1/ota/tasks` | 升级任务列表 / 创建（响应含 `download_url`） |
| `POST /api/v1/ota/tasks/batch` | 灰度批量创建（`rollout_percent` 1–100，单次最多 1000 台） |
| `POST /api/v1/ota/tasks/{id}/report` | 设备上报升级进度（`downloading` / `installing` / `success` / `failed`） |
| `POST /api/v1/ota/tasks/{id}/rollback` | 回滚升级任务 |
| `GET/POST /api/v1/rule/scenes`、`PUT/DELETE /api/v1/rule/scenes/{id}` | 场景 CRUD |
| `POST /api/v1/rule/scenes/{id}/execute` | 立即执行场景（禁用状态也可手动执行） |
| `GET /api/v1/rule/notifications?unread=1` | 通知列表（倒序最多 100 条） |
| `POST /api/v1/rule/notifications/{id}/read` | 标记通知已读 |
| `PUT/DELETE /api/v1/rule/channels/{channel}` | 创建/更新/删除通知渠道（仅 admin） |
| `/api/v1/tenants`、`/api/v1/users`、`/api/v1/api-keys` | 租户 / 用户 / API 密钥管理（仅 admin） |
| `/api/v1/cdn/providers*` | CDN provider 配置（仅 admin；非 admin 查询脱敏） |

固件二进制上传（multipart，单文件上限 64 MiB）由 iot-device 的
`POST /api/v1/ota/firmwares/upload` 提供，落 `FIRMWARE_DIR` 或已配置的 S3/MinIO。
固件公开下载 `GET /api/v1/ota/public/firmwares/{id}/download?token=...` 不校验 JWT，
凭 URL 中的 HMAC token 访问，有效期由 `OTA_SIGN_EXPIRES_SECS` 控制；设备 MQTT 凭据
由 admin 逐个签发，设备连接时 client id 必须等于用户名（设备 id）。
OTA 相关环境变量（`OTA_S3_*`、`OTA_PUBLIC_BASE`、`OTA_CDN_PROVIDER_ID`、`CDN_BASE`）
见 README 与[生产部署指南](deploy/production.md)。

## 5. 安全说明

- app_secret 库中只存 HMAC-SHA256 哈希（以 app_id 为盐），数据库泄露不可逆推。
- 只读 token 调写端点返回 403（JWT role=read-only 受网关 RBAC 拦截）。
- 写密钥可下发设备指令/改规则，属高权限凭证，请按最小权限申请 scope 并妥善保管。
- 全局限流 100 次/分钟按租户计数，开放客户端共享同一配额。
- 吊销立即生效：换 token 时校验 revoked_at，已吊销密钥直接 401。
- 管理端写操作与登录尝试写入审计日志，记录 actor 与来源 IP；审计查询仅 admin 可见。
