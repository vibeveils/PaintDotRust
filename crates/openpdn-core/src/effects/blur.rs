//! Blurs, noise and neighbourhood filters.

use super::dotnet::DotNetRandom;
use super::{cb, lerp, rows, Effect};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::sampling::{bilinear_clamped, bilinear_sample};
use crate::surface::Surface;

// ------------------------------------------------------------ Gaussian

pub struct GaussianBlur(pub i32);

fn gaussian_weights(amount: i32) -> Vec<f32> {
    let size = (1 + amount * 2) as usize;
    let mut w = vec![0i32; size];
    for i in 0..=amount as usize {
        w[i] = 16 * (i as i32 + 1);
        w[size - i - 1] = w[i];
    }
    w.into_iter().map(|v| v as f32).collect()
}

impl Effect for GaussianBlur {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let r = self.0.clamp(0, 200);
        if r == 0 {
            for y in roi.y..roi.bottom() {
                let (a, b) = (roi.x as usize, roi.right() as usize);
                dst.row_mut(y)[a..b].copy_from_slice(&src.row(y)[a..b]);
            }
            return;
        }
        let w = gaussian_weights(r);
        let strip_x0 = (roi.x - r).max(0);
        let strip_x1 = src.width().min(roi.right() + r);
        let sw = (strip_x1 - strip_x0) as usize;
        let n = sw * roi.height as usize;
        // Vertical pass: per pixel of the column strip, five weighted sums.
        let mut sums = vec![[0f32; 5]; n];
        crate::par::for_rows(&mut sums, sw, |i, out| {
            let y = roi.y + i as i32;
            for (wy, &wt) in w.iter().enumerate() {
                let sy = y + wy as i32 - r;
                if (sy as u32) >= src.height() as u32 {
                    continue;
                }
                let row = &src.row(sy)[strip_x0 as usize..strip_x1 as usize];
                for (o, c) in out.iter_mut().zip(row) {
                    o[0] += wt;
                    let wa = wt * c.a as f32;
                    o[1] += wa;
                    o[2] += wa * c.b as f32;
                    o[3] += wa * c.g as f32;
                    o[4] += wa * c.r as f32;
                }
            }
        });
        rows(dst, roi, |y, drow| {
            let base = &sums[(y - roi.y) as usize * sw..][..sw];
            for x in roi.x..roi.right() {
                let (mut tw, mut twa, mut tb, mut tg, mut tr) = (0f32, 0f32, 0f32, 0f32, 0f32);
                for (wx, &wt) in w.iter().enumerate() {
                    let sx = x + wx as i32 - r;
                    if sx < strip_x0 || sx >= strip_x1 {
                        continue;
                    }
                    let s = &base[(sx - strip_x0) as usize];
                    tw += wt * s[0];
                    twa += wt * s[1];
                    tb += wt * s[2];
                    tg += wt * s[3];
                    tr += wt * s[4];
                }
                drow[x as usize] = if tw <= 0.0 || twa <= 0.0 {
                    Bgra::TRANSPARENT
                } else {
                    let a = twa / tw;
                    Bgra::new(
                        cb((tb / twa + 0.5) as i32),
                        cb((tg / twa + 0.5) as i32),
                        cb((tr / twa + 0.5) as i32),
                        cb((a + 0.5) as i32),
                    )
                };
            }
        });
    }
}

// ------------------------------------------------------------ Motion blur

pub struct MotionBlur {
    points: Vec<(f32, f32)>,
}

impl MotionBlur {
    pub fn new(angle_degrees: f64, distance: i32, centered: bool) -> Self {
        let distance = distance.clamp(1, 200);
        let theta = (angle_degrees + 180.0) * 2.0 * std::f64::consts::PI / 360.0;
        let mut end = ((distance as f64 * theta.cos()) as f32, (-distance as f64 * theta.sin()) as f32);
        let mut start = (0f32, 0f32);
        if centered {
            start = (-end.0 / 2.0, -end.1 / 2.0);
            end = (end.0 / 2.0, end.1 / 2.0);
        }
        let n = ((1 + distance) * 3 / 2) as usize;
        let points = if n == 1 {
            vec![(0.0, 0.0)]
        } else {
            (0..n)
                .map(|i| {
                    let frac = i as f32 / (n - 1) as f32;
                    (start.0 + (end.0 - start.0) * frac, start.1 + (end.1 - start.1) * frac)
                })
                .collect()
        };
        MotionBlur { points }
    }
}

