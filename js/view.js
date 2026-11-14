// Canvas view: draws the composited document (kept in GPU textures, updated
// from dirty tiles) with zoom/pan, a transparency checkerboard and a pixel
// grid, plus a 2D overlay for selection outlines, handles and cursors.

import { FILTER_GLSL, svgFilterMarkup } from './a11y.js';

const TILE = 2048;

const VS = `#version 300 es
in vec2 aPos;          // image pixels
uniform vec2 uScale;   // image px -> clip
uniform vec2 uOffset;
out vec2 vImg;
void main() { vImg = aPos; gl_Position = vec4(aPos * uScale + uOffset, 0.0, 1.0); }`;

const FS = `#version 300 es
precision highp float;
in vec2 vImg;
uniform sampler2D uTex;
uniform vec2 uTileOrigin;  // image px of this tile's (0,0)
uniform vec2 uTileSize;    // texture size in px
uniform float uChecker;    // checker cell size in device px
uniform float uGrid;       // 1 = draw pixel grid
uniform float uZoom;       // device px per image px
uniform vec3 uCheckA, uCheckB;
${FILTER_GLSL}
out vec4 o;
void main() {
  vec4 c = texture(uTex, (vImg - uTileOrigin) / uTileSize);
  c.rgb = applyFilter(c.rgb);
  vec2 cell = floor(gl_FragCoord.xy / uChecker);
  vec3 bg = mod(cell.x + cell.y, 2.0) < 1.0 ? uCheckA : uCheckB;
  vec3 rgb = mix(bg, c.rgb, c.a);
  if (uGrid > 0.5) {
    vec2 f = fract(vImg) * uZoom;
    if (f.x < 1.0 || f.y < 1.0) rgb = mix(rgb, vec3(0.5), 0.55);
  }
  o = vec4(rgb, 1.0);
}`;

