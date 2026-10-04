use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use chess_core::ChessGame;
use contracts::{
    CaptureTimeUs, GameId, MoveProvenance, MoveRecord, MoveTiming, TimeBounds, TimingQuality,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 4;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevisionInfo {
    pub revision: u32,
    pub parent_revision: Option<u32>,
    pub replace_from_ply: Option<u32>,
    pub reason: Option<String>,
    pub created_utc_us: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSummary {
    pub id: GameId,
    pub created_utc_us: i64,
    pub white: String,
    pub black: String,
    pub result: String,
    pub status: String,
    pub active_revision: u32,
    pub ply_count: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: String,
    pub game_id: GameId,
    pub session_id: String,
    pub capture_time_us: i64,
    pub sha256: String,
    pub bytes: u64,
    pub model_version: String,
    pub calibration_version: String,
    pub kind: String,
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

    /// Resume the most recently created game, including an empty recording.
    pub fn games(&self) -> Result<Vec<GameSummary>, StorageError> {
        let mut q = self.connection.prepare("SELECT g.id, g.created_utc_us, g.white, g.black, g.result, g.status, g.active_revision, (SELECT COUNT(*) FROM moves m WHERE m.game_id=g.id AND m.revision=g.active_revision) FROM games g ORDER BY g.created_utc_us DESC, g.id DESC")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, u32>(6)?,
                r.get::<_, u32>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (id, created_utc_us, white, black, result, status, active_revision, ply_count) =
                row?;
            Ok(GameSummary {
                id: GameId(
                    Uuid::parse_str(&id)
                        .map_err(|_| StorageError::InvalidCorrection("invalid stored ID".into()))?,
                ),
                created_utc_us,
                white,
                black,
                result,
                status,
                active_revision,
                ply_count,
            })
        })
        .collect()
    }

    pub fn metadata(&self, id: GameId) -> Result<GameSummary, StorageError> {
        self.games()?
            .into_iter()
            .find(|g| g.id == id)
            .ok_or(StorageError::GameNotFound(id))
    }

    pub fn set_metadata(
        &mut self,
        id: GameId,
        white: &str,
        black: &str,
        result: &str,
        status: &str,
        utc_us: i64,
    ) -> Result<(), StorageError> {
        if !["*", "1-0", "0-1", "1/2-1/2"].contains(&result)
            || !["recording", "paused", "finished", "incomplete", "verified"].contains(&status)
            || [white, black]
                .iter()
                .any(|v| v.len() > 200 || v.chars().any(char::is_control))
        {
            return Err(StorageError::InvalidCorrection(
                "invalid game metadata".into(),
            ));
        }
        // Verified status is reserved for a separate audited reference comparison.
        if status == "verified" {
            return Err(StorageError::InvalidCorrection(
                "verification requires an audited reference".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.execute(
            "UPDATE games SET white=?2, black=?3, result=?4, status=?5 WHERE id=?1",
            params![id.0.to_string(), white, black, result, status],
        )? != 1
        {
            return Err(StorageError::GameNotFound(id));
        }
        tx.execute("INSERT INTO activities(game_id, created_utc_us, kind, payload_json) VALUES (?1, ?2, 'metadata', ?3)", params![id.0.to_string(), utc_us, serde_json::to_string(&(white, black, result, status))?])?;
        tx.commit()?;
        Ok(())
    }

    pub fn activity(
        &self,
        id: GameId,
        kind: &str,
        payload: &serde_json::Value,
        utc_us: i64,
    ) -> Result<(), StorageError> {
        self.connection.execute("INSERT INTO activities(game_id, created_utc_us, kind, payload_json) VALUES (?1, ?2, ?3, ?4)", params![id.0.to_string(), utc_us, kind, serde_json::to_string(payload)?])?;
        Ok(())
    }
    pub fn activities(&self, id: GameId) -> Result<Vec<serde_json::Value>, StorageError> {
        let mut q = self.connection.prepare("SELECT id, created_utc_us, kind, payload_json FROM activities WHERE game_id=?1 ORDER BY id")?;
        let rows = q.query_map([id.0.to_string()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|r| { let (id, time, kind, payload) = r?; Ok(serde_json::json!({"id": id, "created_utc_us": time, "kind": kind, "payload": serde_json::from_str::<serde_json::Value>(&payload)?})) }).collect()
    }
    pub fn add_evidence(&self, evidence: &EvidenceRecord) -> Result<(), StorageError> {
        self.connection.execute(
            "INSERT INTO evidence(id, game_id, payload_json) VALUES (?1, ?2, ?3)",
            params![
                evidence.id,
                evidence.game_id.0.to_string(),
                serde_json::to_string(evidence)?
            ],
        )?;
        Ok(())
    }
    pub fn evidence(&self, id: GameId) -> Result<Vec<EvidenceRecord>, StorageError> {
        let mut q = self
            .connection
            .prepare("SELECT payload_json FROM evidence WHERE game_id=?1 ORDER BY rowid")?;
        let rows = q.query_map([id.0.to_string()], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    /// Immutable initial automatic decisions, including moves superseded by corrections.
    pub fn original_decisions(&self, id: GameId) -> Result<Vec<MoveRecord>, StorageError> {
        load_journal_events(&self.connection, id)?
            .into_iter()
            .filter(|e| e.event_type == "move_accepted")
            .map(|e| Ok(serde_json::from_str::<MoveRecord>(&e.payload_json)?))
            .collect()
    }
    /// SQLite creates a transactionally consistent snapshot including committed WAL pages.
    pub fn backup_to(&self, destination: &Path) -> Result<(), StorageError> {
        if destination.exists() {
            return Err(StorageError::InvalidCorrection(
                "backup destination exists".into(),
            ));
        }
        self.connection
            .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        Ok(())
    }

    pub fn latest_game_id(&self) -> Result<Option<GameId>, StorageError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM games ORDER BY created_utc_us DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(id.map(|id| GameId(Uuid::parse_str(&id).expect("stored game ID is a UUID"))))
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
        let revision: u32 = self
            .connection
            .query_row(
                "SELECT active_revision FROM games WHERE id = ?1",
                params![game_id.0.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::GameNotFound(game_id))?;
        self.replay_revision(game_id, revision)
    }

    /// Rebuilds one immutable revision through the rules engine and verifies every stored
    /// UCI/SAN/FEN link. This is the deterministic read path used after restart and for audits.
    pub fn replay_revision(
        &self,
        game_id: GameId,
        revision: u32,
    ) -> Result<ReplayedGame, StorageError> {
        let initial_fen: String = self
            .connection
            .query_row(
                "SELECT games.initial_fen
                 FROM games
                 JOIN game_revisions ON game_revisions.game_id = games.id
                 WHERE games.id = ?1 AND game_revisions.revision = ?2",
                params![game_id.0.to_string(), revision],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(StorageError::RevisionNotFound { game_id, revision })?;
        let moves = self.load_moves(game_id, revision)?;
        let game = validate_replay(&initial_fen, &moves, game_id, revision)?;
        Ok(ReplayedGame {
            revision,
            initial_fen,
            final_fen: game.fen(),
            moves,
        })
    }

    /// Rebuilds the active revision from the append-only event journal, then compares it with the
    /// stored move projection. This detects a missing/extra event, sequence gap, or projection
    /// divergence instead of trusting the denormalized `moves` table alone.
    pub fn replay_journal(&self, game_id: GameId) -> Result<ReplayedGame, StorageError> {
        let (initial_fen, projected_revision, head_sequence_raw): (String, u32, i64) = self
            .connection
            .query_row(
                "SELECT initial_fen, active_revision, head_sequence FROM games WHERE id = ?1",
                params![game_id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .ok_or(StorageError::GameNotFound(game_id))?;
        let head_sequence = u64::try_from(head_sequence_raw)
            .map_err(|_| StorageError::InvalidStoredSequence(head_sequence_raw))?;
        let events = load_journal_events(&self.connection, game_id)?;
        let mut revisions = BTreeMap::from([(0_u32, Vec::<MoveRecord>::new())]);
        let mut active_revision = 0_u32;
        let mut expected_sequence = 1_u64;

        for event in events {
            let sequence = u64::try_from(event.sequence)
                .map_err(|_| StorageError::InvalidStoredSequence(event.sequence))?;
            if sequence != expected_sequence {
                return Err(StorageError::JournalSequenceGap {
                    expected: expected_sequence,
                    actual: sequence,
                });
            }
            if event.schema_version != 1 {
                return Err(StorageError::UnsupportedEventSchema(event.schema_version));
            }
            match event.event_type.as_str() {
                "move_accepted" => {
                    let record: MoveRecord = serde_json::from_str(&event.payload_json)?;
                    if record.revision != active_revision {
                        return Err(StorageError::JournalRevisionMismatch {
                            expected: active_revision,
                            actual: record.revision,
                        });
                    }
                    revisions
                        .get_mut(&active_revision)
                        .expect("active journal revision exists")
                        .push(record);
                    validate_replay(
                        &initial_fen,
                        revisions
                            .get(&active_revision)
                            .expect("active journal revision exists"),
                        game_id,
                        active_revision,
                    )?;
                }
                "correction" => {
                    let (parent, replace_from_ply, _reason, suffix): (
                        u32,
                        u32,
                        String,
                        Vec<MoveRecord>,
                    ) = serde_json::from_str(&event.payload_json)?;
                    if parent != active_revision {
                        return Err(StorageError::JournalRevisionMismatch {
                            expected: active_revision,
                            actual: parent,
                        });
                    }
                    let parent_moves = revisions
                        .get(&parent)
                        .expect("active journal revision exists");
                    if replace_from_ply == 0 || replace_from_ply > parent_moves.len() as u32 + 1 {
                        return Err(StorageError::InvalidReplacementPly(replace_from_ply));
                    }
                    let new_revision = parent
                        .checked_add(1)
                        .ok_or(StorageError::RevisionOverflow)?;
                    let mut corrected = parent_moves
                        .iter()
                        .take((replace_from_ply - 1) as usize)
                        .cloned()
                        .map(|mut record| {
                            record.revision = new_revision;
                            record
                        })
                        .collect::<Vec<_>>();
                    corrected.extend(suffix);
                    validate_replay(&initial_fen, &corrected, game_id, new_revision)?;
                    revisions.insert(new_revision, corrected);
                    active_revision = new_revision;
                }
                event_type => return Err(StorageError::UnknownEventType(event_type.to_owned())),
            }
            expected_sequence += 1;
        }

        let journal_head = expected_sequence - 1;
        if journal_head != head_sequence {
            return Err(StorageError::JournalHeadMismatch {
                expected: head_sequence,
                actual: journal_head,
            });
        }
        if active_revision != projected_revision {
            return Err(StorageError::JournalRevisionMismatch {
                expected: projected_revision,
                actual: active_revision,
            });
        }
        let moves = revisions
            .remove(&active_revision)
            .expect("active journal revision exists");
        if moves != self.load_moves(game_id, active_revision)? {
            return Err(StorageError::ProjectionMismatch(active_revision));
        }
        let game = validate_replay(&initial_fen, &moves, game_id, active_revision)?;
        Ok(ReplayedGame {
            revision: active_revision,
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
        if correction.idempotency_key.trim().is_empty() {
            return Err(StorageError::InvalidCorrection(
                "idempotency key cannot be empty".into(),
            ));
        }
        if correction.reason.trim().is_empty() {
            return Err(StorageError::InvalidCorrection(
                "audit reason cannot be empty".into(),
            ));
        }
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
        let existing: Option<(i64, String, String, Option<u32>)> = transaction
            .query_row(
                "SELECT events.sequence, events.event_type, events.payload_json,
                        game_revisions.revision
                 FROM events
                 LEFT JOIN game_revisions
                   ON game_revisions.game_id = events.game_id
                  AND game_revisions.source_event_sequence = events.sequence
                 WHERE events.game_id = ?1 AND events.idempotency_key = ?2",
                params![game_id, correction.idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if let Some((sequence, event_type, stored_payload, revision)) = existing {
            if event_type != "correction" || stored_payload != payload {
                return Err(StorageError::IdempotencyConflict {
                    key: correction.idempotency_key.clone(),
                });
            }
            return Ok(CorrectionOutcome::AlreadyApplied {
                revision: revision.ok_or(StorageError::CorruptCorrectionEvent(sequence as u64))?,
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
        validate_replay(
            &initial_fen,
            &parent_moves,
            correction.game_id,
            active_revision,
        )?;
        let parent_len = parent_moves.len() as u32;
        if correction.replace_from_ply == 0 || correction.replace_from_ply > parent_len + 1 {
            return Err(StorageError::InvalidReplacementPly(
                correction.replace_from_ply,
            ));
        }
        let new_revision = active_revision
            .checked_add(1)
            .ok_or(StorageError::RevisionOverflow)?;
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
            if current < 1 {
                transaction.execute_batch(SCHEMA_V1)?;
            }
            if current < 2 {
                transaction.execute_batch(SCHEMA_V2)?;
            }
            if current < 3 {
                transaction.execute_batch(SCHEMA_V3)?;
            }
            if current < 4 {
                transaction.execute_batch(SCHEMA_V4)?;
            }
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

struct JournalEvent {
    sequence: i64,
    schema_version: u32,
    event_type: String,
    payload_json: String,
}

fn load_journal_events(
    connection: &Connection,
    game_id: GameId,
) -> Result<Vec<JournalEvent>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT sequence, schema_version, event_type, payload_json
         FROM events WHERE game_id = ?1 ORDER BY sequence",
    )?;
    let rows = statement.query_map(params![game_id.0.to_string()], |row| {
        Ok(JournalEvent {
            sequence: row.get(0)?,
            schema_version: row.get(1)?,
            event_type: row.get(2)?,
            payload_json: row.get(3)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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

fn validate_replay(
    initial_fen: &str,
    moves: &[MoveRecord],
    game_id: GameId,
    revision: u32,
) -> Result<ChessGame, StorageError> {
    let mut game = ChessGame::from_fen(initial_fen)
        .map_err(|error| StorageError::InvalidInitialPosition(error.to_string()))?;
    for record in moves {
        validate_record_against(&mut game, record, game_id, revision)?;
    }
    Ok(game)
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
    record.validate()?;
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
    let record = MoveRecord {
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
    };
    record.validate()?;
    Ok(record)
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

// Journal entries, revision metadata, and move projections are append-only. A correction creates
// another revision; it never rewrites or deletes the evidence needed to audit an older one.
const SCHEMA_V3: &str = r#"
CREATE TRIGGER events_are_immutable_before_update
BEFORE UPDATE ON events BEGIN
    SELECT RAISE(ABORT, 'events are immutable');
END;
CREATE TRIGGER events_are_immutable_before_delete
BEFORE DELETE ON events BEGIN
    SELECT RAISE(ABORT, 'events are immutable');
END;
CREATE TRIGGER moves_are_immutable_before_update
BEFORE UPDATE ON moves BEGIN
    SELECT RAISE(ABORT, 'moves are immutable');
END;
CREATE TRIGGER moves_are_immutable_before_delete
BEFORE DELETE ON moves BEGIN
    SELECT RAISE(ABORT, 'moves are immutable');
END;
CREATE TRIGGER revisions_are_immutable_before_update
BEFORE UPDATE ON game_revisions BEGIN
    SELECT RAISE(ABORT, 'game revisions are immutable');
END;
CREATE TRIGGER revisions_are_immutable_before_delete
BEFORE DELETE ON game_revisions BEGIN
    SELECT RAISE(ABORT, 'game revisions are immutable');
END;
"#;

const SCHEMA_V4: &str = r#"
ALTER TABLE games ADD COLUMN white TEXT NOT NULL DEFAULT '?';
ALTER TABLE games ADD COLUMN black TEXT NOT NULL DEFAULT '?';
CREATE TABLE activities(id INTEGER PRIMARY KEY, game_id TEXT NOT NULL REFERENCES games(id), created_utc_us INTEGER NOT NULL, kind TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE TABLE evidence(id TEXT PRIMARY KEY, game_id TEXT NOT NULL REFERENCES games(id), payload_json TEXT NOT NULL);
CREATE TRIGGER activities_no_update BEFORE UPDATE ON activities BEGIN SELECT RAISE(ABORT, 'activities are immutable'); END;
CREATE TRIGGER activities_no_delete BEFORE DELETE ON activities BEGIN SELECT RAISE(ABORT, 'activities are immutable'); END;
CREATE TRIGGER evidence_no_update BEFORE UPDATE ON evidence BEGIN SELECT RAISE(ABORT, 'evidence is immutable'); END;
CREATE TRIGGER evidence_no_delete BEFORE DELETE ON evidence BEGIN SELECT RAISE(ABORT, 'evidence is immutable'); END;
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
    #[error("invalid correction: {0}")]
    InvalidCorrection(String),
    #[error("correction event at sequence {0} has no revision projection")]
    CorruptCorrectionEvent(u64),
    #[error("revision number overflow")]
    RevisionOverflow,
    #[error("game not found: {0:?}")]
    GameNotFound(GameId),
    #[error("revision {revision} not found for game {game_id:?}")]
    RevisionNotFound { game_id: GameId, revision: u32 },
    #[error("journal sequence gap: expected {expected}, got {actual}")]
    JournalSequenceGap { expected: u64, actual: u64 },
    #[error("database contains an invalid event sequence: {0}")]
    InvalidStoredSequence(i64),
    #[error("unsupported event schema version: {0}")]
    UnsupportedEventSchema(u32),
    #[error("unknown journal event type: {0}")]
    UnknownEventType(String),
    #[error("journal revision mismatch: expected {expected}, got {actual}")]
    JournalRevisionMismatch { expected: u32, actual: u32 },
    #[error("journal head mismatch: expected {expected}, got {actual}")]
    JournalHeadMismatch { expected: u64, actual: u64 },
    #[error("move projection differs from journal replay for revision {0}")]
    ProjectionMismatch(u32),
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
    use std::process::Command;

    const KILL_TEST_DATABASE: &str = "CHESS_STORAGE_KILL_TEST_DATABASE";
    const KILL_TEST_GAME_ID: &str = "CHESS_STORAGE_KILL_TEST_GAME_ID";

    fn record(game_id: GameId) -> MoveRecord {
        records(game_id, 0, &["e2e4"], MoveProvenance::Automatic).remove(0)
    }

    fn records(
        game_id: GameId,
        revision: u32,
        uci_moves: &[&str],
        provenance: MoveProvenance,
    ) -> Vec<MoveRecord> {
        let mut game = ChessGame::standard();
        uci_moves
            .iter()
            .enumerate()
            .map(|(index, uci)| {
                let applied = game.play_uci(uci).unwrap().clone();
                let completion_start = 1_000 + index as i64 * 1_000;
                MoveRecord {
                    game_id,
                    revision,
                    ply: applied.ply,
                    uci: applied.uci,
                    san: applied.san,
                    fen_before: applied.fen_before,
                    fen_after: applied.fen_after,
                    provenance,
                    confidence: Some(0.999),
                    timing: MoveTiming {
                        completion: Some(
                            TimeBounds::new(
                                CaptureTimeUs::new(completion_start).unwrap(),
                                CaptureTimeUs::new(completion_start + 100).unwrap(),
                            )
                            .unwrap(),
                        ),
                        elapsed_since_previous: None,
                        confirmation_time: CaptureTimeUs::new(completion_start + 300).unwrap(),
                        quality: TimingQuality::Bounded,
                    },
                    evidence_ids: vec![format!("frame-{}", index + 10)],
                }
            })
            .collect()
    }

    fn remove_database(path: &Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
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
    fn duplicate_move_event_is_idempotent_after_restart_and_conflicts_are_rejected() {
        let game_id = GameId::new();
        let path = std::env::temp_dir().join(format!("chess-duplicate-{}.sqlite", Uuid::now_v7()));
        let accepted = record(game_id);
        {
            let mut store = Store::open(&path).unwrap();
            store
                .create_game(game_id, ChessGame::standard().initial_fen(), 100)
                .unwrap();
            assert_eq!(
                store
                    .append_move("camera-event-10", &accepted, 200)
                    .unwrap(),
                AppendOutcome::Appended { sequence: 1 }
            );
        }

        {
            let mut reopened = Store::open(&path).unwrap();
            assert_eq!(
                reopened
                    .append_move("camera-event-10", &accepted, 999)
                    .unwrap(),
                AppendOutcome::AlreadyApplied { sequence: 1 }
            );
            let mut conflicting = accepted.clone();
            conflicting.evidence_ids.push("different-frame".into());
            assert!(matches!(
                reopened.append_move("camera-event-10", &conflicting, 999),
                Err(StorageError::IdempotencyConflict { .. })
            ));
            assert_eq!(reopened.load_moves(game_id, 0).unwrap(), vec![accepted]);
            assert_eq!(reopened.replay_revision(game_id, 0).unwrap().moves.len(), 1);
        }
        remove_database(&path);
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
    fn move_projection_failure_rolls_back_journal_and_head() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 123)
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TEMP TRIGGER inject_move_projection_failure
                 BEFORE INSERT ON moves BEGIN
                    SELECT RAISE(ABORT, 'injected move projection failure');
                 END;",
            )
            .unwrap();

        assert!(matches!(
            store.append_move("capture-10", &record(game_id), 200),
            Err(StorageError::Sql(_))
        ));
        let (events, moves, head): (i64, i64, i64) = store
            .connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM events WHERE game_id = ?1),
                    (SELECT COUNT(*) FROM moves WHERE game_id = ?1),
                    head_sequence
                 FROM games WHERE id = ?1",
                params![game_id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!((events, moves, head), (0, 0, 0));

        store
            .connection
            .execute_batch("DROP TRIGGER inject_move_projection_failure;")
            .unwrap();
        assert_eq!(
            store
                .append_move("capture-10", &record(game_id), 200)
                .unwrap(),
            AppendOutcome::Appended { sequence: 1 }
        );
        assert_eq!(store.replay_journal(game_id).unwrap().moves.len(), 1);
    }

    #[test]
    fn head_update_failure_rolls_back_journal_and_projection() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 123)
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TEMP TRIGGER inject_head_update_failure
                 BEFORE UPDATE OF head_sequence ON games BEGIN
                    SELECT RAISE(ABORT, 'injected head update failure');
                 END;",
            )
            .unwrap();

        assert!(matches!(
            store.append_move("capture-10", &record(game_id), 200),
            Err(StorageError::Sql(_))
        ));
        assert!(store.load_moves(game_id, 0).unwrap().is_empty());
        let (event_count, head): (i64, i64) = store
            .connection
            .query_row(
                "SELECT
                    (SELECT COUNT(*) FROM events WHERE game_id = ?1),
                    head_sequence
                 FROM games WHERE id = ?1",
                params![game_id.0.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((event_count, head), (0, 0));
    }

    #[test]
    fn process_termination_rolls_back_an_open_write_transaction() {
        if let (Ok(path), Ok(game_id)) = (
            std::env::var(KILL_TEST_DATABASE),
            std::env::var(KILL_TEST_GAME_ID),
        ) {
            let connection = Connection::open(path).unwrap();
            configure(&connection, true).unwrap();
            connection.execute_batch("BEGIN IMMEDIATE").unwrap();
            connection
                .execute(
                    "INSERT INTO events (
                        game_id, sequence, event_id, idempotency_key, schema_version,
                        event_type, payload_json, created_utc_us
                     ) VALUES (?1, 1, ?2, 'capture-10', 1, 'move_accepted', '{}', 200)",
                    params![game_id, Uuid::now_v7().to_string()],
                )
                .unwrap();
            // `exit` skips Rust destructors, leaving SQLite and the OS to recover the open WAL
            // transaction just as they must after an abruptly terminated recorder process.
            std::process::exit(86);
        }

        let game_id = GameId::new();
        let path = std::env::temp_dir().join(format!("chess-kill-{}.sqlite", Uuid::now_v7()));
        {
            let mut store = Store::open(&path).unwrap();
            store
                .create_game(game_id, ChessGame::standard().initial_fen(), 100)
                .unwrap();
        }

        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::process_termination_rolls_back_an_open_write_transaction",
                "--nocapture",
            ])
            .env(KILL_TEST_DATABASE, &path)
            .env(KILL_TEST_GAME_ID, game_id.0.to_string())
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(86));

        {
            let mut reopened = Store::open(&path).unwrap();
            let replayed = reopened.replay_journal(game_id).unwrap();
            assert!(replayed.moves.is_empty());
            assert_eq!(
                reopened
                    .append_move("capture-10", &record(game_id), 300)
                    .unwrap(),
                AppendOutcome::Appended { sequence: 1 }
            );
            assert_eq!(reopened.replay_journal(game_id).unwrap().moves.len(), 1);
        }
        remove_database(&path);
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
            assert_eq!(store.replay_revision(game_id, 0).unwrap().moves.len(), 1);
            let replayed = store.replay_active_game(game_id).unwrap();
            assert_eq!(replayed.revision, 1);
            assert_eq!(replayed.moves, vec![replacement]);
            assert_eq!(replayed.final_fen, alternative.fen());
            assert_eq!(store.replay_journal(game_id).unwrap(), replayed);
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

            let mut conflicting = correction.clone();
            conflicting.reason = "a different decision".into();
            assert!(matches!(
                store.apply_correction(&conflicting),
                Err(StorageError::IdempotencyConflict { .. })
            ));
        }
        remove_database(&path);
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
    fn correction_revalidates_the_entire_replacement_suffix() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 100)
            .unwrap();
        let original = records(
            game_id,
            0,
            &["e2e4", "d7d5", "e4d5"],
            MoveProvenance::Automatic,
        );
        for (index, record) in original.iter().enumerate() {
            store
                .append_move(&format!("capture-{index}"), record, 200 + index as i64)
                .unwrap();
        }

        let mut replacement = records(game_id, 1, &["d2d4", "d7d5"], MoveProvenance::Reviewed);
        // Reusing the old third ply after changing the first move makes its board link invalid.
        let mut stale_tail = original[2].clone();
        stale_tail.revision = 1;
        replacement.push(stale_tail);
        let correction = CorrectionRevision {
            game_id,
            parent_revision: 0,
            idempotency_key: "invalid-downstream-suffix".into(),
            replace_from_ply: 1,
            reason: "first move was d4".into(),
            replacement_suffix: replacement,
            created_utc_us: 500,
        };

        assert!(matches!(
            store.apply_correction(&correction),
            Err(StorageError::DisconnectedPosition { .. })
        ));
        assert_eq!(store.revision_history(game_id).unwrap().len(), 1);
        assert!(store.load_moves(game_id, 1).unwrap().is_empty());
        assert_eq!(store.replay_active_game(game_id).unwrap().revision, 0);
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
        assert_eq!(store.replay_journal(game_id).unwrap(), replayed);
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
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        assert_eq!(store.revision_history(game_id).unwrap().len(), 1);
        drop(store);
        remove_database(&path);
    }

    #[test]
    fn journal_moves_and_revision_history_are_immutable() {
        let game_id = GameId::new();
        let mut store = Store::open_in_memory().unwrap();
        store
            .create_game(game_id, ChessGame::standard().initial_fen(), 100)
            .unwrap();
        let accepted = record(game_id);
        store.append_move("capture-1", &accepted, 200).unwrap();

        assert!(
            store
                .connection
                .execute(
                    "UPDATE moves SET san = 'tampered' WHERE game_id = ?1",
                    params![game_id.0.to_string()],
                )
                .is_err()
        );
        assert!(
            store
                .connection
                .execute(
                    "DELETE FROM events WHERE game_id = ?1",
                    params![game_id.0.to_string()],
                )
                .is_err()
        );
        assert!(
            store
                .connection
                .execute(
                    "UPDATE game_revisions SET reason = 'tampered' WHERE game_id = ?1",
                    params![game_id.0.to_string()],
                )
                .is_err()
        );
        assert_eq!(
            store.replay_revision(game_id, 0).unwrap().moves,
            vec![accepted]
        );
    }
}
