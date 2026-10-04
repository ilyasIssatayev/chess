# Offline recording evaluation

This development workflow prepares local annotated media, runs the installed base
neural models in the browser, and replays their evidence through the Rust temporal
move decoder. It advances F06/F10 tooling. [Native CPU model parity](native-vision.md)
has a separate check. Physical recording and release qualification gates remain
pending.

## Prepare a recording

Create an [annotation manifest](evaluation-format.md) with independently annotated
initial position, calibration, native frame indexes, capture timestamps and move
completion bounds. Keep complete sessions in separate partitions. Use short,
continuous annotated frame ranges: the decoder deliberately stops after a native
sequence gap rather than pretending that intervening frames were observed.

Install FFmpeg/ffprobe locally and use the project's Rust toolchain. From the
repository root:

```sh
python3 scripts/prepare-evaluation.py local-data/session/manifest.json local-data/session/prepared
# For a manifest containing several sessions:
python3 scripts/prepare-evaluation.py local-data/dataset.json local-data/prepared --session session-id
```

The output folder must not exist. The tool validates the manifest, hashes the
source, verifies any declared source hash, and extracts the selected native video
frames as lossless PNGs without autorotation or frame-rate resampling. It checks
frame count, dimensions and individual image hashes before publishing the staged
folder. The copied manifest seals the selected source hash and adjusts its local
media path relative to the output folder; original annotations remain unchanged.

Limits are 2 GiB source media, 2000 selected frames, 512 MiB extracted images and
five minutes of extraction. FFprobe has a one-minute deadline. Use a shorter clip
when these bounds are exceeded. Local paths only are accepted. Media presentation
annotations must agree with the decoded presentation timeline relative to the
first selected frame (within 2 µs); host/device acquisition times remain their own
clock and are never replaced with presentation or processing time.

Output contains `bundle.json`, a copied `manifest.json` and `frames/`. Keep the
source media alongside its annotations for provenance checks. Calibration refers
to the original encoded image dimensions, including its original orientation.

## Recognize recorded frames

Start `./scripts/start-recorder.sh` and open
<http://localhost:8770/evaluation.html>, also linked from the live recorder. Select
the prepared folder. Images are read and hash-checked locally in the browser; they
are not uploaded and this page never writes games to the recorder database.

The calibrated overlay and Previous/Next controls let you inspect individual
frames. Replay recognition processes every prepared frame in order, measures
motion, runs the actual base model graphs and preserves raw 64-square evidence.
Personal classifier heads are intentionally excluded so an imported recording
cannot silently use another session's training. Model and calibration identities,
source/image hashes, native indexes, motion threshold, capture times and inference
cost are included in the observation trace. Inference cost is a separate field.

Download observations after completion, or expand the inspect/copy control and
save its JSON into a file. Cancellation and errors prevent export of a partial
trace; run the complete recording again. The folder picker and downloads depend
on the browser's normal file support. Neither operation requires a remote service.

## Score the trace

Use the copied, sealed manifest from the preparation folder:

```sh
cargo run --offline -p chess-evaluation --bin observation-replay -- \
  local-data/session/prepared/manifest.json observations-session-id.json > report.json
```

The scorer first validates the complete trace, matching every annotated index and
capture timestamp, one capture session, consistent versions and normalized square
probabilities. It validates the annotated chess history separately. Reference
moves are never supplied to the decoder. The decoder starts from the annotated
initial position and commits its own proposals, including legal-but-wrong moves;
it never resets itself to reference truth after an error.

The deterministic JSON report includes:

- All reference plies, replayed commits, correct/incorrect commits, precision and
  coverage. Extra moves count as incorrect and missing moves stay in the coverage
  denominator. Correctness requires an exact matching prefix at the same ply.
  Commits before the earliest annotated completion are incorrect. Empty
  denominators produce `null`, not an invented perfect score.
- Per-frame decisions and review reasons, and every committed UCI/SAN/FEN chain.
  Native sequence gaps and long capture gaps latch the remaining stream into
  review; the tool does not invent intermediate moves or silently re-arm.
- Completion interval comparisons for correctly recorded moves: containing the
  entire reference interval, overlapping only, or disjoint. Unknown reference
  completion receives no point estimate. P95 completion interval width and P95
  inference cost are separate measurements.
- The full decoder configuration and observation policy identity. Replaying the
  same trace/configuration emits the same report; model inference itself need not
  be bit-for-bit identical across browser runtimes.

Default playback settles for 900000 µs, requires three consistent observations,
and latches acquisition gaps above 1000000 µs. These defaults are unqualified.
An optional third positional argument supplies the full JSON configuration:

```json
{
  "decoder": {
    "settle_time_us": 900000,
    "min_visible_squares": 56,
    "min_expected_square_probability": 0.1,
    "min_average_log_likelihood": -0.75,
    "min_margin": 0.1,
    "consistent_frames": 3
  },
  "max_capture_gap_us": 1000000
}
```

Changing a decoder threshold does not regenerate observations. Changing the
motion threshold, model, or calibration requires a new browser replay. Tune only
on development/validation sessions and record each configuration. Hash assertions
in a trace identify its inputs but cannot prove that its creator ran the claimed
model; retain prepared images and verify original media independently.

## Remaining gates

The tool refuses `qualification` sessions. Its policy immediately commits offline
decoder proposals; this is different from auditing the recorder's original live
automatic decisions, interventions and processing backlog. A frozen original
live decision audit is still required for release. Browser ONNX inference followed
by Rust temporal decoding also does not establish Rust ONNX preprocessing/runtime
parity by itself; the separate native check covers that boundary on the target Mac.
Crop visibility remains an evidence-quality proxy rather than a trained
occlusion detector.

The current recorded-photo regression is a repeated public still from the model
author's corpus. It verifies decoding, calibration, graph execution, capture
clocks and unchanged-position behavior. Independent recordings of the user's
side-camera board, captures, castling, en passant, promotions, adjustments,
occlusions and interruptions remain necessary before any physical accuracy claim.
