//! Brush stamping into coverage masks and mask compositing (C# `Tools.Brushes`).

use crate::blend::over_normal;
use crate::color::Bgra;
use crate::num::round_even_f;
use crate::selection::Selection;
use crate::surface::Surface;

/// Eraser fill patterns (8×8 hatch tiles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillPattern {
    #[default]
    Solid,
    Horizontal,
    Vertical,
    Cross,
    ForwardDiagonal,
    BackwardDiagonal,
    DiagonalCross,
    Percent50,
    Percent25,
    DottedGrid,
    Checkerboard,
}

impl FillPattern {
    pub const ALL: [FillPattern; 11] = [
        FillPattern::Solid,
        FillPattern::Horizontal,
        FillPattern::Vertical,
        FillPattern::Cross,
        FillPattern::ForwardDiagonal,
        FillPattern::BackwardDiagonal,
        FillPattern::DiagonalCross,
        FillPattern::Percent50,
        FillPattern::Percent25,
        FillPattern::DottedGrid,
        FillPattern::Checkerboard,
    ];

    #[inline]
    pub fn covers(self, x: i32, y: i32) -> bool {
        let px = x & 7;
        let py = y & 7;
        match self {
            FillPattern::Solid => true,
            FillPattern::Horizontal => py == 0,
            FillPattern::Vertical => px == 0,
            FillPattern::Cross => px == 0 || py == 0,
            FillPattern::ForwardDiagonal => ((px + py) & 7) == 0,
            FillPattern::BackwardDiagonal => ((px - py) & 7) == 0,
            FillPattern::DiagonalCross => ((px + py) & 7) == 0 || ((px - py) & 7) == 0,
            FillPattern::Percent50 => ((x + y) & 1) == 0,
            FillPattern::Percent25 => (x & 1) == 0 && (y & 1) == 0,
            FillPattern::DottedGrid => (px & 3) == 0 && (py & 3) == 0,
            FillPattern::Checkerboard => (((px >> 2) + (py >> 2)) & 1) == 0,
        }
    }
}

/// Brush diameter → integer radius used by the hard-edged tools.
#[inline]
pub fn radius_for_size(brush_size: i32) -> i32 {
    ((brush_size - 1) / 2).max(0)
}

/// Bresenham walk from (x0,y0) to (x1,y1) inclusive.
#[inline]
pub(crate) fn bresenham(mut x0: i32, mut y0: i32, x1: i32, y1: i32, mut f: impl FnMut(i32, i32)) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        f(x0, y0);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = err * 2;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// Hard disc of `color` (no blending). Returns unclamped inclusive bounds.
pub fn stamp_disc(s: &mut Surface, cx: i32, cy: i32, radius: i32, color: Bgra, sel: Option<&Selection>) -> (i32, i32, i32, i32) {
    if radius <= 0 {
        if s.in_bounds(cx, cy) && sel.is_none_or(|m| m.contains(cx, cy)) {
            s.set(cx, cy, color);
        }
        return (cx, cy, cx, cy);
    }
    let r2 = radius * radius + radius;
    let x0 = (cx - radius).max(0);
    let y0 = (cy - radius).max(0);
    let x1 = (s.width() - 1).min(cx + radius);
    let y1 = (s.height() - 1).min(cy + radius);
    for y in y0..=y1 {
        let dy = y - cy;
        let dy2 = dy * dy;
        let row = s.row_mut(y);
        for x in x0..=x1 {
            let dx = x - cx;
            if dx * dx + dy2 > r2 {
                continue;
            }
            if let Some(m) = sel {
                if !m.contains(x, y) {
                    continue;
                }
            }
            row[x as usize] = color;
        }
    }
    (cx - radius, cy - radius, cx + radius, cy + radius)
}

#[allow(clippy::too_many_arguments)]
pub fn stroke_disc(s: &mut Surface, x0: i32, y0: i32, x1: i32, y1: i32, radius: i32, color: Bgra, sel: Option<&Selection>) {
    bresenham(x0, y0, x1, y1, |x, y| {
        stamp_disc(s, x, y, radius, color, sel);
    });
}

/// Max-accumulates an antialiased disc into `mask`. Returns clamped inclusive bounds.
pub fn stamp_disc_mask(mask: &mut [u8], width: i32, height: i32, cx: i32, cy: i32, radius: f32, aa: bool) -> (i32, i32, i32, i32) {
    let r = radius.max(0.0);
    let x0 = ((cx as f32 - r - 1.0).floor() as i32).max(0);
    let y0 = ((cy as f32 - r - 1.0).floor() as i32).max(0);
    let x1 = (width - 1).min((cx as f32 + r + 1.0).ceil() as i32);
    let y1 = (height - 1).min((cy as f32 + r + 1.0).ceil() as i32);
    let w = width as usize;
    for y in y0..=y1 {
        let dy = y - cy;
        let row = &mut mask[y as usize * w..(y as usize + 1) * w];
        for x in x0..=x1 {
            let dx = x - cx;
            let dist = ((dx * dx + dy * dy) as f32).sqrt();
            let mut cov = (r + 0.5 - dist).clamp(0.0, 1.0);
            if !aa {
                cov = if cov >= 0.5 { 1.0 } else { 0.0 };
            }
            if cov <= 0.0 {
                continue;
            }
            let v = (cov * 255.0 + 0.5) as u8;
            let m = &mut row[x as usize];
            if v > *m {
                *m = v;
            }
        }
    }
    (x0, y0, x1, y1)
}

