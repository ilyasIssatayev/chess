//! Validated neural evidence -> existing temporal/legal-position decoder.
use anyhow::{Result, ensure};
use chess_core::ChessGame;
use contracts::{FrameObservation, SessionId};
use recorder_core::{DecoderConfig, DecoderDecision, ProposedMove, TemporalDecoder};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub const MANIFEST: &str = include_str!("../models/vision-manifest.json");
pub const MODEL_VERSION: &str = "crisp-photo-4690fd5418ff-crops-v2";

fn recognized_version(version: &str) -> bool {
    version == MODEL_VERSION
        || version
            .strip_prefix(&format!("{MODEL_VERSION}:personal:"))
            .is_some_and(|id| {
                id.len() == 64
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            })
}

#[derive(Clone)]
pub struct VisionSession {
    pub id: SessionId,
    decoder: TemporalDecoder,
    last: FrameObservation,
    received: Instant,
    proposal: Option<(String, ProposedMove)>,
}

fn validate(observation: &FrameObservation) -> Result<()> {
    observation.validate()?;
    ensure!(
        recognized_version(&observation.model_version),
        "Unrecognized model version. Reload the recorder."
    );
    for square in &observation.squares {
        let total = square.empty_probability + square.piece_probabilities.iter().sum::<f32>();
        ensure!(
            (total - 1.0).abs() < 0.005,
            "Invalid normalized model evidence for {}",
            square.square
        );
    }
    Ok(())
}

fn contradictions(game: &ChessGame, observation: &FrameObservation, minimum: f32) -> Vec<String> {
    game.piece_classes()
        .iter()
        .filter_map(|(name, piece)| {
            let evidence = observation
                .squares
                .iter()
                .find(|square| &square.square == name);
            match evidence {
                Some(e) if e.visible_probability >= 0.5 && e.probability_for(*piece) >= minimum => {
                    None
                }
                _ => Some(name.clone()),
            }
        })
        .collect()
}

fn supports(game: &ChessGame, observation: &FrameObservation) -> bool {
    if observation.moving || !contradictions(game, observation, 0.10).is_empty() {
        return false;
    }
    let sum: f64 = game
        .piece_classes()
        .iter()
        .map(|(name, piece)| {
            observation
                .squares
                .iter()
                .find(|e| &e.square == name)
                .unwrap()
                .probability_for(*piece)
                .max(1e-6)
                .ln() as f64
        })
        .sum();
    sum / 64.0 >= -0.75
}

impl VisionSession {
    /// Personal heads need physical qualification; reviewed proposals remain usable.
    pub fn allows_automatic(&self) -> bool {
        self.last.model_version == MODEL_VERSION
    }

    pub fn begin(game: &ChessGame, observation: FrameObservation) -> Result<Self> {
        validate(&observation)?;
        ensure!(
            !observation.moving && observation.sequence == 0,
            "Reference must be a settled frame from a new session."
        );
        let mismatches = contradictions(game, &observation, 0.35);
        ensure!(
            mismatches.is_empty(),
            "The model cannot verify the tracked position at {}. Check corners, lighting, piece visibility and the physical position.",
            mismatches.join(", ")
        );
        // The browser already waits 900 ms for pixel motion to settle. Neural
        // moves still require three independent supported model observations.
        let mut decoder = TemporalDecoder::new(DecoderConfig {
            settle_time_us: 0,
            consistent_frames: 3,
            ..DecoderConfig::default()
        })?;
        ensure!(
            matches!(decoder.ingest(game, &observation)?, DecoderDecision::NoMove),
            "The reference is ambiguous. Improve the camera view and try again."
        );
        Ok(Self {
            id: observation.session_id,
            decoder,
            last: observation,
            received: Instant::now(),
            proposal: None,
        })
    }

