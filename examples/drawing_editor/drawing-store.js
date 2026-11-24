const databaseName = "corvid-sketch:" + new URL(".", import.meta.url).pathname;

function committed(transaction) {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = resolve;
    transaction.onabort = () => reject(transaction.error);
  });
}

export function openDrawing(onBlocked = () => {}) {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(databaseName, 2);
    request.onblocked = onBlocked;
    request.onupgradeneeded = ({ oldVersion }) => {
      const pending = request.result.createObjectStore("pendingCells");
      if (oldVersion === 0) {
        request.result.createObjectStore("cells");
      } else {
        const cursor = request.transaction.objectStore("cells").openKeyCursor();
        cursor.onsuccess = () => {
          if (cursor.result) {
            pending.put(true, cursor.result.key);
            cursor.result.continue();
          }
        };
      }
    };
    request.onsuccess = () => {
      const db = request.result;
      db.onversionchange = () => db.close();
      resolve(db);
    };
    request.onerror = () => reject(request.error);
  });
}

async function cellKeys(db, store) {
  const transaction = db.transaction(store);
  const request = transaction.objectStore(store).getAllKeys();
  await committed(transaction);
  return request.result;
}

export function restoreCells(db) {
  return cellKeys(db, "cells");
}

export function saveCells(db, cells) {
  const transaction = db.transaction(["cells", "pendingCells"], "readwrite");
  const drawing = transaction.objectStore("cells");
  const pending = transaction.objectStore("pendingCells");
  for (let i = 0; i < cells.length; i += 2) {
    const key = [cells[i], cells[i + 1]];
    drawing.put(true, key);
    pending.put(true, key);
  }
  return committed(transaction);
}

// Marks are permanent. Union is safe even when another tab has repainted a
// pending coordinate while the request was in flight.
export async function synchronizeDrawing(db, resource) {
  const pending = await cellKeys(db, "pendingCells");
  for (let attempt = 0; attempt < 3; attempt++) {
    const response = await fetch(resource, { cache: "no-store", redirect: "error" });
    let remoteCells = [];
    const headers = { "Content-Type": "application/json" };
    if (response.status === 404) {
      headers["If-None-Match"] = "*";
    } else {
      if (response.status !== 200) throw new Error("Could not read the shared drawing.");
      const document = await response.json();
      if (document.version !== 1 || !Array.isArray(document.cells) ||
          Object.keys(document).some(key => !["version", "cells"].includes(key)) ||
          !document.cells.every(cell => Array.isArray(cell) && cell.length === 2 &&
            cell.every(Number.isInteger) && cell[0] >= 0 && cell[0] < 96 && cell[1] >= 0 && cell[1] < 64)) {
        throw new Error("Unrecognized shared drawing. Reload with a compatible editor.");
      }
      remoteCells = document.cells;
      const revision = response.headers.get("ETag");
      if (!revision || !/^"[^"]*"$/.test(revision)) throw new Error("Missing shared drawing revision.");
      headers["If-Match"] = revision;
    }
    if (pending.length) {
      const union = new Map();
      for (const cell of [...remoteCells, ...pending]) union.set(cell.join(","), cell);
      const write = await fetch(resource, {
        method: "PUT", headers, redirect: "error",
        body: JSON.stringify({ version: 1, cells: [...union.values()] }),
      });
      if (write.status === 412) continue;
      if (![200, 201, 204].includes(write.status)) throw new Error("Shared drawing write failed.");
    }
    const transaction = db.transaction(["cells", "pendingCells"], "readwrite");
    for (const cell of remoteCells) transaction.objectStore("cells").put(true, cell);
    for (const cell of pending) transaction.objectStore("pendingCells").delete(cell);
    await committed(transaction);
    return;
  }
  throw new Error("The shared drawing changed again. Retry synchronization.");
}
