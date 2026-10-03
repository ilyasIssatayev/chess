/* Camera-only change tracker. No piece classifier or calibrated confidence.
   This is intentionally separate from DOM code so actual pixels and temporal
   decisions can be tested deterministically. Legal successors come from Rust. */
const CameraRecorder = (() => {
  const names = Array.from({ length: 64 }, (_, i) => `${"abcdefgh"[i % 8]}${8 - Math.floor(i / 8)}`);
  function median(values) {
    values.sort((a, b) => a - b);
    return values[Math.floor(values.length / 2)];
  }

  function sampleSquares(pixels, width, height, project) {
    return names.map((_, index) => {
      const patch = [];
      const file = index % 8;
      const rank = Math.floor(index / 8);
      for (let y = 0; y < 10; y += 1) {
        for (let x = 0; x < 10; x += 1) {
          // Sample the square interior to reduce grid-edge/corner sensitivity.
          const p = project((file + .12 + x * .076) / 8, (rank + .12 + y * .076) / 8);
          if (!Number.isFinite(p.x) || !Number.isFinite(p.y) || p.x < 0 || p.y < 0 || p.x >= width || p.y >= height) throw new Error("Calibration extends outside the camera image.");
          const offset = (Math.floor(p.y) * width + Math.floor(p.x)) * 4;
          patch.push(pixels[offset], pixels[offset + 1], pixels[offset + 2]);
        }
      }
      return patch;
    });
  }

  function changes(before, after, threshold = 10) {
    if (before.length !== 64 || after.length !== 64) throw new Error("Expected all 64 squares.");
    // Compensate uniform exposure shifts, using the majority unchanged pixels.
    const offsets = [0, 1, 2].map((channel) => median(before.flatMap((patch, square) => patch.filter((_, i) => i % 3 === channel).map((value, i) => after[square][i * 3 + channel] - value))));
    return before.map((patch, square) => {
      let total = 0;
      let different = 0;
      for (let i = 0; i < patch.length; i += 3) {
        const difference = (Math.abs(after[square][i] - patch[i] - offsets[0]) + Math.abs(after[square][i + 1] - patch[i + 1] - offsets[1]) + Math.abs(after[square][i + 2] - patch[i + 2] - offsets[2])) / 3;
        total += difference;
        if (difference > threshold * 1.5) different += 1;
      }
      const score = total / (patch.length / 3);
      return { square: names[square], score, changed: score >= threshold && different / (patch.length / 3) >= .12 };
    });
  }

  function matchMoves(evidence, legalMoves) {
    const changed = evidence.filter((e) => e.changed).map((e) => e.square);
    // Exact set equality is deliberate: shadows, hands, adjustments and moves
    // made while capture was paused must never be absorbed into the baseline.
    const matches = legalMoves.filter((move) => move.changed.length === changed.length && move.changed.every((square) => changed.includes(square)));
    return { changed, matches, kind: changed.length === 0 ? "unchanged" : matches.length === 1 ? "candidate" : "review" };
  }

  class Tracker {
    constructor() { this.reset(); }
    reset() { this.reference = null; this.previous = null; this.lastMotion = 0; this.stableKey = null; this.consistent = 0; }
    setReference(patches, now) { this.reset(); this.reference = patches; this.previous = patches; this.lastMotion = now; }
    ingest(patches, now, legalMoves, threshold = 10) {
      if (!this.reference) return { kind: "unarmed", changed: [], matches: [] };
      const frame = changes(this.previous, patches, threshold / 2);
      this.previous = patches;
      const moving = frame.some((e) => e.changed);
      if (moving) { this.lastMotion = now; this.consistent = 0; this.stableKey = null; }
      if (moving || now - this.lastMotion < 900) return { kind: "moving", changed: [], matches: [] };
      const result = matchMoves(changes(this.reference, patches, threshold), legalMoves);
      const key = `${result.changed.join(",")}:${result.matches.map((m) => m.uci).join(",")}`;
      if (key === this.stableKey) this.consistent += 1;
      else { this.stableKey = key; this.consistent = 1; }
      if (this.consistent < 3) return { kind: "settling", changed: result.changed, matches: [] };
      return { ...result, patches };
    }
  }
  return { names, sampleSquares, changes, matchMoves, Tracker };
})();
