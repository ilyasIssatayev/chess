# Development work log

Status updated on 4 October 2026. Productive time and aggregate token usage are unavailable from the current client, so they are recorded as unknown rather than estimated. The initial wave used the current Codex runtime; helpers inherited the lead model because no override was selected.

Local camera artifacts are stored below ignored `local-data/` and are intentionally absent from Git. This log omits the machine serial number, hardware UUID, and camera images.

## Latest software checkpoint — 4 October

The native live pipeline and packaged Tauri development candidate now exist.
See [the desktop guide](desktop-recorder.md). F11–13 and F15–18 have substantial
software implementations: acquisition timestamps, bounded processing, sampled
hash-checked evidence, stale/gap reference invalidation, library/search, replay,
metadata/finish/resume, audited earlier-move correction, timing JSON/CSV and
annotated PGN, plus consistent backup and restoration into a new persistent
profile. SQLite schema 4 adds metadata, append-only activities and evidence.
Original automatic decisions remain in the immutable journal after correction.
An independent python-chess 1.11.2 corpus now checks fifteen notation/FEN histories
and parses the generated PGNs, including both castling sides, en passant and
all promotion choices. Chess.com analysis imports were exercised for castling,
en passant and knight promotion; the last case revealed that the importer can
infer an insufficient-material draw when the original Result was `*`.


A native 120-frame camera check measured 1920×1080, 29.48 fps and a maximum
116,683 µs presentation-time interval without saving images. A short packaged
native test passed CPU graph execution, preview, invalid-reference rejection and
zero unintended commits. Observed inference readings included 472–677 ms;
no physical board, accuracy or representative soak claim follows from that test.
The packaged recorder and library were visually inspected. Desktop testing found
and fixed speculative WebKit sockets blocking the single writer; request parsing
now runs independently with bounded connections and queues. The lifecycle handler now exits the service when its window is destroyed and explicitly shows/focuses it on macOS ready/reopen. The final build reports a visible window and runs its local service, but the desktop automation tool returned `cgWindowNotFound`; final close/reopen visual verification remains open.

F14, physical F02/F06–10 validation, complete physical export/import workflow qualification,
F19 candidate freezing against a validated configuration, F20–21 accuracy and
F22 clean-device/distribution checks remain open. The user confirmed that no
physical-game recordings exist yet. `dist/ChessCameraRecorder.app` and its
file-hash manifest are development artifacts, not a qualified release. Native
personal heads and measured CoreML adoption remain separate optional work.
No independent reviewer/subagents were used in this continuation.

Validation: the workspace suite, strict Clippy, existing browser regressions plus
seven library checks (35 JavaScript checks total), five Python preparation/original-decision regressions, 124 Rust tests, and
changed-JavaScript parsing pass. The isolated HTTP walkthrough covers slow/idle
connections, correction truncation, original-decision preservation, exports,
consistent backup, non-destructive restoration and restored-profile restart.
A native commit regression checks acquisition-time bounds, two durable supporting
frames and restart without reusing an old completion interval. Build scripts
create and verify the ad-hoc signed desktop bundle. Aggregate tokens and productive
time remain unavailable.


## Current target

| Item | Observed value |
| --- | --- |
| Hardware | MacBook Pro `Mac16,8`, Apple M4 Pro, 24 GB, arm64 |
| macOS | 15.3.1 (24D70) |
| Rust | rustc/cargo 1.99.0, project-local rustup installation |
| Camera | Device 0, `MacBook Pro Camera` |
| Negotiated mode | 1920×1080, 30 fps, YUYV |
| Timestamp source | Live Swift adapter: AVFoundation sample presentation times. Earlier Nokhwa probe: monotonic host receipt time. |
| 120-frame cadence run | 30.359 observed fps, 52.383 ms maximum inter-arrival, 0 intervals above the 83.333 ms gap threshold |
| RGB output check | One 1920×1080 P6 PPM decoded successfully; image contents were not added to Git |

