# Native CPU recognition and parity

F08 now has a Rust photo-recognition adapter in `crates/vision-inference`. It runs
the installed occupancy and piece graphs on CPU and produces the same normalized
64-square evidence contract used by the browser recorder. The native adapter is
available through a library and development CLIs; the live browser recorder still
executes its browser worker. Tauri/native camera integration is the next consumer
of this adapter.

## Install and run

On the target macOS arm64 machine, install the existing models and the separately
pinned native runtime:

```sh
python3 scripts/setup-vision.py
python3 scripts/setup-native-vision.py
python3 scripts/setup-native-vision.py --check
cargo build --offline --release -p vision-inference
./target/release/vision-infer photo.png corners.json > recognition.json
```

The [runtime manifest](../models/native-runtime.json) pins Microsoft's official
[ONNX Runtime 1.23.2 release](https://github.com/microsoft/onnxruntime/releases/tag/v1.23.2),
its archive and individual library/license hashes. The installer extracts only
named, verified members. The Rust adapter checks both model files and the native
library before loading them. Model paths and class order remain tied to
[the existing model manifest](../models/vision-manifest.json). Assets stay under
ignored `local-data/vision/`; startup needs no network after installation.

Corners use normalized image coordinates, labeled by algebraic corner rather
than image left/right. For the existing 1200×800 public regression photo:

```json
{
  "a8": {"x": 0.2708333333333333, "y": 0.2875},
  "h8": {"x": 0.6583333333333333, "y": 0.29},
  "h1": {"x": 0.6541666666666667, "y": 0.8675},
  "a1": {"x": 0.2675, "y": 0.8625}
}
```

Use calibration for the actual input image dimensions/orientation. PNG and JPEG
are supported with bounded decoding: 16 MiB compressed input, dimensions no
larger than 8192 per side, 16 million pixels, and a 64 MiB decoder allocation
limit. Corners must be finite, in-frame, convex and consistently labeled.

The output records model/runtime/provider identity, image dimensions, inference
cost, square evidence, crop coverage, side-view diagnostics and graph
logits/probabilities/features for development inspection. It contains no camera
acquisition timestamp; callers must supply that separately when creating a
`FrameObservation`. Inference time cannot be used as capture time.

Normal inference runs the piece model where occupancy probability is at least
0.10. `--all-pieces` also executes identity crops on likely-empty squares for
inspection/parity. Both modes use batches of at most eight and one CPU thread.
The output's graph features are the pinned 1024-dimensional penultimate outputs;
no personal classifier heads or CoreML provider are enabled.

For native consumers, construct an owned `RgbaFrame`, call
`NativeRecognizer::load(assets_root)` once, then reuse the recognizer across
frames. Initialize this adapter before any other `ort` environment in that
process. A pre-existing committed external environment is rejected. The CLI's
default asset path is for this development checkout; packaged applications must
pass their resource directory explicitly. Native capture queues, interruption
handling and desktop delivery remain separate work.

## Reproduce parity

Build the release tools first. The development reference environment uses
`onnx==1.19.1` and `numpy==2.0.2`; these are not runtime dependencies:

```sh
python3 -m venv local-data/vision/parity-env
local-data/vision/parity-env/bin/pip install -r scripts/native-vision-reference-requirements.txt
local-data/vision/parity-env/bin/python scripts/check-native-vision.py local-data/vision/parity-check
# Optional known public fixture from the existing smoke check:
local-data/vision/parity-env/bin/python scripts/check-native-vision.py \
  local-data/vision/parity-photo-check --photo local-data/vision/tests/fixture.jpg
```

Output directories must be new. The checker creates a synthetic image, exercises
all four label rotations, skew, high-precision calibration, edge-adjacent crops and a shallow board. It can
also run the licensed public photo with its known calibration and labels. It
retains a report and stage artifacts for inspection, and returns nonzero on any
failed comparison. Model input hashes are verified in both native and reference
paths.

The actual `vision-core.js` functions execute under macOS JavaScriptCore on the
same decoded RGBA bytes as Rust. SHA-256 comparisons cover every RGB occupancy
and identity crop and every little-endian NCHW f32 tensor. Square orientation,
crop coverage and side-view diagnostics must match exactly. This includes
asymmetric crop extents, left-half mirroring, padding, bilinear sampling and
ImageNet normalization performed in double precision before f32 storage.

Native CPU graph outputs are compared with ONNX's independently implemented
`ReferenceEvaluator`, not with another call through the Rust adapter. The check
covers logits, penultimate features, top classes, probabilities, normalized
square evidence and visibility flags. It also checks production occupancy-based
selection, empty-square fallback and tail batches. Fixed tolerances are:

| Comparison | Acceptance |
| --- | --- |
| Crop bytes / f32 tensor bytes | Identical SHA-256 hashes |
| Geometry mapping / coverage / diagnostics | Exact match |
| Logits / features | Absolute tolerance 0.0001, relative tolerance 0.0001 |
| Probabilities / square evidence | Absolute tolerance 0.00001 |
| Top classes / visibility flags / selected crop indexes | Exact match |

`vision-preprocess IMAGE CORNERS.json NEW_OUTPUT_FOLDER` exports native stage
artifacts independently. The reference checker calls the same CLI and compares
its outputs with the existing JavaScript implementation. Models run through
`ort = 2.0.0-rc.10` with runtime loading and automatic build-time binary downloads
disabled; the [versioned API documentation](https://docs.rs/ort/2.0.0-rc.10/ort/)
describes that adapter dependency.

## Evidence and limits

The 4 October checkpoint passes nine cases (eight synthetic geometries plus the
public photo), including all 64 crops per graph and the normal selected-crop
path. The public photo retains 64/64 labeled-square agreement. Reports remain
under ignored `local-data/vision/native-parity-*`.

This establishes base-model preprocessing and CPU graph parity at the
already-decoded RGBA boundary on this Mac. JPEG decoders, ICC handling, browser
Canvas scaling/alpha conversion and native camera resampling are outside this
check. Browser WebAssembly graph outputs are not individually compared here;
the existing browser smoke check proves execution and fixture agreement, while
this check uses JavaScript preprocessing and ONNX's reference graph evaluator.

The photo comes from the model author's corpus and the other cases are
synthetic. They establish software consistency, not independent physical
side-camera accuracy. Crop certainty/coverage and compression warnings remain
proxies, not a trained occlusion detector. Complete-session development and
validation recordings, model/decoder selection and physical timing qualification
are still required before Phase 2/3 gates pass. Personal-head native execution,
CoreML, other architectures and Tauri packaging remain unqualified.
