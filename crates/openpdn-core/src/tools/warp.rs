//! Distorting tools: the Distort brush (pinch, bulge, push, twirl) and the
//! Perspective transform (drag corners to skew or put in perspective).

use super::{EditOverlay, Modifiers, PendingAction, PointerButton, Tool, ToolContext, ToolKind};
use crate::color::Bgra;
use crate::history::{BitmapLayerEdit, CompoundEdit, HistoryEntry, SelectionEdit};
use crate::rect::PixelRect;
use crate::resample::{lerp_by_coverage, TransformResampling};
use crate::selection::Selection;
use crate::surface::Surface;

// ---------------------------------------------------------------- sampling

#[inline]
fn premul(c: Bgra) -> [f32; 4] {
    let a = c.a as f32 / 255.0;
    [c.b as f32 * a, c.g as f32 * a, c.r as f32 * a, c.a as f32]
}

#[inline]
fn unpremul(p: [f32; 4]) -> Bgra {
    if p[3] <= 0.5 {
        return Bgra::TRANSPARENT;
    }
    let k = 255.0 / p[3];
    let ch = |v: f32| (v * k + 0.5).clamp(0.0, 255.0) as u8;
    Bgra::new(ch(p[0]), ch(p[1]), ch(p[2]), (p[3] + 0.5).clamp(0.0, 255.0) as u8)
}

/// A rectangle of pixels to sample from.
struct Patch<'a> {
    px: &'a [Bgra],
    /// Row stride of `px`.
    stride: usize,
    /// Where `px[0]` is in image coordinates.
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
}

impl Patch<'_> {
    /// Bilinear sample at image position (x, y) (pixel centres at +0.5),
    /// premultiplied; coordinates are clamped to the patch.
    fn clamped(&self, x: f32, y: f32) -> [f32; 4] {
        let fx = (x - 0.5 - self.x0 as f32).clamp(0.0, (self.w - 1) as f32);
        let fy = (y - 0.5 - self.y0 as f32).clamp(0.0, (self.h - 1) as f32);
        let (ix, iy) = (fx as i32, fy as i32);
        let (tx, ty) = (fx - ix as f32, fy - iy as f32);
        let ix1 = (ix + 1).min(self.w - 1);
        let iy1 = (iy + 1).min(self.h - 1);
        let at = |x: i32, y: i32| premul(self.px[y as usize * self.stride + x as usize]);
        let (p00, p10, p01, p11) = (at(ix, iy), at(ix1, iy), at(ix, iy1), at(ix1, iy1));
        let mut out = [0.0; 4];
        for k in 0..4 {
            let top = p00[k] + (p10[k] - p00[k]) * tx;
            let bot = p01[k] + (p11[k] - p01[k]) * tx;
            out[k] = top + (bot - top) * ty;
        }
        out
    }
}

// ---------------------------------------------------------------- Distort brush

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WarpMode {
    /// Squeeze towards the middle of the brush.
    #[default]
    Pinch,
    /// Swell out from the middle of the brush.
    Bulge,
    /// Drag the pixels along with the brush.
    Push,
    TwirlClockwise,
    TwirlCounterclockwise,
}

impl WarpMode {
    pub const ALL: [WarpMode; 5] = [WarpMode::Pinch, WarpMode::Bulge, WarpMode::Push, WarpMode::TwirlClockwise, WarpMode::TwirlCounterclockwise];
    pub fn name(self) -> &'static str {
        match self {
            WarpMode::Pinch => "Pinch",
            WarpMode::Bulge => "Bulge",
            WarpMode::Push => "Push",
            WarpMode::TwirlClockwise => "TwirlClockwise",
            WarpMode::TwirlCounterclockwise => "TwirlCounterclockwise",
        }
    }
    pub fn parse(s: &str) -> Option<WarpMode> {
        WarpMode::ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(s))
    }
    /// The opposite effect (Alt held).
    fn inverse(self) -> WarpMode {
        match self {
            WarpMode::Pinch => WarpMode::Bulge,
            WarpMode::Bulge => WarpMode::Pinch,
            WarpMode::Push => WarpMode::Push,
            WarpMode::TwirlClockwise => WarpMode::TwirlCounterclockwise,
            WarpMode::TwirlCounterclockwise => WarpMode::TwirlClockwise,
        }
    }
}

