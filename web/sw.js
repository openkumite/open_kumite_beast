// The web app's service worker (CONTRIBUTING.md, "Web app"). tools/pwa_service_worker.sh fills in VERSION and FILES
// after each `trunk build --release` (a debug build gets one that removes itself). Every file of the build is cached
// when this version installs, fetched past the browser's HTTP cache (index.html and the convert worker keep their
// names across versions, and a stale copy would be pinned under the new version). A start of the app after a deploy
// still runs the old version and installs the new one in the background; the new one takes over when no page of the
// old one is open any more (so the start after that runs it), and then deletes the old version's cache. The game
// data is not here: it lives in the browser's origin-private file system, which this does not touch.
const VERSION = "__VERSION__";
const FILES = __FILES__;
const CACHE = `okbeast-${VERSION}`;

self.addEventListener("install", (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(FILES.map((f) => new Request(f, { cache: "reload" })))));
});

self.addEventListener("activate", (e) => {
  e.waitUntil(
    caches.keys().then((keys) => Promise.all(keys.filter((k) => k.startsWith("okbeast-") && k !== CACHE).map((k) => caches.delete(k)))),
  );
});

// Same-origin GETs from the cache, else the network; a page load (any query, e.g. ?signal=…&room=…) gets index.html.
self.addEventListener("fetch", (e) => {
  const req = e.request;
  if (req.method !== "GET" || new URL(req.url).origin !== self.location.origin) return;
  e.respondWith(
    caches.open(CACHE).then(async (c) => {
      const hit = req.mode === "navigate" ? await c.match("./") : await c.match(req, { ignoreSearch: true });
      return hit || fetch(req);
    }),
  );
});
