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
cargo build --offline --release -p chess-desktop -p chess-camera-recorder
app_dir="$project_dir/dist/ChessCameraRecorder.app"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources/ui" "$app_dir/Contents/Resources/vision/models" "$app_dir/Contents/Resources/vision/native" "$app_dir/Contents/Resources/vision/runtime"
install -m 755 target/release/chess-desktop "$app_dir/Contents/MacOS/chess-desktop"
install -m 755 target/release/chess-camera-recorder "$app_dir/Contents/MacOS/chess-camera-recorder"
install -m 755 target/native/chess-camera "$app_dir/Contents/Resources/chess-camera"
cp apps/recorder-ui-prototype/*.html apps/recorder-ui-prototype/*.js apps/recorder-ui-prototype/*.css "$app_dir/Contents/Resources/ui/"
cp local-data/vision/models/*.onnx "$app_dir/Contents/Resources/vision/models/"
cp local-data/vision/runtime/ort.wasm.min.mjs local-data/vision/runtime/ort-wasm-simd-threaded.mjs local-data/vision/runtime/ort-wasm-simd-threaded.wasm "$app_dir/Contents/Resources/vision/runtime/"
cp local-data/vision/native/libonnxruntime.1.23.2.dylib local-data/vision/native/LICENSE "$app_dir/Contents/Resources/vision/native/"
cp models/NOTICE.md models/vision-manifest.json models/native-runtime.json LICENSE "$app_dir/Contents/Resources/"
cat > "$app_dir/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>chess-desktop</string><key>CFBundleIdentifier</key><string>com.local.chess-camera-recorder</string><key>CFBundleName</key><string>Chess Camera Recorder</string><key>CFBundleDisplayName</key><string>Chess Camera Recorder</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleShortVersionString</key><string>0.1.0</string><key>CFBundleVersion</key><string>1</string><key>LSMinimumSystemVersion</key><string>13.0</string><key>NSHighResolutionCapable</key><true/><key>NSCameraUsageDescription</key><string>Record moves from your physical chessboard and retain local evidence for review.</string>
</dict></plist>
PLIST
cat > "$app_dir/Contents/Resources/camera.entitlements" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>com.apple.security.device.camera</key><true/></dict></plist>
PLIST
plutil -lint "$app_dir/Contents/Info.plist"
codesign --force --sign - --entitlements "$app_dir/Contents/Resources/camera.entitlements" "$app_dir/Contents/Resources/chess-camera"
codesign --force --sign - --entitlements "$app_dir/Contents/Resources/camera.entitlements" "$app_dir"
codesign --verify --strict "$app_dir"
python3 scripts/freeze-candidate.py "$app_dir" "$project_dir/dist/candidate-manifest.json"
printf '%s\n' "Packaged development candidate: $app_dir"
