let count = 0;
function test(name, action) { action(); count += 1; print(`PASS ${name}`); }
function assert(condition, message = "assertion failed") { if (!condition) throw new Error(message); }
const e4 = { uci: "e2e4", san: "e4", changed: ["e2", "e4"] };
const d4 = { uci: "d2d4", san: "d4", changed: ["d2", "d4"] };
const base = () => CameraRecorder.names.map(() => Array(300).fill(100));
function changed(patches, squares, amount = 60) {
  return patches.map((patch, i) => patch.map((value) => value + (squares.includes(CameraRecorder.names[i]) ? amount : 0)));
}
function settled(tracker, patches, moves = [e4, d4], offset = 0) {
  tracker.ingest(patches, offset + 1200, moves);
  tracker.ingest(patches, offset + 2200, moves);
  tracker.ingest(patches, offset + 2400, moves);
  return tracker.ingest(patches, offset + 2600, moves);
}

test("sample camera pixels in the calibrated board, including e2/e4", () => {
  const width = 640, height = 640;
  const before = new Uint8ClampedArray(width * height * 4).fill(100);
  const after = before.slice();
  for (const square of ["e2", "e4"]) {
    const i = CameraRecorder.names.indexOf(square);
    for (let y = Math.floor(i / 8) * 80 + 20; y < Math.floor(i / 8) * 80 + 60; y += 1) {
      for (let x = (i % 8) * 80 + 20; x < (i % 8) * 80 + 60; x += 1) {
        const offset = (y * width + x) * 4;
        after[offset] = after[offset + 1] = after[offset + 2] = 240;
      }
    }
  }
  const project = (u, v) => ({ x: u * width, y: v * height });
  const result = CameraRecorder.matchMoves(CameraRecorder.changes(CameraRecorder.sampleSquares(before, width, height, project), CameraRecorder.sampleSquares(after, width, height, project)), [e4, d4]);
  assert(result.kind === "candidate" && result.matches[0].uci === "e2e4", "actual pixel changes must propose e4");
});
test("localized motion settles before proposing a real change", () => {
  const tracker = new CameraRecorder.Tracker();
  const reference = base(); tracker.setReference(reference, 0);
  const position = changed(reference, ["e2", "e4"]);
  assert(tracker.ingest(position, 1200, [e4]).kind === "moving");
  assert(tracker.ingest(position, 1600, [e4]).kind === "moving");
  assert(tracker.ingest(position, 2200, [e4]).kind === "settling");
  assert(tracker.ingest(position, 2400, [e4]).kind === "settling");
  const decision = tracker.ingest(position, 2600, [e4]);
  assert(decision.kind === "candidate" && decision.matches[0].uci === "e2e4");
  tracker.setReference(position, 2700);
  for (const now of [4000, 4200, 4400]) tracker.ingest(position, now, [d4]);
  assert(tracker.ingest(position, 4600, [d4]).kind === "unchanged", "accepted reference must not create duplicate moves");
});
test("uniform lighting changes never become moves", () => {
  const reference = base();
  const exposure = reference.map((p) => p.map((value) => value + 35));
  assert(CameraRecorder.matchMoves(CameraRecorder.changes(reference, exposure), [e4]).kind === "unchanged");
  const position = changed(exposure, ["e2", "e4"]);
  assert(CameraRecorder.matchMoves(CameraRecorder.changes(reference, position), [e4]).kind === "candidate");
});
test("hands and extra changed squares request review instead of committing", () => {
  const tracker = new CameraRecorder.Tracker(); const reference = base(); tracker.setReference(reference, 0);
  const obstructed = changed(reference, ["e2", "e3", "e4", "f4"]);
  const result = settled(tracker, obstructed);
  assert(result.kind === "review" && result.matches.length === 0);
  assert(settled(tracker, changed(reference, ["e2", "e4"]), [e4, d4], 3000).kind === "candidate", "abstaining must retain the previous reference");
});
test("piece adjustments and illegal transitions are not accepted", () => {
  const reference = base();
  assert(CameraRecorder.matchMoves(CameraRecorder.changes(reference, changed(reference, ["e2"])), [e4]).kind === "review");
  assert(CameraRecorder.matchMoves(CameraRecorder.changes(reference, changed(reference, ["e2", "e5"])), [e4]).kind === "review");
});
test("captures, castling and en passant compare every affected square", () => {
  const reference = base();
  for (const move of [
    { uci: "e4d5", changed: ["e4", "d5"] },
    { uci: "e1g1", changed: ["e1", "g1", "h1", "f1"] },
    { uci: "e5d6", changed: ["e5", "d6", "d5"] },
  ]) {
    assert(CameraRecorder.matchMoves(CameraRecorder.changes(reference, changed(reference, move.changed)), [move]).kind === "candidate");
    assert(CameraRecorder.matchMoves(CameraRecorder.changes(reference, changed(reference, [move.changed[0]])), [move]).kind === "review");
  }
});
test("promotion type is ambiguous from square changes and needs review", () => {
  const reference = base();
  const promotions = ["q", "r", "b", "n"].map((piece) => ({ uci: `a7a8${piece}`, changed: ["a7", "a8"] }));
  const decision = CameraRecorder.matchMoves(CameraRecorder.changes(reference, changed(reference, ["a7", "a8"])), promotions);
  assert(decision.kind === "review" && decision.matches.length === 4);
});
test("unarmed/reset tracker never proposes moves", () => {
  const tracker = new CameraRecorder.Tracker(); const reference = base();
  assert(tracker.ingest(reference, 0, [e4]).kind === "unarmed");
  tracker.setReference(reference, 0); tracker.reset();
  assert(settled(tracker, changed(reference, ["e2", "e4"])).kind === "unarmed");
});
print(`${count} recorder tests passed`);
