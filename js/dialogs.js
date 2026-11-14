// Modal dialogs. Each returns a Promise resolved with the result or null.
import { icon } from './icons.js';

function el(tag, props = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') e.className = v;
    else if (k === 'style') e.style.cssText = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? '' : v);
  }
  for (const k of kids.flat()) if (k != null) e.append(k.nodeType ? k : document.createTextNode(k));
  return e;
}
export { el };

/**
 * Shows a modal. `build(body, api)` fills the body; api.ok()/api.cancel() close it.
 * `onOk` returns the value to resolve with (or undefined to keep the dialog open).
 */
export function modal(title, build, { okText = 'OK', cancelText = 'Cancel', onOk, onCancel, width, buttons } = {}) {
  return new Promise(resolve => {
    const prevFocus = document.activeElement;
    const backdrop = el('div', { class: 'modal-backdrop' });
    const box = el('div', { class: 'dialog', role: 'dialog', 'aria-modal': 'true', 'aria-label': title, style: width ? `width:${width}px` : '' });
    const head = el('div', { class: 'dialog-title' }, el('span', {}, title));
    const close = el('button', { class: 'icon-button', title: 'Close', 'aria-label': 'Close' }, icon('window.close', 14));
    head.append(close);
    const body = el('div', { class: 'dialog-body' });
    const foot = el('div', { class: 'dialog-buttons' });
    box.append(head, body, foot);
    backdrop.append(box);
    document.body.append(backdrop);
    let done = false;
    const finish = v => {
      if (done) return;
      done = true;
      backdrop.remove();
      document.removeEventListener('keydown', keys, true);
      prevFocus?.focus?.();
      resolve(v);
    };
    const api = {
      body,
      ok: async () => {
        const v = onOk ? await onOk() : true;
        if (v !== undefined) finish(v);
      },
      cancel: async () => { await onCancel?.(); finish(null); },
      close: finish,
    };
    const defs = buttons ?? [[okText, 'ok', true], [cancelText, 'cancel']];
    for (const [text, action, primary] of defs) {
      const b = el('button', { class: primary ? 'primary' : '' }, text);
      b.addEventListener('click', () => (typeof action === 'function' ? action(api) : api[action]()));
      foot.append(b);
    }
    close.addEventListener('click', api.cancel);
    const keys = e => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); api.cancel(); }
      else if (e.key === 'Enter' && !(e.target instanceof HTMLTextAreaElement) && !(e.target instanceof HTMLButtonElement)) {
        e.preventDefault(); e.stopPropagation(); api.ok();
      }
      e.stopPropagation();
    };
    document.addEventListener('keydown', keys, true);
    // Drag by the title bar.
    head.addEventListener('pointerdown', e => {
      if (e.target.closest('button')) return;
      const r = box.getBoundingClientRect();
      const ox = e.clientX - r.left, oy = e.clientY - r.top;
      box.style.position = 'fixed';
      const move = ev => { box.style.left = `${Math.max(0, ev.clientX - ox)}px`; box.style.top = `${Math.max(0, ev.clientY - oy)}px`; box.style.margin = '0'; };
      const up = () => { removeEventListener('pointermove', move); removeEventListener('pointerup', up); };
      addEventListener('pointermove', move);
      addEventListener('pointerup', up);
    });
    build(body, api);
    // Focus the first input, else the primary button.
    queueMicrotask(() => (body.querySelector('input:not([type=checkbox]),select,textarea') || foot.querySelector('.primary'))?.focus());
  });
}

/** Slider + number box row; `scale` shows value/scale. Returns { row, get, set }. */
export function sliderRow(label, min, max, value, scale = 1, onInput) {
  const fmt = v => (scale === 1 ? String(v) : (v / scale).toFixed(2));
  const range = el('input', { type: 'range', min, max, value, step: 1, 'aria-label': label });
  const num = el('input', { type: 'number', min: min / scale, max: max / scale, step: 1 / scale, value: fmt(value), class: 'num', 'aria-label': label });
  const reset = el('button', { class: 'icon-button small', title: 'Reset', 'aria-label': `Reset ${label}` }, icon('color.reset', 12));
  const initial = value;
  const set = (v, fire = true) => {
    v = Math.min(max, Math.max(min, Math.round(v)));
    range.value = v;
    num.value = fmt(v);
    if (fire) onInput?.(v);
  };
  range.addEventListener('input', () => set(+range.value));
  num.addEventListener('change', () => set(Math.round(parseFloat(num.value || '0') * scale)));
  reset.addEventListener('click', () => set(initial));
  const row = el('div', { class: 'slider-row' }, el('label', {}, label), el('div', { class: 'slider-line' }, range, num, reset));
  return { row, get: () => +range.value, set };
}

