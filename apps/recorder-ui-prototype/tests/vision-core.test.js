let count = 0;
function assert(condition, message = "assertion failed") { if (!condition) throw new Error(message); }
function test(name, action) { action(); count += 1; print(`PASS ${name}`); }
const core = ChessVisionCore;
const corners = { a8: { x: .2, y: .2 }, h8: { x: .8, y: .2 }, h1: { x: .8, y: .8 }, a1: { x: .2, y: .8 } };
test("camera orientation maps all four rotations to algebraic squares", () => {
  const names = ["a8", "h8", "h1", "a1"];
  const positions = names.map((n) => corners[n]);
  for (let rotation = 0; rotation < 4; rotation += 1) {
    const c = Object.fromEntries(names.map((name, i) => [name, positions[(i - rotation + 4) % 4]]));
    const geo = core.geometry(c, 1000, 1000);
    assert(geo.squares[0] === names[rotation]);
    assert(new Set(geo.squares).size === 64);
    assert(geo.squares[7] === names[(rotation + 1) % 4]);
    assert(geo.squares[63] === names[(rotation + 2) % 4]);
    assert(geo.squares[56] === names[(rotation + 3) % 4]);
  }
});
test("reflecting the corner labels is rejected", () => {
  let rejected = false;
  try { core.geometry({ ...corners, a8: corners.h8, h8: corners.a8, a1: corners.h1, h1: corners.a1 }, 1000, 1000); } catch (_) { rejected = true; }
  assert(rejected);
});
test("projective warp preserves source pixels and records missing image coverage", () => {
  const rgba = new Uint8Array(20 * 20 * 4);
  for (let y = 0; y < 20; y += 1) for (let x = 0; x < 20; x += 1) { const i = (y * 20 + x) * 4; rgba[i] = x; rgba[i + 1] = y; rgba[i + 2] = 50; }
  const out = core.warp(rgba, 20, 20, (u, v) => ({ x: u * 400, y: v * 400 }), 0, 20);
  assert(out.rgb[(5 * 20 + 8) * 3] === 8 && out.rgb[(5 * 20 + 8) * 3 + 1] === 5);
  assert(out.coverage[5 * 20 + 8] === 1 && out.coverage[19 * 20 + 19] === 0);
});
test("occupancy crop uses the published 100x100 square and interior coverage", () => {
  const rgb = new Uint8Array(500 * 500 * 3).fill(123), coverage = new Uint8Array(500 * 500).fill(1);
  const crop = core.occupancyCrop({ rgb, coverage, size: 500 }, 7, 7);
  assert(crop.rgb.length === 30000 && crop.rgb.every((v) => v === 123)); assert(crop.coverage === 1);
});
test("piece crops match the trained padding and horizontal flip", () => {
  const rgb = new Uint8Array(800 * 800 * 3), coverage = new Uint8Array(800 * 800).fill(1);
  for (let y = 0; y < 800; y += 1) for (let x = 0; x < 800; x += 1) rgb[(y * 800 + x) * 3] = x % 256;
  const warp = { rgb, coverage, size: 800 };
  const left = core.pieceCrop(warp, 0, 0);
  assert(left.rgb.length === 60000 && left.rgb[0] === 249 && left.rgb[3] === 248, "left crop must be flipped");
  const right = core.pieceCrop(warp, 7, 7);
  assert(right.rgb[0] === 0 && right.rgb[(100 * 100) * 3] === 550 % 256, "near crop must be bottom-aligned on black");
  assert(right.coverage === 1);
});
test("ImageNet RGB normalization uses NCHW and keeps batches independent", () => {
  const crop = { rgb: new Uint8Array([255, 0, 128]) };
  const tensor = core.tensor([crop, crop], 1, 1);
  assert(tensor.length === 6);
  assert(Math.abs(tensor[0] - (1 - .485) / .229) < 1e-5);
  assert(Math.abs(tensor[1] - (0 - .456) / .224) < 1e-5);
  assert(Math.abs(tensor[2] - (128 / 255 - .406) / .225) < 1e-5);
  assert(tensor[0] === tensor[3]);
});
test("probabilities map all model classes to the Rust contract without inventing identities", () => {
  for (const [index, name] of core.modelClasses.entries()) {
    const identity = Array(12).fill(0); identity[index] = 1;
    const e = core.evidence(["a8"], [.9], [identity], [1])[0];
    assert(Math.abs(e.piece_probabilities[core.classes.indexOf(name)] - .9) < 1e-8);
    assert(Math.abs(e.empty_probability + e.piece_probabilities.reduce((a, b) => a + b, 0) - 1) < 1e-8);
  }
  const e = core.evidence(["a8"], [.09], [null], [1])[0];
  assert(e.piece_probabilities.every((p) => Math.abs(p - .09 / 12) < 1e-8));
});
test("poor evidence, missing crops and invalid logits cannot certify visibility", () => {
  assert(core.evidence(["a8"], [.8], [Array(12).fill(1 / 12)], [1])[0].visible_probability === 0);
  assert(core.evidence(["a8"], [.01], [null], [.5])[0].visible_probability === 0);
  const p = core.softmax([1000, 1001]); assert(Math.abs(p.reduce((a, b) => a + b, 0) - 1) < 1e-8);
  let rejected = false; try { core.softmax([NaN, 1]); } catch (_) { rejected = true; } assert(rejected);
});
test("shallow geometry reports compressed square footprints without pretending to detect occlusion", () => {
  const shallow = core.viewQuality((u, v) => ({ x: u * 800, y: 200 + v * 80 }));
  assert(shallow.compressed && shallow.min_square_pixels === 10 && shallow.warning.includes("Shallow side"));
  const tiny = core.viewQuality((u, v) => ({ x: u * 800, y: v * 40 }));
  assert(tiny.min_square_pixels === 5 && tiny.warning.includes("Too few"));
  assert(!core.viewQuality((u, v) => ({ x: u * 500, y: v * 500 })).warning);
});
test("personal calibration binds labelled corners and actual model frame dimensions", () => {
  const key = core.calibrationKey(corners, 1200, 800);
  assert(key !== core.calibrationKey(corners, 1200, 720));
  assert(key !== core.calibrationKey({ ...corners, a8: {x:.21,y:.2} }, 1200, 800));
});
print(`${count} neural preprocessing tests passed`);
