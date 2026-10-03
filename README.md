# Chess camera recorder

A local-first macOS application in progress. It uses the built-in MacBook camera to observe a physical chessboard, infer legal moves conservatively, preserve elapsed-time uncertainty, store games in SQLite, and export standard PGN for Chess.com analysis.

The implementation currently provides:

- an AVFoundation camera feasibility probe with permission, device, format, frame, and cadence reporting;
- shared observation, move, timing, evidence, and review contracts;
- legal UCI move application with canonical SAN/FEN through `cozy-chess`;
- a conservative temporal decoder that compares unchanged and legal-successor positions;
- validated four-corner projective geometry for manual board calibration;
- transactional SQLite game/event/move storage;
- PGN export with optional descriptive timing comments; and
- session-separated evaluation manifests with leakage and annotation validation.

The built-in camera has been exercised on the target Mac, but physical chessboard placement and end-to-end vision accuracy have not been qualified. The recorder must abstain and request review when evidence is ambiguous.

## Build and test

With Rust 1.99 installed through rustup:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -- demo
```

This workspace also has an ignored, project-local toolchain under `.toolchain/`. To use it in the current checkout:

```sh
export CARGO_HOME="$PWD/.toolchain/cargo"
export RUSTUP_HOME="$PWD/.toolchain/rustup"
export RUSTUP_TOOLCHAIN="stable-aarch64-apple-darwin"
export PATH="$CARGO_HOME/bin:$PATH"
cargo test --workspace
```

## Camera probe

```sh
cargo run -p capture-probe -- permission
cargo run -p capture-probe -- devices --json
cargo run -p capture-probe -- formats --device 0 --json
cargo run -p capture-probe -- sample \
  --device 0 --frames 120 --format highest-fps --save-every 0 \
  --output local-data/camera-probe/my-run
```

Camera samples stay under ignored `local-data/` because they may contain private room imagery. See the [camera probe guide](docs/camera-probe.md) before interpreting its process-side timestamps.

The preparatory [manual geometry guide](docs/geometry.md) describes the labeled-corner convention used to map chess squares into an image.

## Project status and plan

The [development plan](docs/development-plan.md) defines the architecture, 22 five-hour work frames, model assignments, phase gates, accuracy targets, and database/export behavior. The [work log](docs/work-log.md) records measured progress and remaining hardware dependencies. Evaluation recordings use the versioned [annotation manifest format](docs/evaluation-format.md).

The accepted workspace, persistence, result, and provisional timestamp choices
are recorded in the [Phase 1 decisions](docs/phase-1-decisions.md).

The [local browser recorder](apps/recorder-ui-prototype/README.md) now connects
calibrated camera square changes to Rust legal moves, review, SQLite persistence,
undo and PGN export. Start it with `./scripts/start-recorder.sh` and open
<http://localhost:8770/>. Physical-board recognition is an experimental baseline
and still needs qualification before production Tauri integration.
