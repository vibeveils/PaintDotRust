//! Geometric distortions.

use super::{rows, Effect};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::sampling::{bilinear_clamped, bilinear_sample, bilinear_wrapped, reflect_coord, rgss_offsets};
use crate::surface::Surface;
use std::f32::consts::PI as PI_F;
use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpEdge {
    Clamp,
    Wrap,
    Reflect,
    Original,
}

/// Polar inversion (the one C# `WarpEffect` subclass).
pub struct PolarInversion {
    pub amount: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    pub edge: WarpEdge,
    pub quality: i32,
}

impl Effect for PolarInversion {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let r = src.width().min(src.height()) as f64 * 0.5;
        let r2 = r * r;
        let cx = src.width() as f64 * (1.0 + self.offset_x) * 0.5;
        let cy = src.height() as f64 * (1.0 + self.offset_y) * 0.5;
        let aa = rgss_offsets(self.quality.clamp(1, 5));
        let (wm, hm) = ((src.width() - 1) as f32, (src.height() - 1) as f32);
        rows(dst, roi, |y, drow| {
            let mut samples = Vec::with_capacity(aa.len());
            let rel_y = y as f64 - cy;
            for x in roi.x..roi.right() {
                let rel_x = x as f64 - cx;
                samples.clear();
                for &(ax, ay) in &aa {
                    let mut tx = rel_x + ax as f64;
                    let mut ty = rel_y - ay as f64;
                    let inv = 1.0 + (r2 / (tx * tx + ty * ty) - 1.0) * self.amount;
                    tx *= inv;
                    ty *= inv;
                    let sx = (tx + cx) as f32;
                    let sy = (ty + cy) as f32;
                    let s = if sx >= 0.0 && sx <= wm && sy >= 0.0 && sy <= hm {
                        bilinear_sample(src, sx, sy)
                    } else {
                        match self.edge {
                            WarpEdge::Clamp => bilinear_clamped(src, sx, sy),
                            WarpEdge::Wrap => bilinear_wrapped(src, sx, sy),
                            WarpEdge::Reflect => {
                                bilinear_clamped(src, reflect_coord(sx, src.width()), reflect_coord(sy, src.height()))
                            }
                            WarpEdge::Original => src.get(x, y),
                        }
                    };
                    samples.push(s);
                }
                drow[x as usize] = Bgra::blend_average(&samples);
            }
        });
    }
}

pub struct Twist {
    twist: f64,
    inv_size: f64,
    offset_x: f64,
    offset_y: f64,
    aa: Vec<(f32, f32)>,
}

impl Twist {
    pub fn new(amount: f64, size: f64, offset_x: f64, offset_y: f64, quality: i32) -> Self {
        let a = -amount;
        let sign = if a > 0.0 { 1.0 } else if a < 0.0 { -1.0 } else { 0.0 };
        Twist {
            twist: a * a * sign / 100.0,
            inv_size: 1.0 / size.max(0.01),
            offset_x,
            offset_y,
            aa: rgss_offsets(quality.clamp(1, 5)),
        }
    }
}

impl Effect for Twist {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let (w, h) = (src.width(), src.height());
        let hw = w as f32 / 2.0 + (w as f64 / 2.0 * self.offset_x) as f32;
        let hh = h as f32 / 2.0 + (h as f64 / 2.0 * self.offset_y) as f32;
        let invmaxrad = 1.0 / (w as f64 / 2.0).min(h as f64 / 2.0);
        rows(dst, roi, |y, drow| {
            let mut samples = Vec::with_capacity(self.aa.len());
            let j = y as f32 - hh;
            for x in roi.x..roi.right() {
                let i = x as f32 - hw;
                samples.clear();
                for &(ax, ay) in &self.aa {
                    let u = i + ax;
                    let v = j + ay;
                    let rad = ((u * u + v * v) as f64).sqrt();
                    let mut theta = (v as f64).atan2(u as f64);
                    let mut t = 1.0 - rad * self.inv_size * invmaxrad;
                    t = if t < 0.0 { 0.0 } else { t * t * t };
                    theta += t * self.twist;
                    samples.push(bilinear_clamped(src, hw + (rad * theta.cos()) as f32, hh + (rad * theta.sin()) as f32));
                }
                drow[x as usize] = Bgra::blend_average(&samples);
            }
        });
    }
}

