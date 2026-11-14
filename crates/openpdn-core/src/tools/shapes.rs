//! Shape tools: line, rectangle, rounded rectangle, ellipse, freeform.
//!
//! Shapes redraw from the stroke-start pixels on every move. Only the area
//! drawn last time is restored (the C# port restored the whole layer), which
//! gives identical pixels at a fraction of the memory traffic.

use super::select::rect_from_drag;
use super::shape_geom::{self, ShapeType};
use super::{EditOverlay, Modifiers, PendingAction, PointerButton, Tool, ToolContext, ToolKind};
use crate::color::Bgra;
use crate::draw;
use crate::history::BitmapLayerEdit;
use crate::num::round_even_f;
use crate::rect::PixelRect;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShapeDrawMode {
    #[default]
    Outline,
    Fill,
    FillOutline,
}

impl ShapeDrawMode {
    fn fills(self) -> bool {
        matches!(self, ShapeDrawMode::Fill | ShapeDrawMode::FillOutline)
    }
    fn outlines(self) -> bool {
        matches!(self, ShapeDrawMode::Outline | ShapeDrawMode::FillOutline)
    }
}

/// Stroke-start snapshot plus the region last drawn over it.
#[derive(Default)]
pub(crate) struct Redraw {
    pub before: Vec<Bgra>,
    pub drawn: PixelRect,
}

impl Redraw {
    pub fn begin(&mut self, ctx: &ToolContext) {
        self.before = ctx.layer_ref().surface.pixels().to_vec();
        self.drawn = PixelRect::EMPTY;
    }

    /// Restores the previously drawn area, runs `paint`, records the new area.
    pub fn redraw(&mut self, ctx: &mut ToolContext, paint: impl FnOnce(&mut crate::Surface, Option<&crate::Selection>) -> PixelRect) {
        let prev = self.drawn;
        let (layer, sel) = ctx.layer_sel();
        let sel = sel.filter(|s| !s.is_empty());
        layer.surface.copy_rect_from(&self.before, prev);
        let r = paint(&mut layer.surface, sel);
        self.drawn = r;
        ctx.invalidate(prev.union(&r));
    }

    /// Puts back the stroke-start pixels.
    pub fn cancel(&mut self, ctx: &mut ToolContext) {
        let (layer, _) = ctx.layer_sel();
        layer.surface.copy_rect_from(&self.before, self.drawn);
        ctx.invalidate(self.drawn);
        self.before = Vec::new();
        self.drawn = PixelRect::EMPTY;
    }

    pub fn commit(&mut self, ctx: &mut ToolContext, name: &str) {
        let before = std::mem::take(&mut self.before);
        if let Some(e) = BitmapLayerEdit::capture(ctx.doc, ctx.layer, &before, self.drawn, name) {
            ctx.history.push(e);
        }
        ctx.invalidate(self.drawn);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    /// The Shape tool: any [`ShapeType`], chosen in the toolbar.
    Any,
}

/// A box shape's geometry: centre, size (pixel-edge coordinates) and rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxGeom {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    /// Radians, clockwise on screen.
    pub angle: f32,
}

impl BoxGeom {
    fn from_rect(x: i32, y: i32, w: i32, h: i32) -> Self {
        BoxGeom { cx: x as f32 + w as f32 / 2.0, cy: y as f32 + h as f32 / 2.0, w: w as f32, h: h as f32, angle: 0.0 }
    }
    /// Shape-frame coordinates of an image point (pixel-edge coordinates).
    fn to_local(&self, x: f32, y: f32) -> (f32, f32) {
        let (sin, cos) = self.angle.sin_cos();
        let (dx, dy) = (x - self.cx, y - self.cy);
        (dx * cos + dy * sin, -dx * sin + dy * cos)
    }
    fn to_world(&self, u: f32, v: f32) -> (f32, f32) {
        let (sin, cos) = self.angle.sin_cos();
        (self.cx + u * cos - v * sin, self.cy + u * sin + v * cos)
    }
    /// The axis-aligned pixel rectangle, if the shape is unrotated and whole-pixel.
    fn pixel_rect(&self) -> Option<PixelRect> {
        let (x, y) = (self.cx - self.w / 2.0, self.cy - self.h / 2.0);
        let whole = |v: f32| (v - v.round()).abs() < 1e-4;
        (self.angle == 0.0 && whole(x) && whole(y) && whole(self.w) && whole(self.h))
            .then(|| PixelRect::new(x.round() as i32, y.round() as i32, self.w.round() as i32, self.h.round() as i32))
    }
}

/// The 8 resize handles as (sx, sy) in {-1, 0, 1}².
pub const BOX_HANDLES: [(i8, i8); 8] = [(-1, -1), (0, -1), (1, -1), (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0)];
/// Distance of the rotation handle above the top edge, in handle radii.
pub const ROTATE_HANDLE_OFFSET: f32 = 4.0;

#[derive(Clone, Copy, Debug)]
enum BoxDrag {
    Create,
    Move { g0: BoxGeom, x0: f32, y0: f32 },
    /// (pu0, pv0): the pointer in the shape's frame when the drag began.
    Resize { g0: BoxGeom, sx: i8, sy: i8, pu0: f32, pv0: f32 },
    Rotate { g0: BoxGeom, a0: f32 },
    /// A corner of a custom polygon.
    Vertex(usize),
}

