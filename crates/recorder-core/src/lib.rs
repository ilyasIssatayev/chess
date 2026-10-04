use std::cmp::Ordering;
use std::collections::HashMap;

use chess_core::{ChessError, ChessGame};
use contracts::{
    CaptureTimeUs, ContractError, FrameObservation, PieceClass, SessionId, TimeBounds,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecoderConfig {
    pub settle_time_us: i64,
    pub min_visible_squares: usize,
    /// Reject a candidate if any visible square assigns less probability to
    /// its expected state. This prevents a good board-wide average from
    /// hiding a small number of decisive contradictions.
    pub min_expected_square_probability: f32,
    pub min_average_log_likelihood: f64,
    pub min_margin: f64,
    pub consistent_frames: u32,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            settle_time_us: 250_000,
            min_visible_squares: 56,
            min_expected_square_probability: 0.10,
            min_average_log_likelihood: -0.75,
            min_margin: 0.10,
            consistent_frames: 2,
        }
    }
}

impl DecoderConfig {
    pub fn validate(self) -> Result<Self, DecoderConfigError> {
        if self.settle_time_us < 0 {
            return Err(DecoderConfigError::NegativeSettleTime);
        }
        if !(1..=contracts::SQUARE_COUNT).contains(&self.min_visible_squares) {
            return Err(DecoderConfigError::InvalidVisibleSquareCount);
        }
        if !self.min_expected_square_probability.is_finite()
            || !(0.0..=1.0).contains(&self.min_expected_square_probability)
            || self.min_expected_square_probability == 0.0
        {
            return Err(DecoderConfigError::InvalidExpectedSquareProbability);
        }
        if !self.min_average_log_likelihood.is_finite() || self.min_average_log_likelihood > 0.0 {
            return Err(DecoderConfigError::InvalidAverageLogLikelihood);
        }
        if !self.min_margin.is_finite() || self.min_margin <= 0.0 {
            return Err(DecoderConfigError::InvalidMargin);
        }
        if self.consistent_frames == 0 {
            return Err(DecoderConfigError::ZeroConsistentFrames);
        }
        Ok(self)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DecoderConfigError {
    #[error("settle time cannot be negative")]
    NegativeSettleTime,
    #[error("minimum visible-square count must be between 1 and 64")]
    InvalidVisibleSquareCount,
    #[error("minimum expected-square probability must be finite and in (0, 1]")]
    InvalidExpectedSquareProbability,
    #[error("minimum average log likelihood must be finite and at most zero")]
    InvalidAverageLogLikelihood,
    #[error("minimum candidate margin must be finite and greater than zero")]
    InvalidMargin,
    #[error("consistent frame count must be at least one")]
    ZeroConsistentFrames,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderState {
    Calibrating,
    Stable,
    Disturbed,
    Settling,
    Candidate,
    AwaitingCommit,
    Uncertain,
    RecalibrationRequired,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProposedMove {
    pub uci: String,
    pub fen_after: String,
    pub average_log_likelihood: f64,
    pub margin: f64,
    pub completion_bounds: TimeBounds,
    pub confirmation_time: CaptureTimeUs,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecoderDecision {
    NoMove,
    Settling,
    NeedsReview {
        reason: String,
        candidates: Vec<(String, f64)>,
    },
    Proposed(ProposedMove),
    AwaitingCommit(ProposedMove),
}

#[derive(Debug, Clone)]
struct PendingCandidate {
    proposed: ProposedMove,
    frames: u32,
}

#[derive(Debug, Clone)]
pub struct TemporalDecoder {
    config: DecoderConfig,
    state: RecorderState,
    stable_since: Option<CaptureTimeUs>,
    last_trusted_unchanged: Option<CaptureTimeUs>,
    pending: Option<PendingCandidate>,
    awaiting: Option<ProposedMove>,
    session_id: Option<SessionId>,
    last_sequence: Option<u64>,
    last_capture_time: Option<CaptureTimeUs>,
    calibration_version: Option<String>,
    model_version: Option<String>,
    requires_reset: bool,
}

impl TemporalDecoder {
    pub fn new(config: DecoderConfig) -> Result<Self, DecoderConfigError> {
        let config = config.validate()?;
        Ok(Self {
            config,
            state: RecorderState::Calibrating,
            stable_since: None,
            last_trusted_unchanged: None,
            pending: None,
            awaiting: None,
            session_id: None,
            last_sequence: None,
            last_capture_time: None,
            calibration_version: None,
            model_version: None,
            requires_reset: false,
        })
    }

    pub fn state(&self) -> &RecorderState {
        &self.state
    }

    /// Withdraw uncommitted evidence when the board changes during review.
    /// Keep the stream identity and last trusted position/time intact.
    pub fn invalidate_candidate(&mut self) {
        self.pending = None;
        self.awaiting = None;
        self.stable_since = None;
        self.state = RecorderState::Disturbed;
    }

    pub fn ingest(
        &mut self,
        game: &ChessGame,
        observation: &FrameObservation,
    ) -> Result<DecoderDecision, DecoderError> {
        observation.validate()?;
        self.validate_stream(observation)?;
        if let Some(proposed) = &self.awaiting {
            return Ok(DecoderDecision::AwaitingCommit(proposed.clone()));
        }

        if observation.moving {
            self.state = RecorderState::Disturbed;
            self.stable_since = None;
            self.pending = None;
            return Ok(DecoderDecision::NoMove);
        }

        let stable_since = *self.stable_since.get_or_insert(observation.capture_time);
        let settled_for = observation.capture_time.get() - stable_since.get();
        if settled_for < self.config.settle_time_us {
            self.state = RecorderState::Settling;
            return Ok(DecoderDecision::Settling);
        }

        let ranked = rank_positions(
            game,
            observation,
            self.config.min_visible_squares,
            self.config.min_expected_square_probability,
        )?;
        let Some(best) = ranked.first() else {
            self.state = RecorderState::Uncertain;
            return Ok(DecoderDecision::NeedsReview {
                reason: "no candidate position had enough visible evidence".into(),
                candidates: vec![],
            });
        };
        let second_score = ranked
            .get(1)
            .map(|candidate| candidate.score)
            .unwrap_or(f64::NEG_INFINITY);
        let margin = best.score - second_score;

        if !best.score.is_finite() {
            self.state = RecorderState::Uncertain;
            self.pending = None;
            return Ok(DecoderDecision::NeedsReview {
                reason: "too few visible squares to evaluate the board".into(),
                candidates: vec![],
            });
        }

        if best.score < self.config.min_average_log_likelihood || margin < self.config.min_margin {
            self.state = RecorderState::Uncertain;
            self.pending = None;
            return Ok(DecoderDecision::NeedsReview {
                reason: format!(
                    "best candidate did not clear evidence thresholds (score {:.3}, margin {:.3})",
                    best.score, margin
                ),
                candidates: ranked
                    .iter()
                    .take(4)
                    .filter_map(|candidate| candidate.uci.clone().map(|uci| (uci, candidate.score)))
                    .collect(),
            });
        }

        let visible_square_count = observation
            .squares
            .iter()
            .filter(|square| square.visible_probability >= 0.5)
            .count();
        if best.uci.is_some() && visible_square_count != contracts::SQUARE_COUNT {
            self.state = RecorderState::Uncertain;
            self.pending = None;
            return Ok(DecoderDecision::NeedsReview {
                reason: format!(
                    "automatic move acceptance requires all 64 squares to exclude hidden replies; {visible_square_count} were visible"
                ),
                candidates: ranked
                    .iter()
                    .take(4)
                    .filter_map(|candidate| candidate.uci.clone().map(|uci| (uci, candidate.score)))
                    .collect(),
            });
        }

        if best.uci.is_none() {
            self.state = RecorderState::Stable;
            self.pending = None;
            self.last_trusted_unchanged = Some(observation.capture_time);
            return Ok(DecoderDecision::NoMove);
        }

        let uci = best.uci.clone().expect("unchanged position handled above");
        let first_supported = self
            .pending
            .as_ref()
            .filter(|candidate| candidate.proposed.uci == uci)
            .map(|candidate| candidate.proposed.completion_bounds.latest)
            .unwrap_or(observation.capture_time);
        let Some(earliest) = self.last_trusted_unchanged else {
            self.enter_uncertain_state();
            return Ok(DecoderDecision::NeedsReview {
                reason: "no trusted pre-move observation exists for a safe completion bound".into(),
                candidates: ranked
                    .iter()
                    .take(4)
                    .filter_map(|candidate| candidate.uci.clone().map(|uci| (uci, candidate.score)))
                    .collect(),
            });
        };
        let completion_bounds = TimeBounds::new(earliest, first_supported)?;
        let proposed = ProposedMove {
            uci,
            fen_after: best.fen.clone(),
            average_log_likelihood: best.score,
            margin,
            completion_bounds,
            confirmation_time: observation.capture_time,
        };

        let frames = self
            .pending
            .as_ref()
            .filter(|candidate| candidate.proposed.uci == proposed.uci)
            .map(|candidate| candidate.frames + 1)
            .unwrap_or(1);
        self.pending = Some(PendingCandidate {
            proposed: proposed.clone(),
            frames,
        });
        self.state = RecorderState::Candidate;

        if frames < self.config.consistent_frames.max(1) {
            return Ok(DecoderDecision::Settling);
        }

        self.state = RecorderState::AwaitingCommit;
        self.awaiting = Some(proposed.clone());
        Ok(DecoderDecision::Proposed(proposed))
    }

    pub fn commit(&mut self, game_after: &ChessGame) -> Result<(), DecoderError> {
        let expected = self.awaiting.as_ref().ok_or(DecoderError::NoPendingMove)?;
        let actual = game_after.fen();
        if expected.fen_after != actual {
            return Err(DecoderError::CommitPositionMismatch {
                expected: expected.fen_after.clone(),
                actual,
            });
        }
        self.last_trusted_unchanged = Some(expected.confirmation_time);
        self.awaiting = None;
        self.pending = None;
        self.stable_since = None;
        self.state = RecorderState::Stable;
        Ok(())
    }

    /// Clears pending evidence and stream identity before a new capture
    /// session or an explicitly approved calibration/model change.
    pub fn reset(&mut self) {
        self.state = RecorderState::Calibrating;
        self.stable_since = None;
        self.last_trusted_unchanged = None;
        self.pending = None;
        self.awaiting = None;
        self.session_id = None;
        self.last_sequence = None;
        self.last_capture_time = None;
        self.calibration_version = None;
        self.model_version = None;
        self.requires_reset = false;
    }

    fn validate_stream(&mut self, observation: &FrameObservation) -> Result<(), DecoderError> {
        if self.requires_reset {
            return Err(DecoderError::ResetRequired);
        }
        if let Some(expected) = self.session_id
            && expected != observation.session_id
        {
            self.latch_uncertain_state();
            return Err(DecoderError::SessionChanged);
        }
        if let Some(previous) = self.last_sequence {
            if observation.sequence <= previous {
                self.latch_uncertain_state();
                return Err(DecoderError::NonIncreasingSequence {
                    previous,
                    current: observation.sequence,
                });
            }
            let expected = previous.saturating_add(1);
            if observation.sequence != expected {
                self.latch_uncertain_state();
                return Err(DecoderError::SequenceGap {
                    expected,
                    current: observation.sequence,
                });
            }
        }
        if let Some(previous) = self.last_capture_time
            && observation.capture_time <= previous
        {
            self.latch_uncertain_state();
            return Err(DecoderError::NonIncreasingCaptureTime {
                previous: previous.get(),
                current: observation.capture_time.get(),
            });
        }
        if let Some(expected) = &self.calibration_version
            && expected != &observation.calibration_version
        {
            let expected = expected.clone();
            self.enter_recalibration_state();
            return Err(DecoderError::CalibrationVersionChanged {
                expected,
                actual: observation.calibration_version.clone(),
            });
        }
        if let Some(expected) = &self.model_version
            && expected != &observation.model_version
        {
            let expected = expected.clone();
            self.enter_recalibration_state();
            return Err(DecoderError::ModelVersionChanged {
                expected,
                actual: observation.model_version.clone(),
            });
        }

        self.session_id = Some(observation.session_id);
        self.last_sequence = Some(observation.sequence);
        self.last_capture_time = Some(observation.capture_time);
        self.calibration_version
            .get_or_insert_with(|| observation.calibration_version.clone());
        self.model_version
            .get_or_insert_with(|| observation.model_version.clone());
        Ok(())
    }

    fn enter_uncertain_state(&mut self) {
        self.state = RecorderState::Uncertain;
        self.stable_since = None;
        self.last_trusted_unchanged = None;
        self.pending = None;
        self.awaiting = None;
    }

    fn enter_recalibration_state(&mut self) {
        self.latch_uncertain_state();
        self.state = RecorderState::RecalibrationRequired;
    }

    fn latch_uncertain_state(&mut self) {
        self.enter_uncertain_state();
        self.requires_reset = true;
    }
}

#[derive(Debug, Clone)]
struct RankedPosition {
    uci: Option<String>,
    fen: String,
    score: f64,
}

fn rank_positions(
    game: &ChessGame,
    observation: &FrameObservation,
    min_visible_squares: usize,
    min_expected_square_probability: f32,
) -> Result<Vec<RankedPosition>, DecoderError> {
    let evidence: HashMap<&str, _> = observation
        .squares
        .iter()
        .map(|square| (square.square.as_str(), square))
        .collect();
    if evidence.len() != observation.squares.len() {
        return Err(DecoderError::DuplicateSquareEvidence);
    }

    let mut candidates = Vec::new();
    candidates.push(RankedPosition {
        uci: None,
        fen: game.fen(),
        score: score_position(
            game.piece_classes(),
            &evidence,
            min_visible_squares,
            min_expected_square_probability,
        )?,
    });
    for uci in game.legal_uci_moves() {
        let mut next = game.clone();
        next.play_uci(&uci)?;
        candidates.push(RankedPosition {
            uci: Some(uci),
            fen: next.fen(),
            score: score_position(
                next.piece_classes(),
                &evidence,
                min_visible_squares,
                min_expected_square_probability,
            )?,
        });
    }
    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    Ok(candidates)
}

fn score_position(
    expected: Vec<(String, Option<PieceClass>)>,
    evidence: &HashMap<&str, &contracts::SquareEvidence>,
    min_visible_squares: usize,
    min_expected_square_probability: f32,
) -> Result<f64, DecoderError> {
    let mut score = 0.0;
    let mut visible = 0;
    for (square, expected_piece) in expected {
        let Some(observed) = evidence.get(square.as_str()) else {
            return Err(DecoderError::MissingSquareEvidence(square));
        };
        if observed.visible_probability < 0.5 {
            continue;
        }
        let expected_probability = observed.probability_for(expected_piece);
        if expected_probability < min_expected_square_probability {
            return Ok(f64::NEG_INFINITY);
        }
        let probability = expected_probability.max(1e-6) as f64;
        score += probability.ln();
        visible += 1;
    }
    if visible < min_visible_squares {
        return Ok(f64::NEG_INFINITY);
    }
    Ok(score / visible as f64)
}

#[derive(Debug, Error)]
pub enum DecoderError {
    #[error(transparent)]
    Contract(#[from] ContractError),
    #[error(transparent)]
    Chess(#[from] ChessError),
    #[error("duplicate square evidence")]
    DuplicateSquareEvidence,
    #[error("missing evidence for square {0}")]
    MissingSquareEvidence(String),
    #[error("there is no proposed move to commit")]
    NoPendingMove,
    #[error("committed position mismatch: expected {expected}, got {actual}")]
    CommitPositionMismatch { expected: String, actual: String },
    #[error("observation belongs to a different capture session; reset the decoder first")]
    SessionChanged,
    #[error("observation sequence must increase: previous {previous}, current {current}")]
    NonIncreasingSequence { previous: u64, current: u64 },
    #[error("capture sequence has a gap: expected {expected}, current {current}")]
    SequenceGap { expected: u64, current: u64 },
    #[error("capture time must increase: previous {previous}, current {current}")]
    NonIncreasingCaptureTime { previous: i64, current: i64 },
    #[error("calibration version changed from {expected} to {actual}; recalibration is required")]
    CalibrationVersionChanged { expected: String, actual: String },
    #[error("model version changed from {expected} to {actual}; reset the decoder first")]
    ModelVersionChanged { expected: String, actual: String },
    #[error("the decoder is suspended after a stream discontinuity; reset it before ingesting")]
    ResetRequired,
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use contracts::{PIECE_CLASS_COUNT, SessionId, SquareEvidence};

    use super::*;

    fn observation(game: &ChessGame, sequence: u64, time: i64, moving: bool) -> FrameObservation {
        static SESSION_ID: OnceLock<SessionId> = OnceLock::new();
        let squares = game
            .piece_classes()
            .into_iter()
            .map(|(square, piece)| {
                let mut probabilities = [0.0001; PIECE_CLASS_COUNT];
                let empty_probability = if let Some(piece) = piece {
                    probabilities[piece.index()] = 0.999;
                    0.0001
                } else {
                    0.999
                };
                SquareEvidence {
                    square,
                    visible_probability: 1.0,
                    empty_probability,
                    piece_probabilities: probabilities,
                }
            })
            .collect();
        FrameObservation {
            session_id: *SESSION_ID.get_or_init(SessionId::new),
            sequence,
            capture_time: CaptureTimeUs::new(time).unwrap(),
            moving,
            calibration_version: "test".into(),
            model_version: "synthetic".into(),
            squares,
        }
    }

    #[test]
    fn proposes_a_unique_legal_move_after_settling() {
        let game = ChessGame::standard();
        let mut resulting = game.clone();
        resulting.play_uci("e2e4").unwrap();
        let mut decoder = TemporalDecoder::new(DecoderConfig {
            settle_time_us: 100,
            consistent_frames: 2,
            ..DecoderConfig::default()
        })
        .unwrap();

        assert_eq!(
            decoder
                .ingest(&game, &observation(&game, 0, 0, false))
                .unwrap(),
            DecoderDecision::Settling
        );
        assert_eq!(
            decoder
                .ingest(&game, &observation(&game, 1, 100, false))
                .unwrap(),
            DecoderDecision::NoMove
        );
        decoder
            .ingest(&game, &observation(&game, 2, 200, true))
            .unwrap();
        decoder
            .ingest(&game, &observation(&resulting, 3, 300, false))
            .unwrap();
        assert_eq!(
            decoder
                .ingest(&game, &observation(&resulting, 4, 400, false))
                .unwrap(),
            DecoderDecision::Settling
        );
        let decision = decoder
            .ingest(&game, &observation(&resulting, 5, 500, false))
            .unwrap();
        let DecoderDecision::Proposed(proposed) = decision else {
            panic!("expected proposed move, got {decision:?}");
        };
        assert_eq!(proposed.uci, "e2e4");
        assert_eq!(proposed.completion_bounds.earliest.get(), 100);
        assert_eq!(proposed.completion_bounds.latest.get(), 400);

        let mut committed = game.clone();
        committed.play_uci(&proposed.uci).unwrap();
        decoder.commit(&committed).unwrap();
        assert_eq!(decoder.state(), &RecorderState::Stable);
    }

    #[test]
    fn piece_adjustment_does_not_create_move() {
        let game = ChessGame::standard();
        let mut decoder = TemporalDecoder::new(DecoderConfig {
            settle_time_us: 0,
            ..DecoderConfig::default()
        })
        .unwrap();
        decoder
            .ingest(&game, &observation(&game, 0, 0, true))
            .unwrap();
        assert_eq!(
            decoder
                .ingest(&game, &observation(&game, 1, 1, false))
                .unwrap(),
            DecoderDecision::NoMove
        );
    }

    #[test]
    fn hidden_board_never_proposes_move() {
        let game = ChessGame::standard();
        let mut obs = observation(&game, 0, 0, false);
        for square in &mut obs.squares {
            square.visible_probability = 0.0;
        }
        let mut decoder = TemporalDecoder::new(DecoderConfig {
            settle_time_us: 0,
            ..DecoderConfig::default()
        })
        .unwrap();
        assert!(matches!(
            decoder.ingest(&game, &obs).unwrap(),
            DecoderDecision::NeedsReview { .. }
        ));
    }

    #[test]
    fn final_board_after_two_hidden_moves_is_not_split_into_automatic_moves() {
        let game = ChessGame::standard();
        let mut two_plies_ahead = game.clone();
        two_plies_ahead.play_uci("e2e4").unwrap();
        two_plies_ahead.play_uci("e7e5").unwrap();
        let mut decoder = TemporalDecoder::new(DecoderConfig {
            settle_time_us: 0,
            consistent_frames: 1,
            ..DecoderConfig::default()
        })
        .unwrap();

        assert_eq!(
            decoder
                .ingest(&game, &observation(&game, 0, 0, false))
                .unwrap(),
            DecoderDecision::NoMove
        );
        decoder
            .ingest(&game, &observation(&game, 1, 1, true))
            .unwrap();
        assert!(matches!(
            decoder
                .ingest(&game, &observation(&two_plies_ahead, 2, 2, false))
                .unwrap(),
            DecoderDecision::NeedsReview { .. }
        ));
        assert_eq!(decoder.state(), &RecorderState::Uncertain);

        let mut decoder = TemporalDecoder::new(DecoderConfig {
            settle_time_us: 0,
            consistent_frames: 1,
            ..DecoderConfig::default()
        })
        .unwrap();
        decoder
            .ingest(&game, &observation(&game, 0, 0, false))
            .unwrap();
        decoder
            .ingest(&game, &observation(&game, 1, 1, true))
            .unwrap();
        let mut hidden_reply = observation(&two_plies_ahead, 2, 2, false);
        for square in &mut hidden_reply.squares {
            if matches!(square.square.as_str(), "e7" | "e5") {
                square.visible_probability = 0.0;
            }
        }
        assert!(matches!(
            decoder.ingest(&game, &hidden_reply).unwrap(),
            DecoderDecision::NeedsReview { .. }
        ));
    }

    #[test]
    fn rejects_invalid_decoder_thresholds() {
        for invalid in [
            DecoderConfig {
                settle_time_us: -1,
                ..DecoderConfig::default()
            },
            DecoderConfig {
                min_visible_squares: 65,
                ..DecoderConfig::default()
            },
            DecoderConfig {
                min_expected_square_probability: f32::NAN,
                ..DecoderConfig::default()
            },
            DecoderConfig {
                min_average_log_likelihood: f64::NAN,
                ..DecoderConfig::default()
            },
            DecoderConfig {
                min_margin: f64::NAN,
                ..DecoderConfig::default()
            },
            DecoderConfig {
                consistent_frames: 0,
                ..DecoderConfig::default()
            },
        ] {
            assert!(TemporalDecoder::new(invalid).is_err());
        }
    }

    #[test]
    fn rejects_duplicate_sequences_backwards_time_and_gaps() {
        let game = ChessGame::standard();
        let mut decoder = TemporalDecoder::new(DecoderConfig::default()).unwrap();
        decoder
            .ingest(&game, &observation(&game, 4, 400, false))
            .unwrap();

        assert!(matches!(
            decoder.ingest(&game, &observation(&game, 4, 500, false)),
            Err(DecoderError::NonIncreasingSequence {
                previous: 4,
                current: 4
            })
        ));
        assert!(matches!(
            decoder.ingest(&game, &observation(&game, 5, 500, false)),
            Err(DecoderError::ResetRequired)
        ));

        decoder.reset();
        decoder
            .ingest(&game, &observation(&game, 4, 400, false))
            .unwrap();
        assert!(matches!(
            decoder.ingest(&game, &observation(&game, 5, 300, false)),
            Err(DecoderError::NonIncreasingCaptureTime {
                previous: 400,
                current: 300
            })
        ));

        decoder.reset();
        decoder
            .ingest(&game, &observation(&game, 4, 400, false))
            .unwrap();
        assert!(matches!(
            decoder.ingest(&game, &observation(&game, 6, 600, false)),
            Err(DecoderError::SequenceGap {
                expected: 5,
                current: 6
            })
        ));
        assert_eq!(decoder.state(), &RecorderState::Uncertain);
    }

    #[test]
    fn version_change_requires_an_explicit_reset() {
        let game = ChessGame::standard();
        let mut decoder = TemporalDecoder::new(DecoderConfig::default()).unwrap();
        decoder
            .ingest(&game, &observation(&game, 1, 100, false))
            .unwrap();
        let mut changed = observation(&game, 2, 200, false);
        changed.calibration_version = "replacement-calibration".into();

        assert!(matches!(
            decoder.ingest(&game, &changed),
            Err(DecoderError::CalibrationVersionChanged { .. })
        ));
        assert_eq!(decoder.state(), &RecorderState::RecalibrationRequired);

        decoder.reset();
        assert!(decoder.ingest(&game, &changed).is_ok());
        assert_eq!(decoder.state(), &RecorderState::Settling);
    }

    #[test]
    fn different_session_requires_an_explicit_reset() {
        let game = ChessGame::standard();
        let mut decoder = TemporalDecoder::new(DecoderConfig::default()).unwrap();
        decoder
            .ingest(&game, &observation(&game, 1, 100, false))
            .unwrap();
        let mut changed = observation(&game, 2, 200, false);
        changed.session_id = SessionId::new();

        assert!(matches!(
            decoder.ingest(&game, &changed),
            Err(DecoderError::SessionChanged)
        ));
        assert_eq!(decoder.state(), &RecorderState::Uncertain);
        assert!(matches!(
            decoder.ingest(&game, &observation(&game, 2, 200, false)),
            Err(DecoderError::ResetRequired)
        ));
    }
}
