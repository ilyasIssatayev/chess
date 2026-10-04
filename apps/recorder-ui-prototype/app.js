const $ = (selector) => document.querySelector(selector);
let nativeTimer, nativePolling = false;
const nativeMode = () => $("#camera-source").value === "native";
if (new URLSearchParams(location.search).get("native") === "1") $("#camera-source").value = "native";
const video = $("#video");
const canvas = $("#motion-canvas");
const context = canvas.getContext("2d", { willReadFrequently: true });
const viewport = $("#viewport");
const overlay = $("#board-overlay");
const cameraButton = $("#camera-button");
const calibrateButton = $("#calibrate-button");
const resetCalibrationButton = $("#reset-calibration-button");
const tracker = new CameraRecorder.Tracker();
const vision = new VisionClient();
const personalStore = new PersonalVisionStore();
let personalData = { samples: [], model: null, enabled: false }, personalLoaded = false;
let lastReadout;
let modelReady = false, modelBusy = false, visionSession = null, visionSequence = 0;
let visionEpoch = 0, motionVersion = 0, modelDisturbed = false, lastModelAt = 0;
const usingModel = () => $("#recognition-mode").value === "model";
vision.ready.then(async () => {
  modelReady = true; $("#model-status").textContent = "Piece model ready"; updateControls();
  try {
    personalData = await personalStore.get();
    if (personalData.enabled && personalData.model) {
      await vision.request("personal", { model: personalData.model });
      $("#personal-enabled").checked = true;
      $("#auto-record").checked = false;
    }
    personalLoaded = true; renderPersonal();
  } catch (error) {
    personalData = { samples: Array.isArray(personalData.samples) ? personalData.samples : [], model: null, enabled: false };
    $("#personal-enabled").checked = false;
    personalLoaded = true; $("#personal-detail").textContent = error.message;
  }
  updateControls();
})
  .catch((error) => { $("#model-status").textContent = "Model unavailable"; $("#model-detail").textContent = error.message; updateControls(); });
let stream, animation, game, latestPatches, candidate, selectedSquare;
let lastSample = 0, lastFrameTime = -1, lastFrameAt = 0, pending = false;
let calibrationEditing = false, draggedCorner;
const defaultCorners = {
  a8: { x: .19, y: .20 }, h8: { x: .81, y: .20 },
  h1: { x: .86, y: .86 }, a1: { x: .14, y: .86 },
};
function loadCorners() {
  try {
    const value = JSON.parse(localStorage.getItem("chess-camera-board-corners-v2"));
    if (["a8", "h8", "h1", "a1"].every((name) => value?.[name] && ["x", "y"].every((axis) => Number.isFinite(value[name][axis]) && value[name][axis] >= 0 && value[name][axis] <= 1))) return value;
  } catch (_) {}
  return null;
}
const savedCorners = loadCorners();
let corners = savedCorners || structuredClone(defaultCorners);
let calibrationSaved = Boolean(savedCorners);
function solveLinear(matrix, values) {
  const size = values.length;
  for (let column = 0; column < size; column += 1) {
    let pivot = column;
    for (let row = column + 1; row < size; row += 1) {
      if (Math.abs(matrix[row][column]) > Math.abs(matrix[pivot][column])) pivot = row;
    }
    [matrix[column], matrix[pivot]] = [matrix[pivot], matrix[column]];
    [values[column], values[pivot]] = [values[pivot], values[column]];
    if (Math.abs(matrix[column][column]) < 1e-9) throw new Error("degenerate calibration");
    const divisor = matrix[column][column];
    for (let index = column; index < size; index += 1) matrix[column][index] /= divisor;
    values[column] /= divisor;
    for (let row = 0; row < size; row += 1) {
      if (row === column) continue;
      const factor = matrix[row][column];
      for (let index = column; index < size; index += 1) matrix[row][index] -= factor * matrix[column][index];
      values[row] -= factor * values[column];
    }
  }
  return values;
}

function homography(points) {
  const sources = [[0, 0], [1, 0], [1, 1], [0, 1]];
  const matrix = [];
  const values = [];
  sources.forEach(([u, v], index) => {
    const { x, y } = points[index];
    matrix.push([u, v, 1, 0, 0, 0, -x * u, -x * v]); values.push(x);
    matrix.push([0, 0, 0, u, v, 1, -y * u, -y * v]); values.push(y);
  });
  const h = solveLinear(matrix, values);
  return (u, v) => {
    const scale = h[6] * u + h[7] * v + 1;
    return { x: (h[0] * u + h[1] * v + h[2]) / scale, y: (h[3] * u + h[4] * v + h[5]) / scale };
  };
}

