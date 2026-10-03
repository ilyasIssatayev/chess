/* IndexedDB holds typed neural features and classifier heads, never camera photos. */
class PersonalVisionStore {
  constructor() {
    this.ready = new Promise((resolve, reject) => {
      const request = indexedDB.open("chess-camera-personal-v1", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("settings");
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(new Error("Local examples storage is unavailable."));
    });
  }
  async get() {
    const db = await this.ready;
    return new Promise((resolve, reject) => {
      const request = db.transaction("settings").objectStore("settings").get("personal");
      request.onsuccess = () => resolve(request.result || { samples: [], model: null });
      request.onerror = () => reject(new Error("Could not load local piece examples."));
    });
  }
  async set(value) {
    const db = await this.ready;
    return new Promise((resolve, reject) => {
      const transaction = db.transaction("settings", "readwrite");
      transaction.objectStore("settings").put(value, "personal");
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(new Error("Could not save local examples. Check browser storage space."));
      transaction.onabort = () => reject(new Error("Local example save was interrupted."));
    });
  }
}