impl Effect for MotionBlur {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let (wm, hm) = ((src.width() - 1) as f32, (src.height() - 1) as f32);
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (mut b, mut g, mut r, mut a, mut count) = (0i64, 0i64, 0i64, 0i64, 0i64);
                for &(px, py) in &self.points {
                    let fx = x as f32 + px;
                    let fy = y as f32 + py;
                    if fx < 0.0 || fy < 0.0 || fx > wm || fy > hm {
                        continue;
                    }
                    let c = bilinear_sample(src, fx, fy);
                    b += c.b as i64;
                    g += c.g as i64;
                    r += c.r as i64;
                    a += c.a as i64;
                    count += 1;
                }
                drow[x as usize] = if count == 0 {
                    src.get(x, y)
                } else {
                    let h = count / 2;
                    Bgra::new(((b + h) / count) as u8, ((g + h) / count) as u8, ((r + h) / count) as u8, ((a + h) / count) as u8)
                };
            }
        });
    }
}

// ------------------------------------------------------------ Pixelate

pub struct Pixelate(pub i32);

impl Effect for Pixelate {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let cell = self.0.clamp(1, 100);
        rows(dst, roi, |y, drow| {
            let top = y - y % cell;
            let bottom = (src.height() - 1).min(top + cell - 1);
            for x in roi.x..roi.right() {
                let left = x - x % cell;
                let right = (src.width() - 1).min(left + cell - 1);
                let (tl, tr, bl, br) = (src.get(left, top), src.get(right, top), src.get(left, bottom), src.get(right, bottom));
                let avg = |a: u8, b: u8, c: u8, d: u8| ((a as i32 + b as i32 + c as i32 + d as i32 + 2) / 4) as u8;
                drow[x as usize] = Bgra::new(
                    avg(tl.b, tr.b, bl.b, br.b),
                    avg(tl.g, tr.g, bl.g, br.g),
                    avg(tl.r, tr.r, bl.r, br.r),
                    avg(tl.a, tr.a, bl.a, br.a),
                );
            }
        });
    }
}

// ------------------------------------------------------------ Local histogram

/// Sliding-window per-channel histograms (B, G, R, A).
pub struct Hist {
    pub b: [i32; 256],
    pub g: [i32; 256],
    pub r: [i32; 256],
    pub a: [i32; 256],
}

impl Hist {
    fn walk(h: &[i32; 256], min_count: i32) -> u8 {
        let mut v = 0usize;
        while v < 255 && h[v] == 0 {
            v += 1;
        }
        let mut count = 0;
        while v < 255 && count < min_count {
            count += h[v];
            v += 1;
        }
        v as u8
    }

    pub fn percentile(&self, percentile: i32, area: i32) -> Bgra {
        let m = area * percentile / 100;
        Bgra::new(Self::walk(&self.b, m), Self::walk(&self.g, m), Self::walk(&self.r, m), Self::walk(&self.a, m))
    }
}

fn local_histogram(src: &Surface, dst: &mut Surface, roi: PixelRect, radius: i32, apply: impl Fn(Bgra, i32, &Hist) -> Bgra + Sync + Send) {
    let r = radius.clamp(1, 200);
    rows(dst, roi, |y, drow| {
        let mut h = Hist { b: [0; 256], g: [0; 256], r: [0; 256], a: [0; 256] };
        let mut area = 0i32;
        let y_top = (y - r).max(0);
        let y_bot = (src.height() - 1).min(y + r);
        let x_left = (roi.x - r).max(0);
        let x_right = (src.width() - 1).min(roi.x + r);
        let add = |h: &mut Hist, c: Bgra, d: i32| {
            h.b[c.b as usize] += d;
            h.g[c.g as usize] += d;
            h.r[c.r as usize] += d;
            h.a[c.a as usize] += d;
        };
        for yy in y_top..=y_bot {
            for &c in &src.row(yy)[x_left as usize..=x_right as usize] {
                add(&mut h, c, 1);
                area += 1;
            }
        }
        let srow = src.row(y);
        drow[roi.x as usize] = apply(srow[roi.x as usize], area, &h);
        for x in roi.x + 1..roi.right() {
            let remove_x = x - 1 - r;
            if remove_x >= 0 {
                for yy in y_top..=y_bot {
                    add(&mut h, src.get(remove_x, yy), -1);
                    area -= 1;
                }
            }
            let add_x = x + r;
            if add_x < src.width() {
                for yy in y_top..=y_bot {
                    add(&mut h, src.get(add_x, yy), 1);
                    area += 1;
                }
            }
            drow[x as usize] = apply(srow[x as usize], area, &h);
        }
    });
}