function calibrationQuality(points, width, height) {
  const cross = (a, b, c) => (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
  const turns = points.map((point, index) => cross(point, points[(index + 1) % 4], points[(index + 2) % 4]));
  if (!(turns.every((turn) => turn > 0) || turns.every((turn) => turn < 0))) return { valid: false, label: "Corners are crossed" };
  const area = Math.abs(points.reduce((sum, point, index) => sum + point.x * points[(index + 1) % 4].y - point.y * points[(index + 1) % 4].x, 0)) / 2;
  if (area < width * height * 0.08) return { valid: false, label: "Board is too small in frame" };
  const length = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
  const edges = points.map((point, index) => length(point, points[(index + 1) % 4]));
  if (Math.min(...edges) < 45) return { valid: false, label: "Corners are too close" };
  const horizontalRatio = Math.min(edges[0], edges[2]) / Math.max(edges[0], edges[2]);
  if (horizontalRatio < 0.18) return { valid: false, label: "Viewing angle is too shallow" };
  try {
    const geometry = ChessVisionCore.geometry(corners, width, height);
    const view = ChessVisionCore.viewQuality(geometry.project);
    return { valid: true, label: view.compressed ? "Shallow side view · check piece bases" : horizontalRatio < 0.45 ? "Strong perspective · check occlusion" : "Tilted view calibrated" };
  } catch (error) { return { valid: false, label: error.message }; }
}

function svgElement(name, attributes = {}) {
  const element = document.createElementNS("http://www.w3.org/2000/svg", name);
  Object.entries(attributes).forEach(([key, value]) => element.setAttribute(key, value));
  return element;
}

function renderCalibration() {
  const width = viewport.clientWidth || 1000;
  const height = viewport.clientHeight || 560;
  overlay.setAttribute("viewBox", `0 0 ${width} ${height}`);
  const points = [corners.a8, corners.h8, corners.h1, corners.a1].map((point) => ({ x: point.x * width, y: point.y * height }));
  let project;
  try { project = homography(points); } catch (_) { project = () => ({ x: 0, y: 0 }); }
  const quality = calibrationQuality(points, width, height);
  overlay.classList.toggle("invalid", !quality.valid);
  document.querySelector("#calibration").textContent = calibrationSaved ? quality.label : "Set board corners first";

  const fills = document.querySelector("#square-fills");
  fills.replaceChildren();
  for (let rankIndex = 0; rankIndex < 8; rankIndex += 1) {
    for (let fileIndex = 0; fileIndex < 8; fileIndex += 1) {
      const mapped = [[fileIndex, rankIndex], [fileIndex + 1, rankIndex], [fileIndex + 1, rankIndex + 1], [fileIndex, rankIndex + 1]]
        .map(([u, v]) => project(u / 8, v / 8));
      const polygon = svgElement("polygon", { points: mapped.map((point) => `${point.x},${point.y}`).join(" "), class: "square-fill", "data-name": `${"abcdefgh"[fileIndex]}${8 - rankIndex}` });
      fills.append(polygon);
    }
  }

  const lines = document.querySelector("#grid-lines");
  lines.replaceChildren();
  for (let index = 0; index <= 8; index += 1) {
    const vertical = [project(index / 8, 0), project(index / 8, 1)];
    const horizontal = [project(0, index / 8), project(1, index / 8)];
    for (const [start, end] of [vertical, horizontal]) {
      lines.append(svgElement("line", { x1: start.x, y1: start.y, x2: end.x, y2: end.y, class: `grid-line ${index === 0 || index === 8 ? "board-boundary" : ""}` }));
    }
  }

  const handles = document.querySelector("#corner-handles");
  handles.replaceChildren();
  for (const name of ["a8", "h8", "h1", "a1"]) {
    const point = { x: corners[name].x * width, y: corners[name].y * height };
    const group = svgElement("g", { class: "corner-handle", "data-corner": name, transform: `translate(${point.x} ${point.y})` });
    group.append(svgElement("circle", { r: calibrationEditing ? 19 : 13 }));
    const text = svgElement("text"); text.textContent = name; group.append(text); handles.append(group);
  }
}

function toggleCalibration() {
  if (calibrationEditing) {
    const points = [corners.a8, corners.h8, corners.h1, corners.a1].map((point) => ({ x: point.x * viewport.clientWidth, y: point.y * viewport.clientHeight }));
    if (!calibrationQuality(points, viewport.clientWidth, viewport.clientHeight).valid) return;
    localStorage.setItem("chess-camera-board-corners-v2", JSON.stringify(corners));
    calibrationSaved = true;
    if (stream?.native) stopCamera();
  }
  disarm("Calibration changed. Set a fresh reference before recording.");
  calibrationEditing = !calibrationEditing;
  overlay.classList.toggle("editing", calibrationEditing);
  document.querySelector("#calibration-help").hidden = !calibrationEditing;
  resetCalibrationButton.hidden = !calibrationEditing;
  calibrateButton.textContent = calibrationEditing ? "Save calibration" : "Calibrate board";
  renderCalibration();
}

overlay.addEventListener("pointerdown", (event) => {
  if (!calibrationEditing) return;
  const handle = event.target.closest(".corner-handle");
  if (!handle) return;
  draggedCorner = handle.dataset.corner;
  overlay.setPointerCapture(event.pointerId);
});
overlay.addEventListener("pointermove", (event) => {
  if (!draggedCorner) return;
  const bounds = overlay.getBoundingClientRect();
  corners[draggedCorner] = { x: Math.min(.98, Math.max(.02, (event.clientX - bounds.left) / bounds.width)), y: Math.min(.98, Math.max(.02, (event.clientY - bounds.top) / bounds.height)) };
  renderCalibration();
});
overlay.addEventListener("pointerup", () => { draggedCorner = undefined; });
overlay.addEventListener("pointercancel", () => { draggedCorner = undefined; });
resetCalibrationButton.addEventListener("click", () => { corners = structuredClone(defaultCorners); renderCalibration(); });
calibrateButton.addEventListener("click", toggleCalibration);
window.addEventListener("resize", renderCalibration);


function setDecision(state, title, copy, icon = "·", move) {
  $("#decision-card").dataset.state = state;
  $("#decision-title").textContent = title;
  $("#decision-copy").textContent = copy;
  $("#decision-icon").textContent = icon;
  $("#candidate").hidden = !move;
  highlight(move?.changed || []);
  if (move) {
    $("#candidate-san").textContent = move.san;
    $("#candidate-squares").textContent = `${move.from} → ${move.to}`;
  }
}

function highlight(squares) {
  document.querySelectorAll(".square-fill").forEach((square) => square.classList.toggle("changed", squares.includes(square.dataset.name)));
}

function disarm(copy = "Set the physical board to match the tracked position, then set a reference.") {
  tracker.reset();
  visionEpoch += 1; visionSession = null; visionSequence = 0; modelDisturbed = false;
  candidate = null;
  $("#position-confirm").checked = false;
  $("#sample-confirm").checked = false;
  $("#recording-label").textContent = "Recording paused";
  if (stream?.native) api("/api/native/disarm",{}).catch(()=>{});
  if (game) setDecision("idle", "Set a reference position", copy);
  updateControls();
}

function updateControls() {
  if (nativeMode()) {
    const ready = game && stream && !calibrationEditing && calibrationSaved && performance.now()-lastFrameAt<2000 && !pending;
    $("#reference-button").disabled = !ready || !$("#position-confirm").checked;
    $("#read-board-button").disabled = !stream;
    for (const id of ["sample-button","train-button","clear-personal-button","personal-enabled","recognition-mode"]) $("#"+id).disabled=true;
    $("#auto-record").disabled=false;
    $("#record-button").disabled=!game || !$("#manual-move").value || pending;
    $("#undo-button").disabled=!game?.moves.length || pending;
    $("#new-game-button").disabled=!game || pending;
    $("#export-button").disabled=!game || pending;
    calibrateButton.disabled=pending;
    return;
  }
  $("#recognition-mode").disabled=false;
  const cameraReady = game && stream && !calibrationEditing && calibrationSaved && latestPatches && performance.now() - lastFrameAt <= 2000 && !pending;
  $("#reference-button").disabled = !cameraReady || !$("#position-confirm").checked || (usingModel() && (!modelReady || modelBusy));
  $("#read-board-button").disabled = !cameraReady || !modelReady || modelBusy || Boolean(tracker.reference);
  $("#auto-record").disabled = !usingModel() || $("#personal-enabled").checked;
  $("#sample-button").disabled = !cameraReady || !modelReady || modelBusy || !personalLoaded || !$("#sample-confirm").checked || personalData.samples.length >= 20;
  $("#train-button").disabled = !modelReady || modelBusy || pending || !personalLoaded || personalData.samples.length < 3;
  $("#clear-personal-button").disabled = modelBusy || pending || !personalLoaded || (!personalData.samples.length && !personalData.model);
  $("#personal-enabled").disabled = !modelReady || modelBusy || pending || !personalLoaded || !personalData.model;
  $("#record-button").disabled = !game || !$("#manual-move").value || pending;
  $("#undo-button").disabled = !game?.moves.length || pending;
  $("#new-game-button").disabled = !game || pending;
  $("#export-button").disabled = !game || pending;
  calibrateButton.disabled = pending;
}

function renderGame() {
  $("#turn").textContent = `${game.turn} to move`;
  $("#game-status").textContent = game.status === "Ongoing" ? "" : `Game status: ${game.status}`;
  $("#move-count").textContent = `${game.moves.length} ${game.moves.length === 1 ? "ply" : "plies"}`;
  $("#empty-moves").hidden = game.moves.length > 0;
  const list = $("#moves");
  list.replaceChildren();
  for (const [index, move] of game.moves.entries()) {
    const item = document.createElement("li");
    const number = document.createElement("span");
    number.textContent = `${Math.floor(index / 2) + 1}${index % 2 ? "…" : "."}`;
    const san = document.createElement("strong"); san.textContent = move.san;
    const source = document.createElement("small"); source.textContent = move.provenance === "automatic" ? "auto" : "reviewed";
    item.append(number, san, source); list.append(item);
  }
  const select = $("#manual-move");
  select.replaceChildren(new Option("Choose the move played…", ""));
  for (const move of game.legal_moves) select.add(new Option(`${move.san} · ${move.from} → ${move.to}${move.uci.length === 5 ? ` = ${move.uci[4].toUpperCase()}` : ""}`, move.uci));
  const symbols = { white_pawn: "♙", white_knight: "♘", white_bishop: "♗", white_rook: "♖", white_queen: "♕", white_king: "♔", black_pawn: "♟", black_knight: "♞", black_bishop: "♝", black_rook: "♜", black_queen: "♛", black_king: "♚" };
  const pieces = Object.fromEntries(game.pieces);
  const board = $("#tracked-board"); board.replaceChildren();
  CameraRecorder.names.forEach((square, i) => {
    const button = document.createElement("button");
    button.className = `chess-square ${(Math.floor(i / 8) + i % 8) % 2 ? "dark" : "light"}`;
    button.dataset.square = square;
    button.setAttribute("aria-label", `${square} ${pieces[square]?.replaceAll("_", " ") || "empty"}`);
    const piece = document.createElement("span"); piece.textContent = symbols[pieces[square]] || "";
    const label = document.createElement("small"); label.textContent = square;
    button.append(piece, label); board.append(button);
  });
  selectedSquare = null;
  updateControls();
}

async function api(path = "/api/game", body) {
  const response = await fetch(path, { cache: "no-store", ...(body ? { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) } : {}) });
  const type = response.headers.get("content-type") || "";
  if (!type.includes("application/json")) throw new Error("Start the recorder with scripts/start-recorder.sh, then open http://localhost:8770/. A static preview cannot save moves.");
  const value = await response.json();
  if (!response.ok) throw new Error(value.error || "Recorder request failed.");
  return value;
}

