//! Stylize and photo effects.

use super::blur::GaussianBlur;
use super::{apply, cb, rows, Effect};
use crate::adjust::{apply_op, BrightnessContrast, Desaturate, InvertColors, PixelOp};
use crate::blend::{blend, BlendMode};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::surface::Surface;
use std::f64::consts::PI;

fn rotated_weights(angle_degrees: f64, center: f64) -> [[f64; 3]; 3] {
    let r = angle_degrees * 2.0 * PI / 360.0;
    let dr = PI / 4.0;
    [
        [(r + dr).cos(), (r + 2.0 * dr).cos(), (r + 3.0 * dr).cos()],
        [r.cos(), center, (r + 4.0 * dr).cos()],
        [(r - dr).cos(), (r - 2.0 * dr).cos(), (r - 3.0 * dr).cos()],
    ]
}

#[inline]
fn kernel_extent(src: &Surface, x: i32, y: i32) -> (usize, usize, usize, usize) {
    (
        if y == 0 { 1 } else { 0 },
        if y == src.height() - 1 { 2 } else { 3 },
        if x == 0 { 1 } else { 0 },
        if x == src.width() - 1 { 2 } else { 3 },
    )
}

pub struct Emboss(pub f64);

impl Effect for Emboss {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let wt = rotated_weights(self.0, 0.0);
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (fy0, fy1, fx0, fx1) = kernel_extent(src, x, y);
                let mut sum = 0.0f64;
                for fy in fy0..fy1 {
                    for fx in fx0..fx1 {
                        sum += wt[fy][fx] * src.get(x - 1 + fx as i32, y - 1 + fy as i32).intensity_byte() as f64;
                    }
                }
                let v = cb((sum as i32).wrapping_add(128));
                drow[x as usize] = Bgra::new(v, v, v, 255);
            }
        });
    }
}

/// Edge Detect (centre weight 0) and Relief (centre weight 1).
pub struct ColorDifference {
    weights: [[f64; 3]; 3],
}

impl ColorDifference {
    pub fn edge_detect(angle: f64) -> Self {
        ColorDifference { weights: rotated_weights(angle, 0.0) }
    }
    pub fn relief(angle: f64) -> Self {
        ColorDifference { weights: rotated_weights(angle, 1.0) }
    }
}

impl Effect for ColorDifference {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let wt = &self.weights;
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (fy0, fy1, fx0, fx1) = kernel_extent(src, x, y);
                let (mut rs, mut gs, mut bs) = (0.0f64, 0.0f64, 0.0f64);
                for fy in fy0..fy1 {
                    for fx in fx0..fx1 {
                        let w = wt[fy][fx];
                        let c = src.get(x - 1 + fx as i32, y - 1 + fy as i32);
                        rs += w * c.r as f64;
                        gs += w * c.g as f64;
                        bs += w * c.b as f64;
                    }
                }
                drow[x as usize] = Bgra::new(cb(bs as i32), cb(gs as i32), cb(rs as i32), 255);
            }
        });
    }
}

pub struct OilPainting {
    pub brush_size: i32,
    pub coarseness: i32,
}

impl Effect for OilPainting {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let size = self.brush_size.clamp(1, 8);
        let max_i = self.coarseness.clamp(3, 255) as u32;
        let lens = max_i as usize + 1;
        let fast_scale = |a: u8| {
            let r1 = a as u32 * max_i + 0x80;
            (((r1 >> 8) + r1) >> 8) as u8 as usize
        };
        rows(dst, roi, |y, drow| {
            let mut count = vec![0i32; lens];
            let mut avg = vec![[0u32; 4]; lens];
            let top = (y - size).max(0);
            let bottom = src.height().min(y + size + 1);
            for x in roi.x..roi.right() {
                count.fill(0);
                avg.fill([0; 4]);
                let left = (x - size).max(0);
                let right = src.width().min(x + size + 1);
                for j in top..bottom {
                    for &c in &src.row(j)[left as usize..right as usize] {
                        let i = fast_scale(c.intensity_byte());
                        count[i] += 1;
                        let a = &mut avg[i];
                        a[0] += c.r as u32;
                        a[1] += c.g as u32;
                        a[2] += c.b as u32;
                        a[3] += c.a as u32;
                    }
                }
                let (mut chosen, mut max_n) = (0usize, 0i32);
                for (i, &n) in count.iter().enumerate() {
                    if n > max_n {
                        chosen = i;
                        max_n = n;
                    }
                }
                let a = avg[chosen];
                let m = max_n as u32;
                drow[x as usize] = Bgra::new((a[2] / m) as u8, (a[1] / m) as u8, (a[0] / m) as u8, (a[3] / m) as u8);
            }
        });
    }
}

