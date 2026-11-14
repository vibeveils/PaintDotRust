//! Move Selected Pixels (C# `MoveTool`): drag to translate, handles to scale.

use super::handles::{self, HandleKind};
use super::{Modifiers, PointerButton, Tool, ToolContext, ToolKind};
use crate::color::Bgra;
use crate::document::Document;
use crate::history::{BitmapLayerEdit, CompoundEdit, HistoryEntry, SelectionEdit};
use crate::rect::{PixelRect, RectF};
use crate::resample::{self, TransformResampling};
use crate::selection::Selection;
use crate::surface::Surface;

#[derive(Default)]
pub struct MoveTool {
    dragging: bool,
    x0: i32,
    y0: i32,
    /// Full-layer snapshot at drag start.
    original: Vec<Bgra>,
    selection_before: Option<Selection>,
    handle: Option<HandleKind>,
    start_rect: RectF,
    aspect: f64,
    live_rect: Option<RectF>,
    mode: TransformResampling,
    gamma: bool,
    /// Area written by the previous frame, to restore before the next.
    prev: PixelRect,
}

fn clear_selected(dst: &mut Surface, sel: &Selection) {
    let b = sel.bounds();
    for y in b.y..b.bottom() {
        let row = dst.row_mut(y);
        for x in b.x..b.right() {
            if sel.contains(x, y) {
                row[x as usize] = Bgra::TRANSPARENT;
            }
        }
    }
}

impl MoveTool {
    fn apply(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        let dx = x - self.x0;
        let dy = y - self.y0;
        let handle = self.handle.unwrap();
        let mode = resample::Resampling { mode: self.mode, linear: self.gamma };
        let start = self.start_rect;
        let aspect = self.aspect;
        let original = &self.original;
        let sel_before = self.selection_before.as_ref();
        let layer = ctx.doc.layer_mut(ctx.layer).expect("active layer missing");
        let s = &mut layer.surface;
        let (w, h) = (s.width(), s.height());
        let full = s.bounds();

        // Restore what the previous frame changed to "original minus lifted pixels".
        let prev = self.prev;
        s.copy_rect_from(original, prev);
        if let Some(sb) = sel_before {
            let r = prev.intersect(&sb.bounds());
            for yy in r.y..r.bottom() {
                let row = s.row_mut(yy);
                for xx in r.x..r.right() {
                    if sb.contains(xx, yy) {
                        row[xx as usize] = Bgra::TRANSPARENT;
                    }
                }
            }
        }

        let mut touched;
        if handle == HandleKind::Move {
            self.live_rect = Some(RectF { x: start.x + dx as f64, y: start.y + dy as f64, ..start });
            match sel_before {
                Some(sb) => {
                    let b = sb.bounds();
                    let ys0 = b.y.max(-dy);
                    let ys1 = b.bottom().min(h - dy);
                    let xs0 = b.x.max(-dx);
                    let xs1 = b.right().min(w - dx);
                    touched = PixelRect::EMPTY;
                    if ys0 < ys1 && xs0 < xs1 {
                        let wu = w as usize;
                        for sy in ys0..ys1 {
                            let row = s.row_mut(sy + dy);
                            for sx in xs0..xs1 {
                                if sb.contains(sx, sy) {
                                    row[(sx + dx) as usize] = original[sy as usize * wu + sx as usize];
                                }
                            }
                        }
                        touched = PixelRect::new(xs0 + dx, ys0 + dy, xs1 - xs0, ys1 - ys0);
                    }
                }
                None => {
                    // Whole layer: translate; vacated area becomes transparent.
                    s.clear(Bgra::TRANSPARENT);
                    let ys0 = (-dy).max(0);
                    let ys1 = h.min(h - dy);
                    let xs0 = (-dx).max(0);
                    let xs1 = w.min(w - dx);
                    if ys0 < ys1 && xs0 < xs1 {
                        let wu = w as usize;
                        let n = (xs1 - xs0) as usize;
                        for sy in ys0..ys1 {
                            let so = sy as usize * wu + xs0 as usize;
                            let d0 = (xs0 + dx) as usize;
                            s.row_mut(sy + dy)[d0..d0 + n].copy_from_slice(&original[so..so + n]);
                        }
                    }
                    touched = full;
                }
            }
            if let Some(sb) = sel_before {
                let mut live = ctx.doc.take_selection().unwrap_or_else(|| sb.clone());
                live.copy_translated_from(sb, dx, dy);
                ctx.doc.set_selection(Some(live));
            }
        } else {
            let target = handles::resize(start, handle, dx as f64, dy as f64, mods.shift(), aspect);
            self.live_rect = Some(target);
            let src_rect = sel_before.map_or(full, |sb| sb.bounds());
            if sel_before.is_none() {
                s.clear(Bgra::TRANSPARENT);
            }
            // Sample-and-lerp over the base (layer without the lifted pixels).
            let clip = sel_before.map(|sb| sb.clip_mask());
            let mask = clip.as_deref();
            let wu = w as usize;
            let mut writes: Vec<(u32, Bgra, f32)> = Vec::new();
            resample::resample(original, w, mask, src_rect, target, w, h, mode, |px, py, c, cov| {
                writes.push(((py as usize * wu + px as usize) as u32, c, cov));
            });
            let pix = s.pixels_mut();
            for (i, c, cov) in writes {
                let i = i as usize;
                pix[i] = resample::lerp_by_coverage(pix[i], c, cov);
            }
            touched = if sel_before.is_none() { full } else { target.to_covering_pixel_rect().intersect(&full) };
            if let Some(sb) = sel_before {
                let mut live = ctx.doc.take_selection().unwrap_or_else(|| sb.clone());
                live.set_scaled_from(sb, target);
                ctx.doc.set_selection(Some(live));
            }
        }
        if sel_before.is_none() {
            touched = full;
        }
        self.prev = touched;
        ctx.doc.invalidate(Some(prev.union(&touched)));
    }
}

