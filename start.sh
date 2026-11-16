#!/bin/bash
# EMA 本地原生开发启动入口。
set -euo pipefail

PROJECT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$PROJECT_DIR"

# Finder 启动时补齐常见工具路径，不依赖当前终端目录。
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$HOME/.volta/bin:$HOME/.bun/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"

on_exit() {
  local result=$?
  if [ "$result" -ne 0 ]; then
    printf '\nEMA 启动失败（退出码 %s），请查看上方错误。\n' "$result"
    if [ -t 0 ]; then
      read -r -p '按回车关闭窗口……' _ || true
    fi
  fi
}
trap on_exit EXIT

for tool in node npm cargo; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf '缺少运行工具：%s。请安装 Node.js 和 Rust 后重试。\n' "$tool" >&2
    exit 1
  fi
done

if [ ! -x node_modules/.bin/tauri ] || [ ! -x node_modules/.bin/vite ]; then
  printf '首次运行，正在安装前端依赖……\n'
  npm ci
fi

printf '正在启动 EMA 原生窗口，首次编译可能需要几分钟。\n'
printf '保持此窗口打开；按 Ctrl+C 停止本地开发服务。\n\n'
npm run tauri dev
