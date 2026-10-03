let count = 0;
function assert(condition, message = "assertion failed") { if (!condition) throw new Error(message); }
function test(name, action) { action(); count += 1; print(`PASS ${name}`); }
const personal = ChessVisionPersonal;
function rejects(action, pattern) { try { action(); } catch (e) { assert(pattern.test(e.message), e.message); return; } throw new Error("Expected rejection"); }
function vector(label, dimensions = 16) { const x = new Float32Array(dimensions); x[label + 1] = 10; return x; }
function sample(position, corrupt = false) {
  const labels = Array.from({ length: 64 }, (_, i) => i < 32 ? i % 12 : -1);
  return { base_version: "test", calibration: "corners", position, labels, coverage: Array(64).fill(1), squares: Array.from({ length: 64 }, (_, i) => `square-${i}`), baseline: labels.map(() => -1),
    occupancy: labels.map((label) => vector(label < 0 ? 0 : 1)), pieces: labels.map((label) => vector(corrupt && label >= 0 ? (label + 1) % 12 : label)) };
}
test("personal head learns separable features, returns normalized probabilities, and handles class balance", () => {
  const xs = Array.from({ length: 12 }, (_, label) => [vector(label), vector(label)]).flat();
  const labels = Array.from({ length: 12 }, (_, label) => [label, label]).flat();
  const head = personal.fit(xs, labels, 12);
  for (let label = 0; label < 12; label += 1) {
    const p = personal.probabilities(vector(label), head);
    assert(p.indexOf(Math.max(...p)) === label && p[label] > .45);
    assert(Math.abs(p.reduce((a, b) => a + b, 0) - 1) < 1e-6);
  }
});
test("missing classes and malformed features are rejected instead of teaching a fabricated identity", () => {
  rejects(() => personal.fit([vector(0), vector(0)], [0, 0], 12), /every piece/);
  rejects(() => personal.fit([new Float32Array([NaN]), new Float32Array([1])], [0, 1], 2), /Invalid/);
});
test("validation excludes every frame from the held-out position before fitting final heads", () => {
  const trained = personal.train([sample("start"), sample("e4"), sample("e4-e5", true), sample("e4-e5", true)], "test");
  assert(trained.metrics.trainingFrames === 2 && trained.metrics.validationFrames === 2);
  assert(trained.metrics.count === 128 && trained.metrics.correct < 128, "Corrupt holdout must expose failures, not leak its labels into validation");
  assert(trained.metrics.baselineCorrect === 64);
});
test("different camera geometry, repeated positions, and insufficient data do not qualify for training", () => {
  rejects(() => personal.train([sample("start")], "test"), /3–20/);
  rejects(() => personal.train([sample("start"), sample("start"), sample("start")], "test"), /three different/);
  const changed = sample("e4-e5"); changed.calibration = "new corners";
  rejects(() => personal.train([sample("start"), sample("e4"), changed], "test"), /same camera/);
});
test("saved heads require exact version, bounded shape and finite parameters", () => {
  const head = { dimension: 1024, classes: 2, weights: new Float32Array(2048), bias: new Float32Array(2), mean: new Float32Array(1024), scale: new Float32Array(1024).fill(1) };
  const model = { format: "personal-heads-v1", id: "a".repeat(64), base_version: "test", calibration: "corners", occupancy: head, pieces: { ...head, classes: 12, weights: new Float32Array(12288), bias: new Float32Array(12) } };
  assert(personal.validate(model, "test") === model);
  rejects(() => personal.validate(model, "different"), /incompatible/);
  model.pieces.weights[0] = NaN;
  rejects(() => personal.validate(model, "test"), /invalid/);
});
print(`${count} personal recognition tests passed`);
