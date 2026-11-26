import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";
import { openDrawing, restoreCells, saveCells, synchronizeDrawing } from "../drawing-store.js";

test("two connections can add overlapping strokes and reopen the drawing", async () => {
  const first = await openDrawing();
  const second = await openDrawing();
  assert.deepEqual(await restoreCells(first), []);
  await Promise.all([
    saveCells(first, Uint16Array.of(0, 0, 1, 1)),
    saveCells(second, Uint16Array.of(1, 1, 95, 63)),
  ]);
  first.close();
  second.close();
  const reopened = await openDrawing();
  assert.deepEqual(await restoreCells(reopened), [[0, 0], [1, 1], [95, 63]]);
  reopened.close();
});

test("migration, stale revisions and a lost acknowledgement preserve every mark", async t => {
  const name = "corvid-sketch:" + new URL("../", import.meta.url).pathname;
  const open = request => new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  await open(indexedDB.deleteDatabase(name));
  const legacy = indexedDB.open(name, 1);
  legacy.onupgradeneeded = () => legacy.result.createObjectStore("cells").put(true, [1, 1]);
  const oldTab = await open(legacy);
  const db = await openDrawing(() => oldTab.close());
  t.after(() => db.close());
  await assert.rejects(open(indexedDB.open(name, 1)), { name: "VersionError" });
  let remote = [[2, 2]];
  let revision = 1;
  let writes = 0;
  t.mock.method(globalThis, "fetch", async (_url, options) => {
    if (options.method !== "PUT") {
      return Response.json({ version: 1, cells: remote }, { headers: { ETag: '"' + revision + '"' } });
    }
    assert.equal(options.headers["If-Match"], '"' + revision + '"');
    writes++;
    revision++;
    if (writes === 1) {
      remote.push([3, 3]);
      return new Response(null, { status: 412 });
    }
    remote = JSON.parse(options.body).cells;
    if (writes === 2) {
      await saveCells(db, Uint16Array.of(4, 4));
      throw new Error("Connection interrupted after write");
    }
    return new Response(null, { status: 204 });
  });
  await assert.rejects(synchronizeDrawing(db, "sketch.json"), /interrupted/);
  await synchronizeDrawing(db, "sketch.json");
  const expected = [[1, 1], [2, 2], [3, 3], [4, 4]];
  assert.deepEqual(remote.sort(), expected);
  assert.deepEqual(await restoreCells(db), expected);
  await synchronizeDrawing(db, "sketch.json");
  assert.equal(writes, 3);
});

test("a newer document format keeps the upload pending until the host is compatible", async t => {
  const db = await openDrawing();
  t.after(() => db.close());
  await saveCells(db, Uint16Array.of(8, 9));
  let compatible = false;
  let uploads = 0;
  t.mock.method(globalThis, "fetch", async (_resource, options) => {
    if (options.method === "PUT") {
      assert.ok(compatible);
      assert.equal(options.headers["If-None-Match"], "*");
      assert.deepEqual(JSON.parse(options.body).cells, [[8, 9]]);
      uploads++;
      return new Response(null, { status: 201 });
    }
    return compatible ? new Response(null, { status: 404 })
      : Response.json({ version: 2, layers: [] }, { headers: { ETag: '"new"' } });
  });
  await assert.rejects(synchronizeDrawing(db, "sketch.json"), /Unrecognized shared drawing/);
  compatible = true;
  await synchronizeDrawing(db, "sketch.json");
  assert.equal(uploads, 1);
});
