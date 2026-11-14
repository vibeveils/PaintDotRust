//! Shared effect helpers (C# `EffectMath`).

use crate::color::Bgra;
use crate::surface::Surface;

/// Integer hash of (x, y, seed) mapped to [0, 1).
#[inline]
pub fn hash01(x: i32, y: i32, seed: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8DA6B343) ^ (y as u32).wrapping_mul(0xD8163841) ^ (seed as u32).wrapping_mul(0xCB1AB31F);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A2D39);
    h ^= h >> 15;
    (h >> 8) as f32 * (1.0 / 16777216.0)
}

#[inline]
pub fn luma(c: Bgra) -> f32 {
    (0.2126f32 * c.r as f32 + 0.7152f32 * c.g as f32 + 0.0722f32 * c.b as f32) / 255.0
}

#[inline]
pub fn smooth_step(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
pub fn to_byte(v01: f32) -> u8 {
    ((v01 * 255.0 + 0.5) as i32).clamp(0, 255) as u8
}

#[inline]
pub fn mix(a: Bgra, b: Bgra, t: f32) -> Bgra {
    let u = 1.0 - t;
    let m = |x: u8, y: u8| (x as f32 * u + y as f32 * t + 0.5) as u8;
    Bgra::new(m(a.b, b.b), m(a.g, b.g), m(a.r, b.r), m(a.a, b.a))
}

/// Alpha-weighted average of a clipped rectangle.
pub fn average_rect(s: &Surface, x0: i32, y0: i32, x1: i32, y1: i32) -> Bgra {
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(s.width()), y1.min(s.height()));
    let (mut sb, mut sg, mut sr, mut sa, mut n) = (0i64, 0i64, 0i64, 0i64, 0i64);
    for y in y0..y1 {
        for c in &s.row(y)[x0 as usize..x1.max(x0) as usize] {
            let a = c.a as i64;
            sb += c.b as i64 * a;
            sg += c.g as i64 * a;
            sr += c.r as i64 * a;
            sa += a;
            n += 1;
        }
    }
    if n == 0 || sa == 0 {
        return Bgra::TRANSPARENT;
    }
    Bgra::new((sb / sa) as u8, (sg / sa) as u8, (sr / sa) as u8, (sa / n) as u8)
}

/// Separable box blur with edge clamping on a float plane.
pub fn box_blur(src: &[f32], w: usize, h: usize, radius: i32) -> Vec<f32> {
    if radius <= 0 {
        return src.to_vec();
    }
    let r = radius as isize;
    let (wi, hi) = (w as isize, h as isize);
    let norm = 1.0f32 / (2 * radius + 1) as f32;
    let mut tmp = vec![0f32; src.len()];
    crate::par::for_rows(&mut tmp, w, |y, trow| {
        let o = y * w;
        let s = &src[o..o + w];
        let mut acc = 0f32;
        for i in -r..=r {
            acc += s[i.clamp(0, wi - 1) as usize];
        }
        for x in 0..wi {
            trow[x as usize] = acc * norm;
            acc += s[(wi - 1).min(x + r + 1) as usize] - s[0.max(x - r) as usize];
        }
    });
    // Vertical pass: each column's running sum, advanced row by row for all columns at once.
    let mut dst = vec![0f32; src.len()];
    let mut acc = vec![0f32; w];
    for i in -r..=r {
        let row = &tmp[i.clamp(0, hi - 1) as usize * w..][..w];
        for x in 0..w {
            acc[x] += row[x];
        }
    }
    for y in 0..hi {
        let d = &mut dst[y as usize * w..][..w];
        let add = &tmp[(hi - 1).min(y + r + 1) as usize * w..][..w];
        let rem = &tmp[0.max(y - r) as usize * w..][..w];
        for x in 0..w {
            d[x] = acc[x] * norm;
            acc[x] += add[x] - rem[x];
        }
    }
    dst
}

pub fn luma_plane(s: &Surface) -> Vec<f32> {
    let w = s.width() as usize;
    let mut plane = vec![0f32; s.pixels().len()];
    crate::par::for_rows2(&mut plane, s.pixels(), w, |_, d, r| {
        for (p, &c) in d.iter_mut().zip(r) {
            *p = luma(c);
        }
    });
    plane
}
