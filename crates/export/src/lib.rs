use std::fmt::Write;

use contracts::{MoveRecord, TimingQuality};
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
    validate_chain(moves)?;

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

    for record in moves {
        if record.ply % 2 == 1 {
            let move_number = record.ply.div_ceil(2);
            write!(output, "{move_number}. ").unwrap();
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

fn validate_chain(moves: &[MoveRecord]) -> Result<(), ExportError> {
    for (index, record) in moves.iter().enumerate() {
        let expected_ply = index as u32 + 1;
        if record.ply != expected_ply {
            return Err(ExportError::PlyMismatch {
                expected: expected_ply,
                actual: record.ply,
            });
        }
        if let Some(previous) = index.checked_sub(1).and_then(|i| moves.get(i))
            && previous.fen_after != record.fen_before
        {
            return Err(ExportError::DisconnectedPosition { ply: record.ply });
        }
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
}

#[cfg(test)]
mod tests {
    use contracts::{CaptureTimeUs, GameId, MoveProvenance, MoveTiming, TimeBounds};

    use super::*;

    fn move_record(ply: u32, san: &str, before: &str, after: &str) -> MoveRecord {
        MoveRecord {
            game_id: GameId::new(),
            revision: 0,
            ply,
            uci: "e2e4".into(),
            san: san.into(),
            fen_before: before.into(),
            fen_after: after.into(),
            provenance: MoveProvenance::Automatic,
            confidence: Some(1.0),
            timing: MoveTiming {
                completion: None,
                elapsed_since_previous: Some(
                    TimeBounds::new(
                        CaptureTimeUs::new(2_000_000).unwrap(),
                        CaptureTimeUs::new(2_200_000).unwrap(),
                    )
                    .unwrap(),
                ),
                confirmation_time: CaptureTimeUs::new(3_000_000).unwrap(),
                quality: TimingQuality::Bounded,
            },
            evidence_ids: vec![],
        }
    }

    #[test]
    fn writes_valid_movetext_and_descriptive_timing() {
        let moves = vec![
            move_record(1, "e4", "start", "after-e4"),
            move_record(2, "e5", "after-e4", "after-e5"),
        ];
        let pgn = to_pgn(&PgnMetadata::default(), &moves, true).unwrap();
        assert!(pgn.contains("1. e4 {elapsed 2.000-2.200s; timing bounded} e5"));
        assert!(pgn.ends_with("*\n"));
    }

    #[test]
    fn rejects_disconnected_history() {
        let moves = vec![
            move_record(1, "e4", "start", "after-e4"),
            move_record(2, "e5", "wrong", "after-e5"),
        ];
        assert!(matches!(
            to_pgn(&PgnMetadata::default(), &moves, false),
            Err(ExportError::DisconnectedPosition { ply: 2 })
        ));
    }
}
