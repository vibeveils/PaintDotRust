// PaintDotRust web UI. All image work happens in the engine (wasm worker or the
// native dev server); this module owns windows, menus, input and file I/O.
import { connect } from './engine.js';
import { CanvasView } from './view.js';
import { icon, glyph } from './icons.js';
import * as D from './dialogs.js';
import * as io from './io.js';
import * as A11y from './a11y.js';
import * as Pal from './palettes.js';
import * as TE from './textedit.js';

const $ = id => document.getElementById(id);
const el = D.el;

// Tools in Paint.NET's Tools window order. `engine` = engine tool name; UI-only tools have none.
const TOOLS = [
  { id: 'rect-select', label: 'Rectangle Select', engine: 'Rect Select', icon: 'tool.rect-select', key: 'S' },
  { id: 'move', label: 'Move Selected Pixels', engine: 'Move', icon: 'tool.move', key: 'M' },
  { id: 'lasso', label: 'Lasso Select', engine: 'Lasso', icon: 'tool.lasso', key: 'S' },
  { id: 'move-selection', label: 'Move Selection', engine: 'Move Selection', icon: 'tool.move-selection', key: 'M' },
  { id: 'ellipse-select', label: 'Ellipse Select', engine: 'Ellipse Select', icon: 'tool.ellipse-select', key: 'S' },
  { id: 'zoom', label: 'Zoom', icon: 'tool.zoom', key: 'Z' },
  { id: 'magic-wand', label: 'Magic Wand', engine: 'Magic Wand', icon: 'tool.magic-wand', key: 'S' },
  { id: 'pan', label: 'Pan', icon: 'tool.pan', key: 'H' },
  { id: 'fill', label: 'Paint Bucket', engine: 'Fill', icon: 'tool.fill', key: 'F' },
  { id: 'gradient', label: 'Gradient', engine: 'Gradient', icon: 'tool.gradient', key: 'G' },
  { id: 'brush', label: 'Paintbrush', engine: 'Brush', icon: 'tool.brush', key: 'B', cursor: 'brush' },
  { id: 'eraser', label: 'Eraser', engine: 'Eraser', icon: 'tool.eraser', key: 'E', cursor: 'brush' },
  { id: 'pencil', label: 'Pencil', engine: 'Pencil', icon: 'tool.pencil', key: 'P', cursor: 'pencil' },
  { id: 'color-picker', label: 'Color Picker', engine: 'Eyedropper', icon: 'tool.color-picker', key: 'K' },
  { id: 'clone-stamp', label: 'Clone Stamp', engine: 'Clone Stamp', icon: 'tool.clone-stamp', key: 'L', cursor: 'brush' },
  { id: 'recolor', label: 'Recolor', engine: 'Recolor', icon: 'tool.recolor', key: 'R', cursor: 'brush' },
  { id: 'distort', label: 'Distort Brush', engine: 'Distort', icon: 'tool.distort', key: 'U', cursor: 'brush' },
  { id: 'perspective', label: 'Perspective', engine: 'Perspective', icon: 'tool.perspective', key: 'M' },
  { id: 'text', label: 'Text', icon: 'tool.text', key: 'T' },
  { id: 'line', label: 'Line / Curve', engine: 'Line', icon: 'tool.line', key: 'O' },
  { id: 'shape', label: 'Shape', engine: 'Shape', icon: 'tool.rectangle', key: 'O' },
  { id: 'freeform', label: 'Freeform Shape', engine: 'Freeform Shape', icon: 'tool.freeform', key: 'O' },
];
// Repeated hotkey presses cycle in this order (legacy).
const HOTKEY_ORDER = {
  S: ['rect-select', 'lasso', 'ellipse-select', 'magic-wand'], M: ['move', 'move-selection', 'perspective'],
  O: ['line', 'shape', 'freeform'],
};
const TOOL_HINTS = {
  'rect-select': 'Drag to select a rectangle (Shift: square). Ctrl+drag adds, Alt+drag subtracts; with the right button, Ctrl inverts (xor) and Alt intersects.',
  move: 'Drag to move the selected pixels; drag a handle to resize. Shift keeps the aspect ratio.',
  lasso: 'Drag to draw a freeform selection. Ctrl adds, Alt subtracts; right button: Ctrl inverts (xor), Alt intersects.',
  'move-selection': 'Drag to move the selection outline; drag a handle to resize it.',
  'ellipse-select': 'Drag to select an ellipse (Shift: circle). Ctrl adds, Alt subtracts; right button: Ctrl inverts (xor), Alt intersects.',
  zoom: 'Click to zoom in, right-click to zoom out, drag to zoom to a rectangle.',
  'magic-wand': 'Click to select an area of similar color. Ctrl+click adds, Alt+click subtracts; right-click: Ctrl inverts (xor), Alt intersects.',
  pan: 'Drag to scroll the canvas. Hold Space with any tool to pan.',
  fill: 'Click to fill a region of similar color. Ctrl toggles sampling the whole image.',
  gradient: 'Drag to draw a gradient (Shift: 15° steps). It stays editable: drag its ends, click the line to add a color stop and drag stops along it; pick a color to recolor the selected stop (or the ends, when none is selected); right-click or Delete removes a stop. Grips below move and rotate it. Enter finishes, Esc cancels.',
  distort: 'Drag or hold to distort: pinch, bulge, push or twirl (chosen in the toolbar). Right button or Alt: the opposite effect. Works inside the selection.',
  perspective: 'Drag a corner for perspective, an edge\u2019s diamond to skew (Shift: along the edge only), or inside to move. Shift+corner: one axis; Ctrl+corner: symmetric. Ctrl+Z steps back, Enter applies, Esc cancels.',
  brush: 'Left-click to paint with the primary color, right-click with the secondary.',
  eraser: 'Drag to erase to transparency.',
  pencil: 'Draw single-pixel, hard-edged lines.',
  'color-picker': 'Left-click sets the primary color, right-click the secondary.',
  'clone-stamp': 'Ctrl+click to set the source, then paint to copy pixels.',
  recolor: 'Paint to replace the secondary color with the primary within the tolerance.',
  text: 'Click and type. Drag the text to move it (Alt+arrow keys nudge) or its round handle to rotate it (Shift: 15° steps). Enter finishes (Shift+Enter for a new line), Esc cancels.',
  line: 'Drag to draw a line (Shift: 15° steps). Then drag its points, drag the line to bend it, or use the grips below it to move or rotate it; right-click a bend point to remove it; arrow keys nudge, Ctrl+Z steps back. Enter finishes, Esc cancels.',
  shape: 'Drag to draw the shape chosen in the toolbar (Shift: equal sides). Then drag inside to move it, a square to resize (Shift: keep proportions) or the round handle to rotate (Shift: 15° steps); custom polygons: drag a corner or an edge, right-click a corner to remove it. Arrow keys nudge, Ctrl+Z steps back. Enter finishes, Esc cancels.',
  freeform: 'Drag to draw a freeform shape.',
};
// Toolbar option groups → tools that show them (as in the C# port).
const SHAPE = ['line', 'shape', 'freeform'];
const OPTION_GROUPS = {
  brush: ['brush', 'eraser', 'clone-stamp', 'recolor', 'distort', ...SHAPE],
  pressure: ['brush', 'eraser', 'distort'],
  warp: ['distort'],
  tolerance: ['magic-wand', 'recolor', 'fill'],
  toleranceAlpha: ['magic-wand', 'fill'],
  wandSampling: ['magic-wand'],
  curve: ['line'],
  caps: ['line'],
  aa: ['brush', 'eraser', 'text', ...SHAPE],
  alphaBlend: ['brush', 'gradient', ...SHAPE],
  combine: ['rect-select', 'ellipse-select', 'lasso', 'magic-wand'],
  selQuality: ['rect-select', 'ellipse-select', 'lasso', 'magic-wand'],
  flood: ['magic-wand', 'fill'],
  selectSize: ['rect-select', 'ellipse-select'],
  shape: ['shape', 'freeform'],
  shapeType: ['shape'],
  fillStyle: ['shape', 'freeform', 'fill'],
  dash: ['line'],
  gradient: ['gradient'],
  sampling: ['fill', 'color-picker'],
  eraserPattern: ['eraser'],
  resampling: ['move'],
  font: ['text'],
};

const app = {
  engine: null,
  mode: '',
  view: null,
  state: null,
  fx: [],
  blendModes: [],
  tool: 'rect-select',
  docs: [],            // open documents (tabs): { id, name, dirty, file, view }
  docId: 0,            // active document id
  textSpec: null,
  thumbTimer: 0,
  menuState: { pixelGrid: false, rulers: false, toolbar: true, options: true, statusbar: true },
};

// The active document's file record ({ name, handle?, format }) lives with its tab.
Object.defineProperty(app, 'file', {
  get() { return curDoc()?.file ?? null; },
  set(f) { const d = curDoc(); if (d) d.file = f; },
});
function curDoc() { return app.docs.find(d => d.id === app.docId); }

// ------------------------------------------------------------------ engine plumbing

function onReply(r) {
  const j = r.json;
  if (j.docs) syncDocs(j.docs);
  const switched = j.doc !== undefined && j.doc !== app.docId;
  if (switched) {
    const old = curDoc();
    if (old && app.view.docW) old.view = { zoom: app.view.zoom, cx: app.view.cx, cy: app.view.cy };
    app.docId = j.doc;
    app.state = null; // the next state snapshot is the new document's
  }
  if (j.tile) {
    const t = j.tile;
    app.view.setDocSize(t.docW, t.docH);
    app.view.updateTile(t.x, t.y, t.w, t.h, r.bins.get('tile')[0]);
  }
  if (j.state) applyState(j.state);
  if (switched) {
    const d = curDoc();
    if (d?.view) { Object.assign(app.view, d.view); app.view.clampPan(); app.view.changed(); }
    else app.view.fit();
    renderTabs();
  }
  if (j.status) status(j.status);
}

let busyTimer = 0, busyEl = null;
async function call(cmd, args, bin) {
  clearTimeout(busyTimer);
  busyTimer = setTimeout(() => {
    if (!app.engine.idle && !busyEl) {
      busyEl = el('div', { class: 'busy-overlay', role: 'status' }, 'Working…');
      $('workspace').append(busyEl);
    }
  }, 400);
  try {
    return await app.engine.call(cmd, args, bin);
  } catch (e) {
    if (e.reply?.json?.error !== 'unsupported') status(e.message);
    throw e;
  } finally {
    if (app.engine.idle) { clearTimeout(busyTimer); busyEl?.remove(); busyEl = null; }
  }
}
const run = (cmd, args, bin) => call(cmd, args, bin).catch(() => {});

function status(text) { $('status-text').textContent = text; }

// ------------------------------------------------------------------ state → UI

function applyState(s) {
  const prev = app.state;
  app.state = { ...prev, ...s, selection: 'selection' in s ? s.selection : prev?.selection };
  const st = app.state;
  const v = app.view;
  v.selection = st.selection || null;
  v.handles = (app.tool === 'move' || app.tool === 'move-selection') ? st.handles || null : null;
  v.requestOverlay();
  const d = curDoc();
  if (d && (d.name !== st.name || d.dirty !== st.dirty)) { d.name = st.name; d.dirty = st.dirty; renderTabs(); }
  document.title = `${st.dirty ? '*' : ''}${st.name} - PaintDotRust`;
  // (The tabs show the name; the status bar shows the size.)
  $('status-size').textContent = `${st.width} × ${st.height}`;
  const b = st.selection?.bounds;
  $('status-sel').textContent = b ? `${b[2]} × ${b[3]}` : '';
  syncPending(st, prev);
  renderHistory();
  renderLayers();
  renderColors();
  syncToolOptions();
  updateMenus();
  clearTimeout(app.thumbTimer);
  app.thumbTimer = setTimeout(refreshThumbs, prev && prev.layers?.length === st.layers.length ? 250 : 0);
}

// ------------------------------------------------------------------ utility windows

const WINDOWS = {
  // autoHeight: the window is as tall as its content and only resizes horizontally
  // (the tools grid reflows into more columns as it gets wider).
  tools: { title: 'Tools', icon: 'window.tools', w: 76, h: 0, x: 8, y: 8, ax: 'left', ay: 'top', minW: 42, autoHeight: true },
  history: { title: 'History', icon: 'window.history', w: 210, h: 220, x: -8, y: 8, ax: 'right', ay: 'top', minW: 140, minH: 90 },
  layers: { title: 'Layers', icon: 'window.layers', w: 210, h: 250, x: -8, y: -8, ax: 'right', ay: 'bottom', minW: 170, minH: 110 },
  colors: { title: 'Colors', icon: 'window.colors', w: 236, h: 0, x: 92, y: -8, ax: 'left', ay: 'bottom', minW: 200, autoHeight: true },
};
const winEls = {};
/** Interface size factor (Accessibility). Window sizes are stored at 100% and shown scaled. */
const uiK = () => (app.a11y?.uiScale ?? 100) / 100;
let winZ = 20;

function loadLayout() {
  try { return JSON.parse(localStorage.getItem('openpdn.windows') || '{}'); } catch { return {}; }
}
function saveLayout() {
  const out = {};
  for (const [id, w] of Object.entries(winEls)) out[id] = { ...w.pos, visible: !w.el.hidden };
  try { localStorage.setItem('openpdn.windows', JSON.stringify(out)); } catch { /* private mode */ }
}

/** Places a window from its edge-relative position (keeps it anchored to its nearest edges). */
function placeWindow(w) {
  const ws = $('workspace').getBoundingClientRect();
  const { x, y, ax, ay } = w.pos;
  const width = w.el.offsetWidth, height = w.el.offsetHeight;
  let left = ax === 'left' ? x : ws.width - width + x;
  let top = ay === 'top' ? y : ws.height - height + y;
  left = Math.max(0, Math.min(ws.width - Math.min(width, 60), left));
  top = Math.max(0, Math.min(ws.height - height, top)); // whole window visible when it fits
  w.el.style.left = `${left}px`;
  w.el.style.top = `${top}px`;
}

/** Re-anchors a window to its nearest horizontal and vertical workspace edges. */
function anchorWindow(w) {
  const ws = $('workspace');
  const left = w.el.offsetLeft, top = w.el.offsetTop, width = w.el.offsetWidth, height = w.el.offsetHeight;
  const right = ws.clientWidth - (left + width), bottom = ws.clientHeight - (top + height);
  const pos = w.pos;
  pos.ax = left <= right ? 'left' : 'right';
  pos.ay = top <= bottom ? 'top' : 'bottom';
  pos.x = pos.ax === 'left' ? left : -right;
  pos.y = pos.ay === 'top' ? top : -bottom;
  pos.w = width / uiK();
  if (!WINDOWS[w.id].autoHeight) pos.h = height / uiK();
}

/** Re-applies window sizes after the interface size changes. */
function rescaleWindows() {
  const k = uiK();
  for (const [id, w] of Object.entries(winEls)) {
    const d = WINDOWS[id];
    w.el.style.width = `${Math.max(w.minW ?? d.minW ?? 0, w.pos.w) * k}px`;
    if (!d.autoHeight) w.el.style.height = `${Math.max(d.minH ?? 0, w.pos.h) * k}px`;
    if (!w.el.hidden) placeWindow(w);
  }
}

/** The "selected" window stays opaque while others fade (see Accessibility). */
function activateWindow(w) {
  for (const o of Object.values(winEls)) o.el.classList.toggle('active', o === w);
}

function raiseWindow(w) {
  if (w.el.style.zIndex == winZ) return;
  if (winZ > 900) { // renumber, keeping order, below menus (1000) and dialogs
    const order = Object.values(winEls).sort((a, b) => (+a.el.style.zIndex || 0) - (+b.el.style.zIndex || 0));
    winZ = 20;
    for (const o of order) o.el.style.zIndex = ++winZ;
  }
  w.el.style.zIndex = ++winZ;
}

const SNAP = 8;
/** Moves `v` (a near edge) and `v + size` (far edge) onto the closest target within SNAP px. */
function snap1(v, size, targets) {
  let best = SNAP + 1, out = v;
  for (const t of targets) {
    const d1 = Math.abs(v - t), d2 = Math.abs(v + size - t);
    if (d1 < best) { best = d1; out = t; }
    if (d2 < best) { best = d2; out = t - size; }
  }
  return out;
}

/**
 * Drags a window: `dir` is 'move' or a resize edge set ('e', 's', 'se', 'w', 'sw').
 * Moves are applied as a transform once per animation frame (no layout per event);
 * the final position is committed to left/top on release.
 */
function windowDrag(e, w, dir) {
  if (e.button !== 0) return;
  e.preventDefault();
  const node = w.el, def = WINDOWS[w.id], handle = e.currentTarget;
  const ws = $('workspace');
  const W = ws.clientWidth, H = ws.clientHeight;
  // Pointer positions are read only from events on the window frame itself,
  // which is never zoomed: browsers disagree about the coordinates of events
  // inside CSS-zoomed elements (the title bar is zoomed by the interface size).
  // The first captured move sets the starting point.
  // The press is on the window's unzoomed title bar or edges: its coordinates are reliable.
  const reliable = !handle.closest('.uw-title > *');
  const s = { l: node.offsetLeft, t: node.offsetTop, w: node.offsetWidth, h: node.offsetHeight, x: reliable ? e.clientX : null, y: reliable ? e.clientY : null };
  // Snap targets: workspace edges plus the edges of the other visible windows.
  const xs = [0, W], ys = [0, H];
  for (const o of Object.values(winEls)) {
    if (o === w || o.el.hidden) continue;
    xs.push(o.el.offsetLeft, o.el.offsetLeft + o.el.offsetWidth);
    ys.push(o.el.offsetTop, o.el.offsetTop + o.el.offsetHeight);
  }
  const minW = (w.minW ?? def.minW ?? 120) * uiK(), minH = (def.minH ?? 80) * uiK();
  // Reduce motion: drag a plain outline; the window follows on release.
  const ghost = app.a11y?.reduceMotion ? el('div', { class: 'uw-ghost', 'aria-hidden': 'true' }) : null;
  if (ghost) {
    Object.assign(ghost.style, { left: `${s.l}px`, top: `${s.t}px`, width: `${s.w}px`, height: `${s.h}px` });
    ws.append(ghost);
  }
  const box = ghost ?? node;
  let last = null, raf = 0, cur = { l: s.l, t: s.t, w: s.w, h: s.h };
  const apply = () => {
    raf = 0;
    if (!last || s.x === null) return;
    const dx = last.clientX - s.x, dy = last.clientY - s.y;
    if (dir === 'move') {
      let l = snap1(s.l + dx, s.w, xs), t = snap1(s.t + dy, s.h, ys);
      l = Math.max(Math.min(0, W - s.w), Math.min(W - Math.min(s.w, 60), l));
      t = Math.max(0, Math.min(H - 24, t));
      cur.l = l; cur.t = t;
      box.style.transform = `translate3d(${l - s.l}px, ${t - s.t}px, 0)`;
      return;
    }
    let { l, t, w: nw, h: nh } = s;
    if (dir.includes('e')) nw = Math.max(minW, Math.min(W - s.l, snap1(s.l + s.w + dx, 0, xs) - s.l));
    if (dir.includes('w')) { const r = s.l + s.w; l = Math.max(0, Math.min(r - minW, snap1(s.l + dx, 0, xs))); nw = r - l; }
    if (dir.includes('s')) nh = Math.max(minH, Math.min(H - s.t, snap1(s.t + s.h + dy, 0, ys) - s.t));
    box.style.left = `${l}px`;
    box.style.width = `${nw}px`;
    if (!def.autoHeight || ghost) box.style.height = `${def.autoHeight ? s.h : nh}px`;
    cur = { l, t, w: nw, h: nh };
  };
  const move = ev => {
    if (s.x === null) { s.x = ev.clientX; s.y = ev.clientY; return; }
    last = ev;
    if (!raf) raf = requestAnimationFrame(apply);
  };
  const up = ev => {
    node.removeEventListener('pointermove', move);
    node.removeEventListener('pointerup', up);
    node.removeEventListener('pointercancel', up);
    if (raf) cancelAnimationFrame(raf);
    if (s.x !== null) { last = ev; apply(); }
    ghost?.remove();
    if (dir === 'move') {
      node.style.transform = '';
      node.style.left = `${cur.l}px`;
      node.style.top = `${cur.t}px`;
    } else if (ghost) {
      node.style.left = `${cur.l}px`;
      node.style.width = `${cur.w}px`;
      if (!def.autoHeight) node.style.height = `${cur.h}px`;
    }
    node.classList.remove('dragging');
    document.body.classList.remove('window-dragging');
    anchorWindow(w);
    saveLayout();
  };
  node.setPointerCapture(e.pointerId);
  node.addEventListener('pointermove', move);
  node.addEventListener('pointerup', up);
  node.addEventListener('pointercancel', up);
  node.classList.add('dragging');
  document.body.classList.add('window-dragging');
  document.body.style.setProperty('--drag-cursor', dir === 'move' ? 'move' : getComputedStyle(handle).cursor);
}

