//! Resampling a source rectangle into a fractional target rectangle
//! (C# `Resampling`), used by Move Selected Pixels and selection scaling.

use crate::color::Bgra;
use crate::num::{clamp_byte_f32, clamp_byte_f64};
use crate::rect::{PixelRect, RectF};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TransformResampling {
    NearestNeighbor,
    NearestNeighborAntialiased,
    #[default]
    Bilinear,
    /// 4×4 bilinear samples per destination pixel: smooth edges when
    /// shrinking a little or moving by fractions of a pixel.
    MultisampleBilinear,
    /// Bilinear samples spread over the pixel's footprint in the source, as
    /// many along each axis as that axis is shrunk (up to 16): sharp when
    /// enlarging, alias-free when shrinking, even by different amounts in x and y.
    Anisotropic,
    /// Catmull-Rom bicubic, widened when shrinking (up to 2×) to avoid aliasing.
    Bicubic,
}

/// Resampling options: filter plus whether to blend in linear light.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Resampling {
    pub mode: TransformResampling,
    /// Gamma-corrected: interpolate premultiplied colour in linear light so
    /// luminance is preserved. Otherwise interpolate the stored sRGB values.
    pub linear: bool,
}

/// Visits each destination pixel overlapping `target`, giving its
/// resampled colour and the fraction of the pixel covered by `target`.
#[allow(clippy::too_many_arguments)]
pub fn resample(
    src: &[Bgra],
    src_width: i32,
    mask: Option<&[u8]>,
    src_rect: PixelRect,
    target: RectF,
    dst_width: i32,
    dst_height: i32,
    opts: impl Into<Resampling>,
    mut write: impl FnMut(i32, i32, Bgra, f32),
) {
    let Resampling { mode, linear } = opts.into();
    if src_rect.is_empty() || target.w <= 0.0 || target.h <= 0.0 {
        return;
    }
    // Same size at a whole-pixel offset: every filter reduces to a copy.
    if target.w == src_rect.width as f64 && target.h == src_rect.height as f64 && target.x.fract() == 0.0 && target.y.fract() == 0.0 {
        let (dx, dy) = (target.x as i32 - src_rect.x, target.y as i32 - src_rect.y);
        let w = src_width as usize;
        for y in src_rect.y.max(-dy)..src_rect.bottom().min(dst_height - dy) {
            for x in src_rect.x.max(-dx)..src_rect.right().min(dst_width - dx) {
                write(x + dx, y + dy, fetch(src, w, mask, x, y), 1.0);
            }
        }
        return;
    }
    let cover = target.to_covering_pixel_rect();
    let x0 = cover.x.max(0);
    let y0 = cover.y.max(0);
    let x1 = dst_width.min(cover.right());
    let y1 = dst_height.min(cover.bottom());
    let sx = src_rect.width as f64 / target.w;
    let sy = src_rect.height as f64 / target.h;
    let w = src_width as usize;
    for y in y0..y1 {
        let ty0 = (y as f64).max(target.y);
        let ty1 = ((y + 1) as f64).min(target.bottom());
        if ty1 <= ty0 {
            continue;
        }
        for x in x0..x1 {
            let tx0 = (x as f64).max(target.x);
            let tx1 = ((x + 1) as f64).min(target.right());
            if tx1 <= tx0 {
                continue;
            }
            let coverage = ((tx1 - tx0) * (ty1 - ty0)) as f32;
            let u0 = src_rect.x as f64 + (tx0 - target.x) * sx;
            let u1 = src_rect.x as f64 + (tx1 - target.x) * sx;
            let v0 = src_rect.y as f64 + (ty0 - target.y) * sy;
            let v1 = src_rect.y as f64 + (ty1 - target.y) * sy;
            let (uc, vc) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            let t = Taps { src, w, mask, r: src_rect, linear };
            let c = match mode {
                TransformResampling::NearestNeighbor => nearest(src, w, mask, src_rect, uc, vc),
                TransformResampling::NearestNeighborAntialiased if !linear => area(src, w, mask, src_rect, u0, u1, v0, v1),
                TransformResampling::Bilinear if !linear => bilinear(src, w, mask, src_rect, uc, vc),
                TransformResampling::NearestNeighborAntialiased => t.finish(t.area(u0, u1, v0, v1)),
                TransformResampling::Bilinear => t.finish(t.bilinear(uc, vc)),
                TransformResampling::MultisampleBilinear => t.finish(t.grid(u0, u1, v0, v1, 4, 4)),
                TransformResampling::Anisotropic => {
                    let n = |a: f64, b: f64| ((b - a) * (1.0 + 1e-9)).ceil().clamp(1.0, 16.0) as usize;
                    t.finish(t.grid(u0, u1, v0, v1, n(u0, u1), n(v0, v1)))
                }
                TransformResampling::Bicubic => t.finish(t.bicubic(uc, vc, sx.clamp(1.0, 2.0), sy.clamp(1.0, 2.0))),
            };
            write(x, y, c, coverage);
        }
    }
}

