//! A 2-D buffer of [`Bgra`] pixels.

use crate::color::{self, Bgra};
use crate::rect::PixelRect;

/// Row-major BGRA pixel buffer (C# `Surface`).
#[derive(Clone, PartialEq, Eq)]
pub struct Surface {
    width: i32,
    height: i32,
    pixels: Vec<Bgra>,
}

impl std::fmt::Debug for Surface {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Surface({}x{})", self.width, self.height)
    }
}

impl Surface {
    /// New transparent surface. Panics on non-positive dimensions.
    pub fn new(width: i32, height: i32) -> Self {
        assert!(width > 0 && height > 0, "surface dimensions must be positive");
        let n = (width as usize).checked_mul(height as usize).expect("surface too large");
        Surface { width, height, pixels: vec![Bgra::TRANSPARENT; n] }
    }

    /// Wraps existing pixels; `pixels.len()` must equal `width * height`.
    pub fn from_pixels(width: i32, height: i32, pixels: Vec<Bgra>) -> Self {
        assert!(width > 0 && height > 0);
        assert_eq!(pixels.len(), width as usize * height as usize);
        Surface { width, height, pixels }
    }

    /// Builds a surface from tightly packed RGBA8 bytes (canvas/ImageData order).
    pub fn from_rgba8(width: i32, height: i32, rgba: &[u8]) -> Self {
        assert_eq!(rgba.len(), width as usize * height as usize * 4);
        let pixels = rgba.chunks_exact(4).map(|p| Bgra::new(p[2], p[1], p[0], p[3])).collect();
        Surface::from_pixels(width, height, pixels)
    }

    #[inline(always)]
    pub fn width(&self) -> i32 {
        self.width
    }

    #[inline(always)]
    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline(always)]
    pub fn bounds(&self) -> PixelRect {
        PixelRect::new(0, 0, self.width, self.height)
    }

    #[inline(always)]
    pub fn pixels(&self) -> &[Bgra] {
        &self.pixels
    }

    #[inline(always)]
    pub fn pixels_mut(&mut self) -> &mut [Bgra] {
        &mut self.pixels
    }

    pub fn into_pixels(self) -> Vec<Bgra> {
        self.pixels
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        color::as_bytes(&self.pixels)
    }

    #[inline(always)]
    pub fn as_bytes_mut(&mut self) -> &mut [u8] {
        color::as_bytes_mut(&mut self.pixels)
    }

    #[inline(always)]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        (x as u32) < self.width as u32 && (y as u32) < self.height as u32
    }

    #[inline(always)]
    fn index(&self, x: i32, y: i32) -> usize {
        assert!(self.in_bounds(x, y), "pixel ({x}, {y}) out of range");
        y as usize * self.width as usize + x as usize
    }

    #[inline(always)]
    pub fn get(&self, x: i32, y: i32) -> Bgra {
        self.pixels[self.index(x, y)]
    }

    #[inline(always)]
    pub fn set(&mut self, x: i32, y: i32, c: Bgra) {
        let i = self.index(x, y);
        self.pixels[i] = c;
    }

    #[inline(always)]
    pub fn get_mut(&mut self, x: i32, y: i32) -> &mut Bgra {
        let i = self.index(x, y);
        &mut self.pixels[i]
    }

    #[inline(always)]
    pub fn row(&self, y: i32) -> &[Bgra] {
        assert!((y as u32) < self.height as u32);
        let w = self.width as usize;
        &self.pixels[y as usize * w..(y as usize + 1) * w]
    }

    #[inline(always)]
    pub fn row_mut(&mut self, y: i32) -> &mut [Bgra] {
        assert!((y as u32) < self.height as u32);
        let w = self.width as usize;
        &mut self.pixels[y as usize * w..(y as usize + 1) * w]
    }

    pub fn clear(&mut self, c: Bgra) {
        self.pixels.fill(c);
    }

    pub fn copy_from(&mut self, other: &Surface) {
        assert!(self.width == other.width && self.height == other.height);
        self.pixels.copy_from_slice(&other.pixels);
    }

    /// Copies pixels of `rect` (clipped) from `src`, which must be the same size.
    pub fn copy_rect_from(&mut self, src: &[Bgra], rect: PixelRect) {
        let r = rect.intersect(&self.bounds());
        if r.is_empty() {
            return;
        }
        let w = self.width as usize;
        for y in r.y..r.bottom() {
            let o = y as usize * w + r.x as usize;
            let n = r.width as usize;
            self.pixels[o..o + n].copy_from_slice(&src[o..o + n]);
        }
    }

    /// Extracts `rect` (must be in bounds) into a packed buffer.
    pub fn read_rect(&self, rect: PixelRect) -> Vec<Bgra> {
        let w = self.width as usize;
        let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize);
        for y in rect.y..rect.bottom() {
            let o = y as usize * w + rect.x as usize;
            out.extend_from_slice(&self.pixels[o..o + rect.width as usize]);
        }
        out
    }

    /// Writes a packed buffer produced by [`Surface::read_rect`] back.
    pub fn write_rect(&mut self, rect: PixelRect, data: &[Bgra]) {
        let w = self.width as usize;
        let rw = rect.width as usize;
        for (i, y) in (rect.y..rect.bottom()).enumerate() {
            let o = y as usize * w + rect.x as usize;
            self.pixels[o..o + rw].copy_from_slice(&data[i * rw..(i + 1) * rw]);
        }
    }

    /// Converts to tightly packed RGBA8 (canvas `ImageData` order).
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut out = vec![0u8; self.pixels.len() * 4];
        write_rgba8(&self.pixels, &mut out);
        out
    }
}

/// BGRA → RGBA swizzle into `out` (len = 4 × pixels).
pub fn write_rgba8(px: &[Bgra], out: &mut [u8]) {
    for (p, o) in px.iter().zip(out.chunks_exact_mut(4)) {
        o[0] = p.r;
        o[1] = p.g;
        o[2] = p.b;
        o[3] = p.a;
    }
}