pub struct Median {
    pub radius: i32,
    pub percentile: i32,
}

impl Effect for Median {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let p = self.percentile;
        local_histogram(src, dst, roi, self.radius, |s, area, h| h.percentile(p, area).with_alpha(s.a));
    }
}

pub struct Sharpen(pub i32);

impl Effect for Sharpen {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        local_histogram(src, dst, roi, self.0, |s, area, h| lerp(s, h.percentile(50, area), -0.5));
    }
}

pub struct ReduceNoise {
    radius: i32,
    strength: f64,
}

impl ReduceNoise {
    pub fn new(radius: i32, strength: f64) -> Self {
        ReduceNoise { radius, strength: -0.2 * strength.clamp(0.0, 1.0) }
    }
}

impl Effect for ReduceNoise {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let st = self.strength;
        local_histogram(src, dst, roi, self.radius, |s, area, h| {
            let below = |hist: &[i32; 256], v: u8| hist[..v as usize].iter().sum::<i32>();
            let (rc, gc, bc) = (below(&h.r, s.r), below(&h.g, s.g), below(&h.b, s.b));
            let normalized = Bgra::bgr((bc * 255 / area) as u8, (gc * 255 / area) as u8, (rc * 255 / area) as u8);
            let l = st * (1.0 - 0.75 * s.intensity());
            lerp(s, normalized, l as f32)
        });
    }
}

// ------------------------------------------------------------ Unfocus

pub struct Unfocus(pub i32);

impl Effect for Unfocus {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let r = self.0.clamp(1, 200);
        rows(dst, roi, |y, drow| {
            let top = (y - r).max(0);
            let bottom = src.height().min(y + r + 1);
            let (mut sb, mut sg, mut sr, mut sa, mut area) = (0i64, 0i64, 0i64, 0i64, 0i32);
            let acc = |c: Bgra, d: i64, sb: &mut i64, sg: &mut i64, sr: &mut i64, sa: &mut i64, area: &mut i32| {
                let a = c.a as i64;
                *sb += d * c.b as i64 * a;
                *sg += d * c.g as i64 * a;
                *sr += d * c.r as i64 * a;
                *sa += d * a;
                *area += d as i32;
            };
            let (mut win_l, mut win_r) = (-1, -1);
            for x in roi.x..roi.right() {
                let left = (x - r).max(0);
                let right = src.width().min(x + r + 1);
                if win_l < 0 {
                    sb = 0;
                    sg = 0;
                    sr = 0;
                    sa = 0;
                    area = 0;
                    for j in top..bottom {
                        for &c in &src.row(j)[left as usize..right as usize] {
                            acc(c, 1, &mut sb, &mut sg, &mut sr, &mut sa, &mut area);
                        }
                    }
                } else {
                    for i in win_l..left {
                        for j in top..bottom {
                            acc(src.get(i, j), -1, &mut sb, &mut sg, &mut sr, &mut sa, &mut area);
                        }
                    }
                    for i in win_r..right {
                        for j in top..bottom {
                            acc(src.get(i, j), 1, &mut sb, &mut sg, &mut sr, &mut sa, &mut area);
                        }
                    }
                }
                win_l = left;
                win_r = right;
                let div = area as i64 * 255;
                drow[x as usize] = Bgra::new(
                    cb((sb / div) as i32),
                    cb((sg / div) as i32),
                    cb((sr / div) as i32),
                    cb((sa / area as i64) as i32),
                );
            }
        });
    }
}

// ------------------------------------------------------------ Fragment

pub struct Fragment {
    offsets: Vec<(i32, i32)>,
}

