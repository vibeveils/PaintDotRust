//! Gradient tool (C# `GradientTool`).

use super::shapes::Redraw;
use super::{EditOverlay, Modifiers, PendingAction, PointerButton, Tool, ToolContext, ToolKind};
use crate::blend::over_normal;
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::selection::Selection;
use crate::surface::Surface;
use std::f32::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GradientType {
    #[default]
    Linear,
    LinearReflected,
    Diamond,
    Radial,
    Conical,
}

/// What happens past the gradient's end points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GradientRepeat {
    /// The end colours carry on (the classic behaviour).
    #[default]
    None,
    /// Start again from the first colour (sawtooth).
    Repeat,
    /// Run back and forth (triangle wave).
    Reflect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GradientColorMode {
    #[default]
    Color,
    Transparency,
}

/// How a gradient is drawn, apart from its geometry and colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStyle {
    pub kind: GradientType,
    pub color_mode: GradientColorMode,
    pub repeat: GradientRepeat,
    /// Ordered (8×8 Bayer) dithering of the colour steps, to hide banding.
    pub dither: bool,
    pub alpha_blend: bool,
}

/// Threshold in (0, 1) from an 8×8 Bayer matrix.
#[inline]
fn bayer8(x: i32, y: i32) -> f32 {
    let (x, y) = (x as u32 & 7, y as u32 & 7);
    let xy = x ^ y;
    let v = ((xy & 1) << 5) | ((x & 1) << 4) | ((xy & 2) << 2) | ((x & 2) << 1) | ((xy & 4) >> 1) | ((x & 4) >> 2);
    (v as f32 + 0.5) / 64.0
}

/// Linear interpolation of one channel; `off` is 0.5 (round) or a dither threshold.
#[inline(always)]
fn lerp_ch(a: u8, b: u8, t: f32, off: f32) -> u8 {
    (a as f32 + (b as i32 - a as i32) as f32 * t + off) as u8
}

#[inline]
fn lerp_color(from: Bgra, to: Bgra, t: f32, off: f32) -> Bgra {
    Bgra::new(lerp_ch(from.b, to.b, t, off), lerp_ch(from.g, to.g, t, off), lerp_ch(from.r, to.r, t, off), lerp_ch(from.a, to.a, t, off))
}

/// Colour at `t` along sorted `stops` (position, colour).
#[inline]
fn stop_color(stops: &[(f32, Bgra)], t: f32, off: f32) -> Bgra {
    let first = stops[0];
    if stops.len() == 1 || t <= first.0 {
        return lerp_color(first.1, first.1, 0.0, off);
    }
    for w in stops.windows(2) {
        let (p0, c0) = w[0];
        let (p1, c1) = w[1];
        if t <= p1 {
            let span = p1 - p0;
            let u = if span <= 1e-6 { 1.0 } else { (t - p0) / span };
            return lerp_color(c0, c1, u, off);
        }
    }
    let last = stops[stops.len() - 1].1;
    lerp_color(last, last, 0.0, off)
}

#[inline]
fn wrap(t: f32, repeat: GradientRepeat) -> f32 {
    match repeat {
        GradientRepeat::None => t.clamp(0.0, 1.0),
        GradientRepeat::Repeat => t - t.floor(),
        GradientRepeat::Reflect => {
            let f = (t * 0.5) - (t * 0.5).floor(); // [0, 1) over two lengths
            1.0 - (f * 2.0 - 1.0).abs()
        }
    }
}

/// Renders a gradient from (ax, ay) to (bx, by) into the selection bounds.
#[allow(clippy::too_many_arguments)]
pub fn render(
    s: &mut Surface,
    kind: GradientType,
    ax: i32,
    ay: i32,
    bx: i32,
    by: i32,
    from: Bgra,
    to: Bgra,
    sel: Option<&Selection>,
    color_mode: GradientColorMode,
    alpha_blend: bool,
) -> PixelRect {
    let style = GradientStyle { kind, color_mode, repeat: GradientRepeat::None, dither: false, alpha_blend };
    render_stops(s, &style, (ax as f32, ay as f32), (bx as f32, by as f32), &[(0.0, from), (1.0, to)], sel)
}

