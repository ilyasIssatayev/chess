use std::fmt::Write;

use chess_core::ChessGame;
use contracts::{ContractError, MoveRecord, TimingQuality};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PgnMetadata {
    pub event: String,
    pub site: String,
    pub date: String,
    pub round: String,
    pub white: String,
    pub black: String,
    pub result: String,
    pub initial_fen: Option<String>,
}

impl Default for PgnMetadata {
    fn default() -> Self {
        Self {
            event: "Casual Game".into(),
            site: "?".into(),
            date: "????.??.??".into(),
            round: "?".into(),
            white: "?".into(),
            black: "?".into(),
            result: "*".into(),
            initial_fen: None,
        }
    }
}

pub fn to_pgn(
    metadata: &PgnMetadata,
    moves: &[MoveRecord],
    include_timing_comments: bool,
) -> Result<String, ExportError> {
    validate_result(&metadata.result)?;
    validate_metadata(metadata)?;
    validate_chain(metadata, moves)?;

    let mut output = String::new();
    for (name, value) in [
        ("Event", metadata.event.as_str()),
        ("Site", metadata.site.as_str()),
        ("Date", metadata.date.as_str()),
        ("Round", metadata.round.as_str()),
        ("White", metadata.white.as_str()),
        ("Black", metadata.black.as_str()),
        ("Result", metadata.result.as_str()),
    ] {
        writeln!(output, "[{name} \"{}\"]", escape_tag(value)).unwrap();
    }
    if let Some(fen) = &metadata.initial_fen {
        writeln!(output, "[SetUp \"1\"]").unwrap();
        writeln!(output, "[FEN \"{}\"]", escape_tag(fen)).unwrap();
    }
    output.push('\n');

    for (index, record) in moves.iter().enumerate() {
        let (white_to_move, move_number) = fen_turn_and_fullmove(&record.fen_before, record.ply)?;
        if white_to_move {
            write!(output, "{move_number}. ").unwrap();
        } else if index == 0 {
            write!(output, "{move_number}... ").unwrap();
        }
        output.push_str(&record.san);
        if include_timing_comments {
            output.push(' ');
            output.push_str(&timing_comment(record));
        }
        output.push(' ');
    }
    output.push_str(&metadata.result);
    output.push('\n');
    Ok(output)
}

fn validate_chain(metadata: &PgnMetadata, moves: &[MoveRecord]) -> Result<(), ExportError> {
    let mut game = match &metadata.initial_fen {
        Some(fen) => ChessGame::from_fen(fen),
        None => Ok(ChessGame::standard()),
    }
    .map_err(|error| ExportError::InvalidInitialPosition(error.to_string()))?;
    let Some(first) = moves.first() else {
        return Ok(());
    };
    let game_id = first.game_id;
    let revision = first.revision;

    for (index, record) in moves.iter().enumerate() {
        record.validate()?;
        let expected_ply = index as u32 + 1;
        if record.ply != expected_ply {
            return Err(ExportError::PlyMismatch {
                expected: expected_ply,
                actual: record.ply,
            });
        }
        if record.game_id != game_id {
            return Err(ExportError::MixedGames { ply: record.ply });
        }
        if record.revision != revision {
            return Err(ExportError::MixedRevisions { ply: record.ply });
        }
        if record.fen_before != game.fen() {
            return Err(ExportError::DisconnectedPosition { ply: record.ply });
        }
        let applied = game
            .play_uci(&record.uci)
            .map_err(|error| ExportError::InvalidMove {
                ply: record.ply,
                reason: error.to_string(),
            })?;
        if applied.uci != record.uci
            || applied.san != record.san
            || applied.fen_before != record.fen_before
            || applied.fen_after != record.fen_after
        {
            return Err(ExportError::DerivedMoveMismatch { ply: record.ply });
        }
    }
    if metadata
        .initial_fen
        .as_deref()
        .is_some_and(|fen| fen.chars().any(char::is_control))
    {
        return Err(ExportError::InvalidTagValue("FEN".to_owned()));
    }
    Ok(())
}

