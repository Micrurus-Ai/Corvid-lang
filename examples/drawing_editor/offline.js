// Bump the version when any shell asset (including generated Wasm) changes.
const prefix = "corvid-sketch:" + self.registration.scope + "|";
const version = prefix + "v3";
const loader = new URL("target/wasm/stroke.js", self.registration.scope);
const wasm = new URL("stroke.wasm", loader);
const assets = [
  "index.html", "editor.js", "drawing-store.js", "stroke-worker.js", "raster.mjs",
  "target/wasm/stroke.js",
].map(path => new URL(path, self.registration.scope).href);

self.addEventListener("install", event => {
  event.waitUntil((async () => {
    const cache = await caches.open(version);
    await cache.addAll(assets.map(url => new Request(url, { cache: "reload" })));
    const source = await (await cache.match(loader.href)).text();
    const revision = source.match(/\.\/stroke\.wasm\?v=([a-f0-9]{64})'/);
    if (!revision) throw new Error("Missing Wasm revision in the drawing loader.");
    const hashBytes = revision[1].match(/../g).map(byte => parseInt(byte, 16));
    await cache.add(new Request(wasm.href + "?v=" + revision[1], {
      cache: "reload", integrity: "sha256-" + btoa(String.fromCharCode(...hashBytes)),
    }));
  })());
});

self.addEventListener("activate", event => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) {
      if (key.startsWith(prefix) && key !== version) await caches.delete(key);
    }
    await self.clients.claim();
  })());
});

self.addEventListener("fetch", event => {
  if (event.request.method !== "GET") return;
  const url = new URL(event.request.url);
  const key = event.request.mode === "navigate" && url.pathname === new URL(self.registration.scope).pathname
    ? assets[0] : url.href;
  if (assets.includes(key) || (url.origin === wasm.origin && url.pathname === wasm.pathname)) {
    event.respondWith(caches.open(version).then(async cache =>
      (await cache.match(key)) || fetch(event.request)
    ));
  }
});
