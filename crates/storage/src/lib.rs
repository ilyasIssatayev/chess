use std::path::Path;
use std::time::Duration;

use chess_core::ChessGame;
use contracts::{
    CaptureTimeUs, GameId, MoveProvenance, MoveRecord, MoveTiming, TimeBounds, TimingQuality,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 2;

pub struct Store {
    connection: Connection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended { sequence: u64 },
    AlreadyApplied { sequence: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionRevision {
    pub game_id: GameId,
    pub parent_revision: u32,
    pub idempotency_key: String,
    /// The first ply replaced in the parent revision. `parent_len + 1` appends a suffix.
    pub replace_from_ply: u32,
    pub reason: String,
    /// The complete replacement suffix, numbered from `replace_from_ply` and assigned to the
    /// revision that follows `parent_revision`. An empty suffix truncates the game.
    pub replacement_suffix: Vec<MoveRecord>,
    pub created_utc_us: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionOutcome {
    Created { revision: u32, sequence: u64 },
    AlreadyApplied { revision: u32, sequence: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReplayedGame {
    pub revision: u32,
    pub initial_fen: String,
    pub final_fen: String,
    pub moves: Vec<MoveRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionInfo {
    pub revision: u32,
    pub parent_revision: Option<u32>,
    pub replace_from_ply: Option<u32>,
    pub reason: Option<String>,
    pub created_utc_us: i64,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let connection = Connection::open(path)?;
        configure(&connection, true)?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, StorageError> {
        let connection = Connection::open_in_memory()?;
        configure(&connection, false)?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    pub fn schema_version(&self) -> Result<i64, StorageError> {
        Ok(self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    pub fn create_game(
        &mut self,
        game_id: GameId,
        initial_fen: &str,
        created_utc_us: i64,
    ) -> Result<(), StorageError> {
        // Parse before writing so a game can always be replayed by the rules engine.
        ChessGame::from_fen(initial_fen)
            .map_err(|error| StorageError::InvalidInitialPosition(error.to_string()))?;

        let game_id_text = game_id.0.to_string();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, i64)> = transaction
            .query_row(
                "SELECT initial_fen, created_utc_us FROM games WHERE id = ?1",
                params![game_id_text],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((stored_fen, stored_created_utc_us)) = existing {
            if stored_fen != initial_fen || stored_created_utc_us != created_utc_us {
                return Err(StorageError::GameDefinitionConflict(game_id));
            }
            transaction.commit()?;
            return Ok(());
        }

        transaction.execute(
            "INSERT INTO games (
                id, created_utc_us, initial_fen, result, status, active_revision, head_sequence
             ) VALUES (?1, ?2, ?3, '*', 'recording', 0, 0)",
            params![game_id_text, created_utc_us, initial_fen],
        )?;
        transaction.execute(
            "INSERT INTO game_revisions (
                game_id, revision, parent_revision, replace_from_ply, reason,
                source_event_sequence, created_utc_us
             ) VALUES (?1, 0, NULL, NULL, NULL, NULL, ?2)",
            params![game_id_text, created_utc_us],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn append_move(
        &mut self,
        idempotency_key: &str,
        record: &MoveRecord,
        event_created_utc_us: i64,
    ) -> Result<AppendOutcome, StorageError> {
        let game_id = record.game_id.0.to_string();
        let payload = serde_json::to_string(record)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        let existing: Option<(i64, String, String)> = transaction
            .query_row(
                "SELECT sequence, event_type, payload_json
                 FROM events WHERE game_id = ?1 AND idempotency_key = ?2",
                params![game_id, idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((sequence, event_type, stored_payload)) = existing {
            if event_type != "move_accepted" || stored_payload != payload {
                return Err(StorageError::IdempotencyConflict {
                    key: idempotency_key.to_owned(),
                });
            }
            transaction.commit()?;
            return Ok(AppendOutcome::AlreadyApplied {
                sequence: sequence as u64,
            });
        }

        let (active_revision, head_sequence, expected_fen): (i64, i64, String) = transaction
            .query_row(
                "SELECT active_revision, head_sequence,
                    COALESCE(
                        (SELECT fen_after FROM moves
                         WHERE game_id = games.id AND revision = games.active_revision
                         ORDER BY ply DESC LIMIT 1),
                        initial_fen
                    )
                 FROM games WHERE id = ?1",
                params![game_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(StorageError::GameNotFound(record.game_id))?;

        if active_revision as u32 != record.revision {
            return Err(StorageError::RevisionMismatch {
                expected: active_revision as u32,
                actual: record.revision,
            });
        }

        let expected_ply: i64 = transaction.query_row(
            "SELECT COUNT(*) + 1 FROM moves WHERE game_id = ?1 AND revision = ?2",
            params![game_id, active_revision],
            |row| row.get(0),
        )?;
        if expected_ply as u32 != record.ply {
            return Err(StorageError::PlyMismatch {
                expected: expected_ply as u32,
                actual: record.ply,
            });
        }
        if expected_fen != record.fen_before {
            return Err(StorageError::DisconnectedPosition {
                expected: expected_fen,
                actual: record.fen_before.clone(),
            });
        }
        validate_appended_move(record)?;

        let sequence = head_sequence + 1;
        let event_id = Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO events (
                game_id, sequence, event_id, idempotency_key, schema_version,
                event_type, payload_json, created_utc_us
             ) VALUES (?1, ?2, ?3, ?4, 1, 'move_accepted', ?5, ?6)",
            params![
                game_id,
                sequence,
                event_id,
                idempotency_key,
                payload,
                event_created_utc_us
            ],
        )?;

        insert_move(&transaction, record, sequence)?;
        transaction.execute(
            "UPDATE games SET head_sequence = ?2 WHERE id = ?1",
            params![game_id, sequence],
        )?;
        transaction.commit()?;

        Ok(AppendOutcome::Appended {
            sequence: sequence as u64,
        })
    }

    pub fn load_moves(
        &self,
        game_id: GameId,
        revision: u32,
    ) -> Result<Vec<MoveRecord>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT ply, uci, san, fen_before, fen_after, provenance, confidence,
                    completion_earliest_us, completion_latest_us,
                    elapsed_earliest_us, elapsed_latest_us, confirmation_time_us,
                    timing_quality, evidence_json
             FROM moves WHERE game_id = ?1 AND revision = ?2 ORDER BY ply",
        )?;
        let rows = statement.query_map(params![game_id.0.to_string(), revision], |row| {
            let completion_earliest: Option<i64> = row.get(7)?;
            let completion_latest: Option<i64> = row.get(8)?;
            let elapsed_earliest: Option<i64> = row.get(9)?;
            let elapsed_latest: Option<i64> = row.get(10)?;
            Ok(RawMove {
                ply: row.get(0)?,
                uci: row.get(1)?,
                san: row.get(2)?,
                fen_before: row.get(3)?,
                fen_after: row.get(4)?,
                provenance: row.get(5)?,
                confidence: row.get(6)?,
                completion_earliest,
                completion_latest,
                elapsed_earliest,
                elapsed_latest,
                confirmation_time: row.get(11)?,
                timing_quality: row.get(12)?,
                evidence_json: row.get(13)?,
            })
        })?;

        rows.map(|row| raw_to_move(game_id, revision, row?))
            .collect()
    }

    fn migrate(&mut self) -> Result<(), StorageError> {
        let current: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current > SCHEMA_VERSION {
            return Err(StorageError::NewerSchema(current));
        }
        if current == 0 {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(SCHEMA_V1)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        Ok(())
    }
}

fn configure(connection: &Connection, persistent: bool) -> Result<(), rusqlite::Error> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    if persistent {
        connection.pragma_update(None, "journal_mode", "WAL")?;
    }
    Ok(())
}

struct RawMove {
    ply: u32,
    uci: String,
    san: String,
    fen_before: String,
    fen_after: String,
    provenance: String,
    confidence: Option<f32>,
    completion_earliest: Option<i64>,
    completion_latest: Option<i64>,
    elapsed_earliest: Option<i64>,
    elapsed_latest: Option<i64>,
    confirmation_time: i64,
    timing_quality: String,
    evidence_json: String,
}

fn raw_to_move(game_id: GameId, revision: u32, raw: RawMove) -> Result<MoveRecord, StorageError> {
    Ok(MoveRecord {
        game_id,
        revision,
        ply: raw.ply,
        uci: raw.uci,
        san: raw.san,
        fen_before: raw.fen_before,
        fen_after: raw.fen_after,
        provenance: parse_provenance(&raw.provenance)?,
        confidence: raw.confidence,
        timing: MoveTiming {
            completion: parse_bounds(raw.completion_earliest, raw.completion_latest)?,
            elapsed_since_previous: parse_bounds(raw.elapsed_earliest, raw.elapsed_latest)?,
            confirmation_time: CaptureTimeUs::new(raw.confirmation_time)?,
            quality: parse_timing_quality(&raw.timing_quality)?,
        },
        evidence_ids: serde_json::from_str(&raw.evidence_json)?,
    })
}

fn parse_bounds(
    earliest: Option<i64>,
    latest: Option<i64>,
) -> Result<Option<TimeBounds>, StorageError> {
    match (earliest, latest) {
        (None, None) => Ok(None),
        (Some(earliest), Some(latest)) => Ok(Some(TimeBounds::new(
            CaptureTimeUs::new(earliest)?,
            CaptureTimeUs::new(latest)?,
        )?)),
        _ => Err(StorageError::CorruptTimingBounds),
    }
}

fn provenance_name(value: MoveProvenance) -> &'static str {
    match value {
        MoveProvenance::Automatic => "automatic",
        MoveProvenance::Reviewed => "reviewed",
        MoveProvenance::Inferred => "inferred",
        MoveProvenance::Manual => "manual",
    }
}

fn parse_provenance(value: &str) -> Result<MoveProvenance, StorageError> {
    match value {
        "automatic" => Ok(MoveProvenance::Automatic),
        "reviewed" => Ok(MoveProvenance::Reviewed),
        "inferred" => Ok(MoveProvenance::Inferred),
        "manual" => Ok(MoveProvenance::Manual),
        value => Err(StorageError::CorruptEnum(value.to_owned())),
    }
}

fn timing_quality_name(value: TimingQuality) -> &'static str {
    match value {
        TimingQuality::Observed => "observed",
        TimingQuality::Bounded => "bounded",
        TimingQuality::Unknown => "unknown",
    }
}

fn parse_timing_quality(value: &str) -> Result<TimingQuality, StorageError> {
    match value {
        "observed" => Ok(TimingQuality::Observed),
        "bounded" => Ok(TimingQuality::Bounded),
        "unknown" => Ok(TimingQuality::Unknown),
        value => Err(StorageError::CorruptEnum(value.to_owned())),
    }
}

const SCHEMA_V1: &str = r#"
CREATE TABLE games (
    id TEXT PRIMARY KEY,
    created_utc_us INTEGER NOT NULL,
    initial_fen TEXT NOT NULL,
    result TEXT NOT NULL,
    status TEXT NOT NULL,
    active_revision INTEGER NOT NULL,
    head_sequence INTEGER NOT NULL
);

CREATE TABLE events (
    game_id TEXT NOT NULL REFERENCES games(id),
    sequence INTEGER NOT NULL,
    event_id TEXT NOT NULL UNIQUE,
    idempotency_key TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_utc_us INTEGER NOT NULL,
    PRIMARY KEY (game_id, sequence),
    UNIQUE (game_id, idempotency_key)
);

CREATE TABLE moves (
    game_id TEXT NOT NULL REFERENCES games(id),
    revision INTEGER NOT NULL,
    ply INTEGER NOT NULL,
    uci TEXT NOT NULL,
    san TEXT NOT NULL,
    fen_before TEXT NOT NULL,
    fen_after TEXT NOT NULL,
    provenance TEXT NOT NULL,
    confidence REAL,
    completion_earliest_us INTEGER,
    completion_latest_us INTEGER,
    elapsed_earliest_us INTEGER,
    elapsed_latest_us INTEGER,
    confirmation_time_us INTEGER NOT NULL,
    timing_quality TEXT NOT NULL,
    evidence_json TEXT NOT NULL,
    source_event_sequence INTEGER NOT NULL,
    PRIMARY KEY (game_id, revision, ply),
    FOREIGN KEY (game_id, source_event_sequence) REFERENCES events(game_id, sequence)
);
"#;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] contracts::ContractError),
    #[error("game not found: {0:?}")]
    GameNotFound(GameId),
    #[error("move revision mismatch: expected {expected}, got {actual}")]
    RevisionMismatch { expected: u32, actual: u32 },
    #[error("move ply mismatch: expected {expected}, got {actual}")]
    PlyMismatch { expected: u32, actual: u32 },
    #[error("move does not connect to the trusted position: expected {expected}, got {actual}")]
    DisconnectedPosition { expected: String, actual: String },
    #[error("database schema {0} is newer than this application supports")]
    NewerSchema(i64),
    #[error("database contains a partial timing range")]
    CorruptTimingBounds,
    #[error("database contains unknown enum value: {0}")]
    CorruptEnum(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{MoveProvenance, TimingQuality};

    fn record(game_id: GameId) -> MoveRecord {
        MoveRecord {
            game_id,
            revision: 0,
            ply: 1,
            uci: "e2e4".into(),
            san: "e4".into(),
            fen_before: "start".into(),
            fen_after: "after-e4".into(),
            provenance: MoveProvenance::Automatic,
            confidence: Some(0.999),
            timing: MoveTiming {
                completion: Some(
                    TimeBounds::new(
                        CaptureTimeUs::new(1_000).unwrap(),
                        CaptureTimeUs::new(1_100).unwrap(),
                    )
                    .unwrap(),
                ),
                elapsed_since_previous: None,
                confirmation_time: CaptureTimeUs::new(1_300).unwrap(),
                quality: TimingQuality::Bounded,
            },
            evidence_ids: vec!["frame-10".into()],
        }
    }

    #[test]
    fn appends_idempotently_and_reloads() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store.create_game(game_id, "start", 123).unwrap();
        let record = record(game_id);

        assert_eq!(
            store.append_move("capture-10", &record, 200).unwrap(),
            AppendOutcome::Appended { sequence: 1 }
        );
        assert_eq!(
            store.append_move("capture-10", &record, 200).unwrap(),
            AppendOutcome::AlreadyApplied { sequence: 1 }
        );
        assert_eq!(store.load_moves(game_id, 0).unwrap(), vec![record]);
    }

    #[test]
    fn rejects_disconnected_move_without_partial_event() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store.create_game(game_id, "start", 123).unwrap();
        let mut record = record(game_id);
        record.fen_before = "wrong".into();

        assert!(matches!(
            store.append_move("capture-10", &record, 200),
            Err(StorageError::DisconnectedPosition { .. })
        ));
        assert!(store.load_moves(game_id, 0).unwrap().is_empty());
    }

    #[test]
    fn schema_is_versioned() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    }
}
