// Halogen service worker — installable + cold-launchable offline. dx content-
// hashes asset names, so no static precache; per-fetch instead: navigations +
// static assets stale-while-revalidate, artwork cache-first, API/audio BYPASSED
// (the sync layer owns that data). _sync-dist rewrites CACHE per build.
const CACHE = "halogen-v2";

self.addEventListener("install", (event) => {
  // Precache only the app shell entry; everything else fills in on first use.
  event.waitUntil(
    caches.open(CACHE).then((cache) => cache.add("/")).catch(() => {}),
  );
  self.skipWaiting();
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    (async () => {
      const keys = await caches.keys();
      await Promise.all(keys.filter((k) => k !== CACHE).map((k) => caches.delete(k)));
      await self.clients.claim();
    })(),
  );
});

// Artwork, BOTH resolutions: `/art` (full-size) + `/art/small` (the list
// thumbnail). Matching only `/art` let thumbnails fall through to the `/api/`
// bypass — offline, the list pages lost all art while player art stayed.
function isArt(url) {
  return /^\/api\/v1\/(episodes|podcasts)\/\d+\/art(\/small)?$/.test(url.pathname);
}

function isBypassed(url) {
  if (isArt(url)) return false; // artwork gets its own cache-first handling below
  // The reachability probe must ALWAYS hit the network: a cached 200 here
  // makes the app think it's online while offline (server-setup checks,
  // connection probes), defeating the probe's entire purpose.
  if (url.pathname === "/healthz") return true;
  // Never cache the rest of the API or audio streams — they're large/auth'd/
  // range-served and the app handles their offline behavior itself.
  return url.pathname.startsWith("/api/") || /\/episodes\/\d+\/audio/.test(url.pathname);
}

// Only a plausible app shell may be stored as "/": same-origin, non-redirected
// HTML 200. Without this gate a captive portal's login page (redirect followed,
// so `ok` is true) became the shell at every launch until the cache was purged.
function isShellResponse(resp) {
  if (!resp || !resp.ok || resp.redirected) return false;
  try {
    if (new URL(resp.url).origin !== self.location.origin) return false;
  } catch (_) {
    return false;
  }
  return (resp.headers.get("content-type") || "").includes("text/html");
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;

  const url = new URL(req.url);
  if (url.origin !== self.location.origin) return; // cross-origin: let the network handle it
  if (isBypassed(url)) return;

  // App shell: stale-while-revalidate. Return the cached "/" shell immediately
  // (instant load even on a degraded link) and refresh it in the background; on
  // a cold cache fall through to the network.
  if (req.mode === "navigate") {
    event.respondWith(
      (async () => {
        const cache = await caches.open(CACHE);
        const cached = await cache.match("/");
        const network = fetch(req)
          .then((resp) => {
            if (isShellResponse(resp)) cache.put("/", resp.clone());
            return resp;
          })
          // Cold cache + offline: this is what respondWith gets, so it MUST
          // resolve to a Response (see the static-asset branch below).
          .catch(() => cached || Response.error());
        return cached || network;
      })(),
    );
    return;
  }

  // Artwork: cache-first. Only 200s are stored, so a 404/401 (placeholder or
  // expired media cookie) is retried on the next request instead of sticking.
  if (isArt(url)) {
    event.respondWith(
      (async () => {
        const cache = await caches.open(CACHE);
        const cached = await cache.match(req);
        if (cached) return cached;
        try {
          const resp = await fetch(req);
          if (resp && resp.ok) cache.put(req, resp.clone());
          return resp;
        } catch (_) {
          return Response.error();
        }
      })(),
    );
    return;
  }

  // Static assets: stale-while-revalidate.
  event.respondWith(
    (async () => {
      const cache = await caches.open(CACHE);
      const cached = await cache.match(req);
      const network = fetch(req)
        .then((resp) => {
          if (resp && resp.ok) cache.put(req, resp.clone());
          return resp;
        })
        // On a cache miss + network error this branch is what `respondWith`
        // gets, so it MUST resolve to a Response — `cached` is `undefined` here,
        // and resolving the handler to `undefined` throws
        // "Failed to convert value to 'Response'". Fall back to `Response.error()`.
        .catch(() => cached || Response.error());
      return cached || network;
    })(),
  );
});
