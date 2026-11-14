//! Documents, layers and compositing.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::blend::{self, BlendMode};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::selection::Selection;
use crate::surface::Surface;

/// Stable identity for a layer, independent of its position in the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LayerId(pub u64);

impl LayerId {
    fn next() -> LayerId {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        LayerId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// A bitmap layer (C# `BitmapLayer`).
#[derive(Clone, Debug)]
pub struct Layer {
    id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: u8,
    pub blend_mode: BlendMode,
    pub surface: Surface,
}

impl Layer {
    pub fn new(width: i32, height: i32, name: impl Into<String>) -> Layer {
        Layer::from_surface(Surface::new(width, height), name)
    }

    pub fn from_surface(surface: Surface, name: impl Into<String>) -> Layer {
        Layer {
            id: LayerId::next(),
            name: name.into(),
            visible: true,
            opacity: 255,
            blend_mode: BlendMode::Normal,
            surface,
        }
    }

    #[inline]
    pub fn id(&self) -> LayerId {
        self.id
    }

    pub fn width(&self) -> i32 {
        self.surface.width()
    }

    pub fn height(&self) -> i32 {
        self.surface.height()
    }

    /// Copy with a fresh id (C# `DeepCopy`).
    pub fn duplicate(&self, new_name: impl Into<String>) -> Layer {
        Layer { id: LayerId::next(), name: new_name.into(), ..self.clone() }
    }

    /// Same layer with properties copied but new pixels (keeps no id).
    pub(crate) fn with_surface(&self, surface: Surface) -> Layer {
        Layer {
            id: LayerId::next(),
            name: self.name.clone(),
            visible: self.visible,
            opacity: self.opacity,
            blend_mode: self.blend_mode,
            surface,
        }
    }
}

/// PNG text chunk kinds preserved through load/save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PngTextKind {
    Text,
    CompressedText,
    InternationalText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PngTextChunk {
    pub keyword: String,
    pub text: String,
    pub kind: PngTextKind,
    pub language_tag: String,
    pub translated_keyword: String,
    pub compressed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub text_chunks: Vec<PngTextChunk>,
}

/// A layered image (C# `Document`). `layers[0]` is the bottom layer.
///
/// Instead of change events the document carries revision counters that the
/// UI compares against to decide what to redraw.
#[derive(Clone, Debug)]
pub struct Document {
    width: i32,
    height: i32,
    pub layers: Vec<Layer>,
    pub metadata: DocumentMetadata,
    selection: Option<Selection>,
    /// Bumped whenever pixels or layer properties change.
    pub visual_revision: u64,
    /// Bumped whenever the selection changes.
    pub selection_revision: u64,
    /// Region touched since the UI last called [`Document::take_dirty`].
    dirty: PixelRect,
    /// Selection quality: hard-edged clipping when true (see [`Selection::clip_mask`]).
    selection_pixelated: bool,
}

impl Document {
    pub fn new(width: i32, height: i32) -> Document {
        assert!(width > 0 && height > 0, "document dimensions must be positive");
        Document {
            width,
            height,
            layers: Vec::new(),
            metadata: DocumentMetadata::default(),
            selection: None,
            visual_revision: 0,
            selection_revision: 0,
            dirty: PixelRect::EMPTY,
            selection_pixelated: false,
        }
    }

    pub fn from_single_layer(surface: Surface, name: &str) -> Document {
        let mut d = Document::new(surface.width(), surface.height());
        d.layers.push(Layer::from_surface(surface, name));
        d
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.width
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    pub fn bounds(&self) -> PixelRect {
        PixelRect::new(0, 0, self.width, self.height)
    }

    #[inline]
    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
    }

    /// The selection when it exists and is non-empty.
    #[inline]
    pub fn active_selection(&self) -> Option<&Selection> {
        self.selection.as_ref().filter(|s| !s.is_empty())
    }

    pub fn selection_mut(&mut self) -> Option<&mut Selection> {
        self.selection_revision += 1;
        self.selection.as_mut()
    }

    pub fn set_selection(&mut self, mut sel: Option<Selection>) {
        if let Some(s) = sel.as_mut() {
            s.set_pixelated(self.selection_pixelated);
        }
        self.selection = sel;
        self.selection_revision += 1;
    }

    /// Sets the selection quality (pixelated or antialiased clipping) for the
    /// current and future selections.
    pub fn set_selection_pixelated(&mut self, on: bool) {
        self.selection_pixelated = on;
        if let Some(s) = self.selection.as_mut() {
            s.set_pixelated(on);
        }
    }

    pub fn selection_pixelated(&self) -> bool {
        self.selection_pixelated
    }

    pub fn take_selection(&mut self) -> Option<Selection> {
        self.selection_revision += 1;
        self.selection.take()
    }

    pub fn layer_index(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }

    /// Splits the borrow: a layer's pixels mutably plus the selection.
    pub fn layer_and_selection(&mut self, id: LayerId) -> Option<(&mut Layer, Option<&Selection>)> {
        let sel = self.selection.as_ref();
        self.layers.iter_mut().find(|l| l.id == id).map(|l| (l, sel))
    }

    /// Records that `rect` changed visually (whole image if `None`).
    pub fn invalidate(&mut self, rect: Option<PixelRect>) {
        self.visual_revision += 1;
        let r = rect.unwrap_or(self.bounds()).intersect(&self.bounds());
        self.dirty = self.dirty.union(&r);
    }

    /// Returns and resets the accumulated dirty region.
    pub fn take_dirty(&mut self) -> PixelRect {
        std::mem::take(&mut self.dirty)
    }

    /// Flattens all visible layers into a new surface.
    pub fn composite(&self) -> Surface {
        let mut s = Surface::new(self.width, self.height);
        self.composite_into(&mut s);
        s
    }

    /// Flattens into `out` (must match the document size).
    ///
    /// Works row by row through every layer, so each output row stays in
    /// cache instead of streaming the whole image once per layer.
    pub fn composite_into(&self, out: &mut Surface) {
        assert!(out.width() == self.width && out.height() == self.height);
        let w = self.width as usize;
        let layers: Vec<&Layer> = self
            .layers
            .iter()
            .filter(|l| l.visible && l.opacity != 0 && l.width() == self.width && l.height() == self.height)
            .collect();
        // Bands of rows: big enough to amortise per-layer setup, small enough
        // (≈256 KB) to stay in L2 while every layer is applied.
        let band_rows = (65536 / w).clamp(1, 64);
        let band = w * band_rows;
        crate::par::for_rows(out.pixels_mut(), band, |b, row| {
            let o = b * band;
            let w = row.len();
            let mut first = true;
            for l in &layers {
                let src = &l.surface.pixels()[o..o + w];
                if first && l.blend_mode == BlendMode::Normal && l.opacity == 255 {
                    // Normal opaque-opacity layer over transparent: a copy, except
                    // that "over" canonicalises zero-alpha pixels.
                    for (d, &s) in row.iter_mut().zip(src) {
                        *d = if s.a == 0 { Bgra::TRANSPARENT } else { s };
                    }
                } else {
                    if first {
                        row.fill(Bgra::TRANSPARENT);
                    }
                    blend::blend_row(row, src, l.blend_mode, l.opacity);
                }
                first = false;
            }
            if first {
                row.fill(Bgra::TRANSPARENT);
            }
        });
    }

    /// Flattens only `rect` into the matching region of `out` (`rect.width × rect.height`
    /// pixels, packed). Used for incremental canvas updates.
    pub fn composite_rect(&self, rect: PixelRect, out: &mut [Bgra]) {
        let r = rect.intersect(&self.bounds());
        assert_eq!(rect, r, "rect must lie inside the document");
        let rw = r.width as usize;
        assert_eq!(out.len(), rw * r.height as usize);
        out.fill(Bgra::TRANSPARENT);
        let w = self.width as usize;
        for layer in &self.layers {
            if !layer.visible || layer.opacity == 0 || layer.width() != self.width || layer.height() != self.height {
                continue;
            }
            let src = layer.surface.pixels();
            for (i, y) in (r.y..r.bottom()).enumerate() {
                let o = y as usize * w + r.x as usize;
                let d = &mut out[i * rw..(i + 1) * rw];
                blend::apply_layer_rows(d, &src[o..o + rw], rw, layer.blend_mode, layer.opacity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_bottom_to_top() {
        let mut d = Document::new(2, 1);
        let mut a = Layer::new(2, 1, "a");
        a.surface.clear(Bgra::new(0, 0, 255, 255));
        let mut b = Layer::new(2, 1, "b");
        b.surface.set(1, 0, Bgra::new(255, 0, 0, 255));
        d.layers.push(a);
        d.layers.push(b);
        let c = d.composite();
        assert_eq!(c.get(0, 0), Bgra::new(0, 0, 255, 255));
        assert_eq!(c.get(1, 0), Bgra::new(255, 0, 0, 255));
        let mut part = vec![Bgra::TRANSPARENT; 1];
        d.composite_rect(PixelRect::new(1, 0, 1, 1), &mut part);
        assert_eq!(part[0], c.get(1, 0));
    }
}