export class CanvasView {
  constructor(host) {
    this.host = host;
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'view-gl';
    this.overlay = document.createElement('canvas');
    this.overlay.className = 'view-overlay';
    host.append(this.canvas, this.overlay);
    this.zoom = 1;
    this.cx = 0;
    this.cy = 0;
    this.docW = 0;
    this.docH = 0;
    this.pixelGrid = false;
    this.selection = null;      // { outline: [[x,y,…]], polygon, bounds }
    this.handles = null;        // [x, y, w, h]
    this.cursor = null;         // { x, y, d } image coords
    this.lasso = null;          // [[x,y], …]
    this.zoomRect = null;       // [x0, y0, x1, y1] image coords
    this.antsPhase = 0;
    this.reduceMotion = false;   // static selection outline instead of marching ants
    this.filter = { id: 'none' };
    this.selectionTint = 'rgba(59,130,246,0.2)'; // fill for selected areas; null = none
    this.shapeEdit = null;  // pending line/curve: { curve, points: [[x, y], …] }
    this.shapeGrip = null;  // screen position of its move grip (for hit tests)
    this.textBox = null;    // text being edited: { box: {x,y,w,h}, caret: {x,y,h} } image coords
    this.caretOn = true;
    this.listeners = new Set();
    this.dirty = true;
    this.overlayDirty = true;
    this.gl = this.canvas.getContext('webgl2', { alpha: false, antialias: false, premultipliedAlpha: false });
    if (this.gl) this.initGL(); else this.ctx2d = this.canvas.getContext('2d');
    this.ov = this.overlay.getContext('2d');
    new ResizeObserver(() => this.resize()).observe(host);
    this.resize();
    const frame = () => {
      if (this.dirty) this.draw();
      if (this.selection && this.selection.outline?.length && !this.reduceMotion) {
        this.antsPhase = (performance.now() / 80) % 8;
        this.overlayDirty = true;
      }
      if (this.textBox) {
        const on = Math.floor(performance.now() / 530) % 2 === 0;
        if (on !== this.caretOn) { this.caretOn = on; this.overlayDirty = true; }
      }
      if (this.overlayDirty) this.drawOverlay();
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  }

  /** Display-only filter (see a11y.js FILTERS). */
  setFilter(f) {
    this.filter = f;
    if (!this.gl) {
      let svg = document.getElementById('openpdn-view-filter-svg');
      const markup = svgFilterMarkup(f);
      if (markup && !svg) {
        svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
        svg.id = 'openpdn-view-filter-svg';
        svg.setAttribute('width', '0'); svg.setAttribute('height', '0');
        svg.style.position = 'absolute';
        document.body.append(svg);
      }
      if (svg) svg.innerHTML = markup;
      this.canvas.style.filter = markup ? 'url(#openpdn-view-filter)' : '';
    }
    this.changed();
  }

  get backend() { return this.gl ? 'WebGL2' : 'Canvas 2D'; }
  get dpr() { return window.devicePixelRatio || 1; }
  get viewW() { return this.host.clientWidth; }
  get viewH() { return this.host.clientHeight; }

  onChange(fn) { this.listeners.add(fn); }
  changed() {
    this.dirty = true;
    this.overlayDirty = true;
    for (const fn of this.listeners) fn();
  }

  resize() {
    const d = this.dpr;
    for (const c of [this.canvas, this.overlay]) {
      c.width = Math.max(1, Math.round(this.viewW * d));
      c.height = Math.max(1, Math.round(this.viewH * d));
    }
    this.changed();
  }

  // ------------------------------------------------------------ document pixels

  initGL() {
    const gl = this.gl;
    const sh = (type, src) => {
      const s = gl.createShader(type);
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
      return s;
    };
    const p = gl.createProgram();
    gl.attachShader(p, sh(gl.VERTEX_SHADER, VS));
    gl.attachShader(p, sh(gl.FRAGMENT_SHADER, FS));
    gl.linkProgram(p);
    this.prog = p;
    this.loc = Object.fromEntries(['uScale', 'uOffset', 'uTex', 'uTileOrigin', 'uTileSize', 'uChecker', 'uGrid', 'uZoom', 'uCheckA', 'uCheckB', 'uFilter', 'uFilterMat']
      .map(n => [n, gl.getUniformLocation(p, n)]));
    this.vbo = gl.createBuffer();
    this.tiles = [];
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  }

  setDocSize(w, h) {
    if (w === this.docW && h === this.docH) return;
    const first = this.docW === 0;
    this.docW = w;
    this.docH = h;
    if (this.gl) {
      const gl = this.gl;
      for (const t of this.tiles) gl.deleteTexture(t.tex);
      this.tiles = [];
      for (let y = 0; y < h; y += TILE) {
        for (let x = 0; x < w; x += TILE) {
          const tw = Math.min(TILE, w - x), th = Math.min(TILE, h - y);
          const tex = gl.createTexture();
          gl.bindTexture(gl.TEXTURE_2D, tex);
          gl.texStorage2D(gl.TEXTURE_2D, 1 + Math.floor(Math.log2(Math.max(tw, th))), gl.RGBA8, tw, th);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
          gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
          this.tiles.push({ x, y, w: tw, h: th, tex, mipDirty: true });
        }
      }
    } else {
      this.img = new OffscreenCanvas(w, h);
      this.imgCtx = this.img.getContext('2d');
    }
    if (first) this.fit(); else this.clampPan();
    this.changed();
  }

  /** Replaces the pixels of an image rectangle (RGBA8, row-major). */
  updateTile(x, y, w, h, rgba) {
    if (this.gl) {
      const gl = this.gl;
      for (const t of this.tiles) {
        const x0 = Math.max(x, t.x), y0 = Math.max(y, t.y);
        const x1 = Math.min(x + w, t.x + t.w), y1 = Math.min(y + h, t.y + t.h);
        if (x1 <= x0 || y1 <= y0) continue;
        gl.bindTexture(gl.TEXTURE_2D, t.tex);
        gl.pixelStorei(gl.UNPACK_ROW_LENGTH, w);
        gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, x0 - x);
        gl.pixelStorei(gl.UNPACK_SKIP_ROWS, y0 - y);
        gl.texSubImage2D(gl.TEXTURE_2D, 0, x0 - t.x, y0 - t.y, x1 - x0, y1 - y0, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
        t.mipDirty = true;
      }
      gl.pixelStorei(gl.UNPACK_ROW_LENGTH, 0);
      gl.pixelStorei(gl.UNPACK_SKIP_PIXELS, 0);
      gl.pixelStorei(gl.UNPACK_SKIP_ROWS, 0);
    } else {
      const data = new ImageData(new Uint8ClampedArray(rgba.buffer, rgba.byteOffset, w * h * 4), w, h);
      this.imgCtx.putImageData(data, x, y);
    }
    this.dirty = true;
  }

  /** Reads back nothing: the engine owns pixels. Exposed for tests. */
  get textureCount() { return this.tiles?.length ?? 0; }

  // ------------------------------------------------------------ transform

  /** Image coordinates (float) of a client point relative to the view. */
  toImage(sx, sy) {
    const o = this.origin();
    return { x: (sx - o.x) / this.zoom, y: (sy - o.y) / this.zoom };
  }
  toScreen(ix, iy) {
    const o = this.origin();
    return { x: ix * this.zoom + o.x, y: iy * this.zoom + o.y };
  }

  /**
   * Screen position (CSS px) of the image's top-left corner, snapped to whole
   * device pixels: otherwise an odd-sized view puts image pixel edges on
   * half pixels and the display drops or doubles rows of fine detail.
   */
  origin() {
    const d = this.dpr;
    return {
      x: Math.round((this.viewW / 2 - this.cx * this.zoom) * d) / d,
      y: Math.round((this.viewH / 2 - this.cy * this.zoom) * d) / d,
    };
  }

  setZoom(z, sx = this.viewW / 2, sy = this.viewH / 2) {
    z = Math.min(64, Math.max(1 / 64, z));
    const before = this.toImage(sx, sy);
    this.zoom = z;
    const after = this.toImage(sx, sy);
    this.cx += before.x - after.x;
    this.cy += before.y - after.y;
    this.clampPan();
    this.changed();
  }

  /** Steps through Paint.NET-style zoom presets. */
  zoomStep(dir, sx, sy) {
    const presets = [1 / 64, 1 / 32, 1 / 24, 1 / 16, 1 / 12, 1 / 8, 1 / 6, 1 / 4, 1 / 3, 1 / 2, 2 / 3, 1, 1.5, 2, 3, 4, 5, 6, 7, 8, 10, 12, 16, 24, 32, 48, 64];
    const z = this.zoom;
    const next = dir > 0 ? presets.find(p => p > z * 1.001) : [...presets].reverse().find(p => p < z / 1.001);
    if (next) this.setZoom(next, sx, sy);
  }

  fit() {
    if (!this.docW) return;
    const z = Math.min((this.viewW - 32) / this.docW, (this.viewH - 32) / this.docH, 1);
    this.zoom = Math.max(1 / 64, z);
    this.cx = this.docW / 2;
    this.cy = this.docH / 2;
    this.changed();
  }

  actualSize() {
    this.zoom = 1;
    this.clampPan();
    this.changed();
  }

  zoomToRect(x, y, w, h) {
    if (w <= 0 || h <= 0) return;
    this.zoom = Math.min(64, Math.max(1 / 64, Math.min((this.viewW - 32) / w, (this.viewH - 32) / h)));
    this.cx = x + w / 2;
    this.cy = y + h / 2;
    this.clampPan();
    this.changed();
  }

  panBy(dx, dy) {
    this.cx -= dx / this.zoom;
    this.cy -= dy / this.zoom;
    this.clampPan();
    this.changed();
  }

  clampPan() {
    // Keep the document centred when it fits; otherwise allow scrolling to each edge.
    const hw = this.viewW / 2 / this.zoom, hh = this.viewH / 2 / this.zoom;
    const clamp = (c, size, half) => (size <= 2 * half ? size / 2 : Math.min(size - half + 16 / this.zoom, Math.max(half - 16 / this.zoom, c)));
    this.cx = clamp(this.cx, this.docW, hw);
    this.cy = clamp(this.cy, this.docH, hh);
  }

  /** Top-left visible image pixel (paste origin). */
  visibleTopLeft() {
    const p = this.toImage(0, 0);
    return { x: Math.max(0, Math.floor(p.x)), y: Math.max(0, Math.floor(p.y)) };
  }

  // ------------------------------------------------------------ drawing

  draw() {
    this.dirty = false;
    const d = this.dpr;
    const css = getComputedStyle(this.host);
    const bg = css.getPropertyValue('--workspace').trim() || '#3a3d41';
    if (!this.gl) return this.draw2d(bg);
    const gl = this.gl;
    const [r, g, b] = hexToRgb(bg);
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(r, g, b, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    if (!this.docW) return;
    gl.useProgram(this.prog);
    const W = this.canvas.width, H = this.canvas.height;
    const z = this.zoom * d;
    // image → device px: (i - c) * z + W/2 ; device → clip: px / W * 2 - 1 (y flipped)
    gl.uniform2f(this.loc.uScale, (2 * z) / W, (-2 * z) / H);
    const o = this.origin(); // device-pixel aligned
    gl.uniform2f(this.loc.uOffset, ((o.x * d) / W) * 2 - 1, 1 - ((o.y * d) / H) * 2);
    gl.uniform1f(this.loc.uChecker, Math.round(8 * d));
    gl.uniform1f(this.loc.uGrid, this.pixelGrid && this.zoom >= 8 ? 1 : 0);
    gl.uniform1f(this.loc.uZoom, z);
    gl.uniform3f(this.loc.uCheckA, 1, 1, 1);
    gl.uniform3f(this.loc.uCheckB, 0.8, 0.8, 0.8);
    gl.uniform1i(this.loc.uTex, 0);
    const f = this.filter;
    gl.uniform1i(this.loc.uFilter, f.matrix ? 1 : f.id === 'falsecolor' ? 2 : 0);
    if (f.matrix) {
      const m = f.matrix; // row-major → column-major
      gl.uniformMatrix3fv(this.loc.uFilterMat, false, [m[0], m[3], m[6], m[1], m[4], m[7], m[2], m[5], m[8]]);
    }
    gl.activeTexture(gl.TEXTURE0);
    const smooth = this.zoom < 1;
    gl.bindBuffer(gl.ARRAY_BUFFER, this.vbo);
    const aPos = gl.getAttribLocation(this.prog, 'aPos');
    gl.enableVertexAttribArray(aPos);
    gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 0, 0);
    for (const t of this.tiles) {
      gl.bindTexture(gl.TEXTURE_2D, t.tex);
      if (smooth && t.mipDirty) {
        gl.generateMipmap(gl.TEXTURE_2D);
        t.mipDirty = false;
      }
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, smooth ? gl.LINEAR_MIPMAP_LINEAR : gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.uniform2f(this.loc.uTileOrigin, t.x, t.y);
      gl.uniform2f(this.loc.uTileSize, t.w, t.h);
      const x0 = t.x, y0 = t.y, x1 = t.x + t.w, y1 = t.y + t.h;
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([x0, y0, x1, y0, x0, y1, x0, y1, x1, y0, x1, y1]), gl.STREAM_DRAW);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
    }
  }

