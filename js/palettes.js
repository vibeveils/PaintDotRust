// Colour palettes: built-in palettes, Paint.NET palette files (.txt), and
// custom palettes kept in the browser (localStorage).
//
// Paint.NET palette format: one colour per line as AARRGGBB (or RRGGBB) hex,
// lines starting with ';' are comments. Paint.NET shows up to 96 colours.

export const MAX_COLORS = 96;

function defaultColors() {
  // Same palette as the C# port: 16 greys, then 5 lightness rows of 16 hues.
  const out = [];
  const h2 = v => Math.round(v).toString(16).padStart(2, '0').toUpperCase();
  for (let i = 0; i < 16; i++) { const v = Math.round(i * 255 / 15); out.push(`${h2(v)}${h2(v)}${h2(v)}`); }
  for (const l of [0.5, 0.75, 0.3, 0.88, 0.18]) {
    for (let k = 0; k < 16; k++) {
      const h = k * 360 / 16, s = 0.85, c = (1 - Math.abs(2 * l - 1)) * s, x = c * (1 - Math.abs((h / 60) % 2 - 1)), m = l - c / 2;
      const [r, g, b] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
      out.push(`${h2((r + m) * 255)}${h2((g + m) * 255)}${h2((b + m) * 255)}`);
    }
  }
  return out;
}

/** Built-in palettes. Colours are RRGGBB (opaque) or RRGGBBAA. */
export const BUILTIN = [
  { id: 'default', name: 'Default', credit: 'PaintDotRust default palette', colors: defaultColors() },
  {"id": "dawnbringer-32", "name": "DawnBringer 32", "credit": "Lospec.com — created by DawnBringer", "colors": ["000000", "222034", "45283C", "663931", "8F563B", "DF7126", "D9A066", "EEC39A", "FBF236", "99E550", "6ABE30", "37946E", "4B692F", "524B24", "323C39", "3F3F74", "306082", "5B6EE1", "639BFF", "5FCDE4", "CBDBFC", "FFFFFF", "9BADB7", "847E87", "696A6A", "595652", "76428A", "AC3232", "D95763", "D77BBA", "8F974A", "8A6F30"]},
  {"id": "aap-64", "name": "AAP-64", "credit": "Lospec.com", "colors": ["060608", "141013", "3B1725", "73172D", "B4202A", "DF3E23", "FA6A0A", "F9A31B", "FFD541", "FFFC40", "D6F264", "9CDB43", "59C135", "14A02E", "1A7A3E", "24523B", "122020", "143464", "285CC4", "249FDE", "20D6C7", "A6FCDB", "FFFFFF", "FEF3C0", "FAD6B8", "F5A097", "E86A73", "BC4A9B", "793A80", "403353", "242234", "221C1A", "322B28", "71413B", "BB7547", "DBA463", "F4D29C", "DAE0EA", "B3B9D1", "8B93AF", "6D758D", "4A5462", "333941", "422433", "5B3138", "8E5252", "BA756A", "E9B5A3", "E3E6FF", "B9BFFB", "849BE4", "588DBE", "477D85", "23674E", "328464", "5DAF8D", "92DCBA", "CDF7E2", "E4D2AA", "C7B08B", "A08662", "796755", "5A4E44", "423934"]},
  {"id": "ammo-8", "name": "Ammo-8", "credit": "Lospec.com — created by rsvp asap", "colors": ["040C06", "112318", "1E3A29", "305D42", "4D8061", "89A257", "BEDC7F", "EEFFCC"]},
  {"id": "ok-31", "name": "👌31", "credit": "Lospec.com — colors sampled from nature", "colors": ["636663", "87857C", "BCAD9F", "F2B888", "EB9661", "B55945", "734C44", "3D3333", "593E47", "7A5859", "A57855", "DE9F47", "FDD179", "FEE1B8", "D4C692", "A6B04F", "819447", "44702D", "2F4D2F", "546756", "89A477", "A4C5AF", "CAE6D9", "F1F6F0", "D5D6DB", "BBC3D0", "96A9C1", "6C81A1", "405273", "303843", "14233A"]},
  {"id": "endesga-32", "name": "Endesga 32", "credit": "Lospec.com — by ENDESGA, originally for NYKRA", "colors": ["BE4A2F", "D77643", "EAD4AA", "E4A672", "B86F50", "733E39", "3E2731", "A22633", "E43B44", "F77622", "FEAE34", "FEE761", "63C74D", "3E8948", "265C42", "193C3E", "124E89", "0099DB", "2CE8F5", "FFFFFF", "C0CBDC", "8B9BB4", "5A6988", "3A4466", "262B44", "181425", "FF0044", "68386C", "B55088", "F6757A", "E8B796", "C28569"]},
  {"id": "lospec500", "name": "Lospec500", "credit": "Lospec.com — collaborative palette from the Lospec Discord", "colors": ["10121C", "2C1E31", "6B2643", "AC2847", "EC273F", "94493A", "DE5D3A", "E98537", "F3A833", "4D3533", "6E4C30", "A26D3F", "CE9248", "DAB163", "E8D282", "F7F3B7", "1E4044", "006554", "26854C", "5AB552", "9DE64E", "008B8B", "62A477", "A6CB96", "D3EED3", "3E3B65", "3859B3", "3388DE", "36C5F4", "6DEAD6", "5E5B8C", "8C78A5", "B0A7B8", "DECEED", "9A4D76", "C878AF", "CC99FF", "FA6E79", "FFA2AC", "FFD1D5", "F6E8E0", "FFFFFF"]},
  {"id": "ega-plus", "name": "EGA Plus", "credit": "Lospec.com — 16 EGA colors plus 8 additions", "colors": ["000000", "0000AA", "00AA00", "00AAAA", "AA0000", "AA00AA", "AA5500", "AAAAAA", "555555", "5555FF", "55FF55", "55FFFF", "FF5555", "FF55FF", "FFFF55", "FFFFFF", "002B55", "2B2BAA", "2B552B", "552B00", "007FFF", "FF2B00", "FF7F00", "FFAA7F"]},
].map(p => ({ ...p, builtin: true }));

