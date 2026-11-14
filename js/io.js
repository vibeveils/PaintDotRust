// Files, codecs and clipboard. The engine reads/writes PNG, .openpdn and .pdn
// itself; every other format goes through the browser's own codecs.
import * as D from './dialogs.js';

const OPEN_EXT = ['openpdn', 'pdn', 'png', 'jpg', 'jpeg', 'jfif', 'webp', 'avif', 'gif', 'bmp', 'ico', 'svg', 'tif', 'tiff', 'heic', 'heif', 'jxl'];
export const SAVE_FORMATS = [
  { id: 'openpdn', label: 'PaintDotRust layered image', ext: 'openpdn', mime: 'application/x-openpdn', layered: true },
  { id: 'png', label: 'PNG', ext: 'png', mime: 'image/png' },
  { id: 'jpeg', label: 'JPEG', ext: 'jpg', mime: 'image/jpeg', quality: true },
  { id: 'webp', label: 'WebP', ext: 'webp', mime: 'image/webp', quality: true },
  { id: 'avif', label: 'AVIF', ext: 'avif', mime: 'image/avif', quality: true, probe: true },
  { id: 'bmp', label: 'BMP', ext: 'bmp', mime: 'image/bmp' },
];

export const isSupported = f => OPEN_EXT.includes(f.name.split('.').pop().toLowerCase()) || f.type.startsWith('image/');

export function formatFor(name) {
  const ext = name.split('.').pop().toLowerCase();
  return SAVE_FORMATS.find(f => f.ext === ext || (ext === 'jpeg' && f.id === 'jpeg'))?.id ?? null;
}

// ------------------------------------------------------------------ open

export async function pickOpenFile() {
  if (window.showOpenFilePicker) {
    try {
      const [handle] = await showOpenFilePicker({
        types: [{ description: 'Images', accept: { 'image/*': OPEN_EXT.map(e => `.${e}`) } }],
        excludeAcceptAllOption: false,
      });
      return { file: await handle.getFile(), handle };
    } catch (e) {
      if (e.name === 'AbortError') return null;
    }
  }
  const input = document.getElementById('file-input');
  input.accept = OPEN_EXT.map(e => `.${e}`).join(',') + ',image/*';
  input.value = '';
  return new Promise(resolve => {
    input.onchange = () => resolve(input.files[0] ? { file: input.files[0] } : null);
    input.oncancel = () => resolve(null);
    input.click();
  });
}

/** Decodes any browser-supported image to straight RGBA. */
export async function decodeImage(blob) {
  const bmp = await createImageBitmap(blob, { premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
  const cv = new OffscreenCanvas(bmp.width, bmp.height);
  const ctx = cv.getContext('2d', { willReadFrequently: true });
  ctx.drawImage(bmp, 0, 0);
  const data = ctx.getImageData(0, 0, bmp.width, bmp.height).data;
  bmp.close();
  return { w: cv.width, h: cv.height, rgba: new Uint8Array(data.buffer, data.byteOffset, data.byteLength) };
}

const isNative = bytes => {
  const head = String.fromCharCode(...bytes.subarray(0, 4));
  return head === '\x89PNG' || head === 'PDN3' || head === 'PK\x03\x04';
};

/** Opens a file in the engine: native formats directly, others via the browser decoder. */
export async function openInto(call, file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (!isNative(bytes)) {
    // JPEG, WebP, AVIF, GIF, BMP, ... Decode from the File itself: `bytes` is
    // transferred to the engine worker by `call` and would be detached.
    let img;
    try { img = await decodeImage(file); } catch { throw new Error('this file format isn\'t supported, or the file is damaged'); }
    await call('open_rgba', { name: file.name, w: img.w, h: img.h }, img.rgba);
    return;
  }
  await call('open', { name: file.name }, bytes);
}

/** For "Import from File": native formats go to the engine as bytes, others as RGBA. */
export async function decodeForImport(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());
  if (isNative(bytes)) {
    const head = String.fromCharCode(...bytes.subarray(0, 4));
    // Size check needs dimensions; PNG has them in IHDR, others are read in the engine.
    let w = 0, h = 0;
    if (head === '\x89PNG') { const dv = new DataView(bytes.buffer); w = dv.getUint32(16); h = dv.getUint32(20); }
    return { w, h, bytes };
  }
  const img = await decodeImage(file);
  return { w: img.w, h: img.h, bytes: img.rgba };
}

// ------------------------------------------------------------------ save

async function encodeRaster(rgba, w, h, fmt, quality) {
  if (fmt.id === 'bmp') return encodeBmp(rgba, w, h);
  const cv = new OffscreenCanvas(w, h);
  cv.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength), w, h), 0, 0);
  const blob = await cv.convertToBlob({ type: fmt.mime, quality: quality / 100 });
  if (blob.type !== fmt.mime) throw new Error(`this browser can't encode ${fmt.label}`);
  return new Uint8Array(await blob.arrayBuffer());
}