Nokhwa 0.10.11 initially requested an unsupported 640×480@15 YUYV default and returned an empty compatibility list. The probe now asks AVFoundation for its highest-frame-rate advertised mode and labels the resulting active mode as `negotiated_fallback` when the compatibility list is empty. Decoding every full-resolution frame reduced the probe loop to about 3 fps, so clean cadence runs skip RGB conversion and decode only frames explicitly selected for saving.

## Frame handoffs

| Frame | Status | Completed in this checkpoint | Checks and evidence | Remaining exact action |
| --- | --- | --- | --- | --- |
| F01 — Toolchain and camera probe | needs continuation | Rust workspace/toolchain established; permission grant, device enumeration, active-format negotiation, bounded cadence capture, RGB decode, raw timing reports, offline evidence verification, probe documentation, and ad-hoc signed app packaging implemented | `local-data/camera-probe/run-002` captured 120 frames at 30.359 observed fps with 0 threshold gaps; all three retained runs pass `capture-probe verify`; plist, entitlement, signature, and the packaged binary's granted-permission path verified on 3 October | Test packaged-app permission denial then recovery and interruption/disconnect behavior on the target Mac. Native sample timestamps are now supplied by the Swift adapter; qualify interruption recovery on physical sessions. |
| F02 — Physical placement and feasibility | needs continuation | Evaluation manifest/schema, annotation procedure, and guided two-orientation collection script are ready | Manifest validator and capture evidence verifier have unit/integration tests; no board footage or accuracy figures were created | Run `scripts/collect-board-readiness.sh local-data/board-readiness/session-001` with a populated board, then record and annotate the scripted events in `docs/camera-probe.md` before the Phase 0 go/no-go decision. |
| F03 — Workspace and contracts | implementation complete; awaiting F02 prerequisite | Rust workspace, pinned dependencies, MIT license text, accepted decisions, validated observation/move/timing contracts, evaluation fixtures, and synthetic observation → decoder → SQLite → rules replay → PGN checkpoint exist | The synthetic pipeline closes and reopens a file-backed database successfully; durable event schema and the provisional timestamp boundary are documented | After F02 passes, freeze the final native/host timestamp and persisted calibration-session contracts. |
| F04 — Legal chess and notation | implementation complete; awaiting F02 prerequisite | `cozy-chess` adapter implements legal UCI transitions, SAN/FEN chains, atomic rebuilding, board piece classes, and explicit threefold/fivefold repetition policy | 23 tests cover checks/mate, every SAN disambiguation form, white/black castling, captures, en passant, every promotion choice, stalemate, malformed histories, halfmove-clock behavior, and repetition identity/counts | Independent reference corpus and parsing of fifteen exported histories now pass. Re-run the integrated gate after F02 establishes the supported setup. |
| F05 — Database journal and replay | implementation complete; awaiting F02 prerequisite | SQLite v4 migrations, rules-validated idempotent append, immutable correction revisions, append-only triggers, historical replay, and journal-versus-projection verification work | 13 storage tests cover restart, duplicates/conflicts, v1 migration, failed correction rollback, corrected suffixes, immutable history, replay, injected statement failures, and abrupt process termination during a WAL transaction | Re-run the integrated gate after F02 freezes the timestamp/session contracts. |
| F06 — Offline playback and board calibration | tooling implemented; physical gate pending | Native-frame extraction with source/image hashes, preserved acquisition clocks, browser playback and a calibrated overlay; projective geometry validates labeled corners | Geometry, orientation, media-selection/hash/timeline and replay regressions pass; public recorded-photo pipeline runs actual neural graphs | Collect independent F02 development/validation clips; measure real mapping, lens distortion and physical recognition failures. |
| F09 — Temporal move decoder | in progress (preparatory) | Synthetic state machine ranks unchanged plus legal successors, applies absolute/per-square/margin gates, requires full visibility for automatic moves, preserves conservative completion bounds, and latches on stream/session/model/calibration discontinuities | 8 tests cover a clear move, adjustment, hidden board, fully visible and partially hidden two-ply gaps, invalid thresholds, reset boundaries, and gap sequencing | Validate captures, compound moves, ambiguity, real visibility, gap recovery, and timing on sessions produced by F06–F08. Thresholds remain unqualified. |
| F15 — Desktop recording screen | packaged Tauri/native development candidate | Dependency-free interactive screen shows local camera preview, draggable four-corner projective calibration, geometry rejection/warnings, coarse motion state, changed squares, accepted/ambiguous decisions, timing/health placeholders, and trusted moves | Initial screen and live camera were visually inspected in Chrome; calibration JavaScript parses with the macOS JavaScript compiler and `git diff --check` passes | Browser now connects camera square changes to Rust rules, journaled moves, review, undo and PGN. Qualify the native pipeline and user workflow on physical sessions; the Tauri shell now integrates acquisition clocks, recognition and journaled evidence. |

