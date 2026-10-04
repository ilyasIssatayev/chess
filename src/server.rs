//! Loopback-only recorder service for the browser and Tauri shell. Native capture,
//! recognition, rules, evidence and persistence run locally in Rust.
use crate::{
    evidence::EvidenceStore,
    native::{NativeCamera, Reading},
};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
#[path = "server_workflows.rs"]
mod workflows;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::vision::{MANIFEST, VisionSession};
use anyhow::{Context, Result, bail, ensure};
use chess_core::ChessGame;
use contracts::{
    CaptureTimeUs, FrameObservation, GameId, MoveProvenance, MoveRecord, MoveTiming, SessionId,
    TimingQuality,
};
use export::{PgnMetadata, to_pgn};
use serde::Deserialize;
use serde_json::{Value, json};
use storage::{CorrectionRevision, Store};

#[derive(Clone)]
pub struct ServerConfig {
    pub ui: PathBuf,
    pub assets: PathBuf,
    pub data: PathBuf,
    pub camera_helper: PathBuf,
}
impl ServerConfig {
    pub fn development() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        Self {
            ui: root.join("apps/recorder-ui-prototype"),
            assets: root.join("local-data/vision"),
            data: std::env::var_os("CHESS_RECORDER_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join("local-data/recorder")),
            camera_helper: root.join("target/native/chess-camera"),
        }
    }
}
pub struct ServerHandle {
    pub port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub fn spawn(mut config: ServerConfig, port: u16) -> Result<ServerHandle> {
    std::fs::create_dir_all(&config.data)?;
    let base_data = config.data.clone();
    if let Ok(profile) = std::fs::read_to_string(base_data.join("active-profile")) {
        let id = uuid::Uuid::parse_str(profile.trim()).context("Invalid active profile")?;
        config.data = base_data.join("profiles").join(id.to_string());
        ensure!(
            config.data.join("games.sqlite").is_file(),
            "Active restored profile is missing"
        );
    }
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let mut app = App::open(Store::open(config.data.join("games.sqlite"))?)?;
    app.evidence = Some(EvidenceStore::open(&config.data)?);
    app.base_data = base_data;
    app.config = config;
    let stop = Arc::new(AtomicBool::new(false));
    let quitting = stop.clone();
    // Parse sockets off the recorder thread. WebKit opens speculative connections;
    // an idle socket must never block acquisition, decoding, or other requests.
    let (sender, receiver) =
        std::sync::mpsc::sync_channel::<(Vec<u8>, std::sync::mpsc::SyncSender<Vec<u8>>)>(8);
    let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let thread = std::thread::spawn(move || {
        while !quitting.load(Ordering::Relaxed) {
            if let Err(e) = app.process_native() {
                app.vision = None;
                app.previous_completion = None;
                app.native_decision = json!({"kind":"review","reason":e.to_string()});
            }
            if let Ok((mut stream, _)) = listener.accept()
                && connections.load(Ordering::Relaxed) < 8
            {
                connections.fetch_add(1, Ordering::Relaxed);
                let active = connections.clone();
                let messages = sender.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                    if let Ok(bytes) = read_packet(&mut stream) {
                        let (reply, response) = std::sync::mpsc::sync_channel(1);
                        if messages.try_send((bytes, reply)).is_ok()
                            && let Ok(bytes) = response.recv_timeout(Duration::from_secs(10))
                        {
                            let _ = stream.write_all(&bytes);
                        }
                    }
                    active.fetch_sub(1, Ordering::Relaxed);
                });
            }
            if let Ok((bytes, reply)) = receiver.recv_timeout(Duration::from_millis(10)) {
                let start = bytes.len();
                let mut memory = std::io::Cursor::new(bytes);
                if let Err(e) = serve_request(&mut memory, &mut app, port) {
                    memory.set_position(start as u64);
                    let _ = respond(
                        &mut memory,
                        400,
                        "application/json",
                        &serde_json::to_vec(&json!({"error":e.to_string()})).unwrap_or_default(),
                    );
                }
                let _ = reply.send(memory.into_inner()[start..].to_vec());
            }
        }
    });
    Ok(ServerHandle {
        port,
        stop,
        thread: Some(thread),
    })
}
pub fn run() -> Result<()> {
    let port = std::env::args()
        .nth(2)
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(8770);
    let server = spawn(ServerConfig::development(), port)?;
    println!("Recorder ready: http://localhost:{}/", server.port);
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

struct App {
    store: Store,
    game_id: GameId,
    vision: Option<VisionSession>,
    config: ServerConfig,
    base_data: PathBuf,
    evidence: Option<EvidenceStore>,
    native: Option<NativeCamera>,
    last_reading: Option<Reading>,
    last_trusted_reading: Option<Reading>,
    native_decision: Value,
    native_sequence: u64,
    native_last_source: u64,
    previous_completion: Option<(SessionId, contracts::TimeBounds)>,
    last_evidence_at: i64,
}

#[derive(Deserialize)]
struct MoveRequest {
    game_id: GameId,
    revision: u32,
    expected_ply: usize,
    uci: String,
    #[serde(default)]
    automatic: bool,
    #[serde(default)]
    vision_session: Option<SessionId>,
    #[serde(default)]
    proposal_id: Option<String>,
}

#[derive(Deserialize)]
struct PositionRequest {
    game_id: GameId,
    revision: u32,
    expected_ply: usize,
}

#[derive(Deserialize)]
struct VisionRequest {
    #[serde(flatten)]
    position: PositionRequest,
    observation: FrameObservation,
}

impl App {
    fn open(mut store: Store) -> Result<Self> {
        let game_id = match store.latest_game_id()? {
            Some(id) => id,
            None => {
                let id = GameId::new();
                store.create_game(id, ChessGame::standard().initial_fen(), utc_us())?;
                id
            }
        };
        Ok(Self {
            store,
            game_id,
            vision: None,
            config: ServerConfig::development(),
            base_data: ServerConfig::development().data,
            evidence: None,
            native: None,
            last_reading: None,
            last_trusted_reading: None,
            native_decision: json!({"kind":"idle"}),
            native_sequence: 0,
            native_last_source: 0,
            previous_completion: None,
            last_evidence_at: -2_000_000,
        })
    }

    fn snapshot(&self) -> Result<Value> {
        let replay = self.store.replay_active_game(self.game_id)?;
        let mut game = ChessGame::from_fen(&replay.initial_fen)?;
        game.rebuild(
            &replay
                .moves
                .iter()
                .map(|m| m.uci.clone())
                .collect::<Vec<_>>(),
        )?;
        let before = game.piece_classes();
        let legal = game.legal_uci_moves().into_iter().map(|uci| {
            let mut successor = game.clone();
            let applied = successor.play_uci(&uci).expect("generated move is legal").clone();
            let changed: Vec<_> = before.iter().zip(successor.piece_classes()).filter_map(|((square, old), (_, new))| (old != &new).then_some(square.clone())).collect();
            json!({"uci": uci, "san": applied.san, "changed": changed, "from": &applied.uci[..2], "to": &applied.uci[2..4]})
        }).collect::<Vec<_>>();
        Ok(json!({
            "game_id": self.game_id, "revision": replay.revision,
            "vision_session": self.vision.as_ref().map(|session| session.id),
            "fen": game.fen(), "moves": replay.moves, "legal_moves": legal,
            "pieces": before, "turn": if game.fen().split_whitespace().nth(1) == Some("w") { "White" } else { "Black" },
            "status": format!("{:?}", game.status()),
            "metadata":self.store.metadata(self.game_id)?,
            "initial_fen":replay.initial_fen,
            "revisions":self.store.revision_history(self.game_id)?,
            "native": self.native.as_ref().map(NativeCamera::health),
            "native_decision": self.native_decision,
            "evidence_lost":self.evidence.as_ref().map_or(0,|e|e.lost),
        }))
    }

    fn trusted_position(&self, request: &PositionRequest) -> Result<ChessGame> {
        let replay = self.store.replay_active_game(self.game_id)?;
        ensure!(
            request.game_id == self.game_id
                && request.revision == replay.revision
                && request.expected_ply == replay.moves.len(),
            "The tracked position changed. Reload and set a fresh reference."
        );
        let mut game = ChessGame::from_fen(&replay.initial_fen)?;
        game.rebuild(
            &replay
                .moves
                .iter()
                .map(|m| m.uci.clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(game)
    }

    fn begin_vision(&mut self, request: VisionRequest) -> Result<Value> {
        ensure!(
            self.store.metadata(self.game_id)?.status == "recording",
            "Resume recording before setting a camera reference."
        );
        let game = self.trusted_position(&request.position)?;
        // Clear any previous proposal even when a new reference fails.
        self.vision = None;
        let session = VisionSession::begin(&game, request.observation)?;
        let id = session.id;
        self.vision = Some(session);
        Ok(json!({"kind": "reference", "session_id": id}))
    }

    fn observe_vision(&mut self, request: VisionRequest) -> Result<Value> {
        let game = self.trusted_position(&request.position)?;
        // A rejected observation may indicate a dropped frame, model change, or
        // malformed evidence. None of its earlier proposals may remain usable.
        let mut session = self
            .vision
            .take()
            .context("Set a model-verified reference first.")?;
        let result = session.observe(&game, request.observation);
        if result.is_ok() {
            self.vision = Some(session);
        }
        result
    }

    fn apply_move(&mut self, request: MoveRequest) -> Result<Value> {
        ensure!(
            request.game_id == self.game_id,
            "The active game changed. Refresh the page."
        );
        let replay = self.store.replay_active_game(self.game_id)?;
        ensure!(
            request.revision == replay.revision,
            "The position was corrected. Refresh the page."
        );
        // A retried request after a lost response must not apply a second move.
        if replay.moves.len() == request.expected_ply + 1
            && replay.moves.last().is_some_and(|m| m.uci == request.uci)
        {
            return self.snapshot();
        }
        ensure!(
            replay.moves.len() == request.expected_ply,
            "The position changed. Refresh the page before recording another move."
        );
        let mut game = ChessGame::from_fen(&replay.initial_fen)?;
        game.rebuild(
            &replay
                .moves
                .iter()
                .map(|m| m.uci.clone())
                .collect::<Vec<_>>(),
        )?;
        let model_approved = self.vision.as_ref().is_some_and(|session| {
            session.approves(
                &request.uci,
                request.vision_session,
                request.proposal_id.as_deref(),
            )
        });
        ensure!(
            !request.automatic || model_approved,
            "Automatic recording requires a current model-supported move. Set a fresh reference or review the move manually."
        );
        ensure!(
            !request.automatic
                || self
                    .vision
                    .as_ref()
                    .is_some_and(VisionSession::allows_automatic),
            "Personal recognition requires review. Confirm and record the suggested move manually."
        );
        ensure!(
            self.store.metadata(self.game_id)?.status != "finished",
            "Resume this game before adding moves."
        );
        let proposed = if model_approved {
            self.vision.as_ref().and_then(|s| s.proposed()).cloned()
        } else {
            None
        };
        let session_id = self.vision.as_ref().map(|s| s.id);
        let native_supported = model_approved && session_id == self.native.as_ref().map(|c| c.id);
        let timing = if native_supported {
            let p = proposed.as_ref().context("No current proposal")?;
            MoveTiming {
                completion: Some(p.completion_bounds),
                elapsed_since_previous: self
                    .previous_completion
                    .filter(|(id, _)| Some(*id) == session_id)
                    .map(|(_, previous)| p.completion_bounds.elapsed_since(previous))
                    .transpose()?,
                confirmation_time: p.confirmation_time,
                quality: TimingQuality::Bounded,
            }
        } else {
            MoveTiming {
                completion: None,
                elapsed_since_previous: None,
                confirmation_time: self
                    .vision
                    .as_ref()
                    .map(|s| s.last_observation().capture_time)
                    .unwrap_or(CaptureTimeUs::new(0)?),
                quality: TimingQuality::Unknown,
            }
        };
        let mut evidence_ids = vec![];
        if native_supported {
            if let (Some(evidence), Some(reading)) =
                (&mut self.evidence, &self.last_trusted_reading)
                && let Some(id) = evidence.save(&self.store, self.game_id, reading, "pre_move")?
            {
                evidence_ids.push(id);
            }
            evidence_ids.extend(self.save_current_evidence("accepted")?);
        }
        let applied = game.play_uci(&request.uci)?.clone();
        let next_vision = if model_approved {
            let mut session = self.vision.clone().unwrap();
            session.commit(&game)?;
            Some(session)
        } else {
            None
        };
        let record = MoveRecord {
            game_id: self.game_id,
            revision: replay.revision,
            ply: applied.ply,
            uci: applied.uci,
            san: applied.san,
            fen_before: applied.fen_before,
            fen_after: applied.fen_after,
            provenance: if request.automatic {
                MoveProvenance::Automatic
            } else {
                MoveProvenance::Reviewed
            },
            confidence: None,
            timing,
            evidence_ids,
        };
        self.store.append_move(
            &format!(
                "browser-{}-{}-{}",
                self.game_id.0, replay.revision, applied.ply
            ),
            &record,
            utc_us(),
        )?;
        if native_supported {
            self.previous_completion =
                Some((session_id.unwrap(), proposed.unwrap().completion_bounds));
        } else {
            self.previous_completion = None;
        }
        self.vision = next_vision;
        self.native_decision = json!({"kind":"accepted","uci":record.uci});
        self.snapshot()
    }

    fn undo(&mut self, request: PositionRequest) -> Result<Value> {
        let replay = self.store.replay_active_game(self.game_id)?;
        ensure!(
            request.game_id == self.game_id
                && request.revision == replay.revision
                && request.expected_ply == replay.moves.len(),
            "The position changed. Refresh the page."
        );
        ensure!(!replay.moves.is_empty(), "There is no move to undo.");
        self.vision = None;
        self.store.apply_correction(&CorrectionRevision {
            game_id: self.game_id,
            parent_revision: replay.revision,
            idempotency_key: format!("browser-undo-{}-{}", self.game_id.0, replay.revision),
            replace_from_ply: replay.moves.len() as u32,
            reason: "User undid the last move in the recorder".into(),
            replacement_suffix: vec![],
            created_utc_us: utc_us(),
        })?;
        self.snapshot()
    }

    fn new_game(&mut self) -> Result<Value> {
        let id = GameId::new();
        self.store
            .create_game(id, ChessGame::standard().initial_fen(), utc_us())?;
        self.game_id = id;
        self.vision = None;
        self.snapshot()
    }

    fn pgn(&self) -> Result<String> {
        self.pgn_with_timing(false)
    }
    fn pgn_with_timing(&self, include_timing: bool) -> Result<String> {
        let replay = self.store.replay_active_game(self.game_id)?;
        let m = self.store.metadata(self.game_id)?;
        let metadata = PgnMetadata {
            date: chrono::DateTime::from_timestamp_micros(m.created_utc_us)
                .map(|d| d.format("%Y.%m.%d").to_string())
                .unwrap_or_else(|| "????.??.??".into()),
            white: m.white,
            black: m.black,
            result: if m.status == "incomplete" {
                "*".into()
            } else {
                m.result
            },
            ..PgnMetadata::default()
        };
        let mut pgn = to_pgn(&metadata, &replay.moves, include_timing)?;
        if m.status == "incomplete" {
            pgn.push_str("{Incomplete recording: exported trusted prefix only.}\n");
        }
        Ok(pgn)
    }
}

pub(crate) fn utc_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_micros() as i64
}

fn read_packet(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut reader = BufReader::new(stream);
    let mut packet = Vec::new();
    let mut length = None;
    loop {
        let mut line = Vec::new();
        let count = reader.by_ref().take(8192).read_until(b'\n', &mut line)?;
        ensure!(
            count > 0 && line.ends_with(b"\n"),
            "Incomplete HTTP headers"
        );
        packet.extend(&line);
        ensure!(packet.len() <= 16384, "Headers too large");
        if line == b"\r\n" {
            break;
        }
        if let Some((name, value)) = std::str::from_utf8(&line)?.trim().split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            ensure!(length.is_none(), "Duplicate Content-Length");
            length = Some(value.trim().parse::<usize>()?);
        }
    }
    let length = length.unwrap_or(0);
    ensure!(length <= 65536, "Request too large");
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    packet.extend(body);
    Ok(packet)
}

fn serve_request<T: Read + Write>(stream: &mut T, app: &mut App, port: u16) -> Result<()> {
    let mut reader = BufReader::new(&mut *stream);
    let mut line = String::new();
    reader.by_ref().take(8192).read_line(&mut line)?;
    let parts = line.split_whitespace().collect::<Vec<_>>();
    ensure!(parts.len() == 3, "Invalid HTTP request");
    let method = parts[0].to_owned();
    let path = parts[1].split('?').next().unwrap().to_owned();
    let mut length = 0;
    let mut content_type = String::new();
    let mut origin = None;
    let mut host = String::new();
    let mut header_bytes = line.len();
    loop {
        line.clear();
        let count = reader.by_ref().take(8192).read_line(&mut line)?;
        ensure!(count > 0, "Incomplete headers");
        header_bytes += count;
        ensure!(header_bytes <= 16384, "Headers too large");
        if line == "\r\n" {
            break;
        }
        let (key, value) = line.trim().split_once(':').context("Invalid header")?;
        match key.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse::<usize>()?,
            "content-type" => content_type = value.trim().to_owned(),
            "origin" => origin = Some(value.trim().to_owned()),
            "host" => host = value.trim().to_owned(),
            "transfer-encoding" => bail!("Chunked requests are unsupported"),
            _ => {}
        }
    }
    ensure!(length <= 65536, "Request too large");
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    drop(reader);
    let hosts = [format!("localhost:{port}"), format!("127.0.0.1:{port}")];
    ensure!(
        hosts.contains(&host),
        "Only the local recorder host is allowed"
    );
    if method == "POST" {
        ensure!(
            content_type == "application/json",
            "Expected application/json"
        );
        if let Some(origin) = origin {
            ensure!(
                hosts.iter().any(|host| origin == format!("http://{host}")),
                "Cross-origin writes are not allowed"
            );
        }
    }
    if let Some((mime, bytes)) = app.extra_route(&method, &path, &body)? {
        return respond(stream, 200, &mime, &bytes);
    }
    let value = match (method.as_str(), path.as_str()) {
        ("GET", "/api/game") => Some(app.snapshot()?),
        ("POST", "/api/vision/reference") => {
            Some(app.begin_vision(serde_json::from_slice(&body)?)?)
        }
        ("POST", "/api/vision/observe") => {
            Some(app.observe_vision(serde_json::from_slice(&body)?)?)
        }
        ("POST", "/api/game/move") => Some(app.apply_move(serde_json::from_slice(&body)?)?),
        ("POST", "/api/game/undo") => Some(app.undo(serde_json::from_slice(&body)?)?),
        ("POST", "/api/game/new") => Some(app.new_game()?),
        ("GET", "/api/game/pgn") => {
            return respond(
                stream,
                200,
                "application/x-chess-pgn",
                app.pgn()?.as_bytes(),
            );
        }
        _ => None,
    };
    if let Some(value) = value {
        return respond(
            stream,
            200,
            "application/json",
            &serde_json::to_vec(&value)?,
        );
    }
    if method == "GET" {
        if std::env::var_os("CHESS_RECORDER_TEST_MODE").is_some() {
            if let Some(file) = path.strip_prefix("/evaluation-fixture/") {
                let png = file
                    .strip_prefix("frames/")
                    .and_then(|name| name.strip_suffix(".png"));
                let allowed = file == "bundle.json"
                    || png.is_some_and(|name| {
                        name.len() == 6 && name.bytes().all(|b| b.is_ascii_digit())
                    });
                if allowed {
                    let bytes = std::fs::read(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("local-data/offline-evaluation-check/prepared")
                            .join(file),
                    )?;
                    return respond(
                        stream,
                        200,
                        if file == "bundle.json" {
                            "application/json"
                        } else {
                            "image/png"
                        },
                        &bytes,
                    );
                }
            }
            if path == "/vision-smoke.html" {
                return respond(
                    stream,
                    200,
                    "text/html",
                    &std::fs::read(app.config.ui.join("tests/vision-smoke.html"))?,
                );
            }
            if path == "/vision-fixture.jpg" {
                return respond(
                    stream,
                    200,
                    "image/jpeg",
                    &std::fs::read(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("local-data/vision/tests/fixture.jpg"),
                    )?,
                );
            }
        }
        if path == "/vision-assets/manifest.json" {
            return respond(stream, 200, "application/json", MANIFEST.as_bytes());
        }
        let model_asset = match path.as_str() {
            "/vision-assets/occupancy.onnx" => {
                Some(("models/occupancy.onnx", "application/octet-stream"))
            }
            "/vision-assets/pieces.onnx" => {
                Some(("models/pieces.onnx", "application/octet-stream"))
            }
            "/vision-assets/ort.wasm.min.mjs" => {
                Some(("runtime/ort.wasm.min.mjs", "text/javascript"))
            }
            "/vision-assets/ort-wasm-simd-threaded.mjs" => {
                Some(("runtime/ort-wasm-simd-threaded.mjs", "text/javascript"))
            }
            "/vision-assets/ort-wasm-simd-threaded.wasm" => {
                Some(("runtime/ort-wasm-simd-threaded.wasm", "application/wasm"))
            }
            _ => None,
        };
        if let Some((file, mime)) = model_asset {
            let asset = app.config.assets.join(file);
            return match std::fs::read(asset) {
                Ok(bytes) => respond(stream, 200, mime, &bytes),
                Err(_) => respond(
                    stream,
                    404,
                    "application/json",
                    br#"{"error":"Vision asset missing. Run python3 scripts/setup-vision.py"}"#,
                ),
            };
        }
    }
    let asset = match path.as_str() {
        "/library.html" => Some(("library.html", "text/html")),
        "/library.js" => Some(("library.js", "text/javascript")),
        "/library-core.js" => Some(("library-core.js", "text/javascript")),
        "/evaluation.html" => Some(("evaluation.html", "text/html")),
        "/evaluation.js" => Some(("evaluation.js", "text/javascript")),
        "/evaluation-core.js" => Some(("evaluation-core.js", "text/javascript")),
        "/evaluation.css" => Some(("evaluation.css", "text/css")),
        "/" | "/apps/recorder-ui-prototype/" | "/apps/recorder-ui-prototype/index.html" => {
            Some(("index.html", "text/html"))
        }
        "/styles.css" | "/apps/recorder-ui-prototype/styles.css" => {
            Some(("styles.css", "text/css"))
        }
        "/app.js" | "/apps/recorder-ui-prototype/app.js" => Some(("app.js", "text/javascript")),
        "/vision-core.js" | "/apps/recorder-ui-prototype/vision-core.js" => {
            Some(("vision-core.js", "text/javascript"))
        }
        "/vision-client.js" | "/apps/recorder-ui-prototype/vision-client.js" => {
            Some(("vision-client.js", "text/javascript"))
        }
        "/vision-worker.js" => Some(("vision-worker.js", "text/javascript")),
        "/vision-personal.js" | "/apps/recorder-ui-prototype/vision-personal.js" => {
            Some(("vision-personal.js", "text/javascript"))
        }
        "/vision-store.js" | "/apps/recorder-ui-prototype/vision-store.js" => {
            Some(("vision-store.js", "text/javascript"))
        }
        "/recorder.js" | "/apps/recorder-ui-prototype/recorder.js" => {
            Some(("recorder.js", "text/javascript"))
        }
        _ => None,
    };
    if method == "GET"
        && let Some((file, mime)) = asset
    {
        return respond(stream, 200, mime, &std::fs::read(app.config.ui.join(file))?);
    }
    respond(stream, 404, "application/json", br#"{"error":"Not found"}"#)
}

fn respond(stream: &mut impl Write, status: u16, mime: &str, body: &[u8]) -> Result<()> {
    let label = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {label}\r\nContent-Type: {mime}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(app: &App, uci: &str) -> MoveRequest {
        let state = app.snapshot().unwrap();
        MoveRequest {
            game_id: app.game_id,
            revision: state["revision"].as_u64().unwrap() as u32,
            expected_ply: state["moves"].as_array().unwrap().len(),
            uci: uci.into(),
            automatic: false,
            vision_session: None,
            proposal_id: None,
        }
    }

    fn neural_observation(
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
            calibration_version: "api-test-corners".into(),
            model_version: crate::vision::MODEL_VERSION.into(),
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
                    contracts::SquareEvidence {
                        square: square.clone(),
                        visible_probability: 1.0,
                        empty_probability: empty,
                        piece_probabilities: probabilities,
                    }
                })
                .collect(),
        }
    }

    fn vision_request(app: &App, observation: FrameObservation) -> VisionRequest {
        let state = app.snapshot().unwrap();
        VisionRequest {
            position: PositionRequest {
                game_id: app.game_id,
                revision: state["revision"].as_u64().unwrap() as u32,
                expected_ply: state["moves"].as_array().unwrap().len(),
            },
            observation,
        }
    }

    #[test]
    fn personal_neural_proposals_require_review_even_with_a_valid_token() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        let id = SessionId::new();
        let version = format!(
            "{}:personal:{}",
            crate::vision::MODEL_VERSION,
            "a".repeat(64)
        );
        let game = ChessGame::standard();
        let mut reference = neural_observation(&game, id, 0, false);
        reference.model_version = version.clone();
        app.begin_vision(vision_request(&app, reference)).unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        let mut decision = Value::Null;
        for sequence in [1, 2, 3] {
            let mut frame = neural_observation(&after, id, sequence, false);
            frame.model_version = version.clone();
            decision = app.observe_vision(vision_request(&app, frame)).unwrap();
        }
        assert_eq!(decision["kind"], "candidate");
        let mut automatic = request(&app, "e2e4");
        automatic.automatic = true;
        automatic.vision_session = Some(id);
        automatic.proposal_id = decision["proposal_id"].as_str().map(str::to_owned);
        assert!(
            app.apply_move(automatic)
                .unwrap_err()
                .to_string()
                .contains("requires review")
        );
        assert!(
            app.snapshot().unwrap()["moves"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let mut reviewed = request(&app, "e2e4");
        reviewed.vision_session = Some(id);
        reviewed.proposal_id = decision["proposal_id"].as_str().map(str::to_owned);
        let state = app.apply_move(reviewed).unwrap();
        assert_eq!(state["moves"][0]["provenance"], "reviewed");
        assert!(app.vision.is_some());
    }

    #[test]
    fn automatic_api_moves_require_a_live_neural_proposal_and_save_once() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        let mut unverified = request(&app, "e2e4");
        unverified.automatic = true;
        assert!(app.apply_move(unverified).is_err());
        let id = SessionId::new();
        let game = ChessGame::standard();
        app.begin_vision(vision_request(
            &app,
            neural_observation(&game, id, 0, false),
        ))
        .unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        let mut decision = Value::Null;
        for sequence in [1, 2, 3] {
            decision = app
                .observe_vision(vision_request(
                    &app,
                    neural_observation(&after, id, sequence, false),
                ))
                .unwrap();
        }
        assert_eq!(decision["kind"], "candidate");
        let mut accepted = request(&app, "e2e4");
        accepted.automatic = true;
        accepted.vision_session = Some(id);
        accepted.proposal_id = decision["proposal_id"].as_str().map(str::to_owned);
        let state = app.apply_move(accepted).unwrap();
        assert_eq!(state["moves"][0]["provenance"], "automatic");
        assert_eq!(state["turn"], "Black");
        assert!(app.vision.is_some());
        let unchanged = app
            .observe_vision(vision_request(
                &app,
                neural_observation(&after, id, 4, false),
            ))
            .unwrap();
        assert_eq!(unchanged["kind"], "unchanged");
        assert_eq!(
            app.snapshot().unwrap()["moves"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn stale_neural_proposals_cannot_commit_after_motion_or_manual_edits() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        let id = SessionId::new();
        let game = ChessGame::standard();
        app.begin_vision(vision_request(
            &app,
            neural_observation(&game, id, 0, false),
        ))
        .unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        let mut decision = Value::Null;
        for sequence in [1, 2, 3] {
            decision = app
                .observe_vision(vision_request(
                    &app,
                    neural_observation(&after, id, sequence, false),
                ))
                .unwrap();
        }
        let mut stale = request(&app, "e2e4");
        stale.automatic = true;
        stale.vision_session = Some(id);
        stale.proposal_id = decision["proposal_id"].as_str().map(str::to_owned);
        app.observe_vision(vision_request(
            &app,
            neural_observation(&after, id, 4, true),
        ))
        .unwrap();
        assert!(app.apply_move(stale).is_err());
        app.apply_move(request(&app, "d2d4")).unwrap();
        assert!(app.vision.is_none());
        app.undo(PositionRequest {
            game_id: app.game_id,
            revision: 0,
            expected_ply: 1,
        })
        .unwrap();
        assert!(app.vision.is_none());
    }

    #[test]
    fn rejected_observation_discards_the_session_and_its_proposal() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        let id = SessionId::new();
        let game = ChessGame::standard();
        app.begin_vision(vision_request(
            &app,
            neural_observation(&game, id, 0, false),
        ))
        .unwrap();
        let mut after = game.clone();
        after.play_uci("e2e4").unwrap();
        let mut decision = Value::Null;
        for sequence in [1, 2, 3] {
            decision = app
                .observe_vision(vision_request(
                    &app,
                    neural_observation(&after, id, sequence, false),
                ))
                .unwrap();
        }
        assert_eq!(decision["kind"], "candidate");
        let mut stale = request(&app, "e2e4");
        stale.automatic = true;
        stale.vision_session = Some(id);
        stale.proposal_id = decision["proposal_id"].as_str().map(str::to_owned);

        let error = app
            .observe_vision(vision_request(
                &app,
                neural_observation(&after, id, 5, false),
            ))
            .unwrap_err();
        assert!(
            error.to_string().contains("sequence has a gap"),
            "{error:#}"
        );
        assert!(app.vision.is_none());
        assert!(app.apply_move(stale).is_err());
        assert!(
            app.snapshot().unwrap()["moves"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn live_moves_are_legal_durable_and_retry_safe() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        assert_eq!(
            app.snapshot().unwrap()["legal_moves"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        assert!(app.apply_move(request(&app, "e2e5")).is_err());
        let retry = request(&app, "e2e4");
        let accepted = app.apply_move(request(&app, "e2e4")).unwrap();
        assert_eq!(accepted["moves"][0]["san"], "e4");
        assert_eq!(accepted["turn"], "Black");
        assert_eq!(
            app.apply_move(retry).unwrap()["moves"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        app.apply_move(request(&app, "e7e5")).unwrap();
        assert!(app.pgn().unwrap().contains("1. e4 e5 *"));
        let id = app.game_id;
        let resumed = App::open(app.store).unwrap();
        assert_eq!(resumed.game_id, id);
        assert_eq!(
            resumed.snapshot().unwrap()["moves"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn undo_keeps_history_and_new_game_preserves_the_old_game() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        let stale = request(&app, "d2d4");
        app.apply_move(request(&app, "e2e4")).unwrap();
        assert!(app.apply_move(stale).is_err());
        let state = app
            .undo(PositionRequest {
                game_id: app.game_id,
                revision: 0,
                expected_ply: 1,
            })
            .unwrap();
        assert_eq!(state["revision"], 1);
        assert!(state["moves"].as_array().unwrap().is_empty());
        assert_eq!(
            app.store
                .replay_revision(app.game_id, 0)
                .unwrap()
                .moves
                .len(),
            1
        );
        app.apply_move(request(&app, "d2d4")).unwrap();
        let old_id = app.game_id;
        let old_request = request(&app, "d7d5");
        app.new_game().unwrap();
        assert!(app.apply_move(old_request).is_err());
        assert_eq!(app.store.replay_active_game(old_id).unwrap().moves.len(), 1);
    }

    #[test]
    fn successor_changes_include_compound_moves_and_promotions() {
        let mut app = App::open(Store::open_in_memory().unwrap()).unwrap();
        for uci in ["e2e4", "a7a6", "e4e5", "d7d5"] {
            app.apply_move(request(&app, uci)).unwrap();
        }
        let state = app.snapshot().unwrap();
        let ep = state["legal_moves"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["uci"] == "e5d6")
            .unwrap();
        assert_eq!(ep["changed"].as_array().unwrap().len(), 3);
        let id = GameId::new();
        app.store
            .create_game(id, "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1", utc_us())
            .unwrap();
        app.game_id = id;
        let state = app.snapshot().unwrap();
        let castle = state["legal_moves"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["san"] == "O-O")
            .unwrap();
        assert_eq!(castle["changed"].as_array().unwrap().len(), 4);
    }
}