/// Alpha-only area resampling of a coverage mask; equivalent to
/// [`resample`] in antialiased-nearest mode over white pixels whose alpha is the mask.
pub(crate) fn resample_alpha(
    mask: &[u8],
    src_width: i32,
    r: PixelRect,
    target: RectF,
    dst_width: i32,
    dst_height: i32,
    mut write: impl FnMut(i32, i32, u8, f32),
) {
    if r.is_empty() || target.w <= 0.0 || target.h <= 0.0 {
        return;
    }
    let cover = target.to_covering_pixel_rect();
    let x0 = cover.x.max(0);
    let y0 = cover.y.max(0);
    let x1 = dst_width.min(cover.right());
    let y1 = dst_height.min(cover.bottom());
    let sx = r.width as f64 / target.w;
    let sy = r.height as f64 / target.h;
    let w = src_width as usize;
    for y in y0..y1 {
        let ty0 = (y as f64).max(target.y);
        let ty1 = ((y + 1) as f64).min(target.bottom());
        if ty1 <= ty0 {
            continue;
        }
        let v0 = r.y as f64 + (ty0 - target.y) * sy;
        let v1 = r.y as f64 + (ty1 - target.y) * sy;
        let iy0 = r.y.max(v0.floor() as i32);
        let iy1 = (r.bottom() - 1).min(v1.ceil() as i32 - 1);
        for x in x0..x1 {
            let tx0 = (x as f64).max(target.x);
            let tx1 = ((x + 1) as f64).min(target.right());
            if tx1 <= tx0 {
                continue;
            }
            let coverage = ((tx1 - tx0) * (ty1 - ty0)) as f32;
            let u0 = r.x as f64 + (tx0 - target.x) * sx;
            let u1 = r.x as f64 + (tx1 - target.x) * sx;
            let ix0 = r.x.max(u0.floor() as i32);
            let ix1 = (r.right() - 1).min(u1.ceil() as i32 - 1);
            let (mut sa, mut area) = (0.0f64, 0.0f64);
            for iy in iy0..=iy1 {
                let wy = ((iy + 1) as f64).min(v1) - (iy as f64).max(v0);
                if wy <= 0.0 {
                    continue;
                }
                let row = &mask[iy as usize * w..];
                for ix in ix0..=ix1 {
                    let wx = ((ix + 1) as f64).min(u1) - (ix as f64).max(u0);
                    if wx <= 0.0 {
                        continue;
                    }
                    let wgt = wx * wy;
                    sa += row[ix as usize] as f64 * wgt;
                    area += wgt;
                }
            }
            let a = if area <= 0.0 || sa <= 0.0 { 0 } else { clamp_byte_f64(sa / area + 0.5) };
            write(x, y, a, coverage);
        }
    }
}