/** Builds a dialog for a catalogue entry (sliders/checks/choices); calls preview(values) live. */
export function fxDialog(def, preview) {
  let values = def.sliders.map(s => s.default);
  const checks = def.checks.map(c => c.default);
  const choices = def.choices.map(c => c.default);
  let linked = def.linked;
  const current = () => ({ values: [...values], checks: [...checks], choices: [...choices] });
  return modal(def.title, body => {
    const rows = [];
    def.choices.forEach((c, i) => {
      const sel = el('select', { 'aria-label': c.label }, c.options.map((o, k) => el('option', { value: k, selected: k === c.default }, o)));
      sel.addEventListener('change', () => { choices[i] = +sel.value; preview(current()); });
      body.append(el('div', { class: 'field' }, el('label', {}, c.label), sel));
    });
    def.sliders.forEach((s, i) => {
      const r = sliderRow(s.label, s.min, s.max, s.default, s.scale, v => {
        values[i] = v;
        if (linked) values.forEach((_, k) => { if (k !== i) { values[k] = v; rows[k].set(v, false); } });
        preview(current());
      });
      rows.push(r);
      body.append(r.row);
    });
    if (def.linked) {
      const cb = el('input', { type: 'checkbox', checked: true });
      cb.addEventListener('change', () => { linked = cb.checked; });
      body.append(el('label', { class: 'check' }, cb, 'Linked'));
    }
    def.checks.forEach((c, i) => {
      const cb = el('input', { type: 'checkbox', checked: c.default });
      cb.addEventListener('change', () => { checks[i] = cb.checked; preview(current()); });
      body.append(el('label', { class: 'check' }, cb, c.label));
    });
  }, { onOk: () => current(), width: 380 });
}

export function message(title, text, buttons = [['OK', 'ok', true]]) {
  return modal(title, b => b.append(el('p', { class: 'message' }, text)), {
    buttons: buttons.map(([t, v, p]) => [t, api => api.close(v), p]),
  });
}

export function newImageDialog(defW = 800, defH = 600) {
  let w = defW, h = defH, bg = 'white';
  return modal('New Image', body => {
    const wi = el('input', { type: 'number', min: 1, max: 65535, value: w, class: 'num' });
    const hi = el('input', { type: 'number', min: 1, max: 65535, value: h, class: 'num' });
    wi.addEventListener('input', () => (w = +wi.value));
    hi.addEventListener('input', () => (h = +hi.value));
    const sel = el('select', {}, ['white', 'secondary', 'transparent'].map(v => el('option', { value: v }, { white: 'White', secondary: 'Secondary color', transparent: 'Transparent' }[v])));
    sel.addEventListener('change', () => (bg = sel.value));
    body.append(
      el('div', { class: 'field' }, el('label', {}, 'Width'), wi, el('span', { class: 'unit' }, 'pixels')),
      el('div', { class: 'field' }, el('label', {}, 'Height'), hi, el('span', { class: 'unit' }, 'pixels')),
      el('div', { class: 'field' }, el('label', {}, 'Background'), sel));
  }, { onOk: () => (w >= 1 && h >= 1 && w * h <= 268435456 ? { w: Math.round(w), h: Math.round(h), bg } : undefined) });
}

