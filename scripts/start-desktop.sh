#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
if [ -x .toolchain/cargo/bin/cargo ]; then
  export CARGO_HOME="$project_dir/.toolchain/cargo" RUSTUP_HOME="$project_dir/.toolchain/rustup" RUSTUP_TOOLCHAIN=stable-aarch64-apple-darwin
  export PATH="$CARGO_HOME/bin:$PATH"
fi
python3 scripts/setup-vision.py --check
python3 scripts/setup-native-vision.py --check
./scripts/build-native-camera.sh
exec cargo run --offline -p chess-desktop
