#!/bin/sh
set -eu

workspace_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
app_dir="$workspace_dir/dist/ChessCameraProbe.app"
contents_dir="$app_dir/Contents"
binary_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"

cd "$workspace_dir"
cargo build --release -p capture-probe

mkdir -p "$binary_dir" "$resources_dir"
install -m 755 target/release/capture-probe "$binary_dir/capture-probe"

cat > "$contents_dir/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>capture-probe</string>
    <key>CFBundleIdentifier</key>
    <string>com.local.chess-camera-recorder.probe</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Chess Camera Probe</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSBackgroundOnly</key>
    <true/>
    <key>NSCameraUsageDescription</key>
    <string>Observe a physical chessboard so the app can record moves.</string>
</dict>
</plist>
PLIST

cat > "$resources_dir/camera.entitlements" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.security.device.camera</key>
    <true/>
</dict>
</plist>
PLIST

plutil -lint "$contents_dir/Info.plist" "$resources_dir/camera.entitlements"
codesign --force --deep --sign - \
    --entitlements "$resources_dir/camera.entitlements" \
    "$app_dir"
codesign --verify --deep --strict "$app_dir"

printf '%s\n' "Packaged $app_dir"
printf '%s\n' "Run: $binary_dir/capture-probe permission"