function positionRequest() { return { game_id: game.game_id, revision: game.revision, expected_ply: game.moves.length }; }

async function recordMove(move, automatic = false, patches) {
  if (pending || !game || !move) return;
  pending = true; visionEpoch += 1; updateControls();
  try {
    game = await api("/api/game/move", { ...positionRequest(), uci: move.uci, automatic, vision_session: move.vision_session, proposal_id: move.proposal_id });
    renderGame();
    candidate = null;
    if (nativeMode() && game.vision_session) {
      visionSession=game.vision_session;
      setDecision("accepted",`${move.san} recorded`, `Saved with capture-time bounds. ${game.turn} to move.`,"✓",move);
    } else if (patches && tracker.reference && stream && !document.hidden && (!usingModel() || visionSession && move.proposal_id && game.vision_session === visionSession) && !CameraRecorder.changes(patches, latestPatches).some((e) => e.changed)) {
      tracker.setReference(patches, performance.now());
      setDecision("accepted", `${move.san} recorded`, `Saved locally. ${game.turn} to move.`, "✓", move);
    } else {
      disarm();
      setDecision("accepted", `${move.san} recorded`, `Saved locally. Match the physical board to the tracked position and set a fresh reference to continue camera recording.`, "✓", move);
    }
  } catch (error) {
    disarm();
    setDecision("review", "Move was not saved", error.message, "!");
  } finally { pending = false; updateControls(); }
}

