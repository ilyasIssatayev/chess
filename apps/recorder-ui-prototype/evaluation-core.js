/* Acquisition time and annotations are immutable inputs, never inference clocks. */
var ChessEvaluationCore = (() => {
  const hash = value => typeof value === "string" && /^[a-f0-9]{64}$/i.test(value);
  const orientations = ["a1_near_left", "a1_near_right", "a1_far_right", "a1_far_left"];
  function validateBundle(bundle) {
    if (bundle?.schema_version !== 1 || bundle.kind !== "chess-frame-bundle" || !bundle.dataset_id || !hash(bundle.source_sha256)) throw new Error("Unsupported or incomplete frame bundle.");
    const session = bundle.session, frames = bundle.frames;
    if (!session?.session_id || !["training", "validation", "test"].includes(session.split)) throw new Error("Use development, validation or test data. Qualification needs a frozen evaluator.");
    if (!["host_acquisition", "device_presentation", "media_presentation"].includes(session.timestamp_source)) throw new Error("Capture timestamp source is missing.");
    if (session.media.sha256 && session.media.sha256.toLowerCase() !== bundle.source_sha256.toLowerCase()) throw new Error("Source media hash differs from annotations.");
    if (![session.media.width_px, session.media.height_px].every(n => Number.isInteger(n) && n > 0 && n <= 8192)) throw new Error("Unsupported frame dimensions.");
    if (!Array.isArray(frames) || frames.length < 1 || frames.length > 2000 || frames.length !== session.frames?.length) throw new Error("The bundle must contain every annotated frame, up to 2000.");
    const files = new Set();
    frames.forEach((frame, i) => {
      const expected = session.frames[i];
      if (![frame.frame_index, frame.capture_us].every(n => Number.isSafeInteger(n) && n >= 0) || frame.frame_index !== expected.frame_index || frame.capture_us !== expected.capture_us || !hash(frame.sha256) || !/^frames\/[0-9]{6}\.png$/.test(frame.file) || files.has(frame.file)) throw new Error("Frame indexes, acquisition timestamps or hashes are invalid.");
      if (i && (frame.frame_index <= frames[i - 1].frame_index || frame.capture_us <= frames[i - 1].capture_us)) throw new Error("Frames must increase in both index and acquisition time.");
      files.add(frame.file);
    });
    const points = session.board?.corners;
    const order = [points?.near_left, points?.near_right, points?.far_right, points?.far_left];
    const start = orientations.indexOf(session.board?.orientation);
    if (start < 0 || order.some(p => !p || !Number.isFinite(p.x_px) || !Number.isFinite(p.y_px) || p.x_px < 0 || p.y_px < 0 || p.x_px > session.media.width_px || p.y_px > session.media.height_px)) throw new Error("Invalid calibrated corners or orientation.");
    const corners = {};
    ["a1", "h1", "h8", "a8"].forEach((name, i) => { const p = order[(start + i) % 4]; corners[name] = { x: p.x_px / session.media.width_px, y: p.y_px / session.media.height_px }; });
    ChessVisionCore.geometry(corners, session.media.width_px, session.media.height_px);
    return { bundle, corners };
  }
  function begin(bundle, modelVersion, threshold, captureId) {
    return { schema_version: 1, kind: "chess-observation-trace", dataset_id: bundle.dataset_id, session_id: bundle.session.session_id,
      source_sha256: bundle.source_sha256, model_version: modelVersion, calibration_version: JSON.stringify(bundle.session.board),
      pipeline_version: "offline-browser-v1", motion_threshold: threshold, frames: [], captureId };
  }
  function append(trace, frame, result, moving) {
    if (result.version !== trace.model_version || result.personalized) throw new Error("The model changed during replay. Start a new trace.");
    trace.frames.push({ frame_index: frame.frame_index, image_sha256: frame.sha256, inference_ms: result.latency_ms,
      observation: { session_id: trace.captureId, sequence: frame.frame_index, capture_time: frame.capture_us, moving,
        calibration_version: trace.calibration_version, model_version: result.version, squares: result.squares } });
  }
  function exportTrace(trace) { const { captureId: _id, ...contents } = trace; return contents; }
  return { validateBundle, begin, append, exportTrace };
})();