/// A shape being edited: its box, type and (custom polygon) corners in unit-box coordinates.
#[derive(Clone, Debug, PartialEq)]
struct ShapeEdit {
    g: BoxGeom,
    ty: ShapeType,
    custom: Vec<(f32, f32)>,
}

/// Drag-a-box shapes (C# `ShapeToolBase` subclasses), and the Shape tool,
/// which draws any [`ShapeType`].
pub struct ShapeTool {
    kind: ShapeKind,
    dragging: bool,
    anchor: (i32, i32),
    button: PointerButton,
    r: Redraw,
    /// The shape being edited (with `ToolSettings::editable_shapes`).
    edit: Option<ShapeEdit>,
    drag: Option<BoxDrag>,
    base: Option<BoxGeom>,
    undo: Vec<ShapeEdit>,
    redo: Vec<ShapeEdit>,
}

impl ShapeTool {
    pub fn new(kind: ShapeKind) -> Self {
        ShapeTool {
            kind,
            dragging: false,
            anchor: (0, 0),
            button: PointerButton::Left,
            r: Redraw::default(),
            edit: None,
            drag: None,
            base: None,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// The type to draw: fixed for the classic tools, the toolbar's for Shape.
    fn shape_type(&self, ctx: &ToolContext) -> ShapeType {
        match self.kind {
            ShapeKind::Rectangle => ShapeType::Rectangle,
            ShapeKind::RoundedRectangle => ShapeType::RoundedRectangle,
            ShapeKind::Ellipse => ShapeType::Ellipse,
            ShapeKind::Any => ctx.settings.shape_type,
        }
    }

    fn snapshot(&mut self) {
        if let Some(e) = &self.edit {
            self.undo.push(e.clone());
            self.redo.clear();
        }
    }

    fn redraw(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        let (rx, ry, rw, rh) = rect_from_drag(self.anchor.0, self.anchor.1, x, y, mods);
        let ty = self.shape_type(ctx);
        let e = ShapeEdit { g: BoxGeom::from_rect(rx, ry, rw, rh), ty, custom: Vec::new() };
        self.render(ctx, &e);
    }

    fn render(&mut self, ctx: &mut ToolContext, e: &ShapeEdit) {
        let st = &ctx.settings;
        let outline = st.color_for(self.button);
        let fill = st.other_color_for(self.button);
        let mode = st.shape_mode;
        let pen = st.brush_size() as f32;
        let (aa, ab) = (st.antialiasing, st.alpha_blending);
        let (sides, inner, radius, style) = (st.shape_sides as usize, st.star_inner, st.shape_radius, st.fill_style);
        let (f, o) = (mode.fills().then_some(fill), mode.outlines().then_some(outline));
        let (doc_w, doc_h) = (ctx.doc.width(), ctx.doc.height());
        let e = e.clone();
        let g = e.g;
        self.r.redraw(ctx, move |s, sel| {
            // One pass draws `fill` and `outline` with clip `sel`.
            let draw_pass = |s: &mut crate::Surface, f: Option<Bgra>, o: Option<Bgra>, sel: Option<&crate::Selection>| -> PixelRect {
                let rect = g.pixel_rect();
                match e.ty {
                    ShapeType::Rectangle if rect.is_some() => {
                        let rect = rect.unwrap();
                        let mut affected = PixelRect::EMPTY;
                        if let Some(fc) = f {
                            affected = draw::fill_rect(s, rect, fc, sel, ab);
                        }
                        if let Some(oc) = o {
                            affected = affected.union(&draw::rect_outline(s, rect, pen as i32, oc, sel, ab));
                        }
                        affected
                    }
                    ShapeType::RoundedRectangle | ShapeType::Ellipse | ShapeType::Terminator => {
                        let rad = if e.ty == ShapeType::Terminator { g.w.min(g.h) / 2.0 } else { radius };
                        match (rect, e.ty) {
                            (Some(r), ShapeType::Ellipse) => draw::ellipse(s, r, f, o, pen, sel, aa, ab),
                            (Some(r), _) => draw::rounded_rect(s, r, rad, f, o, pen, sel, aa, ab),
                            (None, t) => {
                                let shape = if t == ShapeType::Ellipse { draw::BoxShape::Ellipse } else { draw::BoxShape::RoundedRectangle(rad) };
                                draw::rotated_shape(s, shape, g.cx, g.cy, g.w, g.h, g.angle, f, o, pen, sel, aa, ab)
                            }
                        }
                    }
                    t => {
                        let geom = shape_geom::geometry(t, sides, inner, &e.custom);
                        let world = |p: &(f32, f32)| g.to_world(p.0 * g.w, p.1 * g.h);
                        let contours: Vec<Vec<(f32, f32)>> = geom.contours.iter().map(|c| c.iter().map(world).collect()).collect();
                        let details: Vec<Vec<(f32, f32)>> = geom.details.iter().map(|c| c.iter().map(world).collect()).collect();
                        let r = draw::polygon_shape(s, &contours, &details, f, o, pen, sel, aa, ab);
                        r
                    }
                }
            };
            if style == draw::FillStyle::Solid || f.is_none() {
                draw_pass(s, f, o, sel)
            } else {
                // Patterned fill: clip the fill to the pattern (and selection), then outline.
                let (sin, cos) = g.angle.sin_cos();
                let ex = (g.w / 2.0 * cos).abs() + (g.h / 2.0 * sin).abs() + 2.0;
                let ey = (g.w / 2.0 * sin).abs() + (g.h / 2.0 * cos).abs() + 2.0;
                let area = PixelRect::new((g.cx - ex).floor() as i32, (g.cy - ey).floor() as i32, (2.0 * ex).ceil() as i32 + 1, (2.0 * ey).ceil() as i32 + 1);
                let clip = crate::Selection::pattern_clip(doc_w, doc_h, area, style, sel);
                let a = draw_pass(s, f, None, Some(&clip));
                a.union(&if o.is_some() { draw_pass(s, None, o, sel) } else { PixelRect::EMPTY })
            }
        });
    }

    fn render_edit(&mut self, ctx: &mut ToolContext) {
        let Some(mut e) = self.edit.clone() else { return };
        // The Shape tool follows the toolbar's type, converting to a custom polygon on demand.
        let ty = self.shape_type(ctx);
        if ty != e.ty {
            if ty == ShapeType::CustomPolygon && e.custom.len() < 3 {
                let st = &ctx.settings;
                let geom = shape_geom::geometry(e.ty, st.shape_sides as usize, st.star_inner, &[]);
                e.custom = match e.ty {
                    ShapeType::Ellipse | ShapeType::RoundedRectangle | ShapeType::Terminator => shape_geom::regular(st.shape_sides as usize, None),
                    _ => geom.contours.into_iter().next().unwrap_or_default(),
                };
            }
            e.ty = ty;
            self.edit = Some(e.clone());
        }
        self.render(ctx, &e);
    }

    fn commit_edit(&mut self, ctx: &mut ToolContext) {
        if self.edit.take().is_some() {
            self.drag = None;
            self.base = None;
            self.undo.clear();
            self.redo.clear();
            let name = self.history_name();
            self.r.commit(ctx, name);
        }
    }

    fn history_name(&self) -> &'static str {
        match self.kind {
            ShapeKind::Any => "Shape",
            _ => self.name(),
        }
    }

    /// Which part of the shape being edited is under (x, y).
    fn hit(&self, e: &ShapeEdit, x: f32, y: f32, radius: f32) -> Option<BoxDrag> {
        let g = &e.g;
        let near = |(px, py): (f32, f32)| ((px - x).powi(2) + (py - y).powi(2)).sqrt() <= radius * 1.4;
        if e.ty == ShapeType::CustomPolygon {
            for (i, p) in e.custom.iter().enumerate() {
                if near(g.to_world(p.0 * g.w, p.1 * g.h)) {
                    return Some(BoxDrag::Vertex(i));
                }
            }
        }
        let top = g.to_world(0.0, -g.h / 2.0 - ROTATE_HANDLE_OFFSET * radius);
        if near(top) {
            return Some(BoxDrag::Rotate { g0: *g, a0: (y - g.cy).atan2(x - g.cx) });
        }
        for &(sx, sy) in &BOX_HANDLES {
            if near(g.to_world(sx as f32 * g.w / 2.0, sy as f32 * g.h / 2.0)) {
                let (pu0, pv0) = g.to_local(x, y);
                return Some(BoxDrag::Resize { g0: *g, sx, sy, pu0, pv0 });
            }
        }
        let (u, v) = g.to_local(x, y);
        (u.abs() <= g.w / 2.0 + radius && v.abs() <= g.h / 2.0 + radius).then_some(BoxDrag::Move { g0: *g, x0: x, y0: y })
    }

    /// Custom polygon: index of the edge near (x, y), to insert a corner into.
    fn edge_hit(&self, e: &ShapeEdit, x: f32, y: f32, radius: f32) -> Option<usize> {
        if e.ty != ShapeType::CustomPolygon || e.custom.len() < 3 || e.custom.len() >= 64 {
            return None;
        }
        let g = &e.g;
        let w: Vec<(f32, f32)> = e.custom.iter().map(|p| g.to_world(p.0 * g.w, p.1 * g.h)).collect();
        (0..w.len()).find(|&i| seg_distance((x, y), w[i], w[(i + 1) % w.len()]) <= radius.max(2.0))
    }

    /// Refits the box around a custom polygon's corners (after dragging one).
    fn refit(e: &mut ShapeEdit) {
        if e.custom.len() < 3 {
            return;
        }
        let (x0, x1) = e.custom.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
        let (y0, y1) = e.custom.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
        let (sw, sh) = ((x1 - x0).max(1e-3), (y1 - y0).max(1e-3));
        let g = e.g;
        let (cx, cy) = g.to_world((x0 + x1) / 2.0 * g.w, (y0 + y1) / 2.0 * g.h);
        let (w, h) = ((g.w * sw).max(1.0), (g.h * sh).max(1.0));
        for p in &mut e.custom {
            p.0 = (p.0 - (x0 + x1) / 2.0) / sw;
            p.1 = (p.1 - (y0 + y1) / 2.0) / sh;
        }
        e.g = BoxGeom { cx, cy, w, h, angle: g.angle };
    }

    fn drag_to(&mut self, x: f32, y: f32, mods: Modifiers) {
        let (Some(d), Some(e)) = (self.drag, self.edit.as_mut()) else { return };
        let g = &mut e.g;
        match d {
            BoxDrag::Create => {}
            BoxDrag::Vertex(i) => {
                let (u, v) = g.to_local(x, y);
                if let Some(p) = e.custom.get_mut(i) {
                    *p = (u / g.w.max(1e-3), v / g.h.max(1e-3));
                }
            }
            BoxDrag::Move { g0, x0, y0 } => {
                g.cx = g0.cx + (x - x0).round();
                g.cy = g0.cy + (y - y0).round();
            }
            BoxDrag::Rotate { g0, a0 } => {
                let mut a = g0.angle + (y - g0.cy).atan2(x - g0.cx) - a0;
                if mods.shift() {
                    let step = std::f32::consts::PI / 12.0;
                    a = round_even_f(a / step) * step;
                }
                let tau = std::f32::consts::TAU;
                a = (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI;
                g.angle = if a.abs() < 1e-5 { 0.0 } else { a };
            }
            BoxDrag::Resize { g0, sx, sy, pu0, pv0 } => {
                // Work in the shape's frame at drag start: the dragged edge moves
                // with the pointer, the opposite side stays put.
                let (pu, pv) = g0.to_local(x, y);
                let (pu, pv) = (sx as f32 * g0.w / 2.0 + pu - pu0, sy as f32 * g0.h / 2.0 + pv - pv0);
                let range = |s: i8, p: f32, size: f32| {
                    if s == 0 {
                        (-size / 2.0, size / 2.0)
                    } else {
                        let anchor = -(s as f32) * size / 2.0;
                        (anchor.min(p), anchor.max(p))
                    }
                };
                let (mut u0, mut u1) = range(sx, pu, g0.w);
                let (mut v0, mut v1) = range(sy, pv, g0.h);
                if mods.shift() && sx != 0 && sy != 0 && g0.w > 0.0 && g0.h > 0.0 {
                    // Keep the proportions: grow both sides by the larger factor.
                    let k = ((u1 - u0) / g0.w).max((v1 - v0) / g0.h);
                    let (nw, nh) = (g0.w * k, g0.h * k);
                    let au = -(sx as f32) * g0.w / 2.0;
                    let av = -(sy as f32) * g0.h / 2.0;
                    (u0, u1) = if pu >= au { (au, au + nw) } else { (au - nw, au) };
                    (v0, v1) = if pv >= av { (av, av + nh) } else { (av - nh, av) };
                }
                let (w, h) = ((u1 - u0).max(1.0), (v1 - v0).max(1.0));
                let (cx, cy) = g0.to_world((u0 + u1) / 2.0, (v0 + v1) / 2.0);
                // Mirror custom corners when dragged past the opposite side.
                if sx != 0 && (pu - -(sx as f32) * g0.w / 2.0).signum() != sx as f32 {
                    e.custom.iter_mut().for_each(|p| p.0 = -p.0);
                }
                *g = BoxGeom { cx, cy, w, h, angle: g0.angle };
            }
        }
    }
}

impl Tool for ShapeTool {
    fn name(&self) -> &'static str {
        match self.kind {
            ShapeKind::Rectangle => "Rectangle",
            ShapeKind::RoundedRectangle => "Rounded Rect",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Any => "Shape",
        }
    }
    fn kind(&self) -> ToolKind {
        match self.kind {
            ShapeKind::Rectangle => ToolKind::Rectangle,
            ShapeKind::RoundedRectangle => ToolKind::RoundedRectangle,
            ShapeKind::Ellipse => ToolKind::Ellipse,
            ShapeKind::Any => ToolKind::Shape,
        }
    }
    fn is_busy(&self) -> bool {
        self.dragging || self.drag.is_some()
    }
    fn has_pending(&self) -> bool {
        self.edit.is_some()
    }
    fn edit_overlay(&self) -> Option<EditOverlay> {
        self.edit.as_ref().map(|e| {
            let g = e.g;
            let vertices = if e.ty == ShapeType::CustomPolygon { e.custom.iter().map(|p| g.to_world(p.0 * g.w, p.1 * g.h)).collect() } else { Vec::new() };
            EditOverlay::Box { geom: g, shape: e.ty, vertices }
        })
    }
    fn pending(&mut self, ctx: &mut ToolContext, action: PendingAction) {
        match action {
            PendingAction::Commit => self.commit_edit(ctx),
            PendingAction::Cancel => {
                if self.edit.take().is_some() {
                    self.drag = None;
                    self.undo.clear();
                    self.redo.clear();
                    self.r.cancel(ctx);
                }
            }
            PendingAction::Refresh => self.render_edit(ctx),
            PendingAction::Undo => match self.undo.pop() {
                Some(prev) => {
                    if let Some(cur) = self.edit.replace(prev) {
                        self.redo.push(cur);
                    }
                    self.render_edit(ctx);
                }
                // Nothing left to step back through: remove the shape.
                None => self.pending(ctx, PendingAction::Cancel),
            },
            PendingAction::Redo => {
                if let Some(next) = self.redo.pop() {
                    if let Some(cur) = self.edit.replace(next) {
                        self.undo.push(cur);
                    }
                    self.render_edit(ctx);
                }
            }
            PendingAction::Reverse | PendingAction::DeleteSelected => {}
            PendingAction::Translate { dx, dy, start } => {
                if start || self.base.is_none() {
                    self.snapshot();
                    self.base = self.edit.as_ref().map(|e| e.g);
                }
                let (Some(e), Some(b)) = (self.edit.as_mut(), self.base) else { return };
                (e.g.cx, e.g.cy) = (b.cx + dx.round(), b.cy + dy.round());
                self.render_edit(ctx);
            }
            PendingAction::Nudge { dx, dy } => {
                self.snapshot();
                if let Some(e) = self.edit.as_mut() {
                    e.g.cx += dx;
                    e.g.cy += dy;
                    self.render_edit(ctx);
                }
            }
            PendingAction::Rotate { angle, start } => {
                if start || self.base.is_none() {
                    self.snapshot();
                    self.base = self.edit.as_ref().map(|e| e.g);
                }
                let (Some(e), Some(b)) = (self.edit.as_mut(), self.base) else { return };
                let a = b.angle + angle;
                e.g.angle = if a.abs() < 1e-5 { 0.0 } else { a };
                self.render_edit(ctx);
            }
        }
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        if ctx.settings.editable_shapes {
            if let Some(e) = self.edit.clone() {
                let (fx, fy, radius) = (x as f32 + 0.5, y as f32 + 0.5, (ctx.handle_radius as f32).max(1.0));
                let hit = self.hit(&e, fx, fy, radius);
                if button == PointerButton::Right {
                    // Right-click a custom polygon's corner to remove it.
                    if let Some(BoxDrag::Vertex(i)) = hit {
                        if e.custom.len() > 3 {
                            self.snapshot();
                            let ed = self.edit.as_mut().unwrap();
                            ed.custom.remove(i);
                            Self::refit(ed);
                            self.render_edit(ctx);
                        }
                        return;
                    }
                } else if let Some(d) = hit.filter(|d| !matches!(d, BoxDrag::Move { .. })).or_else(|| {
                    // Dragging an edge of a custom polygon adds a corner there.
                    self.edge_hit(&e, fx, fy, radius).map(|i| BoxDrag::Vertex(i + 1))
                }).or(hit) {
                    self.snapshot();
                    if let (BoxDrag::Vertex(i), Some(ed)) = (d, self.edit.as_mut()) {
                        if !matches!(hit, Some(BoxDrag::Vertex(_))) {
                            let (u, v) = ed.g.to_local(fx, fy);
                            ed.custom.insert(i, (u / ed.g.w.max(1e-3), v / ed.g.h.max(1e-3)));
                        }
                    }
                    self.drag = Some(d);
                    return;
                }
                self.commit_edit(ctx);
            }
            self.anchor = (x, y);
            self.button = button;
            self.r.begin(ctx);
            let ty = self.shape_type(ctx);
            let custom = if ty == ShapeType::CustomPolygon { shape_geom::regular(ctx.settings.shape_sides as usize, None) } else { Vec::new() };
            self.edit = Some(ShapeEdit { g: BoxGeom::from_rect(x, y, 1, 1), ty, custom });
            self.drag = Some(BoxDrag::Create);
            self.render_edit(ctx);
            return;
        }
        self.dragging = true;
        self.anchor = (x, y);
        self.button = button;
        self.r.begin(ctx);
        self.redraw(ctx, x, y, mods);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        if self.dragging {
            self.redraw(ctx, x, y, mods);
            return;
        }
        match self.drag {
            Some(BoxDrag::Create) => {
                let (rx, ry, rw, rh) = rect_from_drag(self.anchor.0, self.anchor.1, x, y, mods);
                if let Some(e) = self.edit.as_mut() {
                    e.g = BoxGeom::from_rect(rx, ry, rw, rh);
                }
                self.render_edit(ctx);
            }
            Some(_) => {
                self.drag_to(x as f32 + 0.5, y as f32 + 0.5, mods);
                self.render_edit(ctx);
            }
            None => {}
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, mods: Modifiers) {
        if self.dragging {
            self.redraw(ctx, x, y, mods);
            self.dragging = false;
            let name = self.name();
            self.r.commit(ctx, name);
            return;
        }
        if self.drag.is_none() {
            return;
        }
        self.pointer_move(ctx, x, y, mods);
        match self.drag.take() {
            Some(BoxDrag::Create) if (x, y) == self.anchor => {
                // A click without a drag draws nothing.
                self.edit = None;
                self.r.cancel(ctx);
            }
            Some(BoxDrag::Vertex(_)) => {
                if let Some(e) = self.edit.as_mut() {
                    Self::refit(e);
                }
                self.render_edit(ctx);
            }
            _ => {}
        }
    }
}

/// Line/Curve: how the control points are joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CurveType {
    /// Straight segments, bent at each point.
    #[default]
    Straight,
    /// A smooth curve through every point (Catmull-Rom spline).
    Spline,
    /// A cubic Bézier: two end points and two control points.
    Bezier,
}

/// A line being edited (with `ToolSettings::editable_shapes`).
#[derive(Clone, Debug)]
struct LineEdit {
    points: Vec<(f32, f32)>,
    button: PointerButton,
    curve: CurveType,
}

#[derive(Clone, Debug)]
enum LineDrag {
    Create,
    Point(usize),
    Body { start: Vec<(f32, f32)>, x0: i32, y0: i32 },
}

const MAX_POINTS: usize = 32;

#[derive(Default)]
pub struct LineTool {
    dragging: bool,
    anchor: (i32, i32),
    color: Bgra,
    r: Redraw,
    edit: Option<LineEdit>,
    drag: Option<LineDrag>,
    translate_base: Option<Vec<(f32, f32)>>,
    undo: Vec<LineEdit>,
    redo: Vec<LineEdit>,
}

fn snap15(ax: f32, ay: f32, x: f32, y: f32) -> (f32, f32) {
    if x == ax && y == ay {
        return (x, y);
    }
    let step = std::f32::consts::PI / 12.0;
    let angle = (y - ay).atan2(x - ax);
    let snapped = round_even_f(angle / step) * step;
    let len = ((x - ax) * (x - ax) + (y - ay) * (y - ay)).sqrt();
    (ax + snapped.cos() * len, ay + snapped.sin() * len)
}

/// The polyline a curve is drawn as.
pub fn curve_path(curve: CurveType, p: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let dist = |a: (f32, f32), b: (f32, f32)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    match curve {
        _ if p.len() < 2 => p.to_vec(),
        CurveType::Straight => p.to_vec(),
        CurveType::Spline => {
            let mut out = vec![p[0]];
            for i in 0..p.len() - 1 {
                let p0 = p[i.saturating_sub(1)];
                let (p1, p2) = (p[i], p[i + 1]);
                let p3 = p[(i + 2).min(p.len() - 1)];
                let n = (dist(p1, p2) / 2.0).clamp(4.0, 256.0) as usize;
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let (t2, t3) = (t * t, t * t * t);
                    let f = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (-a + c) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (-a + 3.0 * b - 3.0 * c + d) * t3);
                    out.push((f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1)));
                }
            }
            out
        }
        CurveType::Bezier => {
            let q = bezier_points(p);
            let len = dist(q[0], q[1]) + dist(q[1], q[2]) + dist(q[2], q[3]);
            let n = (len / 2.0).clamp(8.0, 512.0) as usize;
            (0..=n)
                .map(|k| {
                    let t = k as f32 / n as f32;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    (a * q[0].0 + b * q[1].0 + c * q[2].0 + d * q[3].0, a * q[0].1 + b * q[1].1 + c * q[2].1 + d * q[3].1)
                })
                .collect()
        }
    }
}

