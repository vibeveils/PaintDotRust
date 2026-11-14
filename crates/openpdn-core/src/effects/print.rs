//! Print effects: ordered dithering and halftone screens.

use super::dotnet::introsort;
use super::math::{hash01, luma, mix, to_byte};
use super::{rows, Effect};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::sampling::bilinear_clamped;
use crate::surface::Surface;
use std::f32::consts::PI;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DitherPattern {
    Bayer2,
    Bayer4,
    Bayer8,
    Bayer16,
    Spiral,
    DualSpiral,
    BlueNoise,
    WhiteNoise,
}

/// Square threshold matrix, row-major, values in (0, 1).
#[derive(Clone)]
pub struct Matrix {
    pub n: usize,
    pub v: Vec<f32>,
}

fn normalize(n: usize, m: &[f32]) -> Matrix {
    let count = (n * n) as f32;
    Matrix { n, v: m.iter().map(|&x| (x + 0.5) / count).collect() }
}

pub fn bayer(size: usize) -> Matrix {
    let mut m = vec![0f32];
    let mut n = 1;
    while n < size {
        let mut next = vec![0f32; 4 * n * n];
        let nn = 2 * n;
        for y in 0..n {
            for x in 0..n {
                let v = m[y * n + x] * 4.0;
                next[y * nn + x] = v;
                next[y * nn + x + n] = v + 2.0;
                next[(y + n) * nn + x] = v + 3.0;
                next[(y + n) * nn + x + n] = v + 1.0;
            }
        }
        m = next;
        n *= 2;
    }
    normalize(n, &m)
}

pub fn spiral() -> Matrix {
    const N: i32 = 8;
    let mut m = vec![0f32; 64];
    let (mut x, mut y, mut dx, mut dy, mut step, mut rank) = (3i32, 3i32, 1i32, 0i32, 1, 0);
    m[(y * N + x) as usize] = rank as f32;
    rank += 1;
    while rank < N * N {
        for _ in 0..2 {
            let mut i = 0;
            while i < step && rank < N * N {
                x += dx;
                y += dy;
                if (x as u32) < N as u32 && (y as u32) < N as u32 {
                    m[(y * N + x) as usize] = rank as f32;
                    rank += 1;
                }
                i += 1;
            }
            (dx, dy) = (-dy, dx);
        }
        step += 1;
    }
    normalize(8, &m)
}

fn torus_dist(x: f32, y: f32, cx: f32, cy: f32, n: i32) -> f32 {
    let dx = (x - cx).abs();
    let dx = dx.min(n as f32 - dx);
    let dy = (y - cy).abs();
    let dy = dy.min(n as f32 - dy);
    (dx * dx + dy * dy).sqrt()
}

pub fn dual_spiral() -> Matrix {
    let mut keys: Vec<(f32, usize, usize)> = Vec::with_capacity(64);
    for y in 0..8 {
        for x in 0..8 {
            let da = torus_dist(x as f32 + 0.5, y as f32 + 0.5, 2.0, 2.0, 8);
            let db = torus_dist(x as f32 + 0.5, y as f32 + 0.5, 6.0, 6.0, 8);
            keys.push(if da <= db { (da, x, y) } else { (100.0 - db, x, y) });
        }
    }
    // Ties exist, so this must be .NET's (unstable) sort to match.
    introsort(&mut keys, &|a, b| a.0.partial_cmp(&b.0).unwrap());
    let mut m = vec![0f32; 64];
    for (i, &(_, x, y)) in keys.iter().enumerate() {
        m[y * 8 + x] = i as f32;
    }
    normalize(8, &m)
}

/// Void-and-cluster blue noise (64×64, σ 1.5, seed 7), computed once.
pub fn blue_noise() -> &'static Matrix {
    static M: OnceLock<Matrix> = OnceLock::new();
    M.get_or_init(|| void_and_cluster(64, 1.5, 7))
}

