#!/usr/bin/env bash
set -euo pipefail

TRUNK_VERSION=0.22.2
CARGO_BIN_DIR="$(dirname "$(command -v cargo)")"
curl --proto '=https' --tlsv1.2 -fsSL \
  "https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-x86_64-unknown-linux-gnu.tar.gz" \
  | tar -xz -C "$CARGO_BIN_DIR" trunk
rustup target add wasm32-unknown-unknown
