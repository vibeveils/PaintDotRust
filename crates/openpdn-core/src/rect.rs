//! Integer and floating rectangles.

/// Integer pixel rectangle (C# `PixelRect`). Empty when width or height ≤ 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PixelRect {
    pub const EMPTY: PixelRect = PixelRect { x: 0, y: 0, width: 0, height: 0 };

    #[inline(always)]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        PixelRect { x, y, width, height }
    }

    #[inline(always)]
    pub const fn right(&self) -> i32 {
        self.x + self.width
    }

    #[inline(always)]
    pub const fn bottom(&self) -> i32 {
        self.y + self.height
    }

    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    #[inline]
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub fn intersect(&self, o: &PixelRect) -> PixelRect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = self.right().min(o.right());
        let y1 = self.bottom().min(o.bottom());
        if x1 <= x0 || y1 <= y0 { PixelRect::EMPTY } else { PixelRect::new(x0, y0, x1 - x0, y1 - y0) }
    }

    /// Bounding union; an empty operand is ignored.
    pub fn union(&self, o: &PixelRect) -> PixelRect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = self.right().max(o.right());
        let y1 = self.bottom().max(o.bottom());
        PixelRect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Rect from inclusive corners, empty when inverted.
    pub fn from_inclusive(min_x: i32, min_y: i32, max_x: i32, max_y: i32) -> PixelRect {
        if max_x < min_x || max_y < min_y {
            PixelRect::EMPTY
        } else {
            PixelRect::new(min_x, min_y, max_x - min_x + 1, max_y - min_y + 1)
        }
    }

    /// Inclusive corners clamped to a `w`×`h` image (C# tools' `ClampBox`).
    pub fn clamp_box(min_x: i32, min_y: i32, max_x: i32, max_y: i32, w: i32, h: i32) -> PixelRect {
        PixelRect::from_inclusive(min_x.max(0), min_y.max(0), max_x.min(w - 1), max_y.min(h - 1))
    }
}

/// Floating-point rectangle (C# `RectF`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl RectF {
    #[inline(always)]
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        RectF { x, y, w, h }
    }

    #[inline(always)]
    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    #[inline(always)]
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn from_pixel(r: PixelRect) -> RectF {
        RectF::new(r.x as f64, r.y as f64, r.width as f64, r.height as f64)
    }

    pub fn to_covering_pixel_rect(&self) -> PixelRect {
        let x0 = self.x.floor() as i32;
        let y0 = self.y.floor() as i32;
        let x1 = self.right().ceil() as i32;
        let y1 = self.bottom().ceil() as i32;
        PixelRect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }
}