The Phase 1 software implementation gate passes: deterministic replay, legal notation, transactional/idempotent commits, special moves, repetition policy, restart, and abrupt-termination recovery are covered. The planned phase sequence still awaits the Phase 0 physical prerequisite, so Phase 1 is recorded as implementation-complete rather than physically unblocked. No move-recognition accuracy claim is possible until independent real sessions exist.

## Earlier multi-agent waves

| Role | Scope | Result |
| --- | --- | --- |
| Camera probe agent | `crates/capture-probe`, probe guide | Implemented the bounded AVFoundation probe and hardware-free tests; live testing then exposed and fixed default-format, enumeration, and decode-cadence issues. |
| Evaluation tooling agent | `crates/evaluation`, evaluation guide | Implemented versioned manifests, cross-file session partition checks, media hashes, timestamps, calibration, UCI references, completion bounds, CLI validation, and 12 tests. |
| Platform reviewer | Architecture/dependency review | Confirmed that synthetic Phase 1 work can proceed while physical Phase 0 remains open; flagged camera entitlements, native timestamps, repetition handling, and Nokhwa isolation. |
| Storage revision agent | `crates/storage` | Added SQLite v3, append-only history, deterministic historical/journal replay, exact idempotency checks, and correction rollback coverage. |
| Chess special-move agent | `crates/chess-core` | Expanded to 21 rules/notation tests and documented repetition/50-move limitations. |
| Geometry agent | `crates/vision-geometry` | Added manual projective board mapping with seven synthetic validation/round-trip tests; no camera accuracy claim. |
| Independent Phase 1 reviewer | Read-only contracts/rules/export/decoder review | Found unsafe hidden-gap/session behavior, unvalidated decoder thresholds, weak export validation, custom-FEN numbering, tag injection, and projection overflow; fixes and regressions were added. |
| Phase 1 completion wave | Rules, storage fault injection, camera evidence readiness, decisions | Added explicit repetition claims, abrupt-termination recovery coverage, offline camera evidence verification, a guided board collection script, the MIT license text, and the Phase 1 decision record. |

## Current recognition and evaluation progress

| Frame | Status | Remaining gate |
| --- | --- | --- |
| F07 — Observations | Browser motion and neural evidence implemented | Independently measure square mapping, occupancy/color and occlusion failures on the supported board. |
| F08 — Pretrained models and adaptation | Browser graphs/personal heads plus native CPU base-model inference and reference parity implemented | Independent side-camera validation and model selection; native personal heads, camera resampling and CoreML are unqualified. |
| F10 — Timing, gaps and combined validation | Development replay/scoring tooling implemented | Combined physical move/timing validation, threshold freezing and later original live automatic-decision audit. |
| F15 — Recording screen | Packaged Tauri shell with native and browser recording paths | Physical workflow and representative soak validation. |
| F11–13 — Native timing, processing and evidence | Acquisition clocks, bounded queues, gaps and durable sampled evidence implemented | Annotated timing checks, sustained load and evidence-loss recovery on real sessions. |
| F16–18 — Library, corrections and exports | Library/replay, immutable correction, sidecars and consistent backup/restore implemented | Complete physical game walkthrough and user-facing workflow qualification. |
| F19–22 — Qualification and release | Development candidate hash manifest and original-decision audit tooling ready | Freeze a physically validated configuration, score held-out games and test distribution on a clean Mac. |

The latest continuation used one coordinator with the inherited runtime model;
no new helper agents were launched. Productive time/token totals and an independent
review of this continuation are unavailable. Physical phases remain open.

