#!/usr/bin/env bash
# 同步 site/ 到静态站点服务器（rsync over SSH）。
# 用法：
#   SITE_USER=... SITE_PATH=/path/to/site SITE_HOST=package.erik.xyz \
#     [SITE_PORT=22000] [SITE_SSH_KEY=/path/to/key] scripts/deploy-site.sh
# 默认不执行任何部署命令；调用前请确认目标路径，避免 --delete 误删。
set -euo pipefail

host="${SITE_HOST:-package.erik.xyz}"
port="${SITE_PORT:-22000}"
user="${SITE_USER:?SITE_USER is required}"
path="${SITE_PATH:?SITE_PATH is required (absolute target directory)}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source_dir="$root/site"

[[ -d "$source_dir" ]] || { echo "site directory not found: $source_dir" >&2; exit 1; }
command -v rsync >/dev/null || { echo "rsync is required" >&2; exit 1; }

ssh_args=(-p "$port")
if [[ -n "${SITE_SSH_KEY:-}" ]]; then
  ssh_args+=(-i "$SITE_SSH_KEY")
fi

# --delete makes the target mirror the tracked source; confirm the path first.
rsync -az --delete --chmod=D755,F644 \
  -e "ssh ${ssh_args[*]}" \
  "$source_dir/" "$user@$host:$path/"
echo "deployed site/ to $user@$host:$path"
