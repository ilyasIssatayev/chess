# Development work log

Status updated on 3 October 2026. Productive time and aggregate token usage are unavailable from the current client, so they are recorded as unknown rather than estimated. The initial wave used the current Codex runtime; helpers inherited the lead model because no override was selected.

Local camera artifacts are stored below ignored `local-data/` and are intentionally absent from Git. This log omits the machine serial number, hardware UUID, and camera images.

## Current target

| Item | Observed value |
| --- | --- |
| Hardware | MacBook Pro `Mac16,8`, Apple M4 Pro, 24 GB, arm64 |
| macOS | 15.3.1 (24D70) |
| Rust | rustc/cargo 1.99.0, project-local rustup installation |
| Camera | Device 0, `MacBook Pro Camera` |
| Negotiated mode | 1920×1080, 30 fps, YUYV |
| Timestamp source | Process monotonic receipt time after blocking capture; native AVFoundation presentation timestamps are unavailable through the current adapter |
| 120-frame cadence run | 30.359 observed fps, 52.383 ms maximum inter-arrival, 0 intervals above the 83.333 ms gap threshold |
| RGB output check | One 1920×1080 P6 PPM decoded successfully; image contents were not added to Git |

Nokhwa 0.10.11 initially requested an unsupported 640×480@15 YUYV default and returned an empty compatibility list. The probe now asks AVFoundation for its highest-frame-rate advertised mode and labels the resulting active mode as `negotiated_fallback` when the compatibility list is empty. Decoding every full-resolution frame reduced the probe loop to about 3 fps, so clean cadence runs skip RGB conversion and decode only frames explicitly selected for saving.

## Frame handoffs

| Frame | Status | Completed in this checkpoint | Checks and evidence | Remaining exact action |
| --- | --- | --- | --- | --- |
| F01 — Toolchain and camera probe | needs continuation | Rust workspace/toolchain established; permission grant, device enumeration, active-format negotiation, bounded cadence capture, RGB decode, raw timing reports, offline evidence verification, probe documentation, and ad-hoc signed app packaging implemented | `local-data/camera-probe/run-002` captured 120 frames at 30.359 observed fps with 0 threshold gaps; all three retained runs pass `capture-probe verify`; plist, entitlement, signature, and the packaged binary's granted-permission path verified on 3 October | Test packaged-app permission denial then recovery and interruption/disconnect behavior on the target Mac. Replace or supplement Nokhwa if native sample timestamps are required. |
| F02 — Physical placement and feasibility | needs continuation | Evaluation manifest/schema, annotation procedure, and guided two-orientation collection script are ready | Manifest validator and capture evidence verifier have unit/integration tests; no board footage or accuracy figures were created | Run `scripts/collect-board-readiness.sh local-data/board-readiness/session-001` with a populated board, then record and annotate the scripted events in `docs/camera-probe.md` before the Phase 0 go/no-go decision. |
| F03 — Workspace and contracts | implementation complete; awaiting F02 prerequisite | Rust workspace, pinned dependencies, MIT license text, accepted decisions, validated observation/move/timing contracts, evaluation fixtures, and synthetic observation → decoder → SQLite → rules replay → PGN checkpoint exist | The synthetic pipeline closes and reopens a file-backed database successfully; durable event schema and the provisional timestamp boundary are documented | After F02 passes, freeze the final native/host timestamp and persisted calibration-session contracts. |
| F04 — Legal chess and notation | implementation complete; awaiting F02 prerequisite | `cozy-chess` adapter implements legal UCI transitions, SAN/FEN chains, atomic rebuilding, board piece classes, and explicit threefold/fivefold repetition policy | 23 tests cover checks/mate, every SAN disambiguation form, white/black castling, captures, en passant, every promotion choice, stalemate, malformed histories, halfmove-clock behavior, and repetition identity/counts | Add an independent cross-implementation reference corpus as defense in depth; it is not required by the written Phase 1 gate. |
| F05 — Database journal and replay | implementation complete; awaiting F02 prerequisite | SQLite v3 migrations, rules-validated idempotent append, immutable correction revisions, append-only triggers, historical replay, and journal-versus-projection verification work | 13 storage tests cover restart, duplicates/conflicts, v1 migration, failed correction rollback, corrected suffixes, immutable history, replay, injected statement failures, and abrupt process termination during a WAL transaction | Re-run the integrated gate after F02 freezes the timestamp/session contracts. |
| F06 — Offline playback and board calibration | in progress (preparatory) | `vision-geometry` validates manually labeled corners and maps board/image points plus square centers/polygons through an invertible homography | 7 synthetic tests cover skew, both labeled orientations, invalid polygons/inputs, round trips, and projection range | Build deterministic media playback and a calibration overlay; measure real corner reprojection and lens distortion using separate F02 recordings. |
| F09 — Temporal move decoder | in progress (preparatory) | Synthetic state machine ranks unchanged plus legal successors, applies absolute/per-square/margin gates, requires full visibility for automatic moves, preserves conservative completion bounds, and latches on stream/session/model/calibration discontinuities | 8 tests cover a clear move, adjustment, hidden board, fully visible and partially hidden two-ply gaps, invalid thresholds, reset boundaries, and gap sequencing | Validate captures, compound moves, ambiguity, real visibility, gap recovery, and timing on sessions produced by F06–F08. Thresholds remain unqualified. |
| F15 — Desktop recording screen | functional browser baseline; Tauri pending | Dependency-free interactive screen shows local camera preview, draggable four-corner projective calibration, geometry rejection/warnings, coarse motion state, changed squares, accepted/ambiguous decisions, timing/health placeholders, and trusted moves | Initial screen and live camera were visually inspected in Chrome; calibration JavaScript parses with the macOS JavaScript compiler and `git diff --check` passes | Browser now connects camera square changes to Rust rules, journaled moves, review, undo and PGN. Qualify the change detector on physical sessions and integrate piece evidence and versioned events in the Tauri shell. |

The Phase 1 software implementation gate passes: deterministic replay, legal notation, transactional/idempotent commits, special moves, repetition policy, restart, and abrupt-termination recovery are covered. The planned phase sequence still awaits the Phase 0 physical prerequisite, so Phase 1 is recorded as implementation-complete rather than physically unblocked. No move-recognition accuracy claim is possible until independent real sessions exist.

## Current multi-agent wave

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

## Next coordinator action

Run `scripts/collect-board-readiness.sh local-data/board-readiness/session-001` with the populated board, then capture the scripted F02 moves, annotate separate development/validation sessions, and decide Phase 0 feasibility. Also test F01 packaged-app denial/recovery and interruption behavior. Those recordings then feed deterministic playback, real calibration overlays, and observation/model evaluation.

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