## Next coordinator action

Run `scripts/collect-board-readiness.sh local-data/board-readiness/session-001` with the populated board, then capture the scripted F02 moves, annotate separate development/validation sessions, and decide Phase 0 feasibility. Also test F01 packaged-app denial/recovery and interruption behavior. Those recordings can now feed the implemented [offline replay and scoring workflow](offline-evaluation.md). Keep complete training/development and validation sessions separate. Measure side-camera errors before changing or freezing recognition thresholds. Native CPU base-model parity is now checked. Desktop integration with native acquisition clocks and bounded processing now exists. The next prerequisite is physical feasibility, followed by independent side-camera validation/model selection and a representative sixty-minute soak.

The 3 October Phase 1 completion reran formatting, the full offline workspace suite (**87 tests**), strict Clippy, the synthetic demo, and `git diff --check` with the project-local toolchain. The app bundle's code signature and granted camera-permission path also pass. Nokhwa's transitive `block 0.1.6` still emits a future-Rust incompatibility warning; keeping capture behind its adapter remains necessary.


## 3 October — Make the current recording app functional

The visible app previously compared coarse whole-frame motion only. There was
no camera move recognizer, rules API, or storage connection; demo buttons were
the only path to its in-memory move list. The preview also mirrored the camera
while calibration coordinates did not.

Added a loopback Rust recorder server (`cargo run -- serve`, or
`scripts/start-recorder.sh`) and connected the browser to legal successor
positions, canonical SAN, SQLite commits, reload/resume, audited undo, new games
and PGN export. The camera now uses an unmirrored, aspect-matched preview and
samples 64 calibrated square interiors. A user-confirmed reference, exposure
compensation, localized motion gating, 900 ms settling and three consistent
changed-square observations drive legal-move suggestions. Review is the default;
automatic recording is opt-in and experimental. Camera interruptions, hidden
tabs and calibration/sensitivity changes require a fresh reference. Manual
recording and a tracked board provide recovery when recognition abstains.

This is an F15/F07 classical functional baseline, not the production temporal
piece-evidence decoder or an F02/F08 recognition-accuracy gate. Starting piece
identities and visibility are user-confirmed; no learned piece model or physical
accuracy measurement was added. Completion timings remain unknown.

Validation: all 90 Rust workspace tests and strict offline Clippy pass. Eight
JavaScript pixel/temporal regressions cover a real pixel e2–e4 change, exposure
shifts, settling, duplicate prevention, hands, adjustments, illegal patterns,
compound moves, promotion ambiguity and reset. Live browser checks on an isolated database recorded e4, persisted it across a
page reload, recorded e5, and undid e5. HTTP checks verified canonical PGN,
file-backed resume after restarting the server, rejection of private filesystem
routes, and rejection of cross-origin writes. No browser console warnings or
errors appeared. Visual inspection caught and corrected unequal tracked-board
row heights. The user recorder is left running on localhost:8770 with its own
untouched game database; the old static preview on port 8765 has no recorder API.
The physical camera/move workflow still needs user-board validation.


## 3 October — Neural camera move recognition

Installed two pretrained MobileNetV3-Small physical-photo classifiers from
`cstr/chess-board-photo-onnx`, pinned at revision
`4690fd5418ff7c405a76d1c156077ecd865ac053`, plus verified ONNX Runtime Web 1.23.2.
Their hashes, input shapes, class order and provenance are recorded in
`models/vision-manifest.json` and `models/NOTICE.md`. The setup script checks
hashes and installs immutable assets in ignored local storage; startup is
offline after installation.

Added image-order projective warping and the published chesscog-compatible
occupancy/identity crops, asymmetric height/width margins, left-half mirroring,
black padding, RGB ImageNet normalization and explicit model-to-contract class
mapping. A browser module worker executes the actual neural graphs off the UI
thread. Camera images remain in the browser; normalized 64-square evidence
reaches the existing Rust temporal/legal-position decoder through local API
routes. No piece type, king location or engine preference is forced into model
outputs.

