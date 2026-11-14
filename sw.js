// PaintDotRust service worker: makes the installed app work offline and
// receives images shared to it (Android "Share" → PaintDotRust).
//
// Strategy: network first, so an online visit always gets one consistent,
// current set of files (the page, its modules and the wasm engine must match);
// every response is kept, and used when the network is unavailable or slow.

const CACHE = 'paintdotrust-app';
const SHARE_CACHE = 'paintdotrust-shared';
// Everything the app needs to start. Paths are relative to this file, so it
// works from a subpath (GitHub Pages: /<user>.github.io/PaintDotRust/).
const SHELL = [
  './',
  'index.html',
  'manifest.webmanifest',
  'favicon.svg',
  'css/app.css',
  'js/main.js', 'js/a11y.js', 'js/dialogs.js', 'js/engine.js', 'js/icons.js', 'js/io.js',
  'js/palettes.js', 'js/responsive.js', 'js/textedit.js', 'js/view.js',
  'worker.js',
  'pkg/openpdn_wasm.js', 'pkg/openpdn_wasm_bg.wasm',
  'icons/icon.svg', 'icons/icon-192.png', 'icons/icon-512.png',
  'icons/icon-maskable-192.png', 'icons/icon-maskable-512.png', 'icons/apple-touch-icon.png',
];
// How long to wait for the network before using the saved copy.
const NETWORK_TIMEOUT_MS = 4000;

self.addEventListener('install', event => {
  event.waitUntil((async () => {
    const cache = await caches.open(CACHE);
    // One by one, so a missing optional file (e.g. no wasm build on a dev
    // server) doesn't stop the rest; `no-cache` revalidates with the server.
    await Promise.all(SHELL.map(async path => {
      try {
        const res = await fetch(new Request(path, { cache: 'no-cache' }));
        if (res.ok) await cache.put(path, res);
      } catch { /* offline during install: cached on first use instead */ }
    }));
    await self.skipWaiting();
  })());
});

self.addEventListener('activate', event => {
  event.waitUntil((async () => {
    for (const key of await caches.keys()) if (key !== CACHE && key !== SHARE_CACHE) await caches.delete(key);
    await self.clients.claim();
  })());
});

self.addEventListener('fetch', event => {
  const req = event.request;
  const url = new URL(req.url);
  if (url.origin !== location.origin) return;
  if (req.method === 'POST' && url.pathname.endsWith('/share-target')) {
    event.respondWith(receiveShare(req));
    return;
  }
  if (req.method !== 'GET' || url.pathname.endsWith('/ws')) return;
  event.respondWith(networkFirst(event, req));
});

async function networkFirst(event, req) {
  // Keep the worker alive until the copy is saved, even if the saved copy
  // answered first (waitUntil must be called before the response settles).
  let saved = () => {};
  event.waitUntil(new Promise(resolve => (saved = resolve)));
  // Pages are stored under one key: the app is one page, whatever the query (?engine=…, ?action=…).
  const key = req.mode === 'navigate' ? new URL('./', location.href).href : req;
  const network = fetch(req).then(res => {
    if (res.ok && res.type === 'basic') {
      const copy = res.clone();
      caches.open(CACHE).then(c => c.put(key, copy)).catch(() => {}).finally(saved);
    } else saved();
    return res;
  }, err => { saved(); throw err; });
  network.catch(() => {}); // handled below
  const timeout = new Promise(resolve => setTimeout(resolve, NETWORK_TIMEOUT_MS, null));
  try {
    const res = await Promise.race([network, timeout]);
    if (res) return res;
  } catch { /* offline */ }
  const cached = await caches.match(key, { cacheName: CACHE, ignoreSearch: req.mode === 'navigate' });
  if (cached) return cached;
  return network; // nothing saved: wait for the network after all (or fail)
}

/** Files shared to the app: kept for the page to pick up, then open the app. */
async function receiveShare(req) {
  try {
    const form = await req.formData();
    const files = form.getAll('images').filter(f => typeof f !== 'string');
    const cache = await caches.open(SHARE_CACHE);
    await Promise.all(files.map((f, i) => cache.put(new Request(`shared/${Date.now()}-${i}`), new Response(f, {
      headers: { 'Content-Type': f.type || 'application/octet-stream', 'X-File-Name': encodeURIComponent(f.name || `shared-${i + 1}`) },
    }))));
  } catch { /* nothing usable was shared: just open the app */ }
  return Response.redirect(new URL('./?shared=1', location.href).href, 303);
}
