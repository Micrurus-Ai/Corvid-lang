import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { runInNewContext } from "node:vm";

test("the scoped worker serves its shell and built Wasm from an offline cache", async () => {
  const base = "http://localhost/sketch/";
  const files = new Map();
  const requests = new Map();
  const cache = {
    async addAll(entries) {
      for (const { url } of entries) {
        const path = new URL("../" + url.slice(base.length), import.meta.url);
        files.set(url, await readFile(path));
      }
    },
    match: url => files.get(url),
  };
  const self = {
    registration: { scope: base },
    addEventListener: (event, callback) => requests.set(event, callback),
  };
  const code = await readFile(new URL("../offline.js", import.meta.url), "utf8");
  runInNewContext(code, { self, caches: { open: async () => cache }, URL, Request });
  await new Promise(resolve => requests.get("install")({ waitUntil: resolve }));
  function offline(path, mode) {
    let response;
    requests.get("fetch")({
      request: { url: base + path, method: "GET", mode },
      respondWith: value => { response = value; },
    });
    return response;
  }
  assert.match((await offline("", "navigate")).toString(), /<canvas /);
  for (const asset of ["editor.js", "drawing-store.js", "stroke-worker.js", "raster.mjs",
    "target/wasm/stroke.js"]) {
    assert.ok((await offline(asset, "cors")).length > 0, asset);
  }
  const { instance } = await WebAssembly.instantiate(await offline("target/wasm/stroke.wasm", "cors"));
  assert.equal(instance.exports.steps(2n, 3n), 3n);
  assert.equal(offline("other", "navigate"), undefined);
});
