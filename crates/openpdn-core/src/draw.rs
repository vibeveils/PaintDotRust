//! Antialiased shape rasterisation (C# `Tools.Draw`).

use crate::blend::over_normal;
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::selection::{polygon_crossings_int, Selection};
use crate::surface::Surface;

#[inline(always)]
fn sharpen(cov: f32, aa: bool) -> f32 {
    if aa { cov } else if cov >= 0.5 { 1.0 } else { 0.0 }
}

/// Paints one pixel with `color` at `coverage`, clipped by the selection.
#[inline]
pub fn blend_pixel(s: &mut Surface, x: i32, y: i32, color: Bgra, mut coverage: f32, sel: Option<&Selection>, alpha_blend: bool) {
    if !s.in_bounds(x, y) || coverage <= 0.0 {
        return;
    }
    if let Some(sel) = sel {
        let m = sel.coverage_at(x, y);
        if m == 0 {
            return;
        }
        coverage *= m as f32 / 255.0;
    }
    let a = ((color.a as f32 * coverage.min(1.0) + 0.5) as i32).clamp(0, 255) as u8;
    let dst = s.get_mut(x, y);
    if alpha_blend {
        if a == 0 {
            return;
        }
        *dst = over_normal(color.with_alpha(a), *dst, 255);
    } else {
        *dst = color.with_alpha(a);
    }
}

/// [`blend_pixel`] on a pixel already fetched; `sel_cov` is the selection
/// coverage at that pixel (`None` when there is no selection).
#[inline(always)]
fn blend_into(dst: &mut Bgra, color: Bgra, mut coverage: f32, sel_cov: Option<u8>, alpha_blend: bool) {
    if coverage <= 0.0 {
        return;
    }
    if let Some(m) = sel_cov {
        if m == 0 {
            return;
        }
        coverage *= m as f32 / 255.0;
    }
    let a = ((color.a as f32 * coverage.min(1.0) + 0.5) as i32).clamp(0, 255) as u8;
    if alpha_blend {
        if a != 0 {
            *dst = over_normal(color.with_alpha(a), *dst, 255);
        }
    } else {
        *dst = color.with_alpha(a);
    }
}

/// Runs `f(y, row)` for rows `y0..=y1` in parallel. Each pixel depends only
/// on its own coordinates, so row order cannot change the result.
fn par_rows(s: &mut Surface, y0: i32, y1: i32, f: impl Fn(i32, &mut [Bgra]) + Sync + Send) {
    if y1 < y0 {
        return;
    }
    let w = s.width() as usize;
    let rows = &mut s.pixels_mut()[y0 as usize * w..(y1 as usize + 1) * w];
    crate::par::for_rows(rows, w, |i, row| f(y0 + i as i32, row));
}

