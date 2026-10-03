const $ = (selector) => document.querySelector(selector);
const video = $("#video");
const canvas = $("#motion-canvas");
const context = canvas.getContext("2d", { willReadFrequently: true });
const viewport = $("#viewport");
const overlay = $("#board-overlay");
const cameraButton = $("#camera-button");
const calibrateButton = $("#calibrate-button");
const resetCalibrationButton = $("#reset-calibration-button");
const tracker = new CameraRecorder.Tracker();
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
  return { valid: true, label: horizontalRatio < 0.45 ? "Strong perspective · check occlusion" : "Tilted view calibrated" };
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
  candidate = null;
  $("#position-confirm").checked = false;
  $("#recording-label").textContent = "Recording paused";
  if (game) setDecision("idle", "Set a reference position", copy);
  updateControls();
}

function updateControls() {
  $("#reference-button").disabled = !game || !stream || calibrationEditing || !calibrationSaved || !latestPatches || performance.now() - lastFrameAt > 2000 || pending || !$("#position-confirm").checked;
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
  pending = true; updateControls();
  try {
    game = await api("/api/game/move", { ...positionRequest(), uci: move.uci, automatic });
    renderGame();
    candidate = null;
    if (patches && tracker.reference && stream && !document.hidden && !CameraRecorder.changes(patches, latestPatches).some((e) => e.changed)) {
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
    if (moving || !latestPatches) liveStableSince = now;
    latestPatches = patches;
    const badge = $("#motion-badge");
    const settling = now - liveStableSince < 900;
    badge.textContent = moving ? "MOTION" : settling ? "SETTLING" : "STABLE";
    badge.classList.toggle("moving", Boolean(moving || settling));
    $("#timing").textContent = "Preview active";
    updateControls();
    if (!game || calibrationEditing || pending || !tracker.reference) return;
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
      if ($("#auto-record").checked) recordMove(candidate, true, result.patches);
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

$("#reference-button").addEventListener("click", () => {
  if (!game || !stream || pending || !latestPatches || !$("#position-confirm").checked) return;
  const quality = calibrationQuality([corners.a8, corners.h8, corners.h1, corners.a1].map((p) => ({ x: p.x * viewport.clientWidth, y: p.y * viewport.clientHeight })), viewport.clientWidth, viewport.clientHeight);
  if (!quality.valid) { setDecision("review", "Check calibration", quality.label, "!"); return; }
  if (performance.now() - liveStableSince < 900) { setDecision("moving", "Wait for a stable board", "Clear your hands, then set the reference again.", "↻"); return; }
  tracker.setReference(latestPatches, performance.now());
  $("#recording-label").textContent = "Watching calibrated board";
  $("#manual-move").value = "";
  setDecision("stable", "Watching for a move", `${game.turn} to move. Move one piece, then let the board settle.`, "✓");
  updateControls();
});

$("#position-confirm").addEventListener("change", updateControls);
$("#sensitivity").addEventListener("change", () => disarm("Detection sensitivity changed. Set a fresh reference."));
$("#record-button").addEventListener("click", () => {
  const uci = $("#manual-move").value;
  recordMove(game?.legal_moves.find((move) => move.uci === uci), false, candidate?.uci === uci ? candidate.patches : undefined);
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
