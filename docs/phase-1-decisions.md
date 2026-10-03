# Phase 1 decisions

Status: accepted for the software implementation gate on 3 October 2026. The
physical Phase 0 prerequisite remains open until a populated board is tested.

## Reproducible workspace

- Rust 1.99.0, edition 2024, is pinned by `rust-toolchain.toml`.
- Dependency resolution is pinned by the committed `Cargo.lock`.
- The project is licensed under MIT; the full text is in `LICENSE`.
- The planned desktop shell is Tauri. Capture, decoding, chess rules, storage,
  and export remain Rust libraries so the shell does not own recorder state.
- SQLite schema version 3 and journal event schema version 1 are the current
  durable formats. Schema changes require migrations; event payload changes
  require a new event schema version.

## Trusted game and result policy

The event journal and immutable correction revisions are authoritative. The
move projection must replay to the same FEN chain. Accepted events are written
transactionally and are idempotent by `(game_id, idempotency_key)`.

`ChessGame::status` exposes position-only checkmate, stalemate, and dependency
halfmove-clock behavior. `ChessGame::repetition_status` separately counts
FIDE-equivalent positions in the trusted history. Threefold claim availability
and automatic fivefold repetition are recorded as distinct facts. The recorder
does not infer a player's draw claim or silently replace an explicit game
result; an unfinished recording stays `*` until a result workflow records it.

## Time policy boundary

Capture time is monotonic and session-relative. The current camera adapter only
provides process receipt time after frame acquisition, so that source remains
explicitly provisional. Phase 0 physical tests will determine whether it is
adequate or a native AVFoundation presentation-timestamp adapter is required.
Wall-clock and inference-completion times are never move-completion timestamps.

## Gate evidence

The workspace tests cover deterministic FEN/SAN chains, captures, check,
checkmate, all SAN disambiguation forms, both castling directions for both
colors, en passant, every promotion choice, repetition, malformed and illegal
histories, transactional/idempotent persistence, restart, correction replay,
and abrupt termination during an open SQLite transaction. The root demo runs a
synthetic observation through decoding, persistence, replay, and PGN export.

Independent cross-implementation SAN/FEN fixtures are desirable defense in
depth and remain tracked separately; all rules outputs are already checked by
explicit expected SAN/FEN assertions, and that extra comparison is not a Phase
1 gate condition.
