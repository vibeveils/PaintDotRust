//! Generated content: clouds and fractals.

use super::{cb, rows, Effect};
use crate::blend::{blend, BlendMode};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::surface::Surface;
use std::f64::consts::PI;

const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240, 21, 10, 23,
    190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20,
    125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230, 220,
    105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200,
    196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126,
    255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246,
    97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181,
    199, 106, 157, 184, 84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

#[inline(always)]
fn p(i: usize) -> usize {
    PERM[i & 255] as usize
}

#[inline]
fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn grad(hash: usize, x: f64, y: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        0.0
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

#[inline]
fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Perlin noise with the C# 512-entry permutation lookup (indices wrap mod 256).
fn noise(ix: u8, iy: u8, x: f64, y: f64, seed: u8) -> f64 {
    let u = fade(x);
    let v = fade(y);
    let a = p(ix as usize + seed as usize) + iy as usize;
    let aa = p(a);
    let ab = p(a + 1);
    let b = p(ix as usize + 1 + seed as usize) + iy as usize;
    let ba = p(b);
    let bb = p(b + 1);
    let e1 = lerp(grad(p(aa), x, y), grad(p(ba), x - 1.0, y), u);
    let e2 = lerp(grad(p(ab), x, y - 1.0), grad(p(bb), x - 1.0, y - 1.0), u);
    lerp(e1, e2, v)
}

pub struct Clouds {
    pub scale: i32,
    pub power: f64,
    pub seed: i32,
    pub from: Bgra,
    pub to: Bgra,
    pub blend_mode: BlendMode,
}

impl Effect for Clouds {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let (w, h) = (dst.width(), dst.height());
        let s = self.scale.clamp(2, 1000);
        let lb = |a: u8, b: u8, t: f64| (a as f64 + (b as i32 - a as i32) as f64 * t).clamp(0.0, 255.0) as u8;
        rows(dst, roi, |y, drow| {
            let srow = src.row(y);
            let dy = 2 * y - h;
            for x in roi.x..roi.right() {
                let dx = 2 * x - w;
                let (mut val, mut mult, mut div) = (0.0f64, 1.0f64, s);
                let mut i = 0;
                while i < 12 && mult > 0.03 && div > 0 {
                    let mut dxr = 65536.0 + dx as f64 / div as f64;
                    let mut dyr = 65536.0 + dy as f64 / div as f64;
                    let dxd = dxr as i32;
                    let dyd = dyr as i32;
                    dxr -= dxd as f64;
                    dyr -= dyd as f64;
                    val += noise(dxd as u8, dyd as u8, dxr, dyr, (self.seed ^ i) as u8) * mult;
                    div /= 2;
                    mult *= self.power;
                    i += 1;
                }
                let t = (val + 1.0) / 2.0;
                let (f, to) = (self.from, self.to);
                let cloud = Bgra::new(lb(f.b, to.b, t), lb(f.g, to.g, t), lb(f.r, to.r, t), lb(f.a, to.a, t));
                drow[x as usize] = blend(cloud, srow[x as usize], self.blend_mode, 255);
            }
        });
    }
}

pub struct JuliaFractal {
    pub factor: f64,
    pub zoom: f64,
    pub angle_degrees: f64,
    pub quality: i32,
}

