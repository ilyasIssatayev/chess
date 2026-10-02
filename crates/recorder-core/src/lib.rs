use std::cmp::Ordering;
use std::collections::HashMap;

use chess_core::{ChessError, ChessGame};
use contracts::{CaptureTimeUs, ContractError, FrameObservation, PieceClass, TimeBounds};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecoderConfig {
    pub settle_time_us: i64,
    pub min_visible_squares: usize,
    pub min_average_log_likelihood: f64,
    pub min_margin: f64,
    pub consistent_frames: u32,
}

impl Default for DecoderConfig {
    fn default() -> Self {
        Self {
            settle_time_us: 250_000,
            min_visible_squares: 56,
            min_average_log_likelihood: -0.75,
            min_margin: 0.10,
            consistent_frames: 2,
        }
    }
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
    last_motion: Option<CaptureTimeUs>,
    pending: Option<PendingCandidate>,
    awaiting: Option<ProposedMove>,
}

impl TemporalDecoder {
    pub fn new(config: DecoderConfig) -> Self {
        Self {
            config,
            state: RecorderState::Calibrating,
            stable_since: None,
            last_motion: None,
            pending: None,
            awaiting: None,
        }
    }

    pub fn state(&self) -> &RecorderState {
        &self.state
    }

    pub fn ingest(
        &mut self,
        game: &ChessGame,
        observation: &FrameObservation,
    ) -> Result<DecoderDecision, DecoderError> {
        observation.validate()?;
        if let Some(proposed) = &self.awaiting {
            return Ok(DecoderDecision::AwaitingCommit(proposed.clone()));
        }

        if observation.moving {
            self.state = RecorderState::Disturbed;
            self.last_motion = Some(observation.capture_time);
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

        let ranked = rank_positions(game, observation, self.config.min_visible_squares)?;
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

        if best.uci.is_none() {
            self.state = RecorderState::Stable;
            self.pending = None;
            return Ok(DecoderDecision::NoMove);
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

        let uci = best.uci.clone().expect("unchanged position handled above");
        let first_supported = self
            .pending
            .as_ref()
            .filter(|candidate| candidate.proposed.uci == uci)
            .map(|candidate| candidate.proposed.completion_bounds.latest)
            .unwrap_or(observation.capture_time);
        let earliest = self.last_motion.unwrap_or(stable_since);
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
        self.awaiting = None;
        self.pending = None;
        self.stable_since = None;
        self.last_motion = None;
        self.state = RecorderState::Stable;
        Ok(())
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
        score: score_position(game.piece_classes(), &evidence, min_visible_squares)?,
    });
    for uci in game.legal_uci_moves() {
        let mut next = game.clone();
        next.play_uci(&uci)?;
        candidates.push(RankedPosition {
            uci: Some(uci),
            fen: next.fen(),
            score: score_position(next.piece_classes(), &evidence, min_visible_squares)?,
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
        let probability = observed.probability_for(expected_piece).max(1e-6) as f64;
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
}

#[cfg(test)]
mod tests {
    use contracts::{PIECE_CLASS_COUNT, SessionId, SquareEvidence};

    use super::*;

    fn observation(game: &ChessGame, sequence: u64, time: i64, moving: bool) -> FrameObservation {
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
            session_id: SessionId::new(),
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
        });

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
        assert_eq!(proposed.completion_bounds.earliest.get(), 200);
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
        });
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
        });
        assert!(matches!(
            decoder.ingest(&game, &obs).unwrap(),
            DecoderDecision::NeedsReview { .. }
        ));
    }
}
