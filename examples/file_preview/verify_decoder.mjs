import assert from "node:assert/strict";
import { once } from "node:events";
import { readFile } from "node:fs/promises";
import { Worker } from "node:worker_threads";

// Bridge the two Web Worker message operations used by the compiled decoder.
const decoder = await readFile(new URL("./target/site/decode_file.js", import.meta.url), "utf8");
const worker = new Worker(`
    const { parentPort } = require("node:worker_threads");
    globalThis.self = { postMessage: value => parentPort.postMessage(value) };
    ${decoder}
    parentPort.on("message", data => self.onmessage({ data }));
`, { eval: true });
try {
    const bytes = await new File(["Café"], "note.txt").arrayBuffer();
    const decoded = once(worker, "message", { signal: AbortSignal.timeout(5000) });
    worker.postMessage(bytes, [bytes]);
    assert.equal(bytes.byteLength, 0);
    assert.deepEqual(await decoded, ["Café"]);
    console.log("Decoded UTF-8 in a Node worker and verified sender buffer detachment.");
} finally {
    await worker.terminate();
}
