//! Per-pixel adjustments (C# `Adjustments`).

use crate::color::Bgra;
use crate::num::{clamp_byte_f64, clamp_byte_i, round_even};
use crate::selection::Selection;
use crate::surface::Surface;

/// A colour → colour mapping applied independently to every pixel.
pub trait PixelOp: Sync {
    fn apply(&self, c: Bgra) -> Bgra;
}

/// Mask-weighted lerp from `original` to `transformed` (C# `LerpByMask`).
#[inline]
pub fn lerp_by_mask(o: Bgra, t: Bgra, m: u8) -> Bgra {
    let m = m as i32;
    let l = |a: u8, b: u8| ((a as i32 * (255 - m) + b as i32 * m + 127) / 255) as u8;
    Bgra::new(l(o.b, t.b), l(o.g, t.g), l(o.r, t.r), l(o.a, t.a))
}

/// Applies `op` to the surface, weighted by the selection when there is one.
pub fn apply_op(op: &dyn PixelOp, s: &mut Surface, sel: Option<&Selection>) {
    let w = s.width() as usize;
    match sel.filter(|m| !m.is_empty()) {
        Some(sel) => {
            let b = sel.bounds();
            let y1 = s.height().min(b.bottom());
            let x1 = s.width().min(b.right());
            let rows = &mut s.pixels_mut()[b.y as usize * w..y1 as usize * w];
            crate::par::for_rows(rows, w, |i, row| {
                let y = b.y + i as i32;
                for x in b.x..x1 {
                    let m = sel.coverage_at(x, y);
                    if m == 0 {
                        continue;
                    }
                    let p = &mut row[x as usize];
                    let t = op.apply(*p);
                    *p = if m == 255 { t } else { lerp_by_mask(*p, t, m) };
                }
            });
        }
        None => crate::par::for_rows(s.pixels_mut(), w, |_, row| {
            for p in row {
                *p = op.apply(*p);
            }
        }),
    }
}

pub struct Identity;
impl PixelOp for Identity {
    fn apply(&self, c: Bgra) -> Bgra {
        c
    }
}

pub struct InvertColors;
impl PixelOp for InvertColors {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        Bgra::new(255 - c.b, 255 - c.g, 255 - c.r, c.a)
    }
}

pub struct Desaturate;
impl PixelOp for Desaturate {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        let i = c.intensity_byte();
        Bgra::new(i, i, i, c.a)
    }
}

/// Blends a constant colour over the RGB channels by its alpha.
pub struct BlendConstant(pub Bgra);
impl PixelOp for BlendConstant {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        let a = self.0.a as i32;
        let l = |x: u8, y: u8| ((x as i32 * (255 - a) + y as i32 * a + 127) / 255) as u8;
        Bgra::new(l(c.b, self.0.b), l(c.g, self.0.g), l(c.r, self.0.r), c.a)
    }
}

/// Lookup-table op: one 256-entry table per channel (B, G, R).
pub struct ChannelLut {
    pub b: [u8; 256],
    pub g: [u8; 256],
    pub r: [u8; 256],
}
impl PixelOp for ChannelLut {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        Bgra::new(self.b[c.b as usize], self.g[c.g as usize], self.r[c.r as usize], c.a)
    }
}

// ------------------------------------------------------------ Brightness/Contrast

pub struct BrightnessContrast {
    divide: i32,
    table: Vec<u8>,
}

impl BrightnessContrast {
    pub fn new(brightness: i32, contrast: i32) -> Self {
        let brightness = brightness.clamp(-100, 100);
        let contrast = contrast.clamp(-100, 100);
        let (multiply, divide) = if contrast < 0 {
            (contrast + 100, 100)
        } else if contrast > 0 {
            (100, 100 - contrast)
        } else {
            (1, 1)
        };
        let mut table = vec![0u8; 65536];
        if divide == 0 {
            for i in 0..256 {
                table[i as usize] = if i + brightness < 128 { 0 } else { 255 };
            }
        } else {
            for i in 0..256i32 {
                let shift = if divide == 100 {
                    (i - 127) * multiply / divide + 127 - i + brightness
                } else {
                    (i - 127 + brightness) * multiply / divide + 127 - i
                };
                for col in 0..256i32 {
                    table[(i * 256 + col) as usize] = clamp_byte_i(col + shift);
                }
            }
        }
        BrightnessContrast { divide, table }
    }
}

