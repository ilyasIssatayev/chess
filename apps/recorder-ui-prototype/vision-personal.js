/* Train only small classifier heads; the pinned neural backbone stays frozen.
   All examples and learning stay local. Separate positions, not random crops,
   form the validation split. A reported score is not a general accuracy claim. */
var ChessVisionPersonal = (() => {
  const FORMAT = "personal-heads-v1";
  function normalize(vector, head) {
    if (vector.length !== head.dimension) throw new Error("Example feature dimensions changed.");
    const x = new Float32Array(head.dimension);
    let length = 0;
    for (let d = 0; d < x.length; d += 1) {
      x[d] = Math.max(-5, Math.min(5, (vector[d] - head.mean[d]) / head.scale[d]));
      length += x[d] * x[d];
    }
    length = Math.sqrt(length) || 1;
    for (let d = 0; d < x.length; d += 1) x[d] /= length;
    return x;
  }
  function probabilities(vector, head) {
    const x = normalize(vector, head), scores = new Float64Array(head.classes);
    for (let c = 0; c < head.classes; c += 1) {
      let score = head.bias[c];
      const start = c * head.dimension;
      for (let d = 0; d < head.dimension; d += 1) score += head.weights[start + d] * x[d];
      scores[c] = score;
    }
    return ChessVisionCore.softmax(scores);
  }
  function fit(vectors, labels, classes) {
    if (!vectors.length || vectors.length !== labels.length) throw new Error("Training examples are missing.");
    const dimension = vectors[0].length, n = vectors.length;
    const mean = new Float32Array(dimension), scale = new Float32Array(dimension);
    const counts = new Uint32Array(classes);
    vectors.forEach((x, i) => {
      if (x.length !== dimension || x.some((v) => !Number.isFinite(v)) || !Number.isInteger(labels[i]) || labels[i] < 0 || labels[i] >= classes) throw new Error("Invalid training features or labels.");
      counts[labels[i]] += 1;
      for (let d = 0; d < dimension; d += 1) mean[d] += x[d] / n;
    });
    if (counts.some((count) => count < 2)) throw new Error("Capture at least two visible examples of every piece type and color, plus empty squares.");
    vectors.forEach((x) => { for (let d = 0; d < dimension; d += 1) scale[d] += (x[d] - mean[d]) ** 2 / n; });
    for (let d = 0; d < dimension; d += 1) scale[d] = Math.max(Math.sqrt(scale[d]), .05);
    const head = { dimension, classes, mean, scale, weights: new Float32Array(dimension * classes), bias: new Float32Array(classes) };
    const xs = vectors.map((x) => normalize(x, head));
    const gradient = new Float64Array(head.weights.length), biases = new Float64Array(classes);
    for (let step = 0; step < 180; step += 1) {
      gradient.fill(0); biases.fill(0);
      xs.forEach((x, i) => {
        const scores = new Float64Array(classes);
        for (let c = 0; c < classes; c += 1) {
          let score = head.bias[c], start = c * dimension;
          for (let d = 0; d < dimension; d += 1) score += head.weights[start + d] * x[d];
          scores[c] = score;
        }
        const p = ChessVisionCore.softmax(scores), weight = 1 / (classes * counts[labels[i]]);
        for (let c = 0; c < classes; c += 1) {
          const delta = (p[c] - (labels[i] === c ? 1 : 0)) * weight, start = c * dimension;
          biases[c] += delta;
          for (let d = 0; d < dimension; d += 1) gradient[start + d] += delta * x[d];
        }
      });
      for (let j = 0; j < head.weights.length; j += 1) head.weights[j] -= 3 * (gradient[j] + .0001 * head.weights[j]);
      for (let c = 0; c < classes; c += 1) head.bias[c] -= 3 * biases[c];
    }
    return head;
  }
  function rows(samples) {
    const occupancy = [], pieces = [], occupiedLabels = [], pieceLabels = [];
    samples.forEach((sample) => sample.labels.forEach((label, i) => {
      if (sample.coverage[i] < .9) return;
      occupancy.push(sample.occupancy[i]); occupiedLabels.push(label < 0 ? 0 : 1);
      if (label >= 0) { pieces.push(sample.pieces[i]); pieceLabels.push(label); }
    }));
    return { occupancy, pieces, occupiedLabels, pieceLabels };
  }
  function heads(samples) {
    const data = rows(samples);
    return { occupancy: fit(data.occupancy, data.occupiedLabels, 2), pieces: fit(data.pieces, data.pieceLabels, 12) };
  }
  function train(samples, baseVersion) {
    if (samples.length < 3 || samples.length > 20) throw new Error("Capture 3–20 different confirmed positions before training.");
    const key = samples[0].calibration;
    if (samples.some((s) => s.calibration !== key || s.base_version !== baseVersion || s.labels.length !== 64 || s.occupancy.length !== 64 || s.pieces.length !== 64 || s.coverage.length !== 64)) throw new Error("Examples must use the same camera calibration and model.");
    const positions = [...new Set(samples.map((s) => s.position))];
    if (positions.length < 3) throw new Error("Use at least three different board positions. Repeated photos of one position are insufficient.");
    // The last position is held out in its entirety, including repeated frames.
    const holdout = positions[positions.length - 1];
    const learning = samples.filter((s) => s.position !== holdout), validation = samples.filter((s) => s.position === holdout);
    const provisional = heads(learning);
    let correct = 0, baselineCorrect = 0, count = 0;
    const errors = [];
    validation.forEach((sample) => sample.labels.forEach((label, i) => {
      if (sample.coverage[i] < .9) return;
      const occupied = probabilities(sample.occupancy[i], provisional.occupancy)[1];
      const identity = probabilities(sample.pieces[i], provisional.pieces);
      const values = [1 - occupied, ...identity.map((p) => p * occupied)];
      const predicted = values.indexOf(Math.max(...values)) - 1;
      count += 1; correct += predicted === label ? 1 : 0;
      baselineCorrect += sample.baseline[i] === label ? 1 : 0;
      if (predicted !== label) errors.push(sample.squares[i]);
    }));
    if (!count) throw new Error("The validation position has no fully captured squares.");
    return { format: FORMAT, base_version: baseVersion, calibration: key, ...heads(samples),
      metrics: { correct, baselineCorrect, count, errors: [...new Set(errors)], positions: positions.length, frames: samples.length, trainingFrames: learning.length, validationFrames: validation.length } };
  }
  function validate(model, baseVersion) {
    if (model?.format !== FORMAT || model.base_version !== baseVersion || typeof model.calibration !== "string" || !/^[0-9a-f]{64}$/.test(model.id || "")) throw new Error("Saved personal model is incompatible. Train again.");
    for (const [name, classes] of [["occupancy", 2], ["pieces", 12]]) {
      const h = model[name];
      if (h?.dimension !== 1024 || h.classes !== classes || h.weights?.length !== classes * 1024 || h.bias?.length !== classes || h.mean?.length !== 1024 || h.scale?.length !== 1024 || [h.weights, h.bias, h.mean, h.scale].some((v) => Array.from(v).some((n) => !Number.isFinite(n))) || Array.from(h.scale).some((n) => n <= 0)) throw new Error("Saved personal model is invalid. Train again.");
    }
    return model;
  }
  return { probabilities, fit, train, validate };
})();
if (typeof self !== "undefined") self.ChessVisionPersonal = ChessVisionPersonal;