function makeWindow(id, body, foot) {
  const def = WINDOWS[id];
  const saved = loadLayout()[id] || {};
  const pos = { x: def.x * uiK(), y: def.y * uiK(), ax: def.ax, ay: def.ay, w: def.w, h: def.h, ...saved };
  pos.w = Math.max(def.minW ?? 0, pos.w);
  const node = el('section', { class: `uw uw-${id}${def.autoHeight ? ' auto-h' : ''}`, id: `uw-${id}`, 'aria-label': def.title });
  node.style.width = `${pos.w * uiK()}px`;
  if (!def.autoHeight) node.style.height = `${Math.max(def.minH ?? 0, pos.h) * uiK()}px`;
  const title = el('div', { class: 'uw-title', title: def.title }, icon(def.icon, 14), el('span', {}, def.title));
  const close = el('button', { class: 'icon-button small', title: 'Close', 'aria-label': `Close ${def.title}` }, icon('window.close', 12));
  close.addEventListener('click', () => toggleWindow(id, false));
  title.append(close);
  node.append(title, el('div', { class: 'uw-body' }, body));
  if (foot) node.append(foot);
  const w = { id, el: node, pos };
  for (const dir of def.autoHeight ? ['e', 'w'] : ['e', 'w', 's', 'se', 'sw']) {
    const h = el('div', { class: 'uw-rz', 'data-dir': dir, 'aria-hidden': 'true' });
    h.addEventListener('pointerdown', e => windowDrag(e, w, dir));
    node.append(h);
  }
  $('workspace').append(node);
  winEls[id] = w;
  raiseWindow(w);
  if (saved.visible === false) node.hidden = true;
  node.addEventListener('pointerdown', () => { raiseWindow(w); activateWindow(w); }, true);
  title.addEventListener('pointerdown', e => { if (!e.target.closest('button')) windowDrag(e, w, 'move'); });
  requestAnimationFrame(() => placeWindow(w));
  // Keep the docked edge when the window changes size (e.g. bottom-docked Colors
  // window growing upwards when "More" or a bigger palette makes it taller).
  new ResizeObserver(() => { if (!node.hidden && !node.classList.contains('dragging')) placeWindow(w); }).observe(node);
  return node;
}

function toggleWindow(id, show) {
  const w = winEls[id];
  w.el.hidden = !(show ?? w.el.hidden);
  if (!w.el.hidden) { placeWindow(w); raiseWindow(w); }
  saveLayout();
  updateMenus();
  syncWindowToggles();
}

function resetLayout() {
  localStorage.removeItem('openpdn.windows');
  for (const [id, w] of Object.entries(winEls)) {
    const d = WINDOWS[id];
    Object.assign(w.pos, { x: d.x * uiK(), y: d.y * uiK(), ax: d.ax, ay: d.ay, w: d.w, h: d.h });
    w.el.style.width = `${d.w * uiK()}px`;
    if (!d.autoHeight) w.el.style.height = `${d.h * uiK()}px`;
    w.el.hidden = false;
    placeWindow(w);
  }
  updateMenus();
  syncWindowToggles();
}

/** Small buttons in the title bar that show/hide each window. */
const windowToggles = {};
function buildWindowToggles() {
  const keys = { tools: 'F5', history: 'F6', layers: 'F7', colors: 'F8' };
  const group = el('div', { class: 'window-toggles', role: 'group', 'aria-label': 'Windows' });
  for (const id of ['tools', 'colors', 'layers', 'history']) {
    const def = WINDOWS[id];
    const b = el('button', { class: 'icon-button', title: `${def.title} (${keys[id]})`, 'aria-label': `${def.title} window`, 'aria-pressed': 'false' }, icon(def.icon, 15));
    b.addEventListener('click', () => toggleWindow(id));
    windowToggles[id] = b;
    group.append(b);
  }
  $('a11y-button').before(group, el('span', { class: 'title-sep' }));
  syncWindowToggles();
}
function syncWindowToggles() {
  for (const [id, b] of Object.entries(windowToggles)) b.setAttribute('aria-pressed', String(!!winEls[id] && !winEls[id].el.hidden));
}

// Tools window
const toolButtons = {};
function buildToolsWindow() {
  const grid = el('div', { class: 'tools-grid', role: 'toolbar', 'aria-label': 'Tools' });
  for (const t of TOOLS) {
    const b = el('button', { class: 'icon-button', title: `${t.label} (${t.key})`, 'aria-label': t.label, 'aria-pressed': 'false' }, icon(t.icon, 18));
    b.addEventListener('click', () => selectTool(t.id));
    toolButtons[t.id] = b;
    grid.append(b);
  }
  makeWindow('tools', grid);
}

async function selectTool(id) {
  if (['rectangle', 'rounded-rect', 'ellipse'].includes(id)) id = 'shape';
  const t = TOOLS.find(x => x.id === id);
  if (!t) return;
  if (app.state?.busy) return; // mid-drag
  if (app.tool === 'text' && id !== 'text') textCommit();
  app.tool = id;
  for (const [k, b] of Object.entries(toolButtons)) { b.classList.toggle('on', k === id); b.setAttribute('aria-pressed', String(k === id)); }
  syncToolPicker();
  if (t.engine) await run('tool', { name: t.engine });
  status(TOOL_HINTS[id] || t.label);
  app.view.handles = (id === 'move' || id === 'move-selection') ? app.state?.handles || null : null;
  app.view.requestOverlay();
  syncToolOptions();
  updateViewCursor();
}

// History window
const historyList = el('ul', { class: 'list', role: 'listbox', 'aria-label': 'History' });
function buildHistoryWindow() {
  const foot = el('div', { class: 'uw-foot' });
  for (const [ic, label, cmd] of [['history.rewind', 'Rewind to original', 'rewind'], ['edit.undo', 'Undo', 'undo'], ['edit.redo', 'Redo', 'redo'], ['history.forward', 'Fast-forward', 'ff']]) {
    const b = el('button', { class: 'icon-button', title: label, 'aria-label': label, 'data-cmd': cmd }, icon(ic, 16));
    b.addEventListener('click', () => {
      const h = app.state.history;
      if (cmd === 'rewind') run('history_jump', { index: 0 });
      else if (cmd === 'ff') run('history_jump', { index: h.items.length });
      else run(cmd);
    });
    foot.append(b);
  }
  makeWindow('history', historyList, foot);
}
function renderHistory() {
  const h = app.state.history;
  historyList.replaceChildren();
  ['Original Image', ...h.items].forEach((name, i) => {
    const li = el('li', { role: 'option', class: i === h.applied ? 'selected' : i > h.applied ? 'future' : '', 'aria-selected': String(i === h.applied) }, name);
    li.addEventListener('click', () => run('history_jump', { index: i }));
    historyList.append(li);
  });
  historyList.lastChild?.scrollIntoView?.({ block: 'nearest' });
  historyList.children[h.applied]?.scrollIntoView?.({ block: 'nearest' });
}

// Layers window
const layerList = el('ul', { class: 'list', role: 'listbox', 'aria-label': 'Layers' });
let layerOpacity;
function buildLayersWindow() {
  const foot = el('div', { class: 'uw-foot' });
  const btn = (ic, label, fn) => { const b = el('button', { class: 'icon-button', title: label, 'aria-label': label }, icon(ic, 16)); b.addEventListener('click', fn); foot.append(b); return b; };
  btn('layer.add', 'Add New Layer (Ctrl+Shift+N)', () => run('layer_add'));
  btn('layer.delete', 'Delete Layer (Ctrl+Shift+Delete)', () => run('layer_delete'));
  btn('layer.duplicate', 'Duplicate Layer (Ctrl+Shift+D)', () => run('layer_duplicate'));
  btn('layer.merge-down', 'Merge Layer Down (Ctrl+M)', () => run('layer_merge_down'));
  btn('layer.up', 'Move Layer Up', () => run('layer_up'));
  btn('layer.down', 'Move Layer Down', () => run('layer_down'));
  btn('layer.properties', 'Layer Properties (F4)', layerProperties);
  layerOpacity = el('input', { type: 'range', min: 0, max: 255, title: 'Layer opacity', 'aria-label': 'Layer opacity' });
  let from = null;
  layerOpacity.addEventListener('input', () => {
    const idx = app.state.active;
    if (from == null) from = app.state.layers[idx].opacity;
    run('layer_opacity', { index: idx, opacity: +layerOpacity.value, commit: false });
  });
  layerOpacity.addEventListener('change', () => {
    run('layer_opacity', { index: app.state.active, opacity: +layerOpacity.value, from, commit: true });
    from = null;
  });
  foot.append(layerOpacity);
  makeWindow('layers', layerList, foot);
}
const thumbCanvases = [];
function renderLayers() {
  const st = app.state;
  layerList.replaceChildren();
  thumbCanvases.length = 0;
  st.layers.forEach((l, i) => {
    const cv = el('canvas', { width: 40, height: 40 });
    thumbCanvases[i] = cv;
    const vis = el('input', { type: 'checkbox', checked: l.visible, title: 'Visible', 'aria-label': `Show ${l.name}` });
    vis.addEventListener('click', e => e.stopPropagation());
    vis.addEventListener('change', () => run('layer_visible', { index: i, visible: vis.checked }));
    const li = el('li', { class: `layer-row${i === st.active ? ' selected' : ''}`, role: 'option', 'aria-selected': String(i === st.active) }, cv, el('span', { class: 'name', title: `${l.name} — ${l.blend}, ${Math.round(l.opacity / 2.55)}%` }, l.name), vis);
    li.addEventListener('click', () => run('layer_select', { index: i }));
    li.addEventListener('dblclick', layerProperties);
    layerList.prepend(li); // top layer first
  });
  if (layerOpacity && document.activeElement !== layerOpacity) layerOpacity.value = st.layers[st.active]?.opacity ?? 255;
}
async function refreshThumbs() {
  try {
    const r = await call('thumbs', { size: 40 });
    (r.bins.get('thumb') || []).forEach((px, i) => {
      const cv = thumbCanvases[i];
      if (!cv) return;
      cv.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(px), 40, 40), 0, 0);
    });
  } catch { /* ignore */ }
}

// Colors window
const colorsEls = {};
// The Colors window: primary/secondary swatches, an expandable editor
// ("More": hex, RGB, HSV and alpha with sliders and number boxes) and the
// palette, with built-in and custom palettes kept in the browser.
const colorEdit = { which: 'fg', c: { r: 0, g: 0, b: 0, a: 255 }, hsv: { h: 0, s: 0, v: 0 }, sent: { fg: null, bg: null }, expanded: false };

function buildColorsWindow() {
  const body = el('div', { class: 'colors-body' });
  const prim = el('button', { class: 'primary', 'aria-label': 'Primary color' }, el('i'));
  const sec = el('button', { class: 'secondary', 'aria-label': 'Secondary color' }, el('i'));
  const target = which => {
    if (colorEdit.expanded) setEditTarget(which);
    else pickColor(which);
  };
  prim.addEventListener('click', () => target('fg'));
  sec.addEventListener('click', () => target('bg'));
  prim.addEventListener('dblclick', () => pickColor('fg'));
  for (const [btn, key] of [[prim, 'fg'], [sec, 'bg']]) {
    btn.draggable = true;
    btn.addEventListener('dragstart', e => {
      const hex = app.state.settings[key];
      e.dataTransfer.setData('application/x-paintdotrust-color', hex);
      e.dataTransfer.setData('text/plain', hex);
    });
  }
  sec.addEventListener('dblclick', () => pickColor('bg'));
  const swap = el('button', { class: 'icon-button small', title: 'Swap colors (X)', 'aria-label': 'Swap colors' }, icon('color.swap', 12));
  swap.addEventListener('click', () => run('swap_colors'));
  const reset = el('button', { class: 'icon-button small', title: 'Reset colors (D)', 'aria-label': 'Reset colors' }, icon('color.reset', 12));
  reset.addEventListener('click', () => run('reset_colors'));
  const more = el('button', { class: 'more-button', 'aria-expanded': 'false', 'aria-controls': 'color-editor' });
  more.addEventListener('click', () => setColorsExpanded(!colorEdit.expanded));
  const sw = el('div', { class: 'swatches' }, prim, sec);
  const editor = buildColorEditor();
  const recent = el('div', { class: 'palette recent', 'aria-label': 'Recent colors' });

  // Palette picker
  const palSel = el('select', { class: 'palette-select', 'aria-label': 'Palette' });
  palSel.addEventListener('change', () => { Pal.setCurrentId(palSel.value); renderPalette(); });
  const palMenu = el('button', { class: 'icon-button small', title: 'Palette options', 'aria-label': 'Palette options', 'aria-haspopup': 'menu' }, icon('chevron.down', 12));
  palMenu.addEventListener('click', () => popupMenu(palMenu, paletteMenuItems()));
  const palette = el('div', { class: 'palette', role: 'group', 'aria-label': 'Palette colors' });
  palette.addEventListener('contextmenu', e => e.preventDefault());
  // Drop a colour (from Recent, the swatches, or another palette entry) to keep it here.
  const colorOf = e => e.dataTransfer.types.includes('application/x-paintdotrust-color');
  palette.addEventListener('dragover', e => { if (colorOf(e)) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; palette.classList.add('drop-target'); } });
  palette.addEventListener('dragleave', () => palette.classList.remove('drop-target'));
  palette.addEventListener('drop', e => {
    palette.classList.remove('drop-target');
    if (!colorOf(e)) return;
    e.preventDefault();
    const hex = e.dataTransfer.getData('application/x-paintdotrust-color');
    const at = e.target.closest('button');
    addColorToPalette(hex, at ? [...palette.children].indexOf(at) : undefined);
  });

  // Left: swatches and palette. Right (when "More" is on): the colour editor.
  body.append(
    el('div', { class: 'colors-main' },
      el('div', { class: 'colors-row' }, sw, el('div', { class: 'colors-mini' }, swap, reset), more),
      el('div', { class: 'palette-head' }, palSel, palMenu),
      palette,
      el('div', { class: 'group-label' }, 'Recent'), recent),
    editor);
  Object.assign(colorsEls, { prim, sec, recent, palette, palSel, more, editor });
  makeWindow('colors', body);
  setColorsExpanded(localStorage.getItem('openpdn.colorsMore') === '1', 'init');
  renderPalette();
}

const COLOR_EDITOR_W = 232; // width the window grows by when the editor is shown

function setColorsExpanded(on, persist = true) {
  const was = colorEdit.expanded;
  colorEdit.expanded = on;
  colorsEls.editor.hidden = !on;
  colorsEls.editor.parentElement.classList.toggle('expanded', on);
  const w = winEls.colors;
  if (w) {
    w.minW = (WINDOWS.colors.minW) + (on ? COLOR_EDITOR_W : 0);
    if (persist === 'init' && on && w.el.offsetWidth < w.minW * uiK()) w.el.style.width = `${w.minW * uiK()}px`;
    if (was !== on && persist !== 'init') {
      // Grow/shrink to the right, keeping the left edge (moving left only if out of room).
      // Grow/shrink to the right; the vertical dock (e.g. bottom edge) is kept,
      // and the ResizeObserver re-places the window for its new height.
      const W = $('workspace').clientWidth;
      const width = Math.max(w.minW * uiK(), w.el.offsetWidth + (on ? COLOR_EDITOR_W : -COLOR_EDITOR_W) * uiK());
      const left = Math.max(0, Math.min(w.el.offsetLeft, W - width));
      w.el.style.width = `${width}px`;
      w.pos.w = width / uiK();
      w.pos.ax = 'left';
      w.pos.x = left;
      placeWindow(w);
      saveLayout();
    }
  }
  colorsEls.more.setAttribute('aria-expanded', String(on));
  // Sideways chevron: the editor opens to the right.
  const chev = icon('chevron.down', 12);
  chev.style.transform = on ? 'rotate(90deg)' : 'rotate(-90deg)';
  colorsEls.more.replaceChildren(el('span', {}, on ? 'Less' : 'More'), chev);
  colorsEls.more.title = on ? 'Hide the color editor' : 'Show RGB, hex, HSV and alpha controls';
  colorsEls.prim.title = on ? 'Primary color — click to edit it below, double-click for the color picker' : 'Primary color — click to choose';
  colorsEls.sec.title = on ? 'Secondary color — click to edit it below, double-click for the color picker' : 'Secondary color — click to choose';
  if (persist === true) try { localStorage.setItem('openpdn.colorsMore', on ? '1' : '0'); } catch { /* ignore */ }
  syncEditTargetUi();
  if (app.state) loadEditColor(true);
}

function setEditTarget(which) {
  colorEdit.which = which;
  syncEditTargetUi();
  loadEditColor(true);
}
function syncEditTargetUi() {
  colorsEls.prim?.classList.toggle('editing', colorEdit.expanded && colorEdit.which === 'fg');
  colorsEls.sec?.classList.toggle('editing', colorEdit.expanded && colorEdit.which === 'bg');
  if (colorsEls.editingLabel) colorsEls.editingLabel.textContent = colorEdit.which === 'fg' ? 'Primary color' : 'Secondary color';
}

const CHANNELS = [
  // key, label, max, unit, aria
  ['r', 'R', 255, '', 'Red'], ['g', 'G', 255, '', 'Green'], ['b', 'B', 255, '', 'Blue'],
  ['h', 'H', 360, '°', 'Hue'], ['s', 'S', 100, '%', 'Saturation'], ['v', 'V', 100, '%', 'Value'],
  ['a', 'A', 255, '', 'Alpha (opacity)'],
];

