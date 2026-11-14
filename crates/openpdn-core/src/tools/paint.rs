//! Painting tools: pencil, brush, eraser, bucket fill, colour picker,
//! clone stamp and recolour.

use super::{Extent, Modifiers, PointerButton, Tool, ToolContext, ToolKind};
use crate::brushes::{self, bresenham, FillPattern};
use crate::color::Bgra;
use crate::history::BitmapLayerEdit;
use crate::rect::PixelRect;
use crate::selection::{colors_match, scanline_flood};

fn push_bitmap_edit(ctx: &mut ToolContext, before: &[Bgra], rect: PixelRect, name: &str) {
    if let Some(e) = BitmapLayerEdit::capture(ctx.doc, ctx.layer, before, rect, name) {
        ctx.history.push(e);
    }
}

// ---------------------------------------------------------------- Pencil

#[derive(Default)]
pub struct PencilTool {
    drawing: bool,
    last: (i32, i32),
    ext: Option<Extent>,
    color: Bgra,
    before: Vec<Bgra>,
}

fn plot(ctx: &mut ToolContext, x: i32, y: i32, c: Bgra) {
    let (layer, sel) = ctx.layer_sel();
    let s = &mut layer.surface;
    if !s.in_bounds(x, y) {
        return;
    }
    if sel.is_some_and(|m| !m.contains(x, y)) {
        return;
    }
    s.set(x, y, c);
}

impl Tool for PencilTool {
    fn name(&self) -> &'static str {
        "Pencil"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Pencil
    }
    fn is_busy(&self) -> bool {
        self.drawing
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.drawing = true;
        self.before = ctx.layer_ref().surface.pixels().to_vec();
        self.color = ctx.settings.color_for(button);
        self.last = (x, y);
        self.ext = Some(Extent::point(x, y));
        plot(ctx, x, y, self.color);
        ctx.invalidate(PixelRect::new(x, y, 1, 1));
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        let c = self.color;
        let (lx, ly) = self.last;
        bresenham(lx, ly, x, y, |px, py| plot(ctx, px, py, c));
        self.ext.as_mut().unwrap().add(Extent::point(x, y));
        self.last = (x, y);
        ctx.invalidate(PixelRect::from_inclusive(lx.min(x), ly.min(y), lx.max(x), ly.max(y)));
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        let l = ctx.layer_ref();
        let r = self.ext.take().unwrap().clamp(l.width(), l.height());
        let before = std::mem::take(&mut self.before);
        push_bitmap_edit(ctx, &before, r, "Pencil");
    }
}

// ---------------------------------------------------------------- Brush / Eraser

/// Shared antialiased mask-stroke state for brush and eraser.
#[derive(Default)]
struct MaskStroke {
    drawing: bool,
    radius: f32,
    base_radius: f32,
    use_pressure: bool,
    aa: bool,
    last: (i32, i32),
    ext: Extent,
    before: Vec<Bgra>,
    mask: Vec<u8>,
}

impl Default for Extent {
    fn default() -> Self {
        Extent::EMPTY
    }
}

impl MaskStroke {
    fn pressure_radius(&self, pressure: f32) -> f32 {
        if self.use_pressure { 0.5f32.max(self.base_radius * pressure.clamp(0.0, 1.0)) } else { self.base_radius }
    }

    /// Starts a stroke; returns the first stamp's bounds.
    fn begin(&mut self, ctx: &mut ToolContext, x: i32, y: i32) -> (i32, i32, i32, i32) {
        self.drawing = true;
        let l = ctx.layer_ref();
        let (w, h) = (l.width(), l.height());
        self.before = l.surface.pixels().to_vec();
        self.mask = vec![0u8; w as usize * h as usize];
        self.base_radius = ctx.settings.brush_size() as f32 / 2.0;
        self.use_pressure = ctx.settings.pressure_sensitivity;
        self.radius = self.pressure_radius(ctx.pressure);
        self.aa = ctx.settings.antialiasing;
        self.last = (x, y);
        let b = brushes::stamp_disc_mask(&mut self.mask, w, h, x, y, self.radius, self.aa);
        self.ext = Extent::of(b);
        b
    }