/// The four Bézier points for a point list (missing controls on the chord).
fn bezier_points(p: &[(f32, f32)]) -> [(f32, f32); 4] {
    let lerp = |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    match p.len() {
        0 => [(0.0, 0.0); 4],
        1 => [p[0]; 4],
        2 => [p[0], lerp(p[0], p[1], 1.0 / 3.0), lerp(p[0], p[1], 2.0 / 3.0), p[1]],
        3 => [p[0], p[1], p[1], p[2]],
        n => [p[0], p[1], p[n - 2], p[n - 1]],
    }
}

fn seg_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 1e-6 { 0.0 } else { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

impl LineTool {
    fn snapshot(&mut self) {
        if let Some(e) = &self.edit {
            self.undo.push(e.clone());
            self.redo.clear();
        }
    }

    fn redraw(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        let (ax, ay) = self.anchor;
        let (ex, ey) = if mods.shift() { snap15(ax as f32, ay as f32, x as f32, y as f32) } else { (x as f32, y as f32) };
        let width = ctx.settings.brush_size() as f32;
        let (aa, ab, color) = (ctx.settings.antialiasing, ctx.settings.alpha_blending, self.color);
        let caps = (ctx.settings.line_cap_start, ctx.settings.line_cap_end);
        let dash = ctx.settings.line_dash;
        self.r.redraw(ctx, |s, sel| {
            if caps == (draw::LineCap::Round, draw::LineCap::Round) && dash == draw::LineDash::Solid {
                draw::line(s, ax as f32, ay as f32, ex, ey, width, color, sel, aa, ab)
            } else {
                draw::stroke_styled(s, &[(ax as f32, ay as f32), (ex, ey)], width, color, caps.0, caps.1, dash, sel, aa, ab)
            }
        });
    }

    /// Re-renders the line being edited from the stroke-start pixels.
    fn render_edit(&mut self, ctx: &mut ToolContext) {
        let Some(e) = self.edit.as_mut() else { return };
        // Follow the toolbar's curve type, converting the points as needed.
        let curve = ctx.settings.curve_type;
        if curve != e.curve {
            if curve == CurveType::Bezier {
                e.points = bezier_points(&e.points).to_vec();
            }
            e.curve = curve;
        }
        let color = ctx.settings.color_for(e.button);
        let width = ctx.settings.brush_size() as f32;
        let (aa, ab) = (ctx.settings.antialiasing, ctx.settings.alpha_blending);
        let (curve, pts) = (e.curve, e.points.clone());
        let caps = (ctx.settings.line_cap_start, ctx.settings.line_cap_end);
        let dash = ctx.settings.line_dash;
        self.r.redraw(ctx, move |s, sel| {
            if curve != CurveType::Bezier && pts.len() == 2 && caps == (draw::LineCap::Round, draw::LineCap::Round) && dash == draw::LineDash::Solid {
                draw::line(s, pts[0].0, pts[0].1, pts[1].0, pts[1].1, width, color, sel, aa, ab)
            } else {
                draw::stroke_styled(s, &curve_path(curve, &pts), width, color, caps.0, caps.1, dash, sel, aa, ab)
            }
        });
    }

    fn commit_edit(&mut self, ctx: &mut ToolContext) {
        if self.edit.take().is_some() {
            self.drag = None;
            self.translate_base = None;
            self.undo.clear();
            self.redo.clear();
            self.r.commit(ctx, "Line/Curve");
        }
    }
}

impl Tool for LineTool {
    fn name(&self) -> &'static str {
        "Line"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Line
    }
    fn is_busy(&self) -> bool {
        self.dragging || self.drag.is_some()
    }
    fn has_pending(&self) -> bool {
        self.edit.is_some()
    }
    fn edit_overlay(&self) -> Option<EditOverlay> {
        self.edit.as_ref().map(|e| EditOverlay::Line { curve: e.curve, points: e.points.clone() })
    }
    fn pending(&mut self, ctx: &mut ToolContext, action: PendingAction) {
        match action {
            PendingAction::Commit => self.commit_edit(ctx),
            PendingAction::Cancel => {
                if self.edit.take().is_some() {
                    self.drag = None;
                    self.undo.clear();
                    self.redo.clear();
                    self.r.cancel(ctx);
                }
            }
            PendingAction::Refresh => self.render_edit(ctx),
            PendingAction::Undo => match self.undo.pop() {
                Some(prev) => {
                    if let Some(cur) = self.edit.replace(prev) {
                        self.redo.push(cur);
                    }
                    self.render_edit(ctx);
                }
                // Back to before the line was drawn: remove it.
                None => self.pending(ctx, PendingAction::Cancel),
            },
            PendingAction::Redo => {
                if let Some(next) = self.redo.pop() {
                    if let Some(cur) = self.edit.replace(next) {
                        self.undo.push(cur);
                    }
                    self.render_edit(ctx);
                }
            }
            PendingAction::Reverse | PendingAction::DeleteSelected => {}
            PendingAction::Translate { dx, dy, start } => {
                if start || self.translate_base.is_none() {
                    self.snapshot();
                }
                let Some(e) = self.edit.as_mut() else { return };
                if start || self.translate_base.is_none() {
                    self.translate_base = Some(e.points.clone());
                }
                let base = self.translate_base.as_ref().unwrap();
                e.points = base.iter().map(|&(x, y)| (x + dx, y + dy)).collect();
                self.render_edit(ctx);
            }
            PendingAction::Nudge { dx, dy } => {
                self.snapshot();
                let Some(e) = self.edit.as_mut() else { return };
                e.points.iter_mut().for_each(|p| *p = (p.0 + dx, p.1 + dy));
                self.render_edit(ctx);
            }
            PendingAction::Rotate { angle, start } => {
                // Around the middle of the points' bounding box.
                if start || self.translate_base.is_none() {
                    self.snapshot();
                }
                let Some(e) = self.edit.as_mut() else { return };
                if start || self.translate_base.is_none() {
                    self.translate_base = Some(e.points.clone());
                }
                let base = self.translate_base.as_ref().unwrap();
                let fold = |f: fn(f32, f32) -> f32, k: fn(&(f32, f32)) -> f32, init| base.iter().map(k).fold(init, f);
                let cx = (fold(f32::min, |p| p.0, f32::MAX) + fold(f32::max, |p| p.0, f32::MIN)) / 2.0;
                let cy = (fold(f32::min, |p| p.1, f32::MAX) + fold(f32::max, |p| p.1, f32::MIN)) / 2.0;
                let (sin, cos) = angle.sin_cos();
                e.points = base.iter().map(|&(x, y)| (cx + (x - cx) * cos - (y - cy) * sin, cy + (x - cx) * sin + (y - cy) * cos)).collect();
                self.render_edit(ctx);
            }
        }
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        if !ctx.settings.editable_shapes {
            self.dragging = true;
            self.anchor = (x, y);
            self.color = ctx.settings.color_for(button);
            self.r.begin(ctx);
            self.redraw(ctx, x, y, mods);
            return;
        }
        let p = (x as f32, y as f32);
        let before = self.edit.clone();
        if let Some(e) = self.edit.as_mut() {
            let r = (ctx.handle_radius as f32).max(1.5);
            let half = (ctx.settings.brush_size() as f32 / 2.0).max(r);
            // A control point?
            let hit = e.points.iter().enumerate().map(|(i, &q)| (i, seg_distance(p, q, q))).filter(|&(_, d)| d <= r).min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, _)) = hit {
                if button == PointerButton::Right {
                    // Right-click removes a bend point.
                    if e.curve != CurveType::Bezier && i != 0 && i != e.points.len() - 1 {
                        e.points.remove(i);
                        self.undo.extend(before);
                        self.redo.clear();
                        self.render_edit(ctx);
                    }
                    return;
                }
                self.drag = Some(LineDrag::Point(i));
                self.undo.extend(before);
                self.redo.clear();
                return;
            }
            // The line itself: bend it (new point) or, for Bézier curves, move it.
            let path = curve_path(e.curve, &e.points);
            let on_line = path.windows(2).any(|w| seg_distance(p, w[0], w[1]) <= half) || path.len() == 1;
            if on_line && button == PointerButton::Left {
                self.undo.extend(before);
                self.redo.clear();
                if e.curve == CurveType::Bezier || e.points.len() >= MAX_POINTS {
                    self.drag = Some(LineDrag::Body { start: e.points.clone(), x0: x, y0: y });
                } else {
                    let seg = (0..e.points.len() - 1)
                        .min_by(|&a, &b| seg_distance(p, e.points[a], e.points[a + 1]).total_cmp(&seg_distance(p, e.points[b], e.points[b + 1])))
                        .unwrap_or(0);
                    e.points.insert(seg + 1, p);
                    self.drag = Some(LineDrag::Point(seg + 1));
                    self.render_edit(ctx);
                }
                return;
            }
            // Elsewhere: finish this line and start a new one.
            self.commit_edit(ctx);
        }
        self.r.begin(ctx);
        self.edit = Some(LineEdit { points: vec![p, p], button, curve: ctx.settings.curve_type });
        if ctx.settings.curve_type == CurveType::Bezier {
            self.edit.as_mut().unwrap().curve = CurveType::Straight; // converted on render
        }
        self.drag = Some(LineDrag::Create);
        self.render_edit(ctx);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        if self.dragging {
            self.redraw(ctx, x, y, mods);
            return;
        }
        let (Some(e), Some(d)) = (self.edit.as_mut(), self.drag.as_ref()) else { return };
        let p = (x as f32, y as f32);
        match d {
            LineDrag::Create => {
                let a = e.points[0];
                let q = if mods.shift() { snap15(a.0, a.1, p.0, p.1) } else { p };
                *e.points.last_mut().unwrap() = q;
                if e.curve == CurveType::Bezier {
                    e.points = bezier_points(&[a, q]).to_vec();
                }
            }
            &LineDrag::Point(i) => {
                let q = if mods.shift() {
                    let n = if i == 0 { e.points[1] } else { e.points[i - 1] };
                    snap15(n.0, n.1, p.0, p.1)
                } else {
                    p
                };
                e.points[i] = q;
            }
            LineDrag::Body { start, x0, y0 } => {
                let (dx, dy) = ((x - x0) as f32, (y - y0) as f32);
                e.points = start.iter().map(|&(a, b)| (a + dx, b + dy)).collect();
            }
        }
        self.render_edit(ctx);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, mods: Modifiers) {
        if self.dragging {
            self.redraw(ctx, x, y, mods);
            self.dragging = false;
            self.r.commit(ctx, "Line");
            return;
        }
        if self.drag.is_none() {
            return;
        }
        self.pointer_move(ctx, x, y, mods);
        if matches!(self.drag.take(), Some(LineDrag::Create)) {
            // A click without a drag draws nothing.
            if let Some(e) = &self.edit {
                if e.points.first() == e.points.last() {
                    self.edit = None;
                    self.r.cancel(ctx);
                }
            }
        }
    }
}

