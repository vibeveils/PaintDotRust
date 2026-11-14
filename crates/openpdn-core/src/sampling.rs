//! Sub-pixel sampling helpers shared by transforms and effects (C# `Effects.Sampling`).

use crate::color::Bgra;
use crate::surface::Surface;

/// Bilinear sample at (fx, fy) ≥ 0, unweighted by alpha (C# `MotionBlurEffect.BilinearSample`).
#[inline]
pub fn bilinear_sample(s: &Surface, fx: f32, fy: f32) -> Bgra {
    let (w, h) = (s.width(), s.height());
    // Callers pass coordinates inside [0, w-1]×[0, h-1]; clamping the integer
    // cell is a no-op for those and keeps every index in bounds otherwise.
    let x0 = (fx as i32).clamp(0, w - 1);
    let y0 = (fy as i32).clamp(0, h - 1);
    let x1 = (w - 1).min(x0 + 1);
    let y1 = (h - 1).min(y0 + 1);
    let tx = fx - (fx as i32) as f32;
    let ty = fy - (fy as i32) as f32;
    let px = s.pixels();
    let wu = w as usize;
    let (r0, r1) = (y0 as usize * wu, y1 as usize * wu);
    let (c00, c10, c01, c11) = (px[r0 + x0 as usize], px[r0 + x1 as usize], px[r1 + x0 as usize], px[r1 + x1 as usize]);
    let w00 = (1.0 - tx) * (1.0 - ty);
    let w10 = tx * (1.0 - ty);
    let w01 = (1.0 - tx) * ty;
    let w11 = tx * ty;
    let ch = |a: u8, b: u8, c: u8, d: u8| {
        (a as f32 * w00 + b as f32 * w10 + c as f32 * w01 + d as f32 * w11 + 0.5) as u8
    };
    Bgra::new(
        ch(c00.b, c10.b, c01.b, c11.b),
        ch(c00.g, c10.g, c01.g, c11.g),
        ch(c00.r, c10.r, c01.r, c11.r),
        ch(c00.a, c10.a, c01.a, c11.a),
    )
}

#[inline]
pub fn bilinear_clamped(s: &Surface, fx: f32, fy: f32) -> Bgra {
    let fx = fx.clamp(0.0, (s.width() - 1) as f32);
    let fy = fy.clamp(0.0, (s.height() - 1) as f32);
    bilinear_sample(s, fx, fy)
}

#[inline]
fn wrap(v: f32, max: i32) -> f32 {
    // C# `%` on floats is fmod (sign of the dividend), same as Rust's.
    let mut v = v % max as f32;
    if v < 0.0 {
        v += max as f32;
    }
    v
}

#[inline]
pub fn bilinear_wrapped(s: &Surface, fx: f32, fy: f32) -> Bgra {
    let fx = wrap(fx, s.width());
    let fy = wrap(fy, s.height());
    bilinear_sample(s, fx.min((s.width() - 1) as f32), fy.min((s.height() - 1) as f32))
}

pub fn reflect_coord(mut value: f32, max: i32) -> f32 {
    let mut reflection = false;
    let m = max as f32;
    while value < 0.0 {
        value += m;
        reflection = !reflection;
    }
    while value > m {
        value -= m;
        reflection = !reflection;
    }
    if reflection { m - value } else { value }
}

/// Rotated-grid supersampling offsets for `quality`² samples.
pub fn rgss_offsets(quality: i32) -> Vec<(f32, f32)> {
    let n = (quality * quality) as usize;
    if n == 1 {
        return vec![(0.0, 0.0)];
    }
    (0..n)
        .map(|i| {
            let y = (i as f64 + 1.0) / (n as f64 + 1.0);
            let mut x = y * quality as f64;
            x -= (x as i32) as f64;
            ((x - 0.5) as f32, (y - 0.5) as f32)
        })
        .collect()
}
