#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HOME/.cargo/env"
exec "$SCRIPT_DIR/build-web.sh" --release
