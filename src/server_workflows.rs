use super::*;
use contracts::TimeBounds;
use vision_inference::preprocess::Corners;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeStart {
    corners: Corners,
    threshold: f64,
}
#[derive(Deserialize)]
struct MetadataRequest {
    #[serde(flatten)]
    position: PositionRequest,
    white: String,
    black: String,
    result: String,
    status: String,
}
#[derive(Deserialize)]
struct CorrectionRequest {
    #[serde(flatten)]
    position: PositionRequest,
    replace_from_ply: u32,
    uci: String,
    reason: String,
}
#[derive(Deserialize)]
struct RestoreRequest {
    backup_id: String,
}
#[derive(Deserialize)]
struct OpenGame {
    game_id: contracts::GameId,
}
impl App {
    fn invalidate(&mut self, reason: &str) -> Result<()> {
        self.vision = None;
        self.previous_completion = None;
        self.last_trusted_reading = None;
        self.native_decision = json!({"kind":"review","reason":reason});
        self.store.activity(
            self.game_id,
            "observation_gap",
            &json!({"reason":reason}),
            utc_us(),
        )?;
        Ok(())
    }
    pub(super) fn process_native(&mut self) -> Result<()> {
        let Some(camera) = &self.native else {
            return Ok(());
        };
        let health = camera.health();
        let (readings, overflow) = camera.drain();
        if health.state == "interrupted" || overflow {
            if self.vision.is_none() && self.native_decision["kind"] == "review" {
                return Ok(());
            }
            self.invalidate(
                health
                    .error
                    .as_deref()
                    .unwrap_or("Observation backlog exceeded its bound; verify a fresh reference"),
            )?;
            return Ok(());
        }
        for reading in readings {
            if reading.acquired.elapsed() > Duration::from_secs(2) {
                self.invalidate(
                    "Camera evidence is stale. Restart capture and verify the physical position.",
                )?;
                continue;
            }
            let source = reading.source_sequence;
            self.last_reading = Some(reading.clone());
            if self
                .vision
                .as_ref()
                .is_some_and(|s| Some(s.id) == self.native.as_ref().map(|c| c.id))
                && source > self.native_last_source
            {
                self.native_sequence += 1;
                let mut observation = reading.observation.clone();
                observation.sequence = self.native_sequence;
                let game = self.trusted_position(&PositionRequest {
                    game_id: self.game_id,
                    revision: self.store.metadata(self.game_id)?.active_revision,
                    expected_ply: self.store.replay_active_game(self.game_id)?.moves.len(),
                })?;
                let mut session = self.vision.take().unwrap();
                let old_decision = self.native_decision.clone();
                self.native_decision = session.observe(&game, observation.clone())?;
                self.vision = Some(session);
                if self.native_decision["kind"] == "unchanged" {
                    self.last_trusted_reading = Some(reading.clone());
                }
                let changed_proposal = self.native_decision["kind"] == "candidate"
                    && (self.native_decision["proposal_id"] != old_decision["proposal_id"]
                        || old_decision["kind"] != "candidate");
                let entered_review =
                    self.native_decision["kind"] == "review" && old_decision["kind"] != "review";
                if changed_proposal || entered_review {
                    self.store.activity(self.game_id,"native_observation",&json!({"observation":observation,"source_sequence":source,"camera_dropped":reading.dropped,"inference_ms":reading.latency_ms}),utc_us())?;
                }
                if ["candidate", "review"]
                    .contains(&self.native_decision["kind"].as_str().unwrap_or(""))
                    && reading.observation.capture_time.get() - self.last_evidence_at >= 2_000_000
                {
                    self.save_current_evidence("pending")?;
                    self.last_evidence_at = reading.observation.capture_time.get();
                }
            }
            self.native_last_source = source;
        }
        if self.vision.as_ref().is_some_and(|s| !s.is_fresh()) {
            self.invalidate("Observation stream interrupted. Verify a fresh reference.")?;
        }
        Ok(())
    }
    pub(super) fn save_current_evidence(&mut self, kind: &str) -> Result<Option<String>> {
        match (&mut self.evidence, &self.last_reading) {
            (Some(store), Some(reading)) => store.save(&self.store, self.game_id, reading, kind),
            _ => Ok(None),
        }
    }
    fn begin_native(&mut self, position: PositionRequest) -> Result<Value> {
        ensure!(
            self.store.metadata(self.game_id)?.status == "recording",
            "Resume recording before setting a camera reference."
        );
        let game = self.trusted_position(&position)?;
        let reading = self
            .last_reading
            .as_ref()
            .context("Wait for the native camera to read the board")?;
        ensure!(
            reading.acquired.elapsed() < Duration::from_secs(2),
            "Camera evidence is stale"
        );
        ensure!(
            self.native
                .as_ref()
                .is_some_and(|c| c.health().state == "live"),
            "Camera is not live"
        );
        let mut observation = reading.observation.clone();
        observation.sequence = 0;
        self.vision = Some(VisionSession::begin(&game, observation.clone())?);
        self.previous_completion = None;
        self.native_sequence = 0;
        self.native_last_source = reading.source_sequence;
        self.last_trusted_reading = Some(reading.clone());
        self.store.activity(self.game_id,"capture_session",&json!({"session_id":observation.session_id,"timestamp_source":"avfoundation_presentation_time","utc_anchor_us":utc_us()-observation.capture_time.get(),"calibration_version":observation.calibration_version,"model_version":observation.model_version}),utc_us())?;
        self.save_current_evidence("reference")?;
        self.native_decision = json!({"kind":"unchanged"});
        self.snapshot()
    }
    fn metadata(&mut self, request: MetadataRequest) -> Result<Value> {
        self.trusted_position(&request.position)?;
        self.invalidate("Recording state changed; verify a fresh reference")?;
        self.store.set_metadata(
            self.game_id,
            &request.white,
            &request.black,
            &request.result,
            &request.status,
            utc_us(),
        )?;
        self.snapshot()
    }
    fn correct(&mut self, request: CorrectionRequest) -> Result<Value> {
        self.trusted_position(&request.position)?;
        ensure!(
            !request.reason.trim().is_empty(),
            "Give a reason for the correction"
        );
        let replay = self.store.replay_active_game(self.game_id)?;
        ensure!(
            request.replace_from_ply >= 1 && request.replace_from_ply <= replay.moves.len() as u32,
            "Choose an existing ply"
        );
        let mut game = ChessGame::from_fen(&replay.initial_fen)?;
        for m in replay
            .moves
            .iter()
            .take(request.replace_from_ply as usize - 1)
        {
            game.play_uci(&m.uci)?;
        }
        let replacement = game.play_uci(&request.uci)?.clone();
        let mut suffix = vec![MoveRecord {
            game_id: self.game_id,
            revision: replay.revision + 1,
            ply: replacement.ply,
            uci: replacement.uci,
            san: replacement.san,
            fen_before: replacement.fen_before,
            fen_after: replacement.fen_after,
            provenance: MoveProvenance::Reviewed,
            confidence: None,
            timing: MoveTiming {
                completion: None,
                elapsed_since_previous: None,
                confirmation_time: CaptureTimeUs::new(0)?,
                quality: TimingQuality::Unknown,
            },
            evidence_ids: vec![],
        }];
        let mut stopped = None;
        for m in replay.moves.iter().skip(request.replace_from_ply as usize) {
            match game.play_uci(&m.uci) {
                Ok(a) => {
                    let mut record = m.clone();
                    record.revision = replay.revision + 1;
                    record.san = a.san.clone();
                    record.fen_before = a.fen_before.clone();
                    record.fen_after = a.fen_after.clone();
                    record.provenance = MoveProvenance::Reviewed;
                    record.timing.elapsed_since_previous = None;
                    suffix.push(record);
                }
                Err(_) => {
                    stopped = Some(m.ply);
                    break;
                }
            }
        }
        self.vision = None;
        self.previous_completion = None;
        self.store.apply_correction(&CorrectionRevision {
            game_id: self.game_id,
            parent_revision: replay.revision,
            idempotency_key: format!(
                "correction-{}-{}",
                replay.revision, request.replace_from_ply
            ),
            replace_from_ply: request.replace_from_ply,
            reason: request.reason,
            replacement_suffix: suffix,
            created_utc_us: utc_us(),
        })?;
        if let Some(ply) = stopped {
            self.store.activity(self.game_id,"invalid_continuation",&json!({"parent_revision":replay.revision,"first_invalid_ply":ply,"preserved_suffix":replay.moves.iter().skip(ply as usize-1).collect::<Vec<_>>()}),utc_us())?;
            let m = self.store.metadata(self.game_id)?;
            self.store.set_metadata(
                self.game_id,
                &m.white,
                &m.black,
                "*",
                "incomplete",
                utc_us(),
            )?;
        }
        let mut value = self.snapshot()?;
        value["first_invalid_ply"] = json!(stopped);
        Ok(value)
    }
    pub(super) fn extra_route(
        &mut self,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<Option<(String, Vec<u8>)>> {
        let value=match (method,path){
            ("GET","/api/game/pgn-annotated")=>return Ok(Some(("application/x-chess-pgn".into(),self.pgn_with_timing(true)?.into_bytes()))),
            ("GET","/api/games")=>Some(json!(self.store.games()?)),
            ("POST","/api/game/open")=>{let r:OpenGame=serde_json::from_slice(body)?;self.store.replay_journal(r.game_id)?;self.invalidate("Game switched")?;self.native=None;self.last_reading=None;self.game_id=r.game_id;Some(self.snapshot()?)},
            ("POST","/api/game/metadata")=>Some(self.metadata(serde_json::from_slice(body)?)?),
            ("POST","/api/game/correct")=>Some(self.correct(serde_json::from_slice(body)?)?),
            ("GET","/api/game/audit")=>Some(json!({"revisions":self.store.revision_history(self.game_id)?,"activities":self.store.activities(self.game_id)?,"original_decisions":self.store.original_decisions(self.game_id)?})),
            ("GET","/api/game/evidence")=>Some(json!(self.store.evidence(self.game_id)?.iter().map(|record|json!({"record":record,"available":self.evidence.as_ref().is_some_and(|e|e.read(record).is_ok())})).collect::<Vec<_>>())),
            ("POST","/api/native/start")=>{let r:NativeStart=serde_json::from_slice(body)?;self.invalidate("Native capture started")?;self.native=None;self.last_reading=None;self.native=Some(NativeCamera::start(&self.config.camera_helper,&self.config.assets,r.corners,r.threshold)?);Some(self.snapshot()?)},
            ("POST","/api/native/disarm")=>{self.invalidate("Reference invalidated by recorder controls")?;Some(self.snapshot()?)},
            ("POST","/api/native/stop")=>{self.invalidate("Capture paused or stopped")?;self.native=None;self.last_reading=None;Some(self.snapshot()?)},
            ("POST","/api/native/reference")=>Some(self.begin_native(serde_json::from_slice(body)?)?),
            ("GET","/api/native/status")=>Some(json!({"health":self.native.as_ref().map(NativeCamera::health),"decision":self.native_decision,"reading":self.last_reading.as_ref().map(|r|json!({"squares":r.observation.squares,"latency_ms":r.latency_ms,"version":r.observation.model_version,"capture_time_us":r.observation.capture_time,"moving":r.observation.moving})),"game":self.snapshot()?})),
            ("GET","/api/native/preview.jpg")=>{let bytes=self.native.as_ref().context("Native camera is stopped")?.preview();ensure!(!bytes.is_empty(),"Preview not ready");return Ok(Some(("image/jpeg".into(),bytes)));},
            ("GET","/api/backups")=>{let root=self.base_data.join("backups");let mut entries=vec![];if root.is_dir(){for e in std::fs::read_dir(root)?{let e=e?;let name=e.file_name().to_string_lossy().to_string();if name.strip_prefix("backup-").is_some_and(|id|uuid::Uuid::parse_str(id).is_ok())&&e.path().join("backup.json").is_file(){entries.push(name);}}}entries.sort();Some(json!(entries))},
            ("POST","/api/restore")=>{let r:RestoreRequest=serde_json::from_slice(body)?;let id=r.backup_id.strip_prefix("backup-").context("Invalid backup name")?;uuid::Uuid::parse_str(id)?;ensure!(!r.backup_id.contains('/'),"Invalid backup name");self.invalidate("Backup restore switched the data profile")?;self.native=None;let profile=uuid::Uuid::now_v7();let destination=self.base_data.join("profiles").join(profile.to_string());crate::backup::restore(&self.base_data.join("backups").join(r.backup_id),&destination)?;let store=Store::open(destination.join("games.sqlite"))?;let game_id=store.latest_game_id()?.context("Backup contains no games")?;let evidence=EvidenceStore::open(&destination)?;let temp=self.base_data.join("active-profile.partial");std::fs::write(&temp,profile.to_string())?;std::fs::rename(temp,self.base_data.join("active-profile"))?;self.store=store;self.game_id=game_id;self.evidence=Some(evidence);self.config.data=destination;self.last_reading=None;Some(self.snapshot()?)},
            ("POST","/api/backup")=>{self.invalidate("Backup snapshot")?;let folder=self.base_data.join("backups").join(format!("backup-{}",uuid::Uuid::now_v7()));crate::backup::create(&self.store,&self.config.data,&folder)?;Some(json!({"path":folder}))},
            ("GET","/api/game/timing.json")=>{let replay=self.store.replay_active_game(self.game_id)?;Some(json!({"game":self.store.metadata(self.game_id)?,"revision":replay.revision,"moves":replay.moves,"evidence":self.store.evidence(self.game_id)?,"gaps":self.store.activities(self.game_id)?.into_iter().filter(|v|v["kind"]=="observation_gap" || v["kind"]=="evidence_loss").collect::<Vec<_>>()}))},
            ("GET","/api/game/timing.csv")=>{let replay=self.store.replay_active_game(self.game_id)?;let mut csv="ply,uci,san,provenance,completion_earliest_us,completion_latest_us,elapsed_earliest_us,elapsed_latest_us,confirmation_time_us,timing_quality,evidence_ids\n".to_owned();for m in replay.moves{let bound=|v:Option<TimeBounds>|v.map(|t|format!("{},{}",t.earliest.get(),t.latest.get())).unwrap_or_else(||",".into());csv.push_str(&format!("{},\"{}\",\"{}\",{},{},{},{},{},\"{}\"\n",m.ply,m.uci,m.san,serde_json::to_value(m.provenance)?.as_str().unwrap(),bound(m.timing.completion),bound(m.timing.elapsed_since_previous),m.timing.confirmation_time.get(),serde_json::to_value(m.timing.quality)?.as_str().unwrap(),m.evidence_ids.join(";")));}return Ok(Some(("text/csv".into(),csv.into_bytes())));},
            _=>None
        };
        if let Some(id) = path
            .strip_prefix("/api/evidence/")
            .and_then(|p| p.strip_suffix(".jpg"))
            && method == "GET"
        {
            let record = self
                .store
                .evidence(self.game_id)?
                .into_iter()
                .find(|e| e.id == id)
                .context("Evidence not found for this game")?;
            return Ok(Some((
                "image/jpeg".into(),
                self.evidence
                    .as_ref()
                    .context("Evidence store unavailable")?
                    .read(&record)?,
            )));
        }
        Ok(value.map(|v| ("application/json".into(), serde_json::to_vec(&v).unwrap())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn position(app: &App) -> PositionRequest {
        let replay = app.store.replay_active_game(app.game_id).unwrap();
        PositionRequest {
            game_id: app.game_id,
            revision: replay.revision,
            expected_ply: replay.moves.len(),
        }
    }
    fn play(app: &mut App, uci: &str) {
        let p = position(app);
        app.apply_move(MoveRequest {
            game_id: p.game_id,
            revision: p.revision,
            expected_ply: p.expected_ply,
            uci: uci.into(),
            automatic: false,
            vision_session: None,
            proposal_id: None,
        })
        .unwrap();
    }
    #[test]
    fn correction_stops_on_invalid_suffix_and_preserves_original_decisions() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6"] {
            play(&mut app, uci);
        }
        let result = app
            .correct(CorrectionRequest {
                position: position(&app),
                replace_from_ply: 1,
                uci: "g1f3".into(),
                reason: "Camera reference shows Nf3".into(),
            })
            .unwrap();
        assert_eq!(result["first_invalid_ply"], 3);
        let replay = app.store.replay_journal(app.game_id).unwrap();
        assert_eq!(
            replay
                .moves
                .iter()
                .map(|m| m.uci.as_str())
                .collect::<Vec<_>>(),
            vec!["g1f3", "e7e5"]
        );
        assert_eq!(app.store.load_moves(app.game_id, 0).unwrap().len(), 4);
        assert_eq!(app.store.original_decisions(app.game_id).unwrap().len(), 4);
        assert_eq!(
            app.store.metadata(app.game_id).unwrap().status,
            "incomplete"
        );
        assert!(
            app.store
                .activities(app.game_id)
                .unwrap()
                .iter()
                .any(|v| v["kind"] == "invalid_continuation")
        );
        assert!(app.pgn().unwrap().contains("Incomplete recording"));
    }
    #[test]
    fn finished_game_requires_explicit_resume_and_exports_players_and_result() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        play(&mut app, "e2e4");
        app.metadata(MetadataRequest {
            position: position(&app),
            white: "Alice".into(),
            black: "Bob".into(),
            result: "1-0".into(),
            status: "finished".into(),
        })
        .unwrap();
        let p = position(&app);
        assert!(
            app.apply_move(MoveRequest {
                game_id: p.game_id,
                revision: p.revision,
                expected_ply: p.expected_ply,
                uci: "e7e5".into(),
                automatic: false,
                vision_session: None,
                proposal_id: None
            })
            .is_err()
        );
        assert!(app.pgn().unwrap().contains("[White \"Alice\"]"));
        assert!(app.pgn().unwrap().ends_with("1-0\n"));
    }
    #[test]
    fn timing_sidecars_preserve_unknown_and_evidence_paths_are_scoped() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        play(&mut app, "e2e4");
        let (_, bytes) = app
            .extra_route("GET", "/api/game/timing.json", b"")
            .unwrap()
            .unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["moves"][0]["timing"]["quality"], "unknown");
        assert!(
            app.extra_route("GET", "/api/evidence/../../room.jpg", b"")
                .is_err()
        );
        let (_, bytes) = app
            .extra_route("GET", "/api/game/timing.csv", b"")
            .unwrap()
            .unwrap();
        let csv = String::from_utf8(bytes).unwrap();
        assert!(csv.contains("unknown"));
        assert!(csv.contains("completion_earliest_us"));
    }
}

