//! Selection tools.

use super::handles::{self, HandleKind};
use super::{FloodMode, Modifiers, PointerButton, SelectSize, Tool, ToolContext, ToolKind};
use crate::document::Document;
use crate::history::SelectionEdit;
use crate::rect::RectF;
use crate::selection::{Selection, SelectionCombineMode};

/// Modifier overrides of the toolbar mode, as in Paint.NET:
/// left button: Ctrl = union, Alt = subtract; right button: Ctrl = xor,
/// Alt = intersect. Ctrl+Alt (either button) = intersect.
pub fn combine_mode_from(mods: Modifiers, button: PointerButton, default: SelectionCombineMode) -> SelectionCombineMode {
    let right = button == PointerButton::Right;
    match (mods.ctrl(), mods.alt()) {
        (true, true) => SelectionCombineMode::Intersect,
        (true, false) if right => SelectionCombineMode::Xor,
        (true, false) => SelectionCombineMode::Union,
        (false, true) if right => SelectionCombineMode::Intersect,
        (false, true) => SelectionCombineMode::Subtract,
        _ => default,
    }
}

/// Selection drag rectangle honouring the size mode (`Any` = [`rect_from_drag`]).
pub fn select_rect_from_drag(ax: i32, ay: i32, bx: i32, by: i32, mods: Modifiers, size: SelectSize) -> (i32, i32, i32, i32) {
    let (dx, dy) = (bx - ax, by - ay);
    let (sx, sy) = (if dx < 0 { -1 } else { 1 }, if dy < 0 { -1 } else { 1 });
    let place = |w: i32, h: i32| (if sx > 0 { ax } else { ax - w + 1 }, if sy > 0 { ay } else { ay - h + 1 }, w, h);
    match size {
        SelectSize::Any => rect_from_drag(ax, ay, bx, by, mods),
        SelectSize::FixedSize(w, h) => {
            let (w, h) = (w.max(1), h.max(1));
            place(w, h)
        }
        SelectSize::FixedRatio(rw, rh) => {
            // Largest rectangle of the ratio that fits the dragged box.
            let ratio = if rw > 0.0 && rh > 0.0 { rw / rh } else { 1.0 };
            let (w0, h0) = ((dx.abs() + 1) as f64, (dy.abs() + 1) as f64);
            let (w, h) = if w0 / h0 > ratio { (h0 * ratio, h0) } else { (w0, w0 / ratio) };
            place((w.round() as i32).max(1), (h.round() as i32).max(1))
        }
    }
}

/// Drag rectangle from anchor to pointer, inclusive; Shift makes it square.
pub fn rect_from_drag(ax: i32, ay: i32, mut bx: i32, mut by: i32, mods: Modifiers) -> (i32, i32, i32, i32) {
    let dx = bx - ax;
    let dy = by - ay;
    if mods.shift() {
        let size = dx.abs().min(dy.abs());
        bx = ax + (if dx == 0 { 1 } else { dx }).signum() * size;
        by = ay + (if dy == 0 { 1 } else { dy }).signum() * size;
    }
    (ax.min(bx), ay.min(by), (bx - ax).abs() + 1, (by - ay).abs() + 1)
}

fn set_combined(doc: &mut Document, base: Option<&Selection>, shape: &Selection, mode: SelectionCombineMode) {
    let combined = Selection::combine(base, shape, mode);
    doc.set_selection(if combined.is_empty() { None } else { Some(combined) });
}

/// Shared state for drag-shaped selections.
#[derive(Default)]
struct DragSelect {
    active: bool,
    x0: i32,
    y0: i32,
    shape: Option<Selection>,
    base: Option<Selection>,
    mode: SelectionCombineMode,
    history_before: Option<Selection>,
}

impl DragSelect {
    fn begin(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        self.active = true;
        self.x0 = x;
        self.y0 = y;
        self.shape = Some(Selection::new(ctx.doc.width(), ctx.doc.height()));
        self.mode = combine_mode_from(mods, button, ctx.settings.selection_combine_mode);
        self.history_before = ctx.doc.selection().cloned();
        self.base = if self.mode == SelectionCombineMode::Replace { None } else { self.history_before.clone() };
    }

