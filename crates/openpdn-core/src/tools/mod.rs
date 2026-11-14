//! Interactive tools (C# `Tools`). A tool receives pointer events in image
//! pixel coordinates through a [`ToolContext`] and edits the document,
//! pushing history entries when an operation completes.

use crate::brushes::FillPattern;
use crate::color::Bgra;
use crate::document::{Document, Layer, LayerId};
use crate::history::HistoryStack;
use crate::rect::{PixelRect, RectF};
use crate::resample::TransformResampling;
use crate::selection::{Selection, SelectionCombineMode};

pub mod gradient;
pub mod handles;
pub mod move_tool;
pub mod paint;
pub mod select;
pub mod shape_geom;
pub mod shapes;
pub mod warp;

pub use gradient::{GradientColorMode, GradientRepeat, GradientType};
pub use handles::HandleKind;
pub use shapes::ShapeDrawMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    None,
    Left,
    Right,
    Middle,
}

/// Keyboard modifiers held during a pointer event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(pub u8);

impl Modifiers {
    pub const NONE: Modifiers = Modifiers(0);
    pub const SHIFT: Modifiers = Modifiers(1);
    pub const CONTROL: Modifiers = Modifiers(2);
    pub const ALT: Modifiers = Modifiers(4);

    #[inline]
    pub fn shift(self) -> bool {
        self.0 & 1 != 0
    }
    #[inline]
    pub fn ctrl(self) -> bool {
        self.0 & 2 != 0
    }
    #[inline]
    pub fn alt(self) -> bool {
        self.0 & 4 != 0
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, o: Modifiers) -> Modifiers {
        Modifiers(self.0 | o.0)
    }
}

/// Magic wand / paint bucket region: connected to the clicked pixel, or every
/// matching pixel in the image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FloodMode {
    #[default]
    Contiguous,
    Global,
}

/// How rectangle and ellipse selections are sized while dragging.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SelectSize {
    #[default]
    Any,
    /// Width : height (both > 0).
    FixedRatio(f64, f64),
    /// Exact size in pixels, extending from the click in the drag direction.
    FixedSize(i32, i32),
}

/// Something done to a tool's pending (still editable) result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PendingAction {
    /// Make it permanent (one history step).
    Commit,
    /// Throw it away, restoring the pixels underneath.
    Cancel,
    /// Re-render after a setting or colour changed.
    Refresh,
    /// Move it by (dx, dy) from where it was when `start` was set.
    Translate { dx: f32, dy: f32, start: bool },
    /// Move it by (dx, dy) from where it is now (arrow keys).
    Nudge { dx: f32, dy: f32 },
    /// Rotate it by `angle` radians from how it was when `start` was set.
    Rotate { angle: f32, start: bool },
    /// Step back through / forward again the edits made to it.
    Undo,
    Redo,
    /// Turn it around (a gradient's colour order).
    Reverse,
    /// Remove the selected part (a gradient's colour stop).
    DeleteSelected,
}

/// What the UI draws over a pending result: its handles.
#[derive(Clone, Debug, PartialEq)]
pub enum EditOverlay {
    /// A line/curve's control points.
    Line { curve: shapes::CurveType, points: Vec<(f32, f32)> },
    /// A box shape: resize handles, rotation handle (and a custom polygon's corners).
    Box { geom: shapes::BoxGeom, shape: shape_geom::ShapeType, vertices: Vec<(f32, f32)> },
    /// A gradient's end points and colour stops (positions along it).
    Gradient { a: (f32, f32), b: (f32, f32), stops: Vec<(f32, Bgra)>, selected: Option<usize> },
    /// A perspective transform's corners: top-left, top-right, bottom-right, bottom-left.
    Quad { corners: [(f32, f32); 4] },
}