  draw2d(bg) {
    const c = this.ctx2d, d = this.dpr;
    c.setTransform(1, 0, 0, 1, 0, 0);
    c.fillStyle = bg;
    c.fillRect(0, 0, this.canvas.width, this.canvas.height);
    if (!this.docW) return;
    const p = this.toScreen(0, 0);
    const [x, y, w, h] = [p.x * d, p.y * d, this.docW * this.zoom * d, this.docH * this.zoom * d];
    if (!this.checker) {
      const t = new OffscreenCanvas(16, 16), tc = t.getContext('2d');
      tc.fillStyle = '#fff'; tc.fillRect(0, 0, 16, 16);
      tc.fillStyle = '#ccc'; tc.fillRect(0, 0, 8, 8); tc.fillRect(8, 8, 8, 8);
      this.checker = c.createPattern(t, 'repeat');
    }
    c.fillStyle = this.checker;
    c.fillRect(x, y, w, h);
    c.imageSmoothingEnabled = this.zoom < 1;
    c.drawImage(this.img, x, y, w, h);
  }

  requestOverlay() { this.overlayDirty = true; }

  /** Control points of a pending line/curve, and a grip to move it. */
  drawShapeEdit(c, S) {
    const { curve, points } = this.shapeEdit;
    const pts = points.map(([x, y]) => S(x + 0.5, y + 0.5));
    c.save();
    c.lineWidth = 1;
    if (curve === 'Bezier' && pts.length === 4) {
      c.setLineDash([3, 3]);
      c.strokeStyle = 'rgba(30,102,208,0.9)';
      c.beginPath(); c.moveTo(pts[0].x, pts[0].y); c.lineTo(pts[1].x, pts[1].y); c.moveTo(pts[3].x, pts[3].y); c.lineTo(pts[2].x, pts[2].y); c.stroke();
      c.setLineDash([]);
    }
    pts.forEach((p, i) => {
      const end = i === 0 || i === pts.length - 1;
      c.fillStyle = end ? '#fff' : '#cfe0ff';
      c.strokeStyle = '#1e66d0';
      c.lineWidth = 1.5;
      c.beginPath();
      if (end) c.rect(p.x - 4, p.y - 4, 8, 8); else c.arc(p.x, p.y, 4.5, 0, Math.PI * 2);
      c.fill(); c.stroke();
    });
    // Move grip: below the shape (above it if there is no room).
    const xs = pts.map(p => p.x), ys = pts.map(p => p.y);
    const gx = (Math.min(...xs) + Math.max(...xs)) / 2;
    let gy = Math.max(...ys) + 26;
    if (gy > this.viewH - 12) gy = Math.min(...ys) - 26;
    this.shapeGrip = { x: gx, y: gy };
    this.drawGrip(c, gx, gy);
    this.rotateGrip = { x: gx + 26, y: gy };
    this.drawRotateGrip(c, gx + 26, gy);
    c.restore();
  }