pub fn void_and_cluster(n: usize, sigma: f32, seed: i32) -> Matrix {
    let size = n * n;
    let r = (sigma * 4.0).ceil() as i32;
    let kw = (2 * r + 1) as usize;
    let mut kernel = vec![0f32; kw * kw];
    for y in -r..=r {
        for x in -r..=r {
            kernel[(y + r) as usize * kw + (x + r) as usize] = (-((x * x + y * y) as f32) / (2.0 * sigma * sigma)).exp();
        }
    }
    let mut on = vec![false; size];
    let mut energy = vec![0f32; size];
    let ni = n as i32;
    let toggle = |on: &mut [bool], energy: &mut [f32], idx: usize, value: bool| {
        on[idx] = value;
        let sign = if value { 1f32 } else { -1f32 };
        let (cx, cy) = ((idx % n) as i32, (idx / n) as i32);
        for y in -r..=r {
            let yy = ((cy + y) % ni + ni) % ni;
            for x in -r..=r {
                let xx = ((cx + x) % ni + ni) % ni;
                energy[(yy * ni + xx) as usize] += sign * kernel[(y + r) as usize * kw + (x + r) as usize];
            }
        }
    };
    let tightest = |on: &[bool], energy: &[f32]| {
        let (mut best, mut bv) = (usize::MAX, f32::MIN);
        for i in 0..size {
            if on[i] && energy[i] > bv {
                bv = energy[i];
                best = i;
            }
        }
        best
    };
    let loosest = |on: &[bool], energy: &[f32]| {
        let (mut best, mut bv) = (usize::MAX, f32::MAX);
        for i in 0..size {
            if !on[i] && energy[i] < bv {
                bv = energy[i];
                best = i;
            }
        }
        best
    };
    let mut initial = size / 10;
    for i in 0..size {
        if initial == 0 {
            break;
        }
        if hash01((i % n) as i32, (i / n) as i32, seed) < 0.1 {
            toggle(&mut on, &mut energy, i, true);
            initial -= 1;
        }
    }
    loop {
        let t = tightest(&on, &energy);
        toggle(&mut on, &mut energy, t, false);
        let v = loosest(&on, &energy);
        if v == t {
            toggle(&mut on, &mut energy, t, true);
            break;
        }
        toggle(&mut on, &mut energy, v, true);
    }
    let mut rank = vec![0usize; size];
    let (proto, proto_energy) = (on.clone(), energy.clone());
    let ones = on.iter().filter(|&&b| b).count();
    for k in (0..ones).rev() {
        let t = tightest(&on, &energy);
        toggle(&mut on, &mut energy, t, false);
        rank[t] = k;
    }
    on.copy_from_slice(&proto);
    energy.copy_from_slice(&proto_energy);
    for k in ones..size {
        let v = loosest(&on, &energy);
        toggle(&mut on, &mut energy, v, true);
        rank[v] = k;
    }
    let m: Vec<f32> = rank.iter().map(|&k| k as f32).collect();
    normalize(n, &m)
}

pub struct OrderedDither {
    matrix: Option<Matrix>,
    levels: i32,
    grayscale: bool,
    seed: i32,
}

impl OrderedDither {
    pub fn new(pattern: DitherPattern, levels: i32, grayscale: bool, seed: i32) -> Self {
        let matrix = match pattern {
            DitherPattern::Bayer2 => Some(bayer(2)),
            DitherPattern::Bayer4 => Some(bayer(4)),
            DitherPattern::Bayer8 => Some(bayer(8)),
            DitherPattern::Bayer16 => Some(bayer(16)),
            DitherPattern::Spiral => Some(spiral()),
            DitherPattern::DualSpiral => Some(dual_spiral()),
            DitherPattern::BlueNoise => Some(blue_noise().clone()),
            DitherPattern::WhiteNoise => None,
        };
        OrderedDither { matrix, levels, grayscale, seed }
    }

    pub fn threshold(&self, x: i32, y: i32) -> f32 {
        match &self.matrix {
            None => hash01(x, y, self.seed),
            Some(m) => m.v[(y as usize % m.n) * m.n + x as usize % m.n],
        }
    }
}

