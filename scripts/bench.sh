#!/usr/bin/env bash
# HTTP 压测：网关 / 接入 / 数据服务健康检查（roadmap P6.2）。
#
# 前置条件（三选一）：
#   docker compose --profile app up -d
#   或分别启动：cargo run -p ecat-gateway / ecat-access / ecat-data-service
#   或设置 GATEWAY_URL / ACCESS_URL / DATA_URL 指向已部署实例。
#
# 环境变量：
#   N=1000 C=10 W=100   请求数 / 并发数 / 预热请求数
#   TOKEN=<jwt>         可选，设置后附加 Authorization: Bearer
#   GATEWAY_URL / ACCESS_URL / DATA_URL  目标地址（默认见下）
#
# 这是冒烟/负载工具，不是 CI 性能门禁：共享 CI 机器上的数值不可比。
# 退出码：0 全部目标无失败请求；1 任一目标有失败请求（服务不可用/非 2xx）；2 参数错误。
set -euo pipefail

N="${N:-1000}"
C="${C:-10}"
W="${W:-100}"
GATEWAY_URL="${GATEWAY_URL:-http://localhost:8080/health}"
ACCESS_URL="${ACCESS_URL:-http://localhost:8082/health}"
DATA_URL="${DATA_URL:-http://localhost:8083/health}"
TOKEN="${TOKEN:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

args=(
  --url "$GATEWAY_URL"
  --url "$ACCESS_URL"
  --url "$DATA_URL"
  --concurrency "$C"
  --requests "$N"
  --warmup "$W"
)
if [ -n "$TOKEN" ]; then
  args+=(--header "Authorization: Bearer $TOKEN")
fi

cargo run --release --manifest-path "$ROOT/e-cat/Cargo.toml" -p ecat-bench --bin bench -- "${args[@]}"