#[cfg(test)]
mod native_commit_tests {
    use super::*;
    use crate::vision::{MODEL_VERSION, VisionSession};
    fn observation(game: &ChessGame, id: SessionId, seq: u64) -> FrameObservation {
        FrameObservation {
            session_id: id,
            sequence: seq,
            capture_time: CaptureTimeUs::new(seq as i64 * 500_000).unwrap(),
            moving: false,
            calibration_version: "native-test".into(),
            model_version: MODEL_VERSION.into(),
            squares: game
                .piece_classes()
                .into_iter()
                .map(|(square, piece)| {
                    let mut p = [0.00001; 12];
                    let empty = if let Some(piece) = piece {
                        p[piece.index()] = 0.99988;
                        0.00001
                    } else {
                        0.99988
                    };
                    contracts::SquareEvidence {
                        square,
                        visible_probability: 1.0,
                        empty_probability: empty,
                        piece_probabilities: p,
                    }
                })
                .collect(),
        }
    }
    fn reading(observation: FrameObservation) -> Reading {
        Reading {
            source_sequence: observation.sequence,
            observation,
            jpeg: crate::native::jpeg(
                &vision_inference::preprocess::RgbaFrame::new(2, 2, vec![255; 16]).unwrap(),
            )
            .unwrap(),
            latency_ms: 900,
            dropped: 0,
            acquired: std::time::Instant::now(),
        }
    }
    #[test]
    fn native_commits_acquisition_bounds_and_both_evidence_frames_then_restarts() {
        let root =
            std::env::temp_dir().join(format!("chess-native-commit-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&root).unwrap();
        let mut app = App::open(Store::open(root.join("games.sqlite")).unwrap()).unwrap();
        app.evidence = Some(EvidenceStore::open(&root).unwrap());
        let id = SessionId::new();
        let game = ChessGame::standard();
        let before = observation(&game, id, 0);
        app.native = Some(NativeCamera::fixture(id));
        app.last_trusted_reading = Some(reading(before.clone()));
        let mut session = VisionSession::begin(&game, before).unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        let mut proposal = Value::Null;
        for seq in 1..=3 {
            let obs = observation(&after, id, seq);
            proposal = session.observe(&game, obs.clone()).unwrap();
            app.last_reading = Some(reading(obs));
        }
        app.vision = Some(session);
        let replay = app.store.replay_active_game(app.game_id).unwrap();
        app.apply_move(MoveRequest {
            game_id: app.game_id,
            revision: replay.revision,
            expected_ply: 0,
            uci: "e2e4".into(),
            automatic: true,
            vision_session: Some(id),
            proposal_id: proposal["proposal_id"].as_str().map(str::to_owned),
        })
        .unwrap();
        let saved = app
            .store
            .replay_journal(app.game_id)
            .unwrap()
            .moves
            .remove(0);
        assert_eq!(saved.timing.quality, TimingQuality::Bounded);
        assert_eq!(saved.timing.completion.unwrap().latest.get(), 500_000);
        assert_eq!(saved.timing.confirmation_time.get(), 1_500_000);
        assert!(saved.timing.elapsed_since_previous.is_none());
        assert_eq!(saved.evidence_ids.len(), 2);
        for record in app.store.evidence(app.game_id).unwrap() {
            assert!(app.evidence.as_ref().unwrap().read(&record).is_ok());
        }
        let id = app.game_id;
        drop(app);
        let reopened = App::open(Store::open(root.join("games.sqlite")).unwrap()).unwrap();
        assert_eq!(reopened.store.original_decisions(id).unwrap()[0], saved);
        assert!(reopened.previous_completion.is_none());
        drop(reopened);
        std::fs::remove_dir_all(root).unwrap();
    }
}
