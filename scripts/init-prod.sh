#!/usr/bin/env bash
# 单机生产 compose 初始化（幂等）：
#   - deploy/compose/.env：不存在时生成全部账户/口令/密钥；已存在则不改
#   - --regenerate：先备份旧 .env 到 deploy/compose/backups/.env.<时间戳>，再轮换应用密钥
#   - 内部 CA + EMQX 服务端证书（data/certs）；数据目录与属主；域名证书检查
#
# 用法：
#   sudo scripts/init-prod.sh                 # 首次生成
#   sudo scripts/init-prod.sh --regenerate    # 轮换应用密钥（保留旧值备份）
#   SKIP_CERT_CHECK=1 scripts/init-prod.sh    # 仅用于 config 校验/无域名证书环境
#
# 说明：
#   - 基础设施口令（MySQL/Redis/MinIO/TDengine/Grafana/EMQX dashboard）只在首次生成；
#     --regenerate 不修改它们（改了会连不上已落盘的数据），备份旧 .env 并打印人工轮换提示。
#   - 内部 CA 轮换会使已信任旧 CA 的设备/TLS 客户端失败，--regenerate 会备份 .old 并提示。
#   - 旧文件永不删除：.env 备份、证书 .old 均保留在 deploy/compose/ 下（已 gitignore）。

set -euo pipefail
umask 077

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_DIR="$ROOT/deploy/compose"
ENV_FILE="$COMPOSE_DIR/.env"
DATA_DIR="$COMPOSE_DIR/data"
CERT_DIR="$DATA_DIR/certs"
BACKUP_DIR="$COMPOSE_DIR/backups"
DOMAIN_CERT_DIR="$ROOT/deploy/certs"
SKIP_CERT_CHECK="${SKIP_CERT_CHECK:-0}"

log()  { printf '\033[1;32m[init-prod]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[init-prod]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[init-prod]\033[0m %s\n' "$*" >&2; exit 1; }

usage() {
  sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

REGEN=0
for arg in "$@"; do
  case "$arg" in
    --regenerate|-r) REGEN=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "未知参数：$arg（支持 --regenerate / --help）" ;;
  esac
done

command -v openssl >/dev/null 2>&1 || die "需要 openssl（生成密钥与证书）"

gen_secret()  { openssl rand -hex 32; }
gen_pass()    { openssl rand -hex 24; }
now_ts()      { date +%Y%m%d-%H%M%S; }

# 读取现有 .env 的值（只解析 KEY=VALUE，不 source，避免执行任意内容）。
get_var() {
  if [ -f "$ENV_FILE" ]; then
    sed -n "s/^$1=//p" "$ENV_FILE" | tail -n 1
  fi
}

# ---------- 1. 备份旧 .env（仅 --regenerate） ----------
if [ -f "$ENV_FILE" ] && [ "$REGEN" -eq 1 ]; then
  mkdir -p "$BACKUP_DIR"
  chmod 700 "$BACKUP_DIR"
  backup="$BACKUP_DIR/.env.$(now_ts)"
  n=1
  while [ -e "$backup" ]; do
    backup="$BACKUP_DIR/.env.$(now_ts)-$n"
    n=$((n + 1))
  done
  cp -p "$ENV_FILE" "$backup"
  chmod 600 "$backup"
  log "旧 .env 已备份：$backup"
fi

# ---------- 2. 计算值：应用密钥可轮换，基础设施口令只首次生成 ----------
# DOMAIN 可用环境变量覆盖（首次生成或修改域名时），否则沿用 .env，最后落到示例值。
DOMAIN_FILE="$(get_var DOMAIN)"
DOMAIN="${DOMAIN:-${DOMAIN_FILE:-iot.example.com}}"
PLATFORM_TENANT="$(get_var PLATFORM_TENANT)"; PLATFORM_TENANT="${PLATFORM_TENANT:-tenant-1}"
PUID="$(get_var PUID)";                     PUID="${PUID:-1000}"
PGID="$(get_var PGID)";                     PGID="${PGID:-1000}"

rotate_or_keep() {  # 应用密钥：首次为空或 --regenerate 时轮换
  local old_value="$1"
  if [ "$REGEN" -eq 1 ] || [ -z "$old_value" ]; then gen_secret; else printf '%s' "$old_value"; fi
}
keep_or_create_pass() {  # 基础设施口令：只首次生成
  local old_value="$1"
  if [ -z "$old_value" ]; then gen_pass; else printf '%s' "$old_value"; fi
}

