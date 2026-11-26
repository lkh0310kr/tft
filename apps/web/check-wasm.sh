#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
"$SCRIPT_DIR/apply-gpui-web-patch.sh"
cd "$ROOT"
export RUSTC_BOOTSTRAP=1
exec cargo check --manifest-path "$SCRIPT_DIR/Cargo.toml" --target wasm32-unknown-unknown "$@"