impl Effect for OrderedDither {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let steps = (self.levels.clamp(2, 256) - 1) as f32;
        let quant = |v: u8, t: f32| {
            let q = (v as f32 / 255.0 * steps + t).floor();
            ((q / steps * 255.0 + 0.5) as i32).clamp(0, 255) as u8
        };
        rows(dst, roi, |y, drow| {
            let s = src.row(y);
            for x in roi.x..roi.right() {
                let c = s[x as usize];
                let t = self.threshold(x, y);
                drow[x as usize] = if self.grayscale {
                    let g = quant(c.intensity_byte(), t);
                    Bgra::new(g, g, g, c.a)
                } else {
                    Bgra::new(quant(c.b, t), quant(c.g, t), quant(c.r, t), c.a)
                };
            }
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HalftoneMode {
    Cmyk,
    Monochrome,
}

pub struct Halftone {
    pub mode: HalftoneMode,
    pub cell_size: i32,
    pub angle_degrees: i32,
    pub ink: Bgra,
    pub paper: Bgra,
}

struct Screen {
    cos: f32,
    sin: f32,
    cell: f32,
    u0: i32,
    v0: i32,
    cols: i32,
    rows: i32,
    amount: Vec<f32>,
}

impl Screen {
    fn coverage(&self, px: f32, py: f32) -> f32 {
        let u = (px * self.cos + py * self.sin) / self.cell;
        let v = (-px * self.sin + py * self.cos) / self.cell;
        let iu = u.floor() as i32;
        let iv = v.floor() as i32;
        let mut best = 0f32;
        for j in -1..=1 {
            let row = iv + j - self.v0;
            if (row as u32) >= self.rows as u32 {
                continue;
            }
            for i in -1..=1 {
                let col = iu + i - self.u0;
                if (col as u32) >= self.cols as u32 {
                    continue;
                }
                let amount = self.amount[(row * self.cols + col) as usize];
                if amount <= 0.0 {
                    continue;
                }
                let radius = self.cell * 0.7071 * amount.sqrt();
                let du = (u - ((iu + i) as f32 + 0.5)) * self.cell;
                let dv = (v - ((iv + j) as f32 + 0.5)) * self.cell;
                let cov = (radius + 0.5 - (du * du + dv * dv).sqrt()).clamp(0.0, 1.0);
                if cov > best {
                    best = cov;
                }
            }
        }
        best
    }
}

fn ink_amount(c: Bgra, channel: i32) -> f32 {
    if channel < 0 {
        return 1.0 - luma(c);
    }
    let (r, g, b) = (c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0);
    let k = 1.0 - r.max(g.max(b));
    if channel == 3 {
        return k;
    }
    if k >= 0.999 {
        return 0.0;
    }
    let comp = match channel {
        0 => r,
        1 => g,
        _ => b,
    };
    (1.0 - comp - k) / (1.0 - k)
}

impl Halftone {
    fn build(&self, src: &Surface, angle_deg: f32, channel: i32) -> Screen {
        let a = angle_deg * PI / 180.0;
        let (cos, sin, cell) = (a.cos(), a.sin(), 2f32.max(self.cell_size as f32));
        let (mut umin, mut umax, mut vmin, mut vmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        let (w, h) = (src.width() as f32, src.height() as f32);
        for (x, y) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
            let u = (x * cos + y * sin) / cell;
            let v = (-x * sin + y * cos) / cell;
            umin = umin.min(u);
            umax = umax.max(u);
            vmin = vmin.min(v);
            vmax = vmax.max(v);
        }
        let u0 = umin.floor() as i32 - 1;
        let v0 = vmin.floor() as i32 - 1;
        let cols = umax.ceil() as i32 + 2 - u0;
        let rows_n = vmax.ceil() as i32 + 2 - v0;
        let mut amount = vec![0f32; (cols * rows_n) as usize];
        crate::par::for_rows(&mut amount, cols as usize, |r, row| {
            for (c, a) in row.iter_mut().enumerate() {
                let cu = (u0 + c as i32) as f32 + 0.5;
                let cv = (v0 + r as i32) as f32 + 0.5;
                let cx = (cu * cos - cv * sin) * cell;
                let cy = (cu * sin + cv * cos) * cell;
                *a = ink_amount(bilinear_clamped(src, cx - 0.5, cy - 0.5), channel);
            }
        });
        Screen { cos, sin, cell, u0, v0, cols, rows: rows_n, amount }
    }
}

impl Effect for Halftone {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        if self.mode == HalftoneMode::Monochrome {
            let sc = self.build(src, self.angle_degrees as f32, -1);
            rows(dst, roi, |y, drow| {
                let s = src.row(y);
                for x in roi.x..roi.right() {
                    let c = mix(self.paper, self.ink, sc.coverage(x as f32 + 0.5, y as f32 + 0.5));
                    drow[x as usize] = c.with_alpha((c.a as i32 * s[x as usize].a as i32 / 255) as u8);
                }
            });
            return;
        }
        let angles = [15f32, 75.0, 0.0, 45.0];
        let screens: Vec<Screen> = (0..4).map(|i| self.build(src, angles[i], i as i32)).collect();
        rows(dst, roi, |y, drow| {
            let s = src.row(y);
            for x in roi.x..roi.right() {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let cc = screens[0].coverage(px, py);
                let mm = screens[1].coverage(px, py);
                let yy = screens[2].coverage(px, py);
                let kk = screens[3].coverage(px, py);
                drow[x as usize] = Bgra::new(
                    to_byte((1.0 - yy) * (1.0 - kk)),
                    to_byte((1.0 - mm) * (1.0 - kk)),
                    to_byte((1.0 - cc) * (1.0 - kk)),
                    s[x as usize].a,
                );
            }
        });
    }
}