#[inline(always)]
fn fetch(src: &[Bgra], w: usize, mask: Option<&[u8]>, x: i32, y: i32) -> Bgra {
    let i = y as usize * w + x as usize;
    let c = src[i];
    match mask {
        None => c,
        Some(m) => {
            let m = m[i];
            if m == 255 { c } else { c.with_alpha(((c.a as i32 * m as i32 + 127) / 255) as u8) }
        }
    }
}

fn nearest(src: &[Bgra], w: usize, mask: Option<&[u8]>, r: PixelRect, u: f64, v: f64) -> Bgra {
    let x = (u.floor() as i32).clamp(r.x, r.right() - 1);
    let y = (v.floor() as i32).clamp(r.y, r.bottom() - 1);
    fetch(src, w, mask, x, y)
}

#[allow(clippy::too_many_arguments)]
fn area(src: &[Bgra], w: usize, mask: Option<&[u8]>, r: PixelRect, u0: f64, u1: f64, v0: f64, v1: f64) -> Bgra {
    let (mut sb, mut sg, mut sr, mut sa, mut ar) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    let ix0 = r.x.max(u0.floor() as i32);
    let ix1 = (r.right() - 1).min(u1.ceil() as i32 - 1);
    let iy0 = r.y.max(v0.floor() as i32);
    let iy1 = (r.bottom() - 1).min(v1.ceil() as i32 - 1);
    for iy in iy0..=iy1 {
        let wy = ((iy + 1) as f64).min(v1) - (iy as f64).max(v0);
        if wy <= 0.0 {
            continue;
        }
        for ix in ix0..=ix1 {
            let wx = ((ix + 1) as f64).min(u1) - (ix as f64).max(u0);
            if wx <= 0.0 {
                continue;
            }
            let wgt = wx * wy;
            let c = fetch(src, w, mask, ix, iy);
            let a = c.a as f64 * wgt;
            sb += c.b as f64 * a;
            sg += c.g as f64 * a;
            sr += c.r as f64 * a;
            sa += a;
            ar += wgt;
        }
    }
    if ar <= 0.0 || sa <= 0.0 {
        return Bgra::TRANSPARENT;
    }
    Bgra::new(
        clamp_byte_f64(sb / sa + 0.5),
        clamp_byte_f64(sg / sa + 0.5),
        clamp_byte_f64(sr / sa + 0.5),
        clamp_byte_f64(sa / ar + 0.5),
    )
}

fn bilinear(src: &[Bgra], w: usize, mask: Option<&[u8]>, r: PixelRect, u: f64, v: f64) -> Bgra {
    let fx = (u - 0.5).clamp(r.x as f64, (r.right() - 1) as f64);
    let fy = (v - 0.5).clamp(r.y as f64, (r.bottom() - 1) as f64);
    let x0 = fx as i32;
    let y0 = fy as i32;
    let x1 = (x0 + 1).min(r.right() - 1);
    let y1 = (y0 + 1).min(r.bottom() - 1);
    let ax = fx - x0 as f64;
    let ay = fy - y0 as f64;
    let (mut sb, mut sg, mut sr, mut sa) = (0.0f64, 0.0, 0.0, 0.0);
    let mut add = |c: Bgra, wgt: f64| {
        let a = c.a as f64 * wgt;
        sb += c.b as f64 * a;
        sg += c.g as f64 * a;
        sr += c.r as f64 * a;
        sa += a;
    };
    add(fetch(src, w, mask, x0, y0), (1.0 - ax) * (1.0 - ay));
    add(fetch(src, w, mask, x1, y0), ax * (1.0 - ay));
    add(fetch(src, w, mask, x0, y1), (1.0 - ax) * ay);
    add(fetch(src, w, mask, x1, y1), ax * ay);
    if sa <= 0.0 {
        return Bgra::TRANSPARENT;
    }
    Bgra::new(
        clamp_byte_f64(sb / sa + 0.5),
        clamp_byte_f64(sg / sa + 0.5),
        clamp_byte_f64(sr / sa + 0.5),
        clamp_byte_f64(sa + 0.5),
    )
}