impl Tool for MoveTool {
    fn name(&self) -> &'static str {
        "Move"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::MoveSelected
    }
    fn is_busy(&self) -> bool {
        self.dragging
    }
    fn handle_bounds(&self, doc: &Document) -> Option<RectF> {
        if self.dragging {
            if let Some(r) = self.live_rect {
                return Some(r);
            }
        }
        Some(doc.active_selection().map_or(RectF::new(0.0, 0.0, doc.width() as f64, doc.height() as f64), |s| {
            RectF::from_pixel(s.bounds())
        }))
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if button != PointerButton::Left {
            return;
        }
        self.dragging = true;
        let sel = ctx.doc.active_selection().cloned();
        let layer = ctx.doc.layer_mut(ctx.layer).expect("active layer missing");
        self.original = layer.surface.pixels().to_vec();
        self.x0 = x;
        self.y0 = y;
        self.mode = ctx.settings.resampling;
        self.gamma = ctx.settings.gamma_correct;
        self.start_rect = sel.as_ref().map_or(RectF::new(0.0, 0.0, layer.width() as f64, layer.height() as f64), |s| {
            RectF::from_pixel(s.bounds())
        });
        self.aspect = self.start_rect.w / self.start_rect.h.max(1e-6);
        let mut h = handles::hit_test(self.start_rect, x as f64 + 0.5, y as f64 + 0.5, ctx.handle_radius.max(1.0));
        if h == HandleKind::None {
            h = HandleKind::Move;
        }
        self.handle = Some(h);
        self.live_rect = Some(self.start_rect);
        self.prev = PixelRect::EMPTY;
        if let Some(s) = &sel {
            // Lift the selected pixels off the layer.
            clear_selected(&mut layer.surface, s);
            ctx.doc.invalidate(Some(s.bounds()));
        }
        self.selection_before = sel;
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        if self.dragging {
            self.apply(ctx, x, y, mods);
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, mods: Modifiers) {
        if !self.dragging {
            return;
        }
        self.apply(ctx, x, y, mods);
        self.dragging = false;
        let changed = self.live_rect.is_some_and(|r| r != self.start_rect);
        let original = std::mem::take(&mut self.original);
        if changed {
            let (w, h) = {
                let l = ctx.layer_ref();
                (l.width(), l.height())
            };
            let mut entries: Vec<Box<dyn HistoryEntry>> = Vec::new();
            if let Some(e) = BitmapLayerEdit::capture(ctx.doc, ctx.layer, &original, PixelRect::new(0, 0, w, h), "Move") {
                entries.push(Box::new(e));
            }
            if let (Some(before), Some(after)) = (self.selection_before.take(), ctx.doc.selection().cloned()) {
                entries.push(Box::new(SelectionEdit::new(Some(before), Some(after), "Move")));
            }
            if entries.len() == 1 {
                ctx.history.push_boxed(entries.pop().unwrap());
            } else if !entries.is_empty() {
                ctx.history.push(CompoundEdit::new(entries, "Move"));
            }
        } else if let Some(l) = ctx.doc.layer_mut(ctx.layer) {
            l.surface.pixels_mut().copy_from_slice(&original);
            ctx.doc.invalidate(None);
        }
        self.selection_before = None;
        self.live_rect = None;
    }
}
