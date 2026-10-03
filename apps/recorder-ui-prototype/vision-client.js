class VisionClient {
  constructor() {
    this.worker = new Worker("/vision-worker.js", { type: "module" });
    this.requests = new Map(); this.nextId = 0;
    this.canvas = document.createElement("canvas");
    this.context = this.canvas.getContext("2d", { willReadFrequently: true });
    this.worker.onmessage = ({ data }) => {
      const entry = this.requests.get(data.id);
      if (!entry) return;
      clearTimeout(entry.timer); this.requests.delete(data.id);
      if (data.error) entry.reject(new Error(data.error)); else entry.resolve(data.result);
    };
    this.worker.onerror = (event) => this.fail(event.message || "Recognition worker failed to load. Restart the recorder server.");
    this.ready = this.request("load");
  }
  fail(message) {
    this.failed = new Error(message);
    this.worker.terminate();
    for (const entry of this.requests.values()) { clearTimeout(entry.timer); entry.reject(this.failed); }
    this.requests.clear();
  }
  request(type, data = {}, transfer = []) {
    if (this.failed) return Promise.reject(this.failed);
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.fail("Recognition timed out. Reload the recorder and try again."), 30000);
      this.requests.set(id, { resolve, reject, timer });
      this.worker.postMessage({ id, type, ...data }, transfer);
    });
  }
  async infer(source, corners, options = {}) {
    await this.ready;
    const originalWidth = source.videoWidth || source.naturalWidth || source.width;
    const originalHeight = source.videoHeight || source.naturalHeight || source.height;
    if (!originalWidth || !originalHeight) throw new Error("Camera frame is unavailable.");
    this.canvas.width = Math.min(originalWidth, 1200);
    this.canvas.height = Math.round(originalHeight * this.canvas.width / originalWidth);
    this.context.drawImage(source, 0, 0, this.canvas.width, this.canvas.height);
    const rgba = this.context.getImageData(0, 0, this.canvas.width, this.canvas.height).data;
    let labels;
    if (options.sample) {
      const geometry = ChessVisionCore.geometry(corners, this.canvas.width, this.canvas.height);
      labels = geometry.squares.map((square) => ChessVisionCore.modelClasses.indexOf(options.pieces[square]));
    }
    return this.request("infer", { rgba, width: this.canvas.width, height: this.canvas.height, corners, ...options, labels }, [rgba.buffer]);
  }
}
