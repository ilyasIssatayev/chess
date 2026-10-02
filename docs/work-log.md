# Development work log

Status recorded on 2 October 2026. Productive time and aggregate token usage are unavailable from the current client, so they are recorded as unknown rather than estimated. The lead and helpers used the current Codex runtime; spawned helpers inherited the lead model because no override was selected.

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
| F01 — Toolchain and camera probe | needs continuation | Rust workspace/toolchain established; permission grant, device enumeration, active-format negotiation, bounded cadence capture, RGB decode, raw timing reports, and probe documentation implemented | `local-data/camera-probe/run-002` captured 120 frames at 30.359 observed fps with 0 threshold gaps; `run-003` produced a valid-size PPM | Package the probe as an app, test denial then recovery, and test interruption/disconnect behavior. Replace or supplement Nokhwa if native sample timestamps are required. |
| F02 — Physical placement and feasibility | needs continuation | Evaluation manifest/schema and annotation procedure are ready | Manifest validator has unit/integration tests; no board footage or accuracy figures were created | Put a populated board in view, record both orientations and the scripted events in `docs/camera-probe.md`, annotate separate development/validation sessions, then make the Phase 0 go/no-go decision. |
| F03 — Workspace and contracts | in progress (preparatory) | Rust workspace, pinned dependencies, MIT license, shared capture/observation/move/timing/review types, evaluation fixture format, and synthetic decoder evidence exist | Workspace builds; contract and manifest tests pass | After F02 passes, freeze capture timestamp and calibration contracts, then add one synthetic observation-to-database integration fixture. |
| F04 — Legal chess and notation | in progress (preparatory) | `cozy-chess` adapter implements standard position, legal UCI transitions, SAN/FEN chains, atomic history rebuilding, and board piece classes | Unit coverage includes ordinary moves, illegal moves, castling input, and atomic rebuild | Complete independent fixtures for check/mate, SAN disambiguation, both castles, en passant, all promotions, and repeated positions. |
| F05 — Database journal and replay | in progress (preparatory) | SQLite v1 schema, foreign keys, WAL/FULL sync, transactional idempotent event/move append, active-revision and FEN/ply checks, move reload, and restart tests exist | Storage unit tests and root manual demo pass | Finish immutable correction revisions and deterministic rules-based replay validation, including interrupted/duplicate event cases. |
| F09 — Temporal move decoder | in progress (preparatory) | Synthetic state machine ranks unchanged plus every legal successor, gates on settling/visibility/score/margin/consistent frames, and waits for explicit commit | Synthetic tests accept a clear `e2e4`, reject a piece adjustment, and send hidden evidence to review | Validate captures, compound moves, adjustments, ambiguity, gaps, and timing on real sessions after F06–F08 produce calibrated observations. |

Preparatory work on dependent frames does not pass their phase gates. Phase 0 still depends on a person arranging the physical board and performing the scripted moves. No move-recognition accuracy claim is possible until independent real sessions exist.

## Current multi-agent wave

| Role | Scope | Result |
| --- | --- | --- |
| Camera probe agent | `crates/capture-probe`, probe guide | Implemented the bounded AVFoundation probe and hardware-free tests; live testing then exposed and fixed default-format, enumeration, and decode-cadence issues. |
| Evaluation tooling agent | `crates/evaluation`, evaluation guide | Implemented versioned manifests, cross-file session partition checks, media hashes, timestamps, calibration, UCI references, completion bounds, CLI validation, and 12 tests. |
| Platform reviewer | Architecture/dependency review | Confirmed that synthetic Phase 1 work can proceed while physical Phase 0 remains open; flagged camera entitlements, native timestamps, repetition handling, and Nokhwa isolation. |
| Storage revision agent | `crates/storage` | In progress at this handoff. |
| Chess special-move agent | `crates/chess-core` | In progress at this handoff. |
| Independent Phase 1 reviewer | Read-only contracts/rules/export/decoder review | In progress at this handoff. |

## Next coordinator action

Integrate the active storage/chess/review reports, run formatting, Clippy with warnings denied, and the full workspace test suite. Then add the synthetic observation-to-decoder-to-storage-to-PGN integration checkpoint while waiting for the physical board recording required by F02.
