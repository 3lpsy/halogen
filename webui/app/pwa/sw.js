// Build-stamped shell cache; application sync owns API data and audio.
// Artwork remains available offline in a separate runtime cache.
const CACHE = "halogen-shell-__CACHE_VERSION__";
const SHELL = __PRECACHE__;
const ART = "halogen-art";

function cacheable(response) {
  return response && response.ok && !response.redirected &&
    new URL(response.url).origin === self.location.origin;
}

self.addEventListener("install", (event) => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    await Promise.all(SHELL.map(async (url) => {
      const response = await fetch(url, { cache: "no-cache" });
      if (!cacheable(response)) throw new Error(`precache rejected: ${url}`);
      if (url === "/index.html" &&
          !(response.headers.get("content-type") || "").includes("text/html")) {
        throw new Error("precache rejected: index.html is not HTML");
      }
      await cache.put(url, response);
    }));
  })());
});

self.addEventListener("activate", (event) => {
  event.waitUntil((async () => {
    const names = await caches.keys();
    await Promise.all(names.filter((name) =>
      name.startsWith("halogen-") && name !== CACHE && name !== ART
    ).map((name) => caches.delete(name)));
    await self.clients.claim();
  })());
});

self.addEventListener("message", (event) => {
  if (event.data === "SKIP_WAITING") self.skipWaiting();
});

function isArt(url) {
  return /^\/api\/v1\/(episodes|podcasts)\/\d+\/art(\/small)?$/.test(url.pathname);
}

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") return;
  const url = new URL(request.url);
  if (url.origin !== self.location.origin) return;

  if (isArt(url)) {
    event.respondWith((async () => {
      const cache = await caches.open(ART);
      const cached = await cache.match(request);
      if (cached) return cached;
      try {
        const response = await fetch(request);
        if (cacheable(response)) await cache.put(request, response.clone());
        return response;
      } catch (_) {
        return Response.error();
      }
    })());
    return;
  }
  if (url.pathname === "/healthz" || url.pathname.startsWith("/api/")) return;

  if (request.mode === "navigate") {
    event.respondWith((async () => {
      const cache = await caches.open(CACHE);
      return (await cache.match("/index.html")) || fetch(request);
    })());
    return;
  }

  // Fixed-name JS and WASM must come from the same completed installation.
  if (SHELL.includes(url.pathname)) {
    event.respondWith((async () => {
      const cache = await caches.open(CACHE);
      return (await cache.match(request)) || fetch(request);
    })());
  }
});