function buildColorEditor() {
  const box = el('div', { class: 'color-editor', id: 'color-editor', hidden: true });
  // Which colour is being edited follows the swatch clicked last (primary/secondary).
  const editing = el('div', { class: 'editing-label', 'aria-live': 'polite' });
  colorsEls.editingLabel = editing;
  const hexIn = el('input', { type: 'text', class: 'hex', maxlength: 9, spellcheck: 'false', 'aria-label': 'Hexadecimal color' });
  hexIn.addEventListener('input', () => {
    const v = hexIn.value.trim().replace(/^#/, '');
    if (!/^([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(v)) { hexIn.classList.add('invalid'); return; }
    hexIn.classList.remove('invalid');
    const full = v.length <= 4 ? v.split('').map(ch => ch + ch).join('') : v;
    colorEdit.c = D.hexToRgba(full);
    colorEdit.hsv = rgbToHsv(colorEdit.c);
    syncEditor('hex');
    sendEditColor();
  });
  hexIn.addEventListener('change', () => { hexIn.value = editHex().slice(1); hexIn.classList.remove('invalid'); rememberEditColor(); });
  const rows = {};
  const grid = el('div', { class: 'channel-grid' });
  for (const [k, label, max, unit, aria] of CHANNELS) {
    const range = el('input', { type: 'range', class: `cslider${k === 'a' ? ' alpha' : ''}`, min: 0, max, step: 1, 'aria-label': aria });
    const num = el('input', { type: 'number', class: 'num', min: 0, max, step: 1, 'aria-label': aria });
    const apply = v => {
      v = Math.min(max, Math.max(0, Math.round(+v || 0)));
      const ed = colorEdit;
      if ('rgba'.includes(k)) { ed.c[k] = v; if (k !== 'a') ed.hsv = rgbToHsv(ed.c, ed.hsv); }
      else { ed.hsv[k] = k === 'h' ? v % 360 : v / 100; Object.assign(ed.c, hsvToRgb(ed.hsv)); if (k === 'h') ed.hsv.h = v; }
      syncEditor(k);
      sendEditColor();
    };
    range.addEventListener('input', () => apply(range.value));
    range.addEventListener('change', rememberEditColor);
    num.addEventListener('input', () => { if (num.value !== '') apply(num.value); });
    num.addEventListener('change', () => { syncEditor(); rememberEditColor(); });
    rows[k] = { range, num };
    if (k === 'h' || k === 'a') grid.append(el('div', { class: 'channel-sep' }));
    grid.append(el('label', { title: aria }, label), range, num, el('span', { class: 'unit' }, unit));
  }
  colorsEls.editRows = rows;
  colorsEls.hexIn = hexIn;
  box.append(editing,
    el('div', { class: 'hex-row' }, el('label', {}, 'Hex'), el('span', { class: 'hash' }, '#'), hexIn),
    grid);
  return box;
}

function rgbToHsv({ r, g, b }, prev) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let h = prev?.h ?? 0; // keep the hue for greys
  if (d) h = ((max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4) * 60 + 360) % 360;
  return { h, s: max ? d / max : (prev?.s ?? 0), v: max };
}
function hsvToRgb({ h, s, v }) {
  h = ((h % 360) + 360) % 360;
  const c = v * s, x = c * (1 - Math.abs((h / 60) % 2 - 1)), m = v - c;
  const [r, g, b] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return { r: Math.round((r + m) * 255), g: Math.round((g + m) * 255), b: Math.round((b + m) * 255) };
}
const editHex = () => D.rgbaToHex(colorEdit.c);

/** Loads the edited colour from the engine state unless it is our own echo. */
function loadEditColor(force = false) {
  const hex = app.state?.settings?.[colorEdit.which];
  if (!hex) return;
  if (!force && colorEdit.sent[colorEdit.which] && D.rgbaToHex(D.hexToRgba(hex)) === colorEdit.sent[colorEdit.which]) return;
  colorEdit.sent[colorEdit.which] = null;
  colorEdit.c = D.hexToRgba(hex);
  colorEdit.hsv = rgbToHsv(colorEdit.c, colorEdit.hsv);
  syncEditor();
}

function syncEditor(from) {
  const rows = colorsEls.editRows;
  if (!rows) return;
  const { c, hsv } = colorEdit;
  const vals = { r: c.r, g: c.g, b: c.b, a: c.a, h: Math.round(hsv.h) % 360, s: Math.round(hsv.s * 100), v: Math.round(hsv.v * 100) };
  for (const [k, { range, num }] of Object.entries(rows)) {
    range.value = vals[k];
    if (from !== k || document.activeElement !== num) num.value = vals[k];
  }
  if (from !== 'hex') { colorsEls.hexIn.value = editHex().slice(1); colorsEls.hexIn.classList.remove('invalid'); }
  // Slider tracks show what moving each slider would do.
  const rgb = (r, g, b) => `rgb(${r},${g},${b})`;
  const hsvCss = (h, s, v) => { const x = hsvToRgb({ h, s, v }); return rgb(x.r, x.g, x.b); };
  const track = {
    r: [rgb(0, c.g, c.b), rgb(255, c.g, c.b)], g: [rgb(c.r, 0, c.b), rgb(c.r, 255, c.b)], b: [rgb(c.r, c.g, 0), rgb(c.r, c.g, 255)],
    h: [0, 60, 120, 180, 240, 300, 360].map(h => hsvCss(h, Math.max(hsv.s, 0.15), Math.max(hsv.v, 0.35))),
    s: [hsvCss(hsv.h, 0, hsv.v), hsvCss(hsv.h, 1, hsv.v)], v: [rgb(0, 0, 0), hsvCss(hsv.h, hsv.s, 1)],
    a: [`rgba(${c.r},${c.g},${c.b},0)`, rgb(c.r, c.g, c.b)],
  };
  for (const [k, stops] of Object.entries(track)) rows[k].range.style.setProperty('--track', `linear-gradient(90deg, ${stops.join(', ')})`);
}

function sendEditColor() {
  const hex = editHex();
  colorEdit.sent[colorEdit.which] = hex;
  run('colors', { phase: 'live', [colorEdit.which]: hex });
}
function rememberEditColor() {
  run('remember_color', { color: editHex() });
}

function renderColors() {
  const s = app.state.settings;
  colorsEls.prim.firstChild.style.background = cssColor(s.fg);
  colorsEls.sec.firstChild.style.background = cssColor(s.bg);
  colorsEls.prim.setAttribute('aria-label', `Primary color ${s.fg}`);
  colorsEls.sec.setAttribute('aria-label', `Secondary color ${s.bg}`);
  colorsEls.recent.replaceChildren(...s.recent.map(hex => swatchButton(hex, null, true)));
  if (colorEdit.expanded) loadEditColor();
}
function cssColor(hex) {
  const c = D.hexToRgba(hex);
  return `rgba(${c.r},${c.g},${c.b},${c.a / 255})`;
}

/** A palette/recent swatch: click = primary, right-click = secondary (palettes) or a menu (recent). */
function swatchButton(hex, extra, recent = false) {
  const tip = recent ? `${hex} — click: primary · right-click: more (keep it in a palette) · drag it onto the palette to keep it`
    : `${hex} — click: primary, right-click: secondary${extra ? `\n${extra}` : ''}`;
  const b = el('button', { title: tip, 'aria-label': hex, draggable: 'true' }, el('i', { style: `background:${cssColor(hex)}` }));
  b.addEventListener('click', e => { if (!e.shiftKey && !e.altKey) run('colors', { fg: hex }); });
  b.addEventListener('contextmenu', e => {
    e.preventDefault();
    if (!recent) { run('colors', { bg: hex }); return; }
    const pal = currentPalette();
    popupMenu(b, [
      { label: 'Use as Primary', action: () => run('colors', { fg: hex }) },
      { label: 'Use as Secondary', action: () => run('colors', { bg: hex }) },
      '-',
      { label: pal.builtin ? `Keep in a Copy of “${pal.name}”` : `Keep in “${pal.name}”`, action: () => addColorToPalette(hex), enabled: pal.colors.length < Pal.MAX_COLORS },
    ]);
  });
  b.addEventListener('dragstart', e => {
    e.dataTransfer.setData('application/x-paintdotrust-color', hex);
    e.dataTransfer.setData('text/plain', hex);
    e.dataTransfer.effectAllowed = 'copy';
  });
  return b;
}

/** Adds a colour to the current palette (at `index`, or the end); built-ins are copied first. */
function addColorToPalette(hex, index) {
  const c = hex.replace('#', '').toUpperCase();
  editPalette(p => {
    if (p.colors.length >= Pal.MAX_COLORS) return;
    p.colors.splice(index ?? p.colors.length, 0, c);
  }, `Added ${hex} to the palette`);
}

// ------------------------------------------------------------------ palettes

function currentPalette() {
  const list = Pal.all();
  return list.find(p => p.id === Pal.currentId()) ?? list[0];
}

function renderPalette() {
  const cur = currentPalette();
  const custom = Pal.loadCustom();
  const opt = p => el('option', { value: p.id, selected: p.id === cur.id }, `${p.name} (${p.colors.length})`);
  colorsEls.palSel.replaceChildren(
    el('optgroup', { label: 'Built-in' }, ...Pal.BUILTIN.map(opt)),
    ...(custom.length ? [el('optgroup', { label: 'Saved in this browser' }, ...custom.map(opt))] : []));
  colorsEls.palSel.title = cur.credit ? `${cur.name} — ${cur.credit}` : cur.name;
  const editHint = 'Shift+click: replace with the primary color · Alt+click: remove';
  colorsEls.palette.replaceChildren(...cur.colors.map((c, i) => {
    const b = swatchButton(Pal.hex(c), editHint);
    b.addEventListener('click', e => {
      if (e.shiftKey) { e.preventDefault(); editPalette(p => { p.colors[i] = app.state.settings.fg.replace('#', ''); }, 'Replaced a palette color'); }
      else if (e.altKey) { e.preventDefault(); editPalette(p => { p.colors.splice(i, 1); }, 'Removed a palette color'); }
    });
    return b;
  }));
}

/** Applies an edit to the current palette; built-in palettes are copied first. */
function editPalette(fn, what) {
  let cur = currentPalette();
  const custom = Pal.loadCustom();
  let target = custom.find(p => p.id === cur.id);
  if (!target) {
    target = { id: Pal.newId(), name: Pal.uniqueName(`${cur.name} (custom)`), credit: cur.credit, colors: [...cur.colors] };
    custom.push(target);
    status(`Built-in palettes can't be changed: saved your edit to a copy, "${target.name}".`);
  } else if (what) status(what);
  fn(target);
  target.colors = target.colors.slice(0, Pal.MAX_COLORS);
  if (!Pal.saveCustom(custom)) { D.message('Palette', "Couldn't save the palette: the browser's storage is full or disabled."); return; }
  Pal.setCurrentId(target.id);
  renderPalette();
}

function paletteMenuItems() {
  const cur = currentPalette();
  return [
    { label: 'Add Primary Color', action: () => editPalette(p => { p.colors.push(app.state.settings.fg.replace('#', '')); }, 'Added the primary color to the palette'), enabled: cur.colors.length < Pal.MAX_COLORS },
    { label: 'Add Recent Colors', action: () => editPalette(p => { for (const h of app.state.settings.recent) { const c = h.replace('#', ''); if (!p.colors.includes(c)) p.colors.push(c); } }, 'Added the recent colors to the palette'), enabled: !!app.state?.settings?.recent?.length },
    '-',
    { label: 'New Empty Palette…', action: async () => { const name = await promptText('New Palette', 'Name', Pal.uniqueName('My palette')); if (name) addCustomPalette({ name, colors: [] }); } },
    { label: 'Duplicate Palette…', action: async () => { const name = await promptText('Duplicate Palette', 'Name', Pal.uniqueName(`${cur.name} copy`)); if (name) addCustomPalette({ name, colors: [...cur.colors], credit: cur.credit }); } },
    { label: 'Rename…', enabled: !cur.builtin, action: async () => {
      const name = await promptText('Rename Palette', 'Name', cur.name);
      if (name) { const list = Pal.loadCustom(); list.find(p => p.id === cur.id).name = name; Pal.saveCustom(list); renderPalette(); }
    } },
    { label: 'Delete', enabled: !cur.builtin, action: async () => {
      const r = await D.message('Delete Palette', `Delete the palette "${cur.name}" from this browser?`, [['Delete', 'delete', true], ['Cancel', null]]);
      if (r) { Pal.saveCustom(Pal.loadCustom().filter(p => p.id !== cur.id)); Pal.setCurrentId('default'); renderPalette(); }
    } },
    '-',
    { label: 'Import From File…', action: importPalettes },
    { label: 'Export to File…', action: () => exportPalette(cur) },
  ];
}

function addCustomPalette(p) {
  const list = Pal.loadCustom();
  const entry = { id: Pal.newId(), name: Pal.uniqueName(p.name), credit: p.credit, colors: p.colors.slice(0, Pal.MAX_COLORS) };
  list.push(entry);
  if (!Pal.saveCustom(list)) { D.message('Palette', "Couldn't save the palette: the browser's storage is full or disabled."); return null; }
  Pal.setCurrentId(entry.id);
  renderPalette();
  return entry;
}

function importPalettes() {
  const input = el('input', { type: 'file', accept: '.txt,.hex,text/plain', multiple: true, hidden: true });
  document.body.append(input);
  input.addEventListener('change', async () => {
    const names = [], bad = [];
    for (const f of input.files) {
      const p = Pal.parse(await f.text(), f.name.replace(/\.[^.]+$/, ''));
      if (!p.colors.length) { bad.push(f.name); continue; }
      if (addCustomPalette(p)) names.push(p.name);
    }
    input.remove();
    if (bad.length) D.message('Import Palette', `No colors found in ${bad.map(n => `"${n}"`).join(', ')}. Palette files list one color per line as hex (AARRGGBB or RRGGBB).`);
    if (names.length) status(`Imported ${names.length === 1 ? `palette "${names[0]}"` : `${names.length} palettes`}`);
  });
  input.click();
}

function exportPalette(p) {
  const blob = new Blob([Pal.serialize(p)], { type: 'text/plain' });
  const a = el('a', { href: URL.createObjectURL(blob), download: `${p.name.replace(/[\\/:*?"<>|]+/g, '_')}.txt` });
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(a.href); a.remove(); }, 1000);
}

function promptText(title, label, value) {
  let v = value;
  return D.modal(title, body => {
    const input = el('input', { type: 'text', value, 'aria-label': label, maxlength: 80 });
    input.addEventListener('input', () => (v = input.value));
    body.append(el('div', { class: 'field' }, el('label', {}, label), input));
    setTimeout(() => { input.focus(); input.select(); });
  }, { onOk: () => (v.trim() ? v.trim() : undefined), width: 360 });
}

/** A small drop-down menu anchored to a button. */
function popupMenu(anchor, items) {
  document.querySelector('.popup-menu')?.remove();
  const menu = el('div', { class: 'menu popup-menu', role: 'menu' });
  const close = () => { menu.remove(); document.removeEventListener('pointerdown', outside, true); document.removeEventListener('keydown', esc, true); anchor.focus(); };
  const outside = e => { if (!menu.contains(e.target) && e.target !== anchor) close(); };
  const esc = e => { if (e.key === 'Escape') { e.stopPropagation(); close(); } };
  for (const it of items) {
    if (it === '-') { menu.append(el('div', { class: 'menu-sep', role: 'separator' })); continue; }
    const enabled = it.enabled ?? true;
    const row = el('div', { class: `menu-item${it.checked ? ' checked' : ''}`, role: it.checked !== undefined ? 'menuitemradio' : 'menuitem', tabindex: -1, 'aria-disabled': String(!enabled), ...(it.checked !== undefined ? { 'aria-checked': String(!!it.checked) } : {}) },
      it.icon ? el('span', { class: 'menu-icon' }, icon(it.icon, 16)) : null, el('span', { class: 'label' }, it.label), it.key ? el('span', { class: 'key' }, it.key) : null);
    row.addEventListener('click', () => { close(); it.action(); });
    row.addEventListener('keydown', e => {
      if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); row.click(); }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const rows = [...menu.querySelectorAll('.menu-item:not([aria-disabled="true"])')];
        rows[(rows.indexOf(row) + (e.key === 'ArrowDown' ? 1 : rows.length - 1)) % rows.length]?.focus();
      }
    });
    menu.append(row);
  }
  document.body.append(menu);
  const r = anchor.getBoundingClientRect();
  const w = menu.offsetWidth, h = menu.offsetHeight;
  menu.style.left = `${Math.max(4, Math.min(innerWidth - w - 4, r.right - w))}px`;
  menu.style.top = `${r.bottom + h + 4 > innerHeight ? Math.max(4, r.top - h - 2) : r.bottom + 2}px`;
  document.addEventListener('pointerdown', outside, true);
  document.addEventListener('keydown', esc, true);
  (menu.querySelector('.menu-item.checked') ?? menu.querySelector('.menu-item:not([aria-disabled="true"])'))?.focus();
}

async function pickColor(which) {
  const cur = app.state.settings[which];
  const res = await D.colorPickerDialog(which === 'fg' ? 'Primary Color' : 'Secondary Color', cur);
  if (res) { await run('colors', { [which]: res }); run('remember_color', { color: res }); }
}

// ------------------------------------------------------------------ tool options bar