/// One application of the distort brush at (cx, cy). `amount` is the
/// strength in [0, 1]; `push` is the brush movement for [`WarpMode::Push`].
/// Returns the area changed.
#[allow(clippy::too_many_arguments)]
pub fn warp_dab(s: &mut Surface, sel: Option<&Selection>, cx: f32, cy: f32, radius: f32, mode: WarpMode, amount: f32, push: (f32, f32)) -> PixelRect {
    let radius = radius.max(1.0);
    let (w, h) = (s.width(), s.height());
    let area = PixelRect::clamp_box((cx - radius).floor() as i32, (cy - radius).floor() as i32, (cx + radius).ceil() as i32, (cy + radius).ceil() as i32, w, h);
    let area = match sel {
        Some(m) => area.intersect(&m.bounds()),
        None => area,
    };
    if area.is_empty() {
        return PixelRect::EMPTY;
    }
    // Sample from a copy: everything the brush can reach.
    let reach = radius.ceil() as i32 + 2;
    let src_r = PixelRect::clamp_box(area.x - reach, area.y - reach, area.right() - 1 + reach, area.bottom() - 1 + reach, w, h);
    let copy = s.read_rect(src_r);
    let patch = Patch { px: &copy, stride: src_r.width as usize, x0: src_r.x, y0: src_r.y, w: src_r.width, h: src_r.height };
    let r2 = radius * radius;
    for y in area.y..area.bottom() {
        let fy = y as f32 + 0.5 - cy;
        for x in area.x..area.right() {
            let fx = x as f32 + 0.5 - cx;
            let d2 = fx * fx + fy * fy;
            if d2 >= r2 {
                continue;
            }
            let cov = sel.map_or(255, |m| m.coverage_at(x, y));
            if cov == 0 {
                continue;
            }
            // Smooth falloff: full effect in the middle, none at the rim.
            let q = 1.0 - d2 / r2;
            let f = q * q;
            let (sx, sy) = match mode {
                WarpMode::Pinch => {
                    let k = 1.0 + amount * 0.5 * f;
                    (cx + fx * k, cy + fy * k)
                }
                WarpMode::Bulge => {
                    let k = 1.0 - amount * 0.5 * f;
                    (cx + fx * k, cy + fy * k)
                }
                WarpMode::Push => (x as f32 + 0.5 - push.0 * f, y as f32 + 0.5 - push.1 * f),
                WarpMode::TwirlClockwise | WarpMode::TwirlCounterclockwise => {
                    let mut a = amount * 0.6 * f;
                    if mode == WarpMode::TwirlClockwise {
                        a = -a; // sample backwards so the image turns clockwise on screen
                    }
                    let (sin, cos) = a.sin_cos();
                    (cx + fx * cos - fy * sin, cy + fx * sin + fy * cos)
                }
            };
            let c = unpremul(patch.clamped(sx, sy));
            let i = (y - src_r.y) as usize * src_r.width as usize + (x - src_r.x) as usize;
            let orig = copy[i];
            let out = if cov == 255 { c } else { lerp_by_coverage(orig, c, cov as f32 / 255.0) };
            s.set(x, y, out);
        }
    }
    area
}

#[derive(Default)]
pub struct DistortTool {
    active: bool,
    before: Vec<Bgra>,
    last: (f32, f32),
    touched: PixelRect,
    mode: WarpMode,
}

impl DistortTool {
    fn dab(&mut self, ctx: &mut ToolContext, c: (f32, f32), push: (f32, f32)) {
        let radius = ctx.settings.brush_size() as f32 / 2.0;
        let pressure = if ctx.settings.pressure_sensitivity { ctx.pressure.clamp(0.05, 1.0) } else { 1.0 };
        let amount = ctx.settings.warp_strength.clamp(1, 100) as f32 / 100.0 * pressure;
        let mode = self.mode;
        let push = (push.0 * amount, push.1 * amount);
        let (layer, sel) = ctx.layer_sel();
        let sel = sel.filter(|s| !s.is_empty());
        let r = warp_dab(&mut layer.surface, sel, c.0, c.1, radius, mode, amount, push);
        self.touched = self.touched.union(&r);
        ctx.invalidate(r);
    }