export function resizeDialog(curW, curH) {
  let w = curW, h = curH, keep = true, mode = 'Best', pct = 100, byPct = false;
  return modal('Resize Image', body => {
    const pctIn = el('input', { type: 'number', min: 1, max: 1000, value: 100, class: 'num' });
    const wi = el('input', { type: 'number', min: 1, max: 65535, value: w, class: 'num' });
    const hi = el('input', { type: 'number', min: 1, max: 65535, value: h, class: 'num' });
    const keepCb = el('input', { type: 'checkbox', checked: true });
    const byPctRadio = el('input', { type: 'radio', name: 'rz' });
    const byAbsRadio = el('input', { type: 'radio', name: 'rz', checked: true });
    const modeSel = el('select', {}, [['Best', 'Best quality'], ['Bilinear', 'Bilinear'], ['NearestNeighbor', 'Nearest neighbor'], ['SuperSampling', 'Supersampling']].map(([v, t]) => el('option', { value: v }, t)));
    const sync = () => {
      pctIn.disabled = !byPct; wi.disabled = byPct; hi.disabled = byPct;
      if (byPct) { w = Math.max(1, Math.round(curW * pct / 100)); h = Math.max(1, Math.round(curH * pct / 100)); wi.value = w; hi.value = h; }
    };
    pctIn.addEventListener('input', () => { pct = +pctIn.value; sync(); });
    wi.addEventListener('input', () => { w = +wi.value; if (keep) { h = Math.max(1, Math.round(w * curH / curW)); hi.value = h; } });
    hi.addEventListener('input', () => { h = +hi.value; if (keep) { w = Math.max(1, Math.round(h * curW / curH)); wi.value = w; } });
    keepCb.addEventListener('change', () => (keep = keepCb.checked));
    byPctRadio.addEventListener('change', () => { byPct = true; sync(); });
    byAbsRadio.addEventListener('change', () => { byPct = false; sync(); });
    modeSel.addEventListener('change', () => (mode = modeSel.value));
    body.append(
      el('div', { class: 'field' }, el('label', {}, 'Resampling'), modeSel),
      el('label', { class: 'check' }, byPctRadio, 'By percentage'), el('div', { class: 'field indent' }, el('label', {}, 'Percentage'), pctIn, el('span', { class: 'unit' }, '%')),
      el('label', { class: 'check' }, byAbsRadio, 'By absolute size'),
      el('label', { class: 'check indent' }, keepCb, 'Maintain aspect ratio'),
      el('div', { class: 'field indent' }, el('label', {}, 'Width'), wi, el('span', { class: 'unit' }, 'pixels')),
      el('div', { class: 'field indent' }, el('label', {}, 'Height'), hi, el('span', { class: 'unit' }, 'pixels')));
    sync();
  }, { onOk: () => (w >= 1 && h >= 1 ? { w, h, mode } : undefined) });
}

export function canvasSizeDialog(curW, curH) {
  let w = curW, h = curH, keep = false, anchor = 4;
  return modal('Canvas Size', body => {
    const wi = el('input', { type: 'number', min: 1, max: 65535, value: w, class: 'num' });
    const hi = el('input', { type: 'number', min: 1, max: 65535, value: h, class: 'num' });
    const keepCb = el('input', { type: 'checkbox' });
    wi.addEventListener('input', () => { w = +wi.value; if (keep) { h = Math.max(1, Math.round(w * curH / curW)); hi.value = h; } });
    hi.addEventListener('input', () => { h = +hi.value; if (keep) { w = Math.max(1, Math.round(h * curW / curH)); wi.value = w; } });
    keepCb.addEventListener('change', () => (keep = keepCb.checked));
    const grid = el('div', { class: 'anchor-grid', role: 'radiogroup', 'aria-label': 'Anchor' });
    const arrows = ['↖', '↑', '↗', '←', '•', '→', '↙', '↓', '↘'];
    const btns = arrows.map((a, i) => {
      const b = el('button', { class: i === 4 ? 'on' : '', role: 'radio', 'aria-checked': String(i === 4), title: ['Top left', 'Top', 'Top right', 'Left', 'Middle', 'Right', 'Bottom left', 'Bottom', 'Bottom right'][i] }, a);
      b.addEventListener('click', () => { anchor = i; btns.forEach((x, k) => { x.classList.toggle('on', k === i); x.setAttribute('aria-checked', String(k === i)); }); });
      return b;
    });
    grid.append(...btns);
    body.append(
      el('label', { class: 'check' }, keepCb, 'Maintain aspect ratio'),
      el('div', { class: 'field' }, el('label', {}, 'Width'), wi, el('span', { class: 'unit' }, 'pixels')),
      el('div', { class: 'field' }, el('label', {}, 'Height'), hi, el('span', { class: 'unit' }, 'pixels')),
      el('div', { class: 'field' }, el('label', {}, 'Anchor'), grid));
  }, { onOk: () => (w >= 1 && h >= 1 ? { w, h, anchor } : undefined) });
}