const opt = {};
const optGroups = {};
function buildToolOptions() {
  const bar = $('tool-options');
  const group = (name, ...kids) => { const g = el('span', { class: 'opt-group', 'data-group': name, style: 'display:inline-flex;align-items:center' }, ...kids); optGroups[name] = g; bar.append(g); return g; };
  const setting = (key, value) => run('setting', { key, value });
  // Tool picker: the same tools as the Tools window, for those who prefer a list.
  const picker = el('button', { class: 'tool-picker', 'aria-haspopup': 'menu', title: 'Choose a tool' });
  picker.addEventListener('click', () => popupMenu(picker, TOOLS.map(t => ({
    label: `${t.label}`, icon: t.icon, key: t.key, checked: t.id === app.tool, action: () => selectTool(t.id),
  }))));
  const pg = el('span', { class: 'opt-group tool-picker-group', 'data-group': 'toolPicker', style: 'display:inline-flex;align-items:center' }, picker, el('span', { class: 'sep' }));
  bar.append(pg);
  opt.toolPicker = picker;
  // Brush width
  const bw = el('input', { type: 'number', min: 1, max: 500, value: 8, 'aria-label': 'Brush width' });
  const bwr = el('input', { type: 'range', min: 1, max: 100, value: 8, 'aria-label': 'Brush width' });
  bw.addEventListener('input', () => { const v = +bw.value; if (v >= 1 && v <= 500) setting('brush', Math.round(v)); });
  bw.addEventListener('change', () => setting('brush', Math.max(1, Math.min(500, Math.round(+bw.value) || 1))));
  bwr.addEventListener('input', () => setting('brush', +bwr.value));
  group('brush', el('label', { title: 'Brush width' }, el('span', { class: 'group-label' }, 'Width:'), bwr, bw));
  opt.bw = bw; opt.bwr = bwr;
  const pressure = el('input', { type: 'checkbox' });
  pressure.addEventListener('change', () => setting('pressure', pressure.checked));
  group('pressure', el('label', {}, pressure, 'Pressure'));
  opt.pressureCb = pressure;
  const tol = el('input', { type: 'range', min: 0, max: 100, value: 13, 'aria-label': 'Tolerance' });
  const tolText = el('span', { style: 'width:34px' });
  tol.addEventListener('input', () => setting('tolerance', Math.round(+tol.value * 255 / 100)));
  group('tolerance', el('label', {}, el('span', { class: 'group-label' }, 'Tolerance:'), tol, tolText));
  opt.tol = tol; opt.tolText = tolText;
  const toggle = (name, ic, label, key) => {
    const b = el('button', { class: 'icon-button', title: label, 'aria-label': label, 'aria-pressed': 'true' });
    b.textContent = ic;
    b.addEventListener('click', () => setting(key, b.getAttribute('aria-pressed') !== 'true'));
    group(name, b);
    return b;
  };
  opt.aaBtn = toggle('aa', 'AA', 'Antialiasing', 'aa');
  opt.abBtn = toggle('alphaBlend', 'α', 'Alpha blending (off = overwrite)', 'alphaBlend');
  const select = (name, label, key, items, short = label) => {
    const s = el('select', { 'aria-label': label, title: label }, items.map(([v, t]) => el('option', { value: v }, t)));
    s.addEventListener('change', () => setting(key, s.value));
    group(name, el('label', {}, el('span', { class: 'group-label' }, `${short}:`), s));
    return s;
  };
  const SHORT = { 'Selection mode': 'Mode', 'Flood mode': 'Flood' };
  // Icon radio groups.
  const seg = (name, label, key, items) => {
    const wrap = el('span', { class: 'seg glyph-seg', role: 'radiogroup', 'aria-label': label });
    const btns = items.map(([v, g, t]) => {
      const b = el('button', { class: 'icon-button', role: 'radio', 'aria-checked': 'false', title: t, 'aria-label': t.split(' — ')[0] }, glyph(g));
      b.addEventListener('click', () => setting(key, v));
      b.addEventListener('keydown', e => {
        if (!['ArrowLeft', 'ArrowRight'].includes(e.key)) return;
        e.preventDefault();
        const n = btns[(btns.indexOf(b) + (e.key === 'ArrowRight' ? 1 : btns.length - 1)) % btns.length];
        n.focus(); n.click();
      });
      b.dataset.v = String(v);
      wrap.append(b);
      return b;
    });
    group(name, el('span', { class: 'group-label', title: label }, `${SHORT[label] ?? label}:`), wrap);
    return { set(v) { for (const b of btns) { const on = b.dataset.v === String(v); b.setAttribute('aria-checked', String(on)); b.classList.toggle('on', on); b.tabIndex = on ? 0 : -1; } } };
  };
  opt.combine = seg('combine', 'Selection mode', 'combine', [
    ['Replace', 'sel.Replace', 'Replace'],
    ['Union', 'sel.Union', 'Add (union) — or hold Ctrl and drag with the left button'],
    ['Subtract', 'sel.Subtract', 'Subtract — or hold Alt and drag with the left button'],
    ['Intersect', 'sel.Intersect', 'Intersect — or hold Alt and drag with the right button'],
    ['Xor', 'sel.Xor', 'Invert (xor) — or hold Ctrl and drag with the right button'],
  ]);
  opt.selQuality = seg('selQuality', 'Quality', 'selectionAA', [
    [true, 'quality.aa', 'Antialiased — selection edges blend smoothly into edits'],
    [false, 'quality.pixel', 'Pixelated — edits stop at hard, pixel-sharp selection edges'],
  ]);
  opt.flood = seg('flood', 'Flood mode', 'flood', [
    ['Contiguous', 'flood.Contiguous', 'Contiguous — only the connected area around the click'],
    ['Global', 'flood.Global', 'Global — every matching pixel in the image'],
  ]);
  buildSelectSize(group);
  opt.shape = select('shape', 'Draw', 'shape', [['Outline', 'Outline'], ['Fill', 'Interior'], ['FillOutline', 'Interior and outline']]);
  opt.gradient = select('gradient', 'Gradient', 'gradient', [['Linear', 'Linear'], ['LinearReflected', 'Linear (reflected)'], ['Diamond', 'Diamond'], ['Radial', 'Radial'], ['Conical', 'Conical']]);
  opt.gradientMode = el('select', { 'aria-label': 'Gradient mode' }, el('option', { value: 'Color' }, 'Color mode'), el('option', { value: 'Transparency' }, 'Transparency mode'));
  opt.gradientMode.addEventListener('change', () => setting('gradientMode', opt.gradientMode.value));
  opt.gradient.parentElement.parentElement.append(opt.gradientMode);
  opt.gradientRepeat = el('select', { 'aria-label': 'Repeat', title: 'Beyond the end points: keep the end colors, repeat the gradient, or reflect it back and forth' },
    el('option', { value: 'None' }, 'No repeat'), el('option', { value: 'Repeat' }, 'Repeat'), el('option', { value: 'Reflect' }, 'Reflect'));
  opt.gradientRepeat.addEventListener('change', () => setting('gradientRepeat', opt.gradientRepeat.value));
  opt.gradientDither = el('button', { class: 'icon-button', title: 'Dither: mix neighboring shades in a fine pattern so smooth gradients show no bands', 'aria-label': 'Dither', 'aria-pressed': 'true' }, 'Dither');
  opt.gradientDither.style.width = 'auto';
  opt.gradientDither.style.padding = '0 6px';
  opt.gradientDither.addEventListener('click', () => setting('gradientDither', opt.gradientDither.getAttribute('aria-pressed') !== 'true'));
  opt.gradientReverse = el('button', { class: 'icon-button small', title: 'Reverse the colors of the gradient being edited', 'aria-label': 'Reverse gradient' }, icon('color.swap', 12));
  opt.gradientReverse.addEventListener('click', () => run('gradient_reverse'));
  opt.gradient.parentElement.parentElement.append(opt.gradientRepeat, opt.gradientDither, opt.gradientReverse);
  // Distort brush: mode and strength.
  opt.warpMode = el('select', { 'aria-label': 'Distort', title: 'What the brush does (right button or Alt: the opposite)' },
    [['Pinch', 'Pinch'], ['Bulge', 'Bulge (pull)'], ['Push', 'Push'], ['TwirlClockwise', 'Twirl clockwise'], ['TwirlCounterclockwise', 'Twirl counterclockwise']].map(([v, t]) => el('option', { value: v }, t)));
  opt.warpMode.addEventListener('change', () => setting('warpMode', opt.warpMode.value));
  opt.warpStrength = el('input', { type: 'range', min: 1, max: 100, value: 50, 'aria-label': 'Strength' });
  opt.warpStrengthText = el('span', { style: 'width:34px' });
  opt.warpStrength.addEventListener('input', () => setting('warpStrength', +opt.warpStrength.value));
  group('warp', el('label', {}, el('span', { class: 'group-label' }, 'Distort:'), opt.warpMode),
    el('label', {}, el('span', { class: 'group-label' }, 'Strength:'), opt.warpStrength, opt.warpStrengthText));
  opt.sampling = el('select', { 'aria-label': 'Sampling' }, el('option', { value: 'layer' }, 'Layer'), el('option', { value: 'image' }, 'Image'));
  opt.sampling.addEventListener('change', () => setting('sampleAll', opt.sampling.value === 'image'));
  group('sampling', el('label', {}, el('span', { class: 'group-label' }, 'Sampling:'), opt.sampling));
  opt.eraser = select('eraserPattern', 'Fill', 'eraserPattern', [['Solid', 'Solid'], ['Horizontal', 'Horizontal lines'], ['Vertical', 'Vertical lines'], ['Cross', 'Cross'], ['ForwardDiagonal', 'Forward diagonal'], ['BackwardDiagonal', 'Backward diagonal'], ['DiagonalCross', 'Diagonal cross'], ['Percent50', '50% dots'], ['Percent25', '25% dots'], ['DottedGrid', 'Dotted grid'], ['Checkerboard', 'Checkerboard']]);
  opt.resampling = select('resampling', 'Sampling', 'resampling', [['NearestNeighbor', 'Nearest neighbor'], ['Bilinear', 'Bilinear'], ['MultisampleBilinear', 'Multisample bilinear'], ['Anisotropic', 'Anisotropic'], ['Bicubic', 'Bicubic']]);
  opt.resampling.title = 'How pixels are sampled when moved by fractions of a pixel or resized.\nNearest neighbor: hard pixels. Bilinear: smooth. Multisample bilinear: smoother edges when shrinking a little. Anisotropic: clean when shrinking a lot, or by different amounts across and down. Bicubic: sharpest smooth result.';
  opt.gamma = el('select', { 'aria-label': 'Gamma', title: 'Gamma corrected: blending takes gamma into account, so brightness (luminance) is preserved.\nIgnore gamma: blends the stored values directly; edges and shrunk detail come out darker.' },
    el('option', { value: 'on' }, 'Gamma corrected'), el('option', { value: 'off' }, 'Ignore gamma'));
  opt.gamma.addEventListener('change', () => setting('gamma', opt.gamma.value === 'on'));
  opt.resampling.parentElement.parentElement.append(opt.gamma);
  opt.tolAlpha = select('toleranceAlpha', 'Alpha', 'toleranceAlpha', [['Premultiplied', 'Premultiplied'], ['Straight', 'Straight']]);
  opt.tolAlpha.title = 'How transparency counts when comparing pixels.\nPremultiplied: fully transparent pixels match each other whatever color they hold.\nStraight (classic): transparent pixels match only if their color values match too.';
  opt.wandSampling = el('select', { 'aria-label': 'Sampling' }, el('option', { value: 'layer' }, 'Layer'), el('option', { value: 'image' }, 'Image'));
  opt.wandSampling.title = 'Compare colors on the current layer only, or on the whole image as you see it.';
  opt.wandSampling.addEventListener('change', () => setting('wandSampleImage', opt.wandSampling.value === 'image'));
  group('wandSampling', el('label', {}, el('span', { class: 'group-label' }, 'Sampling:'), opt.wandSampling));
  const CAPS = [['Flat', 'Flat'], ['Square', 'Square'], ['Round', 'Round'], ['Arrow', 'Arrow'], ['ArrowOpen', 'Open arrow'], ['ArrowIn', 'Inward arrow'], ['Diamond', 'Diamond'], ['Circle', 'Circle']];
  const capSel = (key, label) => {
    const sel = el('select', { 'aria-label': label, title: label }, CAPS.map(([v, t]) => el('option', { value: v }, t)));
    sel.addEventListener('change', () => setting(key, sel.value));
    return sel;
  };
  opt.capStart = capSel('capStart', 'Start cap');
  opt.capEnd = capSel('capEnd', 'End cap');
  const capSwap = el('button', { class: 'icon-button small', title: 'Swap start and end caps', 'aria-label': 'Swap start and end caps' }, icon('color.swap', 12));
  capSwap.addEventListener('click', async () => {
    const [a, b] = [opt.capStart.value, opt.capEnd.value];
    await run('setting', { key: 'capStart', value: b });
    await run('setting', { key: 'capEnd', value: a });
  });
  group('caps', el('span', { class: 'group-label' }, 'Ends:'), opt.capStart, capSwap, opt.capEnd);
  // Shape type (Shape tool) and its parameters.
  const SHAPE_TYPES = [
    ['Basic', [['Rectangle', 'Rectangle'], ['RoundedRectangle', 'Rounded rectangle'], ['Ellipse', 'Ellipse / circle'], ['Diamond', 'Diamond'], ['Triangle', 'Triangle'], ['RightTriangle', 'Right triangle'], ['Trapezoid', 'Trapezoid'], ['Parallelogram', 'Parallelogram']]],
    ['Polygons and stars', [['Pentagon', 'Pentagon'], ['Hexagon', 'Hexagon'], ['Octagon', 'Octagon'], ['Polygon', 'Regular polygon'], ['Star', 'Star'], ['CustomPolygon', 'Custom polygon (drag corners)']]],
    ['Arrows and symbols', [['Arrow', 'Arrow'], ['DoubleArrow', 'Double arrow'], ['Chevron', 'Chevron'], ['Plus', 'Plus'], ['Cross', 'Cross'], ['Heart', 'Heart'], ['Moon', 'Moon'], ['Lightning', 'Lightning'], ['Callout', 'Speech callout']]],
    ['Flowchart', [['Rectangle', 'Process'], ['Diamond', 'Decision'], ['Parallelogram', 'Data (input/output)'], ['Terminator', 'Terminator (start/end)'], ['Document', 'Document'], ['PredefinedProcess', 'Predefined process'], ['ManualInput', 'Manual input'], ['ManualOperation', 'Manual operation'], ['Preparation', 'Preparation'], ['Delay', 'Delay'], ['OffPageConnector', 'Off-page connector'], ['Database', 'Database'], ['Merge', 'Merge']]],
  ];
  const shapeSel = el('select', { 'aria-label': 'Shape' }, SHAPE_TYPES.map(([g, items]) => el('optgroup', { label: g }, items.map(([v, t]) => el('option', { value: v, 'data-group': g }, t)))));
  shapeSel.addEventListener('change', () => setting('shapeType', shapeSel.value));
  const sides = el('input', { type: 'number', min: 3, max: 64, step: 1, class: 'size-num', 'aria-label': 'Sides' });
  sides.addEventListener('input', () => { const v = +sides.value; if (v >= 3 && v <= 64) setting('shapeSides', v); });
  const inner = el('input', { type: 'number', min: 5, max: 95, step: 1, class: 'size-num', 'aria-label': 'Star inner radius (%)', title: 'Inner radius, as a percentage of the outer radius' });
  inner.addEventListener('input', () => { const v = +inner.value; if (v >= 5 && v <= 95) setting('starInner', v / 100); });
  const radius = el('input', { type: 'number', min: 0, max: 1000, step: 1, class: 'size-num', 'aria-label': 'Corner radius', title: 'Corner radius in pixels' });
  radius.addEventListener('input', () => { const v = +radius.value; if (v >= 0 && v <= 1000) setting('shapeRadius', v); });
  const sidesL = el('label', {}, el('span', { class: 'group-label' }, 'Sides:'), sides);
  const innerL = el('label', {}, el('span', { class: 'group-label' }, 'Inner:'), inner, el('span', { class: 'unit' }, '%'));
  const radiusL = el('label', {}, el('span', { class: 'group-label' }, 'Radius:'), radius);
  group('shapeType', el('label', {}, el('span', { class: 'group-label' }, 'Shape:'), shapeSel), sidesL, innerL, radiusL);
  Object.assign(opt, { shapeSel, sides, inner, radius, sidesL, innerL, radiusL });
  // Fill style: grouped list plus a live preview drawn by the engine (the same
  // pattern code that fills), so what you see is what you get.
  const FILL_STYLES = [
    ['Solid', [['Solid', 'Solid']]],
    ['Lines', [['Horizontal', 'Horizontal'], ['Vertical', 'Vertical'], ['ForwardDiagonal', 'Diagonal /'], ['BackwardDiagonal', 'Diagonal \\'],
      ['LightHorizontal', 'Light horizontal'], ['LightVertical', 'Light vertical'], ['DarkHorizontal', 'Dark horizontal'], ['DarkVertical', 'Dark vertical'],
      ['DashedHorizontal', 'Dashed horizontal'], ['Waves', 'Waves'], ['ZigZag', 'Zigzag']]],
    ['Grids', [['Cross', 'Grid'], ['SmallGrid', 'Small grid'], ['LargeGrid', 'Large grid'], ['DiagonalCross', 'Diagonal grid'], ['DenseCrosshatch', 'Dense crosshatch'],
      ['Diamonds', 'Diamonds'], ['Bricks', 'Bricks'], ['Plaid', 'Plaid']]],
    ['Dots and checks', [['Dots', 'Dots'], ['SparseDots', 'Sparse dots'], ['LargeDots', 'Large dots'], ['SmallCheckerboard', 'Small checkerboard'],
      ['Checkerboard', 'Checkerboard'], ['LargeCheckerboard', 'Large checkerboard']]],
    ['Ordered dither', [['Dither6', 'Dither 6%'], ['Dither12', 'Dither 12.5%'], ['Dither25', 'Dither 25%'], ['Dither37', 'Dither 37.5%'], ['Percent50', 'Dither 50%'],
      ['Dither62', 'Dither 62.5%'], ['Dither75', 'Dither 75%'], ['Dither87', 'Dither 87.5%'], ['Dither94', 'Dither 94%']]],
    ['Noise dither', [['Noise25', 'Noise 25%'], ['Noise50', 'Noise 50%'], ['Noise75', 'Noise 75%']]],
  ];
  opt.fillStyle = el('select', { 'aria-label': 'Fill style', title: 'Fill pattern: the fill color where the pattern is, the image showing through elsewhere' },
    FILL_STYLES.map(([g, items]) => el('optgroup', { label: g }, items.map(([v, t]) => el('option', { value: v }, t)))));
  opt.fillStyle.addEventListener('change', () => setting('fillStyle', opt.fillStyle.value));
  opt.fillPreview = el('canvas', { class: 'fill-preview', width: 24, height: 16, 'aria-hidden': 'true' });
  group('fillStyle', el('label', {}, el('span', { class: 'group-label' }, 'Fill:'), opt.fillPreview, opt.fillStyle));
  opt.dash = select('dash', 'Dash', 'dash', [['Solid', 'Solid ────'], ['Dash', 'Dashed ─ ─ ─'], ['LongDash', 'Long dashes ── ──'], ['Dot', 'Dotted ·····'], ['DenseDot', 'Dense dots ·······'], ['DashDot', 'Dash dot ─ · ─'], ['DashDotDot', 'Dash dot dot ─ · · ─']]);
  opt.curve = seg('curve', 'Curve', 'curve', [
    ['Straight', 'curve.Straight', 'Straight — drag the line to bend it at sharp angles'],
    ['Spline', 'curve.Spline', 'Spline — a smooth curve through every point'],
    ['Bezier', 'curve.Bezier', 'Bézier — drag the two control points to shape the curve'],
  ]);
  buildTextOptions(group);
}
// Rectangle/ellipse select size: any, fixed ratio, or fixed size in px/in/cm.
// The engine works in pixels; inches and centimetres use 96 pixels per inch.
const PPI = 96;
const UNITS = { px: 1, in: PPI, cm: PPI / 2.54 };
const selUi = { ratio: [4, 3], size: [256, 256], unit: (() => { try { return localStorage.getItem('openpdn.selUnit') || 'px'; } catch { return 'px'; } })() };
function buildSelectSize(group) {
  const mode = el('select', { 'aria-label': 'Selection size' },
    el('option', { value: 'Any' }, 'Any size'), el('option', { value: 'FixedRatio' }, 'Fixed ratio'), el('option', { value: 'FixedSize' }, 'Fixed size'));
  const w = el('input', { type: 'number', class: 'size-num', min: 0, 'aria-label': 'Width' });
  const h = el('input', { type: 'number', class: 'size-num', min: 0, 'aria-label': 'Height' });
  const sep = el('span', { class: 'size-sep' });
  const unit = el('select', { 'aria-label': 'Units', title: 'Inches and centimetres use 96 pixels per inch' },
    el('option', { value: 'px' }, 'pixels'), el('option', { value: 'in' }, 'inches'), el('option', { value: 'cm' }, 'cm'));
  const swap = el('button', { class: 'icon-button small', title: 'Swap width and height', 'aria-label': 'Swap width and height' }, icon('color.swap', 12));
  const send = () => {
    const m = mode.value;
    if (m === 'Any') { run('setting', { key: 'selectSize', value: { mode: 'Any' } }); return; }
    const a = parseFloat(w.value), b = parseFloat(h.value);
    if (!(a > 0 && b > 0)) return;
    if (m === 'FixedRatio') { selUi.ratio = [a, b]; run('setting', { key: 'selectSize', value: { mode: m, w: a, h: b } }); return; }
    const k = UNITS[unit.value];
    selUi.size = [Math.max(1, Math.round(a * k)), Math.max(1, Math.round(b * k))];
    run('setting', { key: 'selectSize', value: { mode: m, w: selUi.size[0], h: selUi.size[1] } });
  };
  mode.addEventListener('change', () => {
    const m = mode.value;
    if (m === 'FixedRatio') [w.value, h.value] = selUi.ratio;
    if (m === 'FixedSize') showSize(selUi.size);
    send();
  });
  for (const i of [w, h]) i.addEventListener('change', send);
  unit.addEventListener('change', () => { selUi.unit = unit.value; try { localStorage.setItem('openpdn.selUnit', unit.value); } catch { /* ignore */ } showSize(selUi.size); });
  swap.addEventListener('click', () => { [w.value, h.value] = [h.value, w.value]; send(); });
  const showSize = ([pw, ph]) => {
    const k = UNITS[unit.value], dp = unit.value === 'px' ? 0 : 2;
    w.value = +(pw / k).toFixed(dp); h.value = +(ph / k).toFixed(dp);
    w.step = h.step = unit.value === 'px' ? '1' : '0.01';
  };
  unit.value = selUi.unit;
  const fields = el('span', { class: 'size-fields' }, w, sep, h, unit, swap);
  group('selectSize', el('label', {}, el('span', { class: 'group-label' }, 'Size:'), mode), fields);
  Object.assign(opt, { sizeMode: mode, sizeW: w, sizeH: h, sizeUnit: unit, sizeSep: sep, sizeFields: fields, showSize });
}
// Text tool options: font, size, bold/italic/underline, alignment. Changes
// apply live to the text being typed.
function buildTextOptions(group) {
  const font = el('select', { 'aria-label': 'Font' }, TE.FONTS.map(f => el('option', { value: f, style: `font-family:${/\s/.test(f) ? `"${f}"` : f}` }, f)));
  font.addEventListener('change', () => setTextSpec({ font: font.value }));
  const size = el('input', { type: 'number', min: 4, max: 1000, class: 'size-num', 'aria-label': 'Font size (pixels)', title: 'Font size in pixels' });
  size.addEventListener('input', () => { const v = +size.value; if (v >= 4 && v <= 1000) setTextSpec({ size: v }); });
  const tog = (key, label, text, style) => {
    const b = el('button', { class: 'icon-button text-style', title: label, 'aria-label': label, 'aria-pressed': 'false', style }, text);
    b.addEventListener('click', () => setTextSpec({ [key]: !app.textSpec[key] }));
    return b;
  };
  const bold = tog('bold', 'Bold', 'B', 'font-weight:700'), italic = tog('italic', 'Italic', 'I', 'font-style:italic;font-family:serif'), underline = tog('underline', 'Underline', 'U', 'text-decoration:underline');
  const aligns = ['left', 'center', 'right'].map(a => {
    const b = el('button', { class: 'icon-button', role: 'radio', 'aria-checked': 'false', title: `Align ${a}`, 'aria-label': `Align ${a}` }, glyph(`align.${a}`, 18));
    b.addEventListener('click', () => setTextSpec({ align: a }));
    b.dataset.v = a;
    return b;
  });
  group('font', el('label', {}, el('span', { class: 'group-label' }, 'Font:'), font), size, el('span', { class: 'unit' }, 'px'),
    el('span', { class: 'seg', role: 'group', 'aria-label': 'Style' }, bold, italic, underline),
    el('span', { class: 'seg glyph-seg', role: 'radiogroup', 'aria-label': 'Alignment' }, ...aligns));
  Object.assign(opt, { textFont: font, textSize: size, textBold: bold, textItalic: italic, textUnderline: underline, textAligns: aligns });
}
function syncTextOptions() {
  if (!opt.textFont) return;
  const t = app.textSpec;
  opt.textFont.value = t.font;
  if (document.activeElement !== opt.textSize) opt.textSize.value = t.size;
  for (const [b, on] of [[opt.textBold, t.bold], [opt.textItalic, t.italic], [opt.textUnderline, t.underline]]) b.setAttribute('aria-pressed', String(!!on));
  for (const b of opt.textAligns) { const on = b.dataset.v === t.align; b.setAttribute('aria-checked', String(on)); b.classList.toggle('on', on); }
}

