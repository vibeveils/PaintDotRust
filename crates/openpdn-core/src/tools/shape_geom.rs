//! Shape types for the Shape tool and their geometry.
//!
//! Every shape is described in its own unit box, x and y in [-0.5, 0.5]
//! (scaled to the drawn width and height, then rotated). Rectangles, rounded
//! rectangles, ellipses and terminators are drawn by the box renderers;
//! everything else is one or more closed polygons, plus optional open
//! "detail" lines (e.g. the inner bars of a predefined process).

use std::f32::consts::{FRAC_PI_2, PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShapeType {
    #[default]
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Diamond,
    Triangle,
    RightTriangle,
    Trapezoid,
    Parallelogram,
    Pentagon,
    Hexagon,
    Octagon,
    /// Regular polygon with `sides` sides.
    Polygon,
    /// Star with `sides` points.
    Star,
    /// Starts as a regular polygon; its corners can be dragged anywhere.
    CustomPolygon,
    Plus,
    Cross,
    Arrow,
    DoubleArrow,
    Chevron,
    Heart,
    Callout,
    Moon,
    Lightning,
    // Flowchart
    Terminator,
    Document,
    PredefinedProcess,
    ManualInput,
    ManualOperation,
    Preparation,
    Delay,
    OffPageConnector,
    Database,
    Merge,
}

impl ShapeType {
    pub const ALL: [ShapeType; 33] = [
        ShapeType::Rectangle,
        ShapeType::RoundedRectangle,
        ShapeType::Ellipse,
        ShapeType::Diamond,
        ShapeType::Triangle,
        ShapeType::RightTriangle,
        ShapeType::Trapezoid,
        ShapeType::Parallelogram,
        ShapeType::Pentagon,
        ShapeType::Hexagon,
        ShapeType::Octagon,
        ShapeType::Polygon,
        ShapeType::Star,
        ShapeType::CustomPolygon,
        ShapeType::Plus,
        ShapeType::Cross,
        ShapeType::Arrow,
        ShapeType::DoubleArrow,
        ShapeType::Chevron,
        ShapeType::Heart,
        ShapeType::Callout,
        ShapeType::Moon,
        ShapeType::Lightning,
        ShapeType::Terminator,
        ShapeType::Document,
        ShapeType::PredefinedProcess,
        ShapeType::ManualInput,
        ShapeType::ManualOperation,
        ShapeType::Preparation,
        ShapeType::Delay,
        ShapeType::OffPageConnector,
        ShapeType::Database,
        ShapeType::Merge,
    ];

    pub fn name(self) -> String {
        format!("{self:?}")
    }

    pub fn parse(s: &str) -> Option<ShapeType> {
        Self::ALL.iter().copied().find(|t| t.name() == s)
    }
}

/// Unit-box geometry: closed contours (even-odd filled) and open detail lines.
#[derive(Clone, Debug, Default)]
pub struct Geom {
    pub contours: Vec<Vec<(f32, f32)>>,
    pub details: Vec<Vec<(f32, f32)>>,
}

/// Vertices of a regular polygon (or star when `inner` < 1) fitted to the
/// unit box, first point at the top.
pub fn regular(sides: usize, inner: Option<f32>) -> Vec<(f32, f32)> {
    let n = sides.max(3);
    let k = if inner.is_some() { 2 * n } else { n };
    let mut pts: Vec<(f32, f32)> = (0..k)
        .map(|i| {
            let a = -FRAC_PI_2 + TAU * i as f32 / k as f32;
            let r = if i % 2 == 1 { inner.unwrap_or(1.0) } else { 1.0 };
            (r * a.cos(), r * a.sin())
        })
        .collect();
    // Stretch to fill the box exactly.
    let (x0, x1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.0), b.max(p.0)));
    let (y0, y1) = pts.iter().fold((f32::MAX, f32::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    for p in &mut pts {
        p.0 = (p.0 - x0) / (x1 - x0).max(1e-6) - 0.5;
        p.1 = (p.1 - y0) / (y1 - y0).max(1e-6) - 0.5;
    }
    pts
}

fn arc(cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n).map(|i| {
        let a = a0 + (a1 - a0) * i as f32 / n as f32;
        (cx + rx * a.cos(), cy + ry * a.sin())
    })
    .collect()
}