async function toggleCamera() {
  if (nativeMode()) { await toggleNative(); return; }
  if (stream) { stopCamera(); return; }
  cameraButton.disabled = true;
  let acquired;
  try {
    acquired = await navigator.mediaDevices.getUserMedia({ video: { width: { ideal: 1280 }, height: { ideal: 720 } }, audio: false });
    video.srcObject = acquired;
    await video.play();
    if (!video.videoWidth || !video.videoHeight) throw new Error("Camera is not delivering video frames.");
    stream = acquired;
    // One coordinate system for video display, calibration and pixel sampling.
    viewport.style.aspectRatio = `${video.videoWidth} / ${video.videoHeight}`;
    canvas.width = Math.min(video.videoWidth, 640);
    canvas.height = Math.round(canvas.width * video.videoHeight / video.videoWidth);
    $("#empty-camera").hidden = true;
    $("#capture-dot").classList.add("live");
    $("#capture-label").textContent = "Camera live";
    $("#camera-detail").textContent = "Local preview · frames stay in this browser";
    $("#visibility").textContent = "Check all 64 squares";
    cameraButton.textContent = "Stop camera";
    latestPatches = null; lastFrameTime = -1; lastFrameAt = performance.now();
    for (const track of stream.getTracks()) track.addEventListener("ended", () => { stopCamera(); setDecision("review", "Camera disconnected", "Restart the camera and set a fresh reference.", "!"); });
    disarm(calibrationSaved ? undefined : "Calibrate the board corners, then confirm the physical position matches the tracked board.");
    renderCalibration();
    animation = requestAnimationFrame(monitorCamera);
  } catch (error) {
    acquired?.getTracks().forEach((track) => track.stop());
    stream = undefined; video.srcObject = null;
    setDecision("review", "Camera unavailable", error.message, "!");
  } finally { cameraButton.disabled = false; updateControls(); }
}

function stopCamera() {
  if (stream?.native) {
    clearInterval(nativeTimer); nativeTimer=null; stream=undefined; visionSession=null; candidate=null;
    $("#native-preview").hidden=true; $("#video").hidden=false; $("#empty-camera").hidden=false;
    $("#capture-label").textContent="Camera idle"; cameraButton.textContent="Start camera";
    fetch("/api/native/stop",{method:"POST",headers:{"Content-Type":"application/json"},body:"{}",keepalive:true}).catch(()=>{}); updateControls(); return;
  }
  const old = stream;
  stream = undefined;
  old?.getTracks().forEach((track) => track.stop());
  cancelAnimationFrame(animation);
  video.srcObject = null; latestPatches = null;
  $("#empty-camera").hidden = false;
  $("#capture-dot").classList.remove("live");
  $("#capture-label").textContent = "Camera idle";
  $("#camera-detail").textContent = "Preview unavailable";
  $("#visibility").textContent = "Waiting";
  $("#timing").textContent = "Waiting";
  $("#fps").textContent = "— fps";
  $("#motion-badge").textContent = "NO VIDEO";
  cameraButton.textContent = "Start camera";
  disarm();
}