impl PixelOp for BrightnessContrast {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        let i = c.intensity_byte() as usize;
        if self.divide == 0 {
            let v = self.table[i];
            return Bgra::new(v, v, v, c.a);
        }
        let t = &self.table[i * 256..i * 256 + 256];
        Bgra::new(t[c.b as usize], t[c.g as usize], t[c.r as usize], c.a)
    }
}

// ------------------------------------------------------------ Posterize

fn posterize_levels(level_count: i32) -> [u8; 256] {
    let n = level_count.clamp(2, 64);
    let mut t1 = vec![0u8; n as usize];
    for i in 1..n {
        t1[i as usize] = (255 * i / (n - 1)) as u8;
    }
    let mut levels = [0u8; 256];
    let (mut j, mut k) = (0usize, 0i32);
    for l in levels.iter_mut() {
        *l = t1[j];
        k += n;
        if k > 255 {
            k -= 255;
            j += 1;
        }
    }
    levels
}

pub fn posterize(red: i32, green: i32, blue: i32) -> ChannelLut {
    ChannelLut { b: posterize_levels(blue), g: posterize_levels(green), r: posterize_levels(red) }
}

// ------------------------------------------------------------ Curves

/// Natural cubic spline through control points (C# `SplineInterpolator`).
pub struct Spline {
    xs: Vec<f64>,
    ys: Vec<f64>,
    y2: Vec<f64>,
}

impl Spline {
    /// Points need not be sorted; a repeated x keeps the last y.
    pub fn new(points: &[(f64, f64)]) -> Spline {
        let mut pts: Vec<(f64, f64)> = Vec::new();
        for &(x, y) in points {
            match pts.iter_mut().find(|p| p.0 == x) {
                Some(p) => p.1 = y,
                None => pts.push((x, y)),
            }
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let xs: Vec<f64> = pts.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = pts.iter().map(|p| p.1).collect();
        let n = xs.len();
        let mut u = vec![0.0; n];
        let mut y2 = vec![0.0; n];
        for i in 1..n.saturating_sub(1) {
            let wx = xs[i + 1] - xs[i - 1];
            let sig = (xs[i] - xs[i - 1]) / wx;
            let p = sig * y2[i - 1] + 2.0;
            y2[i] = (sig - 1.0) / p;
            let ddydx = (ys[i + 1] - ys[i]) / (xs[i + 1] - xs[i]) - (ys[i] - ys[i - 1]) / (xs[i] - xs[i - 1]);
            u[i] = (6.0 * ddydx / wx - sig * u[i - 1]) / p;
        }
        if n >= 2 {
            for i in (0..=n - 2).rev() {
                y2[i] = y2[i] * y2[i + 1] + u[i];
            }
        }
        Spline { xs, ys, y2 }
    }

    pub fn interpolate(&self, x: f64) -> f64 {
        let (xa, ya, y2) = (&self.xs, &self.ys, &self.y2);
        let (mut klo, mut khi) = (0usize, xa.len() - 1);
        while khi - klo > 1 {
            let k = (khi + klo) >> 1;
            if xa[k] > x {
                khi = k;
            } else {
                klo = k;
            }
        }
        let h = xa[khi] - xa[klo];
        let a = (xa[khi] - x) / h;
        let b = (x - xa[klo]) / h;
        a * ya[klo] + b * ya[khi] + ((a * a * a - a) * y2[klo] + (b * b * b - b) * y2[khi]) * (h * h) / 6.0
    }

    /// 256-entry curve LUT from integer control points (≥ 2).
    pub fn build_lut(points: &[(i32, i32)]) -> [u8; 256] {
        assert!(points.len() >= 2, "at least two control points required");
        let pts: Vec<(f64, f64)> = points.iter().map(|&(x, y)| (x as f64, y as f64)).collect();
        let s = Spline::new(&pts);
        let mut lut = [0u8; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = clamp_byte_f64(s.interpolate(i as f64));
        }
        lut
    }
}

pub fn channel_curve(r: [u8; 256], g: [u8; 256], b: [u8; 256]) -> ChannelLut {
    ChannelLut { b, g, r }
}

pub struct LuminosityCurve(pub [u8; 256]);
impl PixelOp for LuminosityCurve {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        let l = c.intensity_byte();
        let d = self.0[l as usize] as i32 - l as i32;
        Bgra::new(clamp_byte_i(c.b as i32 + d), clamp_byte_i(c.g as i32 + d), clamp_byte_i(c.r as i32 + d), c.a)
    }
}