    fn stroke_to(&mut self, ctx: &mut ToolContext, p: (f32, f32)) {
        let radius = (ctx.settings.brush_size() as f32 / 2.0).max(1.0);
        let (dx, dy) = (p.0 - self.last.0, p.1 - self.last.1);
        let dist = (dx * dx + dy * dy).sqrt();
        if self.mode == WarpMode::Push {
            if dist < 0.01 {
                return;
            }
            // Small steps, so the pixels follow the brush smoothly.
            let steps = (dist / (radius / 4.0).max(1.0)).ceil().max(1.0) as i32;
            let step = (dx / steps as f32, dy / steps as f32);
            for i in 1..=steps {
                let c = (self.last.0 + step.0 * i as f32, self.last.1 + step.1 * i as f32);
                self.dab(ctx, c, step);
            }
        } else if dist < 0.01 {
            // Held still (the app repeats the position): keep working here.
            self.dab(ctx, p, (0.0, 0.0));
        } else {
            let spacing = (radius / 3.0).max(1.0);
            let steps = (dist / spacing).ceil().max(1.0) as i32;
            for i in 1..=steps {
                let t = i as f32 / steps as f32;
                self.dab(ctx, (self.last.0 + dx * t, self.last.1 + dy * t), (0.0, 0.0));
            }
        }
        self.last = p;
    }
}

impl Tool for DistortTool {
    fn name(&self) -> &'static str {
        "Distort"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Distort
    }
    fn is_busy(&self) -> bool {
        self.active
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.active = true;
        self.before = ctx.layer_ref().surface.pixels().to_vec();
        self.touched = PixelRect::EMPTY;
        // Right button or Alt: the opposite effect.
        let invert = button == PointerButton::Right || mods.alt();
        self.mode = if invert { ctx.settings.warp_mode.inverse() } else { ctx.settings.warp_mode };
        self.last = (x as f32 + 0.5, y as f32 + 0.5);
        if self.mode != WarpMode::Push {
            self.dab(ctx, self.last, (0.0, 0.0));
        }
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if self.active {
            self.stroke_to(ctx, (x as f32 + 0.5, y as f32 + 0.5));
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if !self.active {
            return;
        }
        self.active = false;
        let before = std::mem::take(&mut self.before);
        if let Some(e) = BitmapLayerEdit::capture(ctx.doc, ctx.layer, &before, self.touched, "Distort") {
            ctx.history.push(e);
        }
    }
}

// ---------------------------------------------------------------- Perspective

type Quad = [(f32, f32); 4];

/// A projective map from the unit square to a quadrilateral.
#[derive(Clone, Copy, Debug)]
struct Homography([f64; 9]);

impl Homography {
    /// Unit square corners (0,0), (1,0), (1,1), (0,1) → `q[0..4]` (Heckbert).
    fn square_to_quad(q: &Quad) -> Homography {
        let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q.map(|(x, y)| (x as f64, y as f64));
        let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
        let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
        let den = dx1 * dy2 - dx2 * dy1;
        let (g, h) = if den.abs() < 1e-12 || (dx3.abs() < 1e-9 && dy3.abs() < 1e-9) {
            (0.0, 0.0)
        } else {
            ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
        };
        Homography([x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h, 1.0])
    }

    #[inline]
    fn apply(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let m = &self.0;
        let w = m[6] * x + m[7] * y + m[8];
        if w.abs() < 1e-12 {
            return None;
        }
        Some(((m[0] * x + m[1] * y + m[2]) / w, (m[3] * x + m[4] * y + m[5]) / w))
    }