function syncSelectSize(ss) {
  if (!ss || !opt.sizeMode) return;
  const editing = [opt.sizeW, opt.sizeH].includes(document.activeElement);
  opt.sizeMode.value = ss.mode;
  opt.sizeFields.hidden = ss.mode === 'Any';
  opt.sizeUnit.hidden = ss.mode !== 'FixedSize';
  opt.sizeSep.textContent = ss.mode === 'FixedRatio' ? ':' : '×';
  const what = ss.mode === 'FixedRatio' ? 'Ratio' : 'Size';
  opt.sizeW.setAttribute('aria-label', `${what} width`); opt.sizeH.setAttribute('aria-label', `${what} height`);
  if (ss.mode === 'FixedRatio') { selUi.ratio = [ss.w, ss.h]; if (!editing) { opt.sizeW.value = ss.w; opt.sizeH.value = ss.h; opt.sizeW.step = opt.sizeH.step = 'any'; } }
  if (ss.mode === 'FixedSize') {
    selUi.size = [ss.w, ss.h];
    // Keep what was typed (e.g. "1" cm) when it maps to the same pixels.
    const k = UNITS[opt.sizeUnit.value];
    const same = [opt.sizeW, opt.sizeH].every((i, n) => i.value !== '' && Math.max(1, Math.round(parseFloat(i.value) * k)) === selUi.size[n]);
    if (!editing && !same) opt.showSize(selUi.size);
  }
}

/** Marks the options row when it is cut off, so the edge fades as a hint to scroll. */
function markOptionsOverflow() {
  const o = $('tool-options');
  const more = o.scrollWidth - o.clientWidth - o.scrollLeft > 2;
  o.classList.toggle('more-right', more);
  o.classList.toggle('more-left', o.scrollLeft > 2);
}

/** Draws the fill pattern preview (pattern from the engine, in the fill color). */
let fillPreviewKey = '';
async function drawFillPreview(style, color) {
  const key = `${style}|${color}|${app.docId}`;
  if (key === fillPreviewKey || !opt.fillPreview) return;
  fillPreviewKey = key;
  const cv = opt.fillPreview, w = cv.width, h = cv.height;
  let mask;
  try { mask = (await call('pattern_preview', { style, w, h })).bins.get('mask')?.[0]; } catch { return; }
  if (!mask || key !== fillPreviewKey) return;
  const c = D.hexToRgba(color ?? '#000000');
  const img = new ImageData(w, h);
  for (let i = 0; i < w * h; i++) {
    const on = mask[i] > 0;
    // Pattern in the fill color over a light checker, so any color shows.
    const bg = ((i % w) >> 2 ^ (i / w | 0) >> 2) & 1 ? 255 : 230;
    img.data.set(on ? [c.r, c.g, c.b, 255] : [bg, bg, bg, 255], i * 4);
  }
  cv.getContext('2d').putImageData(img, 0, 0);
}

function syncToolPicker() {
  const b = opt.toolPicker;
  const t = TOOLS.find(x => x.id === app.tool);
  if (!b || !t) return;
  const chev = icon('chevron.down', 12);
  b.replaceChildren(icon(t.icon, 16), el('span', {}, t.label), chev);
  b.setAttribute('aria-label', `Tool: ${t.label}. Choose another tool`);
}

function syncToolOptions() {
  const s = app.state?.settings;
  for (const [name, tools] of Object.entries(OPTION_GROUPS)) if (optGroups[name]) optGroups[name].style.display = tools.includes(app.tool) ? 'inline-flex' : 'none';
  syncToolPicker();
  requestAnimationFrame(markOptionsOverflow);
  if (!s) return;
  if (document.activeElement !== opt.bw) opt.bw.value = s.brush;
  opt.bwr.value = Math.min(100, s.brush);
  opt.pressureCb.checked = s.pressure;
  opt.tol.value = Math.round(s.tolerance * 100 / 255);
  opt.tolText.textContent = `${Math.round(s.tolerance * 100 / 255)}%`;
  opt.aaBtn.setAttribute('aria-pressed', String(s.aa));
  opt.abBtn.setAttribute('aria-pressed', String(s.alphaBlend));
  opt.combine.set(s.combine);
  opt.selQuality.set(s.selectionAA);
  opt.flood.set(s.flood);
  syncSelectSize(s.selectSize);
  opt.shape.value = s.shape;
  opt.gradient.value = s.gradient;
  opt.gradientMode.value = s.gradientMode;
  opt.gradientRepeat.value = s.gradientRepeat ?? 'None';
  opt.gradientDither.setAttribute('aria-pressed', String(!!s.gradientDither));
  opt.gradientReverse.disabled = app.state?.pending?.kind !== 'gradient';
  opt.warpMode.value = s.warpMode ?? 'Pinch';
  opt.warpStrength.value = s.warpStrength ?? 50;
  opt.warpStrengthText.textContent = `${s.warpStrength ?? 50}%`;
  opt.sampling.value = s.sampleAll ? 'image' : 'layer';
  opt.eraser.value = s.eraserPattern;
  opt.resampling.value = s.resampling === 'NearestNeighborAntialiased' ? 'Bilinear' : s.resampling;
  opt.gamma.value = s.gamma ? 'on' : 'off';
  opt.tolAlpha.value = s.toleranceAlpha;
  opt.wandSampling.value = s.wandSampleImage ? 'image' : 'layer';
  opt.curve.set(s.curve);
  opt.capStart.value = s.capStart;
  opt.fillStyle.value = s.fillStyle;
  // Shapes fill with the secondary color (left button), the bucket with the primary.
  drawFillPreview(s.fillStyle, app.tool === 'fill' ? s.fg : s.bg);
  opt.dash.value = s.dash;
  if (opt.shapeSel.value !== s.shapeType) opt.shapeSel.value = s.shapeType;
  const editing = el => document.activeElement === el;
  if (!editing(opt.sides)) opt.sides.value = s.shapeSides;
  if (!editing(opt.inner)) opt.inner.value = Math.round(s.starInner * 100);
  if (!editing(opt.radius)) opt.radius.value = s.shapeRadius;
  opt.sidesL.hidden = !['Polygon', 'Star', 'CustomPolygon'].includes(s.shapeType);
  opt.sidesL.firstChild.textContent = s.shapeType === 'Star' ? 'Points:' : 'Sides:';
  opt.innerL.hidden = s.shapeType !== 'Star';
  opt.radiusL.hidden = s.shapeType !== 'RoundedRectangle';
  opt.capEnd.value = s.capEnd;
  syncTextOptions();
  updateViewCursor();
}

// ------------------------------------------------------------------ menus

let menuDefs = [];
function menuSpec() {
  const st = () => app.state;
  const hasSel = () => !!st()?.selection;
  const fxMenu = cat => app.fx.filter(f => f.menu === cat).map(f => ({ label: `${f.title}${f.direct ? '' : '…'}`, key: f.shortcut, action: () => runFx(f) }));
  return [
    { label: 'File', items: [
      { label: 'New…', key: 'Ctrl+N', icon: 'file.new', action: fileNew },
      { label: 'Open…', key: 'Ctrl+O', icon: 'file.open', action: fileOpen },
      { label: 'Open Recent', submenu: () => recentMenu() },
      { label: 'Close', key: 'Ctrl+Alt+W', icon: 'window.close', action: () => closeDoc() },
      { label: 'Close All', action: closeAll },
      '-',
      { label: 'Save', key: 'Ctrl+S', icon: 'file.save', action: () => fileSave(false) },
      { label: 'Save As…', key: 'Ctrl+Shift+S', icon: 'file.save-as', action: () => fileSave(true) },
    ] },
    { label: 'Edit', items: [
      { label: () => `Undo${st()?.history?.applied ? ' ' + st().history.items[st().history.applied - 1] : ''}`, key: 'Ctrl+Z', icon: 'edit.undo', enabled: () => st()?.history?.applied > 0, action: () => run('undo') },
      { label: () => { const h = st()?.history; return `Redo${h && h.applied < h.items.length ? ' ' + h.items[h.applied] : ''}`; }, key: 'Ctrl+Y', icon: 'edit.redo', enabled: () => st()?.history?.applied < st()?.history?.items.length, action: () => run('redo') },
      '-',
      { label: 'Cut', key: 'Ctrl+X', icon: 'edit.cut', enabled: hasSel, action: () => copy('cut') },
      { label: 'Copy', key: 'Ctrl+C', icon: 'edit.copy', enabled: hasSel, action: () => copy('copy') },
      { label: 'Copy Merged', key: 'Ctrl+Shift+C', action: () => copy('copy_merged') },
      { label: 'Paste', key: 'Ctrl+V', icon: 'edit.paste', action: () => pasteFromMenu('layer') },
      { label: 'Paste into New Layer', key: 'Ctrl+Shift+V', action: () => pasteFromMenu('new_layer') },
      { label: 'Paste into New Image', key: 'Ctrl+Alt+V', action: () => pasteFromMenu('new_image') },
      '-',
      { label: 'Select All', key: 'Ctrl+A', icon: 'edit.select-all', action: () => run('select_all') },
      { label: 'Deselect', key: 'Ctrl+D', icon: 'edit.deselect', enabled: hasSel, action: () => run('deselect') },
      { label: 'Invert Selection', key: 'Ctrl+I', action: () => run('invert_selection') },
      { label: 'Grow Selection by 1 px', enabled: hasSel, action: () => run('grow_selection') },
      { label: 'Shrink Selection by 1 px', enabled: hasSel, action: () => run('shrink_selection') },
      { label: 'Feather Selection…', enabled: hasSel, action: featherSelection },
      '-',
      { label: 'Erase Selection', key: 'Delete', enabled: hasSel, action: () => run('erase_selection') },
      { label: 'Fill Selection', key: 'Backspace', enabled: hasSel, action: () => run('fill_selection') },
    ] },
    { label: 'View', items: [
      { label: 'Zoom In', key: 'Ctrl+Plus', icon: 'view.zoom-in', action: () => app.view.zoomStep(1) },
      { label: 'Zoom Out', key: 'Ctrl+Minus', icon: 'view.zoom-out', action: () => app.view.zoomStep(-1) },
      { label: 'Zoom to Window', key: 'Ctrl+B', icon: 'view.fit', action: () => app.view.fit() },
      { label: 'Zoom to Selection', key: 'Ctrl+Shift+B', enabled: hasSel, action: () => { const b = st().selection.bounds; app.view.zoomToRect(...b); } },
      { label: 'Actual Size', key: 'Ctrl+Shift+A', icon: 'view.actual', action: () => app.view.actualSize() },
      '-',
      { label: 'Pixel Grid', icon: 'view.grid', checked: () => app.menuState.pixelGrid, action: togglePixelGrid },
      { label: 'Rulers', icon: 'view.rulers', checked: () => app.menuState.rulers, action: toggleRulers },
      { label: 'Color Vision Preview', submenu: () => A11y.FILTERS.map(f => f === '-' ? '-' : { label: f.label, checked: () => app.a11y.filter === f.id, action: () => setA11y({ filter: f.id }) }) },
      { label: 'Accessibility…', action: accessibilityDialog },
      '-',
      ...Object.keys(WINDOWS).map(id => ({ label: `${WINDOWS[id].title} Window`, icon: WINDOWS[id].icon, checked: () => !winEls[id]?.el.hidden, key: { tools: 'F5', history: 'F6', layers: 'F7', colors: 'F8' }[id], action: () => toggleWindow(id) })),
      { label: 'Reset Window Layout', icon: 'window.reset', action: resetLayout },
      '-',
      { label: 'Common Actions Toolbar', checked: () => app.menuState.toolbar, action: () => toggleBar('toolbar', 'common-toolbar') },
      { label: 'Tool Options Toolbar', checked: () => app.menuState.options, action: () => toggleBar('options', 'tool-options') },
      { label: 'Status Bar', checked: () => app.menuState.statusbar, action: () => toggleBar('statusbar', 'statusbar') },
    ] },
    { label: 'Image', items: [
      { label: 'Crop to Selection', key: 'Ctrl+Shift+X', icon: 'image.crop', enabled: hasSel, action: () => run('crop') },
      { label: 'Auto-Crop', action: () => run('autocrop') },
      { label: 'Resize…', key: 'Ctrl+R', icon: 'image.resize', action: resizeImage },
      { label: 'Canvas Size…', key: 'Ctrl+Shift+R', icon: 'image.canvas-size', action: canvasSize },
      '-',
      { label: 'Flip Horizontal', icon: 'image.flip-h', action: () => run('flip_h') },
      { label: 'Flip Vertical', icon: 'image.flip-v', action: () => run('flip_v') },
      '-',
      { label: 'Rotate 90° Clockwise', key: 'Ctrl+H', icon: 'image.rotate-cw', action: () => run('rotate_cw') },
      { label: 'Rotate 90° Counter-Clockwise', key: 'Ctrl+G', icon: 'image.rotate-ccw', action: () => run('rotate_ccw') },
      { label: 'Rotate 180°', key: 'Ctrl+J', action: () => run('rotate_180') },
      '-',
      { label: 'Flatten', key: 'Ctrl+Shift+F', icon: 'image.flatten', enabled: () => st()?.layers.length > 1, action: () => run('flatten') },
    ] },
    { label: 'Layers', items: [
      { label: 'Add New Layer', key: 'Ctrl+Shift+N', icon: 'layer.add', action: () => run('layer_add') },
      { label: 'Delete Layer', key: 'Ctrl+Shift+Delete', icon: 'layer.delete', enabled: () => st()?.layers.length > 1, action: () => run('layer_delete') },
      { label: 'Duplicate Layer', key: 'Ctrl+Shift+D', icon: 'layer.duplicate', action: () => run('layer_duplicate') },
      { label: 'Merge Layer Down', key: 'Ctrl+M', icon: 'layer.merge-down', enabled: () => st()?.active > 0, action: () => run('layer_merge_down') },
      { label: 'Import from File…', icon: 'layer.import', action: importLayer },
      '-',
      { label: 'Flip Horizontal', action: () => run('layer_flip_h') },
      { label: 'Flip Vertical', action: () => run('layer_flip_v') },
      { label: 'Rotate / Zoom…', key: 'Ctrl+Shift+Z', action: () => runFx(app.fx.find(f => f.id === 'rotate-zoom')) },
      '-',
      { label: 'Move Layer Up', icon: 'layer.up', enabled: () => st()?.active < st()?.layers.length - 1, action: () => run('layer_up') },
      { label: 'Move Layer Down', icon: 'layer.down', enabled: () => st()?.active > 0, action: () => run('layer_down') },
      '-',
      { label: 'Layer Properties…', key: 'F4', icon: 'layer.properties', action: layerProperties },
    ] },
    { label: 'Adjustments', items: fxMenu('Adjustments') },
    { label: 'Effects', items: ['Artistic', 'Blurs', 'Distort', 'Noise', 'Photo', 'Render', 'Stylize'].map(c => ({ label: c, submenu: () => fxMenu(c) })) },
    { label: 'Help', items: [
      { label: 'Keyboard Shortcuts…', key: 'F1', icon: 'app.help', action: showShortcuts },
      { label: 'About PaintDotRust…', icon: 'app.info', action: showAbout },
    ] },
  ];
}