impl Fragment {
    pub fn new(fragments: i32, distance: i32, rotation_degrees: f64) -> Self {
        let fragments = fragments.clamp(2, 50);
        let distance = distance.clamp(0, 100) as f64;
        let step = 2.0 * std::f64::consts::PI / fragments as f64;
        let rot = (rotation_degrees - 90.0) * std::f64::consts::PI / 180.0;
        let offsets = (0..fragments)
            .map(|i| {
                let cur = rot + step * i as f64;
                ((distance * -cur.sin()).round() as i32, (distance * -cur.cos()).round() as i32)
            })
            .collect();
        Fragment { offsets }
    }
}

impl Effect for Fragment {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        rows(dst, roi, |y, drow| {
            let mut samples = Vec::with_capacity(self.offsets.len());
            for x in roi.x..roi.right() {
                samples.clear();
                for &(ox, oy) in &self.offsets {
                    let (sx, sy) = (x - ox, y - oy);
                    if src.in_bounds(sx, sy) {
                        samples.push(src.get(sx, sy));
                    }
                }
                drow[x as usize] = Bgra::blend_average(&samples);
            }
        });
    }
}

// ------------------------------------------------------------ Radial blur

pub struct RadialBlur {
    pub angle_degrees: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub quality: i32,
}

#[inline(always)]
fn rotate_fixed(fx: &mut i32, fy: &mut i32, fr: i32) {
    let (cx, cy) = (*fx, *fy);
    let frr = fr.wrapping_mul(fr) >> 11;
    *fx = cx.wrapping_sub((cy >> 8).wrapping_mul(fr) >> 8).wrapping_sub((cx >> 14).wrapping_mul(frr) >> 8);
    *fy = cy.wrapping_add((cx >> 8).wrapping_mul(fr) >> 8).wrapping_sub((cy >> 14).wrapping_mul(frr) >> 8);
}

impl Effect for RadialBlur {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let q = self.quality.clamp(1, 5);
        let (w, h) = (src.width(), src.height());
        let fcx = (w << 15).wrapping_add((self.offset_x * (w << 15) as f64) as i32);
        let fcy = (h << 15).wrapping_add((self.offset_y * (h << 15) as f64) as i32);
        let n = q * q * (30 + q * q);
        let fr = (self.angle_degrees * std::f64::consts::PI * 65536.0 / 181.0) as i32;
        rows(dst, roi, |y, drow| {
            let srow = src.row(y);
            for x in roi.x..roi.right() {
                let fx = (x << 16).wrapping_sub(fcx);
                let fy = (y << 16).wrapping_sub(fcy);
                let fsr = fr / n;
                let c0 = srow[x as usize];
                let (mut sr, mut sg, mut sb, mut sa) = (
                    c0.r as i64 * c0.a as i64,
                    c0.g as i64 * c0.a as i64,
                    c0.b as i64 * c0.a as i64,
                    c0.a as i64,
                );
                let mut sc = 1i64;
                let (mut ox1, mut ox2, mut oy1, mut oy2) = (fx, fx, fy, fy);
                let take = |u: i32, v: i32, sr: &mut i64, sg: &mut i64, sb: &mut i64, sa: &mut i64, sc: &mut i64| {
                    if u > 0 && v > 0 && u < w && v < h {
                        let c = src.get(u, v);
                        let a = c.a as i64;
                        *sr += c.r as i64 * a;
                        *sg += c.g as i64 * a;
                        *sb += c.b as i64 * a;
                        *sa += a;
                        *sc += 1;
                    }
                };
                for _ in 0..n {
                    rotate_fixed(&mut ox1, &mut oy1, fsr);
                    rotate_fixed(&mut ox2, &mut oy2, -fsr);
                    let u1 = ox1.wrapping_add(fcx).wrapping_add(32768) >> 16;
                    let v1 = oy1.wrapping_add(fcy).wrapping_add(32768) >> 16;
                    take(u1, v1, &mut sr, &mut sg, &mut sb, &mut sa, &mut sc);
                    let u2 = ox2.wrapping_add(fcx).wrapping_add(32768) >> 16;
                    let v2 = oy2.wrapping_add(fcy).wrapping_add(32768) >> 16;
                    take(u2, v2, &mut sr, &mut sg, &mut sb, &mut sa, &mut sc);
                }
                drow[x as usize] = if sa > 0 {
                    Bgra::new(cb((sb / sa) as i32), cb((sg / sa) as i32), cb((sr / sa) as i32), cb((sa / sc) as i32))
                } else {
                    Bgra::TRANSPARENT
                };
            }
        });
    }
}

// ------------------------------------------------------------ Frosted glass

