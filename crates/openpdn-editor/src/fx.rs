//! Adjustment and effect catalogue: dialog schema (mirroring the C# dialogs'
//! sliders, ranges and defaults) plus a builder from dialog values.

use openpdn_core::adjust::{self, PixelOp};
use openpdn_core::effects::{artistic::*, blur::*, distort::*, print::*, render::*, stylize::*, Effect};
use openpdn_core::{BlendMode, Bgra};
use openpdn_formats::json::Json;

pub struct Slider {
    pub label: &'static str,
    pub min: i32,
    pub max: i32,
    pub default: i32,
    /// Display divisor (value / scale shown to the user).
    pub scale: i32,
}

const fn s(label: &'static str, min: i32, max: i32, default: i32) -> Slider {
    Slider { label, min, max, default, scale: 1 }
}

const fn sc(label: &'static str, min: i32, max: i32, default: i32, scale: i32) -> Slider {
    Slider { label, min, max, default, scale }
}

pub struct Choice {
    pub label: &'static str,
    pub options: &'static [&'static str],
    pub default: usize,
}

pub enum Kind {
    Adjustment,
    Effect,
}

pub struct FxDef {
    pub id: &'static str,
    pub title: &'static str,
    /// Menu: "Adjustments", or the Effects submenu.
    pub menu: &'static str,
    pub kind: Kind,
    /// No dialog: applied immediately.
    pub direct: bool,
    pub sliders: &'static [Slider],
    pub checks: &'static [(&'static str, bool)],
    pub choices: &'static [Choice],
    /// Posterize: sliders move together unless unlinked.
    pub linked: bool,
    pub shortcut: &'static str,
}

/// Values from the dialog.
#[derive(Default, Clone, Debug)]
pub struct Values {
    pub v: Vec<i32>,
    pub c: Vec<bool>,
    pub ch: Vec<usize>,
    /// Curves: mode (0 luminosity, 1 RGB) and control points per curve.
    pub curves: Option<(u8, Vec<Vec<(i32, i32)>>)>,
}

impl Values {
    fn i(&self, n: usize) -> i32 {
        self.v.get(n).copied().unwrap_or(0)
    }
    fn b(&self, n: usize) -> bool {
        self.c.get(n).copied().unwrap_or(false)
    }
    fn k(&self, n: usize) -> usize {
        self.ch.get(n).copied().unwrap_or(0)
    }
}

macro_rules! fx {
    ($id:expr, $title:expr, $menu:expr, $kind:ident, [$($sl:expr),*] $(, checks: [$($c:expr),*])? $(, choices: [$($ch:expr),*])? $(, direct: $d:expr)? $(, linked: $l:expr)? $(, key: $key:literal)?) => {
        FxDef {
            id: $id,
            title: $title,
            menu: $menu,
            kind: Kind::$kind,
            direct: false $(|| $d)?,
            sliders: &[$($sl),*],
            checks: &[$($($c),*)?],
            choices: &[$($($ch),*)?],
            linked: false $(|| $l)?,
            shortcut: concat!("" $(, $key)?),
        }
    };
}

pub const WAVE_TYPES: &[&str] = &["Ripples", "Spiral", "Horizontal", "Vertical", "Grid", "Zigzag", "Organic"];
pub const DITHER_NAMES: &[&str] = &["Bayer 2×2", "Bayer 4×4", "Bayer 8×8", "Bayer 16×16", "Spiral", "Dual Spiral", "Blue Noise", "White Noise"];