The default UI now selects piece recognition, reports graph readiness, provides
an independently recognized board readout, and verifies the tracked reference
before arming. Three consistent neural successor observations are required.
Automatic commits require a current supported proposal and matching session;
contradictory evidence or motion during review withdraws the uncommitted
proposal. Manual moves/corrections and capture discontinuities invalidate the
neural session. The pixel-change path remains an explicit review-only fallback.

Checks: all **96 Rust workspace tests**, strict Clippy, **8 neural preprocessing
regressions**, and the existing **8 pixel/temporal regressions** pass. Neural API
integration tests cover actual automatic commit authorization, save/next-turn
continuity, wrong setup, malformed probabilities, weak evidence, stale
proposals, motion/contradictions during review, hidden two-move gaps and manual
correction recovery. Browser execution of both ONNX graphs succeeded; the
licensed real-photo fixture matched **64/64 squares** with no uncertain squares
at **225–237 ms** per board in the two checks. Sending that photo's actual model
evidence to the Rust reference endpoint correctly rejected it as a standard
starting-position reference. Main app startup shows **Piece model ready**, the
existing game is preserved, and browser console checks are clean. The test
server/data are isolated; the main recorder is running on localhost:8770.

This advances F07/F08/F09/F15 integration, but does not pass physical accuracy
gates. The fixture is from the model author's corpus and is a preprocessing /
runtime regression, not an independent evaluation on the user's board.
Weights are pretrained and have not been fine-tuned on the user's piece set.
Crop coverage and model certainty are evidence-quality proxies, not a learned
occlusion detector or calibrated accuracy estimates. Independent F02 recordings,
real-board move sequences and timing qualification remain outstanding.

## Side-camera diagnostics and local piece-set adaptation (2026-10-03)

User reported frequent identity errors with a side camera and authorized continued
project work while away. The pretrained photo classifier had only been checked
on a largely overhead public fixture, so the earlier result did not qualify the
user's setup. The current work adds an actionable adaptation path without claiming
an out-of-the-box side-view accuracy improvement.

- Added per-square crop inspection after a camera read: occupancy footprint,
  asymmetric identity crop, top-three scores and in-frame crop coverage. Shallow
  projection and low-resolution warnings help distinguish camera/crop problems
  from piece-classification problems. These warnings do not detect occlusion.
- Exposed the existing frozen 1024-dimensional penultimate activation in each
  pinned MIT MobileNet ONNX graph. Original graph weights/logits are unchanged;
  original downloads and transformed files have separately pinned hashes. The
  normal installer remains Python-standard-library-only. ONNX 1.19.1's checker
  validated the derivative graphs during development.
- Added **Teach recognition your pieces**: explicit confirmation of the physical
  position, stable-frame capture of neural features and labels, up to 20 examples,
  and regularized class-balanced softmax-head training entirely in the browser
  worker. Neither camera images nor crop images persist. Examples and heads are
  saved in IndexedDB for the local browser origin.
- Training requires at least three distinct piece placements, a fixed camera
  calibration and two visible examples of every piece class in the learning split.
  Every frame of the last position is held out for a base/personal comparison;
  only afterward are final heads fitted on all examples. The small validation
  score is explicitly not a general accuracy estimate.
- Personal heads use a content digest, exact base-model identity, labelled corners
  and processed frame dimensions. Saved head integrity and finite shapes are
  checked. Changed crop geometry uses the base classifier and explains why.
  Physical camera movement without changed settings remains undetectable and
  requires new examples. Personal proposals require reviewed commits in both
  UI and Rust; a valid proposal token cannot authorize automatic personal moves.

Validation:

- 98 Rust workspace tests pass, including personal-model identity changes,
  rejection of automatic personal commits and successful reviewed commits.
  Clippy passes with warnings denied.
- 23 JavaScript tests pass: 10 preprocessing/geometry, 5 personal-head training
  and leakage/invalid-data regressions, and 8 recorder motion/move regressions.
- Browser execution of the modified graphs still recognizes 64/64 squares of the
  existing licensed public photo. Ordinary inference measured 230 ms; inspecting
  all 64 identity crops/features measured 449 ms. Finite 1024-dimensional features,
  all 64 crop previews and typed feature persistence/restoration were verified.
