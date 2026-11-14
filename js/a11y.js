// Accessibility and view-filter settings. View filters change only what is
// shown on screen (the renderer applies them); the image itself is untouched.

// Colour-vision deficiency simulation: Machado, Oliveira & Fernandes (2009),
// "A Physiologically-based Model for Simulation of Color Vision Deficiency".
// Row-major 3×3 matrices applied to linear RGB. Anomalous trichromacy uses
// severity 0.6, dichromacy severity 1.0.
const M = {
  protanopia: [0.152286, 1.052583, -0.204868, 0.114503, 0.786281, 0.099216, -0.003882, -0.048116, 1.051998],
  deuteranopia: [0.367322, 0.860646, -0.227968, 0.280085, 0.672501, 0.047413, -0.011820, 0.042940, 0.968881],
  tritanopia: [1.255528, -0.076749, -0.178779, -0.078411, 0.930809, 0.147602, 0.004733, 0.691367, 0.303900],
  protanomaly: [0.385450, 0.769005, -0.154455, 0.100526, 0.829802, 0.069673, -0.007442, -0.022190, 1.029632],
  deuteranomaly: [0.547494, 0.607765, -0.155259, 0.181692, 0.781742, 0.036566, -0.010410, 0.027275, 0.983136],
  tritanomaly: [1.104996, -0.046633, -0.058363, -0.032137, 0.971635, 0.060503, 0.001336, 0.317922, 0.680742],
  achromatopsia: [0.2126, 0.7152, 0.0722, 0.2126, 0.7152, 0.0722, 0.2126, 0.7152, 0.0722],
};

export const FILTERS = [
  { id: 'none', label: 'Normal' },
  { id: 'falsecolor', label: 'False Color (lightness)', hint: 'Shows lightness as a heat map: dark blue is darkest, through green and yellow, to dark red for the lightest areas. Useful for checking contrast and values.' },
  { id: 'achromatopsia', label: 'Grayscale (achromatopsia)', matrix: M.achromatopsia, hint: 'No color vision: only lightness is seen.' },
  '-',
  { id: 'protanopia', label: 'Protanopia (no red cones)', matrix: M.protanopia, hint: 'Reds look dark and are confused with greens and browns. About 1% of men.' },
  { id: 'deuteranopia', label: 'Deuteranopia (no green cones)', matrix: M.deuteranopia, hint: 'Reds and greens are confused. About 1% of men.' },
  { id: 'tritanopia', label: 'Tritanopia (no blue cones)', matrix: M.tritanopia, hint: 'Blues and greens, and yellows and violets, are confused. Rare.' },
  '-',
  { id: 'protanomaly', label: 'Protanomaly (weak red)', matrix: M.protanomaly, hint: 'Reduced red sensitivity. About 1% of men.' },
  { id: 'deuteranomaly', label: 'Deuteranomaly (weak green)', matrix: M.deuteranomaly, hint: 'Reduced green sensitivity; the most common type, about 5% of men.' },
  { id: 'tritanomaly', label: 'Tritanomaly (weak blue)', matrix: M.tritanomaly, hint: 'Reduced blue sensitivity. Rare.' },
];
export const filterById = id => FILTERS.find(f => f !== '-' && f.id === id) ?? FILTERS[0];

const KEY = 'openpdn.a11y';
/** Selected areas get a light blue tint by default. */
export const TINT_DEFAULT = { tint: true, tintColor: '#3b82f6', tintOpacity: 20 };
export function tintCss(s) {
  if (!s.tint) return null;
  const h = s.tintColor.replace('#', '');
  const [r, g, b] = [0, 2, 4].map(i => parseInt(h.slice(i, i + 2), 16));
  return `rgba(${r},${g},${b},${Math.max(0, Math.min(100, s.tintOpacity)) / 100})`;
}
const reduceMotionDefault = () => matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;

export function load() {
  let s = {};
  try { s = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch { /* ignore */ }
  return { dimDialogs: false, filter: 'none', reduceMotion: reduceMotionDefault(), strongFocus: false, winFade: true, winOpacity: 70, uiScale: 100, ...TINT_DEFAULT, ...s };
}

export function save(s) {
  try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* private mode */ }
}