const norm = c => c.replace(/^#/, '').toUpperCase();
/** '#RRGGBB' / '#RRGGBBAA' for the engine and CSS. */
export const hex = c => `#${norm(c)}`;

/** Parses a Paint.NET palette file. Returns { name, colors } (colors may be empty). */
export function parse(text, fallbackName = 'Imported palette') {
  let name = fallbackName;
  const colors = [];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) continue;
    if (line.startsWith(';')) {
      const m = /^;\s*Palette Name:\s*(.+)$/i.exec(line);
      if (m) name = m[1].trim();
      continue;
    }
    const v = line.split(/[\s;]/)[0].replace(/^#/, '');
    if (/^[0-9a-f]{8}$/i.test(v)) {
      const a = v.slice(0, 2).toUpperCase(), rgb = v.slice(2).toUpperCase();
      colors.push(a === 'FF' ? rgb : rgb + a);
    } else if (/^[0-9a-f]{6}$/i.test(v)) colors.push(v.toUpperCase());
    if (colors.length >= MAX_COLORS) break;
  }
  return { name: name.slice(0, 80), colors };
}

/** Writes a Paint.NET palette file. */
export function serialize(p) {
  const lines = [';paint.net Palette File', `;Palette Name: ${p.name}`];
  if (p.credit) lines.push(`;Description: ${p.credit}`);
  lines.push(`;Colors: ${p.colors.length}`);
  for (const c of p.colors) {
    const n = norm(c);
    lines.push(`${n.length === 8 ? n.slice(6) : 'FF'}${n.slice(0, 6)}`);
  }
  return lines.join('\r\n') + '\r\n';
}

// ------------------------------------------------------------------ browser storage

const KEY = 'openpdn.palettes', CUR = 'openpdn.palette';

export function loadCustom() {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) || '[]');
    return Array.isArray(v) ? v.filter(p => p && p.id && Array.isArray(p.colors)) : [];
  } catch { return []; }
}
export function saveCustom(list) {
  try { localStorage.setItem(KEY, JSON.stringify(list.map(({ id, name, colors, credit }) => ({ id, name, colors, credit })))); return true; } catch { return false; }
}
export const all = () => [...BUILTIN, ...loadCustom()];
export function currentId() { try { return localStorage.getItem(CUR) || 'default'; } catch { return 'default'; } }
export function setCurrentId(id) { try { localStorage.setItem(CUR, id); } catch { /* private mode */ } }
export const newId = () => `custom-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;

/** A name not already used by another palette ("Name", "Name 2", ...). */
export function uniqueName(name) {
  const names = new Set(all().map(p => p.name));
  if (!names.has(name)) return name;
  for (let i = 2; ; i++) if (!names.has(`${name} ${i}`)) return `${name} ${i}`;
}
