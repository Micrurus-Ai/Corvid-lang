// Each painted cell is its own key so concurrent tabs can add to the same sketch.
export function openDrawing() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open("corvid-sketch:" + new URL(".", import.meta.url).pathname, 1);
    request.onupgradeneeded = () => request.result.createObjectStore("cells");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export function restoreCells(db) {
  return new Promise((resolve, reject) => {
    const transaction = db.transaction("cells");
    const request = transaction.objectStore("cells").getAllKeys();
    transaction.oncomplete = () => resolve(request.result);
    transaction.onabort = () => reject(transaction.error);
  });
}

export function saveCells(db, cells) {
  return new Promise((resolve, reject) => {
    const transaction = db.transaction("cells", "readwrite");
    const store = transaction.objectStore("cells");
    for (let i = 0; i < cells.length; i += 2) {
      store.put(true, [cells[i], cells[i + 1]]);
    }
    transaction.oncomplete = resolve;
    transaction.onabort = () => reject(transaction.error);
  });
}