OLD_ENCRYPT_KEY="$(get_var IOT_CRED_ENCRYPT_KEY)"
OLD_ENCRYPT_KEY_OLD="$(get_var IOT_CRED_ENCRYPT_KEY_OLD)"

JWT_SECRET="$(rotate_or_keep "$(get_var JWT_SECRET)")"
IOT_GATEWAY_SECRET="$(rotate_or_keep "$(get_var IOT_GATEWAY_SECRET)")"
IOT_CRED_ENCRYPT_KEY="$(rotate_or_keep "$OLD_ENCRYPT_KEY")"
IOT_PASSWORD_PEPPER="$(rotate_or_keep "$(get_var IOT_PASSWORD_PEPPER)")"
ADMIN_INITIAL_PASSWORD="$(rotate_or_keep "$(get_var ADMIN_INITIAL_PASSWORD)")"
EMQX_AUTH_SECRET="$(rotate_or_keep "$(get_var EMQX_AUTH_SECRET)")"
MQTT_PASSWORD="$(rotate_or_keep "$(get_var MQTT_PASSWORD)")"
MQTT_USERNAME="$(get_var MQTT_USERNAME)"; MQTT_USERNAME="${MQTT_USERNAME:-ecat-access}"

# 只保留上一把旧密钥：--regenerate 时把当前值移入 _OLD，历史轮换值留在 .env 备份里。
if [ "$REGEN" -eq 1 ] && [ -n "$OLD_ENCRYPT_KEY" ]; then
  IOT_CRED_ENCRYPT_KEY_OLD="$OLD_ENCRYPT_KEY"
else
  IOT_CRED_ENCRYPT_KEY_OLD="$OLD_ENCRYPT_KEY_OLD"
fi

MYSQL_ROOT_PASSWORD="$(keep_or_create_pass "$(get_var MYSQL_ROOT_PASSWORD)")"
MYSQL_PASSWORD="$(keep_or_create_pass "$(get_var MYSQL_PASSWORD)")"
REDIS_PASSWORD="$(keep_or_create_pass "$(get_var REDIS_PASSWORD)")"
MINIO_ROOT_USER="$(get_var MINIO_ROOT_USER)"; MINIO_ROOT_USER="${MINIO_ROOT_USER:-iot$(openssl rand -hex 4)}"
MINIO_ROOT_PASSWORD="$(keep_or_create_pass "$(get_var MINIO_ROOT_PASSWORD)")"
TDENGINE_PASS="$(keep_or_create_pass "$(get_var TDENGINE_PASS)")"
GRAFANA_ADMIN_PASSWORD="$(keep_or_create_pass "$(get_var GRAFANA_ADMIN_PASSWORD)")"
EMQX_DASHBOARD_PASSWORD="$(keep_or_create_pass "$(get_var EMQX_DASHBOARD_PASSWORD)")"

# 地址随 DOMAIN 变化：旧的派生值在域名变更时重算，显式自定义值保持不变。
old_callback="$(get_var ACCESS_CALLBACK_BASE)"
# v1.19 起公开 API 只保留 /api/v1：旧派生路径就地升级，自定义地址保留。
if [ -z "$old_callback" ]; then
  ACCESS_CALLBACK_BASE="https://$DOMAIN/api/v1/access/oauth/callback"
elif [ "${old_callback%/api/access/oauth/callback}" != "$old_callback" ]; then
  ACCESS_CALLBACK_BASE="${old_callback%/api/access/oauth/callback}/api/v1/access/oauth/callback"
elif [ -n "$DOMAIN_FILE" ] && [ "$DOMAIN" != "$DOMAIN_FILE" ] \
     && [ "$old_callback" = "https://$DOMAIN_FILE/api/v1/access/oauth/callback" ]; then
  ACCESS_CALLBACK_BASE="https://$DOMAIN/api/v1/access/oauth/callback"
else
  ACCESS_CALLBACK_BASE="$old_callback"
fi
old_ota_base="$(get_var OTA_PUBLIC_BASE)"
if [ -z "$old_ota_base" ] \
   || { [ -n "$DOMAIN_FILE" ] && [ "$DOMAIN" != "$DOMAIN_FILE" ] \
        && [ "$old_ota_base" = "https://$DOMAIN_FILE" ]; }; then
  OTA_PUBLIC_BASE="https://$DOMAIN"