pub struct Bulge {
    pub amount: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

impl Effect for Bulge {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let mut hw = src.width() as f32 / 2.0;
        let mut hh = src.height() as f32 / 2.0;
        let maxrad = hw.min(hh);
        let amt = (self.amount / 100.0) as f32;
        hw += (self.offset_x * hw as f64) as f32;
        hh += (self.offset_y * hh as f64) as f32;
        rows(dst, roi, |y, drow| {
            let srow = src.row(y);
            let v = y as f32 - hh;
            for x in roi.x..roi.right() {
                let u = x as f32 - hw;
                let r = (u * u + v * v).sqrt();
                let rs1 = 1.0 - r / maxrad;
                drow[x as usize] = if rs1 > 0.0 {
                    let rs2 = 1.0 - amt * rs1 * rs1;
                    bilinear_clamped(src, u * rs2 + hw, v * rs2 + hh)
                } else {
                    srow[x as usize]
                };
            }
        });
    }
}

pub struct TileReflection {
    sin: f32,
    cos: f32,
    scale: f32,
    intensity: f32,
    aa: Vec<(f32, f32)>,
}

impl TileReflection {
    pub fn new(rotation_degrees: f64, square_size: f64, curvature: f64, quality: i32) -> Self {
        let mut q = quality.clamp(1, 5);
        if q != 1 {
            q += 1;
        }
        let sign = if curvature > 0.0 { 1.0 } else if curvature < 0.0 { -1.0 } else { 0.0 };
        TileReflection {
            sin: (rotation_degrees * PI / 180.0).sin() as f32,
            cos: (rotation_degrees * PI / 180.0).cos() as f32,
            scale: (PI / square_size.max(1.0)) as f32,
            intensity: (curvature * curvature / 10.0 * sign) as f32,
            aa: rgss_offsets(q),
        }
    }
}

impl Effect for TileReflection {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let hw = src.width() as f32 / 2.0;
        let hh = src.height() as f32 / 2.0;
        let (s, c, sc, k) = (self.sin, self.cos, self.scale, self.intensity);
        rows(dst, roi, |y, drow| {
            let mut samples = Vec::with_capacity(self.aa.len());
            let j = y as f32 - hh;
            for x in roi.x..roi.right() {
                let i = x as f32 - hw;
                samples.clear();
                for &(ax, ay) in &self.aa {
                    let u1 = i + ax;
                    let v1 = j - ay;
                    let s1 = c * u1 + s * v1;
                    let t1 = -s * u1 + c * v1;
                    let s2 = s1 + k * (s1 * sc).tan();
                    let t2 = t1 + k * (t1 * sc).tan();
                    let u2 = c * s2 - s * t2;
                    let v2 = s * s2 + c * t2;
                    samples.push(bilinear_wrapped(src, hw + u2, hh + v2));
                }
                drow[x as usize] = Bgra::blend_average(&samples);
            }
        });
    }
}

pub struct Spherize {
    pub amount_percent: i32,
    pub radius_percent: i32,
    pub offset_x_percent: i32,
    pub offset_y_percent: i32,
    pub quality: i32,
}

impl Effect for Spherize {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let aa = rgss_offsets(self.quality.clamp(1, 5));
        let a = self.amount_percent.clamp(-100, 100) as f32 / 100.0;
        let (w, h) = (src.width() as f32, src.height() as f32);
        let cx = w / 2.0 + self.offset_x_percent as f32 / 200.0 * w;
        let cy = h / 2.0 + self.offset_y_percent as f32 / 200.0 * h;
        let radius = 1f32.max(self.radius_percent.clamp(1, 200) as f32 / 100.0 * w.min(h) / 2.0);
        let n = aa.len() as i32;
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (mut b, mut g, mut r, mut al) = (0i32, 0i32, 0i32, 0i32);
                for &(ox, oy) in &aa {
                    let px = x as f32 + 0.5 + ox;
                    let py = y as f32 + 0.5 + oy;
                    let dx = (px - cx) / radius;
                    let dy = (py - cy) / radius;
                    let rr = (dx * dx + dy * dy).sqrt();
                    let (mut sxp, mut syp) = (px, py);
                    if rr < 1.0 && rr > 0.0 {
                        let target = if a >= 0.0 { 2.0 / PI_F * rr.asin() } else { (rr * PI_F / 2.0).sin() };
                        let rs = rr + (target - rr) * a.abs();
                        let k = rs / rr;
                        sxp = cx + dx * k * radius;
                        syp = cy + dy * k * radius;
                    }
                    let c = bilinear_clamped(src, sxp - 0.5, syp - 0.5);
                    b += c.b as i32;
                    g += c.g as i32;
                    r += c.r as i32;
                    al += c.a as i32;
                }
                drow[x as usize] = Bgra::new((b / n) as u8, (g / n) as u8, (r / n) as u8, (al / n) as u8);
            }
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveType {
    Ripples,
    Spiral,
    Horizontal,
    Vertical,
    Grid,
    Zigzag,
    Organic,
}

