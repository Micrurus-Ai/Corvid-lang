import { openDrawing, restoreCells, saveCells, synchronizeDrawing } from "./drawing-store.js";

const canvas = document.querySelector("canvas");
const status = document.querySelector('[role="status"]');
const context = canvas.getContext("2d");
const storage = document.querySelector("#storage");
const offline = document.querySelector("#offline");
const syncStatus = document.querySelector("#sync-status");
const retry = document.querySelector("#retry-sync");

if ("serviceWorker" in navigator) {
  const showOffline = () => {
    if (navigator.serviceWorker.controller?.scriptURL === new URL("./offline.js", import.meta.url).href) {
      offline.textContent = "Ready for offline reload.";
    }
  };
  navigator.serviceWorker.addEventListener("controllerchange", showOffline);
  showOffline();
  navigator.serviceWorker.register("./offline.js").then(registration => {
    const worker = registration.installing;
    worker?.addEventListener("statechange", () => {
      if (worker.state === "redundant" && !navigator.serviceWorker.controller) {
        offline.textContent = "Offline setup failed. Rebuild the editor and reload online.";
      }
    });
  }).catch(() => { offline.textContent = "Offline reload is unavailable."; });
} else {
  offline.textContent = "Offline reload is unavailable.";
}

function enableDrawing(worker, onSegment) {
  status.textContent = "Drag to draw.";
  let pointer = null;
  let previous;
  function point(event) {
    const rect = canvas.getBoundingClientRect();
    const x = Math.floor((event.clientX - rect.left) * canvas.width / rect.width);
    const y = Math.floor((event.clientY - rect.top) * canvas.height / rect.height);
    return [Math.max(0, Math.min(canvas.width - 1, x)), Math.max(0, Math.min(canvas.height - 1, y))];
  }
  function extend(event) {
    const next = point(event);
    onSegment();
    worker.postMessage([...previous, ...next]);
    previous = next;
  }
  canvas.onpointerdown = event => {
    if (pointer !== null || event.button !== 0) return;
    pointer = event.pointerId;
    canvas.setPointerCapture(pointer);
    previous = point(event);
    extend(event);
  };
  canvas.onpointermove = event => {
    if (event.pointerId === pointer) extend(event);
  };
  canvas.onpointerup = event => {
    if (event.pointerId === pointer) {
      extend(event);
      pointer = null;
    }
  };
  canvas.onlostpointercapture = canvas.onpointercancel = event => {
    if (event.pointerId === pointer) pointer = null;
  };
}

if (typeof Worker === "undefined" || !context) {
  status.textContent = "Drawing requires a browser with canvas and worker support.";
} else {
  let db;
  try {
    db = await openDrawing(() => {
      storage.textContent = "Close older sketchbook tabs to finish upgrading your drawing.";
    });
    for (const [x, y] of await restoreCells(db)) context.fillRect(x, y, 1, 1);
    storage.textContent = "Drawings save on this device.";
  } catch (error) {
    db?.close();
    db = null;
    storage.textContent = error.name === "VersionError"
      ? "A newer editor saved this drawing. Reload the latest editor to continue."
      : "Storage unavailable. New marks last until reload.";
    if (error.name === "VersionError") canvas.inert = true;
  }
  let syncing = false;
  let requested = false;
  async function sync() {
    if (!db) return;
    requested = true;
    if (syncing) return;
    syncing = true;
    try {
      while (requested) {
        requested = false;
        if (!navigator.onLine) {
          syncStatus.textContent = "Offline. Saved edits will sync on reconnect.";
          break;
        }
        await synchronizeDrawing(db, "./sketch.json");
        for (const [x, y] of await restoreCells(db)) context.fillRect(x, y, 1, 1);
        syncStatus.textContent = "Drawing synchronized.";
      }
    } catch (error) {
      syncStatus.textContent = "Sync incomplete. Saved edits remain on this device. " + error.message;
    } finally {
      syncing = false;
      if (requested) void sync();
    }
  }
  if (db) {
    retry.hidden = false;
    retry.onclick = sync;
    window.addEventListener("online", sync);
    window.addEventListener("focus", sync);
    db.addEventListener("versionchange", () => {
      canvas.inert = true;
      retry.hidden = true;
      storage.textContent = "Drawing storage changed. Reload the latest editor before drawing.";
    });
    void sync();
  }
  let pending = 0;
  const worker = new Worker("./stroke-worker.js", { type: "module" });
  worker.onmessage = async ({ data }) => {
    if (data === "ready") {
      enableDrawing(worker, () => {
        if (db) {
          pending += 1;
          if (pending === 1) storage.textContent = "Saving…";
        }
      });
      return;
    }
    for (let i = 0; i < data.length; i += 2) {
      context.fillRect(data[i], data[i + 1], 1, 1);
    }
    if (db) {
      try {
        await saveCells(db, data);
        if (--pending === 0 && db) storage.textContent = "Drawing saved.";
        void sync();
      } catch {
        db?.close();
        db = null;
        storage.textContent = "Save failed. New marks last until reload.";
      }
    }
  };
  worker.onerror = () => {
    canvas.onpointerdown = canvas.onpointermove = canvas.onpointerup = null;
    worker.terminate();
    status.textContent = "The drawing worker could not run. Check the WebAssembly build and reload.";
  };
}
