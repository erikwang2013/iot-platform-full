#!/usr/bin/env bash
# 轻量压测：网关 /health 与 /api/v1/devices（需 JWT）。
# 用法：
#   ./scripts/loadtest.sh                          # /health；.env 有 ADMIN_INITIAL_PASSWORD 时自动登录并压 /api/v1/devices
#   TOKEN=<jwt> ./scripts/loadtest.sh              # /health + /api/v1/devices
#   ./scripts/loadtest.sh -n 1000 -c 10 -u http://localhost:8080
# 参数：-n 请求数(默认 500) -c 并发(默认 10) -u 网关地址(默认 http://localhost:8080)
# 优先用 ab（ApacheBench）；无 ab 时退化为 curl 并发循环。
set -euo pipefail

GATEWAY="${GATEWAY:-http://localhost:8080}"
N=500
C=10
TOKEN="${TOKEN:-}"

while getopts "n:c:u:" opt; do
  case $opt in
    n) N=$OPTARG ;;
    c) C=$OPTARG ;;
    u) GATEWAY=$OPTARG ;;
    *) exit 1 ;;
  esac
done

# 初始管理员口令不再固定：优先环境变量，其次项目根 .env（scripts/install.sh 生成）
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADMIN_USERNAME="${ADMIN_USERNAME:-admin}"
if [ -z "${ADMIN_INITIAL_PASSWORD:-}" ] && [ -f "$ROOT/.env" ]; then
  ADMIN_INITIAL_PASSWORD=$(sed -n 's/^ADMIN_INITIAL_PASSWORD=//p' "$ROOT/.env" | head -1)
fi
if [ -z "$TOKEN" ] && [ -n "${ADMIN_INITIAL_PASSWORD:-}" ]; then
  TOKEN=$(curl -s -X POST "$GATEWAY/api/v1/auth/login" -H 'Content-Type: application/json' \
    -d "{\"username\":\"$ADMIN_USERNAME\",\"password\":\"$ADMIN_INITIAL_PASSWORD\"}" \
    | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
  if [ -n "$TOKEN" ]; then
    echo "已用 $ADMIN_USERNAME 登录获取 TOKEN"
  else
    echo "WARN: 使用 ADMIN_INITIAL_PASSWORD 登录失败，跳过 /api/v1/devices 压测" >&2
  fi
fi

ab_run() {
  local url="$1" label="$2"
  local args=(-n "$N" -c "$C" -k)
  [ -n "$TOKEN" ] && args+=(-H "Authorization: Bearer $TOKEN")
  echo "== $label: $url"
  ab "${args[@]}" "$url" 2>&1 | grep -E "Requests per second|Failed requests|Time per request|Complete requests" || {
    echo "ab 失败（地址不可达或 ab 缺失），退出码 $?"; return 1
  }
}

curl_run() {
  local url="$1" label="$2"
  echo "== $label: $url (curl 并发 $C × $N)"
  local ok=0 fail=0
  local total_ns=0
  for i in $(seq "$N"); do
    local start
    start=$(date +%s%N)
    if [ -n "$TOKEN" ]; then
      curl -s -o /dev/null -H "Authorization: Bearer $TOKEN" "$url" && ok=$((ok+1)) || fail=$((fail+1))
    else
      curl -s -o /dev/null "$url" && ok=$((ok+1)) || fail=$((fail+1))
    fi
    total_ns=$((total_ns + $(date +%s%N) - start))
  done
  echo "成功 $ok 失败 $fail 平均耗时 $((total_ns / N / 1000000))ms"
}

main() {
  if command -v ab >/dev/null 2>&1; then
    ab_run "$GATEWAY/health" "健康检查" || true
  else
    echo "未找到 ab（ApacheBench），使用 curl 循环（串行，吞吐参考意义有限）"
    curl_run "$GATEWAY/health" "健康检查"
  fi
  if [ -n "$TOKEN" ]; then
    if command -v ab >/dev/null 2>&1; then
      ab_run "$GATEWAY/api/v1/devices" "设备列表(带JWT)" || true
    else
      curl_run "$GATEWAY/api/v1/devices" "设备列表(带JWT)"
    fi
  else
    echo "== 跳过 /api/v1/devices：设置 TOKEN=<jwt> 后测试受保护端点"
    echo "   或 export ADMIN_INITIAL_PASSWORD=<初始管理员口令>（scripts/install.sh 已写入 .env，本脚本会自动读取）"
  fi
}

main