impl Effect for JuliaFractal {
    fn render(&self, _src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let log10000 = 10000f64.ln();
        let julia = |mut x: f64, mut y: f64, r: f64, i: f64| {
            let mut c = 0.0f64;
            while c < 256.0 && x * x + y * y < 10000.0 {
                let t = x;
                x = x * x - y * y + r;
                y = 2.0 * t * y + i;
                c += 1.0;
            }
            c - (2.0 - 2.0 * log10000 / (x * x + y * y).ln())
        };
        let (w, h) = (dst.width(), dst.height());
        let inv_h = 1.0 / h as f64;
        let inv_zoom = 1.0 / self.zoom.max(0.01);
        let q = self.quality.clamp(1, 5);
        let inv_q = 1.0 / q as f64;
        let aspect = h as f64 / w as f64;
        let count = q * q + 1;
        let inv_count = 1.0 / count as f64;
        let theta0 = self.angle_degrees * PI * 2.0 / 360.0;
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (mut r, mut g, mut b, mut a) = (0i32, 0i32, 0i32, 0i32);
                let mut i = 0.0f64;
                while i < count as f64 {
                    let u = (2.0 * x as f64 - w as f64 + i * inv_count) * inv_h;
                    let v = (2.0 * y as f64 - h as f64 + i * inv_q % 1.0) * inv_h;
                    let radius = (u * u + v * v).sqrt();
                    let tp = v.atan2(u) + theta0;
                    let up = radius * tp.cos();
                    let vp = radius * tp.sin();
                    let jx = (up - vp * aspect) * inv_zoom;
                    let jy = (vp + up * aspect) * inv_zoom;
                    let c = self.factor * julia(jx, jy, 0.3125, 0.03);
                    b += cb((c - 768.0) as i32) as i32;
                    g += cb((c - 512.0) as i32) as i32;
                    r += cb((c - 256.0) as i32) as i32;
                    a += cb(c as i32) as i32;
                    i += 1.0;
                }
                drow[x as usize] = Bgra::new(cb(b / count), cb(g / count), cb(r / count), cb(a / count));
            }
        });
    }
}

pub struct MandelbrotFractal {
    pub factor: i32,
    pub zoom: f64,
    pub angle_degrees: f64,
    pub quality: i32,
    pub invert_colors: bool,
}

impl Effect for MandelbrotFractal {
    fn render(&self, _src: &Surface, dst: &mut Surface, roi: PixelRect) {
        const MAX: f64 = 100000.0;
        let inv_log_max = 1.0 / MAX.ln();
        let f = self.factor.clamp(1, 10);
        let mandel = |r: f64, i: f64| {
            let mut c = 0i32;
            let (mut x, mut y) = (0.0f64, 0.0f64);
            while c * f < 1024 && x * x + y * y < MAX {
                let t = x;
                x = x * x - y * y + r;
                y = 2.0 * t * y + i;
                c += 1;
            }
            c as f64 - (y * y + x * x).ln() * inv_log_max
        };
        let (w, h) = (dst.width(), dst.height());
        let inv_h = 1.0 / h as f64;
        let inv_zoom = 1.0 / (1.0 + 20.0 * self.zoom);
        let q = self.quality.clamp(1, 5);
        let inv_q = 1.0 / q as f64;
        let count = q * q + 1;
        let inv_count = 1.0 / count as f64;
        let theta0 = self.angle_degrees * 2.0 * PI / 360.0;
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (mut r, mut g, mut b, mut a) = (0i32, 0i32, 0i32, 0i32);
                let mut i = 0.0f64;
                while i < count as f64 {
                    let u = (2.0 * x as f64 - w as f64 + i * inv_count) * inv_h;
                    let v = (2.0 * y as f64 - h as f64 + i * inv_q % 1.0) * inv_h;
                    let radius = (u * u + v * v).sqrt();
                    let tp = v.atan2(u) + theta0;
                    let up = radius * tp.cos();
                    let vp = radius * tp.sin();
                    let m = mandel(up * inv_zoom - 0.7, vp * inv_zoom - 0.29);
                    let c = 64.0 + f as f64 * m;
                    r += cb((c - 768.0) as i32) as i32;
                    g += cb((c - 512.0) as i32) as i32;
                    b += cb((c - 256.0) as i32) as i32;
                    a += cb(c as i32) as i32;
                    i += 1.0;
                }
                let mut res = Bgra::new(cb(b / count), cb(g / count), cb(r / count), cb(a / count));
                if self.invert_colors {
                    res = Bgra::new(255 - res.b, 255 - res.g, 255 - res.r, res.a);
                }
                drow[x as usize] = res;
            }
        });
    }
}
