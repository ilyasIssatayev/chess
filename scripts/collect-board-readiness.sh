#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    printf '%s\n' "usage: $0 OUTPUT_DIRECTORY" >&2
    exit 2
fi

workspace_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output_dir=$1

case "$output_dir" in
    /*) ;;
    *) output_dir="$workspace_dir/$output_dir" ;;
esac

if [ -e "$output_dir" ]; then
    printf '%s\n' "error: output already exists: $output_dir" >&2
    exit 1
fi

mkdir -p "$output_dir"
cd "$workspace_dir"
cargo build -p capture-probe
probe="$workspace_dir/target/debug/capture-probe"

{
    printf 'captured_utc='; date -u '+%Y-%m-%dT%H:%M:%SZ'
    printf 'macos='; sw_vers -productVersion
    printf 'architecture='; uname -m
    printf 'rust='; rustc --version
    printf 'hardware='; system_profiler SPHardwareDataType | sed -n 's/^[[:space:]]*Model Name: /Model Name: /p; s/^[[:space:]]*Model Identifier: /Model Identifier: /p'
} > "$output_dir/system.txt"

"$probe" devices --json > "$output_dir/devices.json"
"$probe" formats --device 0 --json > "$output_dir/formats.json"

printf '%s\n' "Place the populated board with every boundary visible."
printf '%s\n' "Keep the laptop, board, and lighting fixed for both captures."
printf '%s' "Press Enter to capture orientation A... "
read -r ignored
"$probe" sample --device 0 --frames 300 --format highest-fps --save-every 30 \
    --output "$output_dir/orientation-a"
"$probe" verify "$output_dir/orientation-a"

printf '%s\n' "Rotate the board 180 degrees without moving the laptop."
printf '%s' "Press Enter to capture orientation B... "
read -r ignored
"$probe" sample --device 0 --frames 300 --format highest-fps --save-every 30 \
    --output "$output_dir/orientation-b"
"$probe" verify "$output_dir/orientation-b"

cat > "$output_dir/annotations.csv" <<'CSV'
run_id,event_id,intended_uci,last_unchanged_sequence,motion_start_sequence,first_settled_sequence,source_visible,destination_visible,other_occlusion,notes
CSV

cat > "$output_dir/README.txt" <<'TEXT'
Evidence collection completed. Inspect every saved PPM in both orientation
directories. Record the four board corners and fill annotations.csv during a
separate scripted-move capture. Do not mark feasibility passed until all 64
squares and far-rank piece bases remain visible in both orientations.

These runs use process receipt timestamps. Each frame's acquisition interval is
bounded by capture_started_mono_ns and received_mono_ns; it is not a native
camera presentation timestamp.
TEXT

printf '%s\n' "Board-readiness evidence written to $output_dir"