fn clamp_rect(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> PixelRect {
    PixelRect::from_inclusive(min_x, min_y, max_x, max_y)
}

/// Thick line between pixel centres; returns the touched bounds.
#[allow(clippy::too_many_arguments)]
pub fn line(
    s: &mut Surface,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    color: Bgra,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    let half = 0.5f32.max(width / 2.0);
    let min_x = ((x0.min(x1) - half - 1.0).floor() as i32).max(0);
    let min_y = ((y0.min(y1) - half - 1.0).floor() as i32).max(0);
    let max_x = ((x0.max(x1) + half + 1.0).ceil() as i32).min(s.width() - 1);
    let max_y = ((y0.max(y1) + half + 1.0).ceil() as i32).min(s.height() - 1);
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len2 = dx * dx + dy * dy;
    par_rows(s, min_y, max_y, |y, row| {
        for x in min_x..=max_x {
            let px = x as f32 - x0;
            let py = y as f32 - y0;
            let dist = if len2 <= 0.0001 {
                (px * px + py * py).sqrt()
            } else {
                let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
                let ex = px - t * dx;
                let ey = py - t * dy;
                (ex * ex + ey * ey).sqrt()
            };
            let cov = sharpen((half + 0.5 - dist).clamp(0.0, 1.0), aa);
            if cov > 0.0 {
                blend_into(&mut row[x as usize], color, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
            }
        }
    });
    clamp_rect(min_x, min_y, max_x, max_y)
}

/// Strokes an open polyline of float points with round joins and caps.
/// Coverage is the maximum over segments (each pixel is blended once), so
/// joins of semi-transparent strokes don't darken.
#[allow(clippy::too_many_arguments)]
pub fn stroke_path(s: &mut Surface, pts: &[(f32, f32)], width: f32, color: Bgra, sel: Option<&Selection>, aa: bool, alpha_blend: bool) -> PixelRect {
    stroke_capped(s, pts, width, color, LineCap::Round, LineCap::Round, sel, aa, alpha_blend)
}

/// How the ends of a line are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineCap {
    /// Cut off square at the end point.
    Flat,
    /// Square, extended past the end point by half the width.
    Square,
    /// Rounded (the C# port's only cap).
    #[default]
    Round,
    /// Solid arrowhead pointing outwards.
    Arrow,
    /// Open arrowhead: two spokes pointing outwards.
    ArrowOpen,
    /// Solid arrowhead pointing inwards (a "tail").
    ArrowIn,
    /// Solid diamond centred on the end point.
    Diamond,
    /// Solid circle centred on the end point.
    Circle,
}

/// Which end of a segment, and how it ends.
#[derive(Clone, Copy)]
enum SegEnd {
    Round,
    /// Flat, extended by this many pixels.
    Extend(f32),
}

/// Max-coverage accumulator over a box; shapes are unioned, then blended once.
struct CoverageBuf {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    w: usize,
    cov: Vec<f32>,
}

impl CoverageBuf {
    fn new(s: &Surface, x0: f32, y0: f32, x1: f32, y1: f32) -> Option<Self> {
        let (x0, y0) = ((x0.floor() as i32).max(0), (y0.floor() as i32).max(0));
        let (x1, y1) = ((x1.ceil() as i32).min(s.width() - 1), (y1.ceil() as i32).min(s.height() - 1));
        if x1 < x0 || y1 < y0 {
            return None;
        }
        let w = (x1 - x0 + 1) as usize;
        Some(CoverageBuf { x0, y0, x1, y1, w, cov: vec![0.0; w * (y1 - y0 + 1) as usize] })
    }

    /// Calls `f(x, y)` (pixel-index coordinates) over a sub-box, keeping the max.
    fn paint(&mut self, bx0: f32, by0: f32, bx1: f32, by1: f32, f: impl Fn(f32, f32) -> f32) {
        let xa = (bx0.floor() as i32).max(self.x0);
        let ya = (by0.floor() as i32).max(self.y0);
        let xb = (bx1.ceil() as i32).min(self.x1);
        let yb = (by1.ceil() as i32).min(self.y1);
        for y in ya..=yb {
            let row = &mut self.cov[(y - self.y0) as usize * self.w..][..self.w];
            for x in xa..=xb {
                let c = f(x as f32, y as f32).clamp(0.0, 1.0);
                let slot = &mut row[(x - self.x0) as usize];
                if c > *slot {
                    *slot = c;
                }
            }
        }
    }

    /// A stroke segment of half-width `half` with the given ends.
    fn segment(&mut self, a: (f32, f32), b: (f32, f32), half: f32, start: SegEnd, end: SegEnd) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = (dx * dx + dy * dy).sqrt();
        let pad = half + 1.0 + match (start, end) {
            (SegEnd::Extend(e), _) | (_, SegEnd::Extend(e)) => e.max(0.0),
            _ => 0.0,
        };
        let (ux, uy) = if len > 1e-4 { (dx / len, dy / len) } else { (1.0, 0.0) };
        self.paint(a.0.min(b.0) - pad, a.1.min(b.1) - pad, a.0.max(b.0) + pad, a.1.max(b.1) + pad, |x, y| {
            let (px, py) = (x - a.0, y - a.1);
            let along = px * ux + py * uy;
            let perp = (px * uy - py * ux).abs();
            let mut c = half + 0.5 - perp;
            match start {
                SegEnd::Extend(e) => c = c.min(along + e + 0.5),
                SegEnd::Round if along < 0.0 => c = half + 0.5 - (px * px + py * py).sqrt(),
                SegEnd::Round => {}
            }
            match end {
                SegEnd::Extend(e) => c = c.min(len - along + e + 0.5),
                SegEnd::Round if along > len => c = c.min(half + 0.5 - ((x - b.0).powi(2) + (y - b.1).powi(2)).sqrt()),
                SegEnd::Round => {}
            }
            if len <= 1e-4 {
                if let (SegEnd::Round, SegEnd::Round) = (start, end) {
                    c = half + 0.5 - (px * px + py * py).sqrt();
                }
            }
            c
        });
    }

    /// A filled convex polygon (any winding).
    fn convex(&mut self, pts: &[(f32, f32)]) {
        let n = pts.len();
        if n < 3 {
            return;
        }
        let area: f32 = (0..n).map(|i| pts[i].0 * pts[(i + 1) % n].1 - pts[(i + 1) % n].0 * pts[i].1).sum();
        let sign = if area >= 0.0 { 1.0 } else { -1.0 };
        let edges: Vec<(f32, f32, f32)> = (0..n)
            .map(|i| {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                let (ex, ey) = (b.0 - a.0, b.1 - a.1);
                let l = (ex * ex + ey * ey).sqrt().max(1e-6);
                // Outward normal for this winding.
                let (nx, ny) = (sign * ey / l, -sign * ex / l);
                (nx, ny, -(nx * a.0 + ny * a.1))
            })
            .collect();
        let fold = |f: fn(f32, f32) -> f32, k: fn(&(f32, f32)) -> f32, init| pts.iter().map(k).fold(init, f);
        let (bx0, by0) = (fold(f32::min, |p| p.0, f32::MAX) - 1.0, fold(f32::min, |p| p.1, f32::MAX) - 1.0);
        let (bx1, by1) = (fold(f32::max, |p| p.0, f32::MIN) + 1.0, fold(f32::max, |p| p.1, f32::MIN) + 1.0);
        self.paint(bx0, by0, bx1, by1, |x, y| {
            let d = edges.iter().map(|&(nx, ny, c)| nx * x + ny * y + c).fold(f32::MIN, f32::max);
            0.5 - d
        });
    }

    fn disc(&mut self, c: (f32, f32), r: f32) {
        self.paint(c.0 - r - 1.0, c.1 - r - 1.0, c.0 + r + 1.0, c.1 + r + 1.0, |x, y| r + 0.5 - ((x - c.0).powi(2) + (y - c.1).powi(2)).sqrt());
    }

    fn blend(self, s: &mut Surface, color: Bgra, sel: Option<&Selection>, aa: bool, alpha_blend: bool) -> PixelRect {
        let (x0, x1, y0, w) = (self.x0, self.x1, self.y0, self.w);
        let cov = &self.cov;
        par_rows(s, self.y0, self.y1, |y, row| {
            let crow = &cov[(y - y0) as usize * w..][..w];
            for x in x0..=x1 {
                let c = sharpen(crow[(x - x0) as usize], aa);
                if c > 0.0 {
                    blend_into(&mut row[x as usize], color, c, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
        });
        clamp_rect(self.x0, self.y0, self.x1, self.y1)
    }
}

/// Arrowhead length and half-width for a line width.
fn head_size(width: f32) -> (f32, f32) {
    ((width * 3.0).max(9.0), (width * 1.6).max(4.5))
}

/// Shortens a polyline by `d` pixels from its end (or start, if `from_start`).
fn trim(pts: &mut Vec<(f32, f32)>, mut d: f32, from_start: bool) {
    if from_start {
        pts.reverse();
    }
    while d > 0.0 && pts.len() >= 2 {
        let (b, a) = (pts[pts.len() - 1], pts[pts.len() - 2]);
        let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if l > d {
            let t = (l - d) / l;
            *pts.last_mut().unwrap() = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            break;
        }
        d -= l;
        pts.pop();
    }
    if from_start {
        pts.reverse();
    }
}

/// Line dash patterns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineDash {
    #[default]
    Solid,
    Dash,
    LongDash,
    Dot,
    DenseDot,
    DashDot,
    DashDotDot,
}

impl LineDash {
    /// Alternating on/off lengths in multiples of the line width (an "on" of
    /// 0 is a round dot).
    fn pattern(self) -> &'static [f32] {
        match self {
            LineDash::Solid => &[],
            LineDash::Dash => &[3.0, 2.0],
            LineDash::LongDash => &[7.0, 3.0],
            LineDash::Dot => &[0.0, 2.5],
            LineDash::DenseDot => &[0.0, 1.6],
            LineDash::DashDot => &[4.0, 2.0, 0.0, 2.0],
            LineDash::DashDotDot => &[4.0, 2.0, 0.0, 2.0, 0.0, 2.0],
        }
    }
}

/// Splits a polyline into its dashes: (sub-polylines, dot centres).
fn dash_split(p: &[(f32, f32)], pattern: &[f32], unit: f32) -> (Vec<Vec<(f32, f32)>>, Vec<(f32, f32)>) {
    let (mut dashes, mut dots) = (Vec::new(), Vec::new());
    let mut idx = 0usize; // pattern position
    let mut left = pattern[0] * unit; // length left in the current interval
    let mut on = true;
    let mut cur: Vec<(f32, f32)> = Vec::new();
    if left <= 0.0 {
        dots.push(p[0]);
    } else {
        cur.push(p[0]);
    }
    for w in p.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let mut t = 0.0f32;
        while seg - t > left {
            t += left;
            let q = (a.0 + (b.0 - a.0) * t / seg, a.1 + (b.1 - a.1) * t / seg);
            if on && !cur.is_empty() {
                cur.push(q);
                dashes.push(std::mem::take(&mut cur));
            }
            idx = (idx + 1) % pattern.len();
            on = !on;
            left = pattern[idx] * unit;
            if on {
                if left <= 0.0 {
                    dots.push(q);
                } else {
                    cur.push(q);
                }
            }
            if left <= 0.0 && on {
                // A dot occupies no length: move straight on to the gap.
                idx = (idx + 1) % pattern.len();
                on = false;
                left = pattern[idx] * unit;
            }
        }
        left -= seg - t;
        if on && !cur.is_empty() {
            cur.push(b);
        }
    }
    if cur.len() >= 2 {
        dashes.push(cur);
    }
    (dashes, dots)
}