/// Renders a gradient through colour `stops` (sorted positions in [0, 1])
/// from `a` to `b` into the selection bounds.
pub fn render_stops(s: &mut Surface, style: &GradientStyle, a: (f32, f32), b: (f32, f32), stops: &[(f32, Bgra)], sel: Option<&Selection>) -> PixelRect {
    let bounds = sel.map_or(s.bounds(), |m| m.bounds());
    let x1 = s.width().min(bounds.right());
    let y1 = s.height().min(bounds.bottom());
    if x1 <= bounds.x || y1 <= bounds.y || stops.is_empty() {
        return PixelRect::EMPTY;
    }
    let (ax, ay) = a;
    let dx = b.0 - ax;
    let dy = b.1 - ay;
    let len2 = dx * dx + dy * dy;
    let len = len2.sqrt();
    let base_angle = dy.atan2(dx);
    let w = s.width() as usize;
    let (bx0, by0) = (bounds.x, bounds.y);
    let style = *style;
    let rows = &mut s.pixels_mut()[by0 as usize * w..y1 as usize * w];
    crate::par::for_rows(rows, w, |i, row| {
        let y = by0 + i as i32;
        let py = y as f32 - ay;
        for x in bx0..x1 {
            if sel.is_some_and(|m| !m.contains(x, y)) {
                continue;
            }
            let px = x as f32 - ax;
            let t = if len2 < 0.0001 {
                0.0
            } else {
                let raw = match style.kind {
                    GradientType::Linear => (px * dx + py * dy) / len2,
                    GradientType::LinearReflected => ((px * dx + py * dy) / len2).abs(),
                    GradientType::Diamond => {
                        let u = (px * dx + py * dy) / len;
                        let v = (px * -dy + py * dx) / len;
                        (u.abs() + v.abs()) / len
                    }
                    GradientType::Radial => (px * px + py * py).sqrt() / len,
                    GradientType::Conical => {
                        let mut angle = py.atan2(px) - base_angle;
                        while angle > PI {
                            angle -= 2.0 * PI;
                        }
                        while angle < -PI {
                            angle += 2.0 * PI;
                        }
                        angle.abs() / PI
                    }
                };
                wrap(raw, style.repeat)
            };
            let off = if style.dither { bayer8(x, y) } else { 0.5 };
            let grad = match style.color_mode {
                GradientColorMode::Transparency => {
                    let from = stops[0].1;
                    from.with_alpha(((from.a as f32 * (1.0 - t) + off) as i32).clamp(0, 255) as u8)
                }
                GradientColorMode::Color => stop_color(stops, t, off),
            };
            let d = &mut row[x as usize];
            *d = if style.alpha_blend && grad.a < 255 { over_normal(grad, *d, 255) } else { grad };
        }
    });
    PixelRect::new(bounds.x, bounds.y, x1 - bounds.x, y1 - bounds.y)
}

/// A gradient placed but not committed: end points and colour stops.
#[derive(Clone, Debug, PartialEq)]
struct GradEdit {
    a: (f32, f32),
    b: (f32, f32),
    /// Sorted by position; the first is at 0 and the last at 1.
    stops: Vec<(f32, Bgra)>,
    /// The stop that picks up colour changes (none: the ends follow the
    /// primary and secondary colours).
    selected: Option<usize>,
    button: PointerButton,
    /// Primary/secondary colours last seen, to notice changes.
    seen: (Bgra, Bgra),
}

#[derive(Clone, Debug)]
enum GradDrag {
    Create,
    /// An end point (0 = start, 1 = end).
    End { which: usize, moved: bool, was_selected: bool },
    /// A middle stop, along the line.
    Stop { index: usize, moved: bool, was_selected: bool },
}

#[derive(Default)]
pub struct GradientTool {
    dragging: bool,
    anchor: (i32, i32),
    from: Bgra,
    to: Bgra,
    r: Redraw,
    edit: Option<GradEdit>,
    drag: Option<GradDrag>,
    translate_base: Option<((f32, f32), (f32, f32))>,
    undo: Vec<GradEdit>,
    redo: Vec<GradEdit>,
}