else
  OTA_PUBLIC_BASE="$old_ota_base"
fi
OTA_S3_ENDPOINT="$(get_var OTA_S3_ENDPOINT)"; OTA_S3_ENDPOINT="${OTA_S3_ENDPOINT:-http://minio:9000}"
OTA_S3_REGION="$(get_var OTA_S3_REGION)";     OTA_S3_REGION="${OTA_S3_REGION:-us-east-1}"
OTA_S3_BUCKET="$(get_var OTA_S3_BUCKET)";     OTA_S3_BUCKET="${OTA_S3_BUCKET:-iot-ota}"
OTA_S3_ACCESS_KEY="$(get_var OTA_S3_ACCESS_KEY)"; OTA_S3_ACCESS_KEY="${OTA_S3_ACCESS_KEY:-$MINIO_ROOT_USER}"
OTA_S3_SECRET_KEY="$(get_var OTA_S3_SECRET_KEY)"; OTA_S3_SECRET_KEY="${OTA_S3_SECRET_KEY:-$MINIO_ROOT_PASSWORD}"

# ---------- 3. 写入 .env（不存在或 --regenerate 时） ----------
FIRST_RUN=0
if [ ! -f "$ENV_FILE" ]; then FIRST_RUN=1; fi
enc_old_line=""
[ -n "$IOT_CRED_ENCRYPT_KEY_OLD" ] && enc_old_line="IOT_CRED_ENCRYPT_KEY_OLD=$IOT_CRED_ENCRYPT_KEY_OLD"
if [ ! -f "$ENV_FILE" ] || [ "$REGEN" -eq 1 ]; then
  tmp="$ENV_FILE.tmp.$$"
  trap 'rm -f "$tmp"' EXIT
  cat >"$tmp" <<EOF
# 由 scripts/init-prod.sh 生成；勿提交（.gitignore 已忽略）。
# 应用密钥可用 scripts/init-prod.sh --regenerate 轮换；
# 基础设施口令只首次生成，--regenerate 不改动（需人工在数据库中轮换）。
DOMAIN=$DOMAIN
PLATFORM_TENANT=$PLATFORM_TENANT
PUID=$PUID
PGID=$PGID

JWT_SECRET=$JWT_SECRET
IOT_GATEWAY_SECRET=$IOT_GATEWAY_SECRET
IOT_CRED_ENCRYPT_KEY=$IOT_CRED_ENCRYPT_KEY
$enc_old_line
IOT_PASSWORD_PEPPER=$IOT_PASSWORD_PEPPER
ADMIN_INITIAL_PASSWORD=$ADMIN_INITIAL_PASSWORD
EMQX_AUTH_SECRET=$EMQX_AUTH_SECRET
MQTT_USERNAME=$MQTT_USERNAME
MQTT_PASSWORD=$MQTT_PASSWORD

MYSQL_ROOT_PASSWORD=$MYSQL_ROOT_PASSWORD
MYSQL_PASSWORD=$MYSQL_PASSWORD
REDIS_PASSWORD=$REDIS_PASSWORD
MINIO_ROOT_USER=$MINIO_ROOT_USER
MINIO_ROOT_PASSWORD=$MINIO_ROOT_PASSWORD
TDENGINE_PASS=$TDENGINE_PASS
GRAFANA_ADMIN_PASSWORD=$GRAFANA_ADMIN_PASSWORD
EMQX_DASHBOARD_PASSWORD=$EMQX_DASHBOARD_PASSWORD

ACCESS_CALLBACK_BASE=$ACCESS_CALLBACK_BASE
OTA_PUBLIC_BASE=$OTA_PUBLIC_BASE
OTA_S3_ENDPOINT=$OTA_S3_ENDPOINT
OTA_S3_REGION=$OTA_S3_REGION
OTA_S3_BUCKET=$OTA_S3_BUCKET
OTA_S3_ACCESS_KEY=$OTA_S3_ACCESS_KEY
OTA_S3_SECRET_KEY=$OTA_S3_SECRET_KEY
EOF
  chmod 600 "$tmp"
  mv -f "$tmp" "$ENV_FILE"
  trap - EXIT
  log "已写入 $ENV_FILE（权限 600）"