/// Geometry for polygon-based shapes (not the box shapes). `custom` holds
/// the corners of a custom polygon.
pub fn geometry(t: ShapeType, sides: usize, inner: f32, custom: &[(f32, f32)]) -> Geom {
    let poly = |p: Vec<(f32, f32)>| Geom { contours: vec![p], details: vec![] };
    let (l, r, top, b) = (-0.5f32, 0.5f32, -0.5f32, 0.5f32);
    match t {
        ShapeType::Rectangle | ShapeType::RoundedRectangle | ShapeType::Ellipse | ShapeType::Terminator => {
            poly(vec![(l, top), (r, top), (r, b), (l, b)])
        }
        ShapeType::Diamond => poly(vec![(0.0, top), (r, 0.0), (0.0, b), (l, 0.0)]),
        ShapeType::Triangle => poly(vec![(0.0, top), (r, b), (l, b)]),
        ShapeType::Merge => poly(vec![(l, top), (r, top), (0.0, b)]),
        ShapeType::RightTriangle => poly(vec![(l, top), (r, b), (l, b)]),
        ShapeType::Trapezoid => poly(vec![(-0.3, top), (0.3, top), (r, b), (l, b)]),
        ShapeType::ManualOperation => poly(vec![(l, top), (r, top), (0.3, b), (-0.3, b)]),
        ShapeType::Parallelogram => poly(vec![(-0.25, top), (r, top), (0.25, b), (l, b)]),
        ShapeType::Pentagon => poly(regular(5, None)),
        ShapeType::Hexagon | ShapeType::Preparation => poly(vec![(-0.25, top), (0.25, top), (r, 0.0), (0.25, b), (-0.25, b), (l, 0.0)]),
        ShapeType::Octagon => {
            let k = 0.5 - 0.5 / (1.0 + std::f32::consts::SQRT_2);
            poly(vec![(-k, top), (k, top), (r, -k), (r, k), (k, b), (-k, b), (l, k), (l, -k)])
        }
        ShapeType::Polygon => poly(regular(sides.clamp(3, 64), None)),
        ShapeType::Star => poly(regular(sides.clamp(3, 64), Some(inner.clamp(0.05, 0.95)))),
        ShapeType::CustomPolygon => poly(if custom.len() >= 3 { custom.to_vec() } else { regular(sides.clamp(3, 64), None) }),
        ShapeType::Plus => {
            let k = 1.0 / 6.0;
            poly(vec![(-k, top), (k, top), (k, -k), (r, -k), (r, k), (k, k), (k, b), (-k, b), (-k, k), (l, k), (l, -k), (-k, -k)])
        }
        ShapeType::Cross => {
            let k = 0.14;
            poly(vec![(l, top + k), (l + k, top), (0.0, -k), (r - k, top), (r, top + k), (k, 0.0), (r, b - k), (r - k, b), (0.0, k), (l + k, b), (l, b - k), (-k, 0.0)])
        }
        ShapeType::Arrow => poly(vec![(l, -0.2), (0.1, -0.2), (0.1, top), (r, 0.0), (0.1, b), (0.1, 0.2), (l, 0.2)]),
        ShapeType::DoubleArrow => poly(vec![(l, 0.0), (-0.2, top), (-0.2, -0.2), (0.2, -0.2), (0.2, top), (r, 0.0), (0.2, b), (0.2, 0.2), (-0.2, 0.2), (-0.2, b)]),
        ShapeType::Chevron => poly(vec![(l, top), (0.15, top), (r, 0.0), (0.15, b), (l, b), (-0.15, 0.0)]),
        ShapeType::OffPageConnector => poly(vec![(l, top), (r, top), (r, 0.2), (0.0, b), (l, 0.2)]),
        ShapeType::ManualInput => poly(vec![(l, -0.2), (r, top), (r, b), (l, b)]),
        ShapeType::Callout => poly(vec![(l, top), (r, top), (r, 0.25), (-0.05, 0.25), (-0.3, b), (-0.25, 0.25), (l, 0.25)]),
        ShapeType::Lightning => poly(vec![(0.05, top), (-0.35, 0.05), (-0.02, 0.05), (-0.2, b), (0.35, -0.12), (0.03, -0.12), (0.25, top)]),
        ShapeType::Heart => {
            // Classic parametric heart, normalised to the box.
            let n = 96;
            let mut p: Vec<(f32, f32)> = (0..n)
                .map(|i| {
                    let t = TAU * i as f32 / n as f32;
                    let x = 16.0 * t.sin().powi(3);
                    let y = -(13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos());
                    (x, y)
                })
                .collect();
            normalise(&mut p);
            poly(p)
        }
        ShapeType::Moon => {
            // Outer half circle (bottom → left → top), back down a flatter inner curve.
            let mut p = arc(0.0, 0.0, 0.5, 0.5, FRAC_PI_2, FRAC_PI_2 + PI, 48);
            p.extend(arc(0.0, 0.0, 0.2, 0.5, FRAC_PI_2 + PI, FRAC_PI_2, 48).into_iter().skip(1));
            p.pop();
            normalise(&mut p);
            poly(p)
        }
        ShapeType::Document => {
            let mut p = vec![(l, top), (r, top)];
            p.extend((0..=32).map(|i| {
                let x = r - i as f32 / 32.0;
                (x, 0.38 + 0.08 * (TAU * (x + 0.5)).sin())
            }));
            poly(p)
        }
        ShapeType::PredefinedProcess => Geom {
            contours: vec![vec![(l, top), (r, top), (r, b), (l, b)]],
            details: vec![vec![(-0.4, top), (-0.4, b)], vec![(0.4, top), (0.4, b)]],
        },
        ShapeType::Delay => {
            let mut p = vec![(l, top), (0.0, top)];
            p.extend(arc(0.0, 0.0, 0.5, 0.5, -FRAC_PI_2, FRAC_PI_2, 40).into_iter().skip(1));
            p.push((l, b));
            poly(p)
        }
        ShapeType::Database => {
            let e = 0.12; // half-height of the end ellipses
            let mut p = arc(0.0, top + e, 0.5, e, PI, TAU, 40);
            p.extend(arc(0.0, b - e, 0.5, e, 0.0, PI, 40));
            Geom { contours: vec![p], details: vec![arc(0.0, top + e, 0.5, e, 0.0, PI, 40)] }
        }
    }
}

fn normalise(p: &mut [(f32, f32)]) {
    let (x0, x1) = p.iter().fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q.0), b.max(q.0)));
    let (y0, y1) = p.iter().fold((f32::MAX, f32::MIN), |(a, b), q| (a.min(q.1), b.max(q.1)));
    for q in p {
        q.0 = (q.0 - x0) / (x1 - x0).max(1e-6) - 0.5;
        q.1 = (q.1 - y0) / (y1 - y0).max(1e-6) - 0.5;
    }
}
