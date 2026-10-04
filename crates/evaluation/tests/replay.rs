use chess_core::ChessGame;
use chess_evaluation::{
    CompletionAnnotation, DatasetManifest, DatasetSplit, FrameTimestamp, ReferenceMove,
    replay::{ObservationTrace, ReplayConfig, TraceFrame, replay},
};
use contracts::{CaptureTimeUs, FrameObservation, SessionId, SquareEvidence};
use recorder_core::DecoderConfig;

fn observation(game: &ChessGame, id: SessionId, sequence: u64, time: i64) -> FrameObservation {
    FrameObservation {
        session_id: id,
        sequence,
        capture_time: CaptureTimeUs::new(time).unwrap(),
        moving: false,
        calibration_version: "test-corners".into(),
        model_version: "synthetic-test-not-camera".into(),
        squares: game
            .piece_classes()
            .into_iter()
            .map(|(square, piece)| {
                let mut p = [0.00001; 12];
                let empty = if let Some(piece) = piece {
                    p[piece.index()] = 0.99988;
                    0.00001
                } else {
                    0.99988
                };
                SquareEvidence {
                    square,
                    visible_probability: 1.0,
                    empty_probability: empty,
                    piece_probabilities: p,
                }
            })
            .collect(),
    }
}
fn fixture(
    initial: &str,
    predicted: &[&str],
    reference: &[&str],
) -> (DatasetManifest, ObservationTrace, ReplayConfig) {
    let mut manifest: DatasetManifest =
        serde_json::from_str(include_str!("../examples/manifest.example.json")).unwrap();
    let session = &mut manifest.sessions[0];
    session.media.sha256 = Some("a".repeat(64));
    session.reference_game.initial_position = initial.into();
    session.reference_game.moves = reference
        .iter()
        .enumerate()
        .map(|(i, uci)| ReferenceMove {
            ply: i as u32 + 1,
            uci: (*uci).into(),
            completion: CompletionAnnotation::Bounded {
                earliest_capture_us: 150_000 + i as u64 * 200_000,
                latest_capture_us: 190_000 + i as u64 * 200_000,
            },
        })
        .collect();
    let mut trace = ObservationTrace {
        schema_version: 1,
        kind: "chess-observation-trace".into(),
        dataset_id: manifest.dataset_id.clone(),
        session_id: session.session_id.clone(),
        source_sha256: "a".repeat(64),
        model_version: "synthetic-test-not-camera".into(),
        calibration_version: "test-corners".into(),
        pipeline_version: "offline-browser-v1".into(),
        motion_threshold: 10.0,
        frames: vec![],
    };
    let mut game = if initial == "startpos" {
        ChessGame::standard()
    } else {
        ChessGame::from_fen(initial).unwrap()
    };
    let id = SessionId::new();
    for _ in 0..2 {
        push(&mut trace, &game, id);
    }
    for uci in predicted {
        game.play_uci(uci).unwrap();
        for _ in 0..2 {
            push(&mut trace, &game, id);
        }
    }
    session.frames = trace
        .frames
        .iter()
        .map(|f| FrameTimestamp {
            frame_index: f.frame_index,
            capture_us: f.observation.capture_time.get() as u64,
        })
        .collect();
    let config = ReplayConfig {
        decoder: DecoderConfig {
            settle_time_us: 0,
            consistent_frames: 2,
            ..DecoderConfig::default()
        },
        max_capture_gap_us: 1_000_000,
    };
    (manifest, trace, config)
}
fn push(trace: &mut ObservationTrace, game: &ChessGame, id: SessionId) {
    let i = trace.frames.len() as u64;
    trace.frames.push(TraceFrame {
        frame_index: i,
        image_sha256: "b".repeat(64),
        inference_ms: 200,
        observation: observation(game, id, i, i as i64 * 100_000),
    });
}
#[test]
fn deterministic_blind_replay_matches_history_and_conservative_time_bounds() {
    let (manifest, trace, config) = fixture("startpos", &["e2e4", "e7e5"], &["e2e4", "e7e5"]);
    let report = replay(&manifest, &trace, config.clone()).unwrap();
    assert_eq!(report, replay(&manifest, &trace, config).unwrap());
    assert_eq!(report.replay_precision, Some(1.0));
    assert_eq!(report.replay_coverage, Some(1.0));
    assert!(report.exact_recorded_history);
    assert_eq!(report.timing.prediction_contains_reference, 2);
    assert_eq!(report.moves[0].completion.earliest.get(), 100_000);
    assert_eq!(report.moves[0].completion.latest.get(), 200_000);
    assert_eq!(report.moves[0].confirmation_capture_us, 300_000);
}
#[test]
fn wrong_legal_moves_are_committed_and_counted_without_reference_based_repair() {
    let (manifest, trace, config) = fixture("startpos", &["d2d4", "d7d5"], &["e2e4", "e7e5"]);
    let report = replay(&manifest, &trace, config).unwrap();
    assert_eq!(report.replayed_commits, 2);
    assert_eq!(report.correct_commits, 0);
    assert_eq!(report.incorrect_commits, 2);
    assert_eq!(report.moves[0].uci, "d2d4");
    assert_eq!(report.moves[1].uci, "d7d5");
    assert_eq!(report.replay_coverage, Some(0.0));
    assert!(!report.exact_recorded_history);
}
#[test]
fn extra_commits_remain_errors_and_unchanged_frames_never_duplicate_a_move() {
    let (manifest, trace, config) = fixture("startpos", &["e2e4", "e7e5"], &["e2e4"]);
    let report = replay(&manifest, &trace, config).unwrap();
    assert_eq!(report.correct_commits, 1);
    assert_eq!(report.incorrect_commits, 1);
    assert_eq!(report.replay_precision, Some(0.5));
    assert_eq!(report.replay_coverage, Some(1.0));
    assert!(!report.exact_recorded_history);
    let (mut manifest, mut trace, config) = fixture("startpos", &["e2e4"], &["e2e4"]);
    let mut game = ChessGame::standard();
    game.play_uci("e2e4").unwrap();
    let id = trace.frames[0].observation.session_id;
    for _ in 0..4 {
        push(&mut trace, &game, id);
    }
    manifest.sessions[0].frames = trace
        .frames
        .iter()
        .map(|f| FrameTimestamp {
            frame_index: f.frame_index,
            capture_us: f.observation.capture_time.get() as u64,
        })
        .collect();
    assert_eq!(
        replay(&manifest, &trace, config).unwrap().replayed_commits,
        1
    );
}
#[test]
fn native_frame_gaps_and_long_capture_gaps_latch_without_silent_resynchronization() {
    for sequence_gap in [true, false] {
        let (mut manifest, mut trace, config) =
            fixture("startpos", &["e2e4", "e7e5"], &["e2e4", "e7e5"]);
        for f in &mut trace.frames[2..] {
            if sequence_gap {
                f.frame_index += 1;
                f.observation.sequence += 1;
            } else {
                f.observation.capture_time =
                    CaptureTimeUs::new(f.observation.capture_time.get() + 2_000_000).unwrap();
            }
        }
        manifest.sessions[0].frames = trace
            .frames
            .iter()
            .map(|f| FrameTimestamp {
                frame_index: f.frame_index,
                capture_us: f.observation.capture_time.get() as u64,
            })
            .collect();
        let report = replay(&manifest, &trace, config).unwrap();
        assert_eq!(report.replayed_commits, 0);
        assert_eq!(report.replay_precision, None);
        assert_eq!(report.replay_coverage, Some(0.0));
        assert_eq!(report.discontinuity_frames, 4);
    }
}
#[test]
fn inference_backlog_does_not_change_completion_or_confirmation_capture_time() {
    let (manifest, mut trace, config) = fixture("startpos", &["e2e4"], &["e2e4"]);
    let first = replay(&manifest, &trace, config.clone()).unwrap();
    for frame in &mut trace.frames {
        frame.inference_ms = 10_000;
    }
    let delayed = replay(&manifest, &trace, config).unwrap();
    assert_eq!(first.moves, delayed.moves);
    assert_eq!(first.timing, delayed.timing);
    assert_eq!(delayed.inference_p95_ms, Some(10_000));
}
#[test]
fn premature_commit_unknown_reference_timing_and_interval_overlap_are_distinct() {
    let (mut manifest, trace, config) = fixture("startpos", &["e2e4"], &["e2e4"]);
    manifest.sessions[0].reference_game.moves[0].completion = CompletionAnnotation::Bounded {
        earliest_capture_us: 300_000,
        latest_capture_us: 300_000,
    };
    let report = replay(&manifest, &trace, config.clone()).unwrap();
    assert_eq!(report.timing.disjoint_intervals, 1);
    manifest.sessions[0].reference_game.moves[0].completion = CompletionAnnotation::Bounded {
        earliest_capture_us: 150_000,
        latest_capture_us: 250_000,
    };
    assert_eq!(
        replay(&manifest, &trace, config.clone())
            .unwrap()
            .timing
            .intervals_overlap_only,
        1
    );
    manifest.sessions[0].reference_game.moves[0].completion = CompletionAnnotation::Unknown {
        reason: "hand covers completion".into(),
    };
    let unknown = replay(&manifest, &trace, config.clone()).unwrap();
    assert_eq!(unknown.timing.reference_unknown, 1);
    assert_eq!(unknown.timing.compared_bounded_reference, 0);
    let mut early = trace.clone();
    early.frames[2].observation.capture_time = CaptureTimeUs::new(100_001).unwrap();
    early.frames[3].observation.capture_time = CaptureTimeUs::new(100_002).unwrap();
    manifest.sessions[0].frames[2].capture_us = 100_001;
    manifest.sessions[0].frames[3].capture_us = 100_002;
    // To keep annotation bounds inside the session, a later unchanged frame closes it.
    let mut game = ChessGame::standard();
    game.play_uci("e2e4").unwrap();
    let id = early.frames[0].observation.session_id;
    push(&mut early, &game, id);
    manifest.sessions[0].frames.push(FrameTimestamp {
        frame_index: 4,
        capture_us: 400_000,
    });
    manifest.sessions[0].reference_game.moves[0].completion = CompletionAnnotation::Bounded {
        earliest_capture_us: 150_000,
        latest_capture_us: 190_000,
    };
    let report = replay(&manifest, &early, config).unwrap();
    assert_eq!(report.correct_commits, 0);
    assert!(report.moves[0].accepted_before_reference_completion);
}
#[test]
fn special_moves_and_every_promotion_use_the_rules_and_observed_piece_identity() {
    let cases = [
        ("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", "e1g1"),
        ("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"),
        ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"),
        ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8r"),
        ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8b"),
        ("4k3/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8n"),
    ];
    for (fen, uci) in cases {
        let (manifest, trace, config) = fixture(fen, &[uci], &[uci]);
        let report = replay(&manifest, &trace, config).unwrap();
        assert_eq!(report.correct_commits, 1, "{uci}");
    }
}
#[test]
fn invalid_media_identity_timestamps_probabilities_reference_and_qualification_are_rejected() {
    let (manifest, trace, config) = fixture("startpos", &["e2e4"], &["e2e4"]);
    let mut wrong = trace.clone();
    wrong.source_sha256 = "c".repeat(64);
    assert!(replay(&manifest, &wrong, config.clone()).is_err());
    wrong = trace.clone();
    wrong.frames[0].observation.capture_time = CaptureTimeUs::new(1).unwrap();
    assert!(replay(&manifest, &wrong, config.clone()).is_err());
    wrong = trace.clone();
    wrong.frames[0].observation.squares[0].empty_probability = 1.0;
    assert!(replay(&manifest, &wrong, config.clone()).is_err());
    wrong = trace.clone();
    wrong.frames.pop();
    assert!(replay(&manifest, &wrong, config.clone()).is_err());
    let mut illegal = manifest.clone();
    illegal.sessions[0].reference_game.moves[0].uci = "e2e5".into();
    assert!(replay(&illegal, &trace, config.clone()).is_err());
    let mut qualification = manifest;
    qualification.sessions[0].split = DatasetSplit::Qualification;
    assert!(
        replay(&qualification, &trace, config)
            .unwrap_err()
            .to_string()
            .contains("frozen candidate")
    );
}
#[test]
fn no_decisions_does_not_invent_perfect_precision_or_coverage() {
    let (manifest, trace, config) = fixture("startpos", &[], &[]);
    let report = replay(&manifest, &trace, config).unwrap();
    assert_eq!(report.replay_precision, None);
    assert_eq!(report.replay_coverage, None);
}
