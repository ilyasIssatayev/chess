use anyhow::{Context, Result};
use chess_core::ChessGame;
use contracts::{
    CaptureTimeUs, FrameObservation, GameId, MoveProvenance, MoveRecord, MoveTiming,
    PIECE_CLASS_COUNT, SessionId, SquareEvidence, TimeBounds, TimingQuality,
};
use export::{PgnMetadata, to_pgn};
use recorder_core::{DecoderConfig, DecoderDecision, TemporalDecoder};
use storage::Store;

mod server;
mod vision;

fn main() -> Result<()> {
    if std::env::args().nth(1).as_deref() == Some("serve") {
        return server::run();
    }
    if std::env::args().nth(1).as_deref() != Some("demo") {
        println!("Chess camera recorder scaffold");
        println!("Run `cargo run -- demo` to exercise chess, storage, and PGN export.");
        println!("Run `cargo run -- serve` to open the functional local recorder.");
        return Ok(());
    }

    print!("{}", build_demo_pgn()?);
    Ok(())
}

fn build_demo_pgn() -> Result<String> {
    let mut store = Store::open_in_memory().context("open demo database")?;
    let (game_id, chess) = record_synthetic_game(&mut store)?;
    export_replayed_game(&store, game_id, &chess)
}

fn record_synthetic_game(store: &mut Store) -> Result<(GameId, ChessGame)> {
    let game_id = GameId::new();
    let session_id = SessionId::new();
    let mut chess = ChessGame::standard();
    let mut decoder = TemporalDecoder::new(DecoderConfig {
        settle_time_us: 100_000,
        consistent_frames: 2,
        ..DecoderConfig::default()
    })?;
    store
        .create_game(game_id, chess.initial_fen(), 0)
        .context("create demo game")?;

    let mut sequence = 0;
    let mut clock_us = 0;
    let mut previous_completion: Option<TimeBounds> = None;
    let initial = synthetic_observation(&chess, session_id, sequence, clock_us, false)?;
    let _ = decoder.ingest(&chess, &initial)?;
    sequence += 1;
    clock_us += 100_000;
    let initial_confirmed = synthetic_observation(&chess, session_id, sequence, clock_us, false)?;
    let _ = decoder.ingest(&chess, &initial_confirmed)?;

    for (index, uci) in ["e2e4", "e7e5", "g1f3"].iter().enumerate() {
        sequence += 1;
        clock_us += 900_000;
        let moving = synthetic_observation(&chess, session_id, sequence, clock_us, true)?;
        let _ = decoder.ingest(&chess, &moving)?;

        let mut observed_position = chess.clone();
        observed_position
            .play_uci(uci)
            .context("construct synthetic post-move observation")?;
        let mut proposed = None;
        for delay_us in [50_000, 175_000, 225_000] {
            sequence += 1;
            let observation = synthetic_observation(
                &observed_position,
                session_id,
                sequence,
                clock_us + delay_us,
                false,
            )?;
            if let DecoderDecision::Proposed(candidate) = decoder.ingest(&chess, &observation)? {
                proposed = Some(candidate);
            }
        }
        let proposed = proposed.context("synthetic decoder did not propose a move")?;
        anyhow::ensure!(
            proposed.uci == *uci,
            "synthetic decoder proposed a wrong move"
        );

        let applied = chess.play_uci(uci).context("apply demo move")?.clone();
        let elapsed_since_previous = previous_completion
            .map(|previous| proposed.completion_bounds.elapsed_since(previous))
            .transpose()?;
        let record = MoveRecord {
            game_id,
            revision: 0,
            ply: applied.ply,
            uci: applied.uci,
            san: applied.san,
            fen_before: applied.fen_before,
            fen_after: applied.fen_after,
            provenance: MoveProvenance::Automatic,
            // The decoder score is an evidence likelihood, not a calibrated
            // probability that the move is correct.
            confidence: None,
            timing: MoveTiming {
                completion: Some(proposed.completion_bounds),
                elapsed_since_previous,
                confirmation_time: proposed.confirmation_time,
                quality: TimingQuality::Bounded,
            },
            evidence_ids: vec![format!("synthetic-frame-{sequence}")],
        };
        store
            .append_move(&format!("demo-{index}"), &record, index as i64)
            .context("persist demo move")?;
        previous_completion = Some(proposed.completion_bounds);
        decoder.commit(&chess)?;
        clock_us += 225_000;
    }

    Ok((game_id, chess))
}

fn export_replayed_game(store: &Store, game_id: GameId, expected: &ChessGame) -> Result<String> {
    let replayed = store
        .replay_active_game(game_id)
        .context("replay persisted demo game")?;
    anyhow::ensure!(replayed.final_fen == expected.fen(), "demo replay diverged");
    Ok(to_pgn(&PgnMetadata::default(), &replayed.moves, true)?)
}

fn synthetic_observation(
    game: &ChessGame,
    session_id: SessionId,
    sequence: u64,
    time_us: i64,
    moving: bool,
) -> Result<FrameObservation> {
    let squares = game
        .piece_classes()
        .into_iter()
        .map(|(square, piece)| {
            let mut piece_probabilities = [0.0001; PIECE_CLASS_COUNT];
            let empty_probability = match piece {
                Some(piece) => {
                    piece_probabilities[piece.index()] = 0.999;
                    0.0001
                }
                None => 0.999,
            };
            SquareEvidence {
                square,
                visible_probability: 1.0,
                empty_probability,
                piece_probabilities,
            }
        })
        .collect();
    Ok(FrameObservation {
        session_id,
        sequence,
        capture_time: CaptureTimeUs::new(time_us)?,
        moving,
        calibration_version: "synthetic-v1".to_owned(),
        model_version: "perfect-fixture-v1".to_owned(),
        squares,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_pipeline_decodes_persists_and_exports() {
        let pgn = build_demo_pgn().unwrap();
        assert!(pgn.contains("1. e4"));
        assert!(pgn.contains("e5"));
        assert!(pgn.contains("2. Nf3"));
        assert!(pgn.contains("elapsed"));
    }

    #[test]
    fn synthetic_pipeline_survives_database_reopen() {
        let marker = GameId::new().0;
        let path = std::env::temp_dir().join(format!("chess-pipeline-{marker}.sqlite"));
        let (game_id, expected) = {
            let mut store = Store::open(&path).unwrap();
            record_synthetic_game(&mut store).unwrap()
        };

        let reopened = Store::open(&path).unwrap();
        let pgn = export_replayed_game(&reopened, game_id, &expected).unwrap();
        assert!(pgn.contains("1. e4"));
        drop(reopened);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(format!("{}-wal", path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", path.display()));
    }
}