/// Strokes a polyline with the given end caps (joins are round).
#[allow(clippy::too_many_arguments)]
pub fn stroke_capped(
    s: &mut Surface,
    pts: &[(f32, f32)],
    width: f32,
    color: Bgra,
    start: LineCap,
    end: LineCap,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    stroke_styled(s, pts, width, color, start, end, LineDash::Solid, sel, aa, alpha_blend)
}

/// Strokes a polyline with end caps and a dash pattern.
#[allow(clippy::too_many_arguments)]
pub fn stroke_styled(
    s: &mut Surface,
    pts: &[(f32, f32)],
    width: f32,
    color: Bgra,
    start: LineCap,
    end: LineCap,
    dash: LineDash,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    let mut p: Vec<(f32, f32)> = Vec::with_capacity(pts.len());
    for &q in pts {
        if p.last().is_none_or(|l: &(f32, f32)| (l.0 - q.0).abs() > 1e-3 || (l.1 - q.1).abs() > 1e-3) {
            p.push(q);
        }
    }
    if p.is_empty() {
        return PixelRect::EMPTY;
    }
    let half = 0.5f32.max(width / 2.0);
    let (hl, hw) = head_size(width);
    let margin = hl.max(hw) + half + 2.0;
    let fold = |f: fn(f32, f32) -> f32, k: fn(&(f32, f32)) -> f32, init| p.iter().map(k).fold(init, f);
    let Some(mut buf) = CoverageBuf::new(
        s,
        fold(f32::min, |q| q.0, f32::MAX) - margin,
        fold(f32::min, |q| q.1, f32::MAX) - margin,
        fold(f32::max, |q| q.0, f32::MIN) + margin,
        fold(f32::max, |q| q.1, f32::MIN) + margin,
    ) else {
        return PixelRect::EMPTY;
    };
    if p.len() == 1 {
        buf.disc(p[0], half);
        return buf.blend(s, color, sel, aa, alpha_blend);
    }
    let total: f32 = p.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum();
    // Unit direction pointing out of the line at each end.
    let unit = |a: (f32, f32), b: (f32, f32)| {
        let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1e-6);
        ((b.0 - a.0) / l, (b.1 - a.1) / l)
    };
    let out_start = unit(p[1], p[0]);
    let out_end = unit(p[p.len() - 2], p[p.len() - 1]);
    let (tip_start, tip_end) = (p[0], p[p.len() - 1]);
    // Pull the stroke back under solid heads so it doesn't poke through the tip.
    let pull = |cap: LineCap| match cap {
        LineCap::Arrow => hl * 0.8,
        _ => 0.0,
    };
    let (ps, pe) = (pull(start).min(total * 0.45), pull(end).min(total * 0.45));
    let mut body = p.clone();
    trim(&mut body, pe, false);
    trim(&mut body, ps, true);
    let seg_end = |cap: LineCap| match cap {
        LineCap::Round | LineCap::ArrowOpen => SegEnd::Round,
        LineCap::Square => SegEnd::Extend(half),
        _ => SegEnd::Extend(0.0),
    };
    let pattern = dash.pattern();
    if pattern.is_empty() || body.len() < 2 {
        let n = body.len();
        for i in 0..n.saturating_sub(1) {
            let a = if i == 0 { seg_end(start) } else { SegEnd::Round };
            let b = if i == n - 2 { seg_end(end) } else { SegEnd::Round };
            buf.segment(body[i], body[i + 1], half, a, b);
        }
    } else {
        // Dash ends follow the start cap's style (arrows etc. count as flat).
        let de = match start {
            LineCap::Round => SegEnd::Round,
            LineCap::Square => SegEnd::Extend(half),
            _ => SegEnd::Extend(0.0),
        };
        let (dashes, dots) = dash_split(&body, pattern, width.max(1.0));
        for d in dashes {
            let n = d.len();
            for i in 0..n - 1 {
                let a = if i == 0 { de } else { SegEnd::Round };
                let b = if i == n - 2 { de } else { SegEnd::Round };
                buf.segment(d[i], d[i + 1], half, a, b);
            }
        }
        for c in dots {
            buf.disc(c, half);
        }
    }
    for (cap, tip, (ux, uy)) in [(start, tip_start, out_start), (end, tip_end, out_end)] {
        let (vx, vy) = (-uy, ux);
        match cap {
            LineCap::Arrow => {
                let base = (tip.0 - ux * hl, tip.1 - uy * hl);
                buf.convex(&[tip, (base.0 + vx * hw, base.1 + vy * hw), (base.0 - vx * hw, base.1 - vy * hw)]);
            }
            LineCap::ArrowIn => {
                let point = (tip.0 - ux * hl, tip.1 - uy * hl);
                buf.convex(&[point, (tip.0 + vx * hw, tip.1 + vy * hw), (tip.0 - vx * hw, tip.1 - vy * hw)]);
            }
            LineCap::ArrowOpen => {
                for side in [1.0, -1.0] {
                    let end = (tip.0 - ux * hl + side * vx * hw, tip.1 - uy * hl + side * vy * hw);
                    buf.segment(tip, end, half, SegEnd::Round, SegEnd::Round);
                }
            }
            LineCap::Diamond => {
                let (a, b) = (hw * 1.25, hw);
                buf.convex(&[(tip.0 + ux * a, tip.1 + uy * a), (tip.0 + vx * b, tip.1 + vy * b), (tip.0 - ux * a, tip.1 - uy * a), (tip.0 - vx * b, tip.1 - vy * b)]);
            }
            LineCap::Circle => buf.disc(tip, hw),
            _ => {}
        }
    }
    buf.blend(s, color, sel, aa, alpha_blend)
}