else
  log ".env 已存在，保持不变（如需轮换应用密钥：--regenerate）"
  missing=()
  for k in JWT_SECRET IOT_GATEWAY_SECRET IOT_CRED_ENCRYPT_KEY IOT_PASSWORD_PEPPER \
           ADMIN_INITIAL_PASSWORD EMQX_AUTH_SECRET MQTT_PASSWORD MYSQL_ROOT_PASSWORD \
           MYSQL_PASSWORD REDIS_PASSWORD MINIO_ROOT_USER MINIO_ROOT_PASSWORD \
           TDENGINE_PASS GRAFANA_ADMIN_PASSWORD EMQX_DASHBOARD_PASSWORD OTA_S3_BUCKET; do
    [ -n "$(get_var "$k")" ] || missing+=("$k")
  done
  if [ "${#missing[@]}" -gt 0 ]; then
    warn "现有 .env 缺少：${missing[*]}（不会自动修改；请补全，或用 --regenerate 重新生成）"
  fi
fi

# ---------- 4. 数据目录与属主 ----------
mkdir -p "$DATA_DIR"/{mysql,redis,kafka,minio,tdengine,tdengine-log,opensearch,opensearch-logs,emqx,emqx-log,firmware,keys,prometheus,grafana,certs} \
         "$BACKUP_DIR" "$DOMAIN_CERT_DIR"
chmod 700 "$DATA_DIR" "$BACKUP_DIR" "$DOMAIN_CERT_DIR" "$CERT_DIR"
find "$DATA_DIR" -mindepth 1 -type d -exec chmod 700 {} +

IS_ROOT=0
[ "$(id -u)" -eq 0 ] && IS_ROOT=1
own() {
  local uid="$1" gid="$2" path="$3"
  if [ "$IS_ROOT" -eq 1 ]; then
    chown -R "$uid:$gid" "$path"
  fi
}
# 各镜像默认运行用户（见各自官方镜像）：
own 1000 1000 "$DATA_DIR/minio"                      # minio
own 1001 1001 "$DATA_DIR/kafka"                      # bitnami/kafka
own 1000 1000 "$DATA_DIR/opensearch" "$DATA_DIR/opensearch-logs"   # opensearch
own 1000 1000 "$DATA_DIR/emqx" "$DATA_DIR/emqx-log" "$CERT_DIR"    # emqx
own 65534 65534 "$DATA_DIR/prometheus"               # prometheus (nobody)
own 472 472 "$DATA_DIR/grafana"                      # grafana
own "$PUID" "$PGID" "$DATA_DIR/firmware" "$DATA_DIR/keys"          # 业务容器（PUID/PGID）
# mysql/redis/tdengine 入口脚本以 root 启动并自行 chown 数据卷，不在此处改属主。

# ---------- 5. 内部 CA + EMQX 服务端证书 ----------
gen_certs=0
if [ "$REGEN" -eq 1 ] || [ ! -f "$CERT_DIR/ca.pem" ] || [ ! -f "$CERT_DIR/ca.key" ] \
   || [ ! -f "$CERT_DIR/emqx.pem" ] || [ ! -f "$CERT_DIR/emqx.key" ]; then
  gen_certs=1
fi

if [ "$gen_certs" -eq 1 ]; then
  if [ "$REGEN" -eq 1 ]; then
    for f in ca.pem ca.key emqx.pem emqx.key; do
      [ -f "$CERT_DIR/$f" ] && cp -p "$CERT_DIR/$f" "$CERT_DIR/$f.old"
    done
    [ -f "$CERT_DIR/ca.pem.old" ] && log "旧 CA/EMQX 证书已备份为 $CERT_DIR/*.old"
  fi
  san="DNS:emqx,DNS:localhost,IP:127.0.0.1,DNS:$DOMAIN"
  ext="$(mktemp)"
  printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' "$san" >"$ext"

  log "生成内部 CA 与 EMQX 证书（SAN: $san）"
  openssl req -x509 -newkey rsa:4096 -sha256 -days 3650 -nodes \
    -keyout "$CERT_DIR/ca.key" -out "$CERT_DIR/ca.pem" \
    -subj "/CN=IoT Platform Internal CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" >/dev/null 2>&1
  openssl req -new -newkey rsa:2048 -nodes \
    -keyout "$CERT_DIR/emqx.key" -out "$CERT_DIR/emqx.csr" \
    -subj "/CN=emqx" >/dev/null 2>&1
  openssl x509 -req -in "$CERT_DIR/emqx.csr" \
    -CA "$CERT_DIR/ca.pem" -CAkey "$CERT_DIR/ca.key" -CAcreateserial \
    -out "$CERT_DIR/emqx.pem" -days 825 -sha256 -extfile "$ext" >/dev/null 2>&1
  rm -f "$ext" "$CERT_DIR/emqx.csr" "$CERT_DIR/ca.srl"

  chmod 600 "$CERT_DIR/ca.key" "$CERT_DIR/emqx.key"
  chmod 644 "$CERT_DIR/ca.pem" "$CERT_DIR/emqx.pem"
  own 1000 1000 "$CERT_DIR"
  log "证书：$CERT_DIR/{ca.pem,emqx.pem,emqx.key}（ca.key 不上容器）"
