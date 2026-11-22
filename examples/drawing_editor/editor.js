import { openDrawing, restoreCells, saveCells } from "./drawing-store.js";

const canvas = document.querySelector("canvas");
const status = document.querySelector('[role="status"]');
const context = canvas.getContext("2d");
const storage = document.querySelector("#storage");
const offline = document.querySelector("#offline");

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
    db = await openDrawing();
    for (const [x, y] of await restoreCells(db)) context.fillRect(x, y, 1, 1);
    storage.textContent = "Drawings save on this device.";
  } catch {
    db?.close();
    db = null;
    storage.textContent = "Storage unavailable. New marks last until reload.";
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