  /** A pending gradient: its line, end squares, colour stops and move/rotate grips. */
  drawGradientEdit(c, S) {
    const { a, b, stops, selected } = this.shapeEdit;
    const A = S(a[0] + 0.5, a[1] + 0.5), B = S(b[0] + 0.5, b[1] + 0.5);
    c.save();
    // The line: dark under light, visible on any image.
    c.lineWidth = 3; c.strokeStyle = 'rgba(0,0,0,0.45)';
    c.beginPath(); c.moveTo(A.x, A.y); c.lineTo(B.x, B.y); c.stroke();
    c.lineWidth = 1.5; c.strokeStyle = '#fff';
    c.beginPath(); c.moveTo(A.x, A.y); c.lineTo(B.x, B.y); c.stroke();
    stops.forEach(([t, color], i) => {
      const p = { x: A.x + (B.x - A.x) * t, y: A.y + (B.y - A.y) * t };
      const end = i === 0 || i === stops.length - 1;
      const sel = i === selected;
      const r = sel ? 7 : 5.5;
      c.beginPath();
      if (end) c.rect(p.x - r, p.y - r, r * 2, r * 2); else c.arc(p.x, p.y, r, 0, Math.PI * 2);
      // Checker behind see-through colours.
      c.fillStyle = '#fff'; c.fill();
      c.fillStyle = color; c.fill();
      c.lineWidth = sel ? 2.5 : 1.5;
      c.strokeStyle = sel ? '#1e66d0' : '#333';
      c.stroke();
      if (sel) { c.lineWidth = 1; c.strokeStyle = '#fff'; c.beginPath(); if (end) c.rect(p.x - r - 2, p.y - r - 2, r * 2 + 4, r * 2 + 4); else c.arc(p.x, p.y, r + 2, 0, Math.PI * 2); c.stroke(); }
    });
    // Move and rotate grips, below the line (above if no room).
    const gx = (A.x + B.x) / 2;
    let gy = Math.max(A.y, B.y) + 26;
    if (gy > this.viewH - 12) gy = Math.min(A.y, B.y) - 26;
    this.shapeGrip = { x: gx, y: gy };
    this.drawGrip(c, gx, gy);
    this.rotateGrip = { x: gx + 26, y: gy };
    this.drawRotateGrip(c, gx + 26, gy);
    c.restore();
  }