/** 32-bit BMP (BITMAPV4HEADER with alpha mask). */
function encodeBmp(rgba, w, h) {
  const hdr = 14 + 108, size = hdr + w * h * 4;
  const b = new Uint8Array(size), dv = new DataView(b.buffer);
  b[0] = 0x42; b[1] = 0x4D;
  dv.setUint32(2, size, true); dv.setUint32(10, hdr, true);
  dv.setUint32(14, 108, true); dv.setInt32(18, w, true); dv.setInt32(22, -h, true);
  dv.setUint16(26, 1, true); dv.setUint16(28, 32, true); dv.setUint32(30, 3, true); dv.setUint32(34, w * h * 4, true);
  dv.setUint32(38, 2835, true); dv.setUint32(42, 2835, true);
  dv.setUint32(54, 0x00FF0000, true); dv.setUint32(58, 0x0000FF00, true); dv.setUint32(62, 0x000000FF, true); dv.setUint32(66, 0xFF000000, true);
  dv.setUint32(70, 0x73524742, true); // 'sRGB'
  for (let i = 0, o = hdr; i < w * h * 4; i += 4, o += 4) { b[o] = rgba[i + 2]; b[o + 1] = rgba[i + 1]; b[o + 2] = rgba[i]; b[o + 3] = rgba[i + 3]; }
  return b;
}

function saveOptionsDialog(name, layers, current) {
  let fmt = SAVE_FORMATS.find(f => f.id === current) ?? SAVE_FORMATS[layers > 1 ? 0 : 1];
  let quality = 95;
  let fileName = name.replace(/\.[^.]+$/, '');
  return D.modal('Save As', body => {
    const nameIn = D.el('input', { type: 'text', value: fileName, 'aria-label': 'File name' });
    nameIn.addEventListener('input', () => (fileName = nameIn.value));
    const sel = D.el('select', { 'aria-label': 'Format' }, SAVE_FORMATS.map(f => D.el('option', { value: f.id, selected: f === fmt }, `${f.label} (*.${f.ext})`)));
    const q = D.sliderRow('Quality', 1, 100, quality, 1, v => (quality = v));
    const note = D.el('p', { class: 'hint' });
    const sync = () => {
      q.row.style.display = fmt.quality ? '' : 'none';
      note.textContent = !fmt.layered && layers > 1 ? 'This format stores a single layer: the image will be flattened in the saved file (your layers are kept here).' : '';
    };
    sel.addEventListener('change', () => { fmt = SAVE_FORMATS.find(f => f.id === sel.value); sync(); });
    body.append(D.el('div', { class: 'field' }, D.el('label', {}, 'File name'), nameIn), D.el('div', { class: 'field' }, D.el('label', {}, 'Format'), sel), q.row, note);
    sync();
  }, { onOk: () => (fileName.trim() ? { name: `${fileName.trim()}.${fmt.ext}`, fmt, quality } : undefined), okText: 'Save' });
}

