#!/bin/sh
set -eu
project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"
mkdir -p target/native .toolchain/swift-cache
swiftc -O -module-cache-path "$project_dir/.toolchain/swift-cache" native/camera.swift -o target/native/chess-camera
