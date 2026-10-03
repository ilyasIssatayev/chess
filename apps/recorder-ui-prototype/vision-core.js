/* Photo preprocessing adapted from CrispChess/chesscog (MIT), see
   models/NOTICE.md. Runs unchanged in the worker and deterministic tests. */
var ChessVisionCore = (() => {
  const classes = ["white_pawn", "white_knight", "white_bishop", "white_rook", "white_queen", "white_king", "black_pawn", "black_knight", "black_bishop", "black_rook", "black_queen", "black_king"];
  const modelClasses = ["black_bishop", "black_king", "black_knight", "black_pawn", "black_queen", "black_rook", "white_bishop", "white_king", "white_knight", "white_pawn", "white_queen", "white_rook"];
  const cornerNames = ["a8", "h8", "h1", "a1"];
  const mean = [.485, .456, .406], std = [.229, .224, .225];

  function homography(points) {
    const matrix = [], values = [];
    [[0, 0], [1, 0], [1, 1], [0, 1]].forEach(([u, v], i) => {
      const { x, y } = points[i];
      matrix.push([u, v, 1, 0, 0, 0, -x * u, -x * v]); values.push(x);
      matrix.push([0, 0, 0, u, v, 1, -y * u, -y * v]); values.push(y);
    });
    for (let col = 0; col < 8; col += 1) {
      let pivot = col;
      for (let row = col + 1; row < 8; row += 1) if (Math.abs(matrix[row][col]) > Math.abs(matrix[pivot][col])) pivot = row;
      [matrix[col], matrix[pivot]] = [matrix[pivot], matrix[col]];
      [values[col], values[pivot]] = [values[pivot], values[col]];
      const divisor = matrix[col][col];
      if (Math.abs(divisor) < 1e-9) throw new Error("Board corners are degenerate.");
      for (let i = col; i < 8; i += 1) matrix[col][i] /= divisor;
      values[col] /= divisor;
      for (let row = 0; row < 8; row += 1) {
        if (row === col) continue;
        const factor = matrix[row][col];
        for (let i = col; i < 8; i += 1) matrix[row][i] -= factor * matrix[col][i];
        values[row] -= factor * values[col];
      }
    }
    return (u, v) => {
      const z = values[6] * u + values[7] * v + 1;
      return { x: (values[0] * u + values[1] * v + values[2]) / z, y: (values[3] * u + values[4] * v + values[5]) / z };
    };
  }

  function geometry(corners, width, height) {
    const points = cornerNames.map((name) => ({ name, x: corners[name].x * width, y: corners[name].y * height }));
    if (points.some((p) => !Number.isFinite(p.x) || !Number.isFinite(p.y))) throw new Error("Invalid board corners.");
    points.sort((a, b) => a.y - b.y);
    const top = points.slice(0, 2).sort((a, b) => a.x - b.x);
    const bottom = points.slice(2).sort((a, b) => b.x - a.x);
    const order = [...top, ...bottom];
    const turns = order.map((a, i) => {
      const b = order[(i + 1) % 4], c = order[(i + 2) % 4];
      return (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
    });
    if (!turns.every((v) => v > 1e-6)) throw new Error("Board corners must form a convex, uncrossed board.");
    const rotation = cornerNames.indexOf(order[0].name);
    if (!order.every((point, i) => point.name === cornerNames[(rotation + i) % 4])) throw new Error("Check the corner labels: the board orientation is reflected.");
    const squares = Array.from({ length: 64 }, (_, i) => {
      const row = Math.floor(i / 8), col = i % 8;
      const [file, rank] = rotation === 0 ? [col, row] : rotation === 1 ? [7 - row, col] : rotation === 2 ? [7 - col, 7 - row] : [row, 7 - col];
      return `${"abcdefgh"[file]}${8 - rank}`;
    });
    return { project: homography(order), squares, rotation };
  }

  function calibrationKey(corners, width, height) {
    return JSON.stringify([width, height, ...cornerNames.map((name) => [corners[name].x, corners[name].y])]);
  }

  function viewQuality(project) {
    let minHeight = Infinity, minWidth = Infinity;
    for (let row = 0; row < 8; row += 1) for (let col = 0; col < 8; col += 1) {
      const center = project((col + .5) / 8, (row + .5) / 8);
      const up = project((col + .5) / 8, row / 8), down = project((col + .5) / 8, (row + 1) / 8);
      const left = project(col / 8, (row + .5) / 8), right = project((col + 1) / 8, (row + .5) / 8);
      const distance = (a, b) => Math.hypot(a.x - b.x, a.y - b.y);
      if (!Number.isFinite(center.x) || !Number.isFinite(center.y)) throw new Error("Board projection is invalid.");
      minHeight = Math.min(minHeight, distance(up, down));
      minWidth = Math.min(minWidth, distance(left, right));
    }
    const ratio = Math.min(minHeight, minWidth) / Math.max(minHeight, minWidth);
    return { min_square_pixels: Math.round(Math.min(minHeight, minWidth)), compressed: ratio < .35,
      warning: Math.min(minHeight, minWidth) < 8 ? "Too few camera pixels per square. Move closer or raise the camera." : ratio < .35 ? "Shallow side view: neighboring pieces may block the crops. Raise the camera until every piece base is visible." : "" };
  }

  function warp(rgba, width, height, project, margin, size) {
    const rgb = new Uint8Array(size * size * 3);
    const coverage = new Uint8Array(size * size);
    for (let y = 0; y < size; y += 1) {
      for (let x = 0; x < size; x += 1) {
        const p = project((x - margin) / 400, (y - margin) / 400);
        if (!Number.isFinite(p.x) || !Number.isFinite(p.y) || p.x < 0 || p.y < 0 || p.x >= width - 1 || p.y >= height - 1) continue;
        const x0 = Math.floor(p.x), y0 = Math.floor(p.y), dx = p.x - x0, dy = p.y - y0;
        const o = (y * size + x) * 3, s = (y0 * width + x0) * 4;
        for (let c = 0; c < 3; c += 1) {
          rgb[o + c] = Math.round(rgba[s + c] * (1 - dx) * (1 - dy) + rgba[s + 4 + c] * dx * (1 - dy) + rgba[s + width * 4 + c] * (1 - dx) * dy + rgba[s + (width + 1) * 4 + c] * dx * dy);
        }
        coverage[y * size + x] = 1;
      }
    }
    return { rgb, coverage, size };
  }

  function occupancyCrop(warped, row, col) {
    const rgb = new Uint8Array(100 * 100 * 3);
    const x0 = Math.trunc(50 * (col + .5)), y0 = Math.trunc(50 * (row + .5));
    let visible = 0;
    for (let y = 0; y < 100; y += 1) {
      const offset = ((y0 + y) * warped.size + x0) * 3;
      rgb.set(warped.rgb.subarray(offset, offset + 300), y * 300);
      if (y >= 25 && y < 75) for (let x = 25; x < 75; x += 1) visible += warped.coverage[(y0 + y) * warped.size + x0 + x];
    }
    return { rgb, coverage: visible / 2500 };
  }

  function pieceCrop(warped, row, col) {
    const heightIncrease = 1 + 2 * (7 - row) / 7;
    const left = col >= 4 ? 0 : .25 + .75 * (3 - col) / 3;
    const right = col < 4 ? 0 : .25 + .75 * (col - 4) / 3;
    const x1 = Math.trunc(200 + 50 * (col - left)), x2 = Math.trunc(200 + 50 * (col + 1 + right));
    const y1 = Math.trunc(200 + 50 * (row - heightIncrease)), y2 = Math.trunc(200 + 50 * (row + 1));
    const rgb = new Uint8Array(100 * 200 * 3);
    let visible = 0;
    for (let y = 0; y < y2 - y1; y += 1) {
      for (let x = 0; x < x2 - x1; x += 1) {
        const sx = col < 4 ? x2 - 1 - x : x1 + x;
        const source = ((y1 + y) * warped.size + sx) * 3;
        const target = ((200 - (y2 - y1) + y) * 100 + x) * 3;
        rgb.set(warped.rgb.subarray(source, source + 3), target);
        visible += warped.coverage[(y1 + y) * warped.size + sx];
      }
    }
    return { rgb, coverage: visible / ((x2 - x1) * (y2 - y1)) };
  }

  function tensor(crops, width, height) {
    const n = width * height, out = new Float32Array(crops.length * 3 * n);
    crops.forEach((crop, batch) => {
      for (let c = 0; c < 3; c += 1) for (let i = 0; i < n; i += 1) out[batch * 3 * n + c * n + i] = (crop.rgb[i * 3 + c] / 255 - mean[c]) / std[c];
    });
    return out;
  }

  function softmax(logits) {
    if (logits.some((n) => !Number.isFinite(n))) throw new Error("Model produced invalid logits.");
    const peak = Math.max(...logits), values = Array.from(logits, (n) => Math.exp(n - peak));
    const sum = values.reduce((a, b) => a + b, 0);
    return values.map((v) => v / sum);
  }

  function evidence(squares, occupancy, pieceProbabilities, coverage) {
    return squares.map((square, i) => {
      const occupied = occupancy[i], conditional = pieceProbabilities[i] || Array(12).fill(1 / 12);
      const probabilities = Array(12).fill(0);
      modelClasses.forEach((piece, index) => { probabilities[classes.indexOf(piece)] = occupied * conditional[index]; });
      const empty = 1 - occupied;
      const confidence = Math.max(empty, ...probabilities);
      // This is an evidence-quality gate, not a learned occlusion detector.
      const visible = coverage[i] >= .75 && confidence >= .45;
      return { square, visible_probability: visible ? 1 : 0, empty_probability: empty, piece_probabilities: probabilities };
    });
  }
  return { classes, modelClasses, homography, geometry, calibrationKey, viewQuality, warp, occupancyCrop, pieceCrop, tensor, softmax, evidence };
})();
if (typeof self !== "undefined") self.ChessVisionCore = ChessVisionCore;
