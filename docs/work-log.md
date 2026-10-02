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
| F01 — Toolchain and camera probe | needs continuation | Rust workspace/toolchain established; permission grant, device enumeration, active-format negotiation, bounded cadence capture, RGB decode, raw timing reports, probe documentation, and ad-hoc signed app packaging implemented | `local-data/camera-probe/run-002` captured 120 frames at 30.359 observed fps with 0 threshold gaps; `run-003` produced a valid-size PPM; the bundled plist, entitlement, and code signature verified on 3 October | Test packaged-app permission denial then recovery and interruption/disconnect behavior on the target Mac. Replace or supplement Nokhwa if native sample timestamps are required. |
| F02 — Physical placement and feasibility | needs continuation | Evaluation manifest/schema and annotation procedure are ready | Manifest validator has unit/integration tests; no board footage or accuracy figures were created | Put a populated board in view, record both orientations and the scripted events in `docs/camera-probe.md`, annotate separate development/validation sessions, then make the Phase 0 go/no-go decision. |
| F03 — Workspace and contracts | in progress (preparatory) | Rust workspace, pinned dependencies, MIT license, shared types, evaluation fixture format, and synthetic observation → decoder → SQLite → replay → PGN checkpoint exist | Workspace tests and demo pass | After F02 passes, freeze capture timestamp and calibration contracts. |
| F04 — Legal chess and notation | in progress (preparatory) | `cozy-chess` adapter implements legal UCI transitions, SAN/FEN chains, atomic history rebuilding, and board piece classes | Fixtures now cover checkmate, disambiguation, both castles, captures, en passant, all promotions, and repeated positions | Validate the fixtures with an independent chess reference and complete Phase 1 after F02. |
| F05 — Database journal and replay | in progress (preparatory) | SQLite v2 migration, rules-validated idempotent move append, immutable correction revisions, active replay, and original revision history now work | Restart, v1 migration, failed correction rollback, replay, workspace tests and Clippy pass | Add fault-injected interrupted-commit checks and an independent replay reference before the Phase 1 gate. |
| F09 — Temporal move decoder | in progress (preparatory) | Synthetic state machine ranks unchanged plus every legal successor, gates on settling/visibility/score/margin/consistent frames, and waits for explicit commit | Synthetic tests accept a clear `e2e4`, reject a piece adjustment, and send hidden evidence to review | Validate captures, compound moves, adjustments, ambiguity, gaps, and timing on real sessions after F06–F08 produce calibrated observations. |

Preparatory work on dependent frames does not pass their phase gates. Phase 0 still depends on a person arranging the physical board and performing the scripted moves. No move-recognition accuracy claim is possible until independent real sessions exist.

## Current multi-agent wave

| Role | Scope | Result |
| --- | --- | --- |
| Camera probe agent | `crates/capture-probe`, probe guide | Implemented the bounded AVFoundation probe and hardware-free tests; live testing then exposed and fixed default-format, enumeration, and decode-cadence issues. |
| Evaluation tooling agent | `crates/evaluation`, evaluation guide | Implemented versioned manifests, cross-file session partition checks, media hashes, timestamps, calibration, UCI references, completion bounds, CLI validation, and 12 tests. |
| Platform reviewer | Architecture/dependency review | Confirmed that synthetic Phase 1 work can proceed while physical Phase 0 remains open; flagged camera entitlements, native timestamps, repetition handling, and Nokhwa isolation. |
| Storage revision work | `crates/storage` | Integrated during the 3 October continuation; SQLite v2 and correction/replay checks pass. |
| Chess special-move work | `crates/chess-core` | Fixtures were present at the start of the 3 October continuation and pass. |
| Independent Phase 1 reviewer | Read-only contracts/rules/export/decoder review | Prior handoff listed this as in progress; no report is available in this checkout. |

## Next coordinator action

Test F01 packaged-app permission denial/recovery and interruption behavior on the target Mac. Arrange the populated board and scripted F02 recordings in both orientations, then annotate separate sessions and decide Phase 0 feasibility. Continue Phase 1 only as preparatory work until that physical gate passes. The 3 October continuation packaged and verified the ad-hoc signed probe and ran `cargo test --workspace --offline`, `cargo clippy --workspace --all-targets --offline -- -D warnings`, and `cargo run --offline -- demo` with the project-local toolchain; all passed after storage integration. The default Cargo cache could not resolve crates.io, so verification used the repository's ignored `.toolchain` cache.
