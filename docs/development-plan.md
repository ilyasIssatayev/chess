# Chess camera recorder: phased development plan

Planning baseline: 2 October 2026. Implementation progress and measured hardware results are tracked in [the work log](work-log.md). The repository now contains the Rust workspace, camera probe, shared contracts, chess/storage/export foundations, evaluation tooling, and a synthetic temporal-decoder baseline; phase gates remain governed by the evidence requirements below.

## Goal and first release

Build a macOS application that uses the **built-in MacBook camera**, aimed at a nearby physical chessboard, to record complete games and the elapsed time between moves. Prioritize correct move recording over immediate acceptance. Store games locally and provide a dashboard for replay, corrections, timing inspection, and PGN export for Chess.com.

The first release supports standard chess, a verified standard starting position, a stationary board/camera, and a documented range of board sizes, piece sets, lighting, camera angles, and playing speeds. Manual corner selection, orientation confirmation, and occasional ambiguity review are acceptable. Broader support must earn its own accuracy gate.

The central product rule is: **commit a move only when visual evidence supports it; preserve uncertainty when it does not**. A legal move can still be the wrong move. A chess engine's preferred move must never substitute for evidence.

A nearby laptop is not automatically a workable camera mount. At shallow angles, nearer pieces can hide farther pieces. Perspective correction changes geometry but cannot recover hidden information. Phase 0 must establish a useful built-in-camera placement before substantial implementation proceeds.

## Proposed architecture

Use Rust for capture, image processing, inference, chess logic, timing, persistence, and exports. Use a Tauri desktop shell with a small TypeScript dashboard. This is a Rust application with a web-based presentation layer; if every component must be Rust, choose an egui shell in Phase 1 while preserving the same core interfaces.

```mermaid
flowchart LR
    A[MacBook camera] --> B[Timestamped frames and capture health]
    B --> C[Calibration, motion, visibility, piece evidence]
    C --> D[Temporal decoder and legal chess positions]
    D --> E[Confirmed moves or pending review]
    E --> F[SQLite events, moves, timings, revisions]
    B --> G[Bounded evidence buffer]
    G --> F
    F --> H[Dashboard and replay]
    H --> I[Audited corrections]
    I --> D
    F --> J[PGN and timing exports]
```