export function layerPropertiesDialog(layer, blendModes, preview) {
  const st = { name: layer.name, visible: layer.visible, opacity: layer.opacity, blend: layer.blend };
  return modal('Layer Properties', body => {
    const name = el('input', { type: 'text', value: st.name });
    name.addEventListener('input', () => { st.name = name.value; preview({ ...st }); });
    const vis = el('input', { type: 'checkbox', checked: st.visible });
    vis.addEventListener('change', () => { st.visible = vis.checked; preview({ ...st }); });
    const blend = el('select', {}, blendModes.map(m => el('option', { value: m, selected: m === st.blend }, m.replace(/([a-z])([A-Z])/g, '$1 $2'))));
    blend.addEventListener('change', () => { st.blend = blend.value; preview({ ...st }); });
    const op = sliderRow('Opacity', 0, 255, st.opacity, 1, v => { st.opacity = v; preview({ ...st }); });
    body.append(el('div', { class: 'field' }, el('label', {}, 'Name'), name), el('label', { class: 'check' }, vis, 'Visible'),
      el('div', { class: 'field' }, el('label', {}, 'Blend mode'), blend), op.row);
  }, { onOk: () => ({ ...st }), width: 360 });
}

/** Curves: luminosity (one curve) or RGB (three). preview({mode, points}) live. */
export function curvesDialog(preview) {
  const curves = [[[0, 0], [255, 255]], [[0, 0], [255, 255]], [[0, 0], [255, 255]]];
  let mode = 0;
  const enabled = [true, true, true];
  const state = () => ({ mode, points: (mode === 0 ? [curves[0]] : curves).map(c => c.flat()) });
  return modal('Curves', body => {
    const modeSel = el('select', {}, el('option', { value: 0 }, 'Luminosity'), el('option', { value: 1 }, 'RGB'));
    const chans = ['Red', 'Green', 'Blue'].map((n, i) => {
      const cb = el('input', { type: 'checkbox', checked: true });
      cb.addEventListener('change', () => { enabled[i] = cb.checked; });
      return el('label', { class: 'check inline' }, cb, n);
    });
    const chanRow = el('div', { class: 'field', style: 'display:none' }, ...chans);
    const cv = el('canvas', { width: 256, height: 256, class: 'curve-canvas', 'aria-label': 'Curve editor' });
    const hint = el('p', { class: 'hint' }, 'Drag points; click to add; right-click a point to remove it.');
    body.append(el('div', { class: 'field' }, el('label', {}, 'Transfer map'), modeSel), chanRow, cv, hint);
    const ctx = cv.getContext('2d');
    const colors = ['#d33', '#3a3', '#36d'];
    const edit = () => (mode === 0 ? [0] : [0, 1, 2].filter(i => enabled[i]));
    const evalSpline = pts => {
      // Natural cubic spline, same as the engine's (for drawing only).
      const xs = pts.map(p => p[0]), ys = pts.map(p => p[1]), n = xs.length, y2 = new Array(n).fill(0), u = new Array(n).fill(0);
      for (let i = 1; i < n - 1; i++) {
        const sig = (xs[i] - xs[i - 1]) / (xs[i + 1] - xs[i - 1]), p = sig * y2[i - 1] + 2;
        y2[i] = (sig - 1) / p;
        const d = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]) - (ys[i] - ys[i - 1]) / (xs[i] - xs[i - 1]);
        u[i] = (6 * d / (xs[i + 1] - xs[i - 1]) - sig * u[i - 1]) / p;
      }
      for (let i = n - 2; i >= 0; i--) y2[i] = y2[i] * y2[i + 1] + u[i];
      return x => {
        let lo = 0, hi = n - 1;
        while (hi - lo > 1) { const k = (hi + lo) >> 1; if (xs[k] > x) hi = k; else lo = k; }
        const h = xs[hi] - xs[lo], a = (xs[hi] - x) / h, b = (x - xs[lo]) / h;
        return a * ys[lo] + b * ys[hi] + ((a ** 3 - a) * y2[lo] + (b ** 3 - b) * y2[hi]) * h * h / 6;
      };
    };
    const draw = () => {
      ctx.clearRect(0, 0, 256, 256);
      ctx.strokeStyle = 'rgba(128,128,128,0.35)';
      for (let i = 64; i < 256; i += 64) { ctx.beginPath(); ctx.moveTo(i + 0.5, 0); ctx.lineTo(i + 0.5, 256); ctx.moveTo(0, i + 0.5); ctx.lineTo(256, i + 0.5); ctx.stroke(); }
      (mode === 0 ? [0] : [0, 1, 2]).forEach(ci => {
        const f = evalSpline(curves[ci]);
        ctx.strokeStyle = mode === 0 ? getComputedStyle(cv).color : colors[ci];
        ctx.beginPath();
        for (let x = 0; x < 256; x++) { const y = 255 - Math.max(0, Math.min(255, f(x))); x ? ctx.lineTo(x + 0.5, y + 0.5) : ctx.moveTo(0.5, y + 0.5); }
        ctx.stroke();
        for (const [x, y] of curves[ci]) { ctx.fillStyle = ctx.strokeStyle; ctx.fillRect(x - 3, 255 - y - 3, 7, 7); }
      });
    };
    modeSel.addEventListener('change', () => { mode = +modeSel.value; chanRow.style.display = mode ? '' : 'none'; draw(); preview(state()); });
    let drag = null;
    const pos = e => { const r = cv.getBoundingClientRect(); return [Math.round((e.clientX - r.left) * 255 / r.width), Math.round(255 - (e.clientY - r.top) * 255 / r.height)]; };
    cv.addEventListener('contextmenu', e => e.preventDefault());
    cv.addEventListener('pointerdown', e => {
      const [x, y] = pos(e);
      for (const ci of edit()) {
        const pts = curves[ci];
        const i = pts.findIndex(p => Math.abs(p[0] - x) < 6 && Math.abs(p[1] - y) < 6);
        if (e.button === 2) { if (i > 0 && i < pts.length - 1) pts.splice(i, 1); continue; }
        if (i >= 0) { drag = drag || []; drag.push([ci, i]); }
        else if (!pts.some(p => p[0] === x)) { pts.push([x, y]); pts.sort((a, b) => a[0] - b[0]); drag = drag || []; drag.push([ci, pts.findIndex(p => p[0] === x)]); }
      }
      cv.setPointerCapture(e.pointerId);
      draw(); preview(state());
    });
    cv.addEventListener('pointermove', e => {
      if (!drag) return;
      const [x, y] = pos(e);
      for (const [ci, i] of drag) {
        const pts = curves[ci];
        const lo = i > 0 ? pts[i - 1][0] + 1 : 0, hi = i < pts.length - 1 ? pts[i + 1][0] - 1 : 255;
        pts[i] = [i === 0 || i === pts.length - 1 ? pts[i][0] : Math.max(lo, Math.min(hi, x)), Math.max(0, Math.min(255, y))];
      }
      draw(); preview(state());
    });
    cv.addEventListener('pointerup', () => (drag = null));
    draw();
  }, { onOk: () => state(), width: 310 });
}