/// Blurred, contrast-adjusted copy screened over the original.
pub struct Glow {
    pub radius: i32,
    pub brightness: i32,
    pub contrast: i32,
}

impl Effect for Glow {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let mut p = src.clone();
        apply(&GaussianBlur(self.radius.clamp(1, 20)), &mut p, None);
        apply_op(&BrightnessContrast::new(self.brightness, self.contrast), &mut p, None);
        combine(src, &p, dst, roi, |s, p| blend(p, s, BlendMode::Screen, 255));
    }
}

/// dst = f(src, processed) over the roi.
fn combine(src: &Surface, p: &Surface, dst: &mut Surface, roi: PixelRect, f: impl Fn(Bgra, Bgra) -> Bgra + Sync + Send) {
    rows(dst, roi, |y, drow| {
        let (s, q) = (src.row(y), p.row(y));
        for x in roi.x as usize..roi.right() as usize {
            drow[x] = f(s[x], q[x]);
        }
    });
}

pub struct InkSketch {
    pub ink_outline: i32,
    pub coloring: i32,
}

const INK_CONV: [[i32; 5]; 5] = [
    [-1, -1, -1, -1, -1],
    [-1, -1, -1, -1, -1],
    [-1, -1, 30, -1, -1],
    [-1, -1, -1, -1, -1],
    [-1, -1, -5, -1, -1],
];

impl Effect for InkSketch {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let mut glow = src.clone();
        let k = -(self.coloring - 50) * 2;
        apply(&Glow { radius: 6, brightness: k, contrast: k }, &mut glow, None);
        let threshold = self.ink_outline * 255 / 100;
        rows(dst, roi, |y, drow| {
            let top = (y - 2).max(0);
            let bottom = src.height().min(y + 3);
            let grow = glow.row(y);
            for x in roi.x..roi.right() {
                let left = (x - 2).max(0);
                let right = src.width().min(x + 3);
                let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
                for v in top..bottom {
                    let row = src.row(v);
                    let kr = &INK_CONV[(v - y + 2) as usize];
                    for u in left..right {
                        let w = kr[(u - x + 2) as usize];
                        let c = row[u as usize];
                        r += c.r as i32 * w;
                        g += c.g as i32 * w;
                        b += c.b as i32 * w;
                    }
                }
                let o = Desaturate.apply(Bgra::bgr(cb(b), cb(g), cb(r)));
                let o = if o.r as i32 > threshold { Bgra::new(255, 255, 255, o.a) } else { Bgra::new(0, 0, 0, o.a) };
                drow[x as usize] = blend(grow[x as usize], o, BlendMode::Darken, 255);
            }
        });
    }
}

pub struct PencilSketch {
    pub tip_size: i32,
    pub color_range: i32,
}

impl Effect for PencilSketch {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let mut p = src.clone();
        apply(&GaussianBlur(self.tip_size.clamp(1, 20)), &mut p, None);
        apply_op(&BrightnessContrast::new(self.color_range, -self.color_range), &mut p, None);
        apply_op(&InvertColors, &mut p, None);
        apply_op(&Desaturate, &mut p, None);
        combine(src, &p, dst, roi, |s, p| blend(p, Desaturate.apply(s), BlendMode::ColorDodge, 255));
    }
}

pub struct SoftenPortrait {
    pub softness: i32,
    pub lighting: i32,
    pub warmth: i32,
}

impl Effect for SoftenPortrait {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let mut p = src.clone();
        apply(&GaussianBlur(self.softness.clamp(0, 10) * 3), &mut p, None);
        apply_op(&BrightnessContrast::new(self.lighting, -self.lighting / 2), &mut p, None);
        let red = 1.0f32 + self.warmth as f32 / 100.0;
        let blue = 1.0f32 - self.warmth as f32 / 100.0;
        combine(src, &p, dst, roi, |s, p| {
            let g = Desaturate.apply(s);
            let g = Bgra::new(cb((g.b as f32 * blue) as i32), g.g, cb((g.r as f32 * red) as i32), g.a);
            blend(p, g, BlendMode::Overlay, 255)
        });
    }
}
