//! The OpenPDN application model behind a single message API.
//!
//! A host (the browser worker running the wasm build, or the native dev
//! server) sends JSON requests with an optional binary payload to
//! [`Editor::handle`] and gets a JSON reply plus binary payloads back:
//! command results, a fresh state snapshot when anything visible changed,
//! and the composited pixels of the region that needs repainting.

pub mod fx;
pub mod outline;
pub mod session;
pub use session::Session;

use openpdn_core::adjust::{apply_op, PixelOp};
use openpdn_core::history::{
    BitmapLayerEdit, CompoundEdit, DocumentSwapEdit, HistoryEntry, LayerAddEdit, LayerMoveEdit, LayerProperty, LayerPropertyEdit,
    LayerRemoveEdit, SelectionEdit, SelfInverseEdit,
};
use openpdn_core::tools::shapes::CurveType;
use openpdn_core::draw::{FillStyle, LineCap, LineDash};
use openpdn_core::tools::shape_geom::ShapeType;
use openpdn_core::tools::warp::WarpMode;
use openpdn_core::tools::{EditOverlay, PendingAction, FloodMode, GradientColorMode, GradientRepeat, GradientType, Modifiers, PointerButton, SelectSize, ShapeDrawMode, ToolKind, ToolManager};
use openpdn_core::transforms::{self, CanvasAnchor, ResampleMode};
use openpdn_core::selection::ToleranceAlpha;
use openpdn_core::{
    blend, brushes::FillPattern, resample::TransformResampling, selection_ops, Bgra, BlendMode, Document, HistoryStack, Layer, LayerId,
    PixelRect, Selection, SelectionCombineMode, Surface,
};
use openpdn_formats::json::Json;
use openpdn_formats::{openpdn, pdn, png};

/// Reply to one request.
#[derive(Default)]
pub struct Reply {
    pub json: Vec<(String, Json)>,
    /// Binary payloads, described in order by the `bins` array of the JSON.
    pub bins: Vec<(String, Vec<u8>)>,
}

impl Reply {
    fn set(&mut self, k: &str, v: Json) {
        self.json.push((k.to_owned(), v));
    }
    fn bin(&mut self, name: &str, data: Vec<u8>) {
        self.bins.push((name.to_owned(), data));
    }
    /// Serialised JSON (includes `bins: [{name, len}]`).
    pub fn to_json(&self) -> Json {
        let mut o = self.json.clone();
        o.push((
            "bins".into(),
            Json::Arr(
                self.bins
                    .iter()
                    .map(|(n, d)| Json::Obj(vec![("name".into(), Json::Str(n.clone())), ("len".into(), Json::Num(d.len() as f64))]))
                    .collect(),
            ),
        ));
        Json::Obj(o)
    }
    /// All binary payloads concatenated.
    pub fn binary(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.bins.iter().map(|b| b.1.len()).sum());
        for (_, d) in &self.bins {
            v.extend_from_slice(d);
        }
        v
    }
}

type R<T> = Result<T, String>;

/// A live preview (adjustment/effect dialog, rotate/zoom, feather, layer properties).
enum Preview {
    Fx { id: String, layer: LayerId, original: Vec<Bgra>, sel: Option<Selection> },
    Feather { original: Selection },
    LayerProps { layer: LayerId, before: (String, bool, u8, BlendMode) },
}

pub struct Editor {
    doc: Document,
    history: HistoryStack,
    tools: ToolManager,
    active: LayerId,
    name: String,
    dirty: bool,
    preview: Option<Preview>,
    sent_visual: u64,
    sent_selection: u64,
    sent_history: u64,
    sent_settings: u64,
    force_state: bool,
    force_full: bool,
    status: String,
    /// Text placed but not committed: the layer's pixels from before it.
    text: Option<TextSession>,
    /// What was last reported about pending edits (to resend state on change).
    sent_pending: Option<Json>,
}

/// Commands that leave a pending line/text editable; any other command
/// commits it first ("making another image action" confirms it).
const KEEPS_PENDING: &[&str] = &[
    "pointer", "setting", "colors", "swap_colors", "reset_colors", "remember_color", "render", "thumbs", "init",
    "shape_commit", "shape_cancel", "shape_translate", "shape_rotate", "shape_nudge", "undo", "redo", "pattern_preview", "text_preview", "text_commit", "text_cancel",
    "gradient_reverse", "pending_delete",
];

struct TextSession {
    layer: LayerId,
    before: Vec<Bgra>,
    drawn: PixelRect,
}

fn num(v: impl Into<f64>) -> Json {
    Json::Num(v.into())
}

fn s(v: &str) -> Json {
    Json::Str(v.to_owned())
}

fn hex(c: Bgra) -> Json {
    Json::Str(format!("#{}", c.to_hex()))
}

/// Blends straight-alpha `src` over `dst` at (x, y), clipped to the selection.
fn blend_rgba(dst: &mut Surface, src: &Surface, x: i32, y: i32, sel: Option<&Selection>) -> PixelRect {
    let (x0, y0) = (x.max(0), y.max(0));
    let x1 = dst.width().min(x + src.width());
    let y1 = dst.height().min(y + src.height());
    if x1 <= x0 || y1 <= y0 {
        return PixelRect::EMPTY;
    }
    for py in y0..y1 {
        let srow = src.row(py - y);
        let drow = dst.row_mut(py);
        for px in x0..x1 {
            let c = srow[(px - x) as usize];
            if c.a == 0 {
                continue;
            }
            let k = sel.map_or(255, |m| m.coverage_at(px, py));
            if k != 0 {
                drow[px as usize] = blend::over_normal(c, drow[px as usize], k);
            }
        }
    }
    PixelRect::new(x0, y0, x1 - x0, y1 - y0)
}

fn rgba_to_surface(w: i32, h: i32, rgba: &[u8]) -> R<Surface> {
    if w <= 0 || h <= 0 || rgba.len() != w as usize * h as usize * 4 {
        return Err("bad RGBA payload".into());
    }
    Ok(Surface::from_rgba8(w, h, rgba))
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new(800, 600)
    }
}

impl Editor {
    /// A new editor with a white `w`×`h` document.
    pub fn new(w: i32, h: i32) -> Editor {
        let doc = Self::blank(w, h, Bgra::WHITE);
        let active = doc.layers[0].id();
        // The core's defaults match the C# port (for the parity suite); the
        // app starts with Paint.NET's defaults instead.
        let mut tools = ToolManager::new();
        let st = &mut tools.settings;
        st.resampling = TransformResampling::Bicubic;
        st.gamma_correct = true;
        st.tolerance_alpha = ToleranceAlpha::Premultiplied;
        st.fill_tolerance = true;
        st.wand_sample_image = false;
        st.editable_shapes = true;
        st.gradient_dither = true;
        Editor {
            doc,
            history: HistoryStack::new(200),
            tools,
            active,
            name: "Untitled".into(),
            dirty: false,
            preview: None,
            sent_visual: u64::MAX,
            sent_selection: u64::MAX,
            sent_history: u64::MAX,
            sent_settings: u64::MAX,
            force_state: true,
            force_full: true,
            status: String::new(),
            text: None,
            sent_pending: None,
        }
    }

