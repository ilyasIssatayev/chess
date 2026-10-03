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
| F01 — Toolchain and camera probe | needs continuation | Rust workspace/toolchain established; permission grant, device enumeration, active-format negotiation, bounded cadence capture, RGB decode, raw timing reports, probe documentation, and ad-hoc signed app packaging implemented | `local-data/camera-probe/run-002` captured 120 frames at 30.359 observed fps with 0 threshold gaps; `run-003` produced a valid-size PPM; plist, entitlement, signature, and the packaged binary's granted-permission path verified on 3 October | Test packaged-app permission denial then recovery and interruption/disconnect behavior on the target Mac. Replace or supplement Nokhwa if native sample timestamps are required. |
| F02 — Physical placement and feasibility | needs continuation | Evaluation manifest/schema and annotation procedure are ready | Manifest validator has unit/integration tests; no board footage or accuracy figures were created | Put a populated board in view, record both orientations and the scripted events in `docs/camera-probe.md`, annotate separate development/validation sessions, then make the Phase 0 go/no-go decision. |
| F03 — Workspace and contracts | in progress (preparatory) | Rust workspace, pinned dependencies, MIT license, validated observation/move/timing contracts, evaluation fixtures, and synthetic observation → decoder → SQLite → rules replay → PGN checkpoint exist | The synthetic pipeline also closes and reopens a file-backed database successfully | After F02 passes, freeze the native timestamp and persisted calibration-session contracts. |
| F04 — Legal chess and notation | in progress (preparatory) | `cozy-chess` adapter implements legal UCI transitions, SAN/FEN chains, atomic rebuilding, and board piece classes | 21 tests cover checks/mate, file/rank/full-square disambiguation, white/black castling, captures, en passant, white/black and capture promotions, stalemate, malformed histories, halfmove-clock behavior, and the known repetition boundary | Compare fixtures with an independent PGN/rules implementation; implement product-level repetition/claim policy before the Phase 1 gate. |
| F05 — Database journal and replay | in progress (preparatory) | SQLite v3 migrations, rules-validated idempotent append, immutable correction revisions, append-only triggers, historical replay, and journal-versus-projection verification work | 10 storage tests cover restart, duplicates/conflicts, v1 migration, failed correction rollback, corrected suffixes, immutable history, and replay | Add process-kill fault injection around commits and compare replay with an independent reference before the Phase 1 gate. |
| F06 — Offline playback and board calibration | in progress (preparatory) | `vision-geometry` validates manually labeled corners and maps board/image points plus square centers/polygons through an invertible homography | 7 synthetic tests cover skew, both labeled orientations, invalid polygons/inputs, round trips, and projection range | Build deterministic media playback and a calibration overlay; measure real corner reprojection and lens distortion using separate F02 recordings. |
| F09 — Temporal move decoder | in progress (preparatory) | Synthetic state machine ranks unchanged plus legal successors, applies absolute/per-square/margin gates, requires full visibility for automatic moves, preserves conservative completion bounds, and latches on stream/session/model/calibration discontinuities | 8 tests cover a clear move, adjustment, hidden board, fully visible and partially hidden two-ply gaps, invalid thresholds, reset boundaries, and gap sequencing | Validate captures, compound moves, ambiguity, real visibility, gap recovery, and timing on sessions produced by F06–F08. Thresholds remain unqualified. |

Preparatory work on dependent frames does not pass their phase gates. Phase 0 still depends on a person arranging the physical board and performing the scripted moves. No move-recognition accuracy claim is possible until independent real sessions exist.

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

## Next coordinator action

Test F01 packaged-app denial/recovery and interruption behavior. Arrange the populated board and scripted F02 recordings in both orientations, annotate separate sessions, and decide Phase 0 feasibility. Those recordings then feed deterministic playback, real calibration overlays, and observation/model evaluation. Continue later frames only as preparatory work until the physical gate passes.

The 3 October continuation ran `cargo fmt --all -- --check`, `cargo test --workspace --offline` (**79 tests**), `cargo clippy --workspace --all-targets --offline -- -D warnings`, and `cargo run --offline -- demo` with the project-local toolchain. The app bundle's code signature and granted camera-permission path also pass. Nokhwa's transitive `block 0.1.6` still emits a future-Rust incompatibility warning; keeping capture behind its adapter remains necessary.