function buildMenus() {
  menuDefs = menuSpec();
  const bar = $('menubar');
  bar.replaceChildren();
  let openTop = null;
  const closeAll = () => { bar.querySelectorAll('.open').forEach(n => n.classList.remove('open')); openTop = null; };
  const renderItems = (menu, items) => {
    menu.replaceChildren();
    for (const it of items) {
      if (it === '-') { menu.append(el('div', { class: 'menu-sep', role: 'separator' })); continue; }
      const label = typeof it.label === 'function' ? it.label() : it.label;
      const enabled = it.enabled ? !!it.enabled() : true;
      const checked = it.checked ? !!it.checked() : null;
      const row = el('div', { class: `menu-item${it.submenu ? ' has-sub' : ''}`, role: checked === null ? 'menuitem' : 'menuitemcheckbox', tabindex: -1, 'aria-disabled': String(!enabled), ...(checked !== null ? { 'aria-checked': String(checked) } : {}) },
        el('span', { class: 'icon-slot' }, checked === true ? el('span', { class: 'check-mark' }, '✓') : it.icon ? icon(it.icon, 16) : null),
        el('span', { class: 'label' }, label), it.key ? el('span', { class: 'key' }, it.key.replace('Plus', '+').replace('Minus', '−')) : null);
      if (it.submenu) {
        const sub = el('div', { class: 'menu sub', role: 'menu' });
        row.append(sub);
        row.addEventListener('pointerenter', () => { menu.querySelectorAll(':scope > .menu-item.open').forEach(n => n !== row && n.classList.remove('open')); renderItems(sub, it.submenu()); row.classList.add('open'); });
      } else {
        row.addEventListener('pointerenter', () => menu.querySelectorAll(':scope > .menu-item.open').forEach(n => n.classList.remove('open')));
        row.addEventListener('click', e => { e.stopPropagation(); closeAll(); if (enabled) it.action(); });
      }
      menu.append(row);
    }
  };
  for (const m of menuDefs) {
    const top = el('div', { class: 'menu-top' });
    const btn = el('button', { 'aria-haspopup': 'true', role: 'menuitem' }, m.label);
    const menu = el('div', { class: 'menu', role: 'menu', 'aria-label': m.label });
    top.append(btn, menu);
    const open = () => { closeAll(); renderItems(menu, m.items); top.classList.add('open'); openTop = top; };
    btn.addEventListener('click', e => { e.stopPropagation(); top.classList.contains('open') ? closeAll() : open(); });
    btn.addEventListener('pointerenter', () => { if (openTop && openTop !== top) open(); });
    bar.append(top);
    m.open = open;
  }
  document.addEventListener('pointerdown', e => { if (!e.target.closest('.menubar')) closeAll(); });
  app.closeMenus = closeAll;
}
function updateMenus() {
  // Re-render of open menus happens on open; toolbar button states here.
  const st = app.state;
  if (!st) return;
  for (const b of document.querySelectorAll('#common-toolbar [data-enable]')) {
    const f = { sel: () => !!st.selection, undo: () => st.history.applied > 0, redo: () => st.history.applied < st.history.items.length }[b.dataset.enable];
    b.disabled = !f();
  }
  for (const b of document.querySelectorAll('#common-toolbar [data-toggle]')) b.setAttribute('aria-pressed', String(!!app.menuState[b.dataset.toggle]));
}

function buildCommonToolbar() {
  const bar = $('common-toolbar');
  const b = (ic, label, fn, extra = {}) => {
    const n = el('button', { class: 'icon-button', title: label, 'aria-label': label, ...extra }, icon(ic, 16));
    n.addEventListener('click', fn);
    bar.append(n);
  };
  const sep = () => bar.append(el('span', { class: 'sep' }));
  b('file.new', 'New (Ctrl+N)', fileNew); b('file.open', 'Open (Ctrl+O)', fileOpen); b('file.save', 'Save (Ctrl+S)', () => fileSave(false)); sep();
  b('edit.cut', 'Cut (Ctrl+X)', () => copy('cut'), { 'data-enable': 'sel' }); b('edit.copy', 'Copy (Ctrl+C)', () => copy('copy'), { 'data-enable': 'sel' }); b('edit.paste', 'Paste (Ctrl+V)', () => pasteFromMenu('layer'));
  b('image.crop', 'Crop to Selection (Ctrl+Shift+X)', () => run('crop'), { 'data-enable': 'sel' }); b('edit.deselect', 'Deselect (Ctrl+D)', () => run('deselect'), { 'data-enable': 'sel' }); sep();
  b('edit.undo', 'Undo (Ctrl+Z)', () => run('undo'), { 'data-enable': 'undo' }); b('edit.redo', 'Redo (Ctrl+Y)', () => run('redo'), { 'data-enable': 'redo' }); sep();
  b('view.grid', 'Pixel Grid', togglePixelGrid, { 'data-toggle': 'pixelGrid' }); b('view.rulers', 'Rulers', toggleRulers, { 'data-toggle': 'rulers' });
}

function toggleBar(key, id) {
  app.menuState[key] = !app.menuState[key];
  $(id).hidden = !app.menuState[key];
  app.view.resize();
}
function togglePixelGrid() { app.menuState.pixelGrid = !app.menuState.pixelGrid; app.view.pixelGrid = app.menuState.pixelGrid; app.view.changed(); updateMenus(); }
function toggleRulers() { app.menuState.rulers = !app.menuState.rulers; $('ruler-h').hidden = $('ruler-v').hidden = !app.menuState.rulers; app.view.resize(); drawRulers(); updateMenus(); }

function drawRulers() {
  if (!app.menuState.rulers) return;
  const v = app.view, d = window.devicePixelRatio || 1;
  for (const [id, horiz] of [['ruler-h', true], ['ruler-v', false]]) {
    const cv = $(id).firstChild, ctx = cv.getContext('2d');
    cv.width = cv.clientWidth * d; cv.height = cv.clientHeight * d;
    ctx.setTransform(d, 0, 0, d, 0, 0);
    ctx.clearRect(0, 0, cv.width, cv.height);
    ctx.fillStyle = ctx.strokeStyle = getComputedStyle(document.body).color;
    ctx.font = '9px system-ui';
    const len = horiz ? cv.clientWidth : cv.clientHeight;
    const step = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000].find(s => s * v.zoom >= 50) || 10000;
    const a = horiz ? v.toImage(0, 0).x : v.toImage(0, 0).y;
    const start = Math.floor(a / step) * step;
    ctx.beginPath();
    for (let i = start; ; i += step / 5) {
      const p = horiz ? v.toScreen(i, 0).x : v.toScreen(0, i).y;
      if (p > len) break;
      const major = Math.abs(i / step - Math.round(i / step)) < 1e-6;
      const t = major ? 18 : 5;
      if (horiz) { ctx.moveTo(Math.round(p) + 0.5, 18 - t); ctx.lineTo(Math.round(p) + 0.5, 18); }
      else { ctx.moveTo(18 - t, Math.round(p) + 0.5); ctx.lineTo(18, Math.round(p) + 0.5); }
      if (major) {
        if (horiz) ctx.fillText(String(Math.round(i)), p + 2, 9);
        else { ctx.save(); ctx.translate(9, p + 2); ctx.rotate(Math.PI / 2); ctx.fillText(String(Math.round(i)), 0, 0); ctx.restore(); }
      }
    }
    ctx.globalAlpha = 0.5;
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
}

// ------------------------------------------------------------------ canvas input

const drag = { active: false, mode: null, last: null, button: 0, lasso: null, zoomStart: null };
let spaceDown = false;

function imagePoint(e) {
  const r = app.view.host.getBoundingClientRect();
  const p = app.view.toImage(e.clientX - r.left, e.clientY - r.top);
  return { x: Math.floor(p.x), y: Math.floor(p.y), fx: p.x, fy: p.y };
}
function pointerArgs(e, phase, p) {
  const pen = e.pointerType === 'pen';
  return {
    phase, x: p.x, y: p.y, button: e.button < 0 ? 0 : e.button,
    shift: e.shiftKey, ctrl: e.ctrlKey || e.metaKey, alt: e.altKey,
    pressure: pen ? Math.max(0.05, e.pressure || 0) : 1,
    handleRadius: 6 / app.view.zoom,
  };
}
/** Distort brush held still: keep applying it (one request in flight at a time). */
function holdRepeat(args) {
  if (drag.hold) clearInterval(drag.hold.timer);
  const h = { args, busy: false };
  h.timer = setInterval(async () => {
    if (!drag.active || drag.mode !== 'engine' || app.tool !== 'distort') { clearInterval(h.timer); if (drag.hold === h) drag.hold = null; return; }
    if (h.busy) return;
    h.busy = true;
    try { await run('pointer', h.args); } finally { h.busy = false; }
  }, 60);
  drag.hold = h;
}

function updateViewCursor() {
  const host = app.view?.host;
  if (!host) return;
  const t = TOOLS.find(x => x.id === app.tool);
  host.style.cursor = spaceDown || app.tool === 'pan' ? (drag.mode === 'pan' ? 'grabbing' : 'grab')
    : app.tool === 'zoom' ? 'zoom-in' : app.tool === 'text' ? 'text' : app.tool === 'perspective' ? 'move' : t?.cursor === 'brush' ? 'none' : 'crosshair';
}

function wireCanvas() {
  const host = app.view.host;
  host.addEventListener('contextmenu', e => e.preventDefault());
  host.addEventListener('pointerdown', async e => {
    activateWindow(null);
    host.focus();
    app.closeMenus?.();
    const p = imagePoint(e);
    host.setPointerCapture(e.pointerId);
    drag.active = true;
    drag.button = e.button;
    drag.last = { x: e.clientX, y: e.clientY };
    if (e.button === 1 || spaceDown || app.tool === 'pan') { drag.mode = 'pan'; updateViewCursor(); return; }
    if (app.tool === 'zoom') {
      if (e.button === 2) { app.view.zoomStep(-1, e.clientX - host.getBoundingClientRect().left, e.clientY - host.getBoundingClientRect().top); drag.active = false; return; }
      drag.mode = 'zoom'; drag.zoomStart = p; return;
    }
    if (app.tool === 'text') {
      e.preventDefault(); // no compatibility mousedown: keeps focus in the text input
      if (e.button !== 0) { drag.active = false; return; }
      if (app.textEdit && nearGrip(e, 'rotateGrip')) { drag.mode = 'text-rotate'; textRotateStart(p); return; }
      drag.mode = textPointerDown(p) ? 'text-move' : null;
      if (!drag.mode) drag.active = false;
      return;
    }
    // The move and rotate grips of a pending line/curve.
    if ((app.tool === 'line' || app.tool === 'gradient') && e.button === 0 && nearGrip(e)) {
      drag.mode = 'shape-move'; drag.shapeStart = p; drag.shapeStarted = false;
      return;
    }
    if ((app.tool === 'line' || app.tool === 'gradient') && e.button === 0 && nearGrip(e, 'rotateGrip')) {
      const pd = app.state.pending;
      const pts = pd.kind === 'gradient' ? [pd.a, pd.b] : pd.points, xs = pts.map(q => q[0] + 0.5), ys = pts.map(q => q[1] + 0.5);
      const c = { x: (Math.min(...xs) + Math.max(...xs)) / 2, y: (Math.min(...ys) + Math.max(...ys)) / 2 };
      drag.mode = 'shape-rotate'; drag.rot = { c, a0: Math.atan2(p.fy - c.y, p.fx - c.x), started: false };
      return;
    }
    if (e.button !== 0 && e.button !== 2) { drag.active = false; return; }
    drag.mode = 'engine';
    if (app.tool === 'lasso') { drag.lasso = [[p.x, p.y]]; app.view.lasso = drag.lasso; }
    run('pointer', pointerArgs(e, 'down', p));
    if (app.tool === 'distort') holdRepeat(pointerArgs(e, 'move', p));
  });
  host.addEventListener('pointermove', e => {
    const p = imagePoint(e);
    const st = app.state;
    if (st && p.fx >= 0 && p.fy >= 0 && p.fx < st.width && p.fy < st.height) $('status-pos').textContent = `${p.x}, ${p.y}`;
    else $('status-pos').textContent = '';
    const t = TOOLS.find(x => x.id === app.tool);
    app.view.cursor = t?.cursor === 'brush' && !spaceDown ? { x: p.x, y: p.y, d: st?.settings.brush ?? 1 } : null;
    app.view.requestOverlay();
    if (!drag.active) { hoverCursor(e, p); return; }
    if (drag.mode === 'text-move') { textMove(p); return; }
    if (drag.mode === 'text-rotate') { textRotate(p, e.shiftKey); return; }
    if (drag.mode === 'shape-rotate') {
      const r = drag.rot;
      let a = Math.atan2(p.fy - r.c.y, p.fx - r.c.x) - r.a0;
      if (e.shiftKey) a = Math.round(a / (Math.PI / 12)) * (Math.PI / 12);
      run('shape_rotate', { phase: r.started ? 'move' : 'start', angle: a });
      r.started = true;
      return;
    }
    if (drag.mode === 'shape-move') {
      run('shape_translate', { phase: drag.shapeStarted ? 'move' : 'start', dx: p.fx - drag.shapeStart.fx, dy: p.fy - drag.shapeStart.fy });
      drag.shapeStarted = true;
      return;
    }
    if (drag.mode === 'pan') {
      app.view.panBy(e.clientX - drag.last.x, e.clientY - drag.last.y);
      drag.last = { x: e.clientX, y: e.clientY };
      return;
    }
    if (drag.mode === 'zoom') { app.view.zoomRect = [drag.zoomStart.fx, drag.zoomStart.fy, p.fx, p.fy]; app.view.requestOverlay(); return; }
    if (drag.mode === 'engine') {
      if (drag.lasso) { const l = drag.lasso[drag.lasso.length - 1]; if (l[0] !== p.x || l[1] !== p.y) drag.lasso.push([p.x, p.y]); }
      const args = pointerArgs(e, 'move', p);
      if (drag.hold) drag.hold.args = args;
      run('pointer', args);
    }
  });
  const end = e => {
    if (!drag.active) return;
    drag.active = false;
    const p = imagePoint(e);
    if (drag.mode === 'zoom') {
      const r = app.view.zoomRect;
      app.view.zoomRect = null;
      if (r && Math.abs(r[2] - r[0]) * app.view.zoom > 4 && Math.abs(r[3] - r[1]) * app.view.zoom > 4) app.view.zoomToRect(Math.min(r[0], r[2]), Math.min(r[1], r[3]), Math.abs(r[2] - r[0]), Math.abs(r[3] - r[1]));
      else { const hr = host.getBoundingClientRect(); app.view.zoomStep(1, e.clientX - hr.left, e.clientY - hr.top); }
    } else if (drag.mode === 'text-move' || drag.mode === 'text-rotate') {
      if (drag.mode === 'text-move') textMove(p); else textRotate(p, e.shiftKey);
      app.textEdit?.input.focus();
    } else if (drag.mode === 'engine') {
      if (drag.hold) { clearInterval(drag.hold.timer); drag.hold = null; }
      run('pointer', { ...pointerArgs(e, 'up', p), button: drag.button });
      drag.lasso = null;
      app.view.lasso = null;
    }
    drag.mode = null;
    updateViewCursor();
  };
  host.addEventListener('pointerup', end);
  host.addEventListener('pointercancel', end);
  host.addEventListener('pointerleave', () => { app.view.cursor = null; app.view.requestOverlay(); });
  host.addEventListener('wheel', e => {
    e.preventDefault();
    const r = host.getBoundingClientRect();
    if (e.ctrlKey || e.metaKey) app.view.zoomStep(e.deltaY < 0 ? 1 : -1, e.clientX - r.left, e.clientY - r.top);
    else if (e.shiftKey) app.view.panBy(-e.deltaY, 0);
    else app.view.panBy(-e.deltaX, -e.deltaY);
  }, { passive: false });
}

// ------------------------------------------------------------------ editable text and lines
//
// Text is typed straight onto the canvas. Each change is rendered by the
// browser and sent as a non-destructive preview (`text_preview`); Enter
// commits it, Shift+Enter starts a new line, Esc cancels, and any other image
// action commits it (the engine does that itself). Lines/curves stay
// editable in the engine the same way; the UI draws their handles.

const TEXT_KEY = 'openpdn.text';
app.textSpec = (() => { try { return { ...TE.DEFAULT_SPEC, ...JSON.parse(localStorage.getItem(TEXT_KEY) || '{}') }; } catch { return { ...TE.DEFAULT_SPEC }; } })();
app.textEdit = null; // { x, y, text, live, input, moveFrom }

function textInputEl() {
  let ta = document.getElementById('text-input');
  if (ta) return ta;
  ta = el('textarea', { id: 'text-input', class: 'text-input', 'aria-label': 'Text', spellcheck: 'false', autocomplete: 'off', autocapitalize: 'off' });
  ta.addEventListener('input', textRender);
  ta.addEventListener('keydown', e => {
    const nudge = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
    if (nudge && e.altKey) {
      e.preventDefault();
      const k = e.shiftKey ? 10 : 1;
      app.textEdit.x += nudge[0] * k; app.textEdit.y += nudge[1] * k;
      textRender();
    } else if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); textCommit(); }
    else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); textCancel(); }
    else setTimeout(textRender); // caret moves (arrows, Home/End)
  });
  ta.addEventListener('select', () => textRender());
  app.view.host.append(ta);
  return ta;
}

/** Starts typing at p, or (returns true) begins dragging the text being edited. */
function textPointerDown(p) {
  const te = app.textEdit;
  if (te) {
    if (insideText(te, p)) {
      te.moveFrom = { px: p.fx, py: p.fy, x: te.x, y: te.y, moved: false };
      return true;
    }
    textCommit();
  }
  const input = textInputEl();
  app.textEdit = { x: p.x, y: p.y, angle: 0, text: '', live: false, input, seq: 0, ack: 0 };
  input.value = '';
  input.focus();
  requestAnimationFrame(() => input.focus());
  textRender();
  status(TOOL_HINTS.text);
  return false;
}

/** Whether image point p is on the text box (in the text's rotated frame). */
function insideText(te, p) {
  const L = TE.layout(app.textSpec, te.text, te.x, te.y);
  const q = TE.rotateAbout(p.fx, p.fy, te.x, te.y, -te.angle);
  const m = 6 / app.view.zoom, b = L.box;
  return q.x >= b.x - m && q.x <= b.x + b.w + m && q.y >= b.y - m && q.y <= b.y + b.h + m;
}

