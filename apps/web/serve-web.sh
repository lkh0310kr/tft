#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PORT="${1:-8088}"
shift || true
"$SCRIPT_DIR/build-web.sh" "$@"
exec python3 -m http.server "$PORT" --directory "$SCRIPT_DIR/dist"