impl From<TransformResampling> for Resampling {
    fn from(mode: TransformResampling) -> Self {
        Resampling { mode, linear: false }
    }
}

// ---------------------------------------------------------------- filters
// Accumulate premultiplied [b, g, r, a] in f32 (0..1), in sRGB or linear light.

fn srgb_to_linear_lut() -> &'static [f32; 256] {
    static LUT: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut t = [0f32; 256];
        for (i, v) in t.iter_mut().enumerate() {
            let c = i as f64 / 255.0;
            *v = (if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }) as f32;
        }
        t
    })
}

#[inline]
fn linear_to_srgb_byte(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0) as f64;
    let c = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    clamp_byte_f64(c * 255.0 + 0.5)
}

struct Taps<'a> {
    src: &'a [Bgra],
    w: usize,
    mask: Option<&'a [u8]>,
    r: PixelRect,
    linear: bool,
}

impl Taps<'_> {
    #[inline]
    fn texel(&self, x: i32, y: i32) -> [f32; 4] {
        let x = x.clamp(self.r.x, self.r.right() - 1);
        let y = y.clamp(self.r.y, self.r.bottom() - 1);
        let c = fetch(self.src, self.w, self.mask, x, y);
        let a = c.a as f32 / 255.0;
        if self.linear {
            let l = srgb_to_linear_lut();
            [l[c.b as usize] * a, l[c.g as usize] * a, l[c.r as usize] * a, a]
        } else {
            [c.b as f32 / 255.0 * a, c.g as f32 / 255.0 * a, c.r as f32 / 255.0 * a, a]
        }
    }

    fn finish(&self, p: [f32; 4]) -> Bgra {
        let a = p[3].clamp(0.0, 1.0);
        if a <= 0.5 / 255.0 {
            return Bgra::TRANSPARENT;
        }
        let ch = |v: f32| {
            let v = (v / a).clamp(0.0, 1.0);
            if self.linear { linear_to_srgb_byte(v) } else { clamp_byte_f32(v * 255.0 + 0.5) }
        };
        Bgra::new(ch(p[0]), ch(p[1]), ch(p[2]), clamp_byte_f32(a * 255.0 + 0.5))
    }

    fn bilinear(&self, u: f64, v: f64) -> [f32; 4] {
        let fx = (u - 0.5).clamp(self.r.x as f64, (self.r.right() - 1) as f64);
        let fy = (v - 0.5).clamp(self.r.y as f64, (self.r.bottom() - 1) as f64);
        let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
        let (ax, ay) = ((fx - x0 as f64) as f32, (fy - y0 as f64) as f32);
        let mut acc = [0f32; 4];
        for (dx, dy, wgt) in [(0, 0, (1.0 - ax) * (1.0 - ay)), (1, 0, ax * (1.0 - ay)), (0, 1, (1.0 - ax) * ay), (1, 1, ax * ay)] {
            if wgt > 0.0 {
                let t = self.texel(x0 + dx, y0 + dy);
                for k in 0..4 {
                    acc[k] += t[k] * wgt;
                }
            }
        }
        acc
    }

    /// Average of nx × ny bilinear samples spread over the footprint.
    fn grid(&self, u0: f64, u1: f64, v0: f64, v1: f64, nx: usize, ny: usize) -> [f32; 4] {
        let mut acc = [0f32; 4];
        for j in 0..ny {
            let v = v0 + (v1 - v0) * (j as f64 + 0.5) / ny as f64;
            for i in 0..nx {
                let u = u0 + (u1 - u0) * (i as f64 + 0.5) / nx as f64;
                let t = self.bilinear(u, v);
                for k in 0..4 {
                    acc[k] += t[k];
                }
            }
        }
        let n = (nx * ny) as f32;
        acc.map(|v| v / n)
    }

    /// Box average over the footprint (antialiased nearest).
    fn area(&self, u0: f64, u1: f64, v0: f64, v1: f64) -> [f32; 4] {
        let r = self.r;
        let (ix0, ix1) = (r.x.max(u0.floor() as i32), (r.right() - 1).min(u1.ceil() as i32 - 1));
        let (iy0, iy1) = (r.y.max(v0.floor() as i32), (r.bottom() - 1).min(v1.ceil() as i32 - 1));
        let (mut acc, mut total) = ([0f32; 4], 0f32);
        for iy in iy0..=iy1 {
            let wy = ((iy + 1) as f64).min(v1) - (iy as f64).max(v0);
            for ix in ix0..=ix1 {
                let wx = ((ix + 1) as f64).min(u1) - (ix as f64).max(u0);
                if wx <= 0.0 || wy <= 0.0 {
                    continue;
                }
                let wgt = (wx * wy) as f32;
                let t = self.texel(ix, iy);
                for k in 0..4 {
                    acc[k] += t[k] * wgt;
                }
                total += wgt;
            }
        }
        if total > 0.0 { acc.map(|v| v / total) } else { acc }
    }

    /// Catmull-Rom with the kernel stretched by `kx`/`ky` (≥ 1) when shrinking.
    fn bicubic(&self, u: f64, v: f64, kx: f64, ky: f64) -> [f32; 4] {
        fn cr(t: f64) -> f64 {
            let t = t.abs();
            if t < 1.0 {
                1.5 * t * t * t - 2.5 * t * t + 1.0
            } else if t < 2.0 {
                -0.5 * t * t * t + 2.5 * t * t - 4.0 * t + 2.0
            } else {
                0.0
            }
        }
        let weights = |c: f64, k: f64| -> (i32, Vec<f32>) {
            let first = (c - 2.0 * k).floor() as i32;
            let last = (c + 2.0 * k).ceil() as i32;
            let mut w: Vec<f32> = (first..=last).map(|i| cr((i as f64 + 0.5 - c) / k) as f32).collect();
            let sum: f32 = w.iter().sum();
            if sum.abs() > 1e-6 {
                w.iter_mut().for_each(|x| *x /= sum);
            }
            (first, w)
        };
        let (fx, wx) = weights(u, kx);
        let (fy, wy) = weights(v, ky);
        let mut acc = [0f32; 4];
        for (j, &wyj) in wy.iter().enumerate() {
            if wyj == 0.0 {
                continue;
            }
            let mut row = [0f32; 4];
            for (i, &wxi) in wx.iter().enumerate() {
                if wxi == 0.0 {
                    continue;
                }
                let t = self.texel(fx + i as i32, fy + j as i32);
                for k in 0..4 {
                    row[k] += t[k] * wxi;
                }
            }
            for k in 0..4 {
                acc[k] += row[k] * wyj;
            }
        }
        // Ringing can overshoot: keep a valid premultiplied colour.
        let a = acc[3].clamp(0.0, 1.0);
        [acc[0].clamp(0.0, a), acc[1].clamp(0.0, a), acc[2].clamp(0.0, a), a]
    }
}