pub struct FrostedGlass {
    pub min_radius: f64,
    pub max_radius: f64,
    pub sample_count: i32,
    pub seed: i32,
}

impl Effect for FrostedGlass {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let (wf, hf) = (src.width() as f64, src.height() as f64);
        let min_r = self.min_radius.min(src.width().min(src.height()) as f64 / 2.0);
        let delta = (self.max_radius - min_r).max(0.0);
        let count = self.sample_count.clamp(1, 8) as usize;
        rows(dst, roi, |y, drow| {
            let mut rand = DotNetRandom::new(self.seed.wrapping_mul(1000003).wrapping_add(y));
            let mut samples = [Bgra::TRANSPARENT; 8];
            for x in roi.x..roi.right() {
                for s in samples.iter_mut().take(count) {
                    let (sx, sy) = loop {
                        let angle = rand.next_double() * std::f64::consts::PI * 2.0;
                        let distance = rand.next_double() * delta + min_r;
                        let sx = x as f64 + angle.cos() * distance;
                        let sy = y as f64 + angle.sin() * distance;
                        if sx >= 0.0 && sx <= wf && sy >= 0.0 && sy <= hf {
                            break (sx, sy);
                        }
                    };
                    *s = bilinear_clamped(src, sx as f32, sy as f32);
                }
                drow[x as usize] = Bgra::blend_average(&samples[..count]);
            }
        });
    }
}

// ------------------------------------------------------------ Add noise

const NOISE_TABLE: usize = 16384;

fn noise_lookup() -> &'static [i32; NOISE_TABLE] {
    static TABLE: std::sync::OnceLock<Box<[i32; NOISE_TABLE]>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let curve = |x: f64, scale: f64| scale * (-x * x / 2.0).exp();
        let ts = NOISE_TABLE as i32;
        let (mut l, mut r, mut scale) = (5.0f64, 10.0f64, 50.0f64);
        while r - l > 0.0000001 {
            let mut sum = 0.0;
            scale = (l + r) * 0.5;
            for i in 0..ts {
                sum += curve(16.0 * (i - ts / 2) as f64 / ts as f64, scale);
                if sum > 1000000.0 {
                    break;
                }
            }
            if sum > ts as f64 {
                r = scale;
            } else if sum < ts as f64 {
                l = scale;
            } else {
                break;
            }
        }
        let mut lookup = Box::new([0i32; NOISE_TABLE]);
        let mut sum = 0.0;
        let mut rounded = 0i32;
        for i in 0..ts {
            sum += curve(16.0 * (i - ts / 2) as f64 / ts as f64, scale);
            let last = rounded;
            rounded = sum as i32;
            let mut j = last;
            while j < rounded && j < ts {
                lookup[j as usize] = (i - ts / 2) * 65536 / ts;
                j += 1;
            }
        }
        lookup
    })
}

pub struct AddNoise {
    pub intensity: i32,
    pub saturation: i32,
    pub coverage_percent: f64,
    pub seed: i32,
}

impl Effect for AddNoise {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let lookup = noise_lookup();
        let dev = self.intensity * self.intensity / 4;
        let sat = self.saturation * 4096 / 100;
        let coverage = 0.01 * self.coverage_percent;
        rows(dst, roi, |y, drow| {
            let mut rand = DotNetRandom::new(self.seed.wrapping_mul(1000003).wrapping_add(y));
            let srow = src.row(y);
            for x in roi.x..roi.right() {
                let c = srow[x as usize];
                if rand.next_double() > coverage {
                    drow[x as usize] = c;
                    continue;
                }
                let mut r = lookup[rand.next_max(NOISE_TABLE as i32) as usize];
                let mut g = lookup[rand.next_max(NOISE_TABLE as i32) as usize];
                let mut b = lookup[rand.next_max(NOISE_TABLE as i32) as usize];
                let i = (4899 * r + 9618 * g + 1867 * b) >> 14;
                r = i + (((r - i) * sat) >> 12);
                g = i + (((g - i) * sat) >> 12);
                b = i + (((b - i) * sat) >> 12);
                let n = |ch: u8, v: i32| cb(ch as i32 + ((v * dev + 32768) >> 16));
                drow[x as usize] = Bgra::new(n(c.b, b), n(c.g, g), n(c.r, r), c.a);
            }
        });
    }
}