- The isolated browser personal-head integration check passed: synthetic-feature
  training (645 ms for three frames), validation, save/restore, content-integrity
  rejection, activation, changed-calibration deactivation and base restoration.
  These synthetic checks establish plumbing, not camera recognition accuracy.
- Verified installer migration from the exact original source hashes to the
  derived graph hashes using isolated temporary assets and no network. Checked
  Python syntax and whitespace. A 20-frame synthetic feature training benchmark
  completed in 4.1 seconds in JavaScriptCore.

The user's side-view camera was unavailable during this work. Their physical
accuracy, crop quality, held-out performance and complete-game reliability remain
unqualified. Next: collect confirmed examples with the camera fixed, inspect
occluded/misidentified crops, compare the personal head on an independent position,
and record independent games for F02 qualification. Full backbone fine-tuning,
learned occlusion detection and a second camera remain possible subsequent work.

## 3 October — Fail closed after rejected neural observations

An invalid observation after a model proposal could leave its approval token in
the server session. The recorder now discards that session when observation
validation or stream decoding fails, so a sequence gap cannot be followed by an
automatic commit using the earlier proposal. A server regression covers the
proposal, rejected gap, stale commit and unchanged journal. The full Rust
workspace suite, strict Clippy, formatting, all 23 JavaScript regressions and
`git diff --check` pass with the local toolchain. Physical-board qualification
remains the next gate; no side-view accuracy result was measured in this run.

## 4 October — Recorded-media replay and blind decoder scoring

Reviewed the current implementation and plan. Rules, journaled saving, review,
undo, PGN, browser neural recognition and local piece-set adaptation already
exist. The next missing preparation step was reproducible playback and scoring
of annotated camera recordings; physical F02/side-camera accuracy and Rust model
parity remain open.

Added `scripts/prepare-evaluation.py`: Rust manifest validation, local source
hash verification, bounded FFmpeg native-frame extraction, presentation-timeline
checks, unchanged acquisition timestamps and staged publication of lossless
hash-checked frames. The browser's new offline evaluation page imports those
folders, overlays calibrated squares, inspects frames, replays the actual base
neural graphs, measures motion, and exports complete raw observation traces. It
never writes saved games or installs personal classifier heads.

Added `observation-replay` and a strict trace/configuration contract. Rust decodes
without reference moves, commits its own legal proposals including mistakes, and
scores strict matching prefixes, incorrect/extra/missing moves, capture gaps and
completion-interval containment/overlap. It preserves wrong branches rather than
repairing them from annotations, reports null rates for empty denominators, and
refuses qualification data. The report records full configuration and separates
acquisition clocks from inference cost. This is development tooling, not the
original-live-decision qualification audit.

Validation: **108 Rust workspace tests**, strict offline Clippy, **28 JavaScript
regressions**, **3 real FFmpeg preparation tests**, formatting and diff whitespace
checks pass. New regressions cover native indexes, all corner orientations,
source/frame hashes, timestamp corruption, inference backlogs, stream gaps,
wrong legal histories, early commits, completion-interval comparisons, castling,
en passant, all four promotions, and qualification rejection. FFmpeg-selected
images match an independent full decode of a generated video byte for byte.

In Chrome, a ten-frame recording of the licensed public still executed the real
models and exported all ten observations. Cancellation disabled partial export;
restarting completed replay, and Previous/Next inspection worked. Rust scoring
retained capture times 0–900000 µs, emitted no moves or gaps and preserved the
initial position. Precision/coverage were correctly null because this fixture
contains no moves. Inference P95 was **203 ms** for this run; repeated scoring
reports were byte-identical. This model-author fixture is a plumbing regression,
not an independent physical side-camera accuracy result. Browser console checks
were clean and the replay result was visually inspected.

