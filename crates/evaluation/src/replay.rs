//! Blind observation playback: annotations are used for scoring, never decoding.
//! Development reports are not the original-decision release qualification audit.
use crate::{CompletionAnnotation, DatasetManifest, DatasetSplit, TimestampSource};
use anyhow::{Context, Result, ensure};
use chess_core::ChessGame;
use contracts::{FrameObservation, TimeBounds};
use recorder_core::{DecoderConfig, DecoderDecision, TemporalDecoder};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationTrace {
    pub schema_version: u32,
    pub kind: String,
    pub dataset_id: String,
    /// The annotation entry ID, separate from FrameObservation's capture UUID.
    pub session_id: String,
    pub source_sha256: String,
    pub model_version: String,
    pub calibration_version: String,
    pub pipeline_version: String,
    pub motion_threshold: f32,
    pub frames: Vec<TraceFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceFrame {
    pub frame_index: u64,
    pub image_sha256: String,
    pub inference_ms: u32,
    pub observation: FrameObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayConfig {
    pub decoder: DecoderConfig,
    /// Long acquisition gaps latch review, independently of inference cost.
    pub max_capture_gap_us: i64,
}
impl Default for ReplayConfig {
    fn default() -> Self {
        Self {
            decoder: DecoderConfig {
                settle_time_us: 900_000,
                consistent_frames: 3,
                ..DecoderConfig::default()
            },
            max_capture_gap_us: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayReport {
    pub schema_version: u32,
    pub report_kind: String,
    pub dataset_id: String,
    pub session_id: String,
    pub split: DatasetSplit,
    pub timestamp_source: TimestampSource,
    pub source_sha256: String,
    pub model_version: String,
    pub pipeline_version: String,
    pub motion_threshold: f32,
    pub config: ReplayConfig,
    pub reference_plies: usize,
    pub replayed_commits: usize,
    pub correct_commits: usize,
    pub incorrect_commits: usize,
    /// Null when no commits exist, rather than an invented 100% precision.
    pub replay_precision: Option<f64>,
    pub replay_coverage: Option<f64>,
    pub exact_recorded_history: bool,
    pub all_replayed_commits_correct: bool,
    pub review_frames: usize,
    pub discontinuity_frames: usize,
    pub inference_p95_ms: Option<u32>,
    pub timing: TimingSummary,
    pub moves: Vec<ScoredMove>,
    pub frames: Vec<ReplayFrame>,
    pub final_fen: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct TimingSummary {
    pub compared_bounded_reference: usize,
    pub reference_unknown: usize,
    pub prediction_contains_reference: usize,
    pub intervals_overlap_only: usize,
    pub disjoint_intervals: usize,
    pub completion_width_p95_us: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScoredMove {
    pub ply: u32,
    pub uci: String,
    pub san: String,
    pub fen_after: String,
    pub reference_uci: Option<String>,
    pub prefix_matches: bool,
    pub correct: bool,
    pub accepted_before_reference_completion: bool,
    pub completion: TimeBounds,
    pub confirmation_capture_us: i64,
    pub timing_comparison: String,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayFrame {
    pub frame_index: u64,
    pub capture_us: i64,
    pub kind: String,
    pub reason: Option<String>,
    pub committed_uci: Option<String>,
}

fn hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn position(initial: &str) -> Result<ChessGame> {
    if initial == "startpos" {
        Ok(ChessGame::standard())
    } else {
        Ok(ChessGame::from_fen(initial)?)
    }
}
fn percentile<T: Copy + Ord>(values: &mut [T]) -> Option<T> {
    if values.is_empty() {
        return None;
    }
    values.sort();
    Some(values[(values.len() * 95).div_ceil(100) - 1])
}

pub fn replay(
    manifest: &DatasetManifest,
    trace: &ObservationTrace,
    config: ReplayConfig,
) -> Result<ReplayReport> {
    manifest
        .validate()
        .map_err(|errors| anyhow::anyhow!("Invalid annotation manifest: {errors:?}"))?;
    config.decoder.validate()?;
    ensure!(
        config.max_capture_gap_us > 0,
        "Capture gap limit must be positive."
    );
    ensure!(
        trace.schema_version == 1 && trace.kind == "chess-observation-trace",
        "Unsupported observation trace format."
    );
    ensure!(
        trace.dataset_id == manifest.dataset_id,
        "Trace dataset differs from annotations."
    );
    ensure!(
        hash(&trace.source_sha256),
        "Trace source media hash is invalid."
    );
    ensure!(
        !trace.model_version.trim().is_empty() && !trace.calibration_version.trim().is_empty(),
        "Trace model/calibration identity is missing."
    );
    ensure!(
        trace.pipeline_version == "offline-browser-v1",
        "Unsupported observation pipeline."
    );
    ensure!(
        trace.motion_threshold.is_finite() && (6.0..=24.0).contains(&trace.motion_threshold),
        "Invalid motion threshold."
    );
    let session = manifest
        .sessions
        .iter()
        .find(|s| s.session_id == trace.session_id)
        .context("Trace annotation session not found.")?;
    ensure!(
        session.split != DatasetSplit::Qualification,
        "Development replay cannot qualify release data; use the frozen candidate and original-decision audit."
    );
    if let Some(expected) = &session.media.sha256 {
        ensure!(
            expected.eq_ignore_ascii_case(&trace.source_sha256),
            "Source media hash differs from annotations."
        );
    }
    ensure!(
        !trace.frames.is_empty()
            && trace.frames.len() <= 2000
            && trace.frames.len() == session.frames.len(),
        "Trace must include every annotated frame (at most 2000)."
    );
    // Validate the complete input before producing any partial measurement.
    let capture_id = trace.frames[0].observation.session_id;
    for (f, expected) in trace.frames.iter().zip(&session.frames) {
        f.observation.validate()?;
        ensure!(hash(&f.image_sha256), "Invalid frame image hash.");
        ensure!(
            f.frame_index == expected.frame_index && f.observation.sequence == expected.frame_index,
            "Trace frame order/index differs from annotations."
        );
        ensure!(
            u64::try_from(f.observation.capture_time.get())? == expected.capture_us,
            "Observation time differs from annotated acquisition time."
        );
        ensure!(
            f.observation.session_id == capture_id
                && f.observation.model_version == trace.model_version
                && f.observation.calibration_version == trace.calibration_version,
            "Trace changes session, model or calibration; split it into separate sessions."
        );
        for square in &f.observation.squares {
            let total = square.empty_probability + square.piece_probabilities.iter().sum::<f32>();
            ensure!(
                (total - 1.0).abs() < 0.005,
                "Frame {} has unnormalized square evidence.",
                f.frame_index
            );
        }
    }
    // References are validated separately and never fed into the decoder.
    let mut reference = position(&session.reference_game.initial_position)?;
    for m in &session.reference_game.moves {
        reference
            .play_uci(&m.uci)
            .with_context(|| format!("Illegal reference at ply {}", m.ply))?;
    }
    let mut game = position(&session.reference_game.initial_position)?;
    let mut decoder = TemporalDecoder::new(config.decoder)?;
    let mut prefix_matches = true;
    let mut moves = Vec::new();
    let mut frames = Vec::new();
    let mut latched_gap = false;
    let mut previous_capture = None;
    let mut review_frames = 0;
    let mut discontinuity_frames = 0;
    let mut timing = TimingSummary::default();
    let mut widths = Vec::new();
    for f in &trace.frames {
        let o = &f.observation;
        let mut entry = ReplayFrame {
            frame_index: f.frame_index,
            capture_us: o.capture_time.get(),
            kind: String::new(),
            reason: None,
            committed_uci: None,
        };
        if previous_capture.is_some_and(|t| o.capture_time.get() - t > config.max_capture_gap_us) {
            latched_gap = true;
        }
        previous_capture = Some(o.capture_time.get());
        if latched_gap {
            entry.kind = "discontinuity".into();
            entry.reason = Some("Acquisition gap exceeded the configured limit; no automatic reset or reference repair.".into());
            discontinuity_frames += 1;
        } else {
            match decoder.ingest(&game, o) {
                Ok(DecoderDecision::NoMove) => {
                    entry.kind = if o.moving { "moving" } else { "unchanged" }.into()
                }
                Ok(DecoderDecision::Settling) => entry.kind = "settling".into(),
                Ok(DecoderDecision::NeedsReview { reason, .. }) => {
                    entry.kind = "review".into();
                    entry.reason = Some(reason);
                    review_frames += 1;
                }
                Ok(DecoderDecision::Proposed(proposed)) => {
                    let index = moves.len();
                    let expected = session.reference_game.moves.get(index);
                    prefix_matches &= expected.is_some_and(|m| m.uci == proposed.uci);
                    let early = expected.is_some_and(|m| matches!(&m.completion, CompletionAnnotation::Bounded { earliest_capture_us, .. } if proposed.confirmation_time.get() < *earliest_capture_us as i64));
                    let correct = prefix_matches && !early;
                    let comparison = if correct {
                        match expected.map(|m| &m.completion) {
                            Some(CompletionAnnotation::Bounded {
                                earliest_capture_us,
                                latest_capture_us,
                            }) => {
                                timing.compared_bounded_reference += 1;
                                let lower = proposed.completion_bounds.earliest.get() as u64;
                                let upper = proposed.completion_bounds.latest.get() as u64;
                                if lower <= *earliest_capture_us && upper >= *latest_capture_us {
                                    timing.prediction_contains_reference += 1;
                                    "contains_reference"
                                } else if lower <= *latest_capture_us
                                    && upper >= *earliest_capture_us
                                {
                                    timing.intervals_overlap_only += 1;
                                    "overlap_only"
                                } else {
                                    timing.disjoint_intervals += 1;
                                    "disjoint"
                                }
                            }
                            _ => {
                                timing.reference_unknown += 1;
                                "reference_unknown"
                            }
                        }
                    } else {
                        "incorrect_move_not_compared"
                    };
                    widths.push(
                        proposed.completion_bounds.latest.get()
                            - proposed.completion_bounds.earliest.get(),
                    );
                    // Commit what the decoder proposed, including errors. No truth-based reset.
                    let applied = game.play_uci(&proposed.uci)?.clone();
                    decoder.commit(&game)?;
                    entry.kind = "committed".into();
                    entry.committed_uci = Some(proposed.uci.clone());
                    moves.push(ScoredMove {
                        ply: index as u32 + 1,
                        uci: proposed.uci,
                        san: applied.san,
                        fen_after: applied.fen_after,
                        reference_uci: expected.map(|m| m.uci.clone()),
                        prefix_matches,
                        correct,
                        accepted_before_reference_completion: early,
                        completion: proposed.completion_bounds,
                        confirmation_capture_us: proposed.confirmation_time.get(),
                        timing_comparison: comparison.into(),
                    });
                }
                Ok(DecoderDecision::AwaitingCommit(_)) => {
                    anyhow::bail!("Replay encountered an uncommitted proposal.")
                }
                Err(error) => {
                    entry.kind = "discontinuity".into();
                    entry.reason = Some(error.to_string());
                    discontinuity_frames += 1;
                    latched_gap = true;
                }
            }
        }
        frames.push(entry);
    }
    timing.completion_width_p95_us = percentile(&mut widths);
    let count = moves.len();
    let correct = moves.iter().filter(|m| m.correct).count();
    let reference_count = session.reference_game.moves.len();
    let mut latencies: Vec<_> = trace.frames.iter().map(|f| f.inference_ms).collect();
    Ok(ReplayReport { schema_version: 1, report_kind: "development_decoder_replay".into(), dataset_id: trace.dataset_id.clone(), session_id: trace.session_id.clone(), split: session.split,
        timestamp_source: session.timestamp_source, source_sha256: trace.source_sha256.clone(), model_version: trace.model_version.clone(), pipeline_version: trace.pipeline_version.clone(), motion_threshold: trace.motion_threshold, config,
        reference_plies: reference_count, replayed_commits: count, correct_commits: correct, incorrect_commits: count - correct,
        replay_precision: (count > 0).then(|| correct as f64 / count as f64), replay_coverage: (reference_count > 0).then(|| correct as f64 / reference_count as f64),
        exact_recorded_history: count == reference_count && prefix_matches, all_replayed_commits_correct: correct == count,
        review_frames, discontinuity_frames, inference_p95_ms: percentile(&mut latencies), timing, moves, frames, final_fen: game.fen(),
        limitations: vec!["Development replay is not an audit of original live automatic decisions or a release qualification result.".into(),
            "Timing comparisons use capture-time intervals; unknown reference timing receives no point estimate.".into(),
            "Media/frame hashes identify asserted inputs; verify the prepared bundle and original media separately.".into()] })
}