let liveStableSince = 0;
function monitorCamera(now) {
  if (!stream) return;
  animation = requestAnimationFrame(monitorCamera);
  if (document.hidden || now - lastSample < 150 || video.readyState < 2) return;
  if (now - lastFrameAt > 2000) {
    if (tracker.reference) disarm("Camera frames were interrupted. Check the tracked position and set a fresh reference.");
    $("#timing").textContent = "Frame gap";
  }
  if (video.currentTime === lastFrameTime) return;
  $("#fps").textContent = `${Math.round(1000 / (now - lastFrameAt))} checks/s`;
  lastFrameTime = video.currentTime;
  lastFrameAt = now; lastSample = now;
  try {
    context.drawImage(video, 0, 0, canvas.width, canvas.height);
    const points = [corners.a8, corners.h8, corners.h1, corners.a1].map((p) => ({ x: p.x * canvas.width, y: p.y * canvas.height }));
    const threshold = Number($("#sensitivity").value);
    const patches = CameraRecorder.sampleSquares(context.getImageData(0, 0, canvas.width, canvas.height).data, canvas.width, canvas.height, homography(points));
    const moving = latestPatches && CameraRecorder.changes(latestPatches, patches, threshold / 2).some((e) => e.changed);
    if (moving) { motionVersion += 1; modelDisturbed = true; candidate = null; $("#manual-move").value = ""; $("#sample-confirm").checked = false; }
    if (moving || !latestPatches) liveStableSince = now;
    latestPatches = patches;
    const badge = $("#motion-badge");
    const settling = now - liveStableSince < 900;
    badge.textContent = moving ? "MOTION" : settling ? "SETTLING" : "STABLE";
    badge.classList.toggle("moving", Boolean(moving || settling));
    $("#timing").textContent = "Preview active";
    updateControls();
    if (!game || calibrationEditing || pending || !tracker.reference) return;
    if (usingModel()) {
      if (settling) setDecision("moving", "Waiting for the board to settle", "Keep your hands clear while the camera reads the pieces.", "↻");
      else if (!modelBusy && modelReady && now - lastModelAt >= 350) analyzeModel();
      return;
    }
    const result = tracker.ingest(patches, now, game.legal_moves, threshold);
    if (result.kind === "moving" || result.kind === "settling") {
      candidate = null;
      setDecision("moving", "Waiting for the board to settle", "Keep hands clear until the position is stable.", "↻");
      return;
    }
    if (result.kind === "unchanged") {
      if (candidate) $("#manual-move").value = "";
      candidate = null;
      setDecision("stable", "Watching for a move", `${game.turn} to move. No settled square changes.`, "✓");
      return;
    }
    if (result.kind === "candidate") {
      candidate = { ...result.matches[0], patches: result.patches };
      $("#manual-move").value = candidate.uci;
      setDecision("review", `${candidate.san} detected`, "The square changes match this legal move. Confirm it below.", "?", candidate);
      // Change-only fallback is reviewed; automatic commits require neural evidence.
    } else {
      candidate = null;
      $("#manual-move").value = "";
      setDecision("review", "Position needs review", result.matches.length > 1 ? "Several legal moves have the same changed squares. Choose the move or promotion played below." : `${result.changed.length} changed squares do not match one legal move. Check corners, lighting and board position, or record the move manually.`, "!");
      highlight(result.changed);
    }
    updateControls();
  } catch (error) {
    disarm(); setDecision("review", "Check calibration", error.message, "!");
  }
}

function showModelReadout(result) {
  lastReadout = result;
  $("#crop-inspector").hidden = true;
  const board = $("#recognized-board"); board.replaceChildren();
  const evidence = Object.fromEntries(result.squares.map((square) => [square.square, square]));
  const symbols = ["♙", "♘", "♗", "♖", "♕", "♔", "♟", "♞", "♝", "♜", "♛", "♚"];
  const expected = Object.fromEntries(game?.pieces || []);
  const differences = [], uncertain = [];
  CameraRecorder.names.forEach((name, i) => {
    const e = evidence[name], values = [e.empty_probability, ...e.piece_probabilities];
    const index = values.indexOf(Math.max(...values));
    const piece = index === 0 ? null : ChessVisionCore.classes[index - 1];
    if (e.visible_probability < .5) uncertain.push(name);
    if (game && piece !== expected[name]) differences.push(name);
    const cell = document.createElement("button");
    cell.type = "button"; cell.dataset.square = name;
    cell.className = `chess-square ${(Math.floor(i / 8) + i % 8) % 2 ? "dark" : "light"} ${e.visible_probability < .5 ? "uncertain" : ""}`;
    cell.setAttribute("aria-label", `${name} ${piece?.replaceAll("_", " ") || "empty"}${e.visible_probability < .5 ? " uncertain" : ""}`);
    const symbol = document.createElement("span"); symbol.textContent = index === 0 ? "" : symbols[index - 1];
    const label = document.createElement("small"); label.textContent = name;
    cell.append(symbol, label); board.append(cell);
  });
  $("#model-status").textContent = result.personalized ? "Personal recognition · review" : "Piece model ready";
  $("#model-detail").textContent = `${result.latency_ms} ms per board · ${uncertain.length} uncertain squares${differences.length ? ` · differs from tracked position at ${differences.slice(0, 12).join(", ")}${differences.length > 12 ? "…" : ""}` : " · matches the tracked position"}.${result.quality?.warning ? ` ${result.quality.warning}` : ""}${$("#personal-enabled").checked && !result.personalized ? " Personal examples use different camera geometry; the base model is active. Restore the saved camera position or collect new examples." : ""}`;
}

function observation(result, sessionId, sequence, captureTime, moving = false) {
  return { session_id: sessionId, sequence, capture_time: Math.round(captureTime * 1000), moving,
    calibration_version: JSON.stringify(corners), model_version: result.version, squares: result.squares };
}