else
  log "内部证书已存在，保持不变（--regenerate 可轮换）"
fi
if [ -n "$DOMAIN_FILE" ] && [ "$DOMAIN" != "$DOMAIN_FILE" ] && [ "$gen_certs" -eq 0 ]; then
  warn "DOMAIN 已从 $DOMAIN_FILE 改为 $DOMAIN，但内部证书 SAN 未更新；请运行 --regenerate"
fi

# ---------- 6. 域名证书检查（唯一需要人工提供） ----------
if [ "$SKIP_CERT_CHECK" = "1" ]; then
  warn "SKIP_CERT_CHECK=1：跳过域名证书检查（仅用于 config 校验；正式启动前必须提供）"
elif [ ! -f "$DOMAIN_CERT_DIR/fullchain.pem" ] || [ ! -f "$DOMAIN_CERT_DIR/privkey.pem" ]; then
  die "缺少域名证书：请放置 $DOMAIN_CERT_DIR/fullchain.pem 与 $DOMAIN_CERT_DIR/privkey.pem
  （用现有 ACME 流程签发/续期后复制过来；仅校验配置时设 SKIP_CERT_CHECK=1）"
else
  chmod 600 "$DOMAIN_CERT_DIR/privkey.pem"
  chmod 644 "$DOMAIN_CERT_DIR/fullchain.pem"
  log "域名证书：$DOMAIN_CERT_DIR/{fullchain.pem,privkey.pem}"
fi

# ---------- 7. 摘要 ----------
if [ "$IS_ROOT" -eq 0 ]; then
  warn "当前非 root：已跳过数据目录 chown。若容器用户无法写入 data/，请用 sudo 重跑本脚本。"
fi

echo
echo "=============================================================="
echo "  单机生产 compose 初始化完成"
echo "=============================================================="
echo "  配置：$ENV_FILE（600）"
echo "  数据：$DATA_DIR（bind mount，已 gitignore）"
echo "  备份：$BACKUP_DIR"
if [ "$FIRST_RUN" -eq 1 ]; then
  echo
  echo "  首次管理员：admin（租户 $PLATFORM_TENANT），初始口令："
  echo "    $ADMIN_INITIAL_PASSWORD"
  echo "    —— 已写入 .env 的 ADMIN_INITIAL_PASSWORD，请妥善保管"
fi
if [ "$REGEN" -eq 1 ]; then
  echo
  echo "  已轮换：JWT_SECRET / IOT_GATEWAY_SECRET / IOT_CRED_ENCRYPT_KEY(+_OLD)"
  echo "          IOT_PASSWORD_PEPPER / ADMIN_INITIAL_PASSWORD / EMQX_AUTH_SECRET / MQTT_PASSWORD"
  echo "  生效：docker compose -f deploy/compose/docker-compose.yml up -d --force-recreate"
  echo "  注意：JWT 轮换后所有用户需重新登录；MQTT_PASSWORD 变更需更新平台侧 MQTT 客户端；"
  echo "        ADMIN_INITIAL_PASSWORD 仅在重新播种 admin 时生效。"
  echo "  未自动轮换（需人工在数据库/服务内改）：MySQL / Redis / MinIO / TDengine /"
  echo "        Grafana / EMQX dashboard 口令（旧值见备份 .env）。"
fi
if [ "$DOMAIN" = "iot.example.com" ]; then
  warn "DOMAIN 仍是示例值 iot.example.com。请在 .env 修改为真实域名后运行 --regenerate"
  warn "（或首次生成时使用：DOMAIN=<你的域名> scripts/init-prod.sh）。"
fi
echo
echo "  启动：docker compose -f deploy/compose/docker-compose.yml up -d --build"
echo "  监控：docker compose -f deploy/compose/docker-compose.yml --profile monitoring up -d"
echo "=============================================================="
