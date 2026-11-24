// Bump the version when any shell asset (including generated Wasm) changes.
const prefix = "corvid-sketch:" + self.registration.scope + "|";
const version = prefix + "v2";
const assets = [
  "index.html", "editor.js", "drawing-store.js", "stroke-worker.js", "raster.mjs",
  "target/wasm/stroke.js", "target/wasm/stroke.wasm",
].map(path => new URL(path, self.registration.scope).href);

self.addEventListener("install", event => {
  event.waitUntil(caches.open(version).then(cache =>
    cache.addAll(assets.map(url => new Request(url, { cache: "reload" })))
  ));
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
  if (assets.includes(key)) {
    event.respondWith(caches.open(version).then(async cache =>
      (await cache.match(key)) || fetch(event.request)
    ));
  }
});
