import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { once } from "node:events";
import { Worker } from "node:worker_threads";
import { rasterize } from "../raster.mjs";

const bytes = await readFile(new URL("../target/wasm/stroke.wasm", import.meta.url));
const { instance } = await WebAssembly.instantiate(bytes);
const line = (...points) => [...rasterize(instance.exports, points)];

test("the module worker loads the generated wrapper and transfers a stroke", async () => {
  // Bridge the browser worker globals and static file fetch to Node's worker thread.
  const entry = new URL("../stroke-worker.js", import.meta.url);
  const bootstrap = `
    import { parentPort } from "node:worker_threads";
    import { readFile } from "node:fs/promises";
    globalThis.self = globalThis;
    globalThis.fetch = async url => new Response(await readFile(url), {
      headers: { "Content-Type": "application/wasm" }
    });
    self.postMessage = (value, transfer) => parentPort.postMessage(value, transfer);
    parentPort.on("message", data => self.onmessage({ data }));
    await import(${JSON.stringify(entry.href)});
  `;
  const worker = new Worker(new URL("data:text/javascript," + encodeURIComponent(bootstrap)));
  try {
    const signal = AbortSignal.timeout(5000);
    assert.deepEqual(await once(worker, "message", { signal }), ["ready"]);
    const reply = once(worker, "message", { signal });
    worker.postMessage([6, 5, 4, 1]);
    const [cells] = await reply;
    assert.ok(cells instanceof Uint16Array);
    assert.deepEqual([...cells], [6, 5, 6, 4, 5, 3, 5, 2, 4, 1]);
  } finally {
    await worker.terminate();
  }
});

test("a tap paints one cell and a shallow stroke rounds to the nearest row", () => {
  assert.deepEqual(line(95, 63, 95, 63), [95, 63]);
  assert.deepEqual(line(0, 0, 4, 2), [0, 0, 1, 1, 2, 1, 3, 2, 4, 2]);
});

test("strokes reach every edge without gaps in either direction", () => {
  for (const x of [0, 48, 95]) {
    for (const y of [0, 32, 63]) {
      const cells = line(48, 32, x, y);
      const reverse = line(x, y, 48, 32);
      assert.equal(cells.length, 2 * (1 + Math.max(Math.abs(x - 48), Math.abs(y - 32))));
      assert.deepEqual(cells.slice(0, 2), [48, 32]);
      assert.deepEqual(cells.slice(-2), [x, y]);
      for (let i = 0; i < cells.length; i += 2) {
        const j = cells.length - 2 - i;
        assert.deepEqual(cells.slice(i, i + 2), reverse.slice(j, j + 2));
        if (i > 0) {
          const dx = Math.abs(cells[i] - cells[i - 2]);
          const dy = Math.abs(cells[i + 1] - cells[i - 1]);
          assert.equal(Math.max(dx, dy), 1);
        }
      }
    }
  }
});