Automated folder selection is blocked by the Chrome extension's disabled file-URL
permission; it was not changed. Browser automation also stalled intermittently.
The test-only loopback server served a fixed public fixture through the same
import/validation path. Download event observation timed out, so the complete
export was verified through the app's inspect/copy control instead. Normal folder
picker and download behavior still need a manual browser check. Test data and
fixture routes are isolated from the main recorder; recorded camera directories
are not exposed by the production server. HTTP checks confirmed the new page/assets return 200 while test-fixture and local recording paths return 404 on the main server. The updated recorder is running on localhost:8770 with its existing database. See [the workflow guide](offline-evaluation.md).

F06/F10 tooling is now reviewable; their physical phase gates remain pending.
Next: independent side-camera development/validation recordings and mapping/move
measurements, then model/decoder selection and threshold freezing. Rust inference
parity can proceed independently; Tauri packaging remains later work.

## 4 October — Native CPU base-model inference and parity (F08 preparation)

Continued the independent software milestone after offline replay/scoring.
Added `crates/vision-inference`: native CPU graph execution through pinned
`ort` 2.0.0-rc.10, owned/bounded RGBA frames, image-order geometry and all four
algebraic orientations, exact crop/padding/mirroring/normalization, graph feature
validation, production occupancy-based piece selection, normalized contract
square evidence and the existing side-view quality diagnostics. Inference cost
is reported separately; native callers must supply acquisition timestamps.
The current live browser recorder continues to use its browser worker.

Pinned the official Microsoft ONNX Runtime 1.23.2 macOS arm64 release archive,
library and license hashes in `models/native-runtime.json`. The dedicated
installer extracts only verified named files. The Rust adapter verifies the
library and both existing derived-model hashes before loading. Automatic binary
downloads at Rust build time are disabled. Runtime/model assets remain local and
ignored; licensing/provenance notices were extended. No other platform, CoreML
provider or native personal-head execution is qualified.

Added `vision-infer` and `vision-preprocess` CLIs plus the reproducible
`scripts/check-native-vision.py` reference checker. JavaScriptCore runs the actual
`vision-core.js` on the same decoded RGBA bytes; SHA-256 comparisons cover every
RGB crop and NCHW f32 tensor. ONNX 1.19.1's independent ReferenceEvaluator with
NumPy 2.0.2 checks native logits, 1024-dimensional features, probabilities, top
classes, square evidence/visibility, occupancy selection and tail batches.
Calibration JSON parsing uses exact float round trips; a high-precision case
checks that port boundary. All thresholds/tolerances are recorded rather than
chosen after observing failures.

Validation: **116 Rust workspace tests**, strict offline workspace Clippy,
formatting, Python script compilation, runtime asset verification and diff
whitespace checks pass. Eight new Rust regressions cover malformed frames and
corners, orientation/reflection, padding/coverage, normalization/layout, class
mapping/weak visibility, invalid probabilities/logits, calibration float parsing,
and changed/truncated assets. The reference check passes **nine cases**: four
rotations, skew, precise calibration, image-edge crops, shallow geometry and the
licensed public photo. The current report is
`local-data/vision/native-parity-003/report.json`; intermediate checks and artifacts
are ignored local development evidence. Crops/tensors/coverage/geometry/quality
match exactly. Maximum observed numerical errors and photo inference cost are
recorded in that report; accepted bounds are 0.0001 absolute/relative for
logits/features and 0.00001 absolute for probabilities/evidence.

The public model-author photo still matches **64/64** labeled squares in normal
selected-crop inference. Synthetic cases and this corpus fixture establish code
and graph consistency, not independent side-camera accuracy. JPEG/ICC/Canvas
scaling and native camera resampling are outside the already-decoded RGBA parity
boundary. Browser WASM logits were not individually compared in this checkpoint;
its existing execution/fixture smoke check remains separate evidence. The new
native adapter does not yet drive a native live camera, game session or desktop
window. See [the native recognition guide](native-vision.md).

F08 base-model CPU parity tooling is ready for desktop integration. Phase 2 still
requires supported-board development/validation recordings, observation/model
comparison and a provisional pipeline decision. Next independent software work:
connect native capture and inference with explicit acquisition clocks, bounded
frame handling and interruption states, then expose the recorder in the Tauri
shell. No physical phase gate was marked passed and no new helper agents were
launched. Productive time and aggregate token totals remain unavailable.