/// Blends `over` onto `under` by geometric coverage (C# `LerpByCoverage`).
#[inline]
pub fn lerp_by_coverage(under: Bgra, over: Bgra, coverage: f32) -> Bgra {
    if coverage >= 0.999 {
        return over;
    }
    if coverage <= 0.001 {
        return under;
    }
    let ua = under.a as f32 * (1.0 - coverage);
    let oa = over.a as f32 * coverage;
    let a = ua + oa;
    if a <= 0.0 {
        return Bgra::TRANSPARENT;
    }
    Bgra::new(
        clamp_byte_f32((under.b as f32 * ua + over.b as f32 * oa) / a + 0.5),
        clamp_byte_f32((under.g as f32 * ua + over.g as f32 * oa) / a + 0.5),
        clamp_byte_f32((under.r as f32 * ua + over.r as f32 * oa) / a + 0.5),
        clamp_byte_f32(a + 0.5),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODES: [TransformResampling; 6] = [
        TransformResampling::NearestNeighbor,
        TransformResampling::NearestNeighborAntialiased,
        TransformResampling::Bilinear,
        TransformResampling::MultisampleBilinear,
        TransformResampling::Anisotropic,
        TransformResampling::Bicubic,
    ];

    fn checker(n: i32) -> Vec<Bgra> {
        (0..n * n).map(|i| if (i % n + i / n) % 2 == 0 { Bgra::BLACK } else { Bgra::WHITE }).collect()
    }

    fn run(src: &[Bgra], n: i32, target: RectF, opts: Resampling, dw: i32, dh: i32) -> Vec<Bgra> {
        let mut out = vec![Bgra::TRANSPARENT; (dw * dh) as usize];
        resample(src, n, None, PixelRect::new(0, 0, n, n), target, dw, dh, opts, |x, y, c, _| out[(y * dw + x) as usize] = c);
        out
    }

    #[test]
    fn whole_pixel_moves_are_exact_in_every_mode() {
        let src: Vec<Bgra> = (0..64).map(|i| Bgra::new(i as u8 * 3, 200 - i as u8, 7, (i * 4) as u8)).collect();
        for mode in MODES {
            for linear in [false, true] {
                let out = run(&src, 8, RectF { x: 3.0, y: 2.0, w: 8.0, h: 8.0 }, Resampling { mode, linear }, 12, 12);
                for y in 0..8 {
                    for x in 0..8 {
                        assert_eq!(out[((y + 2) * 12 + x + 3) as usize], src[(y * 8 + x) as usize], "{mode:?} linear={linear}");
                    }
                }
            }
        }
    }

    #[test]
    fn gamma_corrected_downscale_preserves_luminance() {
        // A 1-px black/white checker halved: averaging sRGB values gives 128,
        // averaging light gives sRGB ≈ 188 (the perceived grey of the pattern).
        let src = checker(16);
        for mode in [TransformResampling::NearestNeighborAntialiased, TransformResampling::MultisampleBilinear, TransformResampling::Anisotropic, TransformResampling::Bicubic] {
            let t = RectF { x: 0.0, y: 0.0, w: 8.0, h: 8.0 };
            let plain = run(&src, 16, t, Resampling { mode, linear: false }, 8, 8);
            let lin = run(&src, 16, t, Resampling { mode, linear: true }, 8, 8);
            let mid = |v: &Vec<Bgra>| v[(3 * 8 + 3) as usize].g as i32;
            assert!((mid(&plain) - 128).abs() <= 3, "{mode:?} plain {}", mid(&plain));
            assert!((mid(&lin) - 188).abs() <= 3, "{mode:?} linear {}", mid(&lin));
        }
    }

    #[test]
    fn enlarging_stays_in_range_and_bicubic_is_sharper_than_bilinear() {
        // A hard vertical edge, enlarged 4×: bicubic's transition is steeper.
        let src: Vec<Bgra> = (0..64).map(|i| if i % 8 < 4 { Bgra::BLACK } else { Bgra::WHITE }).collect();
        let t = RectF { x: 0.0, y: 0.0, w: 32.0, h: 32.0 };
        let row = |mode| run(&src, 8, t, Resampling { mode, linear: false }, 32, 32)[16 * 32..17 * 32].iter().map(|c| c.g as i32).collect::<Vec<_>>();
        let (bl, bc) = (row(TransformResampling::Bilinear), row(TransformResampling::Bicubic));
        let mid = |r: &Vec<i32>| r.iter().filter(|&&v| v > 30 && v < 225).count();
        assert!(mid(&bc) < mid(&bl), "bicubic {bc:?} bilinear {bl:?}");
        for mode in MODES {
            let all = run(&src, 8, t, Resampling { mode, linear: true }, 32, 32);
            assert!(all.iter().all(|c| c.a == 255), "{mode:?} keeps opaque pixels opaque");
        }
    }
}