    fn blank(w: i32, h: i32, bg: Bgra) -> Document {
        let mut doc = Document::new(w, h);
        let mut l = Layer::new(w, h, "Background");
        l.surface.clear(bg);
        doc.layers.push(l);
        doc
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    fn set_document(&mut self, doc: Document, name: &str) {
        self.active = doc.layers.last().map(|l| l.id()).unwrap_or(LayerId(0));
        self.doc = doc;
        self.history.clear();
        self.name = name.to_owned();
        self.dirty = false;
        self.preview = None;
        self.force_state = true;
        self.force_full = true;
    }

    fn active_index(&self) -> usize {
        self.doc.layer_index(self.active).unwrap_or(self.doc.layers.len().saturating_sub(1))
    }

    fn fix_active(&mut self) {
        if self.doc.layer_index(self.active).is_none() {
            if let Some(l) = self.doc.layers.last() {
                self.active = l.id();
            }
        }
    }

    fn push(&mut self, e: impl HistoryEntry + 'static) {
        self.history.push(e);
    }

    fn push_all(&mut self, mut entries: Vec<Box<dyn HistoryEntry>>, name: &str) {
        match entries.len() {
            0 => {}
            1 => self.history.push_boxed(entries.pop().unwrap()),
            _ => self.history.push(CompoundEdit::new(entries, name)),
        }
    }

    /// Deselects as part of an operation, returning the history entry for it.
    fn deselect_entry(&mut self, name: &str) -> Option<Box<dyn HistoryEntry>> {
        let before = self.doc.active_selection().cloned()?;
        self.doc.set_selection(None);
        Some(Box::new(SelectionEdit::new(Some(before), None, name)))
    }

    /// Replaces the document as one undoable step (resize, crop, rotate…).
    fn swap_document(&mut self, new_doc: Document, name: &str, clear_selection: bool) {
        let mut entries: Vec<Box<dyn HistoryEntry>> = Vec::new();
        if clear_selection {
            entries.extend(self.deselect_entry(name));
        }
        let active_idx = self.active_index();
        entries.push(Box::new(DocumentSwapEdit::apply(&mut self.doc, new_doc, name)));
        self.active = self.doc.layers[active_idx.min(self.doc.layers.len() - 1)].id();
        self.push_all(entries, name);
        self.force_full = true;
    }

    // ------------------------------------------------------------------ dispatch

    /// Handles one request. Errors become `{ok:false, error}` replies.
    pub fn handle(&mut self, req: &Json, bin: &[u8]) -> Reply {
        let mut reply = Reply::default();
        let cmd = req.get("cmd").and_then(Json::as_str).unwrap_or("").to_owned();
        if let Some(id) = req.get("id") {
            reply.set("id", id.clone());
        }
        // Selection quality is a tool setting; the document applies it when clipping.
        self.doc.set_selection_pixelated(!self.tools.settings.selection_antialias);
        if !KEEPS_PENDING.contains(&cmd.as_str()) {
            self.commit_pending();
        }
        let result = self.dispatch(&cmd, req, bin, &mut reply);
        // Tools that show handles as soon as they are chosen (Perspective)
        // start again after their result is committed or anything else changes.
        if !self.has_pending() && self.tools.active_kind() == ToolKind::Perspective && self.doc.layers.len() > 0 {
            self.fix_active();
            let id = self.active;
            self.tools.activate(&mut self.doc, &mut self.history, id);
        }
        if matches!(cmd.as_str(), "setting" | "colors" | "swap_colors" | "reset_colors") && result.is_ok() {
            self.pending_action(PendingAction::Refresh);
        }
        self.doc.set_selection_pixelated(!self.tools.settings.selection_antialias);
        match result {
            Ok(()) => reply.set("ok", Json::Bool(true)),
            Err(e) => {
                reply.set("ok", Json::Bool(false));
                reply.set("error", s(&e));
            }
        }
        self.after(&mut reply);
        reply
    }

    pub fn tools_mut(&mut self) -> &mut ToolManager {
        &mut self.tools
    }

    /// Whether a line or text is placed but not yet committed.
    pub fn has_pending(&self) -> bool {
        self.tools.has_pending() || self.text.is_some()
    }

    /// Commits any pending line/text (one history step each).
    pub fn commit_pending(&mut self) {
        self.pending_action(PendingAction::Commit);
        if let Some(t) = self.text.take() {
            if let Some(e) = BitmapLayerEdit::capture(&self.doc, t.layer, &t.before, t.drawn, "Text") {
                self.push(e);
            }
        }
    }

    fn pending_action(&mut self, action: PendingAction) {
        if self.tools.has_pending() {
            self.fix_active();
            let id = self.active;
            self.tools.pending(&mut self.doc, &mut self.history, id, action);
        }
    }

    fn pending_json(&self) -> Json {
        match self.tools.edit_overlay() {
            Some(EditOverlay::Line { curve, points }) => {
                let pts = points.iter().map(|&(x, y)| Json::Arr(vec![num(x), num(y)])).collect();
                return Json::Obj(vec![("kind".into(), s("line")), ("curve".into(), s(&format!("{curve:?}"))), ("points".into(), Json::Arr(pts))]);
            }
            Some(EditOverlay::Box { geom: g, shape, vertices }) => {
                let verts = vertices.iter().map(|&(x, y)| Json::Arr(vec![num(x), num(y)])).collect();
                return Json::Obj(vec![
                    ("kind".into(), s("shape")),
                    ("type".into(), s(&shape.name())),
                    ("vertices".into(), Json::Arr(verts)),
                    ("cx".into(), num(g.cx)),
                    ("cy".into(), num(g.cy)),
                    ("w".into(), num(g.w)),
                    ("h".into(), num(g.h)),
                    ("angle".into(), num(g.angle)),
                ]);
            }
            Some(EditOverlay::Gradient { a, b, stops, selected }) => {
                let pt = |p: (f32, f32)| Json::Arr(vec![num(p.0), num(p.1)]);
                let stops = stops.iter().map(|&(t, c)| Json::Arr(vec![num(t), hex(c)])).collect();
                return Json::Obj(vec![
                    ("kind".into(), s("gradient")),
                    ("a".into(), pt(a)),
                    ("b".into(), pt(b)),
                    ("stops".into(), Json::Arr(stops)),
                    ("selected".into(), selected.map_or(Json::Null, |i| num(i as f64))),
                ]);
            }
            Some(EditOverlay::Quad { corners }) => {
                let pts = corners.iter().map(|&(x, y)| Json::Arr(vec![num(x), num(y)])).collect();
                return Json::Obj(vec![("kind".into(), s("quad")), ("corners".into(), Json::Arr(pts))]);
            }
            None => {}
        }
        if self.text.is_some() {
            return Json::Obj(vec![("kind".into(), s("text"))]);
        }
        Json::Null
    }

    /// Restores the pixels under the text preview.
    fn text_restore(&mut self) -> Option<TextSession> {
        let t = self.text.take()?;
        if let Some(l) = self.doc.layer_mut(t.layer) {
            l.surface.copy_rect_from(&t.before, t.drawn);
        }
        self.doc.invalidate(Some(t.drawn));
        Some(t)
    }

    fn after(&mut self, reply: &mut Reply) {
        let pending = self.pending_json();
        if self.sent_pending.as_ref() != Some(&pending) {
            self.force_state = true;
            self.sent_pending = Some(pending);
        }
        // State snapshot when anything the panels show changed.
        let changed = self.force_state
            || self.sent_visual != self.doc.visual_revision
            || self.sent_selection != self.doc.selection_revision
            || self.sent_history != self.history.revision
            || self.sent_settings != self.tools.settings.revision;
        if self.sent_history != self.history.revision && !self.force_state && self.sent_history != u64::MAX {
            self.dirty = true;
        }
        if changed {
            let sel_changed = self.force_state || self.sent_selection != self.doc.selection_revision;
            reply.set("state", self.state(sel_changed));
            self.sent_visual = self.doc.visual_revision;
            self.sent_selection = self.doc.selection_revision;
            self.sent_history = self.history.revision;
            self.sent_settings = self.tools.settings.revision;
            self.force_state = false;
        }
        // Pixels to repaint.
        let mut dirty = self.doc.take_dirty();
        if self.force_full {
            dirty = self.doc.bounds();
            self.force_full = false;
        }
        if !dirty.is_empty() {
            let mut px = vec![Bgra::TRANSPARENT; dirty.width as usize * dirty.height as usize];
            self.doc.composite_rect(dirty, &mut px);
            let mut rgba = vec![0u8; px.len() * 4];
            openpdn_core::surface::write_rgba8(&px, &mut rgba);
            reply.set(
                "tile",
                Json::Obj(vec![
                    ("x".into(), num(dirty.x)),
                    ("y".into(), num(dirty.y)),
                    ("w".into(), num(dirty.width)),
                    ("h".into(), num(dirty.height)),
                    ("docW".into(), num(self.doc.width())),
                    ("docH".into(), num(self.doc.height())),
                ]),
            );
            reply.bin("tile", rgba);
        }
        if !self.status.is_empty() {
            reply.set("status", s(&std::mem::take(&mut self.status)));
        }
    }

    fn state(&self, with_selection: bool) -> Json {
        let layers = self
            .doc
            .layers
            .iter()
            .map(|l| {
                Json::Obj(vec![
                    ("id".into(), num(l.id().0 as f64)),
                    ("name".into(), s(&l.name)),
                    ("visible".into(), Json::Bool(l.visible)),
                    ("opacity".into(), num(l.opacity)),
                    ("blend".into(), s(l.blend_mode.name())),
                ])
            })
            .collect();
        let st = &self.tools.settings;
        let mut o = vec![
            ("width".into(), num(self.doc.width())),
            ("height".into(), num(self.doc.height())),
            ("name".into(), s(&self.name)),
            ("dirty".into(), Json::Bool(self.dirty || self.has_pending())),
            ("pending".into(), self.pending_json()),
            ("layers".into(), Json::Arr(layers)),
            ("active".into(), num(self.active_index() as f64)),
            (
                "history".into(),
                Json::Obj(vec![
                    ("items".into(), Json::Arr(self.history.descriptions().map(s).collect())),
                    ("applied".into(), num(self.history.undo_depth() as f64)),
                ]),
            ),
            ("tool".into(), s(self.tools.active().name())),
            ("toolKind".into(), s(&format!("{:?}", self.tools.active_kind()))),
            ("busy".into(), Json::Bool(self.tools.active().is_busy())),
            (
                "settings".into(),
                Json::Obj(vec![
                    ("brush".into(), num(st.brush_size())),
                    ("tolerance".into(), num(st.wand_tolerance())),
                    ("aa".into(), Json::Bool(st.antialiasing)),
                    ("alphaBlend".into(), Json::Bool(st.alpha_blending)),
                    ("shape".into(), s(&format!("{:?}", st.shape_mode))),
                    ("gradient".into(), s(&format!("{:?}", st.gradient_type))),
                    ("gradientMode".into(), s(&format!("{:?}", st.gradient_color_mode))),
                    ("gradientRepeat".into(), s(&format!("{:?}", st.gradient_repeat))),
                    ("gradientDither".into(), Json::Bool(st.gradient_dither)),
                    ("warpMode".into(), s(st.warp_mode.name())),
                    ("warpStrength".into(), num(st.warp_strength)),
                    ("sampleAll".into(), Json::Bool(st.sample_all_layers)),
                    ("eraserPattern".into(), s(&format!("{:?}", st.eraser_pattern))),
                    ("resampling".into(), s(&format!("{:?}", st.resampling))),
                    ("pressure".into(), Json::Bool(st.pressure_sensitivity)),
                    ("combine".into(), s(&format!("{:?}", st.selection_combine_mode))),
                    ("flood".into(), s(&format!("{:?}", st.flood_mode))),
                    ("gamma".into(), Json::Bool(st.gamma_correct)),
                    ("curve".into(), s(&format!("{:?}", st.curve_type))),
                    ("capStart".into(), s(&format!("{:?}", st.line_cap_start))),
                    ("fillStyle".into(), s(&st.fill_style.name())),
                    ("dash".into(), s(&format!("{:?}", st.line_dash))),
                    ("shapeType".into(), s(&st.shape_type.name())),
                    ("shapeSides".into(), num(st.shape_sides)),
                    ("starInner".into(), num(st.star_inner)),
                    ("shapeRadius".into(), num(st.shape_radius)),
                    ("capEnd".into(), s(&format!("{:?}", st.line_cap_end))),
                    ("toleranceAlpha".into(), s(&format!("{:?}", st.tolerance_alpha))),
                    ("wandSampleImage".into(), Json::Bool(st.wand_sample_image)),
                    ("selectionAA".into(), Json::Bool(st.selection_antialias)),
                    (
                        "selectSize".into(),
                        match st.select_size {
                            SelectSize::Any => Json::Obj(vec![("mode".into(), s("Any"))]),
                            SelectSize::FixedRatio(w, h) => Json::Obj(vec![("mode".into(), s("FixedRatio")), ("w".into(), num(w)), ("h".into(), num(h))]),
                            SelectSize::FixedSize(w, h) => Json::Obj(vec![("mode".into(), s("FixedSize")), ("w".into(), num(w)), ("h".into(), num(h))]),
                        },
                    ),
                    ("fg".into(), hex(st.foreground)),
                    ("bg".into(), hex(st.background)),
                    ("recent".into(), Json::Arr(st.recent_colors().iter().map(|&c| hex(c)).collect())),
                ]),
            ),
            ("previewing".into(), Json::Bool(self.preview.is_some())),
        ];
        if let Some(b) = self.tools.handle_bounds(&self.doc) {
            o.push(("handles".into(), Json::Arr(vec![num(b.x), num(b.y), num(b.w), num(b.h)])));
        }
        if with_selection {
            let sel = match self.doc.active_selection() {
                Some(sel) => {
                    let b = sel.bounds();
                    Json::Obj(vec![
                        ("bounds".into(), Json::Arr(vec![num(b.x), num(b.y), num(b.width), num(b.height)])),
                        ("polygon".into(), Json::Bool(sel.outline().is_some())),
                        (
                            "outline".into(),
                            Json::Arr(outline::outline(sel).into_iter().map(|p| Json::Arr(p.into_iter().map(num).collect())).collect()),
                        ),
                    ])
                }
                None => Json::Null,
            };
            o.push(("selection".into(), sel));
        }
        Json::Obj(o)
    }

    fn arg<'a>(req: &'a Json, k: &str) -> R<&'a Json> {
        req.get(k).ok_or_else(|| format!("missing argument '{k}'"))
    }
    fn int(req: &Json, k: &str) -> R<i32> {
        Self::arg(req, k)?.as_f64().map(|v| v as i32).ok_or_else(|| format!("'{k}' must be a number"))
    }
    fn str<'a>(req: &'a Json, k: &str) -> R<&'a str> {
        Self::arg(req, k)?.as_str().ok_or_else(|| format!("'{k}' must be a string"))
    }
    fn bool(req: &Json, k: &str) -> bool {
        req.get(k).and_then(Json::as_bool).unwrap_or(false)
    }
    fn color(req: &Json, k: &str) -> R<Bgra> {
        Bgra::parse_hex(Self::str(req, k)?).ok_or_else(|| format!("'{k}' is not a colour"))
    }

    fn dispatch(&mut self, cmd: &str, req: &Json, bin: &[u8], out: &mut Reply) -> R<()> {
        if self.preview.is_some() && !matches!(cmd, "fx" | "feather" | "layer_props" | "init" | "render" | "thumbs" | "export_rgba") {
            // Any other command ends a dangling preview by cancelling it.
            self.cancel_preview();
        }
        match cmd {
            "init" => {
                self.force_state = true;
                self.force_full = true;
                out.set("fx", fx::schema());
                out.set("tools", Json::Arr(self.tools.tools().map(|t| Json::Arr(vec![s(t.name()), s(&format!("{:?}", t.kind()))])).collect()));
                out.set("blendModes", Json::Arr(BlendMode::ALL.iter().map(|m| s(m.name())).collect()));
            }
            "render" => self.force_full = true,
            "new" => {
                let (w, h) = (Self::int(req, "w")?, Self::int(req, "h")?);
                openpdn_formats::check_size(w as i64, h as i64).map_err(|e| e.0)?;
                let bg = match req.get("bg").and_then(Json::as_str) {
                    Some("transparent") => Bgra::TRANSPARENT,
                    Some("secondary") => self.tools.settings.background,
                    Some("primary") => self.tools.settings.foreground,
                    _ => Bgra::WHITE,
                };
                self.set_document(Self::blank(w, h, bg), "Untitled");
                self.status = format!("New {w} × {h} image");
            }
            "open" => {
                let name = Self::str(req, "name").unwrap_or("Untitled").to_owned();
                let doc = match openpdn_formats::sniff(bin) {
                    openpdn_formats::Sniffed::Png => {
                        let d = png::decode(bin).map_err(|e| e.0)?;
                        let mut doc = Document::from_single_layer(d.surface, "Background");
                        doc.metadata.text_chunks = d.text;
                        doc
                    }
                    openpdn_formats::Sniffed::OpenPdn => openpdn::read(bin).map_err(|e| e.0)?,
                    openpdn_formats::Sniffed::PdnLegacy => pdn::read(bin).map_err(|e| e.0)?,
                    openpdn_formats::Sniffed::Other => return Err("unsupported".into()),
                };
                self.set_document(doc, &name);
            }
            "open_rgba" => {
                let surface = rgba_to_surface(Self::int(req, "w")?, Self::int(req, "h")?, bin)?;
                let name = Self::str(req, "name").unwrap_or("Untitled").to_owned();
                self.set_document(Document::from_single_layer(surface, "Background"), &name);
            }
            "save" => {
                let bytes = match Self::str(req, "format")? {
                    "openpdn" => openpdn::write(&self.doc).map_err(|e| e.0)?,
                    "png" => png::encode(&self.doc.composite(), &self.doc.metadata.text_chunks, png::Compression::Fast),
                    f => return Err(format!("unknown format {f}")),
                };
                if Self::bool(req, "markSaved") {
                    self.dirty = false;
                    self.force_state = true;
                    if let Some(n) = req.get("name").and_then(Json::as_str) {
                        self.name = n.to_owned();
                    }
                }
                out.bin("file", bytes);
            }
            "mark_saved" => {
                self.dirty = false;
                self.force_state = true;
                if let Some(n) = req.get("name").and_then(Json::as_str) {
                    self.name = n.to_owned();
                }
            }
            "export_rgba" => {
                let c = self.doc.composite();
                out.set("w", num(c.width()));
                out.set("h", num(c.height()));
                out.bin("rgba", c.to_rgba8());
            }
            "thumbs" => {
                let size = Self::int(req, "size").unwrap_or(40).clamp(8, 256);
                for l in &self.doc.layers {
                    out.bin("thumb", thumbnail(&l.surface, size));
                }
                out.set("thumbSize", num(size));
            }
            // -------------------------------------------------------------- tools
            "pointer" => self.pointer(req)?,
            "tool" => {
                let name = Self::str(req, "name")?;
                if !self.tools.set_active_by_name(name) {
                    return Err(format!("unknown tool {name}"));
                }
                self.force_state = true;
            }
            "setting" => self.setting(Self::str(req, "key")?, Self::arg(req, "value")?)?,
            "colors" => {
                let st = &mut self.tools.settings;
                if req.get("fg").is_some() {
                    st.foreground = Self::color(req, "fg")?;
                }
                if req.get("bg").is_some() {
                    st.background = Self::color(req, "bg")?;
                }
                st.revision += 1;
            }
            "remember_color" => {
                let c = Self::color(req, "color")?;
                self.tools.settings.remember_color(c);
            }
            "swap_colors" => self.tools.settings.swap_colors(),
            "reset_colors" => self.tools.settings.reset_colors(),
            "text" => self.stamp_text(req, bin)?,
            // Live, movable text: each preview replaces the last one.
            "text_preview" => {
                let src = rgba_to_surface(Self::int(req, "w")?, Self::int(req, "h")?, bin)?;
                let (x, y) = (Self::int(req, "x")?, Self::int(req, "y")?);
                let session = match self.text_restore() {
                    Some(t) => t,
                    None => {
                        self.fix_active();
                        let layer = self.active;
                        let before = self.doc.layer(layer).ok_or("no layer")?.surface.pixels().to_vec();
                        TextSession { layer, before, drawn: PixelRect::EMPTY }
                    }
                };
                let sel = self.doc.active_selection().cloned();
                let layer = self.doc.layer_mut(session.layer).ok_or("no layer")?;
                let r = blend_rgba(&mut layer.surface, &src, x, y, sel.as_ref());
                self.doc.invalidate(Some(r));
                self.text = Some(TextSession { drawn: r, ..session });
            }
            "text_commit" => self.commit_pending(),
            // A fill pattern as a w×h coverage mask (for the toolbar preview).
            "pattern_preview" => {
                let style = FillStyle::parse(Self::str(req, "style")?).ok_or("unknown fill style")?;
                let (w, h) = (Self::int(req, "w")?.clamp(1, 256), Self::int(req, "h")?.clamp(1, 256));
                let mask: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| if style.on(x, y) { 255 } else { 0 })).collect();
                out.bins.push(("mask".into(), mask));
            }
            "text_cancel" => {
                self.text_restore();
            }
            "shape_commit" => self.commit_pending(),
            "shape_cancel" => {
                self.pending_action(PendingAction::Cancel);
                self.text_restore();
            }
            "shape_translate" => {
                let (dx, dy) = (Self::arg(req, "dx")?.as_f64().ok_or("dx")?, Self::arg(req, "dy")?.as_f64().ok_or("dy")?);
                let start = req.get("phase").and_then(Json::as_str) == Some("start");
                self.pending_action(PendingAction::Translate { dx: dx as f32, dy: dy as f32, start });
            }
            "shape_nudge" => {
                let (dx, dy) = (Self::arg(req, "dx")?.as_f64().ok_or("dx")?, Self::arg(req, "dy")?.as_f64().ok_or("dy")?);
                self.pending_action(PendingAction::Nudge { dx: dx as f32, dy: dy as f32 });
            }
            "shape_rotate" => {
                let angle = Self::arg(req, "angle")?.as_f64().ok_or("angle")?;
                let start = req.get("phase").and_then(Json::as_str) == Some("start");
                self.pending_action(PendingAction::Rotate { angle: angle as f32, start });
            }
            // -------------------------------------------------------------- history
            // While a line/shape is being edited, undo/redo step through its edits.
            "gradient_reverse" => self.pending_action(PendingAction::Reverse),
            "pending_delete" => self.pending_action(PendingAction::DeleteSelected),
            "undo" | "redo" if self.tools.has_pending() && self.tools.has_pending_edits() => {
                self.pending_action(if cmd == "undo" { PendingAction::Undo } else { PendingAction::Redo });
            }
            "undo" => {
                self.commit_pending(); // pending text: commit it, then undo it like any step
                self.history.undo(&mut self.doc)
            }
            "redo" => {
                self.commit_pending();
                self.history.redo(&mut self.doc)
            }
            "history_jump" => {
                let i = Self::int(req, "index")?.max(0) as usize;
                self.history.jump_to(&mut self.doc, i);
            }
            // -------------------------------------------------------------- selection
            "select_all" => {
                let before = self.doc.selection().cloned();
                let mut sel = Selection::new(self.doc.width(), self.doc.height());
                sel.select_all();
                self.doc.set_selection(Some(sel.clone()));
                self.push(SelectionEdit::new(before, Some(sel), "Select All"));
            }
            "deselect" => {
                if let Some(e) = self.deselect_entry("Deselect") {
                    self.history.push_boxed(e);
                }
            }
            "invert_selection" => {
                let before = self.doc.selection().cloned();
                let mut sel = before.clone().unwrap_or_else(|| {
                    let mut s = Selection::new(self.doc.width(), self.doc.height());
                    s.select_all();
                    s
                });
                sel.invert();
                let after = (!sel.is_empty()).then_some(sel);
                self.doc.set_selection(after.clone());
                self.push(SelectionEdit::new(before, after, "Invert Selection"));
            }
            "grow_selection" | "shrink_selection" => {
                let grow = cmd == "grow_selection";
                let Some(sel) = self.doc.active_selection().cloned() else { return Ok(()) };
                let mut w = sel.clone();
                if grow {
                    w.grow(1);
                } else {
                    w.shrink(1);
                }
                let after = (!w.is_empty()).then_some(w);
                self.doc.set_selection(after.clone());
                self.push(SelectionEdit::new(Some(sel), after, if grow { "Grow Selection" } else { "Shrink Selection" }));
            }
            "feather" => self.feather(req)?,
            // -------------------------------------------------------------- edit
            "copy" | "cut" | "copy_merged" => self.copy(cmd, out)?,
            "paste" => self.paste(req, bin)?,
            "erase_selection" => {
                let Some(sel) = self.doc.active_selection().cloned() else { return Ok(()) };
                self.erase_selected(&sel, "Erase Selection");
            }
            "fill_selection" => {
                let Some(sel) = self.doc.active_selection().cloned() else { return Ok(()) };
                let color = self.tools.settings.foreground;
                let idx = self.active_index();
                let before = self.doc.layers[idx].surface.pixels().to_vec();
                let r = selection_ops::fill_selection(&mut self.doc.layers[idx].surface, &sel, color);
                self.doc.invalidate(Some(r));
                if let Some(e) = BitmapLayerEdit::capture(&self.doc, self.active, &before, r, "Fill Selection") {
                    self.push(e);
                }
            }
            // -------------------------------------------------------------- image
            "crop" => {
                let Some(sel) = self.doc.active_selection().cloned() else { return Ok(()) };
                let d = transforms::crop_to_selection(&self.doc, &sel).ok_or("empty selection")?;
                self.status = format!("Cropped to {} × {}", d.width(), d.height());
                self.swap_document(d, "Crop to Selection", true);
            }
            "autocrop" => match transforms::auto_crop(&self.doc) {
                Some(d) => {
                    self.status = format!("Auto-cropped to {} × {}", d.width(), d.height());
                    self.swap_document(d, "Auto-Crop", true)
                }
                None => self.status = "Nothing to auto-crop.".into(),
            },
            "resize" => {
                let (w, h) = (Self::int(req, "w")?, Self::int(req, "h")?);
                openpdn_formats::check_size(w as i64, h as i64).map_err(|e| e.0)?;
                if (w, h) == (self.doc.width(), self.doc.height()) {
                    return Ok(());
                }
                let mode = match req.get("mode").and_then(Json::as_str).unwrap_or("Best") {
                    "Bilinear" => ResampleMode::Bilinear,
                    "NearestNeighbor" => ResampleMode::NearestNeighbor,
                    "SuperSampling" => ResampleMode::SuperSampling,
                    _ => ResampleMode::Best,
                };
                let d = transforms::document_resize(&self.doc, w, h, mode);
                self.status = format!("Resized to {w} × {h}");
                self.swap_document(d, "Resize", true);
            }
            "canvas" => {
                let (w, h) = (Self::int(req, "w")?, Self::int(req, "h")?);
                openpdn_formats::check_size(w as i64, h as i64).map_err(|e| e.0)?;
                if (w, h) == (self.doc.width(), self.doc.height()) {
                    return Ok(());
                }
                let anchors = [
                    CanvasAnchor::TopLeft,
                    CanvasAnchor::TopCenter,
                    CanvasAnchor::TopRight,
                    CanvasAnchor::MiddleLeft,
                    CanvasAnchor::MiddleCenter,
                    CanvasAnchor::MiddleRight,
                    CanvasAnchor::BottomLeft,
                    CanvasAnchor::BottomCenter,
                    CanvasAnchor::BottomRight,
                ];
                let a = anchors[Self::int(req, "anchor").unwrap_or(4).clamp(0, 8) as usize];
                let d = transforms::canvas_size(&self.doc, w, h, a);
                self.status = format!("Canvas resized to {w} × {h}");
                self.swap_document(d, "Canvas Size", true);
            }
            "flip_h" | "flip_v" | "rotate_180" => {
                let name = match cmd {
                    "flip_h" => "Flip Horizontal",
                    "flip_v" => "Flip Vertical",
                    _ => "Rotate 180°",
                };
                let op = cmd.to_owned();
                let apply = move |doc: &mut Document| {
                    for l in &mut doc.layers {
                        match op.as_str() {
                            "flip_h" => transforms::flip_horizontal(&mut l.surface),
                            "flip_v" => transforms::flip_vertical(&mut l.surface),
                            _ => transforms::rotate_180(&mut l.surface),
                        }
                    }
                    doc.invalidate(None);
                };
                let mut entries: Vec<Box<dyn HistoryEntry>> = self.deselect_entry(name).into_iter().collect();
                apply(&mut self.doc);
                entries.push(Box::new(SelfInverseEdit::new(name, apply)));
                self.push_all(entries, name);
            }
            "rotate_cw" | "rotate_ccw" => {
                let d = if cmd == "rotate_cw" { transforms::document_rotate_90_cw(&self.doc) } else { transforms::document_rotate_90_ccw(&self.doc) };
                self.swap_document(d, if cmd == "rotate_cw" { "Rotate 90° Clockwise" } else { "Rotate 90° Counter-Clockwise" }, true);
            }
            "flatten" => {
                if self.doc.layers.len() <= 1 {
                    return Ok(());
                }
                let mut d = transforms::flatten(&self.doc);
                d.set_selection(self.doc.selection().cloned());
                self.swap_document(d, "Flatten", false);
            }
            // -------------------------------------------------------------- layers
            "layer_select" => {
                let i = Self::int(req, "index")? as usize;
                let l = self.doc.layers.get(i).ok_or("no such layer")?;
                self.active = l.id();
                self.force_state = true;
            }
            "layer_add" => {
                let l = Layer::new(self.doc.width(), self.doc.height(), format!("Layer {}", self.doc.layers.len() + 1));
                self.active = l.id();
                let idx = self.doc.layers.len();
                self.doc.layers.push(l);
                self.doc.invalidate(None);
                let e = LayerAddEdit::new(&self.doc, idx, "Add New Layer");
                self.push(e);
            }
            "layer_delete" => {
                if self.doc.layers.len() <= 1 {
                    return Ok(());
                }
                let idx = self.active_index();
                let removed = self.doc.layers.remove(idx);
                self.doc.invalidate(None);
                self.push(LayerRemoveEdit::new(removed, idx));
                self.active = self.doc.layers[idx.min(self.doc.layers.len() - 1)].id();
            }
            "layer_duplicate" => {
                let idx = self.active_index();
                let copy = self.doc.layers[idx].duplicate(format!("{} copy", self.doc.layers[idx].name));
                self.active = copy.id();
                self.doc.layers.insert(idx + 1, copy);
                self.doc.invalidate(None);
                let e = LayerAddEdit::new(&self.doc, idx + 1, "Duplicate Layer");
                self.push(e);
            }
            "layer_merge_down" => {
                let idx = self.active_index();
                if idx < 1 {
                    return Ok(());
                }
                let bottom_id = self.doc.layers[idx - 1].id();
                let before = self.doc.layers[idx - 1].surface.pixels().to_vec();
                let top = self.doc.layers.remove(idx);
                blend::apply_layer(&mut self.doc.layers[idx - 1].surface, &top.surface, top.blend_mode, top.opacity);
                self.doc.invalidate(None);
                let full = self.doc.bounds();
                let mut entries: Vec<Box<dyn HistoryEntry>> = Vec::new();
                if let Some(e) = BitmapLayerEdit::capture(&self.doc, bottom_id, &before, full, "Merge Layer Down") {
                    entries.push(Box::new(e));
                }
                entries.push(Box::new(LayerRemoveEdit::new(top, idx)));
                self.history.push(CompoundEdit::new(entries, "Merge Layer Down"));
                self.active = bottom_id;
            }
            "layer_up" | "layer_down" => {
                let idx = self.active_index();
                let to = if cmd == "layer_up" { idx + 1 } else { idx.wrapping_sub(1) };
                if to >= self.doc.layers.len() {
                    return Ok(());
                }
                let l = self.doc.layers.remove(idx);
                self.doc.layers.insert(to, l);
                self.doc.invalidate(None);
                self.push(LayerMoveEdit::new(idx, to));
            }
            "layer_visible" => {
                let i = Self::int(req, "index")? as usize;
                let v = Self::bool(req, "visible");
                let l = self.doc.layers.get_mut(i).ok_or("no such layer")?;
                if l.visible != v {
                    let id = l.id();
                    l.visible = v;
                    self.doc.invalidate(None);
                    self.push(LayerPropertyEdit::new(id, LayerProperty::Visible(!v), LayerProperty::Visible(v)));
                }
            }
            "layer_opacity" => {
                let i = Self::int(req, "index")? as usize;
                let v = Self::int(req, "opacity")?.clamp(0, 255) as u8;
                let old = req.get("from").and_then(Json::as_f64).map(|f| f as u8);
                let l = self.doc.layers.get_mut(i).ok_or("no such layer")?;
                let id = l.id();
                let before = old.unwrap_or(l.opacity);
                l.opacity = v;
                self.doc.invalidate(None);
                // `commit:false` is a live slider drag; the final call records one edit.
                if Self::bool(req, "commit") && before != v {
                    self.push(LayerPropertyEdit::new(id, LayerProperty::Opacity(before), LayerProperty::Opacity(v)));
                }
            }
            "layer_props" => self.layer_props(req)?,
            "layer_flip_h" | "layer_flip_v" => {
                let id = self.active;
                let h = cmd == "layer_flip_h";
                let apply = move |doc: &mut Document| {
                    if let Some(l) = doc.layer_mut(id) {
                        if h {
                            transforms::flip_horizontal(&mut l.surface);
                        } else {
                            transforms::flip_vertical(&mut l.surface);
                        }
                    }
                    doc.invalidate(None);
                };
                apply(&mut self.doc);
                self.push(SelfInverseEdit::new(if h { "Flip Layer Horizontal" } else { "Flip Layer Vertical" }, apply));
            }
            "import_layer" => self.import_layer(req, bin)?,
            // -------------------------------------------------------------- adjustments / effects
            "fx" => self.fx(req)?,
            other => return Err(format!("unknown command {other}")),
        }
        Ok(())
    }

    // ------------------------------------------------------------------ pointer & settings

    fn pointer(&mut self, req: &Json) -> R<()> {
        let phase = Self::str(req, "phase")?;
        let (x, y) = (Self::int(req, "x")?, Self::int(req, "y")?);
        let button = match req.get("button").and_then(Json::as_f64).unwrap_or(0.0) as i32 {
            0 => PointerButton::Left,
            1 => PointerButton::Middle,
            2 => PointerButton::Right,
            _ => PointerButton::None,
        };
        let mut mods = Modifiers::NONE;
        if Self::bool(req, "shift") {
            mods = mods | Modifiers::SHIFT;
        }
        if Self::bool(req, "ctrl") {
            mods = mods | Modifiers::CONTROL;
        }
        if Self::bool(req, "alt") {
            mods = mods | Modifiers::ALT;
        }
        let pressure = req.get("pressure").and_then(Json::as_f64).unwrap_or(1.0) as f32;
        let hr = req.get("handleRadius").and_then(Json::as_f64).unwrap_or(4.0);
        self.fix_active();
        let id = self.active;
        let busy_before = self.tools.active().is_busy();
        match phase {
            "down" => self.tools.pointer_down(&mut self.doc, &mut self.history, id, x, y, button, mods, pressure, hr),
            "move" => self.tools.pointer_move(&mut self.doc, &mut self.history, id, x, y, mods, pressure, hr),
            "up" => self.tools.pointer_up(&mut self.doc, &mut self.history, id, x, y, button, mods, pressure, hr),
            _ => return Err("bad pointer phase".into()),
        }
        if busy_before != self.tools.active().is_busy() || self.tools.active_kind() == ToolKind::MoveSelected {
            self.force_state = true; // handles / busy flag
        }
        Ok(())
    }

    fn setting(&mut self, key: &str, v: &Json) -> R<()> {
        let st = &mut self.tools.settings;
        let b = || v.as_bool().ok_or_else(|| format!("{key} expects a boolean"));
        let i = || v.as_f64().map(|f| f as i32).ok_or_else(|| format!("{key} expects a number"));
        let name = || v.as_str().ok_or_else(|| format!("{key} expects a name"));
        macro_rules! pick {
            ($($n:literal => $val:expr),* $(,)?) => {
                match name()? { $($n => $val,)* other => return Err(format!("bad value {other} for {key}")) }
            };
        }
        match key {
            "brush" => st.set_brush_size(i()?),
            "tolerance" => st.set_wand_tolerance(i()?),
            "aa" => st.antialiasing = b()?,
            "alphaBlend" => st.alpha_blending = b()?,
            "sampleAll" => st.sample_all_layers = b()?,
            "pressure" => st.pressure_sensitivity = b()?,
            "shape" => st.shape_mode = pick!("Outline" => ShapeDrawMode::Outline, "Fill" => ShapeDrawMode::Fill, "FillOutline" => ShapeDrawMode::FillOutline),
            "gradient" => {
                st.gradient_type = pick!(
                    "Linear" => GradientType::Linear, "LinearReflected" => GradientType::LinearReflected,
                    "Diamond" => GradientType::Diamond, "Radial" => GradientType::Radial, "Conical" => GradientType::Conical
                )
            }
            "gradientMode" => st.gradient_color_mode = pick!("Color" => GradientColorMode::Color, "Transparency" => GradientColorMode::Transparency),
            "gradientRepeat" => st.gradient_repeat = pick!("None" => GradientRepeat::None, "Repeat" => GradientRepeat::Repeat, "Reflect" => GradientRepeat::Reflect),
            "gradientDither" => st.gradient_dither = b()?,
            "warpMode" => st.warp_mode = WarpMode::parse(name()?).ok_or("unknown distort mode")?,
            "warpStrength" => st.warp_strength = i()?.clamp(1, 100),
            "resampling" => {
                st.resampling = pick!(
                    "NearestNeighbor" => TransformResampling::NearestNeighbor,
                    "NearestNeighborAntialiased" => TransformResampling::NearestNeighborAntialiased,
                    "Bilinear" => TransformResampling::Bilinear,
                    "MultisampleBilinear" => TransformResampling::MultisampleBilinear,
                    "Anisotropic" => TransformResampling::Anisotropic,
                    "Bicubic" => TransformResampling::Bicubic
                )
            }
            "combine" => {
                st.selection_combine_mode = pick!(
                    "Replace" => SelectionCombineMode::Replace, "Union" => SelectionCombineMode::Union,
                    "Subtract" => SelectionCombineMode::Subtract, "Intersect" => SelectionCombineMode::Intersect,
                    "Xor" => SelectionCombineMode::Xor
                )
            }
            "flood" => st.flood_mode = pick!("Contiguous" => FloodMode::Contiguous, "Global" => FloodMode::Global),
            "selectionAA" => st.selection_antialias = b()?,
            "fillStyle" => st.fill_style = FillStyle::parse(v.as_str().ok_or("fillStyle expects a name")?).ok_or("unknown fill style")?,
            "dash" => st.line_dash = pick!("Solid" => LineDash::Solid, "Dash" => LineDash::Dash, "LongDash" => LineDash::LongDash, "Dot" => LineDash::Dot, "DenseDot" => LineDash::DenseDot, "DashDot" => LineDash::DashDot, "DashDotDot" => LineDash::DashDotDot),
            "shapeType" => st.shape_type = ShapeType::parse(v.as_str().ok_or("shapeType expects a name")?).ok_or("unknown shape type")?,
            "shapeSides" => st.shape_sides = (v.as_f64().ok_or("shapeSides expects a number")?.round() as u32).clamp(3, 64),
            "starInner" => st.star_inner = (v.as_f64().ok_or("starInner expects a number")? as f32).clamp(0.05, 0.95),
            "shapeRadius" => st.shape_radius = (v.as_f64().ok_or("shapeRadius expects a number")? as f32).clamp(0.0, 10000.0),
            "capStart" => st.line_cap_start = pick!("Flat" => LineCap::Flat, "Square" => LineCap::Square, "Round" => LineCap::Round, "Arrow" => LineCap::Arrow, "ArrowOpen" => LineCap::ArrowOpen, "ArrowIn" => LineCap::ArrowIn, "Diamond" => LineCap::Diamond, "Circle" => LineCap::Circle),
            "capEnd" => st.line_cap_end = pick!("Flat" => LineCap::Flat, "Square" => LineCap::Square, "Round" => LineCap::Round, "Arrow" => LineCap::Arrow, "ArrowOpen" => LineCap::ArrowOpen, "ArrowIn" => LineCap::ArrowIn, "Diamond" => LineCap::Diamond, "Circle" => LineCap::Circle),
            "curve" => st.curve_type = pick!("Straight" => CurveType::Straight, "Spline" => CurveType::Spline, "Bezier" => CurveType::Bezier),
            "gamma" => st.gamma_correct = b()?,
            "toleranceAlpha" => st.tolerance_alpha = pick!("Straight" => ToleranceAlpha::Straight, "Premultiplied" => ToleranceAlpha::Premultiplied),
            "wandSampleImage" => st.wand_sample_image = b()?,
            // {mode: "Any" | "FixedRatio" | "FixedSize", w, h}; FixedSize in pixels.
            "selectSize" => {
                let mode = v.get("mode").and_then(Json::as_str).ok_or("selectSize expects {mode, w, h}")?;
                let num = |k: &str| v.get(k).and_then(Json::as_f64).filter(|f| f.is_finite() && *f > 0.0);
                st.select_size = match mode {
                    "Any" => SelectSize::Any,
                    "FixedRatio" => SelectSize::FixedRatio(num("w").ok_or("ratio width must be > 0")?, num("h").ok_or("ratio height must be > 0")?),
                    "FixedSize" => {
                        let px = |k: &str| num(k).map(|f| (f.round() as i32).clamp(1, 65535)).ok_or(format!("{k} must be > 0"));
                        SelectSize::FixedSize(px("w")?, px("h")?)
                    }
                    other => return Err(format!("bad selectSize mode {other}")),
                };
            }
            "eraserPattern" => {
                let n = name()?;
                st.eraser_pattern = *FillPattern::ALL.iter().find(|p| format!("{p:?}") == n).ok_or("bad pattern")?;
            }
            _ => return Err(format!("unknown setting {key}")),
        }
        st.revision += 1;
        Ok(())
    }

    /// Blends rendered text (RGBA from the host's font renderer) at (x, y).
    fn stamp_text(&mut self, req: &Json, bin: &[u8]) -> R<()> {
        let src = rgba_to_surface(Self::int(req, "w")?, Self::int(req, "h")?, bin)?;
        let (x, y) = (Self::int(req, "x")?, Self::int(req, "y")?);
        self.fix_active();
        let idx = self.active_index();
        let sel = self.doc.active_selection().cloned();
        let layer = &mut self.doc.layers[idx];
        let before = layer.surface.pixels().to_vec();
        let r = blend_rgba(&mut layer.surface, &src, x, y, sel.as_ref());
        if r.is_empty() {
            return Ok(());
        }
        self.doc.invalidate(Some(r));
        if let Some(e) = BitmapLayerEdit::capture(&self.doc, self.active, &before, r, "Text") {
            self.push(e);
        }
        Ok(())
    }

    // ------------------------------------------------------------------ selection / edit

    fn feather(&mut self, req: &Json) -> R<()> {
        match Self::str(req, "phase")? {
            "begin" => {
                let Some(sel) = self.doc.active_selection().cloned() else { return Err("nothing selected".into()) };
                self.preview = Some(Preview::Feather { original: sel });
                self.feather_apply(Self::int(req, "radius").unwrap_or(5));
            }
            "preview" => self.feather_apply(Self::int(req, "radius")?),
            "commit" => {
                if let Some(Preview::Feather { original }) = self.preview.take() {
                    let after = self.doc.selection().cloned();
                    self.push(SelectionEdit::new(Some(original), after, "Feather Selection"));
                }
            }
            _ => self.cancel_preview(),
        }
        Ok(())
    }

    fn feather_apply(&mut self, r: i32) {
        if let Some(Preview::Feather { original }) = &self.preview {
            let mut w = original.clone();
            w.feather(r);
            self.doc.set_selection(Some(w));
        }
    }

    fn erase_selected(&mut self, sel: &Selection, name: &str) {
        let idx = self.active_index();
        let before = self.doc.layers[idx].surface.pixels().to_vec();
        let r = selection_ops::erase_selection(&mut self.doc.layers[idx].surface, sel);
        self.doc.invalidate(Some(r));
        if let Some(e) = BitmapLayerEdit::capture(&self.doc, self.active, &before, r, name) {
            self.push(e);
        }
    }

    fn copy(&mut self, cmd: &str, out: &mut Reply) -> R<()> {
        let sel = self.doc.active_selection().cloned();
        let img = if cmd == "copy_merged" {
            let flat = self.doc.composite();
            match &sel {
                Some(s) => selection_ops::copy_from_selection(&flat, s).ok_or("empty selection")?,
                None => flat,
            }
        } else {
            let sel = sel.ok_or("Nothing selected to copy.")?;
            let idx = self.active_index();
            let img = selection_ops::copy_from_selection(&self.doc.layers[idx].surface, &sel).ok_or("empty selection")?;
            if cmd == "cut" {
                self.erase_selected(&sel, "Cut");
            }
            img
        };
        out.set("w", num(img.width()));
        out.set("h", num(img.height()));
        out.bin("rgba", img.to_rgba8());
        self.status = format!("Copied {} × {}", img.width(), img.height());
        Ok(())
    }

    /// Paste: `mode` = "layer" (into the active layer), "new_layer", "new_image".
    /// `x`,`y` = visible top-left; `expand` = grow the canvas to fit.
    fn paste(&mut self, req: &Json, bin: &[u8]) -> R<()> {
        let img = rgba_to_surface(Self::int(req, "w")?, Self::int(req, "h")?, bin)?;
        let mode = Self::str(req, "mode").unwrap_or("layer");
        if mode == "new_image" {
            self.set_document(Document::from_single_layer(img, "Background"), "Untitled");
            return Ok(());
        }
        let name = if mode == "new_layer" { "Paste in to New Layer" } else { "Paste" };
        let mut entries: Vec<Box<dyn HistoryEntry>> = Vec::new();
        if Self::bool(req, "expand") && (img.width() > self.doc.width() || img.height() > self.doc.height()) {
            entries.extend(self.deselect_entry(name));
            let d = transforms::canvas_size(&self.doc, self.doc.width().max(img.width()), self.doc.height().max(img.height()), CanvasAnchor::TopLeft);
            let idx = self.active_index();
            entries.push(Box::new(DocumentSwapEdit::apply(&mut self.doc, d, name)));
            self.active = self.doc.layers[idx].id();
        }
        let vx = req.get("x").and_then(Json::as_f64).unwrap_or(0.0) as i32;
        let vy = req.get("y").and_then(Json::as_f64).unwrap_or(0.0) as i32;
        let ox = vx.clamp(0, (self.doc.width() - img.width()).max(0));
        let oy = vy.clamp(0, (self.doc.height() - img.height()).max(0));
        let rect;
        if mode == "new_layer" {
            let mut layer = Layer::new(self.doc.width(), self.doc.height(), format!("Layer {}", self.doc.layers.len() + 1));
            rect = selection_ops::paste_into(&mut layer.surface, &img, ox, oy);
            let at = self.active_index() + 1;
            self.active = layer.id();
            self.doc.layers.insert(at, layer);
            entries.push(Box::new(LayerAddEdit::new(&self.doc, at, name)));
        } else {
            let idx = self.active_index();
            let before = self.doc.layers[idx].surface.pixels().to_vec();
            rect = selection_ops::paste_into(&mut self.doc.layers[idx].surface, &img, ox, oy);
            if let Some(e) = BitmapLayerEdit::capture(&self.doc, self.active, &before, rect, name) {
                entries.push(Box::new(e));
            }
        }
        self.doc.invalidate(None);
        let before_sel = self.doc.selection().cloned();
        let after = (!rect.is_empty()).then(|| {
            let mut s = Selection::new(self.doc.width(), self.doc.height());
            s.set_rectangle(rect.x, rect.y, rect.width, rect.height);
            s
        });
        self.doc.set_selection(after.clone());
        entries.push(Box::new(SelectionEdit::new(before_sel, after, name)));
        self.push_all(entries, name);
        self.tools.set_active(ToolKind::MoveSelected);
        self.force_state = true;
        self.status = format!("Pasted {} × {}", img.width(), img.height());
        Ok(())
    }

    fn import_layer(&mut self, req: &Json, bin: &[u8]) -> R<()> {
        let name = Self::str(req, "name").unwrap_or("Imported").to_owned();
        let img = match openpdn_formats::sniff(bin) {
            openpdn_formats::Sniffed::Other => rgba_to_surface(Self::int(req, "w")?, Self::int(req, "h")?, bin)?,
            openpdn_formats::Sniffed::Png => png::decode(bin).map_err(|e| e.0)?.surface,
            openpdn_formats::Sniffed::OpenPdn => openpdn::read(bin).map_err(|e| e.0)?.composite(),
            openpdn_formats::Sniffed::PdnLegacy => pdn::read(bin).map_err(|e| e.0)?.composite(),
        };
        let mut entries: Vec<Box<dyn HistoryEntry>> = Vec::new();
        if Self::bool(req, "expand") && (img.width() > self.doc.width() || img.height() > self.doc.height()) {
            entries.extend(self.deselect_entry("Import From File"));
            let d = transforms::canvas_size(&self.doc, self.doc.width().max(img.width()), self.doc.height().max(img.height()), CanvasAnchor::TopLeft);
            entries.push(Box::new(DocumentSwapEdit::apply(&mut self.doc, d, "Import From File")));
        }
        let mut layer = Layer::new(self.doc.width(), self.doc.height(), name);
        transforms::blit_clipped(&img, &mut layer.surface, 0, 0);
        self.active = layer.id();
        self.doc.layers.push(layer);
        self.doc.invalidate(None);
        let idx = self.doc.layers.len() - 1;
        entries.push(Box::new(LayerAddEdit::new(&self.doc, idx, "Import From File")));
        self.push_all(entries, "Import From File");
        Ok(())
    }

    fn layer_props(&mut self, req: &Json) -> R<()> {
        let phase = Self::str(req, "phase")?;
        if phase == "begin" {
            let l = &self.doc.layers[self.active_index()];
            self.preview = Some(Preview::LayerProps { layer: l.id(), before: (l.name.clone(), l.visible, l.opacity, l.blend_mode) });
            return Ok(());
        }
        let Some(Preview::LayerProps { layer, before }) = &self.preview else { return Err("no layer properties session".into()) };
        let (id, before) = (*layer, before.clone());
        match phase {
            "preview" | "commit" => {
                let l = self.doc.layer_mut(id).ok_or("layer gone")?;
                if let Some(n) = req.get("name").and_then(Json::as_str) {
                    l.name = n.to_owned();
                }
                if let Some(v) = req.get("visible").and_then(Json::as_bool) {
                    l.visible = v;
                }
                if let Some(o) = req.get("opacity").and_then(Json::as_f64) {
                    l.opacity = o.clamp(0.0, 255.0) as u8;
                }
                if let Some(b) = req.get("blend").and_then(Json::as_str) {
                    l.blend_mode = BlendMode::from_name(b).ok_or("bad blend mode")?;
                }
                let now = (l.name.clone(), l.visible, l.opacity, l.blend_mode);
                self.doc.invalidate(None);
                self.force_state = true;
                if phase == "commit" {
                    self.preview = None;
                    let mut e: Vec<Box<dyn HistoryEntry>> = Vec::new();
                    if before.0 != now.0 {
                        e.push(Box::new(LayerPropertyEdit::new(id, LayerProperty::Name(before.0), LayerProperty::Name(now.0))));
                    }
                    if before.1 != now.1 {
                        e.push(Box::new(LayerPropertyEdit::new(id, LayerProperty::Visible(before.1), LayerProperty::Visible(now.1))));
                    }
                    if before.3 != now.3 {
                        e.push(Box::new(LayerPropertyEdit::new(id, LayerProperty::BlendMode(before.3), LayerProperty::BlendMode(now.3))));
                    }
                    if before.2 != now.2 {
                        e.push(Box::new(LayerPropertyEdit::new(id, LayerProperty::Opacity(before.2), LayerProperty::Opacity(now.2))));
                    }
                    self.push_all(e, "Layer Properties");
                }
            }
            _ => self.cancel_preview(),
        }
        Ok(())
    }

    // ------------------------------------------------------------------ effects

    fn values(req: &Json, def: &fx::FxDef) -> fx::Values {
        let mut v = fx::defaults(def);
        if let Some(a) = req.get("values").and_then(Json::as_array) {
            for (i, x) in a.iter().enumerate().take(def.sliders.len()) {
                if let Some(n) = x.as_f64() {
                    v.v[i] = (n as i32).clamp(def.sliders[i].min, def.sliders[i].max);
                }
            }
        }
        if let Some(a) = req.get("checks").and_then(Json::as_array) {
            for (i, x) in a.iter().enumerate().take(v.c.len()) {
                v.c[i] = x.as_bool().unwrap_or(v.c[i]);
            }
        }
        if let Some(a) = req.get("choices").and_then(Json::as_array) {
            for (i, x) in a.iter().enumerate().take(v.ch.len()) {
                if let Some(n) = x.as_f64() {
                    v.ch[i] = (n as usize).min(def.choices[i].options.len() - 1);
                }
            }
        }
        if let Some(c) = req.get("curves") {
            let mode = c.get("mode").and_then(Json::as_f64).unwrap_or(0.0) as u8;
            let pts = c
                .get("points")
                .and_then(Json::as_array)
                .map(|curves| {
                    curves
                        .iter()
                        .map(|p| {
                            p.as_array()
                                .unwrap_or(&[])
                                .chunks(2)
                                .filter_map(|xy| Some((xy.first()?.as_f64()? as i32, xy.get(1)?.as_f64()? as i32)))
                                .map(|(x, y)| (x.clamp(0, 255), y.clamp(0, 255)))
                                .collect()
                        })
                        .collect()
                })
                .unwrap_or_default();
            v.curves = Some((mode, pts));
        }
        v
    }

    fn fx(&mut self, req: &Json) -> R<()> {
        let phase = Self::str(req, "phase")?;
        match phase {
            "begin" | "apply" => {
                let id = Self::str(req, "fx")?;
                let def = fx::find(id).ok_or_else(|| format!("unknown effect {id}"))?;
                self.cancel_preview();
                self.fix_active();
                let idx = self.active_index();
                let original = self.doc.layers[idx].surface.pixels().to_vec();
                let sel = self.doc.active_selection().cloned();
                self.preview = Some(Preview::Fx { id: id.to_owned(), layer: self.active, original, sel });
                self.fx_render(&Self::values(req, def))?;
                if phase == "apply" || def.direct {
                    self.fx_commit(def.title);
                }
            }
            "preview" => {
                let Some(Preview::Fx { id, .. }) = &self.preview else { return Err("no effect preview".into()) };
                let def = fx::find(id).unwrap();
                self.fx_render(&Self::values(req, def))?;
            }
            "commit" => {
                let Some(Preview::Fx { id, .. }) = &self.preview else { return Err("no effect preview".into()) };
                let def = fx::find(id).unwrap();
                if req.get("values").is_some() || req.get("curves").is_some() {
                    self.fx_render(&Self::values(req, def))?;
                }
                self.fx_commit(def.title);
            }
            _ => self.cancel_preview(),
        }
        Ok(())
    }

    fn fx_render(&mut self, v: &fx::Values) -> R<()> {
        let Some(Preview::Fx { id, layer, original, sel }) = &self.preview else { return Ok(()) };
        let (fg, bg) = (self.tools.settings.foreground, self.tools.settings.background);
        let l = self.doc.layer_mut(*layer).ok_or("layer gone")?;
        l.surface.pixels_mut().copy_from_slice(original);
        let built = fx::build(id, v, fg, bg, &l.surface, sel.as_ref()).ok_or("unknown effect")?;
        let region = sel.as_ref().map_or(l.surface.bounds(), |s| s.bounds());
        match built {
            fx::Built::Op(op) => apply_op(op.as_ref() as &dyn PixelOp, &mut l.surface, sel.as_ref()),
            fx::Built::Effect(e) => openpdn_core::effects::apply(e.as_ref(), &mut l.surface, sel.as_ref()),
            fx::Built::RotateZoom { angle, zoom, tile } => {
                let src = Surface::from_pixels(l.width(), l.height(), original.clone());
                transforms::rotate_zoom(&src, &mut l.surface, angle, zoom, 0.0, 0.0, tile);
                self.doc.invalidate(None);
                return Ok(());
            }
        }
        self.doc.invalidate(Some(region));
        Ok(())
    }

    fn fx_commit(&mut self, title: &str) {
        let Some(Preview::Fx { id, layer, original, sel }) = self.preview.take() else { return };
        let full = self.doc.bounds();
        let rect = if id == "rotate-zoom" { full } else { sel.as_ref().map_or(full, |s| s.bounds().intersect(&full)) };
        if let Some(e) = BitmapLayerEdit::capture(&self.doc, layer, &original, rect, title) {
            self.push(e);
        }
        self.status = title.to_owned();
    }

    fn cancel_preview(&mut self) {
        match self.preview.take() {
            Some(Preview::Fx { layer, original, .. }) => {
                if let Some(l) = self.doc.layer_mut(layer) {
                    l.surface.pixels_mut().copy_from_slice(&original);
                }
                self.doc.invalidate(None);
            }
            Some(Preview::Feather { original }) => self.doc.set_selection(Some(original)),
            Some(Preview::LayerProps { layer, before }) => {
                if let Some(l) = self.doc.layer_mut(layer) {
                    (l.name, l.visible, l.opacity, l.blend_mode) = before;
                }
                self.doc.invalidate(None);
                self.force_state = true;
            }
            None => {}
        }
    }
}

/// Area-averaged RGBA thumbnail fitting `size`×`size` (letterboxed into the square).
pub fn thumbnail(s: &Surface, size: i32) -> Vec<u8> {
    let scale = (size as f64 / s.width().max(s.height()) as f64).min(1.0);
    let tw = ((s.width() as f64 * scale).round() as i32).max(1);
    let th = ((s.height() as f64 * scale).round() as i32).max(1);
    let small = transforms::resize(s, tw, th, ResampleMode::SuperSampling);
    let mut out = vec![0u8; (size * size * 4) as usize];
    let (ox, oy) = ((size - tw) / 2, (size - th) / 2);
    for y in 0..th {
        for (x, p) in small.row(y).iter().enumerate() {
            let o = (((oy + y) * size + ox + x as i32) * 4) as usize;
            out[o..o + 4].copy_from_slice(&[p.r, p.g, p.b, p.a]);
        }
    }
    out
}