// ------------------------------------------------------------ Hue/Saturation

pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = rf.max(gf.max(bf));
    let min = rf.min(gf.min(bf));
    let delta = max - min;
    let mut h = 0.0;
    if delta > 0.0 {
        h = if max == rf {
            (gf - bf) / delta % 6.0
        } else if max == gf {
            (bf - rf) / delta + 2.0
        } else {
            (rf - gf) / delta + 4.0
        };
        h *= 60.0;
        if h < 0.0 {
            h += 360.0;
        }
    }
    let s = if max <= 0.0 { 0.0 } else { delta / max };
    (h, s, max)
}

pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let h = (h % 360.0 + 360.0) % 360.0;
    let c = v * s;
    let x = c * (1.0 - (h / 60.0 % 2.0 - 1.0).abs());
    let m = v - c;
    let (rf, gf, bf) = match (h / 60.0) as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |f: f64| round_even((f + m) * 255.0).clamp(0.0, 255.0) as u8;
    (to(rf), to(gf), to(bf))
}

pub struct HueSaturation {
    hue_delta: i32,
    sat_factor: i32,
    lightness: Option<BlendConstant>,
}

impl HueSaturation {
    pub fn new(hue_delta: i32, saturation: i32, lightness: i32) -> Self {
        let lightness = match lightness {
            0 => None,
            l if l > 0 => Some(BlendConstant(Bgra::new(255, 255, 255, (l * 255 / 100) as u8))),
            l => Some(BlendConstant(Bgra::new(0, 0, 0, (-l * 255 / 100) as u8))),
        };
        HueSaturation { hue_delta, sat_factor: saturation * 1024 / 100, lightness }
    }
}

impl PixelOp for HueSaturation {
    fn apply(&self, c: Bgra) -> Bgra {
        let i = c.intensity_byte() as i32;
        let sat = |v: u8| clamp_byte_i((i * 1024 + (v as i32 - i) * self.sat_factor) >> 10);
        let (r, g, b) = (sat(c.r), sat(c.g), sat(c.b));
        let (h, s, v) = rgb_to_hsv(r, g, b);
        let (nr, ng, nb) = hsv_to_rgb(h + self.hue_delta as f64, s, v);
        let mut out = Bgra::new(nb, ng, nr, c.a);
        if let Some(l) = &self.lightness {
            out = l.apply(out);
        }
        out.with_alpha(c.a)
    }
}

// ------------------------------------------------------------ Levels

fn level_channel(value: i32, in_lo: u8, in_hi: u8, gamma: f32, out_lo: u8, out_hi: u8) -> u8 {
    let v = value - in_lo as i32;
    if v < 0 {
        return out_lo;
    }
    if value >= in_hi as i32 {
        return out_hi;
    }
    let g = gamma.clamp(0.1, 10.0);
    let base = v as f32 / (in_hi as i32 - in_lo as i32) as f32;
    let result = out_lo as f64 + (out_hi as i32 - out_lo as i32) as f64 * (base as f64).powf(g as f64);
    clamp_byte_f64(result)
}

/// Levels; `gamma` is in B, G, R order like the C# API.
pub fn levels(in_low: Bgra, in_high: Bgra, gamma: [f32; 3], out_low: Bgra, out_high: Bgra) -> ChannelLut {
    let mut lut = ChannelLut { b: [0; 256], g: [0; 256], r: [0; 256] };
    for j in 0..256 {
        lut.b[j] = level_channel(j as i32, in_low.b, in_high.b, gamma[0], out_low.b, out_high.b);
        lut.g[j] = level_channel(j as i32, in_low.g, in_high.g, gamma[1], out_low.g, out_high.g);
        lut.r[j] = level_channel(j as i32, in_low.r, in_high.r, gamma[2], out_low.r, out_high.r);
    }
    lut
}

