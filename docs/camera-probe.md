# Camera probe guide

This probe supports Frames 1–2 of the development plan. It enumerates the Mac's AVFoundation cameras and formats, records a bounded sequence of timestamped frames, and writes capture-health metadata for review. It does **not** establish that a given MacBook/board placement works; that decision requires running the physical procedure below.

The code is in `crates/capture-probe` and is a member of the root Cargo workspace.

## Prerequisites

- A current stable Rust toolchain (`rustup` is the standard installer).
- macOS with a built-in or attached camera.
- A foreground Terminal process with permission to use the camera.

Build and run from the repository root:

```sh
cargo build --manifest-path crates/capture-probe/Cargo.toml
cargo run --manifest-path crates/capture-probe/Cargo.toml -- permission
cargo run --manifest-path crates/capture-probe/Cargo.toml -- devices
cargo run --manifest-path crates/capture-probe/Cargo.toml -- formats --device 0
```

Create an ad-hoc signed development app bundle containing the camera usage description and entitlement:

```sh
scripts/package-camera-probe.sh
dist/ChessCameraProbe.app/Contents/MacOS/capture-probe permission
```

The bundle is a command-line feasibility tool, not the later dashboard. Ad-hoc signing is suitable for local testing only; distribution signing and notarization remain release work.

The first camera operation can show the macOS permission prompt. If access was denied, open **System Settings → Privacy & Security → Camera**, enable the terminal or packaged app being used, quit that process, and try again. Permission behavior for a Terminal-launched binary is only a development check; the later packaged application must include `NSCameraUsageDescription` and be tested separately.

On non-macOS systems the help command works, while camera commands return a clear unsupported-platform error.

## Record a camera report

First list formats. Then request a reported format exactly, or let the backend select one:

```sh
cargo run --manifest-path crates/capture-probe/Cargo.toml -- sample \
  --device 0 \
  --frames 300 \
  --format highest-fps \
  --save-every 30 \
  --output local-data/camera-probe/run-001
```

`local-data/` should remain outside version control when clips or personally identifying room imagery are stored there. The command writes:

- `manifest.json`: device, negotiated format, timestamp source, and aggregate capture-health values.
- `frames.jsonl`: one record per received frame with monotonic timing, wall-clock receipt time, capture-call duration, inter-arrival time, byte sizes, and any saved image path.
- `frames/frame-NNNNNN.ppm`: uncompressed RGB samples. Preview them with Finder/Preview or convert copies later if smaller images are needed.

The probe refuses to write into a non-empty output directory so an earlier run is not silently overwritten. Use a new `run-NNN` path for each capture.

Verify a completed run independently from the serialized evidence:

```sh
cargo run -p capture-probe -- verify local-data/camera-probe/run-001
```

The verifier recomputes sequences, capture intervals, inter-arrival values, gap counts, and summary totals from `frames.jsonl`. It also checks the timestamp-source contract and the dimensions and lengths of saved PPM files. A successful result detects accidental truncation or inconsistent derived fields; it does not turn process receipt times into native camera timestamps.

The probe deliberately labels its timestamps as `process_monotonic_after_blocking_capture`. Nokhwa's high-level frame API does not expose the native `CMSampleBuffer` presentation timestamp. Therefore the recorded value is a receipt bound after the blocking capture call returns, not a claim about sensor exposure time. Frame 3 should either carry this limitation into the capture contract or replace the adapter with a narrow AVFoundation implementation that exposes native PTS.

Only frames selected by `--save-every` are RGB-decoded, because decoding every 1920×1080 YUYV frame in the measurement loop distorted the observed cadence on the target Mac. Unsampled JSONL records use `decoded_rgb_bytes: 0`. For a clean cadence measurement use `--save-every 0`; run a separate short sample with saved frames to verify RGB conversion.

`gap_count` counts inter-arrival intervals greater than 2.5 times the negotiated frame period. It is a diagnostic signal, not proof of a driver-level dropped frame: application scheduling, decoding, and deliberate `--interval-ms` delays can all create gaps. Run measurement captures with the default zero interval.