    fn inverse(&self) -> Option<Homography> {
        let [a, b, c, d, e, f, g, h, i] = self.0;
        let (ca, cb, cc) = (e * i - f * h, -(d * i - f * g), d * h - e * g);
        let det = a * ca + b * cb + c * cc;
        if det.abs() < 1e-12 {
            return None;
        }
        let k = 1.0 / det;
        Some(Homography([
            ca * k,
            -(b * i - c * h) * k,
            (b * f - c * e) * k,
            cb * k,
            (a * i - c * g) * k,
            -(a * f - c * d) * k,
            cc * k,
            -(a * h - b * g) * k,
            (a * e - b * d) * k,
        ]))
    }
}

/// Whether the corners make a convex, non-degenerate quadrilateral (in order).
fn is_convex(q: &Quad) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let cross = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if cross.abs() < 1e-3 {
            return false;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

fn quad_bounds(q: &Quad, w: i32, h: i32) -> PixelRect {
    let x0 = q.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor() as i32;
    let y0 = q.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor() as i32;
    let x1 = q.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil() as i32;
    let y1 = q.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil() as i32;
    PixelRect::clamp_box(x0, y0, x1 - 1, y1 - 1, w, h)
}

fn rect_quad(r: PixelRect) -> Quad {
    let (x0, y0, x1, y1) = (r.x as f32, r.y as f32, r.right() as f32, r.bottom() as f32);
    [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

struct PerspEdit {
    /// The layer as it was (full snapshot).
    original: Vec<Bgra>,
    sel_before: Option<Selection>,
    /// What is being transformed: the selection's bounds or the whole layer.
    src: PixelRect,
    start: Quad,
    quad: Quad,
    /// Area the last render wrote.
    prev: PixelRect,
}

#[derive(Clone, Copy, Debug)]
enum PDrag {
    Corner(usize),
    /// Edge i runs from corner i to corner i+1: both move (skew).
    Edge(usize),
    Body,
}

#[derive(Default)]
pub struct PerspectiveTool {
    edit: Option<PerspEdit>,
    drag: Option<(PDrag, Quad, f32, f32)>,
    /// Corners before a move/rotate started (pending actions).
    base: Option<Quad>,
    undo: Vec<Quad>,
    redo: Vec<Quad>,
}

impl PerspectiveTool {
    fn begin(&mut self, ctx: &mut ToolContext) {
        let Some(layer) = ctx.doc.layer(ctx.layer) else { return };
        let original = layer.surface.pixels().to_vec();
        let full = layer.surface.bounds();
        let sel = ctx.doc.active_selection().filter(|s| !s.is_empty()).cloned();
        let src = sel.as_ref().map_or(full, |s| s.bounds()).intersect(&full);
        if src.is_empty() {
            return;
        }
        let q = rect_quad(src);
        self.edit = Some(PerspEdit { original, sel_before: sel, src, start: q, quad: q, prev: PixelRect::EMPTY });
        self.undo.clear();
        self.redo.clear();
        self.base = None;
    }

    /// Redraws the layer for the current corners; `with_selection` also bends
    /// the selection to match (on release, not on every drag step).
    fn render(&mut self, ctx: &mut ToolContext, with_selection: bool) {
        let Some(e) = self.edit.as_mut() else { return };
        let mode = ctx.settings.resampling;
        let Some(layer) = ctx.doc.layer_mut(ctx.layer) else { return };
        let s = &mut layer.surface;
        let (w, h) = (s.width(), s.height());
        let old = e.prev;
        s.copy_rect_from(&e.original, old);
        if e.quad == e.start {
            e.prev = PixelRect::EMPTY;
            if with_selection {
                ctx.doc.set_selection(e.sel_before.clone());
            }
            ctx.doc.invalidate(Some(old));
            return;
        }
        // Lift the source pixels off the layer.
        let src = e.src;
        for y in src.y..src.bottom() {
            let row = s.row_mut(y);
            for x in src.x..src.right() {
                if e.sel_before.as_ref().is_none_or(|m| m.contains(x, y)) {
                    row[x as usize] = Bgra::TRANSPARENT;
                }
            }
        }
        let fwd = Homography::square_to_quad(&e.quad);
        let Some(inv) = fwd.inverse() else { return };
        let dest = quad_bounds(&e.quad, w, h);
        let stride = w as usize;
        let (sw, sh) = (src.width as f64, src.height as f64);
        let nearest = mode == TransformResampling::NearestNeighbor;
        let orig = &e.original;
        let sel = e.sel_before.as_ref();
        // Source pixel, weighted by how selected it is; transparent outside.
        let fetch = |x: i32, y: i32| -> [f32; 4] {
            if x < src.x || y < src.y || x >= src.right() || y >= src.bottom() {
                return [0.0; 4];
            }
            let mut p = premul(orig[y as usize * stride + x as usize]);
            if let Some(m) = sel {
                let k = m.coverage_at(x, y) as f32 / 255.0;
                p.iter_mut().for_each(|v| *v *= k);
            }
            p
        };
        let map = |px: f64, py: f64| -> Option<(f32, f32)> {
            let (u, v) = inv.apply(px, py)?;
            if !(-0.01..=1.01).contains(&u) || !(-0.01..=1.01).contains(&v) {
                return None;
            }
            Some(((src.x as f64 + u * sw) as f32, (src.y as f64 + v * sh) as f32))
        };
        for y in dest.y..dest.bottom() {
            for x in dest.x..dest.right() {
                let Some((sx, sy)) = map(x as f64 + 0.5, y as f64 + 0.5) else { continue };
                let p = if nearest {
                    fetch(sx.floor() as i32, sy.floor() as i32)
                } else {
                    let (fx, fy) = (sx - 0.5, sy - 0.5);
                    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
                    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
                    let (a, b, c, d) = (fetch(ix, iy), fetch(ix + 1, iy), fetch(ix, iy + 1), fetch(ix + 1, iy + 1));
                    let mut o = [0.0; 4];
                    for k in 0..4 {
                        let top = a[k] + (b[k] - a[k]) * tx;
                        let bot = c[k] + (d[k] - c[k]) * tx;
                        o[k] = top + (bot - top) * ty;
                    }
                    o
                };
                if p[3] < 0.5 {
                    continue;
                }
                // Over what is left of the layer.
                let under = premul(s.get(x, y));
                let k = 1.0 - p[3] / 255.0;
                s.set(x, y, unpremul([p[0] + under[0] * k, p[1] + under[1] * k, p[2] + under[2] * k, p[3] + under[3] * k]));
            }
        }
        e.prev = src.union(&dest);
        if with_selection {
            if let Some(before) = &e.sel_before {
                let mut warped = before.clone();
                let cov = |x: i32, y: i32| -> u8 {
                    let Some((sx, sy)) = map(x as f64 + 0.5, y as f64 + 0.5) else { return 0 };
                    let (fx, fy) = (sx - 0.5, sy - 0.5);
                    let (ix, iy) = (fx.floor() as i32, fy.floor() as i32);
                    let (tx, ty) = (fx - ix as f32, fy - iy as f32);
                    let m = |x: i32, y: i32| before.coverage_at(x, y) as f32;
                    let top = m(ix, iy) + (m(ix + 1, iy) - m(ix, iy)) * tx;
                    let bot = m(ix, iy + 1) + (m(ix + 1, iy + 1) - m(ix, iy + 1)) * tx;
                    (top + (bot - top) * ty + 0.5).clamp(0.0, 255.0) as u8
                };
                let outline = before.outline().map(|pts| {
                    pts.iter()
                        .filter_map(|&(px, py)| fwd.apply((px - src.x) as f64 / sw, (py - src.y) as f64 / sh))
                        .map(|(x, y)| (x.round() as i32, y.round() as i32))
                        .collect()
                });
                warped.set_warped(dest, cov, outline);
                ctx.doc.set_selection(Some(warped));
            }
        }
        ctx.doc.invalidate(Some(old.union(&e.prev)));
    }

    fn set_quad(&mut self, ctx: &mut ToolContext, q: Quad, with_selection: bool) -> bool {
        if !is_convex(&q) {
            return false;
        }
        if let Some(e) = self.edit.as_mut() {
            e.quad = q;
        }
        self.render(ctx, with_selection);
        true
    }

    fn snapshot(&mut self) {
        if let Some(e) = &self.edit {
            self.undo.push(e.quad);
            self.redo.clear();
        }
    }

    fn finish(&mut self, ctx: &mut ToolContext, keep: bool) {
        let Some(mut e) = self.edit.take() else { return };
        self.drag = None;
        self.base = None;
        self.undo.clear();
        self.redo.clear();
        if e.quad == e.start {
            return; // nothing was changed
        }
        if !keep {
            if let Some(l) = ctx.doc.layer_mut(ctx.layer) {
                l.surface.copy_rect_from(&e.original, e.prev);
            }
            ctx.doc.set_selection(e.sel_before.take());
            ctx.doc.invalidate(Some(e.prev));
            return;
        }
        let mut entries: Vec<Box<dyn HistoryEntry>> = Vec::new();
        if let Some(b) = BitmapLayerEdit::capture(ctx.doc, ctx.layer, &e.original, e.prev, "Perspective") {
            entries.push(Box::new(b));
        }
        if let (Some(before), Some(after)) = (e.sel_before.take(), ctx.doc.selection().cloned()) {
            entries.push(Box::new(SelectionEdit::new(Some(before), Some(after), "Perspective")));
        }
        match entries.len() {
            0 => {}
            1 => ctx.history.push_boxed(entries.pop().unwrap()),
            _ => ctx.history.push(CompoundEdit::new(entries, "Perspective")),
        }
    }

    fn move_all(&mut self, ctx: &mut ToolContext, start: bool, f: impl Fn((f32, f32)) -> (f32, f32)) {
        let Some(e) = &self.edit else { return };
        if start || self.base.is_none() {
            self.base = Some(e.quad);
            self.undo.push(e.quad);
            self.redo.clear();
        }
        let q = self.base.unwrap().map(f);
        self.set_quad(ctx, q, true);
    }
}

impl Tool for PerspectiveTool {
    fn name(&self) -> &'static str {
        "Perspective"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Perspective
    }
    fn is_busy(&self) -> bool {
        self.drag.is_some()
    }
    fn has_pending(&self) -> bool {
        self.edit.is_some()
    }
    fn has_pending_edits(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.quad != e.start) || !self.undo.is_empty() || !self.redo.is_empty()
    }
    fn edit_overlay(&self) -> Option<EditOverlay> {
        self.edit.as_ref().map(|e| EditOverlay::Quad { corners: e.quad })
    }
    fn activate(&mut self, ctx: &mut ToolContext) {
        if self.edit.is_none() {
            self.begin(ctx);
        }
    }
    fn pending(&mut self, ctx: &mut ToolContext, action: PendingAction) {
        match action {
            PendingAction::Commit => self.finish(ctx, true),
            PendingAction::Cancel => self.finish(ctx, false),
            PendingAction::Refresh => self.render(ctx, true),
            PendingAction::Undo => match self.undo.pop() {
                Some(q) => {
                    if let Some(e) = &self.edit {
                        self.redo.push(e.quad);
                    }
                    if let Some(e) = self.edit.as_mut() {
                        e.quad = q;
                    }
                    self.render(ctx, true);
                }
                None => self.finish(ctx, false),
            },
            PendingAction::Redo => {
                if let Some(q) = self.redo.pop() {
                    if let Some(e) = self.edit.as_mut() {
                        self.undo.push(e.quad);
                        e.quad = q;
                    }
                    self.render(ctx, true);
                }
            }
            PendingAction::Translate { dx, dy, start } => self.move_all(ctx, start, |p| (p.0 + dx, p.1 + dy)),
            PendingAction::Nudge { dx, dy } => {
                self.base = None;
                self.move_all(ctx, true, |p| (p.0 + dx, p.1 + dy));
                self.base = None;
            }
            PendingAction::Rotate { angle, start } => {
                let Some(e) = &self.edit else { return };
                let q = match self.base { Some(b) if !start => b, _ => e.quad };
                let c = (q.iter().map(|p| p.0).sum::<f32>() / 4.0, q.iter().map(|p| p.1).sum::<f32>() / 4.0);
                let (sin, cos) = angle.sin_cos();
                self.move_all(ctx, start, |p| (c.0 + (p.0 - c.0) * cos - (p.1 - c.1) * sin, c.1 + (p.0 - c.0) * sin + (p.1 - c.1) * cos));
            }
            PendingAction::Reverse | PendingAction::DeleteSelected => {}
        }
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if button != PointerButton::Left {
            return;
        }
        if self.edit.is_none() {
            self.begin(ctx);
        }
        let Some(e) = &self.edit else { return };
        let p = (x as f32 + 0.5, y as f32 + 0.5);
        let r = (ctx.handle_radius as f32).max(1.5) * 1.25;
        let near = |q: (f32, f32)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt() <= r;
        let q = e.quad;
        let kind = if let Some(i) = (0..4).find(|&i| near(q[i])) {
            PDrag::Corner(i)
        } else if let Some(i) = (0..4).find(|&i| near(((q[i].0 + q[(i + 1) % 4].0) / 2.0, (q[i].1 + q[(i + 1) % 4].1) / 2.0))) {
            PDrag::Edge(i)
        } else {
            PDrag::Body
        };
        self.snapshot();
        self.drag = Some((kind, q, p.0, p.1));
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        let Some((kind, start, x0, y0)) = self.drag else { return };
        let (mut dx, mut dy) = (x as f32 + 0.5 - x0, y as f32 + 0.5 - y0);
        let mut q = start;
        match kind {
            PDrag::Corner(i) => {
                if mods.shift() {
                    // Along one axis only.
                    if dx.abs() > dy.abs() { dy = 0.0 } else { dx = 0.0 }
                }
                q[i] = (start[i].0 + dx, start[i].1 + dy);
                if mods.ctrl() {
                    // Symmetric: the opposite corner of the same edge moves the other way.
                    let j = i ^ 1;
                    q[j] = (start[j].0 - dx, start[j].1 + dy);
                }
            }
            PDrag::Edge(i) => {
                let j = (i + 1) % 4;
                if mods.shift() {
                    // Pure skew: along the edge only.
                    let (ex, ey) = (start[j].0 - start[i].0, start[j].1 - start[i].1);
                    let len2 = (ex * ex + ey * ey).max(1e-6);
                    let t = (dx * ex + dy * ey) / len2;
                    dx = ex * t;
                    dy = ey * t;
                }
                q[i] = (start[i].0 + dx, start[i].1 + dy);
                q[j] = (start[j].0 + dx, start[j].1 + dy);
            }
            PDrag::Body => q = start.map(|(a, b)| (a + dx, b + dy)),
        }
        self.set_quad(ctx, q, false);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, mods: Modifiers) {
        if self.drag.is_none() {
            return;
        }
        self.pointer_move(ctx, x, y, mods);
        let (_, start, _, _) = self.drag.take().unwrap();
        if self.edit.as_ref().is_some_and(|e| e.quad == start) {
            self.undo.pop(); // a click: nothing changed
        } else {
            self.render(ctx, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homography_maps_corners_and_inverts() {
        let q: Quad = [(10.0, 5.0), (90.0, 20.0), (70.0, 80.0), (5.0, 60.0)];
        let h = Homography::square_to_quad(&q);
        for (i, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().enumerate() {
            let (x, y) = h.apply(u, v).unwrap();
            assert!((x - q[i].0 as f64).abs() < 1e-6 && (y - q[i].1 as f64).abs() < 1e-6, "corner {i}");
        }
        let inv = h.inverse().unwrap();
        let (x, y) = h.apply(0.3, 0.7).unwrap();
        let (u, v) = inv.apply(x, y).unwrap();
        assert!((u - 0.3).abs() < 1e-9 && (v - 0.7).abs() < 1e-9);
    }

    #[test]
    fn convexity() {
        assert!(is_convex(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]));
        assert!(!is_convex(&[(0.0, 0.0), (10.0, 0.0), (2.0, 2.0), (0.0, 10.0)]));
        assert!(!is_convex(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)])); // crossed
    }
}