#[allow(clippy::too_many_arguments)]
pub fn stroke_disc_mask(mask: &mut [u8], width: i32, height: i32, x0: i32, y0: i32, x1: i32, y1: i32, radius: f32, aa: bool) {
    bresenham(x0, y0, x1, y1, |x, y| {
        stamp_disc_mask(mask, width, height, x, y, radius, aa);
    });
}

/// Stamps discs along a segment with radius interpolated from `r0` to `r1`.
#[allow(clippy::too_many_arguments)]
pub fn stroke_disc_mask_tapered(
    mask: &mut [u8],
    width: i32,
    height: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    r0: f32,
    r1: f32,
    aa: bool,
) -> (i32, i32, i32, i32) {
    let len = (((x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)) as f32).sqrt();
    let spacing = 0.5f32.max(r0.min(r1) * 0.35);
    let steps = 1.max((len / spacing).ceil() as i32);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let cx = round_even_f(x0 as f32 + (x1 - x0) as f32 * t) as i32;
        let cy = round_even_f(y0 as f32 + (y1 - y0) as f32 * t) as i32;
        let (a, b, c, d) = stamp_disc_mask(mask, width, height, cx, cy, r0 + (r1 - r0) * t, aa);
        min_x = min_x.min(a);
        min_y = min_y.min(b);
        max_x = max_x.max(c);
        max_y = max_y.max(d);
    }
    (min_x, min_y, max_x, max_y)
}

/// Paints `color` through `mask` over the stroke-start pixels `before`.
#[allow(clippy::too_many_arguments)]
pub fn composite_mask_paint(
    dst: &mut Surface,
    before: &[Bgra],
    mask: &[u8],
    color: Bgra,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    sel: Option<&Selection>,
    alpha_blend: bool,
) {
    let w = dst.width() as usize;
    let min_x = min_x.max(0);
    let min_y = min_y.max(0);
    let max_x = max_x.min(dst.width() - 1);
    let max_y = max_y.min(dst.height() - 1);
    for y in min_y..=max_y {
        let o = y as usize * w;
        let row = dst.row_mut(y);
        for x in min_x..=max_x {
            let mut m = mask[o + x as usize] as i32;
            if m == 0 {
                continue;
            }
            if let Some(sel) = sel {
                let sm = sel.coverage_at(x, y) as i32;
                if sm == 0 {
                    continue;
                }
                m = m * sm / 255;
            }
            let b = before[o + x as usize];
            row[x as usize] = if alpha_blend {
                let a = ((color.a as i32 * m + 127) / 255) as u8;
                over_normal(color.with_alpha(a), b, 255)
            } else {
                let l = |p: u8, c: u8| ((p as i32 * (255 - m) + c as i32 * m + 127) / 255) as u8;
                Bgra::new(l(b.b, color.b), l(b.g, color.g), l(b.r, color.r), l(b.a, color.a))
            };
        }
    }
}

/// Erases through `mask` (optionally patterned) from the stroke-start pixels.
#[allow(clippy::too_many_arguments)]
pub fn composite_mask_erase(
    dst: &mut Surface,
    before: &[Bgra],
    mask: &[u8],
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    sel: Option<&Selection>,
    pattern: FillPattern,
) {
    let w = dst.width() as usize;
    let min_x = min_x.max(0);
    let min_y = min_y.max(0);
    let max_x = max_x.min(dst.width() - 1);
    let max_y = max_y.min(dst.height() - 1);
    for y in min_y..=max_y {
        let o = y as usize * w;
        let row = dst.row_mut(y);
        for x in min_x..=max_x {
            let mut m = mask[o + x as usize] as i32;
            if m == 0 {
                continue;
            }
            if pattern != FillPattern::Solid && !pattern.covers(x, y) {
                continue;
            }
            if let Some(sel) = sel {
                let sm = sel.coverage_at(x, y) as i32;
                if sm == 0 {
                    continue;
                }
                m = m * sm / 255;
            }
            let b = before[o + x as usize];
            row[x as usize] = b.with_alpha((b.a as i32 * (255 - m) / 255) as u8);
        }
    }
}