/// Options shared by the tools (C# `ToolManager` properties).
#[derive(Clone, Debug)]
pub struct ToolSettings {
    pub shape_mode: ShapeDrawMode,
    pub antialiasing: bool,
    pub alpha_blending: bool,
    pub selection_combine_mode: SelectionCombineMode,
    pub gradient_type: GradientType,
    pub gradient_color_mode: GradientColorMode,
    pub gradient_repeat: GradientRepeat,
    /// Dither gradients so smooth ramps don't show bands.
    pub gradient_dither: bool,
    /// Distort brush: what it does, and how strongly (1–100).
    pub warp_mode: warp::WarpMode,
    pub warp_strength: i32,
    pub sample_all_layers: bool,
    pub eraser_pattern: FillPattern,
    pub resampling: TransformResampling,
    /// Resample in linear light (gamma corrected) rather than on sRGB values.
    pub gamma_correct: bool,
    pub pressure_sensitivity: bool,
    pub flood_mode: FloodMode,
    /// Lines stay editable (movable, bendable) until committed. Off in the
    /// core (the C# port draws them immediately); the app turns it on.
    pub editable_shapes: bool,
    pub curve_type: shapes::CurveType,
    pub line_dash: crate::draw::LineDash,
    pub fill_style: crate::draw::FillStyle,
    pub shape_type: shape_geom::ShapeType,
    /// Sides of a regular polygon, points of a star.
    pub shape_sides: u32,
    /// A star's inner radius as a fraction of the outer one.
    pub star_inner: f32,
    /// Corner radius of rounded rectangles.
    pub shape_radius: f32,
    pub line_cap_start: crate::draw::LineCap,
    pub line_cap_end: crate::draw::LineCap,
    /// How alpha takes part in wand/bucket tolerance comparisons.
    pub tolerance_alpha: crate::selection::ToleranceAlpha,
    /// Paint bucket compares with the tolerance setting (the C# port, and
    /// so this default, fills exact matches only).
    pub fill_tolerance: bool,
    /// Magic wand samples the whole image (true, the C# port's behaviour) or
    /// only the active layer.
    pub wand_sample_image: bool,
    /// Selection quality: antialiased (true) or pixelated clipping.
    pub selection_antialias: bool,
    pub select_size: SelectSize,
    brush_size: i32,
    wand_tolerance: i32,
    pub foreground: Bgra,
    pub background: Bgra,
    recent_colors: Vec<Bgra>,
    /// Bumped whenever a setting or colour changes.
    pub revision: u64,
}

impl Default for ToolSettings {
    fn default() -> Self {
        ToolSettings {
            shape_mode: ShapeDrawMode::Outline,
            antialiasing: true,
            alpha_blending: true,
            selection_combine_mode: SelectionCombineMode::Replace,
            gradient_type: GradientType::Linear,
            gradient_color_mode: GradientColorMode::Color,
            gradient_repeat: GradientRepeat::None,
            gradient_dither: false,
            warp_mode: warp::WarpMode::Pinch,
            warp_strength: 50,
            sample_all_layers: false,
            eraser_pattern: FillPattern::Solid,
            resampling: TransformResampling::Bilinear,
            gamma_correct: false,
            pressure_sensitivity: true,
            flood_mode: FloodMode::Contiguous,
            editable_shapes: false,
            curve_type: shapes::CurveType::Straight,
            line_dash: crate::draw::LineDash::Solid,
            fill_style: crate::draw::FillStyle::Solid,
            shape_type: shape_geom::ShapeType::Rectangle,
            shape_sides: 5,
            star_inner: 0.45,
            shape_radius: 10.0,
            line_cap_start: crate::draw::LineCap::Round,
            line_cap_end: crate::draw::LineCap::Round,
            tolerance_alpha: crate::selection::ToleranceAlpha::Straight,
            fill_tolerance: false,
            wand_sample_image: true,
            selection_antialias: true,
            select_size: SelectSize::Any,
            brush_size: 8,
            wand_tolerance: 32,
            foreground: Bgra::BLACK,
            background: Bgra::WHITE,
            recent_colors: Vec::new(),
            revision: 0,
        }
    }
}

impl ToolSettings {
    pub fn brush_size(&self) -> i32 {
        self.brush_size
    }
    pub fn set_brush_size(&mut self, v: i32) {
        self.brush_size = v.clamp(1, 500);
        self.revision += 1;
    }
    pub fn wand_tolerance(&self) -> i32 {
        self.wand_tolerance
    }
    pub fn set_wand_tolerance(&mut self, v: i32) {
        self.wand_tolerance = v.clamp(0, 255);
        self.revision += 1;
    }
    pub fn recent_colors(&self) -> &[Bgra] {
        &self.recent_colors
    }
    /// Moves `c` to the front of the recent-colours list (max 12).
    pub fn remember_color(&mut self, c: Bgra) {
        self.recent_colors.retain(|&x| x != c);
        self.recent_colors.insert(0, c);
        self.recent_colors.truncate(12);
        self.revision += 1;
    }
    pub fn swap_colors(&mut self) {
        std::mem::swap(&mut self.foreground, &mut self.background);
        self.revision += 1;
    }
    pub fn reset_colors(&mut self) {
        self.foreground = Bgra::BLACK;
        self.background = Bgra::WHITE;
        self.revision += 1;
    }
    /// Primary colour for `button` (left → foreground).
    pub(crate) fn color_for(&self, button: PointerButton) -> Bgra {
        if button == PointerButton::Left { self.foreground } else { self.background }
    }
    /// The other colour for `button`.
    pub(crate) fn other_color_for(&self, button: PointerButton) -> Bgra {
        if button == PointerButton::Left { self.background } else { self.foreground }
    }
}

