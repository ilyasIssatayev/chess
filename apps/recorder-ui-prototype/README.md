# Recorder UI prototype

This dependency-free prototype demonstrates the recording screen before the
Tauri shell is introduced. It provides a local browser camera preview, board
overlay, simple motion indication, recorder decision states, changed-square
highlighting, and a trusted-move list.

The browser measures coarse frame-to-frame motion only. The **Play decision
demo** and **Show ambiguity** buttons simulate events that will later come from
the Rust observation and temporal-decoder pipeline; they do not recognize chess
moves from the camera.

Serve the repository over localhost so the browser can request camera access:

```sh
python3 -m http.server 8765
```

Then open:

```text
http://localhost:8765/apps/recorder-ui-prototype/
```

Camera frames remain inside the browser page and are not uploaded. The final
Tauri screen should consume explicit Rust events for capture health, motion,
visibility, calibration, candidate transitions, accepted moves, and review
items rather than deriving recorder decisions in JavaScript.