/** Rotation keeps the middle of the text box in place. */
function textRotateStart(p) {
  const te = app.textEdit;
  const L = TE.layout(app.textSpec, te.text, te.x, te.y);
  const o = { x: L.box.x + L.box.w / 2 - te.x, y: L.box.y + L.box.h / 2 - te.y }; // centre − anchor, unrotated
  const c = TE.rotateAbout(te.x + o.x, te.y + o.y, te.x, te.y, te.angle);
  te.rot = { c, o, a0: te.angle, p0: Math.atan2(p.fy - c.y, p.fx - c.x) };
}
function textRotate(p, snap) {
  const te = app.textEdit;
  if (!te?.rot) return;
  const { c, o, a0, p0 } = te.rot;
  let a = a0 + Math.atan2(p.fy - c.y, p.fx - c.x) - p0;
  if (snap) a = Math.round(a / (Math.PI / 12)) * (Math.PI / 12);
  a = Math.atan2(Math.sin(a), Math.cos(a));
  if (Math.abs(a) < 1e-4) a = 0;
  const off = TE.rotateAbout(o.x, o.y, 0, 0, a);
  te.angle = a;
  te.x = c.x - off.x; te.y = c.y - off.y;
  if (a === 0) { te.x = Math.round(te.x); te.y = Math.round(te.y); }
  textRender();
}

function textMove(p) {
  const te = app.textEdit;
  if (!te?.moveFrom) return;
  const f = te.moveFrom;
  const dx = Math.round(p.fx - f.px), dy = Math.round(p.fy - f.py);
  if (!f.moved && Math.abs(dx) + Math.abs(dy) === 0) return;
  f.moved = true;
  te.x = f.x + dx; te.y = f.y + dy;
  textRender();
}

function textRender() {
  const te = app.textEdit;
  if (!te) { app.view.textBox = null; app.view.requestOverlay(); return; }
  te.text = te.input.value;
  const L = TE.layout(app.textSpec, te.text, te.x, te.y);
  const caret = TE.caretAt(L, te.input.selectionEnd ?? te.text.length);
  app.view.textBox = { box: L.box, caret, angle: te.angle, ax: te.x, ay: te.y };
  app.view.caretOn = true;
  app.view.requestOverlay();
  // Keep the (invisible) input at the caret so IME pop-ups appear there.
  const cb = TE.rotateAbout(caret.x, caret.y + caret.h, te.x, te.y, te.angle);
  const sp = app.view.toScreen(cb.x, cb.y);
  te.input.style.left = `${Math.round(sp.x)}px`;
  te.input.style.top = `${Math.round(sp.y)}px`;
  const st = app.state?.settings;
  const img = st && TE.render(app.textSpec, L, D.hexToRgba(st.fg), st.aa, te.angle, te.x, te.y);
  if (img) {
    te.live = true;
    const n = ++te.seq;
    call('text_preview', { x: img.x, y: img.y, w: img.w, h: img.h }, img.rgba).then(() => { te.ack = Math.max(te.ack, n); }, () => {});
  } else if (te.live) {
    te.live = false;
    run('text_cancel');
  }
}

function textEnd() {
  if (app.textEdit) app.textEdit.input.blur();
  app.textEdit = null;
  app.view.textBox = null;
  app.view.requestOverlay();
}
function textCommit() {
  const te = app.textEdit;
  if (!te) return;
  textEnd();
  if (te.live) run('text_commit');
}
function textCancel() {
  const te = app.textEdit;
  if (!te) return;
  textEnd();
  if (te.live) run('text_cancel');
}

/** Called with each state: follows engine-side commits and colour changes. */
function syncPending(st, prev) {
  const pend = st.pending;
  app.view.shapeEdit = ['line', 'shape', 'gradient', 'quad'].includes(pend?.kind) ? pend : null;
  // A gradient stop was selected: show its color as the primary color, so
  // the Colors window edits it from there.
  if (pend?.kind === 'gradient' && pend.selected != null) {
    const was = prev?.pending?.kind === 'gradient' ? prev.pending.selected : null;
    const c = pend.stops[pend.selected]?.[1];
    if (pend.selected !== was && c && c.toUpperCase() !== String(st.settings.fg).toUpperCase()) run('colors', { fg: c });
  }
  if (opt.gradientReverse) opt.gradientReverse.disabled = pend?.kind !== 'gradient';
  // The engine committed the text (another action): end the session here too.
  // (Only once every preview sent has been answered: older replies predate it.)
  const te = app.textEdit;
  if (te?.live && te.ack === te.seq && pend?.kind !== 'text') textEnd();
  else if (app.textEdit && (prev?.settings?.fg !== st.settings.fg || prev?.settings?.aa !== st.settings.aa)) textRender();
}

function setTextSpec(patch) {
  app.textSpec = { ...app.textSpec, ...patch };
  try { localStorage.setItem(TEXT_KEY, JSON.stringify(app.textSpec)); } catch { /* ignore */ }
  syncTextOptions();
  if (app.textEdit) { textRender(); app.textEdit.input.focus(); }
}

function nearGrip(e, which = 'shapeGrip') {
  const g = app.view[which];
  if (!g || (which === 'shapeGrip' && !['line', 'gradient'].includes(app.state?.pending?.kind))) return false;
  const r = app.view.host.getBoundingClientRect();
  return Math.hypot(e.clientX - r.left - g.x, e.clientY - r.top - g.y) <= 12;
}

/** Cursor over a pending box shape: resize arrows on handles (following the rotation), grab, move. */
function boxCursor(e, g) {
  const r = app.view.host.getBoundingClientRect();
  const mx = e.clientX - r.left, my = e.clientY - r.top;
  if (nearGrip(e, 'rotateGrip')) return 'grab';
  const cos = Math.cos(g.angle), sin = Math.sin(g.angle);
  const W = (u, v) => app.view.toScreen(g.cx + u * cos - v * sin, g.cy + u * sin + v * cos);
  for (const [vx, vy] of g.vertices ?? []) {
    const q = app.view.toScreen(vx, vy);
    if (Math.hypot(mx - q.x, my - q.y) <= 8) return 'move';
  }
  const names = ['ew-resize', 'nwse-resize', 'ns-resize', 'nesw-resize'];
  for (const [a, b] of [[-1, -1], [0, -1], [1, -1], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0]]) {
    const q = W(a * g.w / 2, b * g.h / 2);
    if (Math.hypot(mx - q.x, my - q.y) <= 8) {
      const ang = Math.atan2(b, a) + g.angle; // handle direction on screen
      const k = ((Math.round(ang / (Math.PI / 4)) % 4) + 4) % 4;
      return names[k];
    }
  }
  const pt = app.view.toImage(mx, my);
  const dx = pt.x - g.cx, dy = pt.y - g.cy;
  const u = dx * cos + dy * sin, v = -dx * sin + dy * cos;
  return Math.abs(u) <= g.w / 2 && Math.abs(v) <= g.h / 2 ? 'move' : 'crosshair';
}

/** Over a pending gradient: move on stops and grips, add (copy) on the line. */
function gradientCursor(e, g) {
  if (nearGrip(e, 'rotateGrip')) return 'grab';
  if (nearGrip(e)) return 'move';
  const r = app.view.host.getBoundingClientRect();
  const m = { x: e.clientX - r.left, y: e.clientY - r.top };
  const A = app.view.toScreen(g.a[0] + 0.5, g.a[1] + 0.5), B = app.view.toScreen(g.b[0] + 0.5, g.b[1] + 0.5);
  for (const [t] of g.stops) if (Math.hypot(m.x - (A.x + (B.x - A.x) * t), m.y - (A.y + (B.y - A.y) * t)) <= 8) return 'move';
  const dx = B.x - A.x, dy = B.y - A.y, len2 = dx * dx + dy * dy || 1;
  const t = Math.max(0, Math.min(1, ((m.x - A.x) * dx + (m.y - A.y) * dy) / len2));
  return Math.hypot(m.x - A.x - dx * t, m.y - A.y - dy * t) <= 6 ? 'copy' : 'crosshair';
}

/** Over a perspective transform: corners and edges are handles, anywhere else moves it. */
function quadCursor(e, q) {
  const r = app.view.host.getBoundingClientRect();
  const m = { x: e.clientX - r.left, y: e.clientY - r.top };
  const P = q.corners.map(([x, y]) => app.view.toScreen(x, y));
  if (P.some(p => Math.hypot(m.x - p.x, m.y - p.y) <= 8)) return 'crosshair';
  for (let i = 0; i < 4; i++) {
    const a = P[i], b = P[(i + 1) % 4];
    if (Math.hypot(m.x - (a.x + b.x) / 2, m.y - (a.y + b.y) / 2) <= 8) return Math.abs(b.x - a.x) > Math.abs(b.y - a.y) ? 'ew-resize' : 'ns-resize';
  }
  return 'move';
}

/** Cursor feedback over editable lines and text. */
function hoverCursor(e, p) {
  const host = app.view.host;
  if (spaceDown) return;
  if (app.tool === 'line' && app.state?.pending?.kind === 'line') {
    const r = host.getBoundingClientRect();
    const onPoint = app.state.pending.points.some(([x, y]) => { const s = app.view.toScreen(x + 0.5, y + 0.5); return Math.hypot(e.clientX - r.left - s.x, e.clientY - r.top - s.y) <= 7; });
    host.style.cursor = onPoint || nearGrip(e) ? 'move' : 'crosshair';
    if (nearGrip(e, 'rotateGrip')) host.style.cursor = 'grab';
  } else if (app.tool === 'text' && app.textEdit) {
    host.style.cursor = nearGrip(e, 'rotateGrip') ? 'grab' : insideText(app.textEdit, p) ? 'move' : 'text';
  } else if (app.state?.pending?.kind === 'shape') {
    host.style.cursor = boxCursor(e, app.state.pending);
  } else if (app.tool === 'gradient' && app.state?.pending?.kind === 'gradient') {
    host.style.cursor = gradientCursor(e, app.state.pending);
  } else if (app.tool === 'perspective' && app.state?.pending?.kind === 'quad') {
    host.style.cursor = quadCursor(e, app.state.pending);
  } else if (['line', 'text', 'shape', 'gradient', 'perspective'].includes(app.tool)) updateViewCursor();
}

// ------------------------------------------------------------------ keyboard

function keyName(e) {
  const parts = [];
  if (e.ctrlKey || e.metaKey) parts.push('Ctrl');
  if (e.shiftKey) parts.push('Shift');
  if (e.altKey) parts.push('Alt');
  let k = e.key;
  if (k === '+' || k === '=') k = 'Plus';
  else if (k === '-' || k === '_') k = 'Minus';
  else if (k === ' ') k = 'Space';
  else if (k.length === 1) k = k.toUpperCase();
  parts.push(k);
  return parts.join('+');
}

function allShortcuts() {
  const map = new Map();
  const walk = items => {
    for (const it of items) {
      if (it === '-') continue;
      if (it.key && it.action) map.set(it.key, it);
      if (it.submenu) walk(it.submenu());
    }
  };
  for (const m of menuSpec()) walk(m.items);
  map.set('Ctrl+0', { action: () => app.view.fit() });
  map.set('Ctrl+1', { action: () => app.view.actualSize() });
  map.set('Escape', { action: () => run('deselect') });
  map.set('Ctrl+Alt+PageDown', { action: () => cycleDoc(1) });
  map.set('Ctrl+Alt+PageUp', { action: () => cycleDoc(-1) });
  return map;
}

function wireKeyboard() {
  document.addEventListener('keydown', e => {
    if (document.querySelector('.modal-backdrop')) return;
    // Typing on the canvas: editing keys belong to the text.
    if (e.target.id === 'text-input' && (!(e.ctrlKey || e.metaKey) || /^[zyacvx]$/i.test(e.key) || e.key.startsWith('Arrow') || e.key === 'Home' || e.key === 'End' || e.key === 'Backspace' || e.key === 'Delete')) return;
    // A pending line or shape: Enter finishes it, Esc throws it away, arrows nudge it.
    const pk = app.state?.pending?.kind;
    if (['line', 'shape', 'gradient', 'quad'].includes(pk) && !e.target.closest('input, select, textarea')) {
      if (e.key === 'Enter') { e.preventDefault(); run('shape_commit'); return; }
      if (pk === 'gradient' && (e.key === 'Delete' || e.key === 'Backspace') && app.state.pending.selected != null) { e.preventDefault(); run('pending_delete'); return; }
      if (e.key === 'Escape') { e.preventDefault(); run('shape_cancel'); return; }
      const d = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
      if (d && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault();
        const k = e.shiftKey ? 10 : 1;
        run('shape_nudge', { dx: d[0] * k, dy: d[1] * k });
        return;
      }
    }
    const typing = e.target.closest('input, select, textarea');
    if (e.key === ' ' && !typing) { if (!spaceDown) { spaceDown = true; updateViewCursor(); } e.preventDefault(); return; }
    if (typing && !(e.ctrlKey || e.metaKey)) return;
    const name = keyName(e);
    // Paste goes through the 'paste' event so we get the clipboard without a permission prompt.
    if (/^Ctrl(\+Shift|\+Alt)?\+V$/.test(name)) { app.pasteMode = name === 'Ctrl+V' ? 'layer' : name === 'Ctrl+Shift+V' ? 'new_layer' : 'new_image'; return; }
    const sc = allShortcuts().get(name);
    if (sc) { e.preventDefault(); app.closeMenus?.(); sc.action(); return; }
    if (e.ctrlKey || e.metaKey || e.altKey) {
      // Alt+letter opens a menu.
      if (e.altKey && !e.ctrlKey && /^[a-z]$/i.test(e.key)) {
        const m = menuDefs.find(d => d.label[0].toLowerCase() === e.key.toLowerCase());
        if (m) { e.preventDefault(); m.open(); }
      }
      return;
    }
    if (typing) return;
    const k = e.key.toUpperCase();
    if (k === 'X') { run('swap_colors'); return; }
    if (k === 'D') { run('reset_colors'); return; }
    if (e.key === '[' || e.key === ']') { run('setting', { key: 'brush', value: Math.max(1, (app.state.settings.brush) + (e.key === ']' ? 1 : -1)) }); return; }
    if (e.key.startsWith('Arrow') && (app.tool === 'move' || app.tool === 'move-selection')) {
      e.preventDefault();
      nudge({ ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key]);
      return;
    }
    const group = HOTKEY_ORDER[k] || TOOLS.filter(t => t.key === k).map(t => t.id);
    if (group.length) {
      const i = group.indexOf(app.tool);
      selectTool(group[(i + 1) % group.length]);
    }
  });
  document.addEventListener('keyup', e => { if (e.key === ' ') { spaceDown = false; updateViewCursor(); } });
  document.addEventListener('paste', async e => {
    if (e.target.closest?.('input, textarea')) return;
    const file = [...(e.clipboardData?.files || [])].find(f => f.type.startsWith('image/'));
    if (!file) { status('Clipboard has no image.'); return; }
    e.preventDefault();
    await pasteBlob(file, app.pasteMode || 'layer');
    app.pasteMode = null;
  });
}

async function nudge([dx, dy]) {
  const st = app.state;
  const sx = Math.min(Math.max(Math.floor(st.width / 2), 1), st.width - 2), sy = Math.min(Math.max(Math.floor(st.height / 2), 1), st.height - 2);
  const base = { button: 0, shift: false, ctrl: false, alt: false, pressure: 1, handleRadius: 0 };
  await run('pointer', { ...base, phase: 'down', x: sx, y: sy });
  await run('pointer', { ...base, phase: 'move', x: sx + dx, y: sy + dy });
  await run('pointer', { ...base, phase: 'up', x: sx + dx, y: sy + dy });
}

// ------------------------------------------------------------------ commands

// ------------------------------------------------------------------ accessibility

function setA11y(patch, persist = true) {
  const oldScale = app.a11y?.uiScale;
  app.a11y = { ...app.a11y, ...patch };
  if (oldScale !== undefined && oldScale !== app.a11y.uiScale) requestAnimationFrame(() => { rescaleWindows(); app.view.resize(); });
  if (persist) A11y.save(app.a11y);
  A11y.applyDocument(app.a11y);
  const f = A11y.filterById(app.a11y.filter);
  if (app.view.filter?.id !== f.id) app.view.setFilter(f);
  app.view.reduceMotion = !!app.a11y.reduceMotion;
  app.view.selectionTint = A11y.tintCss(app.a11y);
  app.view.requestOverlay();
  // Status bar reminder while a view filter is on (click to turn it off).
  const cell = $('status-filter');
  cell.hidden = f.id === 'none';
  if (f.id !== 'none') {
    const b = el('button', { class: 'filter-chip', title: `View filter: ${f.label}. The image itself is unchanged. Click to turn off.` },
      f.id === 'falsecolor' ? el('i', { class: 'turbo-ramp', 'aria-hidden': 'true' }) : null, el('span', {}, f.label.replace(/ \(.*\)$/, '')), icon('window.close', 10));
    b.addEventListener('click', () => setA11y({ filter: 'none' }));
    cell.replaceChildren(b);
  }
}

/** Selection highlight: on/off, colour and opacity, previewed live. */
function tintControls() {
  const on = el('input', { type: 'checkbox', checked: !!app.a11y.tint });
  const color = el('input', { type: 'color', value: app.a11y.tintColor, 'aria-label': 'Highlight color' });
  const op = el('input', { type: 'range', min: 5, max: 80, value: app.a11y.tintOpacity, 'aria-label': 'Highlight opacity' });
  const opText = el('span', { class: 'unit', style: 'width:34px' });
  const reset = el('button', { class: 'icon-button small', title: 'Default (light blue, 20%)', 'aria-label': 'Reset highlight' }, icon('color.reset', 12));
  const sync = () => { opText.textContent = `${op.value}%`; color.disabled = op.disabled = !on.checked; };
  on.addEventListener('change', () => { setA11y({ tint: on.checked }, false); sync(); });
  color.addEventListener('input', () => setA11y({ tintColor: color.value }, false));
  op.addEventListener('input', () => { setA11y({ tintOpacity: +op.value }, false); sync(); });
  reset.addEventListener('click', () => {
    setA11y({ ...A11y.TINT_DEFAULT }, false);
    on.checked = true; color.value = A11y.TINT_DEFAULT.tintColor; op.value = A11y.TINT_DEFAULT.tintOpacity; sync();
  });
  sync();
  return el('div', {},
    el('label', { class: 'check' }, on, 'Highlight selected areas'),
    el('div', { class: 'tint-row indent-hint' }, color, op, opText, reset),
    el('p', { class: 'hint indent-hint' }, 'Tints the inside of the selection so it stands out. On screen only; the image is not changed.'));
}

/** Floating windows fade when not in use: on/off and how see-through. */
function windowFadeControls() {
  const on = el('input', { type: 'checkbox', checked: !!app.a11y.winFade });
  const op = el('input', { type: 'range', min: 20, max: 95, step: 5, value: app.a11y.winOpacity, 'aria-label': 'Faded window opacity' });
  const txt = el('span', { class: 'unit', style: 'width:34px' });
  const sync = () => { txt.textContent = `${op.value}%`; op.disabled = !on.checked; };
  on.addEventListener('change', () => { setA11y({ winFade: on.checked }, false); sync(); });
  op.addEventListener('input', () => { setA11y({ winOpacity: +op.value }, false); sync(); });
  sync();
  return el('div', {},
    el('label', { class: 'check' }, on, 'Fade windows that are not in use'),
    el('div', { class: 'tint-row indent-hint' }, el('span', { class: 'unit' }, 'Opacity'), op, txt),
    el('p', { class: 'hint indent-hint' }, 'Tools, Colors, Layers and History become see-through until you point at or click them.'));
}

