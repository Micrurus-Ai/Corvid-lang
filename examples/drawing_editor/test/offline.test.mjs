import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { runInNewContext } from "node:vm";

test("the scoped worker serves its shell and built Wasm from an offline cache", async () => {
  const base = "http://localhost/sketch/";
  const files = new Map();
  const requests = new Map();
  const cache = {
    async add(request) {
      const path = new URL("../" + request.url.slice(base.length), import.meta.url);
      const bytes = await readFile(path);
      if (path.pathname.endsWith(".wasm")) {
        assert.equal(request.integrity, "sha256-" + createHash("sha256").update(bytes).digest("base64"));
      }
      files.set(request.url, new Response(bytes));
    },
    async addAll(entries) {
      for (const entry of entries) await this.add(entry);
    },
    match: url => files.get(url)?.clone(),
  };
  const self = {
    registration: { scope: base },
    addEventListener: (event, callback) => requests.set(event, callback),
  };
  const code = await readFile(new URL("../offline.js", import.meta.url), "utf8");
  runInNewContext(code, { self, caches: { open: async () => cache }, URL, Request, btoa });
  await new Promise(resolve => requests.get("install")({ waitUntil: resolve }));
  function offline(path, mode) {
    let response;
    requests.get("fetch")({
      request: { url: base + path, method: "GET", mode },
      respondWith: value => { response = value; },
    });
    return response;
  }
  const shell = await offline("", "navigate");
  assert.match(await shell.text(), /<canvas /);
  for (const asset of ["editor.js", "drawing-store.js", "stroke-worker.js", "raster.mjs"]) {
    const module = await offline(asset, "cors");
    assert.ok((await module.text()).length > 0, asset);
  }
  const loader = await offline("target/wasm/stroke.js", "cors");
  const loaderSource = await loader.text();
  const [, relativeWasm] = loaderSource.match(/new URL\('([^']+\.wasm\?[^']+)'/);
  const wasmUrl = new URL(relativeWasm, base + "target/wasm/stroke.js");
  const response = await offline(wasmUrl.href.slice(base.length), "cors");
  const { instance } = await WebAssembly.instantiate(await response.arrayBuffer());
  assert.equal(instance.exports.steps(2n, 3n), 3n);
  assert.equal(offline("other", "navigate"), undefined);
});