pub static CATALOG: &[FxDef] = &[
    // ---- Adjustments (menu order)
    fx!("auto-level", "Auto-Level", "Adjustments", Adjustment, [], direct: true, key: "Ctrl+Shift+L"),
    fx!("black-and-white", "Black and White", "Adjustments", Adjustment, [], direct: true, key: "Ctrl+Shift+G"),
    fx!("brightness-contrast", "Brightness / Contrast", "Adjustments", Adjustment, [s("Brightness", -100, 100, 0), s("Contrast", -100, 100, 0)], key: "Ctrl+Shift+C"),
    fx!("curves", "Curves", "Adjustments", Adjustment, [], key: "Ctrl+Shift+M"),
    fx!("hue-saturation", "Hue / Saturation", "Adjustments", Adjustment, [s("Hue", -180, 180, 0), s("Saturation", 0, 200, 100), s("Lightness", -100, 100, 0)], key: "Ctrl+Shift+U"),
    fx!("invert-colors", "Invert Colors", "Adjustments", Adjustment, [], direct: true, key: "Ctrl+Shift+I"),
    fx!("levels", "Levels", "Adjustments", Adjustment, [
            s("Input low", 0, 254, 0), s("Input high", 1, 255, 255), sc("Gamma", 10, 1000, 100, 100),
            s("Output low", 0, 254, 0), s("Output high", 1, 255, 255)
        ], key: "Ctrl+L"),
    fx!("posterize", "Posterize", "Adjustments", Adjustment, [s("Red", 2, 64, 16), s("Green", 2, 64, 16), s("Blue", 2, 64, 16)], linked: true, key: "Ctrl+Shift+P"),
    fx!("sepia", "Sepia", "Adjustments", Adjustment, [], direct: true, key: "Ctrl+Shift+E"),
    // ---- Effects › Artistic
    fx!("ink-sketch", "Ink Sketch", "Artistic", Effect, [s("Ink outline", 0, 99, 50), s("Coloring", 0, 100, 50)]),
    fx!("linocut", "Linocut", "Artistic", Effect, [s("Threshold", 1, 99, 50), s("Detail", 1, 20, 3), s("Edge strength", 0, 100, 50), s("Line spacing", 2, 30, 6)],
        checks: [("Use primary / secondary colors", false)]),
    fx!("oil-painting", "Oil Painting", "Artistic", Effect, [s("Brush size", 1, 8, 3), s("Coarseness", 3, 255, 50)]),
    fx!("pencil-sketch", "Pencil Sketch", "Artistic", Effect, [s("Pencil tip size", 1, 20, 2), s("Color range", -20, 20, 0)]),
    fx!("pointillism", "Pointillism", "Artistic", Effect, [s("Dot size", 2, 60, 8), s("Density (%)", 20, 300, 120), s("Color variation", 0, 100, 25), s("Seed", 0, 999, 0)]),
    fx!("stained-glass", "Stained Glass", "Artistic", Effect, [s("Cell size", 5, 300, 40), s("Leading width", 0, 20, 3), s("Randomness", 0, 100, 80), s("Seed", 0, 999, 0)],
        checks: [("Glass shading", true)]),
    // ---- Blurs
    fx!("fragment", "Fragment", "Blurs", Effect, [s("Fragments", 2, 50, 4), s("Distance", 0, 100, 8), s("Rotation", -180, 180, 0)]),
    fx!("gaussian-blur", "Gaussian Blur", "Blurs", Effect, [s("Radius", 0, 200, 2)]),
    fx!("motion-blur", "Motion Blur", "Blurs", Effect, [s("Angle", -180, 180, 25), s("Distance", 1, 200, 10)], checks: [("Centered", true)]),
    fx!("radial-blur", "Radial Blur", "Blurs", Effect, [s("Angle", 0, 360, 2), s("Quality", 1, 5, 2)]),
    fx!("unfocus", "Unfocus", "Blurs", Effect, [s("Radius", 1, 200, 4)]),
    // ---- Distort
    fx!("bulge", "Bulge", "Distort", Effect, [s("Amount", -200, 100, 45)]),
    fx!("frosted-glass", "Frosted Glass", "Distort", Effect, [s("Minimum scatter radius", 0, 200, 3), s("Maximum scatter radius", 0, 200, 3), s("Smoothness", 1, 8, 1)]),
    fx!("mosaic", "Mosaic", "Distort", Effect, [s("Tile size", 2, 200, 16), s("Grout width", 0, 20, 2), s("Bevel width", 0, 30, 3), s("Bevel strength", 0, 100, 50)]),
    fx!("pixelate", "Pixelate", "Distort", Effect, [s("Cell size", 1, 100, 2)]),
    fx!("polar-inversion", "Polar Inversion", "Distort", Effect, [sc("Amount (×100)", -400, 400, 100, 100), s("Quality", 1, 5, 2)]),
    fx!("spherize", "Spherize", "Distort", Effect, [s("Amount", -100, 100, 60), s("Radius (%)", 1, 200, 100), s("Offset X", -100, 100, 0), s("Offset Y", -100, 100, 0), s("Quality", 1, 5, 2)]),
    fx!("tile-reflection", "Tile Reflection", "Distort", Effect, [s("Rotation", -45, 45, 30), s("Tile size", 2, 200, 40), s("Curvature", -100, 100, 8), s("Quality", 1, 5, 2)]),
    fx!("twist", "Twist", "Distort", Effect, [s("Amount", -200, 200, 30), sc("Size (×100)", 1, 200, 100, 100), s("Quality", 1, 5, 2)]),
    fx!("waves", "Waves", "Distort", Effect, [s("Wavelength", 2, 500, 40), s("Amplitude", 0, 200, 8), s("Phase", 0, 360, 0), s("Center X", -100, 100, 0), s("Center Y", -100, 100, 0), s("Quality", 1, 5, 2)],
        choices: [Choice { label: "Waveform", options: WAVE_TYPES, default: 0 }]),
    // ---- Render
    fx!("clouds", "Clouds", "Render", Effect, [s("Scale", 2, 1000, 250), sc("Roughness (×100)", 0, 100, 60, 100), s("Seed", 0, 255, 0)]),
    fx!("julia-fractal", "Julia Fractal", "Render", Effect, [s("Factor", 1, 10, 4), sc("Zoom (×100)", 10, 5000, 100, 100), s("Angle", -180, 180, 0), s("Quality", 1, 5, 2)]),
    fx!("mandelbrot-fractal", "Mandelbrot Fractal", "Render", Effect, [s("Factor", 1, 10, 1), sc("Zoom (×100)", 0, 10000, 1000, 100), s("Angle", -180, 180, 0), s("Quality", 1, 5, 2)],
        checks: [("Invert colors", false)]),
    // ---- Noise
    fx!("add-noise", "Add Noise", "Noise", Effect, [s("Intensity", 0, 100, 64), s("Color saturation", 0, 400, 100), s("Coverage", 0, 100, 100)]),
    fx!("median", "Median", "Noise", Effect, [s("Radius", 1, 200, 10), s("Percentile", 0, 100, 50)]),
    fx!("reduce-noise", "Reduce Noise", "Noise", Effect, [s("Radius", 1, 200, 10), sc("Strength (×100)", 0, 100, 40, 100)]),
    // ---- Photo
    fx!("glow", "Glow", "Photo", Effect, [s("Radius", 1, 20, 6), s("Brightness", -100, 100, 10), s("Contrast", -100, 100, 10)]),
    fx!("red-eye", "Red Eye Removal", "Photo", Adjustment, [s("Tolerance", 0, 100, 70), s("Saturation percentage", 0, 100, 90)]),
    fx!("soften-portrait", "Soften Portrait", "Photo", Effect, [s("Softness", 0, 10, 5), s("Lighting", -20, 20, 0), s("Warmth", 0, 20, 10)]),
    fx!("sharpen", "Sharpen", "Photo", Effect, [s("Amount", 1, 20, 2)]),
    // ---- Stylize
    fx!("edge-detect", "Edge Detect", "Stylize", Effect, [s("Angle", -180, 180, 45)]),
    fx!("emboss", "Emboss", "Stylize", Effect, [s("Angle", -180, 180, 0)]),
    fx!("halftone", "Halftone", "Stylize", Effect, [s("Dot size", 2, 100, 8), s("Angle (mono)", 0, 90, 45)],
        choices: [Choice { label: "Mode", options: &["CMYK", "Monochrome (primary on secondary)"], default: 0 }]),
    fx!("ordered-dither", "Ordered Dither", "Stylize", Effect, [s("Levels", 2, 16, 2), s("Seed", 0, 999, 0)],
        checks: [("Grayscale", false)], choices: [Choice { label: "Pattern", options: DITHER_NAMES, default: 2 }]),
    fx!("relief", "Relief", "Stylize", Effect, [s("Angle", -180, 180, 45)]),
    // ---- Layer › Rotate / Zoom uses the same preview machinery.
    fx!("rotate-zoom", "Rotate / Zoom", "Layer", Effect, [s("Angle", -180, 180, 0), sc("Zoom (×100)", 10, 500, 100, 100)], checks: [("Tile", false)], key: "Ctrl+Shift+Z"),
];