#[derive(Default)]
pub struct FreeformShapeTool {
    dragging: bool,
    points: Vec<(i32, i32)>,
    button: Option<PointerButton>,
    r: Redraw,
}

impl FreeformShapeTool {
    fn redraw(&mut self, ctx: &mut ToolContext) {
        let button = self.button.unwrap_or(PointerButton::Left);
        let outline = ctx.settings.color_for(button);
        let fill = ctx.settings.other_color_for(button);
        let mode = ctx.settings.shape_mode;
        let width = ctx.settings.brush_size() as f32;
        let (aa, ab) = (ctx.settings.antialiasing, ctx.settings.alpha_blending);
        let style = ctx.settings.fill_style;
        let (doc_w, doc_h) = (ctx.doc.width(), ctx.doc.height());
        let pts = &self.points;
        self.r.redraw(ctx, |s, sel| {
            let mut rect = PixelRect::EMPTY;
            if mode.fills() {
                // Patterned fills clip the fill to the pattern, as every fill does.
                let clip = (style != draw::FillStyle::Solid).then(|| {
                    let (x0, x1) = pts.iter().fold((i32::MAX, i32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
                    let (y0, y1) = pts.iter().fold((i32::MAX, i32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
                    crate::Selection::pattern_clip(doc_w, doc_h, PixelRect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1), style, sel)
                });
                rect = rect.union(&draw::fill_polygon(s, pts, fill, clip.as_ref().or(sel), ab));
            }
            if mode.outlines() {
                rect = rect.union(&draw::polyline(s, pts, width, outline, true, sel, aa, ab));
            }
            rect
        });
    }
}

impl Tool for FreeformShapeTool {
    fn name(&self) -> &'static str {
        "Freeform Shape"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::FreeformShape
    }
    fn is_busy(&self) -> bool {
        self.dragging
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        self.dragging = true;
        self.button = Some(button);
        self.r.begin(ctx);
        self.points.clear();
        self.points.push((x, y));
        self.redraw(ctx);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _m: Modifiers) {
        if !self.dragging {
            return;
        }
        if self.points.last() != Some(&(x, y)) {
            self.points.push((x, y));
            self.redraw(ctx);
        }
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, _x: i32, _y: i32, _b: PointerButton, _m: Modifiers) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        self.redraw(ctx);
        self.r.commit(ctx, "Freeform Shape");
        self.points.clear();
    }
}
