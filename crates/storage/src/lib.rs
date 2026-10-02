use std::path::Path;
use std::time::Duration;

use chess_core::ChessGame;
use contracts::{
    CaptureTimeUs, GameId, MoveProvenance, MoveRecord, MoveTiming, TimeBounds, TimingQuality,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
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
        load_moves_in_transaction(&self.connection, game_id, revision)
    }

    pub fn replay_active_game(&self, game_id: GameId) -> Result<ReplayedGame, StorageError> {
        let (revision, initial_fen): (u32, String) = self
            .connection
            .query_row(
                "SELECT active_revision, initial_fen FROM games WHERE id = ?1",
                params![game_id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(StorageError::GameNotFound(game_id))?;
        let moves = self.load_moves(game_id, revision)?;
        let mut game = ChessGame::from_fen(&initial_fen)
            .map_err(|error| StorageError::InvalidInitialPosition(error.to_string()))?;
        for record in &moves {
            validate_record_against(&mut game, record, game_id, revision)?;
        }
        Ok(ReplayedGame {
            revision,
            initial_fen,
            final_fen: game.fen(),
            moves,
        })
    }

    pub fn revision_history(&self, game_id: GameId) -> Result<Vec<RevisionInfo>, StorageError> {
        let mut statement = self.connection.prepare(
            "SELECT revision, parent_revision, replace_from_ply, reason, created_utc_us
             FROM game_revisions WHERE game_id = ?1 ORDER BY revision",
        )?;
        let rows = statement.query_map(params![game_id.0.to_string()], |row| {
            Ok(RevisionInfo {
                revision: row.get(0)?,
                parent_revision: row.get(1)?,
                replace_from_ply: row.get(2)?,
                reason: row.get(3)?,
                created_utc_us: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn apply_correction(
        &mut self,
        correction: &CorrectionRevision,
    ) -> Result<CorrectionOutcome, StorageError> {
        let game_id = correction.game_id.0.to_string();
        let payload = serde_json::to_string(&(
            correction.parent_revision,
            correction.replace_from_ply,
            &correction.reason,
            &correction.replacement_suffix,
        ))?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(i64, String, String)> = transaction
            .query_row(
                "SELECT sequence, event_type, payload_json FROM events
                 WHERE game_id = ?1 AND idempotency_key = ?2",
                params![game_id, correction.idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((sequence, event_type, stored_payload)) = existing {
            if event_type != "correction" || stored_payload != payload {
                return Err(StorageError::IdempotencyConflict {
                    key: correction.idempotency_key.clone(),
                });
            }
            return Ok(CorrectionOutcome::AlreadyApplied {
                revision: correction.parent_revision + 1,
                sequence: sequence as u64,
            });
        }

        let (active_revision, head_sequence, initial_fen): (u32, i64, String) = transaction
            .query_row(
                "SELECT active_revision, head_sequence, initial_fen FROM games WHERE id = ?1",
                params![game_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(StorageError::GameNotFound(correction.game_id))?;
        if active_revision != correction.parent_revision {
            return Err(StorageError::RevisionMismatch {
                expected: active_revision,
                actual: correction.parent_revision,
            });
        }
        let parent_moves =
            load_moves_in_transaction(&transaction, correction.game_id, active_revision)?;
        let parent_len = parent_moves.len() as u32;
        if correction.replace_from_ply == 0 || correction.replace_from_ply > parent_len + 1 {
            return Err(StorageError::InvalidReplacementPly(
                correction.replace_from_ply,
            ));
        }
        let new_revision = active_revision + 1;
        let mut game = ChessGame::from_fen(&initial_fen)
            .map_err(|error| StorageError::InvalidInitialPosition(error.to_string()))?;
        for record in parent_moves
            .iter()
            .take((correction.replace_from_ply - 1) as usize)
        {
            validate_record_against(&mut game, record, correction.game_id, active_revision)?;
        }
        for record in &correction.replacement_suffix {
            validate_record_against(&mut game, record, correction.game_id, new_revision)?;
        }

        let sequence = head_sequence + 1;
        transaction.execute(
            "INSERT INTO events (
                game_id, sequence, event_id, idempotency_key, schema_version,
                event_type, payload_json, created_utc_us
             ) VALUES (?1, ?2, ?3, ?4, 1, 'correction', ?5, ?6)",
            params![
                game_id,
                sequence,
                Uuid::now_v7().to_string(),
                correction.idempotency_key,
                payload,
                correction.created_utc_us
            ],
        )?;
        transaction.execute(
            "INSERT INTO game_revisions (
                game_id, revision, parent_revision, replace_from_ply, reason,
                source_event_sequence, created_utc_us
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                game_id,
                new_revision,
                active_revision,
                correction.replace_from_ply,
                correction.reason,
                sequence,
                correction.created_utc_us
            ],
        )?;
        transaction.execute(
            "INSERT INTO moves (
                game_id, revision, ply, uci, san, fen_before, fen_after, provenance,
                confidence, completion_earliest_us, completion_latest_us,
                elapsed_earliest_us, elapsed_latest_us, confirmation_time_us,
                timing_quality, evidence_json, source_event_sequence
             ) SELECT game_id, ?3, ply, uci, san, fen_before, fen_after, provenance,
                confidence, completion_earliest_us, completion_latest_us,
                elapsed_earliest_us, elapsed_latest_us, confirmation_time_us,
                timing_quality, evidence_json, source_event_sequence
               FROM moves WHERE game_id = ?1 AND revision = ?2 AND ply < ?4",
            params![
                game_id,
                active_revision,
                new_revision,
                correction.replace_from_ply
            ],
        )?;
        for record in &correction.replacement_suffix {
            insert_move(&transaction, record, sequence)?;
        }
        transaction.execute(
            "UPDATE games SET active_revision = ?2, head_sequence = ?3 WHERE id = ?1",
            params![game_id, new_revision, sequence],
        )?;
        transaction.commit()?;
        Ok(CorrectionOutcome::Created {
            revision: new_revision,
            sequence: sequence as u64,
        })
    }
    fn migrate(&mut self) -> Result<(), StorageError> {
        let current: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current > SCHEMA_VERSION {
            return Err(StorageError::NewerSchema(current));
        }
        if current < SCHEMA_VERSION {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            if current == 0 {
                transaction.execute_batch(SCHEMA_V1)?;
            }
            transaction.execute_batch(SCHEMA_V2)?;
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

fn load_moves_in_transaction(
    connection: &Connection,
    game_id: GameId,
    revision: u32,
) -> Result<Vec<MoveRecord>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT ply, uci, san, fen_before, fen_after, provenance, confidence,
                completion_earliest_us, completion_latest_us,
                elapsed_earliest_us, elapsed_latest_us, confirmation_time_us,
                timing_quality, evidence_json
         FROM moves WHERE game_id = ?1 AND revision = ?2 ORDER BY ply",
    )?;
    let rows = statement.query_map(params![game_id.0.to_string(), revision], |row| {
        Ok(RawMove {
            ply: row.get(0)?,
            uci: row.get(1)?,
            san: row.get(2)?,
            fen_before: row.get(3)?,
            fen_after: row.get(4)?,
            provenance: row.get(5)?,
            confidence: row.get(6)?,
            completion_earliest: row.get(7)?,
            completion_latest: row.get(8)?,
            elapsed_earliest: row.get(9)?,
            elapsed_latest: row.get(10)?,
            confirmation_time: row.get(11)?,
            timing_quality: row.get(12)?,
            evidence_json: row.get(13)?,
        })
    })?;
    rows.map(|row| raw_to_move(game_id, revision, row?))
        .collect()
}

fn validate_appended_move(record: &MoveRecord) -> Result<(), StorageError> {
    let mut game = ChessGame::from_fen(&record.fen_before)
        .map_err(|error| StorageError::InvalidMove(error.to_string()))?;
    validate_move_details(&mut game, record)
}

fn validate_record_against(
    game: &mut ChessGame,
    record: &MoveRecord,
    game_id: GameId,
    revision: u32,
) -> Result<(), StorageError> {
    if record.game_id != game_id || record.revision != revision {
        return Err(StorageError::InvalidMove(
            "game or revision mismatch".into(),
        ));
    }
    let expected_ply = game.history().len() as u32 + 1;
    if record.ply != expected_ply {
        return Err(StorageError::PlyMismatch {
            expected: expected_ply,
            actual: record.ply,
        });
    }
    if record.fen_before != game.fen() {
        return Err(StorageError::DisconnectedPosition {
            expected: game.fen(),
            actual: record.fen_before.clone(),
        });
    }
    validate_move_details(game, record)
}

fn validate_move_details(game: &mut ChessGame, record: &MoveRecord) -> Result<(), StorageError> {
    let applied = game
        .play_uci(&record.uci)
        .map_err(|error| StorageError::InvalidMove(error.to_string()))?;
    if applied.uci != record.uci
        || applied.san != record.san
        || applied.fen_after != record.fen_after
    {
        return Err(StorageError::InvalidMove(format!(
            "canonical move, SAN, or final FEN mismatch at ply {}",
            record.ply
        )));
    }
    if record
        .confidence
        .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
    {
        return Err(StorageError::InvalidMove(
            "confidence must be between 0 and 1".into(),
        ));
    }
    Ok(())
}

fn insert_move(
    transaction: &Transaction<'_>,
    record: &MoveRecord,
    sequence: i64,
) -> Result<(), StorageError> {
    transaction.execute(
        "INSERT INTO moves (
            game_id, revision, ply, uci, san, fen_before, fen_after, provenance,
            confidence, completion_earliest_us, completion_latest_us,
            elapsed_earliest_us, elapsed_latest_us, confirmation_time_us,
            timing_quality, evidence_json, source_event_sequence
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            record.game_id.0.to_string(),
            record.revision,
            record.ply,
            record.uci,
            record.san,
            record.fen_before,
            record.fen_after,
            provenance_name(record.provenance),
            record.confidence,
            record.timing.completion.map(|bounds| bounds.earliest.get()),
            record.timing.completion.map(|bounds| bounds.latest.get()),
            record
                .timing
                .elapsed_since_previous
                .map(|bounds| bounds.earliest.get()),
            record
                .timing
                .elapsed_since_previous
                .map(|bounds| bounds.latest.get()),
            record.timing.confirmation_time.get(),
            timing_quality_name(record.timing.quality),
            serde_json::to_string(&record.evidence_ids)?,
            sequence,
        ],
    )?;
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

const SCHEMA_V2: &str = r#"
CREATE TABLE game_revisions (
    game_id TEXT NOT NULL REFERENCES games(id),
    revision INTEGER NOT NULL,
    parent_revision INTEGER,
    replace_from_ply INTEGER,
    reason TEXT,
    source_event_sequence INTEGER,
    created_utc_us INTEGER NOT NULL,
    PRIMARY KEY (game_id, revision),
    FOREIGN KEY (game_id, source_event_sequence) REFERENCES events(game_id, sequence)
);
INSERT INTO game_revisions (
    game_id, revision, parent_revision, replace_from_ply, reason,
    source_event_sequence, created_utc_us
) SELECT id, 0, NULL, NULL, NULL, NULL, created_utc_us FROM games;
"#;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Contract(#[from] contracts::ContractError),
    #[error("invalid initial position: {0}")]
    InvalidInitialPosition(String),
    #[error("invalid move: {0}")]
    InvalidMove(String),
    #[error("game definition conflicts with existing game: {0:?}")]
    GameDefinitionConflict(GameId),
    #[error("idempotency key already used with different content: {key}")]
    IdempotencyConflict { key: String },
    #[error("invalid correction replacement ply: {0}")]
    InvalidReplacementPly(u32),
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
        let mut game = ChessGame::standard();
        let applied = game.play_uci("e2e4").unwrap().clone();
        MoveRecord {
            game_id,
            revision: 0,
            ply: 1,
            uci: applied.uci,
            san: applied.san,
            fen_before: applied.fen_before,
            fen_after: applied.fen_after,
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
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 123)
            .unwrap();
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
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 123)
            .unwrap();
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

    #[test]
    fn correction_preserves_parent_and_replays_after_restart() {
        let game_id = GameId::new();
        let path = std::env::temp_dir().join(format!("chess-storage-{}.sqlite", Uuid::now_v7()));
        let original = record(game_id);
        let mut alternative = ChessGame::standard();
        let applied = alternative.play_uci("d2d4").unwrap().clone();
        let replacement = MoveRecord {
            revision: 1,
            uci: applied.uci,
            san: applied.san,
            fen_before: applied.fen_before,
            fen_after: applied.fen_after,
            provenance: MoveProvenance::Reviewed,
            ..original.clone()
        };
        let correction = CorrectionRevision {
            game_id,
            parent_revision: 0,
            idempotency_key: "review-1".into(),
            replace_from_ply: 1,
            reason: "visual evidence showed d4".into(),
            replacement_suffix: vec![replacement.clone()],
            created_utc_us: 300,
        };

        {
            let mut store = Store::open(&path).unwrap();
            store
                .create_game(game_id, ChessGame::standard().initial_fen(), 100)
                .unwrap();
            store.append_move("capture-1", &original, 200).unwrap();
            assert_eq!(
                store.apply_correction(&correction).unwrap(),
                CorrectionOutcome::Created {
                    revision: 1,
                    sequence: 2
                }
            );
        }

        {
            let mut store = Store::open(&path).unwrap();
            assert_eq!(store.load_moves(game_id, 0).unwrap(), vec![original]);
            let replayed = store.replay_active_game(game_id).unwrap();
            assert_eq!(replayed.revision, 1);
            assert_eq!(replayed.moves, vec![replacement]);
            assert_eq!(replayed.final_fen, alternative.fen());
            assert_eq!(store.revision_history(game_id).unwrap().len(), 2);
            assert_eq!(
                store.apply_correction(&correction).unwrap(),
                CorrectionOutcome::AlreadyApplied {
                    revision: 1,
                    sequence: 2
                }
            );
            assert!(matches!(
                store.append_move("capture-2", &record(game_id), 400),
                Err(StorageError::RevisionMismatch { .. })
            ));
        }
        std::fs::remove_file(&path).unwrap();
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn invalid_correction_leaves_active_revision_unchanged() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 100)
            .unwrap();
        let original = record(game_id);
        store.append_move("capture-1", &original, 200).unwrap();
        let mut bad = original.clone();
        bad.revision = 1;
        bad.san = "d4".into();
        let correction = CorrectionRevision {
            game_id,
            parent_revision: 0,
            idempotency_key: "bad-review".into(),
            replace_from_ply: 1,
            reason: "bad test".into(),
            replacement_suffix: vec![bad],
            created_utc_us: 300,
        };
        assert!(matches!(
            store.apply_correction(&correction),
            Err(StorageError::InvalidMove(_))
        ));
        assert_eq!(store.replay_active_game(game_id).unwrap().revision, 0);
        assert_eq!(store.revision_history(game_id).unwrap().len(), 1);
    }

    #[test]
    fn correction_copies_trusted_prefix_and_replaces_suffix() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 100)
            .unwrap();
        let first = record(game_id);
        store.append_move("capture-1", &first, 200).unwrap();

        let mut game = ChessGame::standard();
        game.play_uci("e2e4").unwrap();
        let original_second = game.play_uci("e7e5").unwrap().clone();
        let second = MoveRecord {
            ply: 2,
            uci: original_second.uci,
            san: original_second.san,
            fen_before: original_second.fen_before,
            fen_after: original_second.fen_after,
            ..first.clone()
        };
        store.append_move("capture-2", &second, 300).unwrap();

        let mut corrected_game = ChessGame::standard();
        corrected_game.play_uci("e2e4").unwrap();
        let corrected_second = corrected_game.play_uci("c7c5").unwrap().clone();
        let replacement = MoveRecord {
            revision: 1,
            ply: 2,
            uci: corrected_second.uci,
            san: corrected_second.san,
            fen_before: corrected_second.fen_before,
            fen_after: corrected_second.fen_after,
            provenance: MoveProvenance::Reviewed,
            ..first.clone()
        };
        store
            .apply_correction(&CorrectionRevision {
                game_id,
                parent_revision: 0,
                idempotency_key: "review-ply-2".into(),
                replace_from_ply: 2,
                reason: "black played c5".into(),
                replacement_suffix: vec![replacement.clone()],
                created_utc_us: 400,
            })
            .unwrap();

        let replayed = store.replay_active_game(game_id).unwrap();
        assert_eq!(replayed.final_fen, corrected_game.fen());
        assert_eq!(replayed.moves.len(), 2);
        assert_eq!(replayed.moves[0].revision, 1);
        assert_eq!(replayed.moves[0].uci, first.uci);
        assert_eq!(replayed.moves[1], replacement);
        assert_eq!(store.load_moves(game_id, 0).unwrap(), vec![first, second]);
    }

    #[test]
    fn upgrades_existing_v1_database() {
        let path = std::env::temp_dir().join(format!("chess-storage-v1-{}.sqlite", Uuid::now_v7()));
        let game_id = GameId::new();
        {
            let connection = Connection::open(&path).unwrap();
            connection.execute_batch(SCHEMA_V1).unwrap();
            connection.pragma_update(None, "user_version", 1).unwrap();
            connection
                .execute(
                    "INSERT INTO games (id, created_utc_us, initial_fen, result, status,
                  active_revision, head_sequence) VALUES (?1, 100, ?2, '*', 'recording', 0, 0)",
                    params![game_id.0.to_string(), ChessGame::standard().initial_fen()],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);
        assert_eq!(store.revision_history(game_id).unwrap().len(), 1);
        drop(store);
        std::fs::remove_file(&path).unwrap();
    }
}
