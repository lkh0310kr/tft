#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFEST="$SCRIPT_DIR/Cargo.toml"
REV="4ce18741ad2ea007cb20a1dd96d2a170d224cc01"
PATCH_FILE="$SCRIPT_DIR/patches/gpui-app-run-wasm-lifetime.patch"

METADATA="$(cargo metadata --format-version 1 --manifest-path "$MANIFEST")"
GPUI_MANIFEST="$(python3 -c 'import json,sys; data=json.load(sys.stdin); packages=[p for p in data["packages"] if p["name"]=="gpui" and p.get("source", "").find(sys.argv[1])>=0]; assert len(packages)==1, f"expected exactly one gpui package at revision {sys.argv[1]}"; print(packages[0]["manifest_path"])' "$REV" <<<"$METADATA")"
GPUI_ROOT="$(dirname "$GPUI_MANIFEST")/../.."
GPUI_ROOT="$(cd "$GPUI_ROOT" && pwd)"
APP_RS="$GPUI_ROOT/crates/gpui/src/app.rs"

if git -C "$GPUI_ROOT" apply --reverse --check "$PATCH_FILE" >/dev/null 2>&1; then
  printf 'GPUI Web lifetime patch already applied at %s\n' "$GPUI_ROOT"
else
  git -C "$GPUI_ROOT" apply --check "$PATCH_FILE"
  git -C "$GPUI_ROOT" apply "$PATCH_FILE"
  printf 'Applied GPUI Web lifetime patch to %s\n' "$GPUI_ROOT"
fi

if ! git -C "$GPUI_ROOT" apply --reverse --check "$PATCH_FILE" >/dev/null 2>&1; then
  printf 'GPUI checkout does not match the expected patched revision: %s\n' "$GPUI_ROOT" >&2
  exit 1
fi
