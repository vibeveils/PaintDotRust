// Engine transport. The editor runs either in a Web Worker (the wasm build)
// or, for development, natively behind the dev server's WebSocket. Both speak
// the same protocol: a JSON request plus an optional binary payload in, a
// JSON reply (with `bins: [{name, len}]`) plus concatenated payloads out.
//
// Requests are processed strictly one at a time. Pointer moves and live
// preview updates are coalesced: while one is in flight, newer ones replace
// any queued one of the same kind, so slow operations never build a backlog.

const COALESCE = new Set(['pointer:move', 'fx:preview', 'feather:preview', 'layer_props:preview', 'layer_opacity:live', 'colors:live:fg', 'colors:live:bg', 'shape_translate:move', 'text_preview:undefined']);

function coalesceKey(req) {
  const phase = req.phase ?? (req.cmd === 'layer_opacity' && !req.commit ? 'live' : undefined);
  // Live colour edits coalesce per colour (primary/secondary).
  const k = req.cmd === 'colors' && phase === 'live' ? `colors:live:${'fg' in req ? 'fg' : 'bg'}` : `${req.cmd}:${phase}`;
  return COALESCE.has(k) ? k : null;
}

export class Engine {
  constructor(transport) {
    this.transport = transport;
    this.queue = [];
    this.busy = false;
    this.nextId = 1;
    this.listeners = new Set();
  }

  /** Called with every reply (state snapshots, tiles, status). */
  onReply(fn) { this.listeners.add(fn); return () => this.listeners.delete(fn); }

  /** Sends a request; resolves with `{ json, bins }` where bins maps name → [Uint8Array]. */
  call(cmd, args = {}, binary = null) {
    const req = { cmd, ...args };
    return new Promise((resolve, reject) => {
      const key = coalesceKey(req);
      if (key) {
        const i = this.queue.findIndex(q => q.key === key);
        if (i >= 0) {
          // Superseded: settle the old promise with the new outcome.
          const old = this.queue[i];
          this.queue[i] = { req, binary, key, resolve: v => { old.resolve(v); resolve(v); }, reject: e => { old.reject(e); reject(e); } };
          return;
        }
      }
      this.queue.push({ req, binary, key, resolve, reject });
      this.pump();
    });
  }

  async pump() {
    if (this.busy) return;
    const item = this.queue.shift();
    if (!item) return;
    this.busy = true;
    item.req.id = this.nextId++;
    try {
      const reply = await this.transport.send(item.req, item.binary);
      for (const fn of this.listeners) fn(reply);
      if (reply.json.ok) item.resolve(reply);
      else item.reject(Object.assign(new Error(reply.json.error || 'engine error'), { reply }));
    } catch (e) {
      item.reject(e);
    } finally {
      this.busy = false;
      this.pump();
    }
  }

  get idle() { return !this.busy && this.queue.length === 0; }
}

/** Splits a reply's binary blob into named parts. */
export function splitBins(json, buffer) {
  const bins = new Map();
  let off = 0;
  for (const b of json.bins || []) {
    const part = new Uint8Array(buffer, off, b.len);
    off += b.len;
    if (!bins.has(b.name)) bins.set(b.name, []);
    bins.get(b.name).push(part);
  }
  return bins;
}

function frame(req, binary) {
  const json = new TextEncoder().encode(JSON.stringify(req));
  const bin = binary ? new Uint8Array(binary.buffer ?? binary, binary.byteOffset ?? 0, binary.byteLength) : new Uint8Array(0);
  const out = new Uint8Array(4 + json.length + bin.length);
  new DataView(out.buffer).setUint32(0, json.length, true);
  out.set(json, 4);
  out.set(bin, 4 + json.length);
  return out;
}

function unframe(buf) {
  const n = new DataView(buf).getUint32(0, true);
  const json = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 4, n)));
  const payload = buf.slice(4 + n);
  return { json, bins: splitBins(json, payload) };
}

/** Native dev server over WebSocket. */
export class SocketTransport {
  static async open(url) {
    const ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('engine connection failed')); });
    return new SocketTransport(ws);
  }
  constructor(ws) {
    this.ws = ws;
    this.pending = null;
    ws.onmessage = e => { const p = this.pending; this.pending = null; p?.resolve(unframe(e.data)); };
    ws.onclose = () => { this.pending?.reject(new Error('engine connection closed')); this.pending = null; };
  }
  send(req, binary) {
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      this.ws.send(frame(req, binary));
    });
  }
}

/** WebAssembly engine in a dedicated worker (see ../worker.js). */
export class WorkerTransport {
  static async open(url) {
    const worker = new Worker(url, { type: 'module' });
    const t = new WorkerTransport(worker);
    await new Promise((res, rej) => {
      worker.onmessage = e => (e.data?.ready ? res() : rej(new Error(e.data?.error || 'worker failed')));
      worker.onerror = e => rej(new Error(e.message || 'worker failed to load'));
    });
    worker.onmessage = e => { const p = t.pending; t.pending = null; p?.resolve({ json: e.data.json, bins: splitBins(e.data.json, e.data.bin) }); };
    return t;
  }
  constructor(worker) { this.worker = worker; this.pending = null; }
  send(req, binary) {
    return new Promise((resolve, reject) => {
      this.pending = { resolve, reject };
      const copy = binary ? (binary.byteOffset === 0 && binary.byteLength === binary.buffer.byteLength ? binary.buffer : binary.slice().buffer) : null;
      this.worker.postMessage({ req, bin: copy }, copy ? [copy] : []);
    });
  }
}

/** Picks the transport: `?engine=ws` (dev server) or the wasm worker (default). */
export async function connect() {
  const params = new URLSearchParams(location.search);
  const mode = params.get('engine') ?? (location.port === '8090' ? 'ws' : 'wasm');
  if (mode === 'ws') {
    const url = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws`;
    return { engine: new Engine(await SocketTransport.open(url)), mode: 'native (dev server)' };
  }
  return { engine: new Engine(await WorkerTransport.open(new URL('../worker.js', import.meta.url))), mode: 'WebAssembly' };
}
