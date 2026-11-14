//! Effects (C# `Effects`). An effect renders from a snapshot of the layer
//! into the layer within a region of interest; selections then blend the
//! result back by coverage.

use crate::adjust::lerp_by_mask;
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::selection::Selection;
use crate::surface::Surface;

pub mod artistic;
pub mod blur;
pub mod distort;
pub mod dotnet;
pub mod math;
pub mod print;
pub mod render;
pub mod stylize;

pub trait Effect: Sync {
    /// Writes output pixels for `roi` into `dst`; `src` is the unmodified input.
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect);
}

/// Renders `effect` over the selection (or whole surface) in place.
pub fn apply(effect: &dyn Effect, surface: &mut Surface, sel: Option<&Selection>) {
    let sel = sel.filter(|s| !s.is_empty());
    let roi = sel.map_or(surface.bounds(), |s| s.bounds().intersect(&surface.bounds()));
    if roi.is_empty() {
        return;
    }
    let src = surface.clone();
    effect.render(&src, surface, roi);
    if let Some(sel) = sel {
        let w = surface.width() as usize;
        let rows = &mut surface.pixels_mut()[roi.y as usize * w..roi.bottom() as usize * w];
        crate::par::for_rows(rows, w, |i, row| {
            let y = roi.y + i as i32;
            let srow = src.row(y);
            for x in roi.x..roi.right() {
                let m = sel.coverage_at(x, y);
                if m == 255 {
                    continue;
                }
                let xi = x as usize;
                row[xi] = if m == 0 { srow[xi] } else { lerp_by_mask(srow[xi], row[xi], m) };
            }
        });
    }
}

/// Runs `f(y, dst_row)` for every row of `roi` in parallel (C# `RowEffect`).
pub(crate) fn rows(dst: &mut Surface, roi: PixelRect, f: impl Fn(i32, &mut [Bgra]) + Sync + Send) {
    let w = dst.width() as usize;
    let rows = &mut dst.pixels_mut()[roi.y as usize * w..roi.bottom() as usize * w];
    crate::par::for_rows(rows, w, |i, row| f(roi.y + i as i32, row));
}

/// `(byte)Math.Clamp(v, 0, 255)`.
#[inline(always)]
pub(crate) fn cb(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Legacy truncating `ClampToByte(float)`.
#[inline(always)]
pub(crate) fn cbf(v: f32) -> u8 {
    if v <= 0.0 {
        0
    } else if v >= 255.0 {
        255
    } else {
        v as u8
    }
}

/// C# `Effect.Lerp`: per channel `from + (to − from)·frac`, truncating clamp.
#[inline]
pub(crate) fn lerp(from: Bgra, to: Bgra, frac: f32) -> Bgra {
    let l = |a: u8, b: u8| cbf(a as f32 + (b as i32 - a as i32) as f32 * frac);
    Bgra::new(l(from.b, to.b), l(from.g, to.g), l(from.r, to.r), l(from.a, to.a))
}