pub fn find(id: &str) -> Option<&'static FxDef> {
    CATALOG.iter().find(|f| f.id == id)
}

/// Defaults for a definition.
pub fn defaults(d: &FxDef) -> Values {
    Values {
        v: d.sliders.iter().map(|s| s.default).collect(),
        c: d.checks.iter().map(|c| c.1).collect(),
        ch: d.choices.iter().map(|c| c.default).collect(),
        curves: (d.id == "curves").then(|| (0, vec![vec![(0, 0), (255, 255)]; 3])),
    }
}

/// Catalogue as JSON for the UI.
pub fn schema() -> Json {
    let s = |x: &str| Json::Str(x.to_owned());
    let n = |x: i32| Json::Num(x as f64);
    Json::Arr(
        CATALOG
            .iter()
            .map(|d| {
                Json::Obj(vec![
                    ("id".into(), s(d.id)),
                    ("title".into(), s(d.title)),
                    ("menu".into(), s(d.menu)),
                    ("kind".into(), s(match d.kind { Kind::Adjustment => "adjustment", Kind::Effect => "effect" })),
                    ("direct".into(), Json::Bool(d.direct)),
                    ("linked".into(), Json::Bool(d.linked)),
                    ("shortcut".into(), s(d.shortcut)),
                    (
                        "sliders".into(),
                        Json::Arr(
                            d.sliders
                                .iter()
                                .map(|x| {
                                    Json::Obj(vec![
                                        ("label".into(), s(x.label)),
                                        ("min".into(), n(x.min)),
                                        ("max".into(), n(x.max)),
                                        ("default".into(), n(x.default)),
                                        ("scale".into(), n(x.scale)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                    (
                        "checks".into(),
                        Json::Arr(d.checks.iter().map(|c| Json::Obj(vec![("label".into(), s(c.0)), ("default".into(), Json::Bool(c.1))])).collect()),
                    ),
                    (
                        "choices".into(),
                        Json::Arr(
                            d.choices
                                .iter()
                                .map(|c| {
                                    Json::Obj(vec![
                                        ("label".into(), s(c.label)),
                                        ("options".into(), Json::Arr(c.options.iter().map(|o| s(o)).collect())),
                                        ("default".into(), n(c.default as i32)),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

/// What a definition builds.
pub enum Built {
    Op(Box<dyn PixelOp>),
    Effect(Box<dyn Effect>),
    /// Rotate / Zoom works on a copy of the original layer, not through `Effect::apply`.
    RotateZoom { angle: f64, zoom: f64, tile: bool },
}

/// Builds the operation (`fg`/`bg` are the palette colours, as C# reads them).
pub fn build(id: &str, v: &Values, fg: Bgra, bg: Bgra, layer: &openpdn_core::Surface, sel: Option<&openpdn_core::Selection>) -> Option<Built> {
    let op = |o: Box<dyn PixelOp>| Some(Built::Op(o));
    let fx = |e: Box<dyn Effect>| Some(Built::Effect(e));
    match id {
        "auto-level" => op(Box::new(adjust::HistogramRgb::new(layer, sel).auto_levels())),
        "black-and-white" => op(Box::new(adjust::Desaturate)),
        "invert-colors" => op(Box::new(adjust::InvertColors)),
        "sepia" => op(Box::new(adjust::Sepia::default())),
        "brightness-contrast" => op(Box::new(adjust::BrightnessContrast::new(v.i(0), v.i(1)))),
        "hue-saturation" => op(Box::new(adjust::HueSaturation::new(v.i(0), v.i(1), v.i(2)))),
        "posterize" => op(Box::new(adjust::posterize(v.i(0), v.i(1), v.i(2)))),
        "levels" => {
            let in_lo = v.i(0).min(v.i(1) - 1);
            let in_hi = v.i(1).max(v.i(0) + 1);
            let out_lo = v.i(3).min(v.i(4) - 1);
            let out_hi = v.i(4).max(v.i(3) + 1);
            let g = v.i(2) as f32 / 100.0;
            let grey = |x: i32| {
                let b = x as u8;
                Bgra::bgr(b, b, b)
            };
            op(Box::new(adjust::levels(grey(in_lo), grey(in_hi), [g, g, g], grey(out_lo), grey(out_hi))))
        }
        "curves" => {
            let (mode, curves) = v.curves.clone().unwrap_or((0, vec![vec![(0, 0), (255, 255)]; 3]));
            let lut = |i: usize| {
                let pts = curves.get(i).filter(|p| p.len() >= 2).cloned().unwrap_or_else(|| vec![(0, 0), (255, 255)]);
                adjust::Spline::build_lut(&pts)
            };
            if mode == 0 {
                op(Box::new(adjust::LuminosityCurve(lut(0))))
            } else {
                op(Box::new(adjust::channel_curve(lut(0), lut(1), lut(2))))
            }
        }
        "red-eye" => op(Box::new(adjust::RedEyeRemove::new(v.i(0), v.i(1)))),
        "gaussian-blur" => fx(Box::new(GaussianBlur(v.i(0)))),
        "motion-blur" => fx(Box::new(MotionBlur::new(v.i(0) as f64, v.i(1), v.b(0)))),
        "sharpen" => fx(Box::new(Sharpen(v.i(0)))),
        "median" => fx(Box::new(Median { radius: v.i(0), percentile: v.i(1) })),
        "add-noise" => fx(Box::new(AddNoise { intensity: v.i(0), saturation: v.i(1), coverage_percent: v.i(2) as f64, seed: 0 })),
        "pixelate" => fx(Box::new(Pixelate(v.i(0)))),
        "emboss" => fx(Box::new(Emboss(v.i(0) as f64))),
        "edge-detect" => fx(Box::new(ColorDifference::edge_detect(v.i(0) as f64))),
        "relief" => fx(Box::new(ColorDifference::relief(v.i(0) as f64))),
        "oil-painting" => fx(Box::new(OilPainting { brush_size: v.i(0), coarseness: v.i(1) })),
        "unfocus" => fx(Box::new(Unfocus(v.i(0)))),
        "fragment" => fx(Box::new(Fragment::new(v.i(0), v.i(1), v.i(2) as f64))),
        "radial-blur" => fx(Box::new(RadialBlur { angle_degrees: v.i(0) as f64, offset_x: 0.0, offset_y: 0.0, quality: v.i(1) })),
        "twist" => fx(Box::new(Twist::new(v.i(0) as f64, v.i(1) as f64 / 100.0, 0.0, 0.0, v.i(2)))),
        "bulge" => fx(Box::new(Bulge { amount: v.i(0) as f64, offset_x: 0.0, offset_y: 0.0 })),
        "polar-inversion" => fx(Box::new(PolarInversion { amount: v.i(0) as f64 / 100.0, offset_x: 0.0, offset_y: 0.0, edge: WarpEdge::Wrap, quality: v.i(1) })),
        "frosted-glass" => fx(Box::new(FrostedGlass {
            min_radius: v.i(0).min(v.i(1)) as f64,
            max_radius: v.i(0).max(v.i(1)) as f64,
            sample_count: v.i(2),
            seed: 0,
        })),
        "tile-reflection" => fx(Box::new(TileReflection::new(v.i(0) as f64, v.i(1) as f64, v.i(2) as f64, v.i(3)))),
        "clouds" => fx(Box::new(Clouds { scale: v.i(0), power: v.i(1) as f64 / 100.0, seed: v.i(2), from: fg, to: bg, blend_mode: BlendMode::Normal })),
        "julia-fractal" => fx(Box::new(JuliaFractal { factor: v.i(0) as f64, zoom: v.i(1) as f64 / 100.0, angle_degrees: v.i(2) as f64, quality: v.i(3) })),
        "mandelbrot-fractal" => fx(Box::new(MandelbrotFractal {
            factor: v.i(0),
            zoom: v.i(1) as f64 / 100.0,
            angle_degrees: v.i(2) as f64,
            quality: v.i(3),
            invert_colors: v.b(0),
        })),
        "ink-sketch" => fx(Box::new(InkSketch { ink_outline: v.i(0), coloring: v.i(1) })),
        "pencil-sketch" => fx(Box::new(PencilSketch { tip_size: v.i(0), color_range: v.i(1) })),
        "soften-portrait" => fx(Box::new(SoftenPortrait { softness: v.i(0), lighting: v.i(1), warmth: v.i(2) })),
        "reduce-noise" => fx(Box::new(ReduceNoise::new(v.i(0), v.i(1) as f64 / 100.0))),
        "glow" => fx(Box::new(Glow { radius: v.i(0), brightness: v.i(1), contrast: v.i(2) })),
        "linocut" => fx(Box::new(Linocut {
            ink: if v.b(0) { fg } else { Bgra::BLACK },
            paper: if v.b(0) { bg } else { Bgra::WHITE },
            threshold_percent: v.i(0),
            detail_radius: v.i(1),
            edge_percent: v.i(2),
            line_spacing: v.i(3),
        })),
        "pointillism" => fx(Box::new(Pointillism { dot_size: v.i(0), density_percent: v.i(1), color_jitter_percent: v.i(2), seed: v.i(3), paper: bg })),
        "stained-glass" => fx(Box::new(StainedGlass {
            cell_size: v.i(0),
            edge_width: v.i(1),
            randomness_percent: v.i(2),
            seed: v.i(3),
            shading: v.b(0),
            leading: fg,
        })),
        "mosaic" => fx(Box::new(Mosaic { tile_size: v.i(0), grout_width: v.i(1), bevel_width: v.i(2), bevel_strength_percent: v.i(3), grout: bg })),
        "spherize" => fx(Box::new(Spherize {
            amount_percent: v.i(0),
            radius_percent: v.i(1),
            offset_x_percent: v.i(2),
            offset_y_percent: v.i(3),
            quality: v.i(4),
        })),
        "waves" => {
            let kinds = [WaveType::Ripples, WaveType::Spiral, WaveType::Horizontal, WaveType::Vertical, WaveType::Grid, WaveType::Zigzag, WaveType::Organic];
            fx(Box::new(Waves {
                kind: kinds[v.k(0).min(6)],
                wavelength: v.i(0),
                amplitude: v.i(1),
                phase_degrees: v.i(2),
                offset_x_percent: v.i(3),
                offset_y_percent: v.i(4),
                quality: v.i(5),
            }))
        }
        "halftone" => fx(Box::new(Halftone {
            mode: if v.k(0) == 0 { HalftoneMode::Cmyk } else { HalftoneMode::Monochrome },
            cell_size: v.i(0),
            angle_degrees: v.i(1),
            ink: fg,
            paper: bg,
        })),
        "ordered-dither" => {
            let p = [
                DitherPattern::Bayer2,
                DitherPattern::Bayer4,
                DitherPattern::Bayer8,
                DitherPattern::Bayer16,
                DitherPattern::Spiral,
                DitherPattern::DualSpiral,
                DitherPattern::BlueNoise,
                DitherPattern::WhiteNoise,
            ];
            fx(Box::new(OrderedDither::new(p[v.k(0).min(7)], v.i(0), v.b(0), v.i(1))))
        }
        "rotate-zoom" => Some(Built::RotateZoom { angle: v.i(0) as f64, zoom: v.i(1) as f64 / 100.0, tile: v.b(0) }),
        _ => None,
    }
}
