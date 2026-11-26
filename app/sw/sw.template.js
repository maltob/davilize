// Service worker for the web version. The build (vite.config.ts) fills in VERSION and PRECACHE.
//
// It stores the whole app, including the DuckDB-Wasm engine and the Rust module, the first time
// the page loads, and then answers from that store: the page opens and works with no network.
// A new build installs beside the old one and waits; the page offers a "Reload" button, so an
// update never swaps files under a report that is open.

const VERSION = "__VERSION__";
const PRECACHE = __PRECACHE__;
const CACHE = `davilize-${VERSION}`;
const scope = self.registration.scope;

self.addEventListener("install", (event) => {
  event.waitUntil(
    (async () => {
      const cache = await caches.open(CACHE);
      // One at a time: the DuckDB engine alone is 34 MB.
      for (const file of PRECACHE) {
        await cache.add(new Request(new URL(file, scope), { cache: "reload" }));
      }
    })(),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      for (const key of await caches.keys()) {
        if (key.startsWith("davilize-") && key !== CACHE) await caches.delete(key);
      }
      await self.clients.claim();
    })(),
  );
});

self.addEventListener("message", (event) => {
  if (event.data && event.data.type === "SKIP_WAITING") self.skipWaiting();
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE);
      const hit =
        (await cache.match(request, { ignoreSearch: true })) ||
        // Any page of the app is the app: serve the cached start page.
        (request.mode === "navigate" ? await cache.match(new URL("index.html", scope)) : undefined);
      return hit || fetch(request);
    })(),
  );
});