    pub fn observe(&mut self, game: &ChessGame, observation: FrameObservation) -> Result<Value> {
        validate(&observation)?;
        ensure!(
            observation.session_id == self.id,
            "Camera session changed. Set a fresh reference."
        );
        ensure!(
            self.received.elapsed() <= Duration::from_secs(15),
            "Model observations were interrupted. Set a fresh reference."
        );
        if let Some((_, proposed)) = &self.proposal {
            let expected = ChessGame::from_fen(&proposed.fen_after)?;
            if !supports(&expected, &observation) {
                self.decoder.invalidate_candidate();
                self.proposal = None;
            }
        }
        let decision = self.decoder.ingest(game, &observation)?;
        self.last = observation;
        self.received = Instant::now();
        match decision {
            DecoderDecision::NoMove => {
                Ok(json!({"kind": if self.last.moving { "moving" } else { "unchanged" }}))
            }
            DecoderDecision::Settling => Ok(json!({"kind": "settling"})),
            DecoderDecision::NeedsReview { reason, candidates } => {
                Ok(json!({"kind": "review", "reason": reason, "candidates": candidates}))
            }
            DecoderDecision::Proposed(proposed) | DecoderDecision::AwaitingCommit(proposed) => {
                let id = match &self.proposal {
                    Some((id, _)) => id.clone(),
                    None => format!("{}:{}", self.id.0, self.last.sequence),
                };
                self.proposal = Some((id.clone(), proposed.clone()));
                Ok(
                    json!({"kind": "candidate", "uci": proposed.uci, "proposal_id": id, "session_id": self.id, "margin": proposed.margin, "score": proposed.average_log_likelihood}),
                )
            }
        }
    }

    pub fn approves(
        &self,
        uci: &str,
        session: Option<SessionId>,
        proposal_id: Option<&str>,
    ) -> bool {
        if session != Some(self.id) || self.received.elapsed() > Duration::from_secs(8) {
            return false;
        }
        self.proposal.as_ref().is_some_and(|(id, proposed)| {
            proposal_id == Some(id.as_str())
                && proposed.uci == uci
                && ChessGame::from_fen(&proposed.fen_after)
                    .is_ok_and(|game| supports(&game, &self.last))
        })
    }

    pub fn proposed(&self) -> Option<&ProposedMove> {
        self.proposal.as_ref().map(|(_, p)| p)
    }
    pub fn last_observation(&self) -> &FrameObservation {
        &self.last
    }
    pub fn is_fresh(&self) -> bool {
        self.received.elapsed() <= Duration::from_secs(8)
    }