/// Everything a tool may touch while handling one event.
pub struct ToolContext<'a> {
    pub doc: &'a mut Document,
    /// The layer being edited.
    pub layer: LayerId,
    pub settings: &'a mut ToolSettings,
    pub history: &'a mut HistoryStack,
    /// Pen pressure in [0, 1]; 1 for mice.
    pub pressure: f32,
    /// Handle hit radius in image pixels (screen radius ÷ zoom).
    pub handle_radius: f64,
}

impl ToolContext<'_> {
    pub(crate) fn layer_ref(&self) -> &Layer {
        self.doc.layer(self.layer).expect("active layer missing")
    }

    /// Active layer plus the selection that clips it (any non-null selection).
    pub(crate) fn layer_sel(&mut self) -> (&mut Layer, Option<&Selection>) {
        self.doc.layer_and_selection(self.layer).expect("active layer missing")
    }

    pub(crate) fn invalidate(&mut self, r: PixelRect) {
        self.doc.invalidate(Some(r));
    }
}

/// Identifies a built-in tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolKind {
    RectangleSelect,
    EllipseSelect,
    LassoSelect,
    MagicWand,
    MoveSelection,
    MoveSelected,
    Pencil,
    Brush,
    Eraser,
    Fill,
    ColorPicker,
    Line,
    Rectangle,
    RoundedRectangle,
    Ellipse,
    FreeformShape,
    /// Any shape type (chosen in the toolbar).
    Shape,
    Gradient,
    CloneStamp,
    Recolor,
    /// Pinch / bulge / push / twirl brush.
    Distort,
    Perspective,
}

pub trait Tool: Send {
    fn name(&self) -> &'static str;
    fn kind(&self) -> ToolKind;
    fn pointer_down(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers);
    fn pointer_move(&mut self, ctx: &mut ToolContext, x: i32, y: i32, mods: Modifiers);
    fn pointer_up(&mut self, ctx: &mut ToolContext, x: i32, y: i32, button: PointerButton, mods: Modifiers);
    /// Rectangle to draw transform handles around, for handle tools.
    fn handle_bounds(&self, _doc: &Document) -> Option<RectF> {
        None
    }
    /// Whether a drag/stroke is in progress.
    fn is_busy(&self) -> bool {
        false
    }
    /// Whether the tool holds an editable result that isn't committed yet.
    fn has_pending(&self) -> bool {
        false
    }
    /// Whether undo should step through the pending result's own edits
    /// (rather than the document history).
    fn has_pending_edits(&self) -> bool {
        self.has_pending()
    }
    fn pending(&mut self, _ctx: &mut ToolContext, _action: PendingAction) {}
    fn edit_overlay(&self) -> Option<EditOverlay> {
        None
    }
    /// Called when the tool becomes the active one.
    fn activate(&mut self, _ctx: &mut ToolContext) {}
}

/// Holds the tool set, the active tool and shared settings (C# `ToolManager`).
pub struct ToolManager {
    tools: Vec<Box<dyn Tool>>,
    active: usize,
    pub settings: ToolSettings,
}

impl Default for ToolManager {
    fn default() -> Self {
        ToolManager::new()
    }
}

