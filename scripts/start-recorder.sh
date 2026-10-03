#!/bin/zsh
set -eu
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"
if [[ -x "$PROJECT_DIR/.toolchain/cargo/bin/cargo" ]]; then
  export CARGO_HOME="$PROJECT_DIR/.toolchain/cargo"
  export RUSTUP_HOME="$PROJECT_DIR/.toolchain/rustup"
  export RUSTUP_TOOLCHAIN="stable-aarch64-apple-darwin"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
if ! python3 scripts/setup-vision.py --check; then
  python3 scripts/setup-vision.py
fi
exec cargo run --offline -- serve "${1:-8770}"