    fn update(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers, ellipse: bool) {
        let (rx, ry, rw, rh) = select_rect_from_drag(self.x0, self.y0, x, y, mods, ctx.settings.select_size);
        let shape = self.shape.as_mut().unwrap();
        if ellipse {
            shape.set_ellipse(rx, ry, rw, rh);
        } else {
            shape.set_rectangle(rx, ry, rw, rh);
        }
        set_combined(ctx.doc, self.base.as_ref(), shape, self.mode);
    }

    fn end(&mut self, ctx: &mut ToolContext, x: i32, y: i32, name: &str) {
        self.active = false;
        // (A fixed-size selection is made by a single click.)
        if x == self.x0 && y == self.y0 && !matches!(ctx.settings.select_size, SelectSize::FixedSize(..)) {
            // A click without a drag deselects in Replace mode, else restores the base.
            if self.mode == SelectionCombineMode::Replace {
                ctx.doc.set_selection(None);
            } else {
                ctx.doc.set_selection(self.base.clone());
            }
        }
        let after = ctx.doc.selection().cloned();
        let before = self.history_before.take();
        if before.is_some() || after.is_some() {
            ctx.history.push(SelectionEdit::new(before, after, name));
        }
        self.base = None;
    }
}

#[derive(Default)]
pub struct RectangleSelectTool(DragSelect);

impl Tool for RectangleSelectTool {
    fn name(&self) -> &'static str {
        "Rect Select"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::RectangleSelect
    }
    fn is_busy(&self) -> bool {
        self.0.active
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.0.begin(ctx, x, y, button, mods);
        self.0.update(ctx, x, y, mods, false);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        if self.0.active {
            self.0.update(ctx, x, y, mods, false);
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, _m: Modifiers) {
        if self.0.active {
            self.0.end(ctx, x, y, "Rect Select");
        }
    }
}

#[derive(Default)]
pub struct EllipseSelectTool(DragSelect);

impl Tool for EllipseSelectTool {
    fn name(&self) -> &'static str {
        "Ellipse Select"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::EllipseSelect
    }
    fn is_busy(&self) -> bool {
        self.0.active
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.0.begin(ctx, x, y, button, mods);
        self.0.update(ctx, x, y, mods, true);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        if self.0.active {
            self.0.update(ctx, x, y, mods, true);
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, _m: Modifiers) {
        if self.0.active {
            self.0.end(ctx, x, y, "Ellipse Select");
        }
    }
}

#[derive(Default)]
pub struct LassoSelectTool {
    drawing: bool,
    points: Vec<(i32, i32)>,
    shape: Option<Selection>,
    base: Option<Selection>,
    mode: SelectionCombineMode,
    history_before: Option<Selection>,
}

impl LassoSelectTool {
    fn apply_shape(&mut self, ctx: &mut ToolContext, antialias: bool) {
        let shape = self.shape.as_mut().unwrap();
        shape.set_polygon(&self.points, antialias);
        set_combined(ctx.doc, self.base.as_ref(), shape, self.mode);
    }

    /// Points collected so far (for drawing the in-progress outline).
    pub fn points(&self) -> &[(i32, i32)] {
        &self.points
    }
}

impl Tool for LassoSelectTool {
    fn name(&self) -> &'static str {
        "Lasso"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::LassoSelect
    }
    fn is_busy(&self) -> bool {
        self.drawing
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.drawing = true;
        self.points.clear();
        self.points.push((x, y));
        self.shape = Some(Selection::new(ctx.doc.width(), ctx.doc.height()));
        self.mode = combine_mode_from(mods, button, ctx.settings.selection_combine_mode);
        self.history_before = ctx.doc.selection().cloned();
        self.base = if self.mode == SelectionCombineMode::Replace { None } else { self.history_before.clone() };
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        if self.points.last() == Some(&(x, y)) {
            return;
        }
        self.points.push((x, y));
        // Refresh the hard-edged preview every few points.
        if self.points.len() % 4 == 0 {
            self.apply_shape(ctx, false);
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if !self.drawing {
            return;
        }
        self.drawing = false;
        if self.points.len() < 3 {
            if self.mode == SelectionCombineMode::Replace {
                ctx.doc.set_selection(None);
            } else {
                ctx.doc.set_selection(self.base.clone());
            }
        } else {
            self.apply_shape(ctx, true);
        }
        let after = ctx.doc.selection().cloned();
        let before = self.history_before.take();
        if before.is_some() || after.is_some() {
            ctx.history.push(SelectionEdit::new(before, after, "Lasso"));
        }
        self.base = None;
        self.points.clear();
    }
}

