import assert from "node:assert/strict";
import test from "node:test";
import "fake-indexeddb/auto";
import { openDrawing, restoreCells, saveCells } from "../drawing-store.js";

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