  /** A pending perspective transform: outline, corner squares, edge diamonds. */
  drawQuadEdit(c, S) {
    const pts = this.shapeEdit.corners.map(([x, y]) => S(x, y));
    c.save();
    c.lineWidth = 1;
    c.setLineDash([4, 3]);
    c.strokeStyle = 'rgba(30,102,208,0.95)';
    c.beginPath(); pts.forEach((p, i) => (i ? c.lineTo(p.x, p.y) : c.moveTo(p.x, p.y))); c.closePath(); c.stroke();
    c.setLineDash([]);
    // Faint guides: the mid lines show the perspective.
    const mid = (p, q) => ({ x: (p.x + q.x) / 2, y: (p.y + q.y) / 2 });
    c.strokeStyle = 'rgba(30,102,208,0.35)';
    c.beginPath();
    const m = [0, 1, 2, 3].map(i => mid(pts[i], pts[(i + 1) % 4]));
    c.moveTo(m[0].x, m[0].y); c.lineTo(m[2].x, m[2].y); c.moveTo(m[1].x, m[1].y); c.lineTo(m[3].x, m[3].y); c.stroke();
    c.lineWidth = 1.5; c.strokeStyle = '#1e66d0';
    for (const p of pts) { c.fillStyle = '#fff'; c.beginPath(); c.rect(Math.round(p.x) - 5, Math.round(p.y) - 5, 10, 10); c.fill(); c.stroke(); }
    for (const p of m) {
      c.fillStyle = '#cfe0ff';
      c.beginPath(); c.moveTo(p.x, p.y - 5.5); c.lineTo(p.x + 5.5, p.y); c.lineTo(p.x, p.y + 5.5); c.lineTo(p.x - 5.5, p.y); c.closePath(); c.fill(); c.stroke();
    }
    c.restore();
  }

