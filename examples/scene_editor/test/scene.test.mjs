import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("scene commands pick transformed tiles and undo moves in edit order", async () => {
  let scene;
  globalThis.self = { postMessage: value => { scene = structuredClone(value); } };
  globalThis.fetch = async url => new Response(await readFile(url), {
    headers: { "Content-Type": "application/wasm" },
  });
  await import("../scene-worker.js");
  const send = data => self.onmessage({ data });
  send({ action: "zoom", change: 10 });
  assert.equal(scene.scale, 4);
  send({ action: "zoom", change: -10 });
  assert.equal(scene.scale, 1);
  send({ action: "zoom", change: 1 });
  send({ action: "pan", x: -200, y: -30 });
  assert.equal(scene.tiles[1].left, -24);
  send({ action: "pick", x: 40, y: 50 });
  assert.equal(scene.selected, -1);
  send({ action: "pick", x: 39, y: 50 });
  assert.equal(scene.selected, 1);
  send({ action: "move", x: 8, y: 0 });
  send({ action: "select", index: 0 });
  send({ action: "move", x: 0, y: 8 });
  assert.deepEqual(scene.tiles.map(tile => [tile.x, tile.y]), [[24, 32], [96, 40]]);
  send({ action: "undo" });
  assert.equal(scene.selected, 0);
  assert.equal(scene.canUndo, true);
  send({ action: "undo" });
  assert.equal(scene.selected, 1);
  assert.equal(scene.canUndo, false);
  assert.deepEqual(scene.tiles.map(tile => [tile.x, tile.y]), [[24, 24], [88, 40]]);
});