    /// Extends the stroke; returns the dirty box (unclamped).
    fn extend(&mut self, ctx: &ToolContext, x: i32, y: i32) -> (i32, i32, i32, i32) {
        let l = ctx.layer_ref();
        let (w, h) = (l.width(), l.height());
        let r0 = self.radius;
        self.radius = self.pressure_radius(ctx.pressure);
        let (lx, ly) = self.last;
        brushes::stroke_disc_mask_tapered(&mut self.mask, w, h, lx, ly, x, y, r0, self.radius, self.aa);
        let r = r0.max(self.radius).ceil() as i32 + 1;
        let b = (lx.min(x) - r, ly.min(y) - r, lx.max(x) + r, ly.max(y) + r);
        self.ext.add(Extent::of(b));
        self.last = (x, y);
        b
    }

    fn finish(&mut self, ctx: &mut ToolContext, name: &str) {
        self.drawing = false;
        let l = ctx.layer_ref();
        let r = self.ext.clamp(l.width(), l.height());
        let before = std::mem::take(&mut self.before);
        self.mask = Vec::new();
        push_bitmap_edit(ctx, &before, r, name);
    }
}

#[derive(Default)]
pub struct BrushTool {
    s: MaskStroke,
    color: Bgra,
    alpha_blend: bool,
}

impl BrushTool {
    fn composite(&mut self, ctx: &mut ToolContext, b: (i32, i32, i32, i32)) {
        let (color, ab) = (self.color, self.alpha_blend);
        let (layer, sel) = ctx.layer_sel();
        brushes::composite_mask_paint(&mut layer.surface, &self.s.before, &self.s.mask, color, b.0, b.1, b.2, b.3, sel, ab);
        ctx.invalidate(PixelRect::from_inclusive(b.0, b.1, b.2, b.3));
    }
}

impl Tool for BrushTool {
    fn name(&self) -> &'static str {
        "Brush"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Brush
    }
    fn is_busy(&self) -> bool {
        self.s.drawing
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.color = ctx.settings.color_for(button);
        self.alpha_blend = ctx.settings.alpha_blending;
        let b = self.s.begin(ctx, x, y);
        self.composite(ctx, b);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.s.drawing {
            return;
        }
        let b = self.s.extend(ctx, x, y);
        self.composite(ctx, b);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if self.s.drawing {
            self.s.finish(ctx, "Brush");
        }
    }
}

#[derive(Default)]
pub struct EraserTool {
    s: MaskStroke,
    pattern: FillPattern,
}

impl EraserTool {
    fn composite(&mut self, ctx: &mut ToolContext, b: (i32, i32, i32, i32)) {
        let pattern = self.pattern;
        let (layer, sel) = ctx.layer_sel();
        brushes::composite_mask_erase(&mut layer.surface, &self.s.before, &self.s.mask, b.0, b.1, b.2, b.3, sel, pattern);
        ctx.invalidate(PixelRect::from_inclusive(b.0, b.1, b.2, b.3));
    }
}

impl Tool for EraserTool {
    fn name(&self) -> &'static str {
        "Eraser"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Eraser
    }
    fn is_busy(&self) -> bool {
        self.s.drawing
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if button != PointerButton::Left {
            return;
        }
        self.pattern = ctx.settings.eraser_pattern;
        let b = self.s.begin(ctx, x, y);
        self.composite(ctx, b);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.s.drawing {
            return;
        }
        let b = self.s.extend(ctx, x, y);
        self.composite(ctx, b);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if self.s.drawing {
            self.s.finish(ctx, "Eraser");
        }
    }
}

// ---------------------------------------------------------------- Fill

pub struct FillTool;

