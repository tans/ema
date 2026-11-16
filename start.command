#!/bin/bash
# macOS Finder 双击入口，所有启动逻辑统一在 start.sh。
SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec /bin/bash "$SCRIPT_DIR/start.sh"