impl ToolManager {
    pub fn new() -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(select::RectangleSelectTool::default()),
            Box::new(select::EllipseSelectTool::default()),
            Box::new(select::LassoSelectTool::default()),
            Box::new(select::MagicWandTool),
            Box::new(select::MoveSelectionTool::default()),
            Box::new(move_tool::MoveTool::default()),
            Box::new(paint::PencilTool::default()),
            Box::new(paint::BrushTool::default()),
            Box::new(paint::EraserTool::default()),
            Box::new(paint::FillTool),
            Box::new(paint::ColorPickerTool),
            Box::new(shapes::LineTool::default()),
            Box::new(shapes::ShapeTool::new(shapes::ShapeKind::Rectangle)),
            Box::new(shapes::ShapeTool::new(shapes::ShapeKind::RoundedRectangle)),
            Box::new(shapes::ShapeTool::new(shapes::ShapeKind::Ellipse)),
            Box::new(shapes::ShapeTool::new(shapes::ShapeKind::Any)),
            Box::new(shapes::FreeformShapeTool::default()),
            Box::new(gradient::GradientTool::default()),
            Box::new(paint::CloneStampTool::default()),
            Box::new(paint::RecolorTool::default()),
            Box::new(warp::DistortTool::default()),
            Box::new(warp::PerspectiveTool::default()),
        ];
        ToolManager { tools, active: 0, settings: ToolSettings::default() }
    }

    /// Adds a custom tool (plugin).
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn tools(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.iter().map(|t| t.as_ref())
    }

    pub fn active(&self) -> &dyn Tool {
        self.tools[self.active].as_ref()
    }

    pub fn active_kind(&self) -> ToolKind {
        self.tools[self.active].kind()
    }

    /// Selects a tool; ignored while the current tool is mid-drag.
    pub fn set_active(&mut self, kind: ToolKind) -> bool {
        if self.tools[self.active].is_busy() {
            return false;
        }
        match self.tools.iter().position(|t| t.kind() == kind) {
            Some(i) => {
                self.active = i;
                self.settings.revision += 1;
                true
            }
            None => false,
        }
    }

    pub fn set_active_by_name(&mut self, name: &str) -> bool {
        match self.tools.iter().find(|t| t.name().eq_ignore_ascii_case(name)).map(|t| t.kind()) {
            Some(k) => self.set_active(k),
            None => false,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer_down(
        &mut self,
        doc: &mut Document,
        history: &mut HistoryStack,
        layer: LayerId,
        x: i32,
        y: i32,
        button: PointerButton,
        mods: Modifiers,
        pressure: f32,
        handle_radius: f64,
    ) {
        let mut ctx = ToolContext { doc, layer, settings: &mut self.settings, history, pressure, handle_radius };
        self.tools[self.active].pointer_down(&mut ctx, x, y, button, mods);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer_move(
        &mut self,
        doc: &mut Document,
        history: &mut HistoryStack,
        layer: LayerId,
        x: i32,
        y: i32,
        mods: Modifiers,
        pressure: f32,
        handle_radius: f64,
    ) {
        let mut ctx = ToolContext { doc, layer, settings: &mut self.settings, history, pressure, handle_radius };
        self.tools[self.active].pointer_move(&mut ctx, x, y, mods);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pointer_up(
        &mut self,
        doc: &mut Document,
        history: &mut HistoryStack,
        layer: LayerId,
        x: i32,
        y: i32,
        button: PointerButton,
        mods: Modifiers,
        pressure: f32,
        handle_radius: f64,
    ) {
        let mut ctx = ToolContext { doc, layer, settings: &mut self.settings, history, pressure, handle_radius };
        self.tools[self.active].pointer_up(&mut ctx, x, y, button, mods);
    }

    pub fn handle_bounds(&self, doc: &Document) -> Option<RectF> {
        self.tools[self.active].handle_bounds(doc)
    }

    pub fn has_pending(&self) -> bool {
        self.tools[self.active].has_pending()
    }

    pub fn edit_overlay(&self) -> Option<EditOverlay> {
        self.tools[self.active].edit_overlay()
    }

    pub fn has_pending_edits(&self) -> bool {
        self.tools[self.active].has_pending_edits()
    }

    /// Lets the active tool set itself up (e.g. show handles straight away).
    pub fn activate(&mut self, doc: &mut Document, history: &mut HistoryStack, layer: LayerId) {
        let mut ctx = ToolContext { doc, layer, settings: &mut self.settings, history, pressure: 1.0, handle_radius: 4.0 };
        self.tools[self.active].activate(&mut ctx);
    }

    pub fn pending(&mut self, doc: &mut Document, history: &mut HistoryStack, layer: LayerId, action: PendingAction) {
        if !self.tools[self.active].has_pending() {
            return;
        }
        let mut ctx = ToolContext { doc, layer, settings: &mut self.settings, history, pressure: 1.0, handle_radius: 4.0 };
        self.tools[self.active].pending(&mut ctx, action);
    }
}

/// Inclusive-bounds accumulator used by stroke tools.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Extent {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl Extent {
    pub const EMPTY: Extent = Extent { min_x: i32::MAX, min_y: i32::MAX, max_x: i32::MIN, max_y: i32::MIN };

    pub fn point(x: i32, y: i32) -> Extent {
        Extent { min_x: x, min_y: y, max_x: x, max_y: y }
    }

    pub fn of(t: (i32, i32, i32, i32)) -> Extent {
        Extent { min_x: t.0, min_y: t.1, max_x: t.2, max_y: t.3 }
    }

    pub fn add(&mut self, o: Extent) {
        self.min_x = self.min_x.min(o.min_x);
        self.min_y = self.min_y.min(o.min_y);
        self.max_x = self.max_x.max(o.max_x);
        self.max_y = self.max_y.max(o.max_y);
    }

    pub fn clamp(&self, w: i32, h: i32) -> PixelRect {
        PixelRect::clamp_box(self.min_x, self.min_y, self.max_x, self.max_y, w, h)
    }
}