/// Fill patterns for shapes, freeform shapes and the paint bucket: the fill
/// colour where the pattern is on, the image untouched where it is off.
/// Patterns are aligned to the image grid, so neighbouring fills line up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillStyle {
    #[default]
    Solid,
    // Lines
    Horizontal,
    Vertical,
    ForwardDiagonal,
    BackwardDiagonal,
    LightHorizontal,
    LightVertical,
    DarkHorizontal,
    DarkVertical,
    DashedHorizontal,
    Waves,
    ZigZag,
    // Grids
    Cross,
    SmallGrid,
    LargeGrid,
    DiagonalCross,
    DenseCrosshatch,
    Diamonds,
    Bricks,
    Plaid,
    // Dots and checks
    Dots,
    SparseDots,
    LargeDots,
    Checkerboard,
    SmallCheckerboard,
    LargeCheckerboard,
    // Ordered (Bayer 4×4) dithers, by coverage
    Dither6,
    Dither12,
    Dither25,
    Dither37,
    Percent50,
    Dither62,
    Dither75,
    Dither87,
    Dither94,
    // Random (noise) dithers, stable per pixel
    Noise25,
    Noise50,
    Noise75,
}

const BAYER4: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

impl FillStyle {
    pub const ALL: [FillStyle; 38] = [
        FillStyle::Solid,
        FillStyle::Horizontal,
        FillStyle::Vertical,
        FillStyle::ForwardDiagonal,
        FillStyle::BackwardDiagonal,
        FillStyle::LightHorizontal,
        FillStyle::LightVertical,
        FillStyle::DarkHorizontal,
        FillStyle::DarkVertical,
        FillStyle::DashedHorizontal,
        FillStyle::Waves,
        FillStyle::ZigZag,
        FillStyle::Cross,
        FillStyle::SmallGrid,
        FillStyle::LargeGrid,
        FillStyle::DiagonalCross,
        FillStyle::DenseCrosshatch,
        FillStyle::Diamonds,
        FillStyle::Bricks,
        FillStyle::Plaid,
        FillStyle::Dots,
        FillStyle::SparseDots,
        FillStyle::LargeDots,
        FillStyle::Checkerboard,
        FillStyle::SmallCheckerboard,
        FillStyle::LargeCheckerboard,
        FillStyle::Dither6,
        FillStyle::Dither12,
        FillStyle::Dither25,
        FillStyle::Dither37,
        FillStyle::Percent50,
        FillStyle::Dither62,
        FillStyle::Dither75,
        FillStyle::Dither87,
        FillStyle::Dither94,
        FillStyle::Noise25,
        FillStyle::Noise50,
        FillStyle::Noise75,
    ];

