import * as ort from "/vision-assets/ort.wasm.min.mjs";
import "/vision-core.js";
import "/vision-personal.js";
const core = self.ChessVisionCore;
let occupancySession, pieceSession, manifest, personal;
const learner = self.ChessVisionPersonal;
ort.env.wasm.numThreads = 1;
ort.env.wasm.proxy = false;
ort.env.wasm.wasmPaths = "/vision-assets/";

async function personalHash(model) {
  const { id: _id, ...contents } = model;
  const bytes = new TextEncoder().encode(JSON.stringify(contents));
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (n) => n.toString(16).padStart(2, "0")).join("");
}

async function model(entry) {
  const response = await fetch(`/vision-assets/${entry.file}`);
  if (!response.ok) throw new Error(`Missing ${entry.file}. Run python3 scripts/setup-vision.py.`);
  const bytes = await response.arrayBuffer();
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), (n) => n.toString(16).padStart(2, "0")).join("");
  if (hash !== entry.sha256) throw new Error(`Model integrity check failed: ${entry.file}`);
  return ort.InferenceSession.create(bytes, { executionProviders: ["wasm"], graphOptimizationLevel: "all" });
}

async function run(session, crops, width, height, count) {
  const probabilities = [], features = [];
  // Bound activation memory and keep dynamic batches aligned to crop order.
  for (let start = 0; start < crops.length; start += 8) {
    const batch = crops.slice(start, start + 8);
    const input = new ort.Tensor("float32", core.tensor(batch, width, height), [batch.length, 3, height, width]);
    const outputs = await session.run({ input });
    const output = outputs.logits;
    if (!output || output.dims[0] !== batch.length || output.dims[1] !== count) throw new Error("Unexpected model output shape.");
    const embedding = outputs[manifest.features.tensor];
    if (!embedding || embedding.dims[0] !== batch.length || embedding.dims[1] !== 1024) throw new Error("Unexpected neural feature shape.");
    for (let i = 0; i < batch.length; i += 1) {
      probabilities.push(core.softmax(output.data.subarray(i * count, (i + 1) * count)));
      features.push(new Float32Array(embedding.data.subarray(i * 1024, (i + 1) * 1024)));
    }
    input.dispose(); output.dispose(); embedding.dispose();
  }
  return { probabilities, features };
}

self.onmessage = async ({ data }) => {
  const { id, type } = data;
  try {
    if (type === "load") {
      const response = await fetch("/vision-assets/manifest.json");
      if (!response.ok) throw new Error("Model manifest unavailable. Restart the updated recorder server.");
      manifest = await response.json();
      [occupancySession, pieceSession] = await Promise.all(manifest.models.map(model));
      // Execute both graphs, rather than considering parsed graphs sufficient.
      await run(occupancySession, [{ rgb: new Uint8Array(30000) }], 100, 100, 2);
      await run(pieceSession, [{ rgb: new Uint8Array(60000) }], 100, 200, 12);
      self.postMessage({ id, result: { version: manifest.version } });
      return;
    }
    if (!occupancySession || !pieceSession) throw new Error("Piece recognition is still loading.");
    if (type === "personal") {
      const next = data.model ? learner.validate(data.model, manifest.version) : null;
      if (next && await personalHash(next) !== next.id) throw new Error("Saved personal model integrity failed. Train again.");
      personal = next;
      self.postMessage({ id, result: { active: Boolean(personal) } }); return;
    }
    if (type === "train") {
      const trained = learner.train(data.samples, manifest.version);
      trained.id = await personalHash(trained);
      self.postMessage({ id, result: trained }); return;
    }
    const started = performance.now();
    const { rgba, width, height, corners } = data;
    const geometry = core.geometry(corners, width, height);
    const occupancyWarp = core.warp(rgba, width, height, geometry.project, 50, 500);
    const crops = Array.from({ length: 64 }, (_, i) => core.occupancyCrop(occupancyWarp, Math.floor(i / 8), i % 8));
    const outputs = await run(occupancySession, crops, 100, 100, 2);
    const calibration = core.calibrationKey(corners, width, height);
    // A calibrated personal head is never silently applied to a new crop geometry.
    const adapted = personal && personal.calibration === calibration && !data.sample;
    const occupied = outputs.probabilities.map((p, i) => adapted ? learner.probabilities(outputs.features[i], personal.occupancy)[1] : p[1]);
    const indices = Array.from({ length: 64 }, (_, i) => i).filter((i) => data.sample || data.inspect || occupied[i] >= .10);
    const probabilities = Array(64).fill(null), coverage = crops.map((c) => c.coverage), pieceFeatures = Array(64).fill(null);
    const previews = [];
    if (indices.length) {
      const warped = core.warp(rgba, width, height, geometry.project, 200, 800);
      const pieceCrops = indices.map((i) => core.pieceCrop(warped, Math.floor(i / 8), i % 8));
      const pieces = await run(pieceSession, pieceCrops, 100, 200, 12);
      indices.forEach((index, i) => {
        probabilities[index] = adapted ? learner.probabilities(pieces.features[i], personal.pieces) : pieces.probabilities[i];
        pieceFeatures[index] = pieces.features[i];
        coverage[index] = Math.min(coverage[index], pieceCrops[i].coverage);
        if (data.inspect) previews.push({ square: geometry.squares[index], rgb: pieceCrops[i].rgb, occupancy: crops[index].rgb, coverage: coverage[index] });
      });
    }
    const squares = core.evidence(geometry.squares, occupied, probabilities, coverage);
    const result = { version: adapted ? `${manifest.version}:personal:${personal.id}` : manifest.version,
      personalized: Boolean(adapted), calibration, squares, previews, quality: core.viewQuality(geometry.project), latency_ms: Math.round(performance.now() - started) };
    if (data.sample) {
      if (!Array.isArray(data.labels) || data.labels.length !== 64 || data.labels.some((l) => !Number.isInteger(l) || l < -1 || l > 11)) throw new Error("Every sample square needs a confirmed label.");
      result.sample = { base_version: manifest.version, calibration, position: data.position, labels: data.labels,
        squares: geometry.squares, coverage, occupancy: outputs.features, pieces: pieceFeatures,
        baseline: squares.map((s) => { const v = [s.empty_probability, ...s.piece_probabilities]; const best = v.indexOf(Math.max(...v)); return best === 0 ? -1 : core.modelClasses.indexOf(core.classes[best - 1]); }) };
    }
    self.postMessage({ id, result });
  } catch (error) { self.postMessage({ id, error: error.message || String(error) }); }
};