    pub fn commit(&mut self, game: &ChessGame) -> Result<()> {
        self.decoder.commit(game)?;
        self.proposal = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{CaptureTimeUs, SquareEvidence};
    fn observation(
        game: &ChessGame,
        id: SessionId,
        sequence: u64,
        moving: bool,
    ) -> FrameObservation {
        FrameObservation {
            session_id: id,
            sequence,
            capture_time: CaptureTimeUs::new(sequence as i64 * 500_000).unwrap(),
            moving,
            calibration_version: "test-corners".into(),
            model_version: MODEL_VERSION.into(),
            squares: game
                .piece_classes()
                .iter()
                .map(|(square, piece)| {
                    let mut probabilities = [0.00001; 12];
                    let empty = match piece {
                        Some(piece) => {
                            probabilities[piece.index()] = 0.99988;
                            0.00001
                        }
                        None => 0.99988,
                    };
                    SquareEvidence {
                        square: square.clone(),
                        visible_probability: 1.0,
                        empty_probability: empty,
                        piece_probabilities: probabilities,
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn neural_observations_propose_commit_and_track_the_next_move() {
        let id = SessionId::new();
        let game = ChessGame::standard();
        let mut session = VisionSession::begin(&game, observation(&game, id, 0, false)).unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        assert_eq!(
            session
                .observe(&game, observation(&after, id, 1, true))
                .unwrap()["kind"],
            "moving"
        );
        for sequence in [2, 3] {
            assert_eq!(
                session
                    .observe(&game, observation(&after, id, sequence, false))
                    .unwrap()["kind"],
                "settling"
            );
        }
        let proposal = session
            .observe(&game, observation(&after, id, 4, false))
            .unwrap();
        assert_eq!(proposal["uci"], "e2e4");
        assert!(!session.approves("e2e4", Some(id), Some("forged")));
        assert!(session.approves("e2e4", Some(id), proposal["proposal_id"].as_str()));
        session.commit(&after).unwrap();
        assert_eq!(
            session
                .observe(&after, observation(&after, id, 5, false))
                .unwrap()["kind"],
            "unchanged"
        );
        let mut reply = after.clone();
        reply.play_uci("e7e5").unwrap();
        for sequence in [6, 7, 8] {
            session
                .observe(&after, observation(&reply, id, sequence, false))
                .unwrap();
        }
        assert!(
            session
                .proposal
                .as_ref()
                .is_some_and(|(_, p)| p.uci == "e7e5")
        );
    }

    #[test]
    fn model_identity_is_strict_and_personal_head_changes_invalidate_the_session() {
        assert!(recognized_version(MODEL_VERSION));
        let version = format!("{MODEL_VERSION}:personal:{}", "a".repeat(64));
        assert!(recognized_version(&version));
        for suffix in ["", "abc", &"A".repeat(64), &"g".repeat(64), &"a".repeat(65)] {
            assert!(!recognized_version(&format!(
                "{MODEL_VERSION}:personal:{suffix}"
            )));
        }
        let id = SessionId::new();
        let game = ChessGame::standard();
        let mut first = observation(&game, id, 0, false);
        first.model_version = version;
        let mut session = VisionSession::begin(&game, first).unwrap();
        assert!(!session.allows_automatic());
        assert!(
            session
                .observe(&game, observation(&game, id, 1, false))
                .is_err()
        );
    }

    #[test]
    fn rejects_wrong_setup_low_quality_and_malformed_probabilities() {
        let id = SessionId::new();
        let game = ChessGame::standard();
        let mut wrong = game.clone();
        wrong.play_uci("e2e4").unwrap();
        assert!(VisionSession::begin(&game, observation(&wrong, id, 0, false)).is_err());
        let mut unclear = observation(&game, id, 0, false);
        unclear.squares[0].visible_probability = 0.0;
        assert!(VisionSession::begin(&game, unclear).is_err());
        let mut invalid = observation(&game, id, 0, false);
        invalid.squares[0].empty_probability = 1.0;
        assert!(VisionSession::begin(&game, invalid).is_err());
    }

    #[test]
    fn motion_or_contradictory_evidence_withdraws_an_uncommitted_proposal() {
        let id = SessionId::new();
        let game = ChessGame::standard();
        for moving in [false, true] {
            let mut session =
                VisionSession::begin(&game, observation(&game, id, 0, false)).unwrap();
            let mut after = game.clone();
            after.play_uci("e2e4").unwrap();
            for sequence in [1, 2, 3] {
                session
                    .observe(&game, observation(&after, id, sequence, false))
                    .unwrap();
            }
            let token = session.proposal.as_ref().unwrap().0.clone();
            let position = if moving { &after } else { &game };
            let decision = session
                .observe(&game, observation(position, id, 4, moving))
                .unwrap();
            assert_ne!(decision["kind"], "candidate");
            assert!(!session.approves("e2e4", Some(id), Some(&token)));
        }
    }

    #[test]
    fn hidden_replies_and_capture_session_changes_never_become_moves() {
        let id = SessionId::new();
        let game = ChessGame::standard();
        let mut session = VisionSession::begin(&game, observation(&game, id, 0, false)).unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        after.play_uci("e7e5").unwrap();
        for sequence in [1, 2, 3] {
            assert_eq!(
                session
                    .observe(&game, observation(&after, id, sequence, false))
                    .unwrap()["kind"],
                "review"
            );
        }
        assert!(
            session
                .observe(&game, observation(&game, SessionId::new(), 4, false))
                .is_err()
        );
    }
}
