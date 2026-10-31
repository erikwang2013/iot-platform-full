#!/usr/bin/env bash
# 一键安装：拉起基础设施 → 构建业务服务 → 生成 .env → 打印启动指引
# 幂等：可重复运行
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="$ROOT/scripts/bin"
BINARIES=(iot-gateway iot-device iot-access iot-rule iot-data iot-cdn)

log()  { printf '\033[1;32m[install]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[install]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[install]\033[0m %s\n' "$*" >&2; exit 1; }

# 32 字节强随机值（64 位十六进制）。openssl 缺失时退到 /dev/urandom；
# 两者都没有的极端环境用 sha256($RANDOM+时间) 兜底（熵较低，仅本机开发）。
gen_secret() {
  if command -v openssl >/dev/null 2>&1 && openssl rand -hex 32 2>/dev/null; then
    return 0
  fi
  if [ -r /dev/urandom ]; then
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
    return 0
  fi
  printf '%s-%s-%s' "$RANDOM" "$(date +%s%N 2>/dev/null || date +%s)" "$RANDOM" \
    | sha256sum | cut -d' ' -f1
}

# --- 1. 基础设施（docker compose up -d） ---
if ! command -v docker >/dev/null 2>&1; then
  die "未检测到 docker。请先安装 Docker 24+：https://docs.docker.com/engine/install/（或 apt install docker.io / dnf install docker）"
fi
COMPOSE=""
if docker compose version >/dev/null 2>&1; then
  COMPOSE="docker compose"
elif command -v docker-compose >/dev/null 2>&1; then
  COMPOSE="docker-compose"
else
  die "未检测到 docker compose 插件。请安装：https://docs.docker.com/compose/install/"
fi
log "使用 $COMPOSE 拉起基础设施（MySQL / Redis / EMQX / Kafka / MinIO / TDengine）"
(cd "$ROOT" && $COMPOSE up -d)

# --- 2. 业务服务构建 ---
if command -v cargo >/dev/null 2>&1; then
  log "检测到 cargo，构建 6 个业务服务（release，首次构建较慢）"
  (cd "$ROOT/e-cat" && cargo build --release \
    --bin iot-gateway --bin iot-device --bin iot-access --bin iot-rule --bin iot-data --bin iot-cdn)
  mkdir -p "$BIN_DIR"
  for b in "${BINARIES[@]}"; do
    if [ -f "$ROOT/e-cat/target/release/$b" ]; then
      cp -f "$ROOT/e-cat/target/release/$b" "$BIN_DIR/$b"
      log "已复制 $b -> scripts/bin/"
    else
      warn "未找到 $b 构建产物，跳过复制"
    fi
  done
else
  warn "未检测到 cargo，跳过构建。安装 Rust 1.80+（https://rustup.rs）后手动执行：cd e-cat && cargo build --release"
fi

# --- 3. .env 示例（不存在才生成） ---
ENV_FILE="$ROOT/.env"
ADMIN_PASSWORD=""
if [ ! -f "$ENV_FILE" ]; then
  log "生成 .env（$ENV_FILE），密钥均为 32 字节随机值"
  ADMIN_PASSWORD="$(gen_secret)"
  cat > "$ENV_FILE" <<EOF
# 基础设施端口（对应 docker-compose.yml 中的 \${VAR:-default} 变量）
MYSQL_PORT=3306
REDIS_PORT=6379
EMQX_MQTT_PORT=1883
KAFKA_PORT=9092
MINIO_API_PORT=9000
MINIO_CONSOLE_PORT=9001
TDENGINE_PORT=6041

# 平台超管所在租户（该租户的 admin 即平台超管）
PLATFORM_TENANT=tenant-1

# 服务密钥：各服务必须使用同一组值（妥善保管；轮换方法见 docs/deploy/production.md）
JWT_SECRET=$(gen_secret)
IOT_GATEWAY_SECRET=$(gen_secret)
IOT_CRED_ENCRYPT_KEY=$(gen_secret)
IOT_PASSWORD_PEPPER=$(gen_secret)
EMQX_AUTH_SECRET=$(gen_secret)

# iot-access 首次启动的管理员口令（仅首次建号时使用，勿泄露）
ADMIN_INITIAL_PASSWORD=$ADMIN_PASSWORD

# 平台侧 MQTT 账号（加固模式用，可访问 iot/#）；设备凭据由管理端逐个签发
MQTT_USERNAME=ecat-access
MQTT_PASSWORD=$(gen_secret)

# 连接串（本地二进制启动用；docker compose 会用服务名覆盖）
DATABASE_URL=mysql://iot:iot@127.0.0.1:3306/iot
REDIS_URL=redis://127.0.0.1:6379
EOF
  chmod 600 "$ENV_FILE"
else
  log ".env 已存在，跳过生成"
fi

# --- 4. 输出摘要 ---
if [ -d "$BIN_DIR" ]; then
  BIN_LIST="$BIN_DIR"
  BIN_HINT="构建完成"
else
  BIN_LIST="$ROOT/e-cat/target/release"
  BIN_HINT="未构建（未检测到 cargo）"
fi

echo
echo "=============================================================="
echo "  安装完成"
echo "=============================================================="
echo "基础设施（docker compose）已启动："
echo "  MySQL 3306 · Redis 6379 · EMQX 1883（控制台 127.0.0.1:18083）"
echo "  Kafka 9092 · MinIO 9000（控制台 9001）· TDengine 6041"
echo
echo "业务服务二进制（$BIN_HINT）：$BIN_LIST"
echo
echo "服务端口表："
echo "  8080  iot-gateway（网关 / 开放 API）"
echo "  8081  iot-device（设备服务）"
echo "  8082  iot-access（接入 / 认证）"
echo "  8083  iot-data（数据服务）"
echo "  8084  iot-rule（规则引擎）"
echo "  8085  iot-cdn（CDN 管理）"
echo
echo "启动服务（每个服务一个终端；命令先加载 .env，或自行接 supervisor/systemd）："
for b in "${BINARIES[@]}"; do
  printf '  set -a; . %q; set +a; %q\n' "$ENV_FILE" "$BIN_LIST/$b"
done
echo
if [ -n "$ADMIN_PASSWORD" ]; then
  echo "首次管理员账号：admin（租户 tenant-1），初始口令："
  echo "  $ADMIN_PASSWORD"
  echo "  —— 已写入 .env 的 ADMIN_INITIAL_PASSWORD（权限 600），请勿泄露"
else
  echo "首次管理员账号：admin（租户 tenant-1），初始口令见 .env 的 ADMIN_INITIAL_PASSWORD"
  echo "  —— 若该值为空，iot-access 首次启动会在日志中随机打印一次"
fi
echo "  —— 默认租户与管理员由 iot-access 首次启动创建；数据库迁移同样在服务启动时自动执行"
echo "配置：.env（$ENV_FILE）"
echo "=============================================================="