// ------------------------------------------------------------------ colour picker

export function hexToRgba(hex) {
  const h = hex.replace('#', '');
  return { r: parseInt(h.slice(0, 2), 16), g: parseInt(h.slice(2, 4), 16), b: parseInt(h.slice(4, 6), 16), a: h.length >= 8 ? parseInt(h.slice(6, 8), 16) : 255 };
}
export function rgbaToHex({ r, g, b, a }) {
  const x = v => Math.round(v).toString(16).padStart(2, '0').toUpperCase();
  return `#${x(r)}${x(g)}${x(b)}${a === 255 ? '' : x(a)}`;
}
function rgbToHsv(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let h = 0;
  if (d) h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return { h: (h * 60 + 360) % 360, s: max ? d / max : 0, v: max };
}
function hsvToRgb(h, s, v) {
  const c = v * s, x = c * (1 - Math.abs((h / 60) % 2 - 1)), m = v - c;
  const [r, g, b] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return { r: (r + m) * 255, g: (g + m) * 255, b: (b + m) * 255 };
}

export function colorPickerDialog(title, hex) {
  let c = hexToRgba(hex);
  let hsv = rgbToHsv(c.r, c.g, c.b);
  return modal(title, body => {
    const sv = el('canvas', { width: 200, height: 200, class: 'sv-canvas', 'aria-label': 'Saturation and value' });
    const hue = el('input', { type: 'range', min: 0, max: 359, value: Math.round(hsv.h), class: 'hue-slider', 'aria-label': 'Hue' });
    const swatch = el('div', { class: 'swatch-big' });
    const hexIn = el('input', { type: 'text', value: rgbaToHex(c).slice(1), class: 'hex', maxlength: 8, spellcheck: 'false', 'aria-label': 'Hex' });
    const rows = {};
    const sync = (from) => {
      if (from !== 'hsv') hsv = rgbToHsv(c.r, c.g, c.b);
      if (from !== 'hex') hexIn.value = rgbaToHex(c).slice(1);
      for (const k of ['r', 'g', 'b', 'a']) if (from !== k) rows[k].set(Math.round(c[k]), false);
      if (from !== 'hue') hue.value = Math.round(hsv.h);
      swatch.style.background = `rgba(${c.r},${c.g},${c.b},${c.a / 255})`;
      drawSv();
    };
    for (const [k, label] of [['r', 'Red'], ['g', 'Green'], ['b', 'Blue'], ['a', 'Alpha']]) {
      rows[k] = sliderRow(label, 0, 255, Math.round(c[k]), 1, v => { c[k] = v; sync(k); });
    }
    const ctx = sv.getContext('2d');
    const drawSv = () => {
      const g1 = ctx.createLinearGradient(0, 0, 200, 0);
      const pure = hsvToRgb(hsv.h, 1, 1);
      g1.addColorStop(0, '#fff'); g1.addColorStop(1, `rgb(${pure.r},${pure.g},${pure.b})`);
      ctx.fillStyle = g1; ctx.fillRect(0, 0, 200, 200);
      const g2 = ctx.createLinearGradient(0, 0, 0, 200);
      g2.addColorStop(0, 'rgba(0,0,0,0)'); g2.addColorStop(1, '#000');
      ctx.fillStyle = g2; ctx.fillRect(0, 0, 200, 200);
      ctx.strokeStyle = hsv.v > 0.5 ? '#000' : '#fff';
      ctx.beginPath(); ctx.arc(hsv.s * 200, (1 - hsv.v) * 200, 5, 0, Math.PI * 2); ctx.stroke();
    };
    const pick = e => {
      const r = sv.getBoundingClientRect();
      hsv.s = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
      hsv.v = Math.min(1, Math.max(0, 1 - (e.clientY - r.top) / r.height));
      Object.assign(c, hsvToRgb(hsv.h, hsv.s, hsv.v));
      sync('hsv');
    };
    sv.addEventListener('pointerdown', e => { sv.setPointerCapture(e.pointerId); pick(e); sv.onpointermove = pick; });
    sv.addEventListener('pointerup', () => (sv.onpointermove = null));
    hue.addEventListener('input', () => { hsv.h = +hue.value; Object.assign(c, hsvToRgb(hsv.h, hsv.s, hsv.v)); sync('hue'); });
    hexIn.addEventListener('input', () => {
      const v = hexIn.value.trim().replace(/^#/, '');
      if (/^([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(v)) {
        const full = v.length <= 4 ? v.split('').map(ch => ch + ch).join('') : v;
        c = hexToRgba(full);
        hexIn.classList.remove('invalid');
        sync('hex');
      } else hexIn.classList.add('invalid');
    });
    body.append(el('div', { class: 'picker' }, el('div', {}, sv, hue), el('div', { class: 'picker-side' }, swatch,
      el('div', { class: 'field' }, el('label', {}, 'Hex'), hexIn), rows.r.row, rows.g.row, rows.b.row, rows.a.row)));
    sync();
  }, { onOk: () => rgbaToHex({ r: Math.round(c.r), g: Math.round(c.g), b: Math.round(c.b), a: Math.round(c.a) }), width: 520 });
}

