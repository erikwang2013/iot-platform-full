# 多实例水平扩展部署说明

目标：业务服务多副本部署时，把跨实例不一致收敛到"与单实例重启相同"的程度。
**不再承诺完全无状态**：规则去重窗口、场景 latest 缓存、异常检测基线都在进程内，
Kafka rebalance 或进程重启后会重置——这与单实例重启的行为一致，期间可能重复处理
最后一批事件（at-least-once），写入侧必须幂等。

> **单机 compose 不是多副本方案**：[`deploy/compose/docker-compose.yml`](../../deploy/compose/docker-compose.yml)
> 每个服务只起一个副本，使用固定宿主机端口与 `deploy/compose/data/` bind mount，且
> Kafka/OpenSearch 等基础设施均为单节点。需要多副本时使用 `deploy/k8s/` 或自建编排；
> 不要对同一 compose 项目直接 `--scale`（端口与数据目录冲突，基础设施仍是单点）。

## 关键状态与多实例行为

| 有状态点 | 存储 / 机制 | 多实例行为 |
|---------|------------|-----------|
| 登录限流 / 全局限流 | Redis（不可用降级内存） | 计数共享；全局限流按 **JWT 解出的租户** 计数（未登录按来源 IP），不信任客户端 `x-tenant-id` |
| Kafka 事件 | `iot.events` 以 `tenant_id` 为分区 key | 同一租户的事件固定落到同一分区 |
| 规则评估 | 消费组 `iot-rule-rules` | 同租户事件只在一个实例评估，告警去重窗口与场景 latest 不会跨实例分裂；不同租户分摊到不同副本 |
| 数据落库 | 消费组 `iot-data-ingest` | 分区在副本间拆分；TDengine 同 ts 覆盖写入幂等兜底 |
| 设备指令消费（access） | 消费组 `iot-commands`（`auto_commit=true`，约 5s） | 指令按分区只由一个副本消费，不重复下发；重启从最近提交位点续传，最后一批可能重复 |
| AI 异常检测 | 消费组 `iot-anomaly` | 同租户样本只进一个实例的 Welford 基线；rebalance 后进程内基线重置并重新学习（与单实例重启一致） |
| 消费位点 | Kafka `auto_commit=true`（约 5s） | 重启从最近提交位点继续，不跳过停机窗口；最后一批可能重复消费 |
| 告警 WS 推送 | 内存 broadcast + Redis pub/sub（`iot:alerts`） | 任意实例的订阅者全量可达，无回声重复 |
| WS 连接上限 | 每副本进程内 Semaphore（`WS_MAX_CONNECTIONS`，默认 1024） | 总上限=副本数×单副本上限；nginx/ingress 另按 IP 限流 |
| 设备影子 | Redis / MySQL | 共享，无本地状态 |
| 离线巡检（access） | `ecat-scheduler` + Redis 锁 | 同一时刻只有一个实例执行；持锁实例退出后由其他副本接管 |
| 日报（rule） | MySQL `daily_reports` 唯一键 `(tenant_id, report_date)` | 多副本重复生成会命中唯一键，结果幂等 |
| 数据保留清理（data） | 按时间条件删除 | 删除条件幂等，重复执行无额外副作用 |
| 审计日志 | MySQL `audit_log`（actor + source IP） | 共享；保留期由 `AUDIT_RETENTION_DAYS`（默认 90）控制 |
| 主键生成 | snowflake worker 号取 pod HOSTNAME 哈希（0-1023） | 显式 `SNOWFLAKE_WORKER_ID` 优先；不同 hostname 同余 1024 的碰撞概率极低 |
| 网关服务发现 | `ServiceResolver` 缓存，TTL `SERVICE_DISCOVERY_TTL_SECS`（默认 30s） | 实例下线后最多 TTL 内仍可能被路由到；跨实例调用需能容忍短暂失败 |

## 部署要求

1. **共享配置**：所有实例使用同一组密钥——`JWT_SECRET`、`IOT_GATEWAY_SECRET`、
   `IOT_PASSWORD_PEPPER`、`IOT_CRED_ENCRYPT_KEY`、`EMQX_AUTH_SECRET`；MQTT 账号
   `MQTT_USERNAME` / `MQTT_PASSWORD` 也必须一致。
2. **基础设施共享**：MySQL / Redis / Kafka / EMQX 不能随副本私有；Redis 不可用时
   限流与告警广播降级为单机模式（fail-open，日志告警）。
3. **Kafka 分区数**：`iot.events` 分区数应 ≥ 消费副本数；按租户分区意味着单个热点租户
   的吞吐上限是一个分区（无法靠加副本提速），按租户量级预留分区。
4. **MQTT client id**：iot-access 每个副本必须设置唯一的 `MQTT_CLIENT_ID`（如
   `ecat-mqtt-1`、`ecat-mqtt-2`），否则同 client id 会被 EMQX 互相踢下线。v1.23.0 起
   access 发布端使用 clean_session=false 持久会话，client id 仍须逐副本唯一；离线指令
   补发的去重 guard 是进程内的，多副本可能重复补发（at-least-once，设备侧应幂等）。
5. **本地落盘**：iot-device 的 `FIRMWARE_DIR` 与 iot-access 的 `IOT_CRED_KEYS_FILE`
   需要共享存储或粘性路由；当前代码未内置共享存储。
6. **扩容即改副本数**：Deployment `replicas` 即可（K8s 清单见 `deploy/k8s/`），
   无需改业务代码；`SNOWFLAKE_WORKER_ID` 显式设置时须各副本唯一。
7. **系统时钟**：snowflake 在时钟大步回拨（>10ms）时会 fail-loud panic，运维调时请用
   `slew`/`adjtime` 渐进校准，进程重启即恢复。
8. **WS 连接上限**：每副本按 `WS_MAX_CONNECTIONS` 独立计数，扩副本会线性放大总上限；
   在 ingress/nginx 层再按 IP 限流，避免单账号耗尽。

## 副本数建议

- 网关（8080）：HTTP 无状态，可按流量水平扩。
- iot-rule（8084）：按租户分摊消费分区；单租户吞吐受单分区限制。
- iot-data（8083）：落库与异常检测是两个消费组，扩副本提升跨租户吞吐。
- iot-access（8082）：HTTP 部分可扩；MQTT 客户端需按第 4 条设置唯一 client id。
- iot-device / iot-cdn：HTTP 无状态，按流量扩；device 的本地固件目录按第 5 条处理。

## 验证方法

```bash
# 双副本规则服务 + 两个 WS 客户端连不同副本，制造同租户告警
# 预期：两端都收到（Redis 广播），且去重窗口内各只收到一次
```
