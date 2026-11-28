#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MANIFEST="$SCRIPT_DIR/Cargo.toml"
REV="4ce18741ad2ea007cb20a1dd96d2a170d224cc01"
PATCH_FILE="$SCRIPT_DIR/patches/gpui-app-run-wasm-lifetime.patch"
CLIPBOARD_PATCH_FILE="$SCRIPT_DIR/patches/gpui-web-clipboard-paste.patch"

METADATA="$(cargo metadata --format-version 1 --manifest-path "$MANIFEST")"
GPUI_MANIFEST="$(python3 -c 'import json,sys; data=json.load(sys.stdin); packages=[p for p in data["packages"] if p["name"]=="gpui" and p.get("source", "").find(sys.argv[1])>=0]; assert len(packages)==1, f"expected exactly one gpui package at revision {sys.argv[1]}"; print(packages[0]["manifest_path"])' "$REV" <<<"$METADATA")"
GPUI_ROOT="$(dirname "$GPUI_MANIFEST")/../.."
GPUI_ROOT="$(cd "$GPUI_ROOT" && pwd)"

# The Cargo git checkout is cached between builds, so it can be left with a
# partially applied patch (or a patch from another revision). Reset only the
# files the patch owns before applying it again.
apply_patch() {
  local patch="$1"
  if git -C "$GPUI_ROOT" apply --reverse --check "$patch" >/dev/null 2>&1; then
    printf 'GPUI Web patch already applied at %s\n' "$GPUI_ROOT"
    return
  fi
  if ! git -C "$GPUI_ROOT" apply --check "$patch" >/dev/null 2>&1; then
    local paths
    paths="$(sed -n 's|^+++ b/||p' "$patch")"
    # shellcheck disable=SC2086
    git -C "$GPUI_ROOT" checkout -- $paths
    printf 'Reset cached GPUI files before patching %s\n' "$GPUI_ROOT"
  fi
  git -C "$GPUI_ROOT" apply "$patch"
  printf 'Applied GPUI Web patch to %s\n' "$GPUI_ROOT"
}

apply_patch "$PATCH_FILE"
if ! git -C "$GPUI_ROOT" apply --reverse --check "$PATCH_FILE" >/dev/null 2>&1; then
  printf 'GPUI checkout does not match the expected patched revision: %s\n' "$GPUI_ROOT" >&2
  exit 1
fi

apply_patch "$CLIPBOARD_PATCH_FILE"
if ! git -C "$GPUI_ROOT" apply --reverse --check "$CLIPBOARD_PATCH_FILE" >/dev/null 2>&1; then
  printf 'GPUI checkout does not match the expected clipboard patch revision: %s\n' "$GPUI_ROOT" >&2
  exit 1
fi