/** Applies document-level settings (dialog dimming, focus outlines). */
export function applyDocument(s) {
  const root = document.documentElement.classList;
  root.toggle('dim-dialogs', !!s.dimDialogs);
  root.toggle('strong-focus', !!s.strongFocus);
  root.toggle('fade-windows', !!s.winFade);
  const st = document.documentElement.style;
  st.setProperty('--idle-opacity', String(Math.max(0.2, Math.min(1, (s.winOpacity ?? 70) / 100))));
  st.setProperty('--ui-scale', String(Math.max(0.5, Math.min(3, (s.uiScale ?? 100) / 100))));
}

// Turbo colour map (Google, Apache-2.0) — polynomial approximation, x in [0, 1].
export function turbo(x) {
  x = Math.min(1, Math.max(0, x));
  const v4 = [1, x, x * x, x * x * x], v2 = [v4[2] * v4[2], v4[3] * v4[2]];
  const d4 = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
  const d2 = (a, b) => a[0] * b[0] + a[1] * b[1];
  return [
    d4(v4, [0.13572138, 4.61539260, -42.66032258, 132.13108234]) + d2(v2, [-152.94239396, 59.28637943]),
    d4(v4, [0.09140261, 2.19418839, 4.84296658, -14.18503333]) + d2(v2, [4.27729857, 2.82956604]),
    d4(v4, [0.10667330, 12.64194608, -60.58204836, 110.36276771]) + d2(v2, [-89.90310912, 27.34824973]),
  ].map(c => Math.min(1, Math.max(0, c)));
}

/** GLSL for the filter stage (straight-alpha sRGB in, sRGB out). */
export const FILTER_GLSL = `
uniform int uFilter;       // 0 none, 1 matrix (linear RGB), 2 false colour
uniform mat3 uFilterMat;
vec3 toLin(vec3 c) { return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(0.04045, c)); }
vec3 toSrgb(vec3 c) { c = clamp(c, 0.0, 1.0); return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c)); }
vec3 turbo(float x) {
  x = clamp(x, 0.0, 1.0);
  vec4 v4 = vec4(1.0, x, x * x, x * x * x);
  vec2 v2 = v4.zw * v4.z;
  return clamp(vec3(
    dot(v4, vec4(0.13572138, 4.61539260, -42.66032258, 132.13108234)) + dot(v2, vec2(-152.94239396, 59.28637943)),
    dot(v4, vec4(0.09140261, 2.19418839, 4.84296658, -14.18503333)) + dot(v2, vec2(4.27729857, 2.82956604)),
    dot(v4, vec4(0.10667330, 12.64194608, -60.58204836, 110.36276771)) + dot(v2, vec2(-89.90310912, 27.34824973))), 0.0, 1.0);
}
vec3 applyFilter(vec3 c) {
  if (uFilter == 1) return toSrgb(uFilterMat * toLin(c));
  if (uFilter == 2) {
    float y = dot(toLin(c), vec3(0.2126, 0.7152, 0.0722));
    float l = y > 0.008856 ? 1.16 * pow(y, 1.0 / 3.0) - 0.16 : 9.033 * y; // CIE L*/100
    return turbo(l);
  }
  return c;
}`;

/** An SVG filter equivalent, for the Canvas 2D fallback renderer (CSS `filter: url(#…)`). */
export function svgFilterMarkup(f) {
  if (f.matrix) {
    const m = f.matrix;
    const vals = [m[0], m[1], m[2], 0, 0, m[3], m[4], m[5], 0, 0, m[6], m[7], m[8], 0, 0, 0, 0, 0, 1, 0].join(' ');
    // Filters interpolate in linear RGB by default, as the matrices expect.
    return `<filter id="openpdn-view-filter"><feColorMatrix type="matrix" values="${vals}"/></filter>`;
  }
  if (f.id === 'falsecolor') {
    const n = 33, cols = [[], [], []];
    for (let i = 0; i < n; i++) turbo(i / (n - 1)).forEach((v, k) => cols[k].push(v.toFixed(4)));
    return `<filter id="openpdn-view-filter" color-interpolation-filters="sRGB">
      <feColorMatrix type="matrix" values="0.2126 0.7152 0.0722 0 0  0.2126 0.7152 0.0722 0 0  0.2126 0.7152 0.0722 0 0  0 0 0 1 0"/>
      <feComponentTransfer><feFuncR type="table" tableValues="${cols[0].join(' ')}"/><feFuncG type="table" tableValues="${cols[1].join(' ')}"/><feFuncB type="table" tableValues="${cols[2].join(' ')}"/></feComponentTransfer>
    </filter>`;
  }
  return '';
}