async function analyzeModel(readOnly = false) {
  if (modelBusy || !stream || !modelReady) return;
  const epoch = visionEpoch, motion = motionVersion, captured = performance.now(), patches = latestPatches;
  const disturbed = modelDisturbed;
  modelBusy = true; lastModelAt = captured; updateControls();
  try {
    const result = await vision.infer(video, structuredClone(corners), { inspect: readOnly });
    if (epoch !== visionEpoch || motion !== motionVersion || document.hidden || !stream || performance.now() - lastFrameAt > 2000) return;
    showModelReadout(result);
    if (readOnly) return;
    if (!visionSession || !tracker.reference || pending) return;
    const decision = await api("/api/vision/observe", { ...positionRequest(), observation: observation(result, visionSession, ++visionSequence, captured, disturbed) });
    if (epoch !== visionEpoch || motion !== motionVersion || document.hidden || !stream) return;
    modelDisturbed = false;
    if (decision.kind === "candidate") {
      const move = game.legal_moves.find((m) => m.uci === decision.uci);
      if (!move) throw new Error("The model proposal belongs to a different tracked position.");
      candidate = { ...move, patches, vision_session: decision.session_id, proposal_id: decision.proposal_id };
      $("#manual-move").value = candidate.uci;
      setDecision("review", `${candidate.san} detected`, "Piece recognition supports this legal move across three observations. Confirm it below.", "?", candidate);
      if ($("#auto-record").checked && !result.personalized) recordMove(candidate, true, patches);
    } else if (decision.kind === "unchanged") {
      candidate = null; $("#manual-move").value = "";
      setDecision("stable", "Watching the pieces", `${game.turn} to move. The camera position matches the tracked board.`, "✓");
    } else if (decision.kind === "review") {
      candidate = null; $("#manual-move").value = "";
      setDecision("review", "Camera position needs review", "The model cannot identify one supported legal move. Check the camera readout, lighting and corner calibration, or enter the move manually.", "!");
    } else {
      candidate = null; $("#manual-move").value = "";
      setDecision("moving", "Reading the new position", "Waiting for consistent piece recognition before recording a move.", "…");
    }
  } catch (error) {
    if (epoch === visionEpoch) { disarm(); setDecision("review", "Recognition needs attention", error.message, "!"); }
  } finally { modelBusy = false; updateControls(); }
}

$("#reference-button").addEventListener("click", async () => {
  if (nativeMode()) {
    if (!game || pending || !$("#position-confirm").checked) return;
    pending=true; updateControls();
    try { game=await api("/api/native/reference",positionRequest()); visionSession=game.vision_session; $("#recording-label").textContent="Watching with native recognition"; setDecision("stable","Watching for a move",`${game.turn} to move. Timing uses camera presentation timestamps.`,"✓"); }
    catch(error) {setDecision("review","Reference could not be verified",error.message,"!");}
    finally{pending=false;updateControls();} return;
  }
  if (!game || !stream || pending || !latestPatches || !$("#position-confirm").checked) return;
  const quality = calibrationQuality([corners.a8, corners.h8, corners.h1, corners.a1].map((p) => ({ x: p.x * viewport.clientWidth, y: p.y * viewport.clientHeight })), viewport.clientWidth, viewport.clientHeight);
  if (!quality.valid) { setDecision("review", "Check calibration", quality.label, "!"); return; }
  if (performance.now() - liveStableSince < 900) { setDecision("moving", "Wait for a stable board", "Clear your hands, then set the reference again.", "↻"); return; }
  const epoch = visionEpoch, motion = motionVersion, captured = performance.now(), patches = latestPatches;
  if (usingModel()) {
    if (!modelReady || modelBusy) return;
    pending = true; modelBusy = true; updateControls();
    setDecision("moving", "Verifying the reference position", "Reading all 64 squares before recording starts.", "…");
    try {
      const result = await vision.infer(video, structuredClone(corners));
      if (epoch !== visionEpoch || motion !== motionVersion || !stream || document.hidden) throw new Error("The board changed during verification. Clear your hands and try again.");
      showModelReadout(result);
      const id = crypto.randomUUID();
      await api("/api/vision/reference", { ...positionRequest(), observation: observation(result, id, 0, captured) });
      if (epoch !== visionEpoch || motion !== motionVersion || !stream || document.hidden) throw new Error("The camera changed during verification. Set a fresh reference.");
      visionSession = id; visionSequence = 0; modelDisturbed = false;
    } catch (error) {
      if (epoch === visionEpoch) { disarm(); setDecision("review", "Reference could not be verified", error.message, "!"); }
      return;
    } finally { pending = false; modelBusy = false; updateControls(); }
  }
  tracker.setReference(patches, performance.now());
  $("#recording-label").textContent = usingModel() ? "Watching with piece recognition" : "Watching square changes for review";
  $("#manual-move").value = "";
  setDecision("stable", "Watching for a move", `${game.turn} to move. Play one move, then let the board settle.`, "✓");
  updateControls();
});
$("#read-board-button").addEventListener("click", () => {
  if (nativeMode()) { $("#camera-readout").open=true; pollNative(); return; }
  if (performance.now() - liveStableSince < 900) { setDecision("moving", "Wait for a stable board", "Keep your hands clear, then read the board again.", "↻"); return; }
  $("#camera-readout").open = true;
  analyzeModel(true);
});
$("#recognition-mode").addEventListener("change", () => {
  if (!usingModel()) $("#auto-record").checked = false;
  disarm("Recognition mode changed. Set a fresh reference.");
});

$("#position-confirm").addEventListener("change", updateControls);
$("#sensitivity").addEventListener("change", () => { if (stream?.native) stopCamera(); disarm("Detection sensitivity changed. Restart the camera and set a fresh reference."); });
$("#record-button").addEventListener("click", () => {
  const uci = $("#manual-move").value;
  recordMove(candidate?.uci === uci ? candidate : game?.legal_moves.find((move) => move.uci === uci), false, candidate?.uci === uci ? candidate.patches : undefined);
});
$("#manual-move").addEventListener("change", () => { if (tracker.reference) disarm("Manual review selected. Record the move, then set a fresh reference."); updateControls(); });
$("#tracked-board").addEventListener("click", (event) => {
  const button = event.target.closest("[data-square]");
  if (!button || !game || pending) return;
  if (tracker.reference) disarm("Manual review selected. Record the move, then set a fresh reference.");
  const square = button.dataset.square;
  if (selectedSquare) {
    const choices = game.legal_moves.filter((m) => m.from === selectedSquare && m.to === square);
    if (choices.length === 1) { $("#manual-move").value = choices[0].uci; selectedSquare = null; }
    else if (choices.length > 1) { setDecision("review", "Choose a promotion", "Select the piece promoted to in the move menu.", "!"); selectedSquare = null; }
    else selectedSquare = square;
  } else selectedSquare = square;
  document.querySelectorAll(".chess-square").forEach((cell) => cell.classList.toggle("selected", cell.dataset.square === selectedSquare));
  updateControls();
});