impl Tool for FillTool {
    fn name(&self) -> &'static str {
        "Fill"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Fill
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        let new_color = ctx.settings.color_for(button);
        let ctx_flood_global = ctx.settings.flood_mode == super::FloodMode::Global;
        let ctx_style = ctx.settings.fill_style;
        let (ctx_tolerance, ctx_alpha, ctx_fill_tolerance) = (ctx.settings.wand_tolerance(), ctx.settings.tolerance_alpha, ctx.settings.fill_tolerance);
        let sample_image = ctx.settings.sample_all_layers ^ mods.ctrl();
        let composite = sample_image.then(|| ctx.doc.composite());
        let (layer, sel) = ctx.layer_sel();
        let s = &mut layer.surface;
        if !s.in_bounds(x, y) || sel.is_some_and(|m| !m.contains(x, y)) {
            return;
        }
        let target = composite.as_ref().map_or_else(|| s.get(x, y), |c| c.get(x, y));
        if !sample_image && target == new_color && !ctx_fill_tolerance {
            return;
        }
        let (w, h) = (s.width(), s.height());
        let wu = w as usize;
        let before = s.pixels().to_vec();
        // Region = 4-connected pixels equal to `target` in the sampled image,
        // restricted to the selection. A scanline fill visits the same set as
        // the C# stack fill, so results are identical.
        let mut mask = vec![0u8; wu * h as usize];
        let sample: &[Bgra] = composite.as_ref().map_or(&before[..], |c| c.pixels());
        let selc = sel.map(|m| m.clip_mask());
        let selm = selc.as_deref();
        let (tol, alpha, tolerant) = (ctx_tolerance, ctx_alpha, ctx_fill_tolerance);
        let matches = |i: usize| {
            (if tolerant { colors_match(sample[i], target, tol, alpha) } else { sample[i] == target }) && selm.is_none_or(|m| m[i] != 0)
        };
        let r = if ctx_flood_global {
            // Global: every matching pixel in the image.
            let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
            for (i, m) in mask.iter_mut().enumerate() {
                if matches(i) {
                    *m = 255;
                    let (px, py) = ((i % wu) as i32, (i / wu) as i32);
                    x0 = x0.min(px);
                    x1 = x1.max(px);
                    y0 = y0.min(py);
                    y1 = y1.max(py);
                }
            }
            if x1 < x0 { PixelRect::EMPTY } else { PixelRect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1) }
        } else {
            scanline_flood(w, h, x, y, &mut mask, matches)
        };
        let px = s.pixels_mut();
        for yy in r.y..r.bottom() {
            let o = yy as usize * wu;
            for xx in r.x..r.right() {
                if mask[o + xx as usize] != 0 && ctx_style.on(xx, yy) {
                    px[o + xx as usize] = new_color;
                }
            }
        }
        let rect = PixelRect::clamp_box(r.x, r.y, r.right() - 1, r.bottom() - 1, w, h);
        push_bitmap_edit(ctx, &before, rect, "Fill");
        ctx.invalidate(rect);
    }
    fn pointer_move(&mut self, _: &mut ToolContext, _: i32, _: i32, _: Modifiers) {}
    fn pointer_up(&mut self, _: &mut ToolContext, _: i32, _: i32, _: PointerButton, _: Modifiers) {}
}

// ---------------------------------------------------------------- Colour picker

pub struct ColorPickerTool;

impl Tool for ColorPickerTool {
    fn name(&self) -> &'static str {
        "Eyedropper"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::ColorPicker
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) || !ctx.doc.bounds().contains(x, y) {
            return;
        }
        let sample_image = ctx.settings.sample_all_layers ^ mods.ctrl();
        let sampled = if sample_image {
            // Only the one pixel is needed.
            let mut px = [Bgra::TRANSPARENT];
            ctx.doc.composite_rect(PixelRect::new(x, y, 1, 1), &mut px);
            px[0]
        } else {
            ctx.layer_ref().surface.get(x, y)
        };
        if button == PointerButton::Left {
            ctx.settings.foreground = sampled;
        } else {
            ctx.settings.background = sampled;
        }
        ctx.settings.revision += 1;
    }
    fn pointer_move(&mut self, _: &mut ToolContext, _: i32, _: i32, _: Modifiers) {}
    fn pointer_up(&mut self, _: &mut ToolContext, _: i32, _: i32, _: PointerButton, _: Modifiers) {}
}