fn style_of(ctx: &ToolContext) -> GradientStyle {
    let st = &ctx.settings;
    GradientStyle { kind: st.gradient_type, color_mode: st.gradient_color_mode, repeat: st.gradient_repeat, dither: st.gradient_dither, alpha_blend: st.alpha_blending }
}

fn snap15(a: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    if p == a {
        return p;
    }
    let step = PI / 12.0;
    let angle = (p.1 - a.1).atan2(p.0 - a.0);
    let snapped = (angle / step).round() * step;
    let len = ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    (a.0 + snapped.cos() * len, a.1 + snapped.sin() * len)
}

impl GradEdit {
    fn stop_point(&self, i: usize) -> (f32, f32) {
        let t = self.stops[i].0;
        (self.a.0 + (self.b.0 - self.a.0) * t, self.a.1 + (self.b.1 - self.a.1) * t)
    }
    /// Position along the line of `p`'s projection (unclamped).
    fn project(&self, p: (f32, f32)) -> f32 {
        let (dx, dy) = (self.b.0 - self.a.0, self.b.1 - self.a.1);
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-6 { 0.0 } else { ((p.0 - self.a.0) * dx + (p.1 - self.a.1) * dy) / len2 }
    }
    fn distance_to_line(&self, p: (f32, f32)) -> f32 {
        let t = self.project(p).clamp(0.0, 1.0);
        let q = (self.a.0 + (self.b.0 - self.a.0) * t, self.a.1 + (self.b.1 - self.a.1) * t);
        ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
    }
}

impl GradientTool {
    fn redraw(&mut self, ctx: &mut ToolContext, x: i32, y: i32) {
        let (ax, ay) = self.anchor;
        let (from, to) = (self.from, self.to);
        let style = style_of(ctx);
        self.r.redraw(ctx, |s, sel| render_stops(s, &style, (ax as f32, ay as f32), (x as f32, y as f32), &[(0.0, from), (1.0, to)], sel));
    }

    fn snapshot(&mut self) {
        if let Some(e) = &self.edit {
            self.undo.push(e.clone());
            self.redo.clear();
        }
    }

    /// Picks up colour changes: they go to the selected stop, or else the
    /// first stop follows the primary colour and the last the secondary.
    fn follow_colors(&mut self, ctx: &ToolContext) {
        let Some(e) = &self.edit else { return };
        let now = (ctx.settings.color_for(e.button), ctx.settings.other_color_for(e.button));
        if now == e.seen {
            return;
        }
        let mut next = e.clone();
        let last = next.stops.len() - 1;
        match next.selected {
            Some(i) => next.stops[i].1 = if now.0 != e.seen.0 { now.0 } else { now.1 },
            None => {
                if now.0 != e.seen.0 {
                    next.stops[0].1 = now.0;
                }
                if now.1 != e.seen.1 {
                    next.stops[last].1 = now.1;
                }
            }
        }
        next.seen = now;
        if next.stops != e.stops {
            self.snapshot();
        }
        self.edit = Some(next);
    }

    fn render_edit(&mut self, ctx: &mut ToolContext) {
        let Some(e) = &self.edit else { return };
        let style = style_of(ctx);
        let (a, b, stops) = (e.a, e.b, e.stops.clone());
        self.r.redraw(ctx, move |s, sel| render_stops(s, &style, a, b, &stops, sel));
    }

    fn commit_edit(&mut self, ctx: &mut ToolContext) {
        if self.edit.take().is_some() {
            self.drag = None;
            self.translate_base = None;
            self.undo.clear();
            self.redo.clear();
            self.r.commit(ctx, "Gradient");
        }
    }

    fn transform(&mut self, ctx: &mut ToolContext, start: bool, f: impl Fn((f32, f32)) -> (f32, f32)) {
        if start || self.translate_base.is_none() {
            self.snapshot();
            let Some(e) = &self.edit else { return };
            self.translate_base = Some((e.a, e.b));
        }
        let Some(e) = self.edit.as_mut() else { return };
        let (a, b) = self.translate_base.unwrap();
        e.a = f(a);
        e.b = f(b);
        self.render_edit(ctx);
    }
}

