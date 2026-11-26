#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRUNK_BIN="${TRUNK_BIN:-$(command -v trunk || true)}"
if [[ -z "$TRUNK_BIN" ]]; then
  echo "trunk is required (set TRUNK_BIN=/path/to/trunk)" >&2
  exit 127
fi

"$SCRIPT_DIR/apply-gpui-web-patch.sh"
export RUSTC_BOOTSTRAP=1
exec "$TRUNK_BIN" build "$@"