// ---------------------------------------------------------------- Clone stamp / Recolor

/// Interpolated stamps along a drag, shared by clone stamp and recolour.
fn stamp_along(last: (i32, i32), x: i32, y: i32, mut stamp: impl FnMut(i32, i32)) {
    let (lx, ly) = last;
    let steps = (x - lx).abs().max((y - ly).abs());
    for i in 1..=steps {
        let d = steps.max(1);
        stamp(lx + (x - lx) * i / d, ly + (y - ly) * i / d);
    }
}

pub struct CloneStampTool {
    src: (i32, i32),
    drawing: bool,
    off: (i32, i32),
    last: (i32, i32),
    before: Vec<Bgra>,
    ext: Extent,
    dirty: PixelRect,
}

impl Default for CloneStampTool {
    fn default() -> Self {
        CloneStampTool {
            src: (-1, -1),
            drawing: false,
            off: (0, 0),
            last: (0, 0),
            before: Vec::new(),
            ext: Extent::EMPTY,
            dirty: PixelRect::EMPTY,
        }
    }
}

impl CloneStampTool {
    /// The sampling origin set with Ctrl+click, if any.
    pub fn source(&self) -> Option<(i32, i32)> {
        (self.src.0 >= 0).then_some(self.src)
    }

    fn stamp(&mut self, ctx: &mut ToolContext, cx: i32, cy: i32) {
        let radius = brushes::radius_for_size(ctx.settings.brush_size());
        let (ox, oy) = self.off;
        let (layer, sel) = ctx.layer_sel();
        let sel = sel.filter(|s| !s.is_empty());
        let s = &mut layer.surface;
        let r2 = radius * radius + radius;
        let x0 = (cx - radius).max(0);
        let y0 = (cy - radius).max(0);
        let x1 = (s.width() - 1).min(cx + radius);
        let y1 = (s.height() - 1).min(cy + radius);
        for y in y0..=y1 {
            let dy = y - cy;
            for x in x0..=x1 {
                let dx = x - cx;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let (sx, sy) = (x + ox, y + oy);
                if !s.in_bounds(sx, sy) {
                    continue;
                }
                if sel.is_some_and(|m| !m.contains(x, y)) {
                    continue;
                }
                // Reads the live surface, so overlapping strokes re-clone (as in C#).
                let c = s.get(sx, sy);
                s.set(x, y, c);
            }
        }
        let e = Extent { min_x: cx - radius, min_y: cy - radius, max_x: cx + radius, max_y: cy + radius };
        self.ext.add(e);
        self.dirty = self.dirty.union(&PixelRect::from_inclusive(e.min_x, e.min_y, e.max_x, e.max_y));
    }
}

impl Tool for CloneStampTool {
    fn name(&self) -> &'static str {
        "Clone Stamp"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::CloneStamp
    }
    fn is_busy(&self) -> bool {
        self.drawing
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if button != PointerButton::Left {
            return;
        }
        if mods.ctrl() {
            self.src = (x, y);
            return;
        }
        if self.src.0 < 0 {
            return;
        }
        self.drawing = true;
        self.before = ctx.layer_ref().surface.pixels().to_vec();
        self.off = (self.src.0 - x, self.src.1 - y);
        self.last = (x, y);
        self.ext = Extent::EMPTY;
        self.dirty = PixelRect::EMPTY;
        self.stamp(ctx, x, y);
        let d = std::mem::take(&mut self.dirty);
        ctx.invalidate(d);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        let mut pts = Vec::new();
        stamp_along(self.last, x, y, |sx, sy| pts.push((sx, sy)));
        for (sx, sy) in pts {
            self.stamp(ctx, sx, sy);
        }
        self.last = (x, y);
        let d = std::mem::take(&mut self.dirty);
        ctx.invalidate(d);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        let l = ctx.layer_ref();
        let r = self.ext.clamp(l.width(), l.height());
        let before = std::mem::take(&mut self.before);
        push_bitmap_edit(ctx, &before, r, "Clone Stamp");
    }
}

