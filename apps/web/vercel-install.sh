#!/usr/bin/env bash
set -euo pipefail

TRUNK_VERSION=0.22.2
if ! command -v trunk >/dev/null || [[ "$(trunk --version 2>/dev/null || true)" != *"${TRUNK_VERSION}"* ]]; then
  CARGO_BIN_DIR="$(dirname "$(command -v cargo)")"
  case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) TRUNK_TARGET=aarch64-apple-darwin ;;
    Darwin-x86_64) TRUNK_TARGET=x86_64-apple-darwin ;;
    Linux-x86_64) TRUNK_TARGET=x86_64-unknown-linux-musl ;;
    *)
      printf 'Unsupported Trunk host: %s-%s\n' "$(uname -s)" "$(uname -m)" >&2
      exit 1
      ;;
  esac
  curl --proto '=https' --tlsv1.2 -fsSL \
    "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-${TRUNK_TARGET}.tar.gz" \
    | tar -xz -C "$CARGO_BIN_DIR" trunk
fi
rustup target add wasm32-unknown-unknown
