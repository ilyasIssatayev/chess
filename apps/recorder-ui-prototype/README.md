# Local neural camera recorder

The recorder now uses two pretrained photo classifiers to read square occupancy
and piece identity from the live camera. It feeds normalized per-square evidence
into the existing Rust temporal/legal-position decoder, then saves accepted
moves in SQLite and exports canonical PGN. Camera images stay in the browser.

## Run

```sh
./scripts/start-recorder.sh
```

Open <http://localhost:8770/>. The script uses the project-local Rust toolchain if
present. On a fresh checkout it installs about 24 MB of pinned, hash-checked
model/runtime assets in ignored `local-data/vision/`. Once installed it runs
fully offline. To install/check independently:

```sh
python3 scripts/setup-vision.py
python3 scripts/setup-vision.py --check
```

Choose a different port with `./scripts/start-recorder.sh 8771`. A static Python
HTTP server has no recorder API or model asset routes and cannot record games.

## Record

1. Leave **Recognition** on **Piece recognition**, and wait for **Piece model
   ready**. Model loading executes both ONNX graphs before reporting ready.
2. Start the camera and place the board so every square and piece is visible.
   Leave space around the board: identity crops extend above the square to
   include tall pieces. A near-overhead view is easiest.
3. Calibrate the labeled outer corners (`a8`, `h8`, `h1`, `a1`) and save.
   The unmirrored preview and image sampling share the camera's aspect ratio.
   The model supports all four board rotations, but reflected labels are
   rejected. Earlier mirrored prototype calibration is not reused.
4. Use **Read camera position** to inspect the model's readout. Amber outlines
   mark weak/cropped evidence. Compare it with **Tracked position**.
5. Set the physical pieces to the tracked position, check the confirmation box,
   and click **Set reference position**. The model verifies the position before
   arming; it lists squares it cannot verify instead of assuming the setup.
6. Play one move and let the board settle. Pixel motion suppresses inference
   while hands/pieces move. Neural readings must support the same legal successor
   across three observations. Captures, castling, en passant and promotions use
   the Rust rules and piece identity; no engine move preference is involved.
7. Confirm **Record selected move**, or enable **Automatically record
   model-supported moves**. Automatic requests require a current server-side
   proposal token, a matching capture session and supporting latest evidence.

The **Camera model readout** shows the independently recognized board, inference
latency, uncertain squares and differences from the tracked game. If recognition
abstains, improve lighting/view/corners or enter the actual legal move manually.
Click a square after **Read camera position** to inspect its exact occupancy and
identity crops, top three scores and image coverage. Shallow-view warnings measure
the projected square footprint; they do not detect a hidden piece.
Manual entry, undo, new games, calibration/mode changes, hidden tabs and capture
interruptions invalidate recognition and require a fresh reference. A reviewed
model suggestion advances the reference and lets the next player move.

**Square changes · manual review** is an explicit fallback when the model does
not work on a piece set. Its threshold controls pixel motion/change detection.
It cannot make automatic commits. In piece mode the same threshold controls
motion sensitivity; it does not change model confidence thresholds.

**Undo last move** creates an audited correction. Restore the physical position
to the tracked board afterward. **New game** retains the old game in SQLite.
**Export PGN** downloads the current game. Reloading resumes the most recent
game but always requires a fresh camera reference. Games are stored in ignored
`local-data/recorder/games.sqlite`; `CHESS_RECORDER_DATA_DIR` selects an isolated
folder for tests.

## Adapt to a side camera and your pieces

1. Fix the camera and lighting; calibrate once. Keep piece bases visible and leave
   room above the board for tall pieces. If the crop inspector shows a neighboring
   piece blocking the intended one, improve the camera angle first.
2. Expand **Teach recognition your pieces**. Match the physical board to the
   tracked position, clear your hands, confirm the example checkbox and click
   **Save position example**. Saving disarms recording so training labels cannot
   silently follow an unconfirmed camera move.
3. Enter and record the actual move manually, then save another confirmed example.
   Collect at least three *different piece placements*; 6–10 positions spread
   across the board are a useful starting point for testing. Every piece type and
   color needs at least two unobstructed examples in the training positions.
4. **Train personal recognition** learns regularized, class-balanced softmax
   heads from the pretrained networks' frozen 1024-dimensional neural features.
   The entire last position (including repeated frames) is held out to compare
   personal and base predictions before the final heads are fit on all examples.
   Read the score and listed errors. One held-out position cannot establish
   general accuracy; independent play remains necessary.
