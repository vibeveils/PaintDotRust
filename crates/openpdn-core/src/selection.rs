//! Antialiased selection masks (C# `Selection`).

use std::borrow::Cow;
use std::collections::VecDeque;
use std::sync::Arc;

use crate::color::Bgra;
use crate::draw::ellipse_distance;
use crate::num::round_even;
use crate::rect::{PixelRect, RectF};
use crate::resample;
use crate::surface::Surface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionShape {
    Rectangle,
    Ellipse,
    Polygon,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectionCombineMode {
    #[default]
    Replace,
    Union,
    Subtract,
    Intersect,
    /// Exclusive or: selects what is in exactly one of the two.
    Xor,
}

/// Per-pixel coverage mask (0 = unselected, 255 = fully selected) plus cached
/// bounds and, for lasso selections, the polygon outline.
#[derive(Clone, Debug)]
pub struct Selection {
    width: i32,
    height: i32,
    mask: Vec<u8>,
    bounds: PixelRect,
    shape: SelectionShape,
    outline: Option<Arc<Vec<(i32, i32)>>>,
    /// "Pixelated" selection quality: edits are clipped to a hard-edged
    /// version of the mask (coverage ≥ 50% counts as selected). The
    /// antialiased mask is kept, so switching back restores soft edges.
    pixelated: bool,
}

impl Selection {
    pub fn new(width: i32, height: i32) -> Selection {
        assert!(width > 0 && height > 0);
        Selection {
            width,
            height,
            mask: vec![0; width as usize * height as usize],
            bounds: PixelRect::EMPTY,
            shape: SelectionShape::Rectangle,
            outline: None,
            pixelated: false,
        }
    }

    /// A clip mask for patterned fills: `style` within `area`, multiplied by
    /// `base` (the current selection), if any.
    pub fn pattern_clip(width: i32, height: i32, area: PixelRect, style: crate::draw::FillStyle, base: Option<&Selection>) -> Selection {
        let mut s = Selection::new(width, height);
        s.shape = SelectionShape::Mask;
        let area = area.intersect(&PixelRect::new(0, 0, width, height));
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if style.on(x, y) {
                    let c = base.map_or(255, |b| b.coverage_at(x, y));
                    s.mask[(y * width + x) as usize] = c;
                }
            }
        }
        s.bounds = area;
        s.pixelated = base.is_some_and(|b| b.pixelated);
        s
    }

    pub fn is_pixelated(&self) -> bool {
        self.pixelated
    }

    pub fn set_pixelated(&mut self, on: bool) {
        self.pixelated = on;
    }

    /// The mask edits are clipped to: the stored mask, or its hard-edged
    /// version when the selection quality is pixelated.
    pub fn clip_mask(&self) -> Cow<'_, [u8]> {
        if self.pixelated {
            Cow::Owned(self.mask.iter().map(|&c| if c >= 128 { 255 } else { 0 }).collect())
        } else {
            Cow::Borrowed(&self.mask)
        }
    }

    pub fn width(&self) -> i32 {
        self.width
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    pub fn bounds(&self) -> PixelRect {
        self.bounds
    }

    pub fn shape(&self) -> SelectionShape {
        self.shape
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    pub fn outline(&self) -> Option<&[(i32, i32)]> {
        self.outline.as_deref().map(|v| v.as_slice())
    }

    #[inline]
    pub fn mask(&self) -> &[u8] {
        &self.mask
    }

    #[inline(always)]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.coverage_at(x, y) != 0
    }

    #[inline(always)]
    pub fn coverage_at(&self, x: i32, y: i32) -> u8 {
        if (x as u32) >= self.width as u32 || (y as u32) >= self.height as u32 {
            return 0;
        }
        let c = self.mask[y as usize * self.width as usize + x as usize];
        if self.pixelated {
            if c >= 128 { 255 } else { 0 }
        } else {
            c
        }
    }

    fn reset(&mut self, shape: SelectionShape) {
        self.mask.fill(0);
        self.shape = shape;
        self.outline = None;
    }

    /// Box blur of the mask with the given radius.
    pub fn feather(&mut self, radius: i32) {
        if radius <= 0 || self.is_empty() {
            return;
        }
        self.shape = SelectionShape::Mask;
        self.outline = None;
        let (w, h) = (self.width as usize, self.height as usize);
        let r = radius as isize;
        let window = (2 * radius + 1) as i32;
        let area = window * window;
        let mut temp = vec![0i32; self.mask.len()];
        crate::par::for_rows2(&mut temp, &self.mask, w, |_, trow, row| {
            let mut sum: i32 = 0;
            for i in -r..=r {
                if (i as usize) < w && i >= 0 {
                    sum += row[i as usize] as i32;
                }
            }
            for x in 0..w as isize {
                trow[x as usize] = sum;
                let add = x + r + 1;
                let remove = x - r;
                if add >= 0 && (add as usize) < w {
                    sum += row[add as usize] as i32;
                }
                if remove >= 0 && (remove as usize) < w {
                    sum -= row[remove as usize] as i32;
                }
            }
        });
        // Vertical pass, column-wise; process in column strips for cache locality.
        let mask = &mut self.mask;
        let mut sums = vec![0i32; w];
        for (x, s) in sums.iter_mut().enumerate() {
            for i in -r..=r {
                if i >= 0 && (i as usize) < h {
                    *s += temp[i as usize * w + x];
                }
            }
        }
        for y in 0..h as isize {
            let row = &mut mask[y as usize * w..(y as usize + 1) * w];
            for x in 0..w {
                row[x] = (sums[x] / area) as u8;
            }
            let add = y + r + 1;
            let remove = y - r;
            if add >= 0 && (add as usize) < h {
                let a = &temp[add as usize * w..(add as usize + 1) * w];
                for x in 0..w {
                    sums[x] += a[x];
                }
            }
            if remove >= 0 && (remove as usize) < h {
                let rm = &temp[remove as usize * w..(remove as usize + 1) * w];
                for x in 0..w {
                    sums[x] -= rm[x];
                }
            }
        }
        self.bounds = self.expand_bounds(radius);
    }

    fn expand_bounds(&self, radius: i32) -> PixelRect {
        let b = self.bounds;
        let bx = (b.x - radius).max(0);
        let by = (b.y - radius).max(0);
        let bx1 = (b.right() + radius).min(self.width);
        let by1 = (b.bottom() + radius).min(self.height);
        PixelRect::new(bx, by, bx1 - bx, by1 - by)
    }

    pub fn set_rectangle(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.reset(SelectionShape::Rectangle);
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let sw = self.width as usize;
        for yy in y0..y1 {
            let o = yy as usize * sw;
            self.mask[o + x0 as usize..o + x1 as usize].fill(255);
        }
        self.bounds = PixelRect::new(x0, y0, x1 - x0, y1 - y0);
    }

    pub fn set_ellipse(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.reset(SelectionShape::Ellipse);
        if w <= 0 || h <= 0 {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let cx = x as f64 + w as f64 / 2.0;
        let cy = y as f64 + h as f64 / 2.0;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let ha = (w as f64 / 2.0) as f32;
        let hb = (h as f64 / 2.0) as f32;
        let sw = self.width as usize;
        let rows = &mut self.mask[y0 as usize * sw..y1 as usize * sw];
        crate::par::for_rows(rows, sw, |i, row| {
            let yy = y0 + i as i32;
            let dy = (yy as f64 + 0.5 - cy) as f32;
            for xx in x0..x1 {
                let dx = (xx as f64 + 0.5 - cx) as f32;
                let dist = ellipse_distance(dx, dy, ha, hb);
                let cov = (0.5f32 - dist).clamp(0.0, 1.0);
                if cov > 0.0 {
                    row[xx as usize] = (cov * 255.0 + 0.5) as u8;
                }
            }
        });
        self.bounds = PixelRect::new(x0, y0, x1 - x0, y1 - y0);
    }

    /// Magic-wand flood: 4-connected region whose channels are all within
    /// `tolerance` of the seed pixel.
    pub fn set_by_color_flood(&mut self, src: &Surface, sx: i32, sy: i32, tolerance: i32) {
        self.set_by_color(src, sx, sy, tolerance, ToleranceAlpha::Straight, false);
    }

    /// Magic wand: contiguous (4-connected flood) or global selection of the
    /// pixels within `tolerance` of the seed pixel.
    pub fn set_by_color(&mut self, src: &Surface, sx: i32, sy: i32, tolerance: i32, alpha: ToleranceAlpha, global: bool) {
        if global {
            return self.set_by_color_global(src, sx, sy, tolerance, alpha);
        }
        self.reset(SelectionShape::Mask);
        assert!(src.width() == self.width && src.height() == self.height, "source dimensions must match");
        if !src.in_bounds(sx, sy) {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let target = src.get(sx, sy);
        let px = src.pixels();
        let matches = |i: usize| colors_match(px[i], target, tolerance, alpha);
        let r = scanline_flood(self.width, self.height, sx, sy, &mut self.mask, matches);
        self.bounds = r;
    }

    /// Global magic wand: every pixel in the image whose channels are all
    /// within `tolerance` of the seed pixel, connected or not.
    pub fn set_by_color_global(&mut self, src: &Surface, sx: i32, sy: i32, tolerance: i32, alpha: ToleranceAlpha) {
        self.reset(SelectionShape::Mask);
        assert!(src.width() == self.width && src.height() == self.height, "source dimensions must match");
        if !src.in_bounds(sx, sy) {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let target = src.get(sx, sy);
        let w = self.width as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (y, (row, mrow)) in src.pixels().chunks_exact(w).zip(self.mask.chunks_exact_mut(w)).enumerate() {
            let mut any = false;
            for (x, (&p, m)) in row.iter().zip(mrow.iter_mut()).enumerate() {
                if colors_match(p, target, tolerance, alpha) {
                    *m = 255;
                    x0 = x0.min(x as i32);
                    x1 = x1.max(x as i32);
                    any = true;
                }
            }
            if any {
                y0 = y0.min(y as i32);
                y1 = y1.max(y as i32);
            }
        }
        self.bounds = if x1 < x0 { PixelRect::EMPTY } else { PixelRect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1) };
    }

    pub fn set_polygon(&mut self, points: &[(i32, i32)], antialias: bool) {
        self.reset(SelectionShape::Polygon);
        if points.len() < 3 {
            self.outline = None;
            self.bounds = PixelRect::EMPTY;
            return;
        }
        self.outline = Some(Arc::new(points.to_vec()));
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for &(px, py) in points {
            min_x = min_x.min(px);
            max_x = max_x.max(px);
            min_y = min_y.min(py);
            max_y = max_y.max(py);
        }
        if antialias {
            self.fill_polygon_aa(points, min_x, min_y, max_x, max_y);
        } else {
            self.fill_polygon_hard(points, min_y, max_y);
        }
        let bx = min_x.max(0);
        let by = min_y.max(0);
        let bx1 = (max_x + 1).min(self.width);
        let by1 = (max_y + 1).min(self.height);
        self.bounds = if bx1 <= bx || by1 <= by { PixelRect::EMPTY } else { PixelRect::new(bx, by, bx1 - bx, by1 - by) };
    }

    fn fill_polygon_hard(&mut self, points: &[(i32, i32)], min_y: i32, max_y: i32) {
        let y0 = min_y.max(0);
        let y1 = (self.height - 1).min(max_y);
        let sw = self.width as usize;
        let mut xs: Vec<i32> = Vec::new();
        for y in y0..=y1 {
            polygon_crossings_int(points, y, &mut xs);
            let mut i = 0;
            while i + 1 < xs.len() {
                let sx = xs[i].max(0);
                let ex = (self.width - 1).min(xs[i + 1]);
                if ex >= sx {
                    let o = y as usize * sw;
                    self.mask[o + sx as usize..=o + ex as usize].fill(255);
                }
                i += 2;
            }
        }
    }

    fn fill_polygon_aa(&mut self, points: &[(i32, i32)], min_x: i32, min_y: i32, max_x: i32, max_y: i32) {
        const SS: i32 = 4;
        let y0 = min_y.max(0);
        let y1 = (self.height - 1).min(max_y);
        let row_x0 = min_x.max(0);
        let row_x1 = (self.width - 1).min(max_x);
        if row_x1 < row_x0 || y1 < y0 {
            return;
        }
        let row_width = (row_x1 - row_x0 + 1) as usize;
        let sw = self.width as usize;
        let rows = &mut self.mask[y0 as usize * sw..(y1 as usize + 1) * sw];
        crate::par::for_rows(rows, sw, |i, row| {
            let y = y0 + i as i32;
            let mut cover = vec![0i32; row_width];
            let mut xs: Vec<f64> = Vec::new();
            for sub in 0..SS {
                let sample_y = y as f64 + (sub as f64 + 0.5) / SS as f64 - 0.5;
                xs.clear();
                let n = points.len();
                for k in 0..n {
                    let a = points[k];
                    let b = points[(k + 1) % n];
                    let (ay, by) = (a.1 as f64, b.1 as f64);
                    if ay == by {
                        continue;
                    }
                    let ymin = ay.min(by);
                    let ymax = ay.max(by);
                    if sample_y < ymin || sample_y >= ymax {
                        continue;
                    }
                    let t = (sample_y - ay) / (by - ay);
                    xs.push(a.0 as f64 + t * (b.0 - a.0) as f64);
                }
                xs.sort_by(|p, q| p.total_cmp(q));
                let mut j = 0;
                while j + 1 < xs.len() {
                    accumulate_span(&mut cover, row_x0, row_x1, xs[j], xs[j + 1], SS);
                    j += 2;
                }
            }
            for (x, &c) in cover.iter().enumerate() {
                if c > 0 {
                    row[row_x0 as usize + x] = (c * 255 / (SS * SS)).min(255) as u8;
                }
            }
        });
    }

    pub fn select_all(&mut self) {
        self.mask.fill(255);
        self.shape = SelectionShape::Rectangle;
        self.outline = None;
        self.bounds = PixelRect::new(0, 0, self.width, self.height);
    }

    pub fn invert(&mut self) {
        self.shape = SelectionShape::Mask;
        self.outline = None;
        for m in &mut self.mask {
            *m = 255 - *m;
        }
        self.recompute_bounds();
    }

    pub fn clear(&mut self) {
        self.reset(SelectionShape::Rectangle);
        self.bounds = PixelRect::EMPTY;
    }

    /// Combines `shape` into `base` (C# `Selection.Combine`).
    pub fn combine(base: Option<&Selection>, shape: &Selection, mode: SelectionCombineMode) -> Selection {
        let base = match base {
            Some(b) if mode != SelectionCombineMode::Replace && !b.is_empty() => b,
            _ => {
                if matches!(mode, SelectionCombineMode::Subtract | SelectionCombineMode::Intersect) {
                    return Selection::new(shape.width, shape.height);
                }
                return shape.clone();
            }
        };
        assert!(base.width == shape.width && base.height == shape.height, "mismatched selection dimensions");
        let mut result = Selection::new(base.width, base.height);
        result.shape = SelectionShape::Mask;
        let (a, b) = (&base.mask, &shape.mask);
        match mode {
            SelectionCombineMode::Union => {
                for ((d, &x), &y) in result.mask.iter_mut().zip(a).zip(b) {
                    *d = x.max(y);
                }
                result.bounds = base.bounds.union(&shape.bounds);
            }
            SelectionCombineMode::Subtract => {
                for ((d, &x), &y) in result.mask.iter_mut().zip(a).zip(b) {
                    *d = x.saturating_sub(y);
                }
                result.recompute_bounds_in(base.bounds);
            }
            SelectionCombineMode::Intersect => {
                for ((d, &x), &y) in result.mask.iter_mut().zip(a).zip(b) {
                    *d = x.min(y);
                }
                result.recompute_bounds_in(base.bounds.intersect(&shape.bounds));
            }
            SelectionCombineMode::Xor => {
                for ((d, &x), &y) in result.mask.iter_mut().zip(a).zip(b) {
                    *d = x.abs_diff(y);
                }
                result.recompute_bounds_in(base.bounds.union(&shape.bounds));
            }
            SelectionCombineMode::Replace => unreachable!(),
        }
        result
    }

    /// Dilates the mask by a square of the given radius.
    pub fn grow(&mut self, radius: i32) {
        if radius <= 0 || self.is_empty() {
            return;
        }
        self.shape = SelectionShape::Mask;
        self.outline = None;
        self.morph(radius, true);
        self.bounds = self.expand_bounds(radius);
    }

    /// Erodes the mask by a square of the given radius; outside the image counts as unselected.
    pub fn shrink(&mut self, radius: i32) {
        if radius <= 0 || self.is_empty() {
            return;
        }
        self.shape = SelectionShape::Mask;
        self.outline = None;
        self.morph(radius, false);
        self.recompute_bounds();
    }

    /// Separable max (grow) or min (shrink) filter using monotonic deques: O(n)
    /// regardless of radius, same output as the C# brute-force loops.
    fn morph(&mut self, radius: i32, max: bool) {
        let (w, h) = (self.width as usize, self.height as usize);
        let r = radius as usize;
        let mut temp = vec![0u8; self.mask.len()];
        crate::par::for_rows2(&mut temp, &self.mask, w, |_, out, row| {
            if max {
                sliding_max(row, r, out);
            } else {
                sliding_min_zero_edges(row, r, out);
            }
        });
        // Transpose-free vertical pass: gather each column, filter, scatter.
        let mut col = vec![0u8; h];
        let mut out = vec![0u8; h];
        for x in 0..w {
            for y in 0..h {
                col[y] = temp[y * w + x];
            }
            if max {
                sliding_max(&col, r, &mut out);
            } else {
                sliding_min_zero_edges(&col, r, &mut out);
            }
            for y in 0..h {
                self.mask[y * w + x] = out[y];
            }
        }
    }

    /// Copies `source` shifted by (dx, dy).
    pub fn copy_translated_from(&mut self, source: &Selection, dx: i32, dy: i32) {
        assert!(source.width == self.width && source.height == self.height);
        self.mask.fill(0);
        self.shape = source.shape;
        // Outline is intentionally left as-is, matching C#.
        let (w, h) = (self.width, self.height);
        let ys0 = (-dy).max(0);
        let ys1 = h.min(h - dy);
        let xs0 = (-dx).max(0);
        let xs1 = w.min(w - dx);
        if xs0 >= xs1 || ys0 >= ys1 {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let n = (xs1 - xs0) as usize;
        let sw = w as usize;
        for sy in ys0..ys1 {
            let so = sy as usize * sw + xs0 as usize;
            let dof = (sy + dy) as usize * sw + (xs0 + dx) as usize;
            self.mask[dof..dof + n].copy_from_slice(&source.mask[so..so + n]);
        }
        self.bounds = if source.bounds.is_empty() {
            PixelRect::EMPTY
        } else {
            let sb = source.bounds;
            let nx = (sb.x + dx).max(0);
            let ny = (sb.y + dy).max(0);
            let nx1 = (sb.right() + dx).min(w);
            let ny1 = (sb.bottom() + dy).min(h);
            if nx1 <= nx || ny1 <= ny { PixelRect::EMPTY } else { PixelRect::new(nx, ny, nx1 - nx, ny1 - ny) }
        };
    }

    /// Replaces the mask with `coverage` over `area` (nothing elsewhere), as
    /// after a free transform; `outline` is the transformed lasso outline.
    pub fn set_warped(&mut self, area: PixelRect, mut coverage: impl FnMut(i32, i32) -> u8, outline: Option<Vec<(i32, i32)>>) {
        self.mask.fill(0);
        let area = area.intersect(&PixelRect::new(0, 0, self.width, self.height));
        let w = self.width as usize;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                self.mask[y as usize * w + x as usize] = coverage(x, y);
            }
        }
        self.shape = SelectionShape::Mask;
        self.outline = outline.map(Arc::new);
        self.recompute_bounds_in(area);
    }

    /// Scales `source`'s bounds into `target` with area-averaged coverage.
    pub fn set_scaled_from(&mut self, source: &Selection, target: RectF) {
        assert!(source.width == self.width && source.height == self.height);
        let sb = source.bounds;
        self.mask.fill(0);
        if sb.is_empty() || target.w <= 0.0 || target.h <= 0.0 {
            self.bounds = PixelRect::EMPTY;
            self.outline = None;
            return;
        }
        let sw = self.width as usize;
        let mask = &mut self.mask;
        resample::resample_alpha(&source.mask, self.width, sb, target, self.width, self.height, |x, y, a, cov| {
            mask[y as usize * sw + x as usize] = ((a as f32 * cov + 0.5) as i32).clamp(0, 255) as u8;
        });
        self.shape = source.shape;
        self.outline = source.outline.as_ref().map(|pts| {
            let kx = target.w / sb.width as f64;
            let ky = target.h / sb.height as f64;
            Arc::new(
                pts.iter()
                    .map(|&(px, py)| {
                        (
                            round_even(target.x + (px - sb.x) as f64 * kx) as i32,
                            round_even(target.y + (py - sb.y) as f64 * ky) as i32,
                        )
                    })
                    .collect(),
            )
        });
        self.recompute_bounds();
    }

    fn recompute_bounds(&mut self) {
        self.recompute_bounds_in(PixelRect::new(0, 0, self.width, self.height));
    }

    fn recompute_bounds_in(&mut self, roi: PixelRect) {
        if roi.is_empty() {
            self.bounds = PixelRect::EMPTY;
            return;
        }
        let xs = roi.x.max(0);
        let ys = roi.y.max(0);
        let xe = self.width.min(roi.right());
        let ye = self.height.min(roi.bottom());
        let sw = self.width as usize;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for y in ys..ye {
            let row = &self.mask[y as usize * sw + xs as usize..y as usize * sw + xe.max(xs) as usize];
            let Some(first) = row.iter().position(|&m| m != 0) else { continue };
            let last = row.iter().rposition(|&m| m != 0).unwrap();
            min_x = min_x.min(xs + first as i32);
            max_x = max_x.max(xs + last as i32);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        self.bounds = PixelRect::from_inclusive(min_x, min_y, max_x, max_y);
    }
}

/// How the alpha channel takes part in tolerance comparisons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToleranceAlpha {
    /// Classic: every channel, colour and alpha, compared as stored. Two
    /// transparent pixels match only if their (invisible) colours match too.
    #[default]
    Straight,
    /// Colour weighted by alpha: transparent pixels match each other
    /// whatever their colour channels hold.
    Premultiplied,
}

/// Tolerance comparison used by the magic wand and paint bucket.
#[inline]
pub fn colors_match(a: Bgra, b: Bgra, tol: i32, mode: ToleranceAlpha) -> bool {
    match mode {
        ToleranceAlpha::Straight => color_matches(a, b, tol),
        ToleranceAlpha::Premultiplied => {
            let pm = |c: u8, al: u8| (c as i32 * al as i32 + 127) / 255;
            (pm(a.b, a.a) - pm(b.b, b.a)).abs() <= tol
                && (pm(a.g, a.a) - pm(b.g, b.a)).abs() <= tol
                && (pm(a.r, a.a) - pm(b.r, b.a)).abs() <= tol
                && (a.a as i32 - b.a as i32).abs() <= tol
        }
    }
}

#[inline(always)]
fn color_matches(a: Bgra, b: Bgra, tol: i32) -> bool {
    (a.b as i32 - b.b as i32).abs() <= tol
        && (a.g as i32 - b.g as i32).abs() <= tol
        && (a.r as i32 - b.r as i32).abs() <= tol
        && (a.a as i32 - b.a as i32).abs() <= tol
}

/// 4-connected scanline flood fill. Marks `mask[i] = 255` for every pixel
/// reachable from (sx, sy) through pixels where `matches(i)`; returns the
/// bounds of the filled region. Same set as a naive stack fill.
pub(crate) fn scanline_flood(
    w: i32,
    h: i32,
    sx: i32,
    sy: i32,
    mask: &mut [u8],
    matches: impl Fn(usize) -> bool,
) -> PixelRect {
    let wu = w as usize;
    if !matches(sy as usize * wu + sx as usize) {
        return PixelRect::EMPTY;
    }
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (sx, sy, sx, sy);
    let mut stack: Vec<(i32, i32)> = vec![(sx, sy)];
    while let Some((x, y)) = stack.pop() {
        let row = y as usize * wu;
        if mask[row + x as usize] != 0 {
            continue;
        }
        let mut l = x;
        while l > 0 && mask[row + l as usize - 1] == 0 && matches(row + l as usize - 1) {
            l -= 1;
        }
        let mut r = x;
        while r + 1 < w && mask[row + r as usize + 1] == 0 && matches(row + r as usize + 1) {
            r += 1;
        }
        mask[row + l as usize..=row + r as usize].fill(255);
        min_x = min_x.min(l);
        max_x = max_x.max(r);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
        for ny in [y - 1, y + 1] {
            if ny < 0 || ny >= h {
                continue;
            }
            let nrow = ny as usize * wu;
            let mut xx = l;
            while xx <= r {
                let i = nrow + xx as usize;
                if mask[i] == 0 && matches(i) {
                    stack.push((xx, ny));
                    // Skip the rest of this run; it will be filled from the seed.
                    while xx <= r && mask[nrow + xx as usize] == 0 && matches(nrow + xx as usize) {
                        xx += 1;
                    }
                } else {
                    xx += 1;
                }
            }
        }
    }
    PixelRect::new(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
}

/// Sorted x-crossings of scanline `y` with the polygon edges (C# hard fill rule).
pub(crate) fn polygon_crossings_int(points: &[(i32, i32)], y: i32, xs: &mut Vec<i32>) {
    xs.clear();
    let n = points.len();
    for i in 0..n {
        let a = points[i];
        let b = points[(i + 1) % n];
        if a.1 == b.1 {
            continue;
        }
        let ymin = a.1.min(b.1);
        let ymax = a.1.max(b.1);
        if y < ymin || y >= ymax {
            continue;
        }
        let t = (y - a.1) as f64 / (b.1 - a.1) as f64;
        xs.push(a.0 + round_even(t * (b.0 - a.0) as f64) as i32);
    }
    xs.sort_unstable();
}

fn accumulate_span(cover: &mut [i32], row_x0: i32, row_x1: i32, sx0: f64, sx1: f64, ss: i32) {
    let px0 = row_x0.max((sx0 - 0.5).floor() as i32);
    let px1 = row_x1.min((sx1 - 0.5).ceil() as i32);
    for x in px0..=px1 {
        for s in 0..ss {
            let sample_x = x as f64 + (s as f64 + 0.5) / ss as f64 - 0.5;
            if sample_x >= sx0 && sample_x < sx1 {
                cover[(x - row_x0) as usize] += 1;
            }
        }
    }
}

/// out[i] = max(v[max(0,i-r) ..= min(n-1,i+r)])
fn sliding_max(v: &[u8], r: usize, out: &mut [u8]) {
    let n = v.len();
    let mut dq: VecDeque<usize> = VecDeque::with_capacity(2 * r + 2);
    let mut next = 0;
    for i in 0..n {
        let hi = (i + r).min(n - 1);
        while next <= hi {
            while let Some(&b) = dq.back() {
                if v[b] <= v[next] { dq.pop_back(); } else { break; }
            }
            dq.push_back(next);
            next += 1;
        }
        let lo = i.saturating_sub(r);
        while let Some(&f) = dq.front() {
            if f < lo { dq.pop_front(); } else { break; }
        }
        out[i] = v[*dq.front().unwrap()];
    }
}

/// out[i] = 0 if the window [i-r, i+r] leaves the array, else its min.
fn sliding_min_zero_edges(v: &[u8], r: usize, out: &mut [u8]) {
    let n = v.len();
    if 2 * r + 1 > n {
        out.fill(0);
        return;
    }
    let mut dq: VecDeque<usize> = VecDeque::with_capacity(2 * r + 2);
    for i in 0..n {
        while let Some(&b) = dq.back() {
            if v[b] >= v[i] { dq.pop_back(); } else { break; }
        }
        dq.push_back(i);
        if i >= 2 * r {
            let lo = i - 2 * r;
            while let Some(&f) = dq.front() {
                if f < lo { dq.pop_front(); } else { break; }
            }
            out[i - r] = v[*dq.front().unwrap()];
        }
    }
    out[..r].fill(0);
    out[n - r..].fill(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute_grow(m: &[u8], w: usize, h: usize, r: i32) -> Vec<u8> {
        let mut t = vec![0u8; m.len()];
        for y in 0..h {
            for x in 0..w {
                let x0 = (x as i32 - r).max(0) as usize;
                let x1 = (x as i32 + r).min(w as i32 - 1) as usize;
                t[y * w + x] = (x0..=x1).map(|i| m[y * w + i]).max().unwrap();
            }
        }
        let mut o = vec![0u8; m.len()];
        for x in 0..w {
            for y in 0..h {
                let y0 = (y as i32 - r).max(0) as usize;
                let y1 = (y as i32 + r).min(h as i32 - 1) as usize;
                o[y * w + x] = (y0..=y1).map(|i| t[i * w + x]).max().unwrap();
            }
        }
        o
    }

    fn brute_shrink(m: &[u8], w: usize, h: usize, r: i32) -> Vec<u8> {
        let mut t = vec![0u8; m.len()];
        for y in 0..h {
            for x in 0..w as i32 {
                t[y * w + x as usize] = if x - r < 0 || x + r >= w as i32 {
                    0
                } else {
                    ((x - r)..=(x + r)).map(|i| m[y * w + i as usize]).min().unwrap()
                };
            }
        }
        let mut o = vec![0u8; m.len()];
        for x in 0..w {
            for y in 0..h as i32 {
                o[y as usize * w + x] = if y - r < 0 || y + r >= h as i32 {
                    0
                } else {
                    ((y - r)..=(y + r)).map(|i| t[i as usize * w + x]).min().unwrap()
                };
            }
        }
        o
    }

    #[test]
    fn morph_matches_brute_force() {
        let (w, h) = (23usize, 17usize);
        let mut s = Selection::new(w as i32, h as i32);
        s.set_ellipse(3, 2, 15, 11);
        let orig = s.mask.clone();
        for r in 1..6 {
            let mut g = s.clone();
            g.grow(r);
            assert_eq!(g.mask, brute_grow(&orig, w, h, r), "grow r={r}");
            let mut k = s.clone();
            k.shrink(r);
            assert_eq!(k.mask, brute_shrink(&orig, w, h, r), "shrink r={r}");
        }
    }

    #[test]
    fn flood_matches_naive() {
        let mut surf = Surface::new(40, 30);
        for y in 0..30 {
            for x in 0..40 {
                let v = (((x * 7) ^ (y * 13)) % 5) as u8 * 40;
                surf.set(x, y, Bgra::new(v, v, v, 255));
            }
        }
        for tol in [0, 40, 80] {
            let mut s = Selection::new(40, 30);
            s.set_by_color_flood(&surf, 5, 5, tol);
            // naive
            let target = surf.get(5, 5);
            let mut m = vec![0u8; 1200];
            let mut st = vec![(5, 5)];
            while let Some((x, y)) = st.pop() {
                if !surf.in_bounds(x, y) { continue; }
                let i = (y * 40 + x) as usize;
                if m[i] != 0 || !color_matches(surf.get(x, y), target, tol) { continue; }
                m[i] = 255;
                st.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
            }
            assert_eq!(s.mask, m, "tol={tol}");
        }
    }
}
