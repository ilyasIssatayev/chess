const video = document.querySelector("#video");
const canvas = document.querySelector("#motion-canvas");
const context = canvas.getContext("2d", { willReadFrequently: true });
const overlay = document.querySelector("#board-overlay");
const cameraButton = document.querySelector("#camera-button");
const demoButton = document.querySelector("#demo-button");
const reviewButton = document.querySelector("#review-button");
let stream;
let previousPixels;
let lastMotion = 0;
let animation;
let demoTimers = [];
let moves = [];

for (let rank = 8; rank >= 1; rank -= 1) {
  for (const file of "abcdefgh") {
    const square = document.createElement("div");
    square.className = "square";
    square.dataset.name = `${file}${rank}`;
    overlay.append(square);
  }
}

function setDecision(state, title, copy, icon, candidate) {
  const card = document.querySelector("#decision-card");
  card.dataset.state = state;
  document.querySelector("#decision-title").textContent = title;
  document.querySelector("#decision-copy").textContent = copy;
  document.querySelector("#decision-icon").textContent = icon;
  document.querySelector("#candidate").hidden = !candidate;
  document.querySelectorAll(".square.changed").forEach((square) => square.classList.remove("changed"));
  if (candidate) {
    document.querySelector("#candidate-san").textContent = candidate.san;
    document.querySelector("#candidate-squares").textContent = `${candidate.from} → ${candidate.to}`;
    for (const name of [candidate.from, candidate.to]) {
      document.querySelector(`[data-name="${name}"]`)?.classList.add("changed");
    }
  }
}

function renderMoves() {
  const list = document.querySelector("#moves");
  list.replaceChildren();
  moves.forEach((move, index) => {
    const item = document.createElement("li");
    item.innerHTML = `<span>${index + 1}.</span><strong>${move.san}</strong><time>${move.elapsed}</time>`;
    list.append(item);
  });
  document.querySelector("#empty-moves").hidden = moves.length > 0;
  document.querySelector("#move-count").textContent = `${moves.length} ${moves.length === 1 ? "ply" : "plies"}`;
}

async function toggleCamera() {
  if (stream) {
    stream.getTracks().forEach((track) => track.stop());
    stream = undefined;
    cancelAnimationFrame(animation);
    previousPixels = undefined;
    video.srcObject = null;
    document.querySelector("#empty-camera").hidden = false;
    document.querySelector("#capture-dot").classList.remove("live");
    document.querySelector("#capture-label").textContent = "Camera idle";
    document.querySelector("#camera-detail").textContent = "Preview unavailable";
    cameraButton.textContent = "Start camera";
    return;
  }
  try {
    stream = await navigator.mediaDevices.getUserMedia({ video: { width: 1280, height: 720 }, audio: false });
    video.srcObject = stream;
    document.querySelector("#empty-camera").hidden = true;
    document.querySelector("#capture-dot").classList.add("live");
    document.querySelector("#capture-label").textContent = "Camera live";
    document.querySelector("#camera-detail").textContent = "Local preview · frames are not uploaded";
    document.querySelector("#visibility").textContent = "Overlay ready";
    document.querySelector("#timing").textContent = "Measuring";
    cameraButton.textContent = "Stop camera";
    setDecision("stable", "Board stable", "No move has been detected.", "✓");
    monitorMotion();
  } catch (error) {
    setDecision("review", "Camera unavailable", error.message, "!");
  }
}

function monitorMotion(now = performance.now()) {
  if (!stream) return;
  context.drawImage(video, 0, 0, canvas.width, canvas.height);
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
  let difference = 0;
  if (previousPixels) {
    for (let i = 0; i < pixels.length; i += 16) difference += Math.abs(pixels[i] - previousPixels[i]);
    difference /= pixels.length / 16;
  }
  previousPixels = pixels.slice();
  const moving = difference > 7;
  if (moving) lastMotion = now;
  const badge = document.querySelector("#motion-badge");
  badge.textContent = moving ? "MOTION" : now - lastMotion < 700 ? "SETTLING" : "STABLE";
  badge.classList.toggle("moving", moving || now - lastMotion < 700);
  document.querySelector("#fps").textContent = "live";
  animation = requestAnimationFrame(monitorMotion);
}

function playDemo() {
  demoTimers.forEach(clearTimeout);
  const candidate = { san: "e4", from: "e2", to: "e4" };
  const steps = [
    [0, () => setDecision("moving", "Movement detected", "Waiting until the hand clears and the board settles.", "↻")],
    [1200, () => setDecision("moving", "Board settling", "Comparing the stable position with legal successors.", "…", candidate)],
    [2500, () => setDecision("accepted", "Move recorded", "The visual evidence supports one legal move.", "✓", candidate)],
    [2600, () => { if (!moves.some((move) => move.san === "e4")) { moves.push({ san: "e4", elapsed: "1.24s" }); renderMoves(); } }],
  ];
  demoTimers = steps.map(([delay, action]) => setTimeout(action, delay));
}

function showReview() {
  demoTimers.forEach(clearTimeout);
  setDecision("review", "Move needs review", "Two legal moves fit the visible evidence. Nothing was added to the trusted game.", "!", { san: "?", from: "g1", to: "f3" });
}

cameraButton.addEventListener("click", toggleCamera);
demoButton.addEventListener("click", playDemo);
reviewButton.addEventListener("click", showReview);
renderMoves();