    pub fn name(self) -> String {
        format!("{self:?}")
    }

    pub fn parse(s: &str) -> Option<FillStyle> {
        Self::ALL.iter().copied().find(|f| f.name() == s)
    }

    #[inline]
    pub fn on(self, x: i32, y: i32) -> bool {
        let (mx, my) = (x.rem_euclid(8), y.rem_euclid(8));
        let bayer = |n: u8| BAYER4[y.rem_euclid(4) as usize][x.rem_euclid(4) as usize] < n;
        let noise = |n: u32| {
            // Integer hash of the pixel position: the same "random" dots every time.
            let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
            h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
            ((h ^ (h >> 16)) & 255) < n
        };
        match self {
            FillStyle::Solid => true,
            FillStyle::Horizontal => my < 2,
            FillStyle::Vertical => mx < 2,
            FillStyle::ForwardDiagonal => (x + y).rem_euclid(8) < 2,
            FillStyle::BackwardDiagonal => (x - y).rem_euclid(8) < 2,
            FillStyle::LightHorizontal => y.rem_euclid(4) == 0,
            FillStyle::LightVertical => x.rem_euclid(4) == 0,
            FillStyle::DarkHorizontal => y.rem_euclid(4) < 3,
            FillStyle::DarkVertical => x.rem_euclid(4) < 3,
            FillStyle::DashedHorizontal => my < 2 && (x + if y.rem_euclid(16) < 8 { 0 } else { 4 }).rem_euclid(8) < 5,
            FillStyle::Waves => {
                let k = [0, 1, 2, 3, 3, 2, 1, 0][mx as usize];
                my == k || my == k + 1
            }
            FillStyle::ZigZag => {
                let k = (2 * mx - 8).abs() % 8; // steep zigzag, period 8
                my == k || my == (k + 1) % 8
            }
            FillStyle::Cross => mx < 2 || my < 2,
            FillStyle::SmallGrid => x.rem_euclid(4) == 0 || y.rem_euclid(4) == 0,
            FillStyle::LargeGrid => x.rem_euclid(16) == 0 || y.rem_euclid(16) == 0,
            FillStyle::DiagonalCross => (x + y).rem_euclid(8) < 2 || (x - y).rem_euclid(8) < 2,
            FillStyle::DenseCrosshatch => (x + y).rem_euclid(4) == 0 || (x - y).rem_euclid(4) == 0,
            FillStyle::Diamonds => (mx - 4).abs() + (my - 4).abs() == 4,
            FillStyle::Bricks => my < 1 || (if y.rem_euclid(16) < 8 { mx } else { (x + 4).rem_euclid(8) }) < 1,
            FillStyle::Plaid => ((mx < 3 || my < 3) && (x + y).rem_euclid(2) == 0) || (mx < 3 && my < 3),
            FillStyle::Dots => x.rem_euclid(6) < 2 && y.rem_euclid(6) < 2,
            FillStyle::SparseDots => x.rem_euclid(4) == 0 && y.rem_euclid(4) == 0 && (x.div_euclid(4) + y.div_euclid(4)).rem_euclid(2) == 0,
            FillStyle::LargeDots => {
                let (dx, dy) = (mx as f32 - 3.5, my as f32 - 3.5);
                dx * dx + dy * dy <= 6.5
            }
            FillStyle::Checkerboard => (x.div_euclid(4) + y.div_euclid(4)) % 2 == 0,
            FillStyle::SmallCheckerboard => (x.div_euclid(2) + y.div_euclid(2)) % 2 == 0,
            FillStyle::LargeCheckerboard => (x.div_euclid(8) + y.div_euclid(8)) % 2 == 0,
            FillStyle::Dither6 => bayer(1),
            FillStyle::Dither12 => bayer(2),
            FillStyle::Dither25 => bayer(4),
            FillStyle::Dither37 => bayer(6),
            FillStyle::Percent50 => (x + y).rem_euclid(2) == 0,
            FillStyle::Dither62 => bayer(10),
            FillStyle::Dither75 => bayer(12),
            FillStyle::Dither87 => bayer(14),
            FillStyle::Dither94 => bayer(15),
            FillStyle::Noise25 => noise(64),
            FillStyle::Noise50 => noise(128),
            FillStyle::Noise75 => noise(192),
        }
    }
}

