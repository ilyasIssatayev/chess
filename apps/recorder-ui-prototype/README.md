# Local camera recorder

The screen is now connected to the Rust chess rules, SQLite journal and PGN
export. The earlier static prototype only measured coarse motion and simulated
move events with buttons; it could not register a physical move.

## Run

From the repository root:

```sh
./scripts/start-recorder.sh
```

Open <http://localhost:8770/apps/recorder-ui-prototype/>. The script uses the
project-local Rust toolchain if present. To choose another port, pass it as an
argument, e.g. `./scripts/start-recorder.sh 8771`.

A plain `python3 -m http.server` serves the visual assets but does **not** provide
the rules/storage API. The screen explicitly reports a recorder connection
error in that case.

## Record a physical game

1. Start the camera. Place the board so every square is visible.
2. Click **Calibrate board**, drag the labeled corners to the matching outer
   corners (`a8`, `h8`, `h1`, `a1`), then save. The preview is unmirrored, and its
   aspect ratio matches the camera pixels. Earlier prototype calibration is not
   reused because that preview was mirrored.
3. Put the pieces in the position shown under **Tracked position**. A new game
   starts in the standard position. Check the position confirmation box and
   click **Set reference position** after clearing your hands.
4. Play one move and let the board settle. Changed square interiors are compared
   against the previous reference. Rust supplies all legal successor moves and
   the exact affected squares, including castling and en passant.
5. Confirm a unique suggestion with **Record selected move**. The detected
   reference advances when the camera still shows that position, so the next
   player can move. Optionally enable **Automatically record unique matches**.
   This is experimental; keep review enabled until it works on your setup.
6. If detection is uncertain, select the actual legal move in the menu, or click
   its source and destination on the tracked board, then record it. Manual moves
   require a fresh camera reference. Promotions require selecting the piece.

The change threshold can be lowered for small pieces/weak changes or raised for
noise. Changing calibration or sensitivity, stopping the camera, hiding the
page, or a frame interruption pauses recognition and requires a fresh reference.
Do not play while capture is paused. To recover, enter any missed moves manually,
then make the physical board match the tracked position and set a new reference.

**Undo last move** creates an audited correction; it does not erase the original
history. Return the physical pieces to the tracked position afterward. **New
game** retains the old game in SQLite. **Export PGN** downloads canonical SAN for
the current game. Reloading the page/server resumes the most recently created
game; camera recording always needs a new reference after restart.

## Limits and local data

This is a classical change-detection baseline, not a trained piece classifier.
It does not verify starting piece identities, measure occlusion probabilities,
or certify that a legal square-change match is visually correct. Exact changed
square sets, exposure-shift compensation, 900 ms settling and three consistent
observations reduce false suggestions; ambiguity and promotion type require
review. Strong perspective, tall pieces, shadows, camera movement and small
piece images can prevent a match. Physical-board recognition accuracy is not
qualified yet. Move-completion timings are explicitly unknown.

Camera images stay in the browser. Only game/move commands go to the loopback
Rust server. Games are journaled in ignored `local-data/recorder/games.sqlite`.
Corner settings stay in browser-local storage. An optional
`CHESS_RECORDER_DATA_DIR` environment variable selects a separate database folder
for local tests. The server serves only recorder assets, rejects nonlocal hosts
and cross-origin writes, and never exposes the repository or camera evidence.

## Checks

```sh
cargo test --offline --workspace
cargo clippy --offline --workspace --all-targets -- -D warnings
/System/Library/Frameworks/JavaScriptCore.framework/Versions/A/Helpers/jsc \
  apps/recorder-ui-prototype/recorder.js \
  apps/recorder-ui-prototype/tests/recorder.test.js
```

Pixel/temporal regressions cover localized moves, exposure changes, hand/extra
square changes, adjustments, illegal patterns, captures, castling, en passant,
promotions and reset behavior. Rust integration tests cover legal commits,
canonical notation, retry safety, resume, stale writes, undo and retained games.
