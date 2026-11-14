// Text layout and rendering for the on-canvas text tool, using the
// browser's font engine. Positions are in image pixels; the anchor (x, y)
// is where the user clicked: the top of the first line, at its left edge,
// centre or right edge depending on the alignment.

export const FONTS = ['sans-serif', 'serif', 'monospace', 'system-ui', 'Arial', 'Helvetica', 'Verdana', 'Tahoma', 'Trebuchet MS',
  'Georgia', 'Times New Roman', 'Garamond', 'Courier New', 'Impact', 'Comic Sans MS'];

export const DEFAULT_SPEC = { font: 'sans-serif', size: 24, bold: false, italic: false, underline: false, align: 'left' };

const cssFont = s => `${s.italic ? 'italic ' : ''}${s.bold ? 'bold ' : ''}${s.size}px ${/\s/.test(s.font) ? `"${s.font}"` : s.font}`;

let measureCtx = null;
function measurer(font) {
  measureCtx ??= new OffscreenCanvas(1, 1).getContext('2d');
  measureCtx.font = font;
  return measureCtx;
}

/** Lines, their widths and offsets, and the block's box (image coords). */
export function layout(spec, text, x, y) {
  const font = cssFont(spec);
  const m = measurer(font);
  const lines = text.split('\n');
  const widths = lines.map(l => m.measureText(l).width);
  const maxW = Math.max(0, ...widths);
  const lineH = Math.ceil(spec.size * 1.25);
  const k = spec.align === 'center' ? 0.5 : spec.align === 'right' ? 1 : 0;
  const left = x - maxW * k;
  const offsets = widths.map(w => (maxW - w) * k);
  return { font, lines, widths, offsets, maxW, lineH, left, top: y, box: { x: left, y, w: Math.max(maxW, 1), h: lineH * lines.length } };
}

/** Caret rectangle for a character index (image coords). */
export function caretAt(L, index) {
  let i = index, line = 0;
  while (line < L.lines.length - 1 && i > L.lines[line].length) { i -= L.lines[line].length + 1; line++; }
  const m = measurer(L.font);
  const cx = L.left + L.offsets[line] + m.measureText(L.lines[line].slice(0, i)).width;
  return { x: cx, y: L.top + line * L.lineH, h: L.lineH };
}

/** Character index nearest an image point (for clicks inside the text). */
export function indexAt(L, px, py) {
  const line = Math.max(0, Math.min(L.lines.length - 1, Math.floor((py - L.top) / L.lineH)));
  const m = measurer(L.font);
  const s = L.lines[line];
  let best = 0, bestD = Infinity;
  for (let i = 0; i <= s.length; i++) {
    const d = Math.abs(L.left + L.offsets[line] + m.measureText(s.slice(0, i)).width - px);
    if (d < bestD) { bestD = d; best = i; }
  }
  let index = best;
  for (let l = 0; l < line; l++) index += L.lines[l].length + 1;
  return index;
}

/** Rotates image point (px, py) by `angle` about (ax, ay). */
export function rotateAbout(px, py, ax, ay, angle) {
  const c = Math.cos(angle), s = Math.sin(angle), dx = px - ax, dy = py - ay;
  return { x: ax + dx * c - dy * s, y: ay + dx * s + dy * c };
}

/**
 * Renders the text as straight-alpha RGBA in a single colour, rotated by
 * `angle` radians about the anchor. Returns the image and where its top-left
 * goes ({ x, y } image pixels), or null if empty.
 */
export function render(spec, L, { r, g, b, a }, antialias = true, angle = 0, ax = L.left, ay = L.top) {
  if (!L.lines.some(l => l.length)) return null;
  // Padding for italic overhangs and descenders.
  const pad = Math.ceil(spec.size * 0.3);
  const corners = [[L.left - pad, L.top - pad], [L.left + L.maxW + pad, L.top - pad],
    [L.left + L.maxW + pad, L.top + L.lineH * L.lines.length + pad], [L.left - pad, L.top + L.lineH * L.lines.length + pad]]
    .map(([x, y]) => rotateAbout(x, y, ax, ay, angle));
  const ox = Math.floor(Math.min(...corners.map(p => p.x))), oy = Math.floor(Math.min(...corners.map(p => p.y)));
  const w = Math.max(1, Math.ceil(Math.max(...corners.map(p => p.x))) - ox);
  const h = Math.max(1, Math.ceil(Math.max(...corners.map(p => p.y))) - oy);
  const cv = new OffscreenCanvas(w, h);
  const ctx = cv.getContext('2d');
  // Draw in image coordinates: shift to the canvas origin, rotate about the anchor.
  ctx.translate(ax - ox, ay - oy);
  ctx.rotate(angle);
  ctx.translate(-ax, -ay);
  ctx.font = L.font;
  ctx.textBaseline = 'alphabetic';
  ctx.fillStyle = '#000';
  const asc = ctx.measureText('Mg').fontBoundingBoxAscent ?? spec.size * 0.8;
  const top = (L.lineH - spec.size * 1.15) / 2; // centre the em box in the line
  L.lines.forEach((line, i) => {
    const bx = L.left + L.offsets[i], by = L.top + i * L.lineH + top + asc;
    ctx.fillText(line, bx, by);
    if (spec.underline && line.length) {
      const t = Math.max(1, Math.round(spec.size / 15));
      ctx.fillRect(bx, Math.round(by + spec.size * 0.09), L.widths[i], t);
    }
  });
  const img = ctx.getImageData(0, 0, w, h).data;
  const out = new Uint8Array(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    let cov = img[i * 4 + 3];
    if (!antialias) cov = cov >= 128 ? 255 : 0;
    out[i * 4] = r; out[i * 4 + 1] = g; out[i * 4 + 2] = b; out[i * 4 + 3] = Math.round(cov * a / 255);
  }
  return { x: ox, y: oy, w, h, rgba: out };
}
