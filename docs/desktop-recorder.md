# Desktop development candidate

Updated 4 October 2026. The Tauri application and native recording path are
implemented. This is a development candidate: physical placement, side-camera
accuracy, timing accuracy, supported playing speed and release qualification
remain unmeasured.

## Open or rebuild

Open `dist/ChessCameraRecorder.app` after running `scripts/package-desktop.sh`.
Use `scripts/start-desktop.sh` for a development build. Both scripts use the
project-local Rust toolchain and verify installed model/runtime assets. Native
assets can be installed with `scripts/setup-vision.py` and
`scripts/setup-native-vision.py`. Builds use the committed Cargo lockfile.

The bundle includes the Tauri executable, Rust recorder CLI, native camera helper,
UI, pretrained graphs, browser runtime, native ONNX Runtime and license notices.
It starts its own loopback-only Rust service on an available port. Release builds
use bundle resources rather than paths into this checkout. Local data uses
Tauri's application-data directory; `CHESS_RECORDER_DATA_DIR` selects a separate
profile for development or testing. This candidate is ad-hoc signed, not notarized
for public distribution. `dist/candidate-manifest.json` hashes the packaged files
and explicitly records that the candidate is not release-qualified. The final
build reports successful window creation and runs its service; final close/reopen
visual verification remains pending because desktop automation could not locate
that window. Earlier recorder/library visual checks passed.

## Native recording

1. Select **Native MacBook camera**, then start the camera.
2. Calibrate the four labeled corners and save. Saving changed corners or motion
   sensitivity stops native capture; restart it to use the new configuration.
3. Arrange the physical pieces to match the tracked position, clear your hands,
   confirm visibility and set a reference. Rust verifies all 64 squares before
   arming the decoder.
4. Review supported suggestions or enable experimental automatic recording.
   Manual moves, corrections, pauses and capture interruptions require a fresh
   reference. Native personal classifier heads are not implemented; adaptation
   remains available in the browser camera path.

A narrow Swift AVFoundation adapter supplies bounded BGRA frames and sample
presentation timestamps. Rust owns conversion, geometry, motion measurements,
CPU inference, temporal/legal decoding, persistence and evidence handling.
The adapter uses the built-in camera and discards late callbacks. A latest-frame
mailbox and a sixteen-reading output queue bound backlog; counters distinguish
camera-reported drops from frames skipped before inference. Discontinuities,
stale frames or observation-queue overflow invalidate the reference. Inference
runs independently of the database writer and request parsing.

Native model-supported moves store conservative completion bounds and elapsed
bounds within one continuous verified session. First moves and moves after a
new reference/restart have no preceding completion interval. Browser-only and
manual moves retain unknown completion timing. Confirmation time and inference
cost are separate; none is official chess-clock time. These intervals have not
been validated against annotated physical moves.

Native reference, pending/review and accepted evidence is stored locally as
hash-checked JPEGs. A retained trusted pre-move frame and accepted frame support
committed native moves. Files are staged before manifest insertion. Partial
files are reconciled on open; orphan complete files do not create evidence
claims and still count toward the quota. The evidence folder has a 512 MiB cap.
Reaching it records evidence loss rather than silently deleting reviewed media.
Missing or changed files are labeled unavailable in the library. Evidence is
sampled frames, not continuous game video; the separate collection workflow is
still needed for accuracy evaluation.

## Library, review and exports

**Game library, replay and corrections** opens saved games, searches players/date,
filters recording status, edits players/result/status, replays the active FEN/SAN
chain, displays elapsed intervals and evidence, and exposes original decisions
and recording gaps. Finish and pause are recording states; `*` remains the result
until explicitly changed. Resume requires a fresh camera reference.

Earlier-move correction accepts UCI (including a promotion suffix) and an audit
reason. It creates an immutable revision, recomputes subsequent notation and
stops at the first illegal continuation. The original revision and invalid
suffix remain available for review; the game is marked incomplete and exports
its legal prefix with `*`. Corrections never remove original automatic decisions
from the audit.

Export clean PGN, PGN with descriptive timing comments, JSON or CSV timing
sidecars. Date is derived from game creation in UTC. JSON includes timings,
evidence and recorded gaps; CSV preserves explicit unknown values. Independent PGN parsing passes for fifteen Rust exports covering both castling
sides, en passant and every promotion choice. Chess.com analysis imports were
checked for an ordinary game with castling, en passant and knight promotion.
The importer inferred a draw for an insufficient-material promotion even when
input Result was `*`; explicit known results should be entered in the library.
A complete physical recording/export walkthrough remains unqualified.

## Backup and restore

**Back up games and evidence** creates a consistent SQLite snapshot using
`VACUUM INTO`, copies available hash-verified evidence and seals a file manifest.
**Restore into a new data profile** validates hashes, bounds and paths, checks
journal replay, then switches to a new folder. Original data is preserved and
an atomic profile pointer makes the restored profile survive restart. Backup
paths are visible in the library. Missing evidence remains missing after restore.

For standalone restoration into a new folder:

```sh
./target/release/chess-camera-recorder restore BACKUP_DIRECTORY NEW_DATA_DIRECTORY
```

## Checks and remaining gates

`python3 scripts/check-native-camera.py` measures timestamp cadence without saving
images. `python3 scripts/check-recorder-workflows.py` runs isolated HTTP checks for
idle sockets, persistence, corrections, immutable originals, exports, backup,
restore and restored-profile restart. It needs local socket access.

The native camera probe measured 120 frames at 1920×1080 and about 29.48 fps using
AVFoundation presentation timestamps, with a maximum 116,683 µs interval. A short
packaged live test executed native inference, delivered JPEG preview, rejected
an invalid reference and committed no moves. The observed inference readings
included 472–677 ms per board; this short run is not a representative latency
benchmark or the required 60-minute soak.

Remaining: populated-board feasibility in both orientations; independent
annotated development/validation recordings; drift/occlusion and real outage
measurements; model/threshold selection; representative soak and playing-speed
limits; complete physical recording/export walkthroughs; frozen original-decision
qualification on 30 complete games and at least 3,000 reference and automatic
plies; and clean-Mac/distribution verification. No physical or release phase gate
is marked passed by this software checkpoint.