impl Tool for GradientTool {
    fn name(&self) -> &'static str {
        "Gradient"
    }
    fn kind(&self) -> ToolKind {
        ToolKind::Gradient
    }
    fn is_busy(&self) -> bool {
        self.dragging || self.drag.is_some()
    }
    fn has_pending(&self) -> bool {
        self.edit.is_some()
    }
    fn edit_overlay(&self) -> Option<EditOverlay> {
        self.edit.as_ref().map(|e| EditOverlay::Gradient { a: e.a, b: e.b, stops: e.stops.clone(), selected: e.selected })
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
            PendingAction::Refresh => {
                self.follow_colors(ctx);
                self.render_edit(ctx);
            }
            PendingAction::Undo => match self.undo.pop() {
                Some(prev) => {
                    if let Some(cur) = self.edit.replace(prev) {
                        self.redo.push(cur);
                    }
                    self.render_edit(ctx);
                }
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
            PendingAction::Translate { dx, dy, start } => self.transform(ctx, start, |p| (p.0 + dx, p.1 + dy)),
            PendingAction::Nudge { dx, dy } => {
                self.translate_base = None;
                self.transform(ctx, true, |p| (p.0 + dx, p.1 + dy));
                self.translate_base = None;
            }
            PendingAction::Rotate { angle, start } => {
                if start || self.translate_base.is_none() {
                    self.snapshot();
                    let Some(e) = &self.edit else { return };
                    self.translate_base = Some((e.a, e.b));
                }
                let (a, b) = self.translate_base.unwrap();
                let c = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                let (sin, cos) = angle.sin_cos();
                let rot = |p: (f32, f32)| (c.0 + (p.0 - c.0) * cos - (p.1 - c.1) * sin, c.1 + (p.0 - c.0) * sin + (p.1 - c.1) * cos);
                self.transform(ctx, false, rot);
            }
            PendingAction::Reverse => {
                self.snapshot();
                let Some(e) = self.edit.as_mut() else { return };
                let n = e.stops.len();
                e.stops = e.stops.iter().rev().map(|&(p, c)| (1.0 - p, c)).collect();
                e.stops[0].0 = 0.0;
                e.stops[n - 1].0 = 1.0;
                e.selected = e.selected.map(|i| n - 1 - i);
                self.render_edit(ctx);
            }
            PendingAction::DeleteSelected => {
                let Some(e) = &self.edit else { return };
                let Some(i) = e.selected.filter(|&i| i > 0 && i + 1 < e.stops.len()) else { return };
                self.snapshot();
                let e = self.edit.as_mut().unwrap();
                e.stops.remove(i);
                e.selected = None;
                self.render_edit(ctx);
            }
        }
    }
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, _m: Modifiers) {
        if !matches!(button, PointerButton::Left | PointerButton::Right) {
            return;
        }
        if !ctx.settings.editable_shapes {
            self.dragging = true;
            self.anchor = (x, y);
            self.from = ctx.settings.color_for(button);
            self.to = ctx.settings.other_color_for(button);
            self.r.begin(ctx);
            self.redraw(ctx, x, y);
            return;
        }
        let p = (x as f32, y as f32);
        if let Some(e) = &self.edit {
            let r = (ctx.handle_radius as f32).max(1.5);
            let last = e.stops.len() - 1;
            let hit = (0..e.stops.len())
                .map(|i| {
                    let q = e.stop_point(i);
                    (i, ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt())
                })
                .filter(|&(_, d)| d <= r)
                // Prefer middle stops when they sit on an end point.
                .min_by(|a, b| (a.1 + if a.0 == 0 || a.0 == last { 0.01 } else { 0.0 }).total_cmp(&(b.1 + if b.0 == 0 || b.0 == last { 0.01 } else { 0.0 })));
            if let Some((i, _)) = hit {
                let was_selected = e.selected == Some(i);
                if button == PointerButton::Right {
                    if i != 0 && i != last {
                        self.snapshot();
                        let e = self.edit.as_mut().unwrap();
                        e.stops.remove(i);
                        e.selected = None;
                        self.render_edit(ctx);
                    }
                    return;
                }
                self.snapshot();
                let e = self.edit.as_mut().unwrap();
                e.selected = Some(i);
                self.drag = Some(if i == 0 || i == last {
                    GradDrag::End { which: (i != 0) as usize, moved: false, was_selected }
                } else {
                    GradDrag::Stop { index: i, moved: false, was_selected }
                });
                return;
            }
            // On the line: add a colour stop there (in the colour already
            // there, so nothing changes until a colour is picked).
            if button == PointerButton::Left && e.distance_to_line(p) <= r.max(3.0) {
                let t = e.project(p).clamp(0.0, 1.0);
                if t > 0.0 && t < 1.0 {
                    self.snapshot();
                    let e = self.edit.as_mut().unwrap();
                    let c = stop_color(&e.stops, t, 0.5);
                    let i = e.stops.iter().position(|s| s.0 > t).unwrap_or(e.stops.len() - 1);
                    e.stops.insert(i, (t, c));
                    e.selected = Some(i);
                    self.drag = Some(GradDrag::Stop { index: i, moved: true, was_selected: false });
                    self.render_edit(ctx);
                    return;
                }
            }
            // Elsewhere: finish this gradient and start a new one.
            self.commit_edit(ctx);
        }
        self.r.begin(ctx);
        let seen = (ctx.settings.color_for(button), ctx.settings.other_color_for(button));
        self.edit = Some(GradEdit { a: p, b: p, stops: vec![(0.0, seen.0), (1.0, seen.1)], selected: None, button, seen });
        self.drag = Some(GradDrag::Create);
        self.render_edit(ctx);
    }
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers) {
        if self.dragging {
            self.redraw(ctx, x, y);
            return;
        }
        let (Some(e), Some(d)) = (self.edit.as_mut(), self.drag.as_mut()) else { return };
        let p = (x as f32, y as f32);
        match d {
            GradDrag::Create => e.b = if mods.shift() { snap15(e.a, p) } else { p },
            GradDrag::End { which, moved, .. } => {
                let (end, other) = if *which == 0 { (&mut e.a, e.b) } else { (&mut e.b, e.a) };
                let q = if mods.shift() { snap15(other, p) } else { p };
                if q != *end {
                    // A click (no drag) only selects: the end stays where it is.
                    if !*moved && ((q.0 - end.0).abs() < 1.0 && (q.1 - end.1).abs() < 1.0) {
                        return;
                    }
                    *moved = true;
                    *end = q;
                }
            }
            GradDrag::Stop { index, moved, .. } => {
                let i = *index;
                let t = e.project(p);
                let lo = e.stops[i - 1].0 + 0.001;
                let hi = e.stops[i + 1].0 - 0.001;
                if lo < hi {
                    let t = t.clamp(lo, hi);
                    if (t - e.stops[i].0).abs() > 1e-5 {
                        *moved = true;
                    }
                    e.stops[i].0 = t;
                }
            }
        }
        self.render_edit(ctx);
    }
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, _b: PointerButton, mods: Modifiers) {
        if self.dragging {
            self.redraw(ctx, x, y);
            self.dragging = false;
            self.r.commit(ctx, "Gradient");
            return;
        }
        if self.drag.is_none() {
            return;
        }
        self.pointer_move(ctx, x, y, mods);
        match self.drag.take() {
            Some(GradDrag::Create) => {
                if let Some(e) = &self.edit {
                    if e.a == e.b {
                        // A click without a drag draws nothing.
                        self.edit = None;
                        self.r.cancel(ctx);
                    }
                }
            }
            // A selected stop clicked again: no longer selected.
            Some(GradDrag::Stop { moved: false, was_selected: true, .. }) | Some(GradDrag::End { moved: false, was_selected: true, .. }) => {
                if let Some(e) = self.edit.as_mut() {
                    e.selected = None;
                }
                // Selection changes alone are not worth an undo step.
                self.undo.pop();
            }
            Some(GradDrag::End { .. }) | Some(GradDrag::Stop { .. }) => {
                // Unchanged (only selected): drop the snapshot again.
                if let (Some(prev), Some(e)) = (self.undo.last(), &self.edit) {
                    if prev.a == e.a && prev.b == e.b && prev.stops == e.stops {
                        self.undo.pop();
                    }
                }
            }
            None => {}
        }
    }
}