5. Enable **Use personal recognition**, read the board, then set a fresh reference.
   Personal suggestions must be reviewed. Automatic requests are also rejected
   by the Rust server, even if a proposal token is otherwise valid.

Up to 20 examples and the fitted heads persist in IndexedDB for this browser and
localhost origin. Only neural features, confirmed labels, calibration and model
metadata are stored; camera photos/crops are not saved. New examples require
retraining to update the head. **Clear examples** clears that browser's profile.
The profile is bound to the base model, labeled corners and processed frame size.
Changed geometry disables its use; physically moving a camera without changing
these settings cannot be detected, so collect new examples after any repositioning.

This is local classifier-head adaptation, not a replacement side-view model or
full backbone fine-tuning. It may improve unfamiliar piece identities but cannot
recover fully occluded pieces or repair crops dominated by neighboring pieces.

## Model and limits

[Model source](https://huggingface.co/cstr/chess-board-photo-onnx): two MIT
MobileNetV3-Small classifiers (100×100 occupancy and 100×200 piece crops), pinned
at revision `4690fd5418ff7c405a76d1c156077ecd865ac053`. The published class order,
ImageNet normalization, image-order projective warps, asymmetric piece crops,
left-half mirroring and padding are implemented in `vision-core.js`. Inference
runs off the UI thread in ONNX Runtime Web 1.23.2, WebAssembly CPU, batch size 8.
[Manifest](../../models/vision-manifest.json) records hashes and graph inputs;
[notices](../../models/NOTICE.md) record source/licensing attribution.

The base models are pretrained; optional personal heads learn from confirmed
examples without changing the backbone weights.
They can mistake unfamiliar pieces, shallow views, shadows and occlusion. Crop
coverage and output certainty gate usable evidence; they are **not** a learned
hand/occlusion detector or a calibrated accuracy estimate. Physical-board
move-recognition accuracy remains unqualified until independent recordings are
annotated and evaluated. Move-completion timing remains unknown. Poor readings
fail closed and offer manual review; there is no silent fallback to automatic
pixel-change guesses.

The browser only requests assets from localhost and sends probability/game
commands to the local Rust server. There is no cloud camera upload or hosted
inference. The server's asset allowlist exposes neither repository files nor
private camera data. Normalized evidence, model version, calibration version,
session identity, monotonic time, observation sequence, position revision and
proposal freshness are checked before accepting automatic moves.

## Replay recorded camera evidence

Open <http://localhost:8770/evaluation.html> to inspect and replay a prepared
annotated clip. [The offline evaluation guide](../../docs/offline-evaluation.md)
covers frame extraction, hash checks, preserved capture clocks, observation
exports and Rust move/timing scoring. This page uses the base models and does not
write saved games. Independent physical recordings are still needed to measure
the user's side-camera accuracy.

## Checks

```sh
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc \
  apps/recorder-ui-prototype/vision-core.js \
  apps/recorder-ui-prototype/tests/vision-core.test.js
/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc \
  apps/recorder-ui-prototype/vision-core.js \
  apps/recorder-ui-prototype/vision-personal.js \
  apps/recorder-ui-prototype/tests/vision-personal.test.js
/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc \
  apps/recorder-ui-prototype/recorder.js \
  apps/recorder-ui-prototype/tests/recorder.test.js
/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc \
  apps/recorder-ui-prototype/vision-core.js \
  apps/recorder-ui-prototype/evaluation-core.js \
  apps/recorder-ui-prototype/tests/evaluation-core.test.js
python3 -m unittest discover -s scripts/tests
```

Neural regressions cover rotations, reflected labels, projective sampling,
crop sizes/flipping/padding, normalization, class mapping, probability sums and
weak/cropped evidence. Rust integration covers verified references, temporal
confirmation, automatic commit authorization, contradictions, motion during
review, hidden two-move gaps, session changes and manual correction recovery.

The optional browser smoke page is served only with `CHESS_RECORDER_TEST_MODE=1`
at `/vision-smoke.html`. It needs the licensed public photo saved at
`local-data/vision/tests/fixture.jpg` (see `models/NOTICE.md`). It executes the
actual weights and compares its reading to the photo's labelled placement.
The fixture comes from the model author's test corpus and is a plumbing
regression, not an independent physical accuracy evaluation.