pub struct Waves {
    pub kind: WaveType,
    pub wavelength: i32,
    pub amplitude: i32,
    pub phase_degrees: i32,
    pub offset_x_percent: i32,
    pub offset_y_percent: i32,
    pub quality: i32,
}

#[inline]
fn triangle(t: f32) -> f32 {
    2.0 * (2.0 * (t - (t + 0.5).floor())).abs() - 1.0
}

impl Effect for Waves {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let aa = rgss_offsets(self.quality.clamp(1, 5));
        let lambda = 2f32.max(self.wavelength as f32);
        let amp = self.amplitude as f32;
        let phase = self.phase_degrees as f32 * PI_F / 180.0;
        let k = 2.0 * PI_F / lambda;
        let (w, h) = (src.width() as f32, src.height() as f32);
        let cx = w / 2.0 + self.offset_x_percent as f32 / 200.0 * w;
        let cy = h / 2.0 + self.offset_y_percent as f32 / 200.0 * h;
        let n = aa.len() as i32;
        let kind = self.kind;
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (mut b, mut g, mut r, mut al) = (0i32, 0i32, 0i32, 0i32);
                for &(ox, oy) in &aa {
                    let px = x as f32 + 0.5 + ox;
                    let py = y as f32 + 0.5 + oy;
                    let dx = px - cx;
                    let dy = py - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let (ux, uy) = if dist > 1e-3 { (dx / dist, dy / dist) } else { (0.0, 0.0) };
                    let (mut mx, mut my) = (0f32, 0f32);
                    match kind {
                        WaveType::Ripples => {
                            let d = amp * (k * dist + phase).sin();
                            mx = ux * d;
                            my = uy * d;
                        }
                        WaveType::Spiral => {
                            let d = amp * (k * dist + 3.0 * dy.atan2(dx) + phase).sin();
                            mx = -uy * d;
                            my = ux * d;
                        }
                        WaveType::Horizontal => mx = amp * (k * py + phase).sin(),
                        WaveType::Vertical => my = amp * (k * px + phase).sin(),
                        WaveType::Grid => {
                            mx = amp * (k * py + phase).sin();
                            my = amp * (k * px + phase).sin();
                        }
                        WaveType::Zigzag => {
                            let d = amp * triangle(dist / lambda + phase / (2.0 * PI_F));
                            mx = ux * d;
                            my = uy * d;
                        }
                        WaveType::Organic => {
                            mx = amp
                                * (0.6 * (k * py + phase).sin()
                                    + 0.3 * (k * 2.17 * (px + py) * 0.7071 + phase * 1.3).sin()
                                    + 0.1 * (k * 4.31 * px).sin());
                            my = amp
                                * (0.6 * (k * px + phase * 0.8).sin()
                                    + 0.3 * (k * 1.73 * (px - py) * 0.7071 + phase).sin()
                                    + 0.1 * (k * 3.89 * py).sin());
                        }
                    }
                    let sxp = reflect_coord(px + mx, src.width());
                    let syp = reflect_coord(py + my, src.height());
                    let c = bilinear_clamped(src, sxp - 0.5, syp - 0.5);
                    b += c.b as i32;
                    g += c.g as i32;
                    r += c.r as i32;
                    al += c.a as i32;
                }
                drow[x as usize] = Bgra::new((b / n) as u8, (g / n) as u8, (r / n) as u8, (al / n) as u8);
            }
        });
    }
}