/// Fills and/or outlines closed polygons (pixel-edge coordinates; even-odd).
/// The outline lies inside the polygon's edge, so corners stay sharp and the
/// shape keeps the size it was drawn at.
#[allow(clippy::too_many_arguments)]
pub fn polygon_shape(
    s: &mut Surface,
    contours: &[Vec<(f32, f32)>],
    details: &[Vec<(f32, f32)>],
    fill: Option<Bgra>,
    outline: Option<Bgra>,
    pen_width: f32,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    let pts: Vec<(f32, f32)> = contours.iter().flatten().copied().collect();
    if pts.len() < 3 {
        return PixelRect::EMPTY;
    }
    let half = 0.5f32.max(pen_width / 2.0);
    let fold = |f: fn(f32, f32) -> f32, k: fn(&(f32, f32)) -> f32, init| pts.iter().map(k).fold(init, f);
    let x0 = ((fold(f32::min, |p| p.0, f32::MAX) - 1.0).floor() as i32).max(0);
    let y0 = ((fold(f32::min, |p| p.1, f32::MAX) - 1.0).floor() as i32).max(0);
    let x1 = ((fold(f32::max, |p| p.0, f32::MIN) + 1.0).ceil() as i32).min(s.width() - 1);
    let y1 = ((fold(f32::max, |p| p.1, f32::MIN) + 1.0).ceil() as i32).min(s.height() - 1);
    if x1 < x0 || y1 < y0 {
        return PixelRect::EMPTY;
    }
    let (bw, bh) = ((x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let edges: Vec<((f32, f32), (f32, f32))> = contours.iter().filter(|c| c.len() >= 2).flat_map(|c| (0..c.len()).map(move |i| (c[i], c[(i + 1) % c.len()]))).collect();
    // Inside (even-odd) at pixel centres, by scanline crossings.
    let mut inside = vec![false; bw * bh];
    let mut xs: Vec<f32> = Vec::new();
    for row in 0..bh {
        let yc = (y0 + row as i32) as f32 + 0.5;
        xs.clear();
        for &(a, b) in &edges {
            if (a.1 <= yc) != (b.1 <= yc) {
                xs.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            let from = ((pair[0] - 0.5).ceil() as i32).max(x0);
            let to = ((pair[1] - 0.5).floor() as i32).min(x1);
            for x in from..=to {
                inside[row * bw + (x - x0) as usize] = true;
            }
        }
    }
    // Exact distance to the nearest edge, only near edges.
    let band = if outline.is_some() { 2.0 * half + 1.5 } else { 1.5 };
    let mut dist = vec![f32::MAX; bw * bh];
    for &(a, b) in &edges {
        let ex0 = ((a.0.min(b.0) - band).floor() as i32).max(x0);
        let ey0 = ((a.1.min(b.1) - band).floor() as i32).max(y0);
        let ex1 = ((a.0.max(b.0) + band).ceil() as i32).min(x1);
        let ey1 = ((a.1.max(b.1) + band).ceil() as i32).min(y1);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = (dx * dx + dy * dy).max(1e-9);
        for y in ey0..=ey1 {
            let row = &mut dist[(y - y0) as usize * bw..][..bw];
            for x in ex0..=ex1 {
                let (px, py) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
                let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
                let d = ((px - t * dx).powi(2) + (py - t * dy).powi(2)).sqrt();
                let slot = &mut row[(x - x0) as usize];
                if d < *slot {
                    *slot = d;
                }
            }
        }
    }
    // Detail lines (outline colour), clipped to the inside of the shape.
    let mut detail = None;
    if outline.is_some() && !details.is_empty() {
        if let Some(mut buf) = CoverageBuf::new(s, x0 as f32, y0 as f32, x1 as f32, y1 as f32) {
            for d in details {
                let n = d.len();
                for (i, w) in d.windows(2).enumerate() {
                    // Pixel-edge → pixel-index coordinates; flat ends, round joins.
                    let (a, b) = ((w[0].0 - 0.5, w[0].1 - 0.5), (w[1].0 - 0.5, w[1].1 - 0.5));
                    let ea = if i == 0 { SegEnd::Extend(0.0) } else { SegEnd::Round };
                    let eb = if i + 2 == n { SegEnd::Extend(0.0) } else { SegEnd::Round };
                    buf.segment(a, b, half, ea, eb);
                }
            }
            detail = Some(buf);
        }
    }
    let detail_at = |x: i32, y: i32| detail.as_ref().map_or(0.0, |b| b.cov[(y - b.y0) as usize * b.w + (x - b.x0) as usize]);
    let (inside, dist) = (&inside, &dist);
    par_rows(s, y0, y1, |y, row| {
        let o = (y - y0) as usize * bw;
        for x in x0..=x1 {
            let i = o + (x - x0) as usize;
            let d = if inside[i] { -dist[i] } else { dist[i] };
            if let Some(fc) = fill {
                let cov = sharpen((0.5 - d).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], fc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
            if let Some(oc) = outline {
                let edge = (0.5 - d).min(d + 2.0 * half + 0.5);
                let inner = detail_at(x, y) * (0.5 - d).clamp(0.0, 1.0);
                let cov = sharpen(edge.max(inner).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], oc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
        }
    });
    clamp_rect(x0, y0, x1, y1)
}

/// The kinds of box-based shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxShape {
    Rectangle,
    RoundedRectangle(f32),
    Ellipse,
}

/// A rectangle, rounded rectangle or ellipse of size w×h centred on (cx, cy)
/// (pixel-edge coordinates, as `PixelRect`), rotated by `angle` radians.
/// Outlines stay inside the box, like the unrotated shapes.
#[allow(clippy::too_many_arguments)]
pub fn rotated_shape(
    s: &mut Surface,
    shape: BoxShape,
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    angle: f32,
    fill: Option<Bgra>,
    outline: Option<Bgra>,
    pen_width: f32,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    if w <= 0.0 || h <= 0.0 {
        return PixelRect::EMPTY;
    }
    let half = 0.5f32.max(pen_width / 2.0);
    // Outline inside the edge (see `ellipse`).
    let (hw, hh) = ((w / 2.0).max(0.0), (h / 2.0).max(0.0));
    let (sin, cos) = angle.sin_cos();
    let ext_x = (w / 2.0 * cos).abs() + (h / 2.0 * sin).abs() + 2.0;
    let ext_y = (w / 2.0 * sin).abs() + (h / 2.0 * cos).abs() + 2.0;
    let x0 = ((cx - ext_x).floor() as i32).max(0);
    let y0 = ((cy - ext_y).floor() as i32).max(0);
    let x1 = ((cx + ext_x).ceil() as i32).min(s.width() - 1);
    let y1 = ((cy + ext_y).ceil() as i32).min(s.height() - 1);
    if x1 < x0 || y1 < y0 {
        return PixelRect::EMPTY;
    }
    let dist = move |x: i32, y: i32| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        // Into the shape's own (unrotated) frame.
        let (u, v) = (dx * cos + dy * sin, -dx * sin + dy * cos);
        match shape {
            BoxShape::Ellipse => ellipse_distance(u, v, hw.max(0.5), hh.max(0.5)),
            BoxShape::Rectangle | BoxShape::RoundedRectangle(_) => {
                let rad = if let BoxShape::RoundedRectangle(r) = shape { r.clamp(0.0, hw.min(hh)) } else { 0.0 };
                let qx = u.abs() - (hw - rad);
                let qy = v.abs() - (hh - rad);
                let (ox, oy) = (qx.max(0.0), qy.max(0.0));
                (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - rad
            }
        }
    };
    par_rows(s, y0, y1, |y, row| {
        for x in x0..=x1 {
            let d = dist(x, y);
            if let Some(fc) = fill {
                let cov = sharpen((0.5 - d).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], fc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
            if let Some(oc) = outline {
                let cov = sharpen((0.5 - d).min(d + 2.0 * half + 0.5).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], oc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
        }
    });
    clamp_rect(x0, y0, x1, y1)
}

pub fn fill_rect(s: &mut Surface, r: PixelRect, color: Bgra, sel: Option<&Selection>, alpha_blend: bool) -> PixelRect {
    let x0 = r.x.max(0);
    let y0 = r.y.max(0);
    let x1 = s.width().min(r.right());
    let y1 = s.height().min(r.bottom());
    if sel.is_none() && (!alpha_blend || color.a == 255) {
        // Opaque or overwrite fill without a selection is a plain span fill.
        let c = color;
        for y in y0..y1 {
            s.row_mut(y)[x0 as usize..x1 as usize].fill(c);
        }
    } else {
        for y in y0..y1 {
            for x in x0..x1 {
                blend_pixel(s, x, y, color, 1.0, sel, alpha_blend);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 { PixelRect::EMPTY } else { PixelRect::new(x0, y0, x1 - x0, y1 - y0) }
}

pub fn rect_outline(s: &mut Surface, r: PixelRect, pen_width: i32, color: Bgra, sel: Option<&Selection>, alpha_blend: bool) -> PixelRect {
    let pen_width = pen_width.max(1);
    let w = pen_width.min(r.width.min(r.height));
    fill_rect(s, PixelRect::new(r.x, r.y, r.width, w), color, sel, alpha_blend);
    if r.height > w {
        fill_rect(s, PixelRect::new(r.x, r.bottom() - w, r.width, w), color, sel, alpha_blend);
    }
    if r.height > 2 * w {
        fill_rect(s, PixelRect::new(r.x, r.y + w, w, r.height - 2 * w), color, sel, alpha_blend);
        fill_rect(s, PixelRect::new(r.right() - w, r.y + w, w, r.height - 2 * w), color, sel, alpha_blend);
    }
    clamp_rect(r.x.max(0), r.y.max(0), (r.right() - 1).min(s.width() - 1), (r.bottom() - 1).min(s.height() - 1))
}

/// Approximate signed distance from (dx, dy) to an axis-aligned ellipse.
#[inline]
pub(crate) fn ellipse_distance(dx: f32, dy: f32, a: f32, b: f32) -> f32 {
    let a = a.max(0.5);
    let b = b.max(0.5);
    let g = (dx * dx / (a * a) + dy * dy / (b * b)).sqrt();
    if g < 1e-6 {
        return -a.min(b);
    }
    let gx = dx / (a * a);
    let gy = dy / (b * b);
    let grad = (gx * gx + gy * gy).sqrt() / g;
    if grad < 1e-9 { 0.0 } else { (g - 1.0) / grad }
}

#[allow(clippy::too_many_arguments)]
pub fn ellipse(
    s: &mut Surface,
    r: PixelRect,
    fill: Option<Bgra>,
    outline: Option<Bgra>,
    pen_width: f32,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    if r.width <= 0 || r.height <= 0 {
        return PixelRect::EMPTY;
    }
    let half = 0.5f32.max(pen_width / 2.0);
    // The outline lies inside the shape's edge (the band from the edge inwards
    // by the pen width), so any pen width stays inside the box; very thick
    // pens simply fill the shape.
    let a = (r.width as f32 / 2.0).max(0.5);
    let b = (r.height as f32 / 2.0).max(0.5);
    let cx = r.x as f32 + r.width as f32 / 2.0;
    let cy = r.y as f32 + r.height as f32 / 2.0;
    let x0 = (r.x - 1).max(0);
    let y0 = (r.y - 1).max(0);
    let x1 = (s.width() - 1).min(r.right() + 1);
    let y1 = (s.height() - 1).min(r.bottom() + 1);
    par_rows(s, y0, y1, |y, row| {
        for x in x0..=x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let dist = ellipse_distance(dx, dy, a, b);
            if let Some(fc) = fill {
                let cov = sharpen((0.5 - dist).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], fc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
            if let Some(oc) = outline {
                let cov = sharpen((0.5 - dist).min(dist + 2.0 * half + 0.5).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], oc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
        }
    });
    clamp_rect(x0, y0, x1, y1)
}

#[allow(clippy::too_many_arguments)]
pub fn rounded_rect(
    s: &mut Surface,
    r: PixelRect,
    corner_radius: f32,
    fill: Option<Bgra>,
    outline: Option<Bgra>,
    pen_width: f32,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    if r.width <= 0 || r.height <= 0 {
        return PixelRect::EMPTY;
    }
    let half = 0.5f32.max(pen_width / 2.0);
    // Outline inside the edge (see `ellipse`).
    let hw = (r.width as f32 / 2.0).max(0.0);
    let hh = (r.height as f32 / 2.0).max(0.0);
    let rad = corner_radius.clamp(0.0, hw.min(hh));
    let cx = r.x as f32 + r.width as f32 / 2.0;
    let cy = r.y as f32 + r.height as f32 / 2.0;
    let x0 = (r.x - 1).max(0);
    let y0 = (r.y - 1).max(0);
    let x1 = (s.width() - 1).min(r.right() + 1);
    let y1 = (s.height() - 1).min(r.bottom() + 1);
    par_rows(s, y0, y1, |y, row| {
        for x in x0..=x1 {
            let qx = (x as f32 + 0.5 - cx).abs() - (hw - rad);
            let qy = (y as f32 + 0.5 - cy).abs() - (hh - rad);
            let ox = qx.max(0.0);
            let oy = qy.max(0.0);
            let dist = (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - rad;
            if let Some(fc) = fill {
                let cov = sharpen((0.5 - dist).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], fc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
            if let Some(oc) = outline {
                let cov = sharpen((0.5 - dist).min(dist + 2.0 * half + 0.5).clamp(0.0, 1.0), aa);
                if cov > 0.0 {
                    blend_into(&mut row[x as usize], oc, cov, sel.map(|m| m.coverage_at(x, y)), alpha_blend);
                }
            }
        }
    });
    clamp_rect(x0, y0, x1, y1)
}

pub fn fill_polygon(s: &mut Surface, points: &[(i32, i32)], color: Bgra, sel: Option<&Selection>, alpha_blend: bool) -> PixelRect {
    if points.len() < 3 {
        return PixelRect::EMPTY;
    }
    let min_y = points.iter().map(|p| p.1).min().unwrap();
    let max_y = points.iter().map(|p| p.1).max().unwrap();
    let y0 = min_y.max(0);
    let y1 = (s.height() - 1).min(max_y);
    let mut xs = Vec::new();
    let (mut min_x, mut max_x) = (i32::MAX, i32::MIN);
    for y in y0..=y1 {
        polygon_crossings_int(points, y, &mut xs);
        let mut i = 0;
        while i + 1 < xs.len() {
            let sx = xs[i].max(0);
            let ex = (s.width() - 1).min(xs[i + 1]);
            for x in sx..=ex {
                blend_pixel(s, x, y, color, 1.0, sel, alpha_blend);
            }
            if ex >= sx {
                min_x = min_x.min(sx);
                max_x = max_x.max(ex);
            }
            i += 2;
        }
    }
    if min_x > max_x { PixelRect::EMPTY } else { PixelRect::new(min_x, y0, max_x - min_x + 1, y1 - y0 + 1) }
}

#[allow(clippy::too_many_arguments)]
pub fn polyline(
    s: &mut Surface,
    points: &[(i32, i32)],
    width: f32,
    color: Bgra,
    closed: bool,
    sel: Option<&Selection>,
    aa: bool,
    alpha_blend: bool,
) -> PixelRect {
    match points.len() {
        0 => PixelRect::EMPTY,
        1 => {
            let (x, y) = (points[0].0 as f32, points[0].1 as f32);
            line(s, x, y, x, y, width, color, sel, aa, alpha_blend)
        }
        n => {
            let last = if closed { n } else { n - 1 };
            let mut total = PixelRect::EMPTY;
            for i in 0..last {
                let a = points[i];
                let b = points[(i + 1) % n];
                let r = line(s, a.0 as f32, a.1 as f32, b.0 as f32, b.1 as f32, width, color, sel, aa, alpha_blend);
                total = total.union(&r);
            }
            total
        }
    }
}