pub struct MagicWandTool;

impl Tool for MagicWandTool {
    fn name(&self) -> &'static str {
        "Magic Wand"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::MagicWand
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        let doc = &mut *ctx.doc;
        if !doc.bounds().contains(x, y) {
            return;
        }
        let st = &ctx.settings;
        let mut shape = Selection::new(doc.width(), doc.height());
        let global = st.flood_mode == FloodMode::Global;
        let layer_index = doc.layer_index(ctx.layer);
        match layer_index.filter(|_| !st.wand_sample_image) {
            Some(i) => shape.set_by_color(&doc.layers[i].surface, x, y, st.wand_tolerance(), st.tolerance_alpha, global),
            None => shape.set_by_color(&doc.composite(), x, y, st.wand_tolerance(), st.tolerance_alpha, global),
        }
        let mode = combine_mode_from(mods, button, ctx.settings.selection_combine_mode);
        let before = doc.selection().cloned();
        let base = if mode == SelectionCombineMode::Replace { None } else { before.as_ref() };
        let combined = Selection::combine(base, &shape, mode);
        if combined.is_empty() {
            doc.set_selection(None);
            ctx.history.push(SelectionEdit::new(before, None, "Magic Wand"));
        } else {
            doc.set_selection(Some(combined.clone()));
            ctx.history.push(SelectionEdit::new(before, Some(combined), "Magic Wand"));
        }
    }
    fn pointer_move(&mut self, _: &mut ToolContext, _: i32, _: i32, _: Modifiers) {}
    fn pointer_up(&mut self, _: &mut ToolContext, _: i32, _: i32, _: PointerButton, _: Modifiers) {}
}

#[derive(Default)]
pub struct MoveSelectionTool {
    dragging: bool,
    x0: i32,
    y0: i32,
    original: Option<Selection>,
    handle: Option<HandleKind>,
    start_rect: RectF,
    aspect: f64,
    live_rect: Option<RectF>,
}

impl MoveSelectionTool {
    fn apply(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        let dx = x - self.x0;
        let dy = y - self.y0;
        let original = self.original.as_ref().unwrap();
        let mut live = ctx.doc.take_selection().unwrap_or_else(|| original.clone());
        if self.handle == Some(HandleKind::Move) {
            self.live_rect = Some(RectF { x: self.start_rect.x + dx as f64, y: self.start_rect.y + dy as f64, ..self.start_rect });
            live.copy_translated_from(original, dx, dy);
        } else {
            let target = handles::resize(self.start_rect, self.handle.unwrap(), dx as f64, dy as f64, mods.shift(), self.aspect);
            self.live_rect = Some(target);
            live.set_scaled_from(original, target);
        }
        ctx.doc.set_selection(Some(live));
    }
}

impl Tool for MoveSelectionTool {
    fn name(&self) -> &'static str {
        "Move Selection"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::MoveSelection
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
        doc.active_selection().map(|s| RectF::from_pixel(s.bounds()))
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if button != PointerButton::Left {
            return;
        }
        let Some(sel) = ctx.doc.active_selection() else { return };
        self.dragging = true;
        self.x0 = x;
        self.y0 = y;
        self.start_rect = RectF::from_pixel(sel.bounds());
        self.original = Some(sel.clone());
        self.aspect = self.start_rect.w / self.start_rect.h.max(1e-6);
        let mut h = handles::hit_test(self.start_rect, x as f64 + 0.5, y as f64 + 0.5, ctx.handle_radius.max(1.0));
        if h == HandleKind::None {
            h = HandleKind::Move;
        }
        self.handle = Some(h);
        self.live_rect = Some(self.start_rect);
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
        if self.live_rect.is_some_and(|r| r != self.start_rect) {
            let after = ctx.doc.selection().cloned();
            ctx.history.push(SelectionEdit::new(self.original.take(), after, "Move Selection"));
        }
        self.original = None;
        self.live_rect = None;
    }
}