/// Auto levels from low / mid / high sample colours.
pub fn levels_auto_from(lo: Bgra, md: Bgra, hi: Bgra) -> ChannelLut {
    let ch = [(lo.b, md.b, hi.b), (lo.g, md.g, hi.g), (lo.r, md.r, hi.r)];
    let mut gamma = [1.0f32; 3];
    for (i, &(l, m, h)) in ch.iter().enumerate() {
        if l < m && m < h {
            let base = (m - l) as f64 / (h - l) as f64;
            gamma[i] = (0.5f64.ln() / base.ln()).clamp(0.1, 10.0) as f32;
        }
    }
    levels(lo, hi, gamma, Bgra::BLACK, Bgra::WHITE)
}

pub struct Sepia {
    level: ChannelLut,
}

impl Default for Sepia {
    fn default() -> Self {
        Sepia { level: levels(Bgra::BLACK, Bgra::WHITE, [1.2, 1.0, 0.8], Bgra::BLACK, Bgra::WHITE) }
    }
}

impl PixelOp for Sepia {
    #[inline]
    fn apply(&self, c: Bgra) -> Bgra {
        self.level.apply(Desaturate.apply(c))
    }
}

/// Per-channel histogram (C# `HistogramRgb`).
pub struct HistogramRgb {
    b: [u64; 256],
    g: [u64; 256],
    r: [u64; 256],
    total: u64,
}

impl HistogramRgb {
    pub fn new(s: &Surface, sel: Option<&Selection>) -> Self {
        let mut h = HistogramRgb { b: [0; 256], g: [0; 256], r: [0; 256], total: 0 };
        let mut add = |c: Bgra| {
            h.b[c.b as usize] += 1;
            h.g[c.g as usize] += 1;
            h.r[c.r as usize] += 1;
            h.total += 1;
        };
        match sel.filter(|m| !m.is_empty()) {
            Some(sel) => {
                let bd = sel.bounds();
                for y in bd.y..s.height().min(bd.bottom()) {
                    let row = s.row(y);
                    for x in bd.x..s.width().min(bd.right()) {
                        if sel.contains(x, y) {
                            add(row[x as usize]);
                        }
                    }
                }
            }
            None => s.pixels().iter().for_each(|&c| add(c)),
        }
        h
    }

    fn percentile(&self, counts: &[u64; 256], fraction: f64) -> u8 {
        if self.total == 0 {
            return 0;
        }
        let threshold = (self.total as f64 * fraction) as u64;
        let mut sum = 0;
        for (i, &c) in counts.iter().enumerate() {
            sum += c;
            if sum > threshold {
                return i as u8;
            }
        }
        255
    }

    pub fn percentile_color(&self, fraction: f64) -> Bgra {
        Bgra::new(self.percentile(&self.b, fraction), self.percentile(&self.g, fraction), self.percentile(&self.r, fraction), 255)
    }

    pub fn mean_color(&self) -> Bgra {
        if self.total == 0 {
            return Bgra::BLACK;
        }
        let m = |c: &[u64; 256]| (c.iter().enumerate().map(|(i, &n)| n * i as u64).sum::<u64>() / self.total) as u8;
        Bgra::new(m(&self.b), m(&self.g), m(&self.r), 255)
    }

    pub fn auto_levels(&self) -> ChannelLut {
        levels_auto_from(self.percentile_color(0.005), self.mean_color(), self.percentile_color(0.995))
    }
}

pub struct RedEyeRemove {
    tolerance: i32,
    set_saturation: f64,
}

impl RedEyeRemove {
    pub fn new(tolerance: i32, saturation_percent: i32) -> Self {
        RedEyeRemove { tolerance, set_saturation: saturation_percent as f64 / 100.0 }
    }
}

impl PixelOp for RedEyeRemove {
    fn apply(&self, c: Bgra) -> Bgra {
        let (r, g, b) = (c.r as f64 / 255.0, c.g as f64 / 255.0, c.b as f64 / 255.0);
        let min = r.min(g).min(b);
        let max = r.max(g).max(b);
        let delta = max - min;
        let s = if max == 0.0 || delta == 0.0 { 0.0 } else { delta / max };
        let saturation = (s * 255.0) as i32;
        let difference = c.r as i32 - c.b.max(c.g) as i32;
        if difference > self.tolerance && saturation > 100 {
            let i = 255.0 * c.intensity();
            return Bgra::new(c.b, c.g, clamp_byte_f64(i * self.set_saturation), c.a);
        }
        c
    }
}
