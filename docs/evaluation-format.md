# Evaluation manifest format

The evaluation manifest is the versioned input contract for deterministic playback and scoring of annotated chess-camera sessions. The initial implementation lives in `crates/evaluation`; it is self-contained until the root Cargo workspace is created in Frame 3.

This format records ground truth and dataset membership. It does not claim that any camera recording exists, that a placement is feasible, or that an accuracy target has been measured. The checked-in JSON is a schema example only.

## File shape

A file contains one dataset and one or more annotated sessions. Its top-level fields are `schema_version`, `dataset_id`, and the nonempty `sessions` array.

Unknown JSON fields are rejected. `schema_version` must currently be `1`. IDs are opaque stable strings; do not derive them from a mutable filename.

The complete synthetic example is at `crates/evaluation/examples/manifest.example.json`. Validate a manifest with:

```sh
cargo run --manifest-path crates/evaluation/Cargo.toml --bin manifest-check -- path/to/manifest.json
```

The command reports all structural validation errors it can find and returns a nonzero exit status for an unreadable, malformed, or invalid manifest. When several files have the same `dataset_id`, the command also checks session IDs and partition split isolation across those files.

## Session separation and split labels

Each session has both a `session_id` and a `partition_key`:

- `session_id` uniquely identifies one continuous clip or manifest entry.
- `partition_key` groups every clip derived from the same uninterrupted recording or physical game.
- `split` is one of `training`, `validation`, `test`, or `qualification`.

Within a dataset, all entries with the same `partition_key` must use the same split. Validation rejects a partition key found in multiple splits. This prevents frames or derived clips from one game leaking into both tuning and evaluation sets. When in doubt, assign recordings from the same setup event or game the same partition key.

`qualification` is reserved for locked release-candidate evaluation. Do not tune preprocessing, models, decoder thresholds, or acceptance policy using those sessions.

## Media and frame timestamps

`media` supplies the playback location and dimensions:

```json
{
  "uri": "recordings/session-001.mov",
  "sha256": "<64 lowercase or uppercase hexadecimal characters>",
  "width_px": 1920,
  "height_px": 1080
}
```

`sha256` is optional while collecting training, validation, or test data. It is required for `qualification` sessions so locked evaluation media cannot change unnoticed. The crate validates its syntax but does not read or hash media; the ingest tool should calculate and compare the bytes.

`timestamp_source` records how the timeline was obtained: `device_presentation` for a capture-backend/device timestamp, `host_acquisition` for a monotonic host timestamp taken as the application receives a frame, or `media_presentation` for presentation timestamps recovered later from the media container. These sources have different latency and precision limits and must remain distinguishable in timing reports.

`frames` maps decoded frame indexes to monotonic acquisition times normalized to the capture-session epoch:

```json
[
  { "frame_index": 0, "capture_us": 0 },
  { "frame_index": 1, "capture_us": 33333 }
]
```

Both fields must increase strictly. Missing indexes are allowed because the capture or playback layer may report dropped frames. `capture_us` is an integer number of microseconds from the session epoch. It must come from the measured capture/acquisition timeline, not wall-clock display time, file decoding time, model inference completion, or move confirmation.

## Board corners and orientation

The four board-boundary points use image pixel coordinates. Origin `(0, 0)` is the top-left of the decoded image, `x` grows right, and `y` grows down.

Corner names are relative to the camera side of the board and must follow the perimeter:

1. `near_left`
2. `near_right`
3. `far_right`
4. `far_left`

The quadrilateral must be finite, convex, non-degenerate, and within `[0, width_px] × [0, height_px]`. The validator checks these conditions but cannot tell whether an annotator clicked the true board boundary.

`orientation` identifies the calibrated corner containing the `a1` corner of the board. It is one of `a1_near_left`, `a1_near_right`, `a1_far_right`, or `a1_far_left`. Moving forward through the perimeter order from that corner maps to `h1`, then `h8`, then `a8`. For example, `a1_near_left` maps the named perimeter to `a1, h1, h8, a8`; `a1_far_right` maps it to `h8, a8, a1, h1`. This supports a camera on any side of the board and removes the need to infer orientation from piece recognition.

Use a new session entry when calibration changes materially during a clip. Later drift annotations can reference the source frame range without changing this version-1 baseline.

## Reference moves and completion bounds

`reference_game.initial_position` is `startpos` for standard chess or a FEN string. Reference moves use one-based `ply` values and syntactic UCI notation:

```json
{
  "ply": 1,
  "uci": "e2e4",
  "completion": {
    "kind": "bounded",
    "earliest_capture_us": 1200000,
    "latest_capture_us": 1266666
  }
}
```

Promotion adds `q`, `r`, `b`, or `n`, for example `a7a8q`. The manifest validator checks UCI shape and sequential ply numbering. A later chess-rules adapter must verify legality and position history from `initial_position`.

A bounded completion interval is the inclusive acquisition-time range in which the full physical chess transition became complete. For a capture, it follows removal and placement; for castling, it follows movement of both pieces. Bounds must lie within the session frame range and allow a chronological ordering of the annotated moves. Overlapping bounds are valid when annotation uncertainty still permits an ordered sequence.

When the camera cannot support a defensible interval, record uncertainty explicitly:

```json
{
  "kind": "unknown",
  "reason": "hand occludes the board until after both players moved"
}
```

An unknown completion never receives an invented point estimate. It also does not mean the move itself is uncertain: move-sequence review and timing observability are separate judgments.

## Validation boundary

The crate currently validates:

- supported schema version, required IDs, unique session IDs, and nonempty media details;
- partition-level separation between training, validation, test, and qualification;
- positive dimensions, optional SHA-256 syntax, increasing frame indexes, and increasing capture timestamps;
- finite, in-image, convex board corners in the required perimeter order;
- consecutive plies, syntactic UCI moves, bounded or explained-unknown completion annotations;
- completion intervals within available frames and temporal feasibility across known bounds.

The following checks belong to later adapters and evaluation tooling:

- reading the media, verifying the SHA-256, and matching decoded frames to the timestamp table;
- chess legality, FEN parsing, move-result positions, SAN, and special-move semantics;
- visual confirmation that corners, orientation, moves, and completion bounds match the recording;
- dataset counts, setup distribution, prediction scoring, timing coverage, and statistical reports.

Keeping these boundaries explicit lets Frame 2 annotation begin without coupling it to an unfinished chess engine or video decoder, while preventing structurally invalid data from entering later measurements.
