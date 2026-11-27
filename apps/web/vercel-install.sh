#!/usr/bin/env bash
set -euo pipefail

if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

source "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown
cargo install trunk --locked --version 0.22.2