| Component | Initial choice | Decision or validation required |
| --- | --- | --- |
| Camera | `nokhwa` with its AVFoundation backend | Verify initialization, permissions, actual formats, timestamps, frame rate, and disconnect behavior on the target MacBook. Use a narrow native AVFoundation adapter if the wrapper cannot meet the capture contract. [Backend documentation](https://docs.rs/nokhwa/latest/nokhwa/backends/capture/struct.AVFoundationCaptureDevice.html) |
| Classical vision | Rust OpenCV bindings | Manual board-plane calibration, transforms, motion and change detection. Prove native dependency packaging early. [Official bindings](https://github.com/twistedfall/opencv-rust) |
| Model inference | `ort` / ONNX Runtime, CPU baseline | Export and validate actual model graphs before adopting them. Benchmark CoreML separately after CPU correctness; provider support does not guarantee acceleration for every graph. [Rust runtime](https://github.com/pykeio/ort), [CoreML provider](https://onnxruntime.ai/docs/execution-providers/CoreML-ExecutionProvider.html) |
| Chess rules | A crate behind a project-owned `RulesEngine` interface | Prefer `cozy-chess` as an MIT baseline; verify SAN helpers or implement a thoroughly tested notation adapter. `shakmaty` already provides legal moves and FEN/SAN/UCI, but is GPL-3.0-or-later. Record the project licensing choice before selecting it. [cozy-chess](https://github.com/analog-hors/cozy-chess), [shakmaty](https://github.com/niklasf/shakmaty) |
| Persistence | SQLite through `rusqlite` | One writer, versioned migrations, transactional move commits, consistent backup/restore. [Rust API](https://docs.rs/rusqlite/latest/rusqlite/), [SQLite transactions](https://www.sqlite.org/lang_transaction.html) |
| Desktop UI | Tauri | Native capture stays in Rust. Validate a packaged camera-enabled application early, including the usage description and required entitlements for the chosen distribution mode. [macOS bundle documentation](https://v2.tauri.app/distribute/macos-application-bundle/) |
| Interchange | Standard PGN using SAN | Chess.com supports loading PGN into its analysis board. Timing annotation preservation needs a separate test. [Chess.com documentation](https://support.chess.com/en/articles/8583825-how-do-i-use-the-analysis-board) |

These are proposed dependencies, not installed or pinned versions. Phase 1 selects versions against the target Rust toolchain and macOS release, then records lockfiles and reproducible build instructions. Python may be used for offline training/export; the shipped recording pipeline runs in Rust.

## Vision strategy and free model candidates

Begin with a known chess state and track changes. Use square occupancy, piece color/type probabilities, motion, and visibility across time to rank legal successor positions. Full-board classification is useful for setup verification and recovery, but should not replace trusted game history every frame.

| Candidate | Planned use | Constraint |
| --- | --- | --- |
| Classical change detection plus legal transitions | First baseline for the calibrated, known piece set | Cheap and explainable; may struggle with shadows, off-center pieces and occlusion. Measure before adding complexity. |
| ChessCog occupancy and piece models | First pretrained physical-board baseline; optional local fine-tuning | The project publishes model downloads and adaptation instructions. Code is MIT; checkpoint and dataset redistribution terms must be verified separately. Its training distribution is not evidence of success on this MacBook setup. [Official project](https://github.com/georg-wolflein/chesscog) |
| Small locally adapted occupancy/color/piece model | Improve failures using recordings of supported setups | Use data with documented rights. Preserve independent validation/test sessions and export equivalent preprocessing to ONNX. |
| ChessQueries | Optional research comparator | Public code and weights use PolyForm Noncommercial terms. It is a large model, so license suitability and MacBook resource use must pass before product adoption. Do not make it an unconditional dependency. [Official project](https://github.com/JSeytre/chessqueries), [model repository](https://huggingface.co/joelseytre/chessqueries) |

ChessReD can help benchmark physical-board recognition, but its dataset terms must be verified before use. Public still-image benchmarks cannot establish continuous move-recording or timing accuracy. [Original research](https://arxiv.org/abs/2310.04086)

For each adopted model, store a manifest containing source URL, code/weights/data terms, file hash, architecture, input/output schema, preprocessing, export procedure, inference provider, and evaluation results. Free download alone is insufficient to establish permitted redistribution. No paid inference service is required by this plan.

## Multi-agent execution flow

Use **one coordinator and up to three worker agents per wave**, matching the available four-agent capacity. To manage usage, start with one coordinator and one worker only when an independent task justifies it; add workers for substantial parallel tasks. Roles change by phase; this is a development workflow, not a multi-agent architecture inside the product. The phase tables describe responsibilities, not a requirement to run every role simultaneously.

The coordinator owns integration, shared contracts, dependency changes, migrations, phase gates, and the release decision. Worker agents receive explicit objectives, owned directories, dependencies, fixtures, and acceptance criteria. Prefer separate branches/worktrees during implementation; in a shared checkout, allocate non-overlapping files. Only the coordinator edits shared manifests and contract files once they are agreed.

Every wave follows this sequence:

1. Coordinator defines one reviewable deliverable and freezes the relevant contracts/fixtures.
2. Up to three agents implement independent tasks in parallel, using fixtures or mocks where upstream work is incomplete.
3. Each agent reports changes, validation, measurements, assumptions, and remaining failures.
4. Coordinator integrates changes in dependency order and runs the phase checks.
5. A reviewer who did not author the relevant change verifies the gate. Rotate an agent into this role after implementation rather than exceeding the concurrency limit.
6. Failed gates produce scoped follow-up tasks. A phase does not pass because its implementation tasks are finished.

Suggested ownership boundaries after Phase 1:

```text
crates/contracts/       # Coordinator: shared types and serialization
crates/chess-core/      # Rules, move decoding, timing and revisions
crates/capture/         # Camera adapters, timestamps and health events
crates/vision/          # Calibration, observations and model adapters
crates/storage/         # SQLite, evidence manifests and projections
crates/export/          # PGN and detailed timing exports
apps/desktop/           # Tauri shell and dashboard
tools/evaluate/         # Offline replay, annotations and reports
fixtures/               # Small reproducible cases; large recordings outside Git
docs/                   # Contracts, decisions, setup and evaluation results
```

The coordinator owns `Cargo.toml`, `Cargo.lock`, database migration numbering, and cross-component API changes. Model files and full recordings belong in versioned artifact storage or local fixture storage with hashes, not ordinary Git history.

## Five-hour work frames and development models

A work frame below is a planning block of **approximately five productive hours**, ending in a reviewable checkpoint. It is not a guarantee of five hours of agent execution within a subscription usage window. Usage depends on the model, task, context, reasoning and tools; account limits and reset times must be checked separately. A project token budget does not determine a fixed number of subscription windows. [Official OpenAI usage guidance](https://learn.chatgpt.com/docs/pricing#what-are-the-usage-limits-for-my-plan)

The initial allocation is **18 implementation frames (about 90 productive hours) to an integrated MVP**, followed by candidate preparation, repeatable qualification frames and release. This is a scope allocation, not a fixed delivery promise. Native capture fallback, model adaptation, failed gates and additional recordings add frames. Physical play and annotation also take real-world time and depend on the user or another person being available. Re-estimate after Frame 2 and after the first combined vision/decoder validation in Frame 10.

### Model policy

These are **Codex development models**. The computer-vision models shipped inside the app remain the candidates in the vision strategy section; selecting Astra for development does not make the app call Astra while recording games.

| Label used below | Select in Codex | Assignment |
| --- | --- | --- |
| `Sol/M` | GPT-6.1 Sol, Medium reasoning | Default coordinator and implementation model. |
| `Sol/H` | GPT-6.1 Sol, High reasoning | Complex decoder, timing, correction and recovery logic; correctness reviews. |
| `Luna/H` | GPT-6 Luna, High reasoning | Optional scoped helper for documentation lookup, fixture tooling, small UI tasks and report formatting. |
| `Astra/M` | GPT-6 Astra, Medium reasoning initially | Optional targeted review when a specific design problem or unexplained error needs deeper analysis. |

This project allocation follows official OpenAI guidance on demanding versus scoped agent work. Model availability depends on the account/client. If the recommended model is unavailable, choose an available alternative and evaluate it on the same checkpoint rather than assume equivalent results. [Model selection](https://developers.openai.com/api/docs/guides/model-selection), [Subagent guidance](https://learn.chatgpt.com/docs/agent-configuration/subagents)

Start at Standard speed. Higher effort or additional workers should solve a concrete need. Astra reviews replace an existing worker/reviewer slot and remain bounded to the question at hand. Independent review means a different agent from the author, even when both use Sol. Do not silently downgrade critical correctness work to conserve allowance; reduce its scope or carry it into the next frame.

### Structure of each frame

| Elapsed productive time | Work |
| --- | --- |
| 00:00–00:20 | Read the previous handoff, inspect the current code and confirm prerequisites and available allowance. |
| 00:20–00:40 | Define the smallest deliverable, contracts, file ownership and required checks; dispatch an independent helper if useful. |
| 00:40–03:40 | Implement the numbered steps below, integrating dependencies in order. |
| 03:40–04:35 | Run the relevant checks, review the result and fix scoped failures. |
| 04:35–05:00 | Save the checkpoint and handoff, record evidence and remaining work, and choose the next frame. |

A frame may finish sooner or need continuation. If allowance runs short, save a consistent checkpoint when possible and resume the unfinished gate in the next available session. Do not mark it complete because five hours elapsed. Hardware/data-dependent steps wait for actual inputs; agents can prepare fixtures or tooling in parallel without claiming that the physical test passed.

### Frame sequence

All frames are initially **planned**. Begin each dependent frame only after its prerequisites pass. A failed gate creates a continuation such as `F08a` or `F10a`; later frame numbers remain the intended sequence, not calendar dates.

#### Frame 1 — Toolchain and camera probe · Phase 0

**Models:** lead `Sol/M`; optional `Luna/H` for documentation and probe instructions.

1. Establish the Rust toolchain and record the target MacBook/macOS details.
2. Build a minimal camera preview/capture probe and enumerate supported formats and timestamp sources.
3. Package the probe and exercise camera permission denial, grant and recovery.

**Checkpoint:** runnable capture probe with sample frames and a format/timestamp report. Create `docs/work-log.md` for future frame handoffs. Requires access to the actual MacBook camera.

#### Frame 2 — Physical placement and feasibility · Phase 0

**Models:** lead `Sol/M`; optional `Luna/H` for annotation tooling. Use `Astra/M` only for a concrete unresolved geometry review.

1. Position the laptop and populated board; manually map corners and orientation.
2. Record scripted moves in both orientations, including far-rank moves, captures and hands covering the board; annotate representative events.
3. Measure visibility and geometry, document the practical placement, and decide whether the built-in-camera setup passes Phase 0.

**Checkpoint:** placement guide, annotated development clips and feasibility decision. A person must arrange the hardware and perform the physical moves. If no useful placement works, revisit scope before dependent vision work.

#### Frame 3 — Workspace and shared contracts · Phase 1

**Models:** lead `Sol/M`; optional `Luna/H` for dependency/API research.

1. Create the workspace boundaries and select dependency versions, shell and project licensing.
2. Define frame, observation, trusted move, review, revision and timing contracts.
3. Add a minimal synthetic pipeline and fixture format so workers can implement adapters independently.

**Checkpoint:** compiling workspace and versioned contracts. Prerequisite: Frame 2 feasibility decision.

#### Frame 4 — Legal chess and notation · Phase 1

**Models:** lead `Sol/H`; optional `Luna/H` for independently sourced reference fixtures.

1. Implement the rules adapter and verified standard starting state.
2. Derive canonical moves, SAN and FEN, including special moves and disambiguation.
3. Validate legal transitions against independent reference histories, including repeated positions.

**Checkpoint:** deterministic rules/notation module with the required special-move checks passing.

#### Frame 5 — Database journal and replay · Phase 1

**Models:** lead `Sol/M`, raising to `Sol/H` for revision invariants; optional `Luna/H` for fixture tooling.

1. Add migrations and transactional, idempotent game/move/event persistence.
2. Replay stored synthetic events into the trusted position chain and preserve correction revisions.
3. Check duplicate events, interrupted commits and restart against the reference history.

**Checkpoint:** synthetic game survives restart and replays identically; Phase 1 gate passes.

#### Frame 6 — Offline playback and board calibration · Phase 2

**Models:** lead `Sol/M`; optional `Luna/H` for dataset-manifest tooling.

1. Replay timestamped development clips deterministically.
2. Implement manual corner/orientation calibration and board-plane square mapping.
3. Record mapping quality and establish complete-session train/validation/test separation; acquire and annotate independent validation recordings before observation/decoder tuning.

**Checkpoint:** repeatable offline frames, calibrated square overlay, and separate annotated development/validation sessions. Actual clips from Frame 2 must be available; add a collection/annotation continuation if independent validation data is missing before Frames 7 and 10.

#### Frame 7 — Classical observations · Phase 2

**Models:** lead `Sol/M`; optional `Luna/H` for evaluation/report scripts.

1. Implement motion/change measurements and initial square occupancy/color evidence.
2. Mark blurred or obscured evidence as unknown and detect calibration drift.
3. Benchmark observations on development/validation sessions and catalogue failure cases.

**Checkpoint:** reproducible baseline observation stream with quality/visibility flags and measured errors.

#### Frame 8 — Pretrained model and Rust parity · Phase 2

**Models:** lead `Sol/M`; optional `Luna/H` for artifact provenance and documentation lookup.

1. Audit/download ChessCog candidates and record artifact terms, hashes and preprocessing.
2. Export the selected candidate to ONNX and compare reference outputs with Rust CPU inference.
3. Compare observations and resource use against Frame 7; select a provisional pipeline.

**Checkpoint:** tested inference parity and provisional model decision; Phase 2 gate passes. If export or adaptation fails, add a scoped continuation; a successful download alone is not completion.

#### Frame 9 — Temporal move decoder · Phase 3

**Models:** lead `Sol/H`; optional `Luna/H` for narrow fixture preparation. Use a separate `Sol/H` reviewer after implementation.

1. Implement motion/settling states and unchanged/legal-successor hypotheses.
2. Accumulate evidence across frames and accept only sufficiently visible, stable and distinguishable transitions.
3. Verify captures, compound moves, physical adjustments and ambiguous positions.

**Checkpoint:** offline decoder emits trusted moves or explicit review items without inventing intermediate moves.

#### Frame 10 — Timing, gaps and combined validation · Phase 3

**Models:** lead `Sol/H`; optional `Luna/H` for annotated timing fixtures; optional bounded `Astra/M` review of decoder/timing assumptions.

1. Implement completion bounds and elapsed intervals separately from confirmation time.
2. Preserve trusted history across gaps; validate inferred/manual corrections and their affected suffixes.
3. Measure the combined pipeline on validation sessions, select the model/decoder configuration and freeze acceptance thresholds.

**Checkpoint:** Phase 3 gate and timing checks pass; publish baseline precision, coverage, review burden and failure classes. Re-estimate the remaining frames from these results. Add continuation frames if the recording quality is inadequate.

#### Frame 11 — Live pipeline integration · Phase 4

**Models:** lead `Sol/M`, using `Sol/H` for scheduling issues; optional `Luna/H` for telemetry fixtures.

1. Connect live capture, observations, decoder and database through bounded queues.
2. Preserve acquisition timestamps and report dropped frames/backlog explicitly.
3. Keep capture and preview responsive while inference runs.

**Checkpoint:** live ordinary moves persist correctly and the recorder reports capture health.

#### Frame 12 — Durable evidence and retention · Phase 4

**Models:** lead `Sol/M`; optional `Luna/H` for file-lifecycle fixtures.

1. Add the bounded evidence buffer and persist move/uncertainty clips or frames.
2. Link evidence to events and reconcile interrupted file/manifest writes.
3. Enforce retention limits and visibly mark missing evidence.

**Checkpoint:** accepted and uncertain events can retrieve the correct timestamped evidence after restart.

#### Frame 13 — Outages, drift and restart · Phase 4

**Models:** lead `Sol/H`; optional `Luna/H` for fault-injection tooling; separate `Sol/H` review.

1. Handle camera loss, pause, sleep/wake and changed camera/board geometry.
2. Restore the trusted prefix after restart and revalidate the observed board.
3. Test hidden multi-move gaps, duplicate frames and interrupted writes without silently accepting a fabricated sequence.

**Checkpoint:** each failure has a tested recovery/review state; no acknowledged move is lost in the tested restart cases.

#### Frame 14 — Live soak and latency · Phase 4

**Models:** lead `Sol/M`; use `Sol/H` for identified concurrency faults; optional `Luna/H` for measurement summaries.

1. Run the 60-minute session with the actual camera and representative play.
2. Measure memory, dropped frames, confirmation latency and timing behavior under load.
3. Resolve scoped failures and document supported playing speed and capture limits.

**Checkpoint:** Phase 4 gate passes on the actual MacBook. Budget the hour of physical testing inside this frame; repeat the frame if failures require a new soak run.

#### Frame 15 — Desktop shell and game library · Phase 5

**Models:** lead `Sol/M`; optional `Luna/H` for a clearly scoped list/filter UI task.

1. Connect the chosen desktop shell to Rust commands/events.
2. Build the recording controls and searchable game library with completeness/status fields.
3. Open a stored game and show its metadata and trusted move list.

**Checkpoint:** the desktop app can start/finish a session and reopen saved games.

#### Frame 16 — Replay, timing and evidence views · Phase 5

**Models:** lead `Sol/M`; optional `Luna/H` for small rendering or formatting tasks.

1. Add move-by-move digital board replay from the trusted revision.
2. Display elapsed estimates/bounds and unknown timing clearly.
3. Show linked evidence and recording gaps alongside the move history.

**Checkpoint:** replay matches stored FEN/SAN and distinguishes uncertainty from precise measurements.

#### Frame 17 — Review and correction UI · Phase 5

**Models:** lead `Sol/H`; optional `Luna/H` for scoped presentation work; separate `Sol/H` correctness review.

1. Present pending candidates, promotion choices and their visual evidence.
2. Allow manual resolution and earlier-move correction through audited revisions.
3. Recompute/validate subsequent history and stop at the first invalid continuation.

**Checkpoint:** corrections consistently update replay and preserve the original decision/evidence.

#### Frame 18 — Export and integrated MVP walkthrough · Phase 5

**Models:** lead `Sol/M`; use `Sol/H` for notation/revision failures; optional `Luna/H` for reference exports and user instructions.

1. Export clean PGN, descriptive timing comments and detailed JSON/CSV sidecars.
2. Check independent parsing and Chess.com loading, including special moves and incomplete games.
3. Walk through record, close, reopen, replay, correct and export in the desktop app.

**Checkpoint:** Phase 5 gate passes and the integrated MVP is reviewable. This checkpoint does not establish the release accuracy targets.

#### Frame 19 — Package and freeze the qualification candidate · Phase 6 preparation / Phase 7 packaging

**Models:** lead `Sol/M`; optional `Luna/H` for evaluator/manifest tooling; separate `Sol/H` review of measurement logic.

1. Bundle the actual application, native dependencies and model assets; smoke-test capture and runtime loading outside development paths.
2. Freeze candidate binary/assets/configuration and the independent evaluator, including original-decision scoring rules.
3. Freeze the qualification collection protocol, supported conditions, count targets and metric definitions; lock manifests for any already reserved qualification recordings.

**Checkpoint:** a reproducible packaged candidate and locked evaluation procedure. Packaging happens before qualification; a later change to capture, preprocessing, models or decoding requires renewed qualification.

#### Frame 20 — Collect and qualify held-out games · Phase 6 · Repeat as needed

**Models:** lead `Sol/M` for evaluation; optional `Luna/H` for annotation tooling and report formatting. Ground-truth play/annotation requires people and independent checking.

1. Collect and annotate held-out complete games using the frozen protocol; hash/seal each new batch manifest and keep qualification material out of tuning.
2. Run the frozen candidate in batches and compare original automatic decisions, complete histories and completion-time bounds against reference annotations.
3. Accumulate at least 30 complete games, 3,000 reference plies and 3,000 automatic decisions, plus the edge-case suite.

**Checkpoint:** save each batch manifest and results as `F20a`, `F20b`, etc. This frame repeats until the data/count requirements are met. It is not credible to promise all physical recording and annotation in one five-hour block. Allow more reference games when abstentions reduce the automatic-decision count.

#### Frame 21 — Accuracy report and release gate · Phase 6

**Models:** independent lead `Sol/H`; optional `Luna/H` for report formatting; `Astra/M` only for a specific unexplained error or challenged assumption.

1. Publish precision, coverage, whole-game correctness, timing uncertainty and reliability with denominators and statistical uncertainty.
2. Audit silent errors, reviewed decisions and setup distribution against the Phase 6 targets.
3. Pass the release gate or create a scoped repair frame followed by fresh held-out qualification.

**Checkpoint:** documented release decision. Diagnosis may examine failures, but tuning after a failed qualification requires a new held-out set; the same final test is not reused to claim improvement.

#### Frame 22 — Supported release and handoff · Phase 7

**Models:** lead `Sol/M`; optional `Luna/H` for setup/release documentation; separate reviewer for packaged critical flows.

1. Validate the qualified package on a clean target Mac, including permission handling, restart, backup/restore, replay and export.
2. Finalize the placement guide, retention/review guidance and measured supported conditions.
3. Prepare the reproducible release bundle and release notes for the chosen distribution channel.

**Checkpoint:** Phase 7 gate passes. Signing/account setup or clean-device availability may need a continuation. If packaging fixes affect the recording pipeline, return to qualification before release.

### Handoff and estimation record

At the end of each frame, update `docs/work-log.md` with the frame ID, status (`planned`, `in progress`, `passed`, `needs continuation`), actual productive time, models/reasoning used, changed files, checks/results, hardware/data needs, and the exact next action. Record usage before/after if the account dashboard or client exposes it; mark it unknown otherwise. Total token counts must include all agents when available. Do not invent a token allowance per window.

Run one bounded milestone first and use its observed usage to adjust helper count and scope. Keep short handoffs and targeted reads. Preserve independent review and the existing accuracy gates when shortening a frame.

## Phases and gates

### Phase 0 — Prove built-in-camera feasibility

**Deliverable:** a working capture probe, annotated sample recordings, a camera placement guide, and a go/no-go report.

| Agent | Parallel responsibility |
| --- | --- |
| Platform agent | Establish the Rust toolchain and a minimal packaged macOS capture probe; enumerate formats and timestamp behavior; test permission denial and recovery. |
| Vision agent | Test nearby placement, laptop elevation, distance and lid angle; map all squares; measure far-rank occlusion, blur and lighting sensitivity. |
| Evaluation agent | Define ground-truth move/timing annotation, record representative play, and catalogue setup failures and difficult events. |

Start with the actual board/pieces and a full starting formation. Test both board orientations and both players' hands. Capture quiet moves, captures, fast replies, and special-move clips. Use manual four-corner calibration first. An empty checkerboard demonstration is insufficient.

**Gate:** demonstrate a practical built-in-camera placement where all 64 square locations and the necessary piece changes are distinguishable between moves. Record the exact MacBook model/architecture and macOS version, achieved resolution/frame rate, supported geometry, far-rank square/piece-base pixel sizes, and calibration reprojection error. Set a minimum supported inter-move interval from evidence. If no practical placement works, document the limit and revisit scope with the user; do not silently substitute an external camera or promise that a bigger model fixes occlusion.

### Phase 1 — Establish contracts, chess state and durable storage

**Deliverable:** a reproducible Rust workspace, shared contracts, synthetic end-to-end recording, and a database that can replay the trusted game.

| Agent | Parallel responsibility |
| --- | --- |
| Chess agent | Implement rules adapter, legal transitions, UCI/SAN/FEN, and standard-position initialization. |
| Storage agent | Implement schema, migrations, event journal, move projection and correction revisions against frozen contracts. |
| Evaluation agent | Build synthetic observation sequences and independent reference cases, including special moves, repeated positions and malformed histories. |

The coordinator selects shell/dependency versions, app licensing, and timestamp/event schemas before workers implement adapters. Initialize side to move, castling rights and en passant state explicitly. Images alone do not establish historical rights; custom FEN starts are later work.

**Gate:** deterministic replay of recorded events produces the same trusted FEN chain. Each accepted ply and its supporting metadata commits transactionally and idempotently. Rules and notation cover captures, check, mate, disambiguation, both castling directions, en passant and all promotion choices. Duplicate events cannot create duplicate moves.

### Phase 2 — Build calibrated offline observations

**Deliverable:** replay recorded camera frames into probabilistic board observations with reproducible benchmark reports.

| Agent | Parallel responsibility |
| --- | --- |
| Geometry agent | Implement four-corner calibration, orientation, board-plane mapping, drift checks and setup quality feedback. |
| Model agent | Compare the classical baseline and ChessCog; audit artifacts; export the selected candidate and verify Rust preprocessing/inference parity. |
| Evaluation agent | Create session-separated train/validation/test manifests; measure square mapping, visibility failures, exact-board recognition and runtime cost. |

Map pieces using their board-plane location/base evidence where possible. Bounding-box centers and square crops can be misleading when upright pieces overlap adjacent squares at a shallow angle. Mark occluded/blurred squares as unknown rather than empty.

**Gate:** reproducible observations on real MacBook recordings; verified artifact terms and CPU inference; usable motion/visibility signals; documented failure classes. Select a provisional model from observation quality and runtime measurements. Final selection depends on its impact on move decoding in Phase 3. Keep held-out qualification sessions untouched.

### Phase 3 — Decode precise moves and preserve timing uncertainty

**Deliverable:** an offline recorder producing trusted moves, elapsed intervals, review items and evidence references.

| Agent | Parallel responsibility |
| --- | --- |
| Decoder agent | Implement temporal state machine, unchanged/legal-successor scoring and acceptance/abstention rules. |
| Timing/recovery agent | Implement observation bounds, interval calculations, bounded gap-reconstruction candidates and replay after corrections. |
| Evaluation agent | Challenge decoding with compound moves, hidden events, physical adjustments, takebacks and capture gaps. |

Use `Calibrating → Stable → Disturbed → Settling → Candidate → Confirmed`, with explicit `Uncertain` and `RecalibrationRequired` exits. Stability is measured in elapsed capture time, not a fixed number of frames. Include the unchanged position as a hypothesis so touching or centering a piece creates no move.

Compare expected legal positions against evidence across the whole board and multiple frames. Require enough visibility, absolute evidence quality and a margin over alternatives. Castling, en passant and promotion are single completed chess transitions even though players perform several physical actions. Wait for complete evidence; request review when the promoted piece is unresolved.

**Gate:** no automatic move for a piece adjustment, incomplete compound move or unsupported ambiguity. Legal-but-wrong hypotheses are rejected on evidence. Uncertain history never overwrites the trusted prefix. Test timing against annotated capture events and processing backlogs. Select the combined vision/decoder pipeline and freeze acceptance thresholds on validation sessions. Later changes require a new version and renewed validation before qualification. Proceed only with a measured improvement over the baseline and no unresolved systemic silent-error pattern.

### Phase 4 — Integrate live recording and recovery

**Deliverable:** a live recorder that saves a game while reporting its recording health.

| Agent | Parallel responsibility |
| --- | --- |
| Capture agent | Implement production timestamped capture, bounded queues, preview, dropped-frame accounting and camera health events. |
| Recorder agent | Connect observations, decoder, storage and evidence lifecycle; keep inference independent of capture and dashboard responsiveness. |
| Reliability agent | Exercise disconnects, board/camera movement, sleep/wake, backlog, process termination and restart. |

Capture may outpace inference. Use bounded queues and explicitly record discarded-frame gaps; never allow an unbounded backlog to make the application appear current. A gap can hide multiple moves and must enter recovery rather than assume one legal successor. Continue retaining evidence while trusted move acceptance is suspended.

Keep a bounded evidence buffer and persist clips/frames around moves and unresolved segments. Retention limits must be visible; if an unresolved segment exceeds available storage, mark the resulting evidence loss. Store model/calibration versions with observations. On restart, restore the last trusted prefix and revalidate the live board before accepting new moves.

**Gate:** a live sample game survives restart with every acknowledged move preserved; stale capture, drift and gaps become visible states. Hardware timestamps or the fallback host timestamp policy are measured and documented. A 60-minute session has bounded memory and acceptable latency on the target MacBook.

### Phase 5 — Deliver the dashboard and correction workflow

**Deliverable:** a usable application for recording and revisiting games.

| Agent | Parallel responsibility |
| --- | --- |
| Dashboard agent | Build game list/search/filter, game details, move-by-move board replay, timing chart and evidence viewer. |
| Review agent | Build pending-move resolution, manual move entry, promotion selection and audited correction/revalidation flow. |
| Export agent | Implement standard PGN, optional timing comments and JSON/CSV timing sidecars; validate round trips and Chess.com loading. |

The recording screen includes camera preview with board overlay, setup checks, trusted digital board, last accepted move, pending review, pause/resume/finish, and capture health. The game library includes date, players, result, ply count, duration and recording completeness. Replay shows SAN, board state, elapsed time, uncertainty and linked evidence.

A correction creates a revision, recomputes SAN/FEN for the affected suffix and validates every later move. Stop at the first invalid continuation and preserve it for review. Do not export a disconnected or silently repaired history. Finishing recording does not imply a known chess result; unresolved results use `*`.

**Gate:** a user can record, close, reopen, replay, resolve an uncertain move, correct an earlier move, and export the resulting game. Standard PGN imports into Chess.com analysis with matching moves and result; verify ordinary games and special moves. Timing survives in the database and sidecar even if an importer discards comments.

### Phase 6 — Qualify accuracy on held-out complete games

**Deliverable:** a frozen-candidate evaluation report, supported-setup statement and release decision.

Before locking qualification, prepare and smoke-test the packaged candidate using the Phase 7 platform tasks (Frame 19). Qualify that candidate; final release remains conditional on this phase's gate.

| Agent | Parallel responsibility |
| --- | --- |
| Independent QA agent | Run the locked test manifest and full-game move/timing comparison; include denominator counts and uncertainty. |
| Reliability agent | Validate database recovery, correction consistency, export, backup/restore and packaged capture behavior. |
| Vision/decoder agent | Triage failures and propose scoped fixes without tuning against the locked final test set. |

Include multiple complete sessions and supported board/piece sets, both orientations, lighting changes, far-rank moves, captures, castling, en passant, every promotion, repeated positions, piece adjustments, takebacks, two hands, long occlusion, fast replies, board bumps and interrupted capture. Keep targeted edge-case clips separate from the complete-game qualification set.

Score automatic precision and coverage from the original automatic decisions before review, including decisions later corrected. Match them against the reference sequence; wrong, duplicate and extra automatic plies count as errors. Active correction revisions must not improve the reported automatic metrics.

Provisional release targets within the declared setup:

| Metric | Target and reporting rule |
| --- | --- |
| Automatic move precision | At least 99.9% correct among automatically committed plies; report errors and total automatic plies. |
| Automatic coverage | At least 95% of reference plies correctly recorded automatically; abstentions and gaps stay in the denominator. |
| Unattended whole-game correctness | Report exact games with no corrections or missing plies separately; aim for at least 90% for the first supported setup. Move-level accuracy cannot establish this metric. |
| Correctness after review | Every game labelled verified must exactly match the audited reference. Incomplete/unresolved games remain clearly labelled. |
| Timing | For visibly observable completion events, aim for 95th-percentile error within 0.5 seconds. Also report interval width, uncertain/unknown fraction and interval coverage against annotations. Hidden events are not counted as precisely timed. |
| Commit latency | Aim for 95th percentile within 1 second after sufficient clear settled evidence becomes available. Report separately from timing error. |
| Persistence/export | No duplicate plies, illegal accepted histories, lost acknowledged commits in tested crashes, or PGN/reference mismatches. |

Qualify on at least 3,000 reference plies, at least 3,000 automatically committed plies, and 30 complete games, plus the edge-case suite. Extend the reference recordings as needed to reach the automatic-decision count. Report statistical uncertainty and setup distribution; this sample alone does not guarantee the target error rate. If tuning follows a failed final evaluation, use a fresh held-out qualification set for the next release decision. Coverage, review burden and timing uncertainty prevent inflated results from excessive abstention.

**Gate:** all required measurements are published and the supported-setup targets pass. Investigate any unflagged wrong move in the qualification run before release. Narrow the declared supported conditions or improve the pipeline when targets fail; do not relabel reviewed games as unattended successes.

### Phase 7 — Package and release the supported version

**Deliverable:** a reproducible macOS application bundle, local database lifecycle, setup instructions, and release notes with measured limits.

| Agent | Parallel responsibility |
| --- | --- |
| Platform agent | Bundle native libraries and model assets; verify runtime loading, camera permission and installation on a clean target Mac. |
| Data agent | Finalize migration/backup/restore, media retention and database-plus-evidence consistency. |
| QA/documentation agent | Repeat the critical packaged-app flows and publish placement guide, review guidance and accuracy report. |

CPU inference remains the correctness reference. Adopt CoreML only if parity and target-hardware performance pass. Declare Apple Silicon support first if that matches the actual MacBook; Intel or additional macOS versions require separate qualification. Signing/notarization and its account requirements depend on the chosen distribution channel; local development and published distribution are separate milestones.

**Gate:** the packaged application completes the same recording, review, restart, replay and export flows without development-only paths or services.

### Later phases — Expand only after precision is established

Custom initial FEN positions, automated board localization, broader piece-set support, faster play, optional external cameras, clock recognition and engine analysis can follow. Each changes the supported conditions and needs new fixtures and accuracy gates. Cloud sync/accounts are optional later product work; the first release uses local storage.

## Data and timing contracts

| Record | Required content |
| --- | --- |
| Game | ID, players, UTC date, result, initial FEN, recording/completeness status, active revision, optional event/site/time-control metadata. |
| Capture session | Game ID, session/clock epoch, UTC anchor, camera identity/format, timestamp source, calibration/model versions, pause/outage spans. |
| Observation | Frame sequence, capture timestamp, square likelihoods, visibility/motion/quality flags and observation version. Keep high-volume transient data bounded; persist the observations needed for evidence and recovery. |
| Move | Game/revision/ply, side, canonical move/UCI, derived SAN, FEN before/after, evidence references, confidence, provenance (`automatic`, `reviewed`, `inferred`, `manual`). |
| Timing | Observation estimate when justified, lower/upper completion bounds, elapsed estimate/bounds, confirmation time, and quality (`observed`, `bounded`, `unknown`). |
| Event/review/revision | Original decision, alternatives, gap reason, reviewer correction, affected suffix and parent revision. Original evidence/history remains auditable. |
| Evidence | File reference, hash, capture interval, retention status and associated events/plies. Stage files and reconcile manifests so interrupted writes do not create false evidence claims. |

Store elapsed durations using integer units, such as microseconds, and UTC for display/session identity. Normalize device presentation timestamps into a session monotonic timeline when available. If only host acquisition timestamps are available, record that fallback and characterize capture delay. Do not compute durations from wall-clock differences or inference completion times.

Define a move boundary as the physical completion of the full chess transition, estimated from the first clear settled evidence of its resulting position. Preserve bounds from the last trusted pre-move evidence to the first trusted completed-position evidence. The estimate and interval describe what the camera observed; debounce/confirmation latency is a separate field.

For two move-completion intervals `[L_previous, U_previous]` and `[L_current, U_current]`, the elapsed interval is `[L_current - U_previous, U_current - L_previous]`, constrained by chronological ordering. A point estimate is optional. The first move has elapsed time only when a reliable game-start marker exists. Across gaps, restarts or unobserved play, individual move times can remain unknown.

If several moves occur while hidden, bounded legal sequence search may offer reconstruction candidates. Even a unique candidate is stored as inferred; it does not acquire invented timestamps. Multiple indistinguishable histories require review. Pause/resume marks an observation gap and never implies that players stopped playing.

This elapsed interval is the time between observed move completions. It is not a remaining clock reading or necessarily official thinking time.

## Chess.com export contract

Export clean PGN with the standard Event, Site, Date, Round, White, Black and Result tags, legal SAN movetext and the matching termination marker. Future nonstandard starts require `SetUp` and `FEN`. Validate exports against the active trusted revision and an independent parser. [PGN specification](https://www.saremba.de/chessgml/standards/pgn/pgn-complete.htm)

Offer an annotated export with ordinary comments, for example `1. e4 {elapsed 2.340s; timing estimated}`. A comment is descriptive metadata, not a guarantee that Chess.com preserves or displays it. Use detailed JSON/CSV sidecars for exact timings, bounds, confidence and evidence identifiers. Never encode elapsed duration as a `[%clk]` remaining-clock value.

The compatibility milestone is loading exported games into Chess.com analysis and verifying the moves. Direct account synchronization is future scope and is not assumed by this plan. Incomplete games can export only their valid trusted prefix with result `*` and a clear incompleteness note.

## First implementation wave

Start with Frames 1 and 2 of Phase 0 using `Sol/M`: establish the toolchain, package a minimal camera probe, document a practical built-in-camera placement, and produce annotated reference clips. An optional `Luna/H` helper prepares documentation or annotation tooling. The resulting evidence determines the supported setup, initial model choice and achievable timing precision, and supplies the first measured work-frame estimate.