  /** A curved-arrow handle for rotating. */
  drawRotateGrip(c, x, y) {
    c.fillStyle = '#fff';
    c.strokeStyle = '#1e66d0';
    c.lineWidth = 1.5;
    c.beginPath(); c.arc(x, y, 10, 0, Math.PI * 2); c.fill(); c.stroke();
    c.beginPath(); c.arc(x, y, 5.5, -Math.PI * 0.9, Math.PI * 0.55); c.stroke();
    const ex = x + 5.5 * Math.cos(Math.PI * 0.55), ey = y + 5.5 * Math.sin(Math.PI * 0.55);
    c.beginPath(); c.moveTo(ex - 3.5, ey - 0.5); c.lineTo(ex, ey); c.lineTo(ex + 0.5, ey - 3.5); c.stroke();
  }

  /** Handles of a pending rectangle/ellipse: 8 resize squares and a rotation handle. */
  drawBoxEdit(c, S) {
    const { cx, cy, w, h, angle } = this.shapeEdit;
    const cos = Math.cos(angle), sin = Math.sin(angle);
    const W = (u, v) => S(cx + u * cos - v * sin, cy + u * sin + v * cos);
    const corners = [[-1, -1], [1, -1], [1, 1], [-1, 1]].map(([a, b]) => W(a * w / 2, b * h / 2));
    c.save();
    c.lineWidth = 1;
    c.setLineDash([4, 3]);
    c.strokeStyle = 'rgba(30,102,208,0.9)';
    c.beginPath(); corners.forEach((p, i) => (i ? c.lineTo(p.x, p.y) : c.moveTo(p.x, p.y))); c.closePath(); c.stroke();
    c.setLineDash([]);
    // Rotation handle: 24 px beyond the top edge, along the box's "up".
    const top = W(0, -h / 2);
    const up = { x: sin, y: -cos };
    const rh = { x: top.x + up.x * 24, y: top.y + up.y * 24 };
    c.strokeStyle = '#1e66d0';
    c.beginPath(); c.moveTo(top.x, top.y); c.lineTo(rh.x, rh.y); c.stroke();
    for (const [a, b] of [[-1, -1], [0, -1], [1, -1], [1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0]]) {
      const p = W(a * w / 2, b * h / 2);
      c.fillStyle = '#fff'; c.lineWidth = 1.5;
      c.beginPath(); c.rect(Math.round(p.x) - 4, Math.round(p.y) - 4, 8, 8); c.fill(); c.stroke();
    }
    this.drawRotateGrip(c, rh.x, rh.y);
    // Custom polygon corners.
    for (const [vx, vy] of this.shapeEdit.vertices ?? []) {
      const p = S(vx, vy);
      c.fillStyle = '#cfe0ff'; c.strokeStyle = '#1e66d0'; c.lineWidth = 1.5;
      c.beginPath(); c.arc(p.x, p.y, 4.5, 0, Math.PI * 2); c.fill(); c.stroke();
    }
    c.restore();
  }