async function bytesFor(call, fmt, quality) {
  if (fmt.id === 'openpdn' || fmt.id === 'png') {
    const r = await call('save', { format: fmt.id });
    return r.bins.get('file')[0];
  }
  const r = await call('export_rgba');
  return encodeRaster(r.bins.get('rgba')[0], r.json.w, r.json.h, fmt, quality);
}

/**
 * Saves the document. With a writable handle and no `saveAs`, writes in place;
 * otherwise asks for name/format (native picker when available, else download).
 * Returns the new file record or null when cancelled.
 */
export async function save(call, state, file, saveAs) {
  if (!saveAs && file?.handle && file.format && (file.format !== 'jpeg' && file.format !== 'webp' && file.format !== 'avif' || file.quality)) {
    const fmt = SAVE_FORMATS.find(f => f.id === file.format);
    const bytes = await bytesFor(call, fmt, file.quality ?? 95);
    const w = await file.handle.createWritable();
    await w.write(bytes);
    await w.close();
    return file;
  }
  const opts = await saveOptionsDialog(file?.name ?? `${state.name === 'Untitled' ? 'Untitled' : state.name}`, state.layers.length, file?.format);
  if (!opts) return null;
  const bytes = await bytesFor(call, opts.fmt, opts.quality);
  let handle = null;
  if (window.showSaveFilePicker) {
    try {
      handle = await showSaveFilePicker({ suggestedName: opts.name, types: [{ description: opts.fmt.label, accept: { [opts.fmt.mime]: [`.${opts.fmt.ext}`] } }] });
    } catch (e) {
      if (e.name === 'AbortError') return null;
      handle = null;
    }
  }
  if (handle) {
    const w = await handle.createWritable();
    await w.write(bytes);
    await w.close();
    addRecent(handle);
  } else {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([bytes], { type: opts.fmt.mime }));
    a.download = opts.name;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 10000);
  }
  return { name: handle?.name ?? opts.name, handle, format: opts.fmt.id, quality: opts.quality };
}

// ------------------------------------------------------------------ clipboard

export async function writeClipboard(w, h, rgba) {
  const cv = new OffscreenCanvas(w, h);
  cv.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, rgba.byteLength), w, h), 0, 0);
  const blob = await cv.convertToBlob({ type: 'image/png' });
  await navigator.clipboard.write([new ClipboardItem({ 'image/png': blob })]);
}

export async function readClipboardImage() {
  const items = await navigator.clipboard.read();
  for (const it of items) {
    const type = it.types.find(t => t.startsWith('image/'));
    if (type) return it.getType(type);
  }
  return null;
}

// ------------------------------------------------------------------ recent files (IndexedDB stores file handles)

let recentCache = [];
function db() {
  return new Promise((res, rej) => {
    const r = indexedDB.open('openpdn', 1);
    r.onupgradeneeded = () => r.result.createObjectStore('recent');
    r.onsuccess = () => res(r.result);
    r.onerror = () => rej(r.error);
  });
}
async function loadRecent() {
  try {
    const d = await db();
    recentCache = await new Promise(res => {
      const g = d.transaction('recent').objectStore('recent').get('list');
      g.onsuccess = () => res(g.result || []);
      g.onerror = () => res([]);
    });
  } catch { recentCache = []; }
}
loadRecent();
export const recent = () => recentCache;
export async function addRecent(handle) {
  if (!handle) return;
  const list = [];
  for (const r of recentCache) if (!(await r.handle.isSameEntry?.(handle))) list.push(r);
  recentCache = [{ name: handle.name, handle }, ...list].slice(0, 10);
  try {
    const d = await db();
    d.transaction('recent', 'readwrite').objectStore('recent').put(recentCache, 'list');
  } catch { /* storage unavailable */ }
}
export async function openRecent(r) {
  if ((await r.handle.queryPermission?.({ mode: 'read' })) !== 'granted' && (await r.handle.requestPermission?.({ mode: 'read' })) !== 'granted') return null;
  return { file: await r.handle.getFile(), handle: r.handle };
}