pub struct RecolorTool {
    drawing: bool,
    last: (i32, i32),
    basis: Bgra,
    paint: Bgra,
    before: Vec<Bgra>,
    ext: Extent,
    dirty: PixelRect,
}

impl Default for RecolorTool {
    fn default() -> Self {
        RecolorTool {
            drawing: false,
            last: (0, 0),
            basis: Bgra::TRANSPARENT,
            paint: Bgra::TRANSPARENT,
            before: Vec::new(),
            ext: Extent::EMPTY,
            dirty: PixelRect::EMPTY,
        }
    }
}

impl RecolorTool {
    fn stamp(&mut self, ctx: &mut ToolContext, cx: i32, cy: i32) {
        let tol = ctx.settings.wand_tolerance();
        let radius = brushes::radius_for_size(ctx.settings.brush_size());
        let (basis, paint) = (self.basis, self.paint);
        let (layer, sel) = ctx.layer_sel();
        let sel = sel.filter(|s| !s.is_empty());
        let s = &mut layer.surface;
        let r2 = radius * radius + radius;
        let x0 = (cx - radius).max(0);
        let y0 = (cy - radius).max(0);
        let x1 = (s.width() - 1).min(cx + radius);
        let y1 = (s.height() - 1).min(cy + radius);
        for y in y0..=y1 {
            let dy = y - cy;
            let row = s.row_mut(y);
            for x in x0..=x1 {
                let dx = x - cx;
                if dx * dx + dy * dy > r2 || sel.is_some_and(|m| !m.contains(x, y)) {
                    continue;
                }
                let c = row[x as usize];
                if (c.b as i32 - basis.b as i32).abs() > tol
                    || (c.g as i32 - basis.g as i32).abs() > tol
                    || (c.r as i32 - basis.r as i32).abs() > tol
                {
                    continue;
                }
                row[x as usize] = paint.with_alpha(c.a);
            }
        }
        let e = Extent { min_x: cx - radius, min_y: cy - radius, max_x: cx + radius, max_y: cy + radius };
        self.ext.add(e);
        self.dirty = self.dirty.union(&PixelRect::from_inclusive(e.min_x, e.min_y, e.max_x, e.max_y));
    }
}

impl Tool for RecolorTool {
    fn name(&self) -> &'static str {
        "Recolor"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Recolor
    }
    fn is_busy(&self) -> bool {
        self.drawing
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.drawing = true;
        self.before = ctx.layer_ref().surface.pixels().to_vec();
        self.basis = ctx.settings.other_color_for(button);
        self.paint = ctx.settings.color_for(button);
        self.last = (x, y);
        self.ext = Extent::EMPTY;
        self.dirty = PixelRect::EMPTY;
        self.stamp(ctx, x, y);
        let d = std::mem::take(&mut self.dirty);
        ctx.invalidate(d);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        let mut pts = Vec::new();
        stamp_along(self.last, x, y, |sx, sy| pts.push((sx, sy)));
        for (sx, sy) in pts {
            self.stamp(ctx, sx, sy);
        }
        self.last = (x, y);
        let d = std::mem::take(&mut self.dirty);
        ctx.invalidate(d);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        let l = ctx.layer_ref();
        let r = self.ext.clamp(l.width(), l.height());
        let before = std::mem::take(&mut self.before);
        push_bitmap_edit(ctx, &before, r, "Recolor");
    }
}