fn timing_comment(record: &MoveRecord) -> String {
    match record.timing.elapsed_since_previous {
        Some(bounds) if bounds.earliest == bounds.latest => format!(
            "{{elapsed {:.3}s; timing {}}}",
            bounds.earliest.get() as f64 / 1_000_000.0,
            timing_quality_name(record.timing.quality)
        ),
        Some(bounds) => format!(
            "{{elapsed {:.3}-{:.3}s; timing {}}}",
            bounds.earliest.get() as f64 / 1_000_000.0,
            bounds.latest.get() as f64 / 1_000_000.0,
            timing_quality_name(record.timing.quality)
        ),
        None => "{elapsed unknown}".into(),
    }
}

fn timing_quality_name(quality: TimingQuality) -> &'static str {
    match quality {
        TimingQuality::Observed => "observed",
        TimingQuality::Bounded => "bounded",
        TimingQuality::Unknown => "unknown",
    }
}

fn validate_result(result: &str) -> Result<(), ExportError> {
    if matches!(result, "1-0" | "0-1" | "1/2-1/2" | "*") {
        Ok(())
    } else {
        Err(ExportError::InvalidResult(result.to_owned()))
    }
}

fn validate_metadata(metadata: &PgnMetadata) -> Result<(), ExportError> {
    for (name, value) in [
        ("Event", metadata.event.as_str()),
        ("Site", metadata.site.as_str()),
        ("Date", metadata.date.as_str()),
        ("Round", metadata.round.as_str()),
        ("White", metadata.white.as_str()),
        ("Black", metadata.black.as_str()),
    ] {
        if value.chars().any(char::is_control) {
            return Err(ExportError::InvalidTagValue(name.to_owned()));
        }
    }
    Ok(())
}

fn fen_turn_and_fullmove(fen: &str, ply: u32) -> Result<(bool, u32), ExportError> {
    let fields = fen.split_whitespace().collect::<Vec<_>>();
    let white_to_move = match fields.get(1).copied() {
        Some("w") => true,
        Some("b") => false,
        _ => return Err(ExportError::InvalidFenFields { ply }),
    };
    let fullmove = fields
        .get(5)
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .ok_or(ExportError::InvalidFenFields { ply })?;
    Ok((white_to_move, fullmove))
}