async function changeGame(path) {
  if (!game || pending) return;
  pending = true; disarm(); updateControls();
  try { game = await api(path, positionRequest()); renderGame(); disarm(); }
  catch (error) { setDecision("review", "Game was not changed", error.message, "!"); }
  finally { pending = false; updateControls(); }
}
$("#undo-button").addEventListener("click", () => changeGame("/api/game/undo"));
$("#new-game-button").addEventListener("click", () => changeGame("/api/game/new"));
$("#export-button").addEventListener("click", async () => {
  try {
    const response = await fetch("/api/game/pgn");
    if (!response.ok) throw new Error("Could not export this game.");
    const url = URL.createObjectURL(new Blob([await response.text()], { type: "application/x-chess-pgn" }));
    const link = document.createElement("a"); link.href = url; link.download = `chess-${game.game_id}.pgn`; link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  } catch (error) { setDecision("review", "Export failed", error.message, "!"); }
});
document.addEventListener("visibilitychange", () => { if (document.hidden && tracker.reference) disarm("The tab was hidden. Check the tracked position and set a fresh reference."); });
window.addEventListener("pagehide", stopCamera);
cameraButton.addEventListener("click", toggleCamera);
renderCalibration();
updateControls();
api().then((value) => { game = value; renderGame(); disarm(); }).catch((error) => setDecision("review", "Recorder connection needed", error.message, "!"));

function renderPersonal() {
  const positions = new Set(personalData.samples.map((s) => s.position)).size;
  $("#personal-detail").textContent = `${personalData.samples.length}/20 examples · ${positions} different positions. Neural features stay in this browser; photos are not saved. Changing the camera angle requires new examples.`;
  const m = personalData.model?.metrics;
  $("#personal-validation").textContent = m ? `On ${m.validationFrames} held-out photo(s) of one position: personal recognition ${m.correct}/${m.count} squares; base model ${m.baselineCorrect}/${m.count}. This is a small local check, not a general accuracy estimate.${m.errors.length ? ` Check ${m.errors.join(", ")}.` : ""} Review each suggested move.` : "The last position will be reserved for validation before the final head is trained on all examples.";
}

$("#sample-confirm").addEventListener("change", updateControls);
$("#sample-button").addEventListener("click", async () => {
  if (!stream || !game || pending || modelBusy || !$("#sample-confirm").checked) return;
  if (performance.now() - liveStableSince < 900) { $("#personal-detail").textContent = "Wait until the board is stable, then confirm and save again."; return; }
  // Keep the label snapshot fixed for the whole asynchronous capture.
  const position = game.fen.split(" ")[0], pieces = Object.fromEntries(game.pieces), motion = motionVersion;
  disarm("Collecting a piece-set example. Set a fresh reference before recording.");
  const epoch = visionEpoch;
  pending = true; modelBusy = true; updateControls();
  try {
    const result = await vision.infer(video, structuredClone(corners), { sample: true, inspect: true, position, pieces });
    if (epoch !== visionEpoch || motion !== motionVersion || !stream || document.hidden || performance.now() - lastFrameAt > 2000) throw new Error("The board changed during capture. Clear your hands and save again.");
    const sample = result.sample;
    if (sample.coverage.some((c, i) => c < .9 && sample.labels[i] >= 0)) throw new Error("Some labelled pieces are clipped by the camera frame. Check the crop inspector and move the camera back or higher.");
    if (personalData.samples.length && personalData.samples[0].calibration !== sample.calibration) throw new Error("The camera calibration differs from saved examples. Restore it or clear the old examples first.");
    const next = { ...personalData, samples: [...personalData.samples, sample] };
    await personalStore.set(next); personalData = next;
    showModelReadout(result); $("#camera-readout").open = true; renderPersonal();
  } catch (error) { $("#personal-detail").textContent = error.message; }
  finally { pending = false; modelBusy = false; updateControls(); }
});

$("#train-button").addEventListener("click", async () => {
  if (pending || modelBusy || personalData.samples.length < 3) return;
  disarm("Personal recognition changed. Read the board, then set a fresh reference.");
  pending = true; modelBusy = true; updateControls();
  $("#personal-detail").textContent = "Training two small heads on local neural features and checking a held-out position…";
  try {
    const model = await vision.request("train", { samples: personalData.samples });
    const next = { ...personalData, model, enabled: false };
    await personalStore.set(next);
    await vision.request("personal", { model: null });
    personalData = next; $("#personal-enabled").checked = false; renderPersonal();
  } catch (error) { $("#personal-detail").textContent = error.message; }
  finally { pending = false; modelBusy = false; updateControls(); }
});

$("#personal-enabled").addEventListener("change", async () => {
  const enabled = $("#personal-enabled").checked;
  disarm("Recognition model changed. Read the camera position, then set a fresh reference.");
  pending = true; updateControls();
  $("#auto-record").checked = false;
  try {
    await vision.request("personal", { model: enabled ? personalData.model : null });
    const next = { ...personalData, enabled };
    await personalStore.set(next); personalData = next; renderPersonal();
  } catch (error) {
    await vision.request("personal", { model: null }).catch(() => {});
    personalData.enabled = false; $("#personal-enabled").checked = false; $("#personal-detail").textContent = error.message;
  } finally { pending = false; updateControls(); }
});

