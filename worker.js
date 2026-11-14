// Hosts the wasm engine off the main thread. Messages: { req, bin } → { json, bin }.
// The package comes from `wasm-pack build crates/openpdn-wasm --target web --out-dir ../../web/pkg`.
import * as pkg from './pkg/openpdn_wasm.js';

let editor;
try {
  await pkg.default();
  // Threads need cross-origin isolation (COOP/COEP); fall back to one thread otherwise.
  if (self.crossOriginIsolated && typeof pkg.initThreadPool === 'function') {
    try { await pkg.initThreadPool(navigator.hardwareConcurrency || 4); } catch { /* single-threaded */ }
  }
  editor = new pkg.WasmEditor();
  self.postMessage({ ready: true });
} catch (e) {
  self.postMessage({ error: String(e?.message || e) });
}

self.onmessage = e => {
  const { req, bin } = e.data;
  const out = editor.handle(JSON.stringify(req), bin ? new Uint8Array(bin) : new Uint8Array(0));
  const json = JSON.parse(out.json);
  const bytes = out.bin; // a fresh Uint8Array copied out of wasm memory
  out.free();
  const buffer = bytes.byteOffset === 0 && bytes.buffer.byteLength === bytes.byteLength ? bytes.buffer : bytes.slice().buffer;
  self.postMessage({ json, bin: buffer }, [buffer]);
};