## Frame 1 hardware checks

Run these checks on the target MacBook and preserve the console output plus the generated manifest:

1. Record the Mac model, macOS version, Rust version, physical camera name, and `formats` output in the work log.
2. With permission initially unset, run `permission` and record whether the prompt and grant path work.
3. Capture 300 frames at `highest-fps`, then at the highest practical resolution. Confirm that saved PPM images decode and show the expected field of view.
4. Deny camera permission, rerun `devices`, and record the exact error. Re-enable permission and verify recovery after restarting the terminal process.
5. Start a capture, cover the camera, unplug an external camera if one is being tested, or close the lid only when safe. Record whether the command returns an error or stalls. Do not infer disconnect handling from source code alone.
6. Inspect `observed_fps`, `max_interarrival_ns`, `mean_capture_block_ns`, and `gap_count`. Retain the raw JSONL so later adapters can be compared using the same fields.

Frame 1 passes only when a runnable probe, sample frames, format listing, permission/recovery observations, and timestamp report exist for the actual target MacBook. The current target has a granted permission result, device/active-format report, a 120-frame timing run, and a one-frame RGB output check recorded in the [work log](work-log.md). Permission denial/recovery and interruption behavior remain untested, so Frame 1 has not passed.

## Frame 2 placement procedure

For the first board session, the guided collector records machine/device metadata and a short verified sample in each orientation:

```sh
scripts/collect-board-readiness.sh local-data/board-readiness/session-001
```

The collector refuses to reuse an existing directory. It creates an annotation header and a session README beside the evidence. The two short samples establish field of view and saved-frame integrity; use a separate scripted-move recording for the event annotations below.

Use a stable table, an unmoving laptop, the complete intended board and pieces, and steady lighting. Tape or mark the board/laptop positions only after finding a view that includes all 64 squares.

1. Place the populated board so the built-in camera sees every square boundary. Avoid a very shallow angle: near pieces must not hide the bases or occupancy of far-rank pieces.
2. Record both board orientations. At the start of each run, point to `a1`, `h1`, `a8`, and `h8` in that order so orientation can be annotated without guessing.
3. Perform a scripted sequence containing quiet moves on near and far ranks, diagonal movement, a capture, castling, a piece adjustment that is not a move, and a hand resting over the center for several seconds.
4. Repeat under each supported lighting condition and at normal and quick move speeds. Change only one setup variable between runs.
5. For each scripted action, record the intended move, the last clearly unchanged frame, the first motion frame, the first clearly settled frame, visibility of the source/destination squares, and whether any other square was obscured.
6. Keep development and validation sessions separate. Do not tune placement or thresholds using the session reserved for validation.

A useful annotation CSV can use this header:

```text
run_id,event_id,intended_uci,last_unchanged_sequence,motion_start_sequence,first_settled_sequence,source_visible,destination_visible,other_occlusion,notes
```

Pass Phase 0 only if representative near- and far-rank moves remain distinguishable in both required orientations, all four board corners stay visible, hands clear long enough to observe the settled position, and capture health is adequate for the intended playing speed. Report failures and uncertainty directly. Perspective correction cannot recover a square hidden behind a piece or hand.

## Known implementation risks

- Nokhwa 0.10.11 is pinned for reproducibility, but its AVFoundation behavior must be exercised on the target macOS release and hardware. The upstream project has historical reports of AVFoundation pixel-format/decoding problems, which is why saved RGB frames are an explicit gate.
- Format enumeration can be incomplete or backend-specific. Treat the negotiated format in `manifest.json` as authoritative for a run.
- Nokhwa 0.10.11 returned an empty AVFoundation compatibility list on the current target Mac even though capture mode negotiation succeeded. In that case `formats` reports the active mode with `source: negotiated_fallback`; it must not be read as a complete list of every hardware mode.
- PPM output is intentionally simple and lossless but large. Use bounded captures and copy only small, non-sensitive fixtures into Git.
- The CLI provides sampled frames and terminal health output, not a graphical live preview. A GUI preview belongs in the integrated desktop phase once capture viability is established.