/** Size of the interface (menus, toolbars, windows, dialogs); the image is unaffected. */
function uiScaleControls() {
  const sel = el('select', { 'aria-label': 'Interface size' }, [75, 90, 100, 110, 125, 150, 175, 200].map(v => el('option', { value: v, selected: v === app.a11y.uiScale }, `${v}%`)));
  sel.addEventListener('change', () => setA11y({ uiScale: +sel.value }, false));
  return el('div', { class: 'field' }, el('label', {}, 'Interface size'), sel);
}

function accessibilityDialog() {
  const before = { ...app.a11y };
  return D.modal('Accessibility', body => {
    const check = (key, label, hint) => {
      const cb = el('input', { type: 'checkbox', checked: !!app.a11y[key] });
      cb.addEventListener('change', () => setA11y({ [key]: cb.checked }, false));
      return el('div', {}, el('label', { class: 'check' }, cb, label), hint ? el('p', { class: 'hint indent-hint' }, hint) : null);
    };
    const sel = el('select', { 'aria-label': 'View filter' }, A11y.FILTERS.filter(f => f !== '-').map(f => el('option', { value: f.id, selected: f.id === app.a11y.filter }, f.label)));
    const hint = el('p', { class: 'hint indent-hint' });
    const syncHint = () => { hint.textContent = A11y.filterById(sel.value).hint ?? 'Shows the image as it is.'; };
    sel.addEventListener('change', () => { setA11y({ filter: sel.value }, false); syncHint(); });
    syncHint();
    body.append(
      el('div', { class: 'field' }, el('label', {}, 'View filter'), sel), hint,
      el('p', { class: 'hint' }, 'View filters only change how the image is shown on screen, to check how it looks to people with different kinds of color vision. Saved files are not affected.'),
      tintControls(),
      windowFadeControls(),
      uiScaleControls(),
      check('dimDialogs', 'Dim the image behind dialogs', 'Helps tell the dialog apart from the image, at the cost of judging adjustments against a darker view.'),
      check('reduceMotion', 'Reduce motion', 'Draws the selection outline without the moving "marching ants" animation.'),
      check('strongFocus', 'Strong keyboard focus outline', 'Draws a thick, high-contrast outline around the focused control.'));
  }, {
    width: 460,
    onOk: () => { A11y.save(app.a11y); return true; },
    onCancel: () => setA11y(before, false),
  });
}

// ------------------------------------------------------------------ document tabs

/** Merges the engine's document list into the tabs, keeping per-tab UI state. */
function syncDocs(list) {
  app.docs = list.map(n => ({ file: null, view: null, ...app.docs.find(d => d.id === n.id), id: n.id, name: n.name, dirty: n.dirty }));
  renderTabs();
}

function renderTabs() {
  const bar = $('tabbar');
  if (!bar) return;
  const tabs = app.docs.map(d => {
    const active = d.id === app.docId;
    const tab = el('div', { class: `tab${active ? ' active' : ''}${d.dirty ? ' dirty' : ''}`, role: 'tab', 'aria-selected': String(active), tabindex: active ? '0' : '-1',
      title: d.dirty ? `${d.name} (unsaved changes)` : d.name, 'data-doc': d.id });
    const close = el('button', { class: 'icon-button small tab-close', title: 'Close', 'aria-label': `Close ${d.name}` }, icon('window.close', 11));
    tab.append(el('span', { class: 'tab-dot', 'aria-hidden': 'true' }), el('span', { class: 'tab-name' }, d.name), close);
    close.addEventListener('click', e => { e.stopPropagation(); closeDoc(d.id); });
    tab.addEventListener('pointerdown', e => { if (e.button === 0 && !e.target.closest('button')) switchDoc(d.id); });
    tab.addEventListener('auxclick', e => { if (e.button === 1) { e.preventDefault(); closeDoc(d.id); } });
    tab.addEventListener('keydown', e => {
      const i = app.docs.indexOf(d);
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        const n = app.docs[(i + (e.key === 'ArrowRight' ? 1 : app.docs.length - 1)) % app.docs.length];
        switchDoc(n.id).then(() => bar.querySelector(`[data-doc="${n.id}"]`)?.focus());
      } else if (e.key === 'Delete') closeDoc(d.id);
    });
    return tab;
  });
  const add = el('button', { class: 'icon-button tab-add', title: 'New image (Ctrl+N)', 'aria-label': 'New image' }, icon('file.new', 14));
  add.addEventListener('click', fileNew);
  bar.replaceChildren(...tabs, add);
  bar.querySelector('.tab.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

async function switchDoc(id) {
  if (id === app.docId || app.state?.busy) return;
  await run('doc_switch', { doc: id });
}

function cycleDoc(step) {
  const i = app.docs.findIndex(d => d.id === app.docId);
  if (app.docs.length > 1) switchDoc(app.docs[(i + step + app.docs.length) % app.docs.length].id);
}

/** Closes a tab, offering to save it first when it has unsaved changes. */
async function closeDoc(id = app.docId) {
  const d = app.docs.find(x => x.id === id);
  if (!d) return false;
  if (d.dirty) {
    await switchDoc(id);
    if (!(await confirmDiscard())) return false;
  }
  await run('doc_close', { doc: id });
  return true;
}

async function closeAll() {
  for (const d of [...app.docs]) if (!(await closeDoc(d.id))) return;
}

/** True when the active tab is an untouched blank image, which Open can reuse. */
function isPristine() {
  const st = app.state;
  return !!st && !app.file && !st.dirty && st.name === 'Untitled' && (st.history?.items?.length ?? 1) <= 1;
}

/** `call` that runs an open/new command in a new tab (or in the pristine active one). */
function docCall(reuse = isPristine()) {
  return reuse ? call : (cmd, args, bin) => call('doc_add', { ...args, then: cmd }, bin);
}

async function confirmDiscard() {
  if (!app.state?.dirty) return true;
  const r = await D.message('Unsaved Changes', `Save changes to "${app.state.name}" before closing it?`, [['Save', 'save', true], ["Don't Save", 'discard'], ['Cancel', null]]);
  if (r === 'save') return await fileSave(false);
  return r === 'discard';
}

async function fileNew() {
  const r = await D.newImageDialog(app.state?.width ?? 800, app.state?.height ?? 600);
  if (!r) return;
  try { await docCall(false)('new', r); } catch { return; }
  app.view.fit();
}

async function fileOpen() {
  const picked = await io.pickOpenFile();
  if (picked) await openFile(picked.file, picked.handle);
}

async function openFile(file, handle) {
  // Already open? Just switch to it.
  const same = handle && app.docs.find(d => d.file?.handle && d.file.name === file.name);
  if (same && await same.file.handle.isSameEntry?.(handle)) { await switchDoc(same.id); return; }
  try {
    await io.openInto(docCall(), file);
    app.file = { name: file.name, handle, format: io.formatFor(file.name) };
    if (handle) io.addRecent(handle);
    app.view.fit();
    status(`Opened ${file.name}`);
  } catch (e) {
    D.message('Open', `Couldn't open "${file.name}": ${e.message}`);
  }
}

async function fileSave(saveAs) {
  try {
    const saved = await io.save(call, app.state, app.file, saveAs);
    if (!saved) return false;
    app.file = saved;
    await run('mark_saved', { name: saved.name });
    status(`Saved ${saved.name}`);
    return true;
  } catch (e) {
    if (e.name !== 'AbortError') D.message('Save', `Couldn't save: ${e.message}`);
    return false;
  }
}

function recentMenu() {
  const items = io.recent().map(r => ({ label: r.name, action: async () => {
    const f = await io.openRecent(r);
    if (f) await openFile(f.file, f.handle);
  } }));
  return items.length ? items : [{ label: '(empty)', enabled: () => false, action() {} }];
}

async function importLayer() {
  const picked = await io.pickOpenFile();
  if (picked) await importFile(picked.file);
}
async function importFile(file) {
  const img = await io.decodeForImport(file);
  let expand = false;
  if (img.w > app.state.width || img.h > app.state.height) {
    const r = await D.message('Import From File', 'The imported image is larger than the canvas. Expand the canvas to fit it?', [['Yes', 'yes', true], ['No', 'no'], ['Cancel', null]]);
    if (!r) return;
    expand = r === 'yes';
  }
  await run('import_layer', { name: file.name.replace(/\.[^.]+$/, ''), w: img.w, h: img.h, expand }, img.bytes);
}

async function copy(cmd) {
  try {
    const r = await call(cmd);
    await io.writeClipboard(r.json.w, r.json.h, r.bins.get('rgba')[0]);
  } catch (e) { if (e.message) status(e.message); }
}

async function pasteFromMenu(mode) {
  try {
    const blob = await io.readClipboardImage();
    if (!blob) { status('Clipboard has no image.'); return; }
    await pasteBlob(blob, mode);
  } catch {
    D.message('Paste', 'The browser blocked clipboard access from the menu. Press Ctrl+V (or Ctrl+Shift+V / Ctrl+Alt+V) instead.');
  }
}

async function pasteBlob(blob, mode) {
  const img = await io.decodeImage(blob);
  let expand = false;
  if (mode !== 'new_image' && (img.w > app.state.width || img.h > app.state.height)) {
    const r = await D.message('Paste', 'The image on the clipboard is larger than the canvas. Expand the canvas to fit it?', [['Yes', 'yes', true], ['No', 'no'], ['Cancel', null]]);
    if (!r) return;
    expand = r === 'yes';
  }
  const tl = app.view.visibleTopLeft();
  const args = { mode, w: img.w, h: img.h, x: tl.x, y: tl.y, expand };
  if (mode === 'new_image') {
    try { await docCall()('paste', args, img.rgba); } catch { return; }
    app.view.fit();
    return;
  }
  await run('paste', args, img.rgba);
  selectTool('move');
}

async function runFx(def) {
  if (!def) return;
  try {
    if (def.direct) { await call('fx', { phase: 'begin', fx: def.id }); return; }
    if (def.id === 'curves') {
      await call('fx', { phase: 'begin', fx: def.id });
      const r = await D.curvesDialog(c => run('fx', { phase: 'preview', curves: c }));
      await run('fx', r ? { phase: 'commit', curves: r } : { phase: 'cancel' });
      return;
    }
    await call('fx', { phase: 'begin', fx: def.id });
    const r = await D.fxDialog(def, v => run('fx', { phase: 'preview', ...v }));
    await run('fx', r ? { phase: 'commit', ...r } : { phase: 'cancel' });
  } catch { /* status shows the error */ }
}

async function featherSelection() {
  await run('feather', { phase: 'begin', radius: 5 });
  const def = { title: 'Feather Selection', sliders: [{ label: 'Radius', min: 1, max: 100, default: 5, scale: 1 }], checks: [], choices: [] };
  const r = await D.fxDialog(def, v => run('feather', { phase: 'preview', radius: v.values[0] }));
  if (r) { await run('feather', { phase: 'preview', radius: r.values[0] }); await run('feather', { phase: 'commit' }); }
  else await run('feather', { phase: 'cancel' });
}

async function resizeImage() {
  const r = await D.resizeDialog(app.state.width, app.state.height);
  if (r) { await run('resize', r); app.view.fit(); }
}
async function canvasSize() {
  const r = await D.canvasSizeDialog(app.state.width, app.state.height);
  if (r) { await run('canvas', r); app.view.fit(); }
}
async function layerProperties() {
  const l = app.state.layers[app.state.active];
  await run('layer_props', { phase: 'begin' });
  const r = await D.layerPropertiesDialog(l, app.blendModes, s => run('layer_props', { phase: 'preview', ...s }));
  await run('layer_props', r ? { phase: 'commit', ...r } : { phase: 'cancel' });
}

function showShortcuts() {
  const rows = [];
  for (const [k, it] of allShortcuts()) if (it.label) rows.push([k, typeof it.label === 'function' ? it.label().replace(/^(Undo|Redo).*/, '$1') : it.label]);
  rows.push(['S M Z H F G B E P K L R T O', 'Select tool (press again to cycle its group)'], ['X', 'Swap primary and secondary colors'], ['D', 'Reset colors to black and white'],
    ['[ ]', 'Brush width −/+'], ['Space + drag', 'Pan'], ['Ctrl + wheel', 'Zoom'], ['Arrow keys', 'Nudge (Move tools)']);
  return D.modal('Keyboard Shortcuts', b => b.append(el('div', { class: 'shortcuts' }, rows.flatMap(([k, l]) => [el('kbd', {}, k), el('span', {}, l)]))), { buttons: [['Close', 'ok', true]], width: 560 });
}
function showAbout() {
  return D.modal('About PaintDotRust', b => b.append(
    el('p', { class: 'message' }, 'PaintDotRust — an open-source image editor descended from Paint.NET 3.36 (via the OpenPDN port), reimplemented in Rust for the web.'),
    el('p', { class: 'hint' }, `Engine: ${app.mode}. Renderer: ${app.view.backend}.`),
    el('p', { class: 'hint' }, 'MIT License. Icons: Lucide (ISC). Paint.NET is a trademark of dotPDN LLC; this project is not affiliated with it.')), { buttons: [['Close', 'ok', true]] });
}

// ------------------------------------------------------------------ drag & drop, theme, boot

function wireDragDrop() {
  const ws = $('workspace');
  let hint = null;
  ws.addEventListener('dragover', e => {
    if (![...e.dataTransfer.types].includes('Files')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    if (!hint) { hint = el('div', { class: 'drop-hint' }); ws.append(hint); }
  });
  ws.addEventListener('dragleave', e => { if (!ws.contains(e.relatedTarget)) { hint?.remove(); hint = null; } });
  ws.addEventListener('drop', async e => {
    e.preventDefault();
    hint?.remove(); hint = null;
    const files = [...e.dataTransfer.files].filter(io.isSupported);
    if (!files.length) { status('Dropped file type isn\'t supported.'); return; }
    const noun = files.length === 1 ? `"${files[0].name}"` : `${files.length} files`;
    const choice = await D.message('Open or Add Layers', `Open ${noun}, or add ${files.length === 1 ? 'it' : 'them'} to this image as new layers?`, [['Open', 'open'], ['Add Layers', 'add', true], ['Cancel', null]]);
    if (!choice) return;
    let rest = files;
    if (choice === 'open') {
      for (const f of files) await openFile(f);
      rest = [];
    }
    for (const f of rest) await importFile(f);
  });
}

function wireTheme() {
  const btn = $('theme-button');
  const order = ['system', 'light', 'dark'];
  const apply = t => {
    if (t === 'system') delete document.documentElement.dataset.theme; else document.documentElement.dataset.theme = t;
    btn.replaceChildren(icon(`theme.${t}`, 16));
    btn.title = `Theme: ${t}`;
    app.view?.changed();
  };
  let cur = localStorage.getItem('openpdn.theme') || 'system';
  apply(cur);
  btn.addEventListener('click', () => { cur = order[(order.indexOf(cur) + 1) % 3]; localStorage.setItem('openpdn.theme', cur); apply(cur); });
}

function wireZoomBox() {
  const box = $('zoom-box');
  const show = () => { if (document.activeElement !== box) box.value = `${Math.round(app.view.zoom * 1000) / 10}%`; };
  app.view.onChange(() => { show(); drawRulers(); });
  box.addEventListener('change', () => { const v = parseFloat(box.value); if (v > 0) app.view.setZoom(v / 100); box.blur(); show(); });
  $('zoom-in').append(icon('view.zoom-in', 12));
  $('zoom-out').append(icon('view.zoom-out', 12));
  $('zoom-in').addEventListener('click', () => app.view.zoomStep(1));
  $('zoom-out').addEventListener('click', () => app.view.zoomStep(-1));
  show();
}

async function boot() {
  try {
    const { engine, mode } = await connect();
    app.engine = engine;
    app.mode = mode;
  } catch (e) {
    $('boot').classList.add('error');
    $('boot-text').textContent = `Couldn't start the image engine: ${e.message}`;
    return;
  }
  $('app').hidden = false;
  app.view = new CanvasView($('view'));
  app.a11y = A11y.load();
  setA11y({}, false);
  app.engine.onReply(onReply);
  wireTheme();
  buildCommonToolbar();
  buildToolOptions();
  buildToolsWindow();
  buildHistoryWindow();
  buildLayersWindow();
  buildColorsWindow();
  buildWindowToggles();
  $('a11y-button').append(icon('app.accessibility', 16));
  $('a11y-button').addEventListener('click', accessibilityDialog);
  wireCanvas();
  wireKeyboard();
  wireDragDrop();
  wireZoomBox();
  new ResizeObserver(() => { for (const w of Object.values(winEls)) if (!w.el.hidden) placeWindow(w); }).observe($('workspace'));
  const init = await call('init');
  await call('docs');
  app.fx = init.json.fx;
  app.blendModes = init.json.blendModes;
  buildMenus();
  await selectTool('rect-select');
  app.view.fit();
  $('boot').remove();
  $('tool-options').addEventListener('scroll', markOptionsOverflow);
  addEventListener('resize', markOptionsOverflow);
  // The tool options row scrolls sideways (wheel over its empty parts) when it doesn't fit.
  $('tool-options').addEventListener('wheel', e => {
    const o = e.currentTarget;
    if (o.scrollWidth <= o.clientWidth || e.target.closest('input, select')) return;
    e.preventDefault();
    o.scrollLeft += e.deltaY || e.deltaX;
  }, { passive: false });

  // Mouse wheel over any number box or slider changes it, and everything that
  // listens to it (the matching slider, the brush preview, the engine) follows.
  document.addEventListener('wheel', e => {
    const t = e.target.closest?.('input[type=number], input[type=range]');
    if (!t || t.disabled) return;
    e.preventDefault();
    const dir = (e.deltaY || e.deltaX) < 0 ? 1 : -1;
    const step = (t.step && t.step !== 'any' ? +t.step : 1) * (e.shiftKey ? 10 : 1);
    let v = (+t.value || 0) + dir * step;
    if (t.min !== '') v = Math.max(+t.min, v);
    if (t.max !== '') v = Math.min(+t.max, v);
    const dp = (String(step).split('.')[1] || '').length;
    t.value = v.toFixed(dp);
    t.dispatchEvent(new Event('input', { bubbles: true }));
    // 'change' (e.g. "remember this colour") only once the wheel has stopped.
    clearTimeout(t._wheelChange);
    t._wheelChange = setTimeout(() => t.dispatchEvent(new Event('change', { bubbles: true })), 600);
  }, { passive: false });

  // Warn before leaving with any unsaved document (browsers show their own wording).
  addEventListener('beforeunload', e => { if (app.docs.some(d => d.dirty) || app.state?.dirty) { e.preventDefault(); e.returnValue = ''; } });
  window.openpdn = app; // for debugging and tests
  app.runFx = runFx;
  status(`Ready — engine: ${app.mode}, renderer: ${app.view.backend}`);
}

boot();