$("#clear-personal-button").addEventListener("click", async () => {
  if (pending || modelBusy) return;
  pending = true; disarm("Personal examples cleared. Set a fresh reference with the base model."); updateControls();
  try {
    const next = { samples: [], model: null, enabled: false };
    await personalStore.set(next); await vision.request("personal", { model: null });
    personalData = next; $("#personal-enabled").checked = false; renderPersonal();
  } catch (error) { $("#personal-detail").textContent = error.message; }
  finally { pending = false; updateControls(); }
});

$("#recognized-board").addEventListener("click", (event) => {
  const name = event.target.closest("[data-square]")?.dataset.square;
  if (!name || !lastReadout) return;
  const preview = lastReadout.previews?.find((p) => p.square === name);
  if (!preview) { $("#model-detail").textContent = "Pause recording and click Read camera position to inspect all 64 crops."; return; }
  for (const [selector, rgb, width, height] of [["#occupancy-crop", preview.occupancy, 100, 100], ["#piece-crop", preview.rgb, 100, 200]]) {
    const target = $(selector), ctx = target.getContext("2d"), data = ctx.createImageData(width, height);
    for (let i = 0; i < width * height; i += 1) { data.data.set(rgb.subarray(i * 3, i * 3 + 3), i * 4); data.data[i * 4 + 3] = 255; }
    ctx.putImageData(data, 0, 0);
  }
  const evidence = lastReadout.squares.find((e) => e.square === name);
  const top = [{ label: "empty", probability: evidence.empty_probability }, ...ChessVisionCore.classes.map((label, i) => ({ label: label.replaceAll("_", " "), probability: evidence.piece_probabilities[i] }))].sort((a, b) => b.probability - a.probability).slice(0, 3);
  $("#crop-detail").textContent = `${name} · ${Math.round(preview.coverage * 100)}% crop inside frame. ${top.map((p) => `${p.label}: ${Math.round(p.probability * 100)}%`).join("; ")}. These scores do not measure whether another piece blocks the view.`;
  $("#crop-inspector").hidden = false;
});

$("#camera-source").addEventListener("change",()=>{stopCamera(); $("#recognition-mode").value="model"; disarm(); updateControls();});
async function toggleNative() {
  if (stream) { stopCamera(); return; }
  pending=true; updateControls();
  try {
    game=await api("/api/native/start",{corners:structuredClone(corners),threshold:Number($("#sensitivity").value)});
    stream={native:true}; visionSession=null; candidate=null;
    $("#video").hidden=true; $("#native-preview").hidden=false;
    $("#empty-camera").hidden=true; viewport.style.aspectRatio="16 / 9";
    cameraButton.textContent="Stop camera"; $("#camera-detail").textContent="Native CPU recognition · local evidence retention";
    $("#capture-label").textContent="Starting native camera…";
    nativeTimer=setInterval(pollNative,400); pollNative();
  } catch(error) {setDecision("review","Native camera unavailable",error.message,"!");}
  finally{pending=false;updateControls();}
}
async function pollNative() {
  if (!stream?.native || nativePolling) return;
  nativePolling=true;
  try {
    const state=await api("/api/native/status");
    if (!stream?.native) return;
    const health=state.health;
    $("#capture-label").textContent=health?.state === "live" ? "Native camera live" : health?.state || "Camera stopped";
    $("#timing").textContent=health ? `Camera PTS · ${health.overwritten} skipped · ${health.camera_dropped} capture drops` : "Stopped";
    $("#model-status").textContent=health?.state === "live" ? `Native CPU · ${health.inference_ms} ms` : "Waiting";
    if (health?.state === "live" && state.reading) {
      lastFrameAt=performance.now();
      $("#native-preview").src=`/api/native/preview.jpg?t=${health.capture_time_us}`;
      $("#visibility").textContent=`${state.reading.squares.filter(s=>s.visible_probability>=.5).length}/64 usable squares`;
      showModelReadout(state.reading);
    }
    if (!pending && (state.game.revision!==game.revision || state.game.moves.length!==game.moves.length)) { game=state.game; renderGame(); }
    if (visionSession && !state.game.vision_session) {visionSession=null;candidate=null;setDecision("review","Reference required",state.decision.reason || health?.error || "Verify the physical position.","!");}
    if (health?.state === "interrupted") {visionSession=null;candidate=null;setDecision("review","Capture interrupted",health.error,"!");}
    const decision=state.decision;
    if (visionSession && !pending && decision.kind === "candidate") {
      const move=game.legal_moves.find(m=>m.uci===decision.uci);
      if(move){candidate={...move,vision_session:decision.session_id,proposal_id:decision.proposal_id};$("#manual-move").value=move.uci;setDecision("review",`${move.san} detected`,"Review the model-supported move or enable automatic recording.","?",move);if($("#auto-record").checked && !document.hidden) recordMove(candidate,true);}
    } else if (visionSession && !pending && decision.kind === "review") {candidate=null;setDecision("review","Position needs review",decision.reason,"!");}
    else if (visionSession && !pending && decision.kind === "unchanged") {candidate=null;$("#manual-move").value="";setDecision("stable","Watching the pieces",`${game.turn} to move.`,"✓");}
    updateControls();
  } catch(error){if(stream?.native){visionSession=null;candidate=null;setDecision("review","Native recorder interrupted",error.message,"!");}}
  finally{nativePolling=false;}
}
