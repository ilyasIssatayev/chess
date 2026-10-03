use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const SQUARE_COUNT: usize = 64;
pub const PIECE_CLASS_COUNT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PieceClass {
    WhitePawn,
    WhiteKnight,
    WhiteBishop,
    WhiteRook,
    WhiteQueen,
    WhiteKing,
    BlackPawn,
    BlackKnight,
    BlackBishop,
    BlackRook,
    BlackQueen,
    BlackKing,
}

impl PieceClass {
    pub const fn index(self) -> usize {
        match self {
            Self::WhitePawn => 0,
            Self::WhiteKnight => 1,
            Self::WhiteBishop => 2,
            Self::WhiteRook => 3,
            Self::WhiteQueen => 4,
            Self::WhiteKing => 5,
            Self::BlackPawn => 6,
            Self::BlackKnight => 7,
            Self::BlackBishop => 8,
            Self::BlackRook => 9,
            Self::BlackQueen => 10,
            Self::BlackKing => 11,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GameId(pub Uuid);

impl GameId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for GameId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CaptureTimeUs(i64);

impl CaptureTimeUs {
    pub fn new(value: i64) -> Result<Self, ContractError> {
        if value < 0 {
            return Err(ContractError::NegativeCaptureTime(value));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeBounds {
    pub earliest: CaptureTimeUs,
    pub latest: CaptureTimeUs,
}

impl TimeBounds {
    pub fn new(earliest: CaptureTimeUs, latest: CaptureTimeUs) -> Result<Self, ContractError> {
        if earliest > latest {
            return Err(ContractError::ReversedTimeBounds {
                earliest: earliest.get(),
                latest: latest.get(),
            });
        }
        Ok(Self { earliest, latest })
    }

    pub fn elapsed_since(self, previous: Self) -> Result<Self, ContractError> {
        let earliest = self.earliest.get() - previous.latest.get();
        let latest = self.latest.get() - previous.earliest.get();
        if latest < 0 {
            return Err(ContractError::NonChronologicalTimeBounds {
                previous_earliest: previous.earliest.get(),
                current_latest: self.latest.get(),
            });
        }
        Self::new(
            CaptureTimeUs::new(earliest.max(0))?,
            CaptureTimeUs::new(latest)?,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardOrientation {
    WhiteNear,
    BlackNear,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImagePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardCalibration {
    pub orientation: BoardOrientation,
    /// Corners are ordered a1, h1, h8, a8 in chess coordinates.
    pub corners: [ImagePoint; 4],
    pub reprojection_error_px: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SquareEvidence {
    /// Algebraic square name, for example `e4`.
    pub square: String,
    pub visible_probability: f32,
    pub empty_probability: f32,
    /// Piece classes are white PNBRQK, then black PNBRQK.
    pub piece_probabilities: [f32; PIECE_CLASS_COUNT],
}

impl SquareEvidence {
    pub fn probability_for(&self, expected: Option<PieceClass>) -> f32 {
        expected
            .map(|piece| self.piece_probabilities[piece.index()])
            .unwrap_or(self.empty_probability)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameObservation {
    pub session_id: SessionId,
    pub sequence: u64,
    pub capture_time: CaptureTimeUs,
    pub moving: bool,
    pub calibration_version: String,
    pub model_version: String,
    pub squares: Vec<SquareEvidence>,
}

impl FrameObservation {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.calibration_version.trim().is_empty() {
            return Err(ContractError::EmptyCalibrationVersion);
        }
        if self.model_version.trim().is_empty() {
            return Err(ContractError::EmptyModelVersion);
        }
        if self.squares.len() != SQUARE_COUNT {
            return Err(ContractError::WrongSquareCount(self.squares.len()));
        }
        let mut square_names = HashSet::with_capacity(SQUARE_COUNT);
        for evidence in &self.squares {
            if !is_square_name(&evidence.square) {
                return Err(ContractError::InvalidSquareName(evidence.square.clone()));
            }
            if !square_names.insert(evidence.square.as_str()) {
                return Err(ContractError::DuplicateSquareName(evidence.square.clone()));
            }
            validate_probability(evidence.visible_probability)?;
            validate_probability(evidence.empty_probability)?;
            for probability in evidence.piece_probabilities {
                validate_probability(probability)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoveProvenance {
    Automatic,
    Reviewed,
    Inferred,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingQuality {
    Observed,
    Bounded,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveTiming {
    pub completion: Option<TimeBounds>,
    pub elapsed_since_previous: Option<TimeBounds>,
    pub confirmation_time: CaptureTimeUs,
    pub quality: TimingQuality,
}

impl MoveTiming {
    pub fn validate(&self) -> Result<(), ContractError> {
        match self.quality {
            TimingQuality::Observed | TimingQuality::Bounded if self.completion.is_none() => {
                return Err(ContractError::MissingCompletionBounds(self.quality));
            }
            TimingQuality::Unknown
                if self.completion.is_some() || self.elapsed_since_previous.is_some() =>
            {
                return Err(ContractError::UnknownTimingHasBounds);
            }
            _ => {}
        }
        if let Some(completion) = self.completion
            && self.confirmation_time < completion.latest
        {
            return Err(ContractError::ConfirmationBeforeCompletion {
                confirmation: self.confirmation_time.get(),
                completion_latest: completion.latest.get(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveRecord {
    pub game_id: GameId,
    pub revision: u32,
    pub ply: u32,
    pub uci: String,
    pub san: String,
    pub fen_before: String,
    pub fen_after: String,
    pub provenance: MoveProvenance,
    pub confidence: Option<f32>,
    pub timing: MoveTiming,
    pub evidence_ids: Vec<String>,
}

impl MoveRecord {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.ply == 0 {
            return Err(ContractError::ZeroPly);
        }
        if self.uci.trim().is_empty()
            || self.san.trim().is_empty()
            || self.fen_before.trim().is_empty()
            || self.fen_after.trim().is_empty()
        {
            return Err(ContractError::EmptyMoveField);
        }
        if let Some(confidence) = self.confidence {
            validate_probability(confidence)?;
        }
        self.timing.validate()?;
        let mut evidence_ids = HashSet::with_capacity(self.evidence_ids.len());
        for evidence_id in &self.evidence_ids {
            if evidence_id.trim().is_empty() {
                return Err(ContractError::EmptyEvidenceId);
            }
            if !evidence_ids.insert(evidence_id.as_str()) {
                return Err(ContractError::DuplicateEvidenceId(evidence_id.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveCandidate {
    pub uci: String,
    pub score: f32,
    pub fen_after: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub game_id: GameId,
    pub revision: u32,
    pub expected_ply: u32,
    pub reason: String,
    pub candidates: Vec<MoveCandidate>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CaptureHealthEvent {
    Started,
    DroppedFrames { count: u64 },
    CameraUnavailable { message: String },
    CalibrationDrift,
    Paused,
    Resumed,
    Stopped,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ContractError {
    #[error("capture time cannot be negative: {0}")]
    NegativeCaptureTime(i64),
    #[error("time bounds are reversed: {earliest} > {latest}")]
    ReversedTimeBounds { earliest: i64, latest: i64 },
    #[error(
        "completion bounds are not chronological: current latest {current_latest} precedes previous earliest {previous_earliest}"
    )]
    NonChronologicalTimeBounds {
        previous_earliest: i64,
        current_latest: i64,
    },
    #[error("frame observation must contain 64 squares, got {0}")]
    WrongSquareCount(usize),
    #[error("probability must be finite and between 0 and 1")]
    InvalidProbability,
    #[error("calibration version cannot be empty")]
    EmptyCalibrationVersion,
    #[error("model version cannot be empty")]
    EmptyModelVersion,
    #[error("invalid chess square name: {0}")]
    InvalidSquareName(String),
    #[error("duplicate chess square evidence: {0}")]
    DuplicateSquareName(String),
    #[error("{0:?} timing requires completion bounds")]
    MissingCompletionBounds(TimingQuality),
    #[error("unknown timing cannot contain completion or elapsed bounds")]
    UnknownTimingHasBounds,
    #[error(
        "confirmation time {confirmation} precedes the latest completion bound {completion_latest}"
    )]
    ConfirmationBeforeCompletion {
        confirmation: i64,
        completion_latest: i64,
    },
    #[error("move ply must be at least one")]
    ZeroPly,
    #[error("UCI, SAN, and FEN move fields cannot be empty")]
    EmptyMoveField,
    #[error("evidence identifier cannot be empty")]
    EmptyEvidenceId,
    #[error("duplicate evidence identifier: {0}")]
    DuplicateEvidenceId(String),
}

fn validate_probability(value: f32) -> Result<(), ContractError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(ContractError::InvalidProbability)
    }
}

fn is_square_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 2 && (b'a'..=b'h').contains(&bytes[0]) && (b'1'..=b'8').contains(&bytes[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_bounds_keep_uncertainty() {
        let previous = TimeBounds::new(
            CaptureTimeUs::new(1_000_000).unwrap(),
            CaptureTimeUs::new(1_100_000).unwrap(),
        )
        .unwrap();
        let current = TimeBounds::new(
            CaptureTimeUs::new(3_000_000).unwrap(),
            CaptureTimeUs::new(3_200_000).unwrap(),
        )
        .unwrap();

        let elapsed = current.elapsed_since(previous).unwrap();
        assert_eq!(elapsed.earliest.get(), 1_900_000);
        assert_eq!(elapsed.latest.get(), 2_200_000);
    }

    #[test]
    fn rejects_reversed_bounds() {
        let result = TimeBounds::new(
            CaptureTimeUs::new(2).unwrap(),
            CaptureTimeUs::new(1).unwrap(),
        );
        assert!(matches!(
            result,
            Err(ContractError::ReversedTimeBounds { .. })
        ));
    }

    #[test]
    fn rejects_definitely_reversed_completion_intervals() {
        let previous = TimeBounds::new(
            CaptureTimeUs::new(2_000_000).unwrap(),
            CaptureTimeUs::new(2_100_000).unwrap(),
        )
        .unwrap();
        let current = TimeBounds::new(
            CaptureTimeUs::new(1_000_000).unwrap(),
            CaptureTimeUs::new(1_100_000).unwrap(),
        )
        .unwrap();

        assert!(matches!(
            current.elapsed_since(previous),
            Err(ContractError::NonChronologicalTimeBounds { .. })
        ));
    }

    #[test]
    fn overlapping_completion_intervals_have_zero_lower_bound() {
        let previous = TimeBounds::new(
            CaptureTimeUs::new(1_000_000).unwrap(),
            CaptureTimeUs::new(1_200_000).unwrap(),
        )
        .unwrap();
        let current = TimeBounds::new(
            CaptureTimeUs::new(1_100_000).unwrap(),
            CaptureTimeUs::new(1_300_000).unwrap(),
        )
        .unwrap();

        let elapsed = current.elapsed_since(previous).unwrap();
        assert_eq!(elapsed.earliest.get(), 0);
        assert_eq!(elapsed.latest.get(), 300_000);
    }

    fn valid_observation() -> FrameObservation {
        let mut squares = Vec::with_capacity(SQUARE_COUNT);
        for rank in b'1'..=b'8' {
            for file in b'a'..=b'h' {
                squares.push(SquareEvidence {
                    square: String::from_utf8(vec![file, rank]).unwrap(),
                    visible_probability: 1.0,
                    empty_probability: 1.0,
                    piece_probabilities: [0.0; PIECE_CLASS_COUNT],
                });
            }
        }
        FrameObservation {
            session_id: SessionId::new(),
            sequence: 0,
            capture_time: CaptureTimeUs::new(0).unwrap(),
            moving: false,
            calibration_version: "calibration-v1".into(),
            model_version: "model-v1".into(),
            squares,
        }
    }

    #[test]
    fn observation_requires_every_square_exactly_once() {
        let mut observation = valid_observation();
        assert_eq!(observation.validate(), Ok(()));

        observation.squares[1].square = observation.squares[0].square.clone();
        assert!(matches!(
            observation.validate(),
            Err(ContractError::DuplicateSquareName(_))
        ));

        observation = valid_observation();
        observation.squares[0].square = "z9".into();
        assert!(matches!(
            observation.validate(),
            Err(ContractError::InvalidSquareName(_))
        ));
    }

    #[test]
    fn timing_quality_and_confirmation_are_consistent() {
        let completion = TimeBounds::new(
            CaptureTimeUs::new(100).unwrap(),
            CaptureTimeUs::new(200).unwrap(),
        )
        .unwrap();
        let valid = MoveTiming {
            completion: Some(completion),
            elapsed_since_previous: None,
            confirmation_time: CaptureTimeUs::new(250).unwrap(),
            quality: TimingQuality::Bounded,
        };
        assert_eq!(valid.validate(), Ok(()));

        let missing = MoveTiming {
            completion: None,
            ..valid.clone()
        };
        assert!(matches!(
            missing.validate(),
            Err(ContractError::MissingCompletionBounds(
                TimingQuality::Bounded
            ))
        ));

        let early = MoveTiming {
            confirmation_time: CaptureTimeUs::new(199).unwrap(),
            ..valid
        };
        assert!(matches!(
            early.validate(),
            Err(ContractError::ConfirmationBeforeCompletion { .. })
        ));
    }
}