fn escape_tag(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExportError {
    #[error("invalid PGN result: {0}")]
    InvalidResult(String),
    #[error("expected ply {expected}, got {actual}")]
    PlyMismatch { expected: u32, actual: u32 },
    #[error("move at ply {ply} does not connect to the previous position")]
    DisconnectedPosition { ply: u32 },
    #[error("invalid initial PGN position: {0}")]
    InvalidInitialPosition(String),
    #[error("move at ply {ply} belongs to a different game")]
    MixedGames { ply: u32 },
    #[error("move at ply {ply} belongs to a different revision")]
    MixedRevisions { ply: u32 },
    #[error("move at ply {ply} is invalid: {reason}")]
    InvalidMove { ply: u32, reason: String },
    #[error("canonical UCI, SAN, or FEN differs at ply {ply}")]
    DerivedMoveMismatch { ply: u32 },
    #[error("PGN tag {0} contains a control character")]
    InvalidTagValue(String),
    #[error("stored FEN at ply {ply} is missing side-to-move or fullmove fields")]
    InvalidFenFields { ply: u32 },
    #[error(transparent)]
    Contract(#[from] ContractError),
}

#[cfg(test)]
mod tests {
    use contracts::{CaptureTimeUs, GameId, MoveProvenance, MoveTiming, TimeBounds};

    use super::*;

    fn records(initial_fen: Option<&str>, uci_moves: &[&str]) -> Vec<MoveRecord> {
        let game_id = GameId::new();
        let mut game = initial_fen
            .map(ChessGame::from_fen)
            .transpose()
            .unwrap()
            .unwrap_or_default();
        uci_moves
            .iter()
            .enumerate()
            .map(|(index, uci)| {
                let applied = game.play_uci(uci).unwrap().clone();
                let completion_earliest = 1_000_000 + index as i64 * 3_000_000;
                MoveRecord {
                    game_id,
                    revision: 0,
                    ply: applied.ply,
                    uci: applied.uci,
                    san: applied.san,
                    fen_before: applied.fen_before,
                    fen_after: applied.fen_after,
                    provenance: MoveProvenance::Automatic,
                    confidence: Some(1.0),
                    timing: MoveTiming {
                        completion: Some(
                            TimeBounds::new(
                                CaptureTimeUs::new(completion_earliest).unwrap(),
                                CaptureTimeUs::new(completion_earliest + 200_000).unwrap(),
                            )
                            .unwrap(),
                        ),
                        elapsed_since_previous: Some(
                            TimeBounds::new(
                                CaptureTimeUs::new(2_000_000).unwrap(),
                                CaptureTimeUs::new(2_200_000).unwrap(),
                            )
                            .unwrap(),
                        ),
                        confirmation_time: CaptureTimeUs::new(completion_earliest + 300_000)
                            .unwrap(),
                        quality: TimingQuality::Bounded,
                    },
                    evidence_ids: vec![],
                }
            })
            .collect()
    }

    #[test]
    fn writes_valid_movetext_and_descriptive_timing() {
        let moves = records(None, &["e2e4", "e7e5"]);
        let pgn = to_pgn(&PgnMetadata::default(), &moves, true).unwrap();
        assert!(pgn.contains("1. e4 {elapsed 2.000-2.200s; timing bounded} e5"));
        assert!(pgn.ends_with("*\n"));
    }

    #[test]
    fn rejects_disconnected_history() {
        let mut moves = records(None, &["e2e4", "e7e5"]);
        moves[1].fen_before = ChessGame::standard().fen();
        assert!(matches!(
            to_pgn(&PgnMetadata::default(), &moves, false),
            Err(ExportError::DisconnectedPosition { ply: 2 })
        ));
    }

    #[test]
    fn rejects_forged_san_and_mixed_histories() {
        let mut forged = records(None, &["e2e4", "e7e5"]);
        forged[1].san = "not-san".into();
        assert!(matches!(
            to_pgn(&PgnMetadata::default(), &forged, false),
            Err(ExportError::DerivedMoveMismatch { ply: 2 })
        ));

        let mut mixed = records(None, &["e2e4", "e7e5"]);
        mixed[1].game_id = GameId::new();
        assert!(matches!(
            to_pgn(&PgnMetadata::default(), &mixed, false),
            Err(ExportError::MixedGames { ply: 2 })
        ));
    }

    #[test]
    fn validates_and_exports_a_custom_start() {
        let fen = "7k/P7/8/8/8/8/8/7K w - - 0 1";
        let moves = records(Some(fen), &["a7a8q"]);
        let metadata = PgnMetadata {
            initial_fen: Some(fen.into()),
            ..PgnMetadata::default()
        };
        let pgn = to_pgn(&metadata, &moves, false).unwrap();
        assert!(pgn.contains("[SetUp \"1\"]"));
        assert!(pgn.contains("1. a8=Q+"));
    }

    #[test]
    fn numbers_a_black_to_move_custom_start_from_its_fen() {
        let fen = "7k/8/8/8/8/8/p7/7K b - - 0 17";
        let moves = records(Some(fen), &["a2a1q"]);
        let metadata = PgnMetadata {
            initial_fen: Some(fen.into()),
            ..PgnMetadata::default()
        };
        let pgn = to_pgn(&metadata, &moves, false).unwrap();
        assert!(pgn.contains("17... a1=Q+"));
    }

    #[test]
    fn rejects_control_characters_in_tag_values() {
        let metadata = PgnMetadata {
            white: "Alice\n[Result \"1-0\"]".into(),
            ..PgnMetadata::default()
        };
        assert!(matches!(
            to_pgn(&metadata, &[], false),
            Err(ExportError::InvalidTagValue(name)) if name == "White"
        ));
    }
}