  drawGrip(c, x, y) {
    c.fillStyle = '#fff';
    c.strokeStyle = '#1e66d0';
    c.lineWidth = 1.5;
    c.beginPath(); c.arc(x, y, 10, 0, Math.PI * 2); c.fill(); c.stroke();
    c.beginPath();
    for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) {
      c.moveTo(x, y); c.lineTo(x + dx * 6, y + dy * 6);
      c.moveTo(x + dx * 6, y + dy * 6); c.lineTo(x + dx * 4 - dy * 2, y + dy * 4 - dx * 2);
      c.moveTo(x + dx * 6, y + dy * 6); c.lineTo(x + dx * 4 + dy * 2, y + dy * 4 + dx * 2);
    }
    c.stroke();
  }

  /** Text being typed: its (possibly rotated) box, caret and rotation handle. */
  drawTextBox(c, S) {
    const { box, caret, angle = 0, ax = box.x, ay = box.y } = this.textBox;
    const cos = Math.cos(angle), sin = Math.sin(angle);
    const R = (x, y) => S(ax + (x - ax) * cos - (y - ay) * sin, ay + (x - ax) * sin + (y - ay) * cos);
    const m = 3 / this.zoom;
    const corners = [[box.x - m, box.y - m], [box.x + box.w + m, box.y - m], [box.x + box.w + m, box.y + box.h + m], [box.x - m, box.y + box.h + m]].map(([x, y]) => R(x, y));
    c.save();
    c.lineWidth = 1;
    c.setLineDash([4, 3]);
    c.strokeStyle = 'rgba(30,102,208,0.9)';
    c.beginPath(); corners.forEach((p, i) => (i ? c.lineTo(p.x, p.y) : c.moveTo(p.x, p.y))); c.closePath(); c.stroke();
    c.setLineDash([]);
    if (this.caretOn && caret) {
      const p = R(caret.x, caret.y), q = R(caret.x, caret.y + caret.h);
      for (const [lw, col] of [[3, '#fff'], [1.5, '#000']]) {
        c.lineWidth = lw; c.strokeStyle = col;
        c.beginPath(); c.moveTo(p.x, p.y); c.lineTo(q.x, q.y); c.stroke();
      }
    }
    // Rotation handle above the middle of the top edge.
    const top = { x: (corners[0].x + corners[1].x) / 2, y: (corners[0].y + corners[1].y) / 2 };
    const rh = { x: top.x + sin * 24, y: top.y - cos * 24 };
    c.strokeStyle = '#1e66d0'; c.lineWidth = 1;
    c.beginPath(); c.moveTo(top.x, top.y); c.lineTo(rh.x, rh.y); c.stroke();
    this.rotateGrip = rh;
    this.drawRotateGrip(c, rh.x, rh.y);
    c.restore();
  }

  drawOverlay() {
    this.overlayDirty = false;
    const c = this.ov, d = this.dpr;
    c.setTransform(1, 0, 0, 1, 0, 0);
    c.clearRect(0, 0, this.overlay.width, this.overlay.height);
    c.setTransform(d, 0, 0, d, 0, 0);
    const S = (x, y) => this.toScreen(x, y);
    // Selection: marching ants (dashed black over white).
    if (this.selection?.outline?.length) {
      const off = this.selection.polygon ? 0.5 : 0;
      c.beginPath();
      for (const poly of this.selection.outline) {
        for (let i = 0; i < poly.length; i += 2) {
          const p = S(poly[i] + off, poly[i + 1] + off);
          const px = Math.round(p.x) + 0.5, py = Math.round(p.y) + 0.5;
          if (i === 0) c.moveTo(px, py); else c.lineTo(px, py);
        }
        c.closePath();
      }
      if (this.selectionTint) {
        c.fillStyle = this.selectionTint;
        c.fill('evenodd');
      }
      c.lineWidth = 1;
      c.setLineDash([]);
      c.strokeStyle = '#fff';
      c.stroke();
      c.setLineDash([4, 4]);
      c.lineDashOffset = -this.antsPhase;
      c.strokeStyle = '#000';
      c.stroke();
      c.setLineDash([]);
    }
    this.shapeGrip = null;
    this.rotateGrip = null;
    if (this.shapeEdit?.kind === 'line' && this.shapeEdit.points?.length) this.drawShapeEdit(c, S);
    if (this.shapeEdit?.kind === 'shape') this.drawBoxEdit(c, S);
    if (this.shapeEdit?.kind === 'gradient') this.drawGradientEdit(c, S);
    if (this.shapeEdit?.kind === 'quad') this.drawQuadEdit(c, S);
    if (this.textBox) this.drawTextBox(c, S);
    if (this.lasso?.length > 1) {
      c.beginPath();
      this.lasso.forEach(([x, y], i) => { const p = S(x + 0.5, y + 0.5); i ? c.lineTo(p.x, p.y) : c.moveTo(p.x, p.y); });
      c.strokeStyle = '#000';
      c.setLineDash([3, 3]);
      c.stroke();
      c.setLineDash([]);
    }
    if (this.zoomRect) {
      const [x0, y0, x1, y1] = this.zoomRect;
      const a = S(Math.min(x0, x1), Math.min(y0, y1)), b = S(Math.max(x0, x1), Math.max(y0, y1));
      c.strokeStyle = '#0a84ff';
      c.strokeRect(a.x + 0.5, a.y + 0.5, b.x - a.x, b.y - a.y);
    }
    // Transform handles for the move tools.
    if (this.handles) {
      const [x, y, w, h] = this.handles;
      const a = S(x, y), b = S(x + w, y + h);
      c.strokeStyle = 'rgba(0,0,0,0.6)';
      c.setLineDash([2, 2]);
      c.strokeRect(Math.round(a.x) + 0.5, Math.round(a.y) + 0.5, Math.round(b.x - a.x), Math.round(b.y - a.y));
      c.setLineDash([]);
      const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
      for (const [hx, hy] of [[a.x, a.y], [mx, a.y], [b.x, a.y], [b.x, my], [b.x, b.y], [mx, b.y], [a.x, b.y], [a.x, my]]) {
        c.fillStyle = '#fff';
        c.strokeStyle = '#1e66d0';
        c.beginPath();
        c.rect(Math.round(hx) - 4.5, Math.round(hy) - 4.5, 9, 9);
        c.fill();
        c.stroke();
      }
    }
    // Brush outline.
    if (this.cursor && this.cursor.d > 0) {
      const { x, y, d: dia } = this.cursor;
      const p = S(x + 0.5, y + 0.5);
      const r = Math.max(0.5, (dia / 2) * this.zoom);
      c.beginPath();
      c.arc(p.x, p.y, r, 0, Math.PI * 2);
      c.strokeStyle = 'rgba(255,255,255,0.9)';
      c.lineWidth = 2.5;
      c.stroke();
      c.strokeStyle = 'rgba(0,0,0,0.9)';
      c.lineWidth = 1;
      c.stroke();
    }
  }
}

function hexToRgb(s) {
  const m = /^#?([0-9a-f]{6})$/i.exec(s.trim());
  if (m) {
    const n = parseInt(m[1], 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
  }
  const r = /rgba?\(([^)]+)\)/.exec(s);
  if (r) {
    const [a, b, c] = r[1].split(',').map(v => parseFloat(v) / 255);
    return [a, b, c];
  }
  return [0.23, 0.24, 0.26];
}
