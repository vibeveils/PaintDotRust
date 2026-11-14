//! Undo/redo (C# `History`). Entries mutate the [`Document`] they are given,
//! so they hold no references and the stack can live beside the document.

use crate::blend::BlendMode;
use crate::color::Bgra;
use crate::document::{Document, Layer, LayerId};
use crate::rect::PixelRect;
use crate::selection::Selection;

pub trait HistoryEntry: Send {
    fn description(&self) -> &str;
    fn undo(&mut self, doc: &mut Document);
    fn redo(&mut self, doc: &mut Document);
    /// Approximate heap bytes held, for memory budgeting.
    fn memory_size(&self) -> usize {
        0
    }
}

/// Before/after pixels of a rectangle of one layer (C# `BitmapLayerEdit`).
pub struct BitmapLayerEdit {
    layer: LayerId,
    rect: PixelRect,
    before: Vec<Bgra>,
    after: Vec<Bgra>,
    description: String,
}

impl BitmapLayerEdit {
    /// Captures `rect` from the full-surface snapshot `before_full` and the
    /// layer's current pixels. Returns `None` for an empty rect or missing layer.
    pub fn capture(doc: &Document, layer: LayerId, before_full: &[Bgra], rect: PixelRect, description: &str) -> Option<BitmapLayerEdit> {
        if rect.is_empty() {
            return None;
        }
        let l = doc.layer(layer)?;
        let stride = l.width() as usize;
        let (w, h) = (rect.width as usize, rect.height as usize);
        let mut before = Vec::with_capacity(w * h);
        for row in 0..h {
            let o = (rect.y as usize + row) * stride + rect.x as usize;
            before.extend_from_slice(&before_full[o..o + w]);
        }
        let after = l.surface.read_rect(rect);
        Some(BitmapLayerEdit { layer, rect, before, after, description: description.to_owned() })
    }

    fn restore(&self, doc: &mut Document, data: &[Bgra]) {
        if let Some(l) = doc.layer_mut(self.layer) {
            l.surface.write_rect(self.rect, data);
        }
        doc.invalidate(Some(self.rect));
    }

    pub fn rect(&self) -> PixelRect {
        self.rect
    }
}

impl HistoryEntry for BitmapLayerEdit {
    fn description(&self) -> &str {
        &self.description
    }
    fn undo(&mut self, doc: &mut Document) {
        self.restore(doc, &self.before);
    }
    fn redo(&mut self, doc: &mut Document) {
        self.restore(doc, &self.after);
    }
    fn memory_size(&self) -> usize {
        (self.before.len() + self.after.len()) * 4
    }
}

pub struct SelectionEdit {
    before: Option<Selection>,
    after: Option<Selection>,
    description: String,
}

impl SelectionEdit {
    pub fn new(before: Option<Selection>, after: Option<Selection>, description: &str) -> Self {
        SelectionEdit { before, after, description: description.to_owned() }
    }
}

impl HistoryEntry for SelectionEdit {
    fn description(&self) -> &str {
        &self.description
    }
    fn undo(&mut self, doc: &mut Document) {
        doc.set_selection(self.before.clone());
    }
    fn redo(&mut self, doc: &mut Document) {
        doc.set_selection(self.after.clone());
    }
    fn memory_size(&self) -> usize {
        self.before.as_ref().map_or(0, |s| s.mask().len()) + self.after.as_ref().map_or(0, |s| s.mask().len())
    }
}

/// Inserting a layer. The layer is held here while undone.
pub struct LayerAddEdit {
    index: usize,
    id: LayerId,
    parked: Option<Layer>,
    description: String,
}

impl LayerAddEdit {
    /// Records a layer already inserted at `index`.
    pub fn new(doc: &Document, index: usize, description: &str) -> Self {
        LayerAddEdit { index, id: doc.layers[index].id(), parked: None, description: description.to_owned() }
    }
}

impl HistoryEntry for LayerAddEdit {
    fn description(&self) -> &str {
        &self.description
    }
    fn undo(&mut self, doc: &mut Document) {
        if let Some(i) = doc.layer_index(self.id) {
            self.parked = Some(doc.layers.remove(i));
            doc.invalidate(None);
        }
    }
    fn redo(&mut self, doc: &mut Document) {
        if let Some(l) = self.parked.take() {
            doc.layers.insert(self.index.min(doc.layers.len()), l);
            doc.invalidate(None);
        }
    }
}

/// Removing a layer. The layer is held here while removed.
pub struct LayerRemoveEdit {
    index: usize,
    id: LayerId,
    parked: Option<Layer>,
}

impl LayerRemoveEdit {
    /// Records a layer already removed from `index`.
    pub fn new(layer: Layer, index: usize) -> Self {
        LayerRemoveEdit { index, id: layer.id(), parked: Some(layer) }
    }
}

impl HistoryEntry for LayerRemoveEdit {
    fn description(&self) -> &str {
        "Delete Layer"
    }
    fn undo(&mut self, doc: &mut Document) {
        if let Some(l) = self.parked.take() {
            doc.layers.insert(self.index.min(doc.layers.len()), l);
            doc.invalidate(None);
        }
    }
    fn redo(&mut self, doc: &mut Document) {
        if let Some(i) = doc.layer_index(self.id) {
            self.parked = Some(doc.layers.remove(i));
            doc.invalidate(None);
        }
    }
    fn memory_size(&self) -> usize {
        self.parked.as_ref().map_or(0, |l| l.surface.pixels().len() * 4)
    }
}

pub struct LayerMoveEdit {
    from: usize,
    to: usize,
}

impl LayerMoveEdit {
    pub fn new(from: usize, to: usize) -> Self {
        LayerMoveEdit { from, to }
    }
}

fn move_layer(doc: &mut Document, from: usize, to: usize) {
    if from < doc.layers.len() && to < doc.layers.len() {
        let l = doc.layers.remove(from);
        doc.layers.insert(to, l);
        doc.invalidate(None);
    }
}

impl HistoryEntry for LayerMoveEdit {
    fn description(&self) -> &str {
        "Reorder Layer"
    }
    fn undo(&mut self, doc: &mut Document) {
        move_layer(doc, self.to, self.from);
    }
    fn redo(&mut self, doc: &mut Document) {
        move_layer(doc, self.from, self.to);
    }
}

/// A layer property value for [`LayerPropertyEdit`].
#[derive(Clone, Debug, PartialEq)]
pub enum LayerProperty {
    Visible(bool),
    Opacity(u8),
    BlendMode(BlendMode),
    Name(String),
}

impl LayerProperty {
    fn apply(&self, l: &mut Layer) {
        match self {
            LayerProperty::Visible(v) => l.visible = *v,
            LayerProperty::Opacity(v) => l.opacity = *v,
            LayerProperty::BlendMode(v) => l.blend_mode = *v,
            LayerProperty::Name(v) => l.name = v.clone(),
        }
    }
}

pub struct LayerPropertyEdit {
    layer: LayerId,
    old: LayerProperty,
    new: LayerProperty,
}

impl LayerPropertyEdit {
    pub fn new(layer: LayerId, old: LayerProperty, new: LayerProperty) -> Self {
        LayerPropertyEdit { layer, old, new }
    }

    fn set(&self, doc: &mut Document, v: &LayerProperty) {
        if let Some(l) = doc.layer_mut(self.layer) {
            v.apply(l);
        }
        doc.invalidate(None);
    }
}

impl HistoryEntry for LayerPropertyEdit {
    fn description(&self) -> &str {
        match self.new {
            LayerProperty::Visible(_) => "Toggle Visibility",
            LayerProperty::Opacity(_) => "Change Opacity",
            LayerProperty::BlendMode(_) => "Change Blend Mode",
            LayerProperty::Name(_) => "Rename Layer",
        }
    }
    fn undo(&mut self, doc: &mut Document) {
        let v = self.old.clone();
        self.set(doc, &v);
    }
    fn redo(&mut self, doc: &mut Document) {
        let v = self.new.clone();
        self.set(doc, &v);
    }
}

/// Swaps the whole document (C# `DocumentSwapEdit`): resize, rotate, crop…
/// Holds the inactive version; undo and redo both swap.
pub struct DocumentSwapEdit {
    other: Box<Document>,
    description: String,
}

impl DocumentSwapEdit {
    /// Replaces `*doc` with `new_doc` and returns the entry that undoes it.
    pub fn apply(doc: &mut Document, new_doc: Document, description: &str) -> Self {
        let old = std::mem::replace(doc, new_doc);
        doc.visual_revision = old.visual_revision;
        doc.selection_revision = old.selection_revision + 1;
        doc.invalidate(None);
        DocumentSwapEdit { other: Box::new(old), description: description.to_owned() }
    }

    fn swap(&mut self, doc: &mut Document) {
        let (vr, sr) = (doc.visual_revision, doc.selection_revision);
        std::mem::swap(doc, &mut self.other);
        doc.visual_revision = vr + 1;
        doc.selection_revision = sr + 1;
        doc.invalidate(None);
    }
}

impl HistoryEntry for DocumentSwapEdit {
    fn description(&self) -> &str {
        &self.description
    }
    fn undo(&mut self, doc: &mut Document) {
        self.swap(doc);
    }
    fn redo(&mut self, doc: &mut Document) {
        self.swap(doc);
    }
    fn memory_size(&self) -> usize {
        self.other.layers.iter().map(|l| l.surface.pixels().len() * 4).sum()
    }
}

/// An operation that is its own inverse (flip, rotate 180°).
pub struct SelfInverseEdit {
    apply: Box<dyn FnMut(&mut Document) + Send>,
    description: String,
}

impl SelfInverseEdit {
    pub fn new(description: &str, apply: impl FnMut(&mut Document) + Send + 'static) -> Self {
        SelfInverseEdit { apply: Box::new(apply), description: description.to_owned() }
    }
}

impl HistoryEntry for SelfInverseEdit {
    fn description(&self) -> &str {
        &self.description
    }
    fn undo(&mut self, doc: &mut Document) {
        (self.apply)(doc);
    }
    fn redo(&mut self, doc: &mut Document) {
        (self.apply)(doc);
    }
}

pub struct CompoundEdit {
    entries: Vec<Box<dyn HistoryEntry>>,
    description: String,
}

impl CompoundEdit {
    pub fn new(entries: Vec<Box<dyn HistoryEntry>>, description: &str) -> Self {
        CompoundEdit { entries, description: description.to_owned() }
    }
}

impl HistoryEntry for CompoundEdit {
    fn description(&self) -> &str {
        &self.description
    }
    fn undo(&mut self, doc: &mut Document) {
        for e in self.entries.iter_mut().rev() {
            e.undo(doc);
        }
    }
    fn redo(&mut self, doc: &mut Document) {
        for e in self.entries.iter_mut() {
            e.redo(doc);
        }
    }
    fn memory_size(&self) -> usize {
        self.entries.iter().map(|e| e.memory_size()).sum()
    }
}

/// Linear undo stack with a capacity limit (C# `HistoryStack`).
pub struct HistoryStack {
    entries: Vec<Box<dyn HistoryEntry>>,
    applied: usize,
    capacity: usize,
    /// Bumped on every change, for UI refresh.
    pub revision: u64,
}

impl Default for HistoryStack {
    fn default() -> Self {
        HistoryStack::new(100)
    }
}

impl HistoryStack {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        HistoryStack { entries: Vec::new(), applied: 0, capacity, revision: 0 }
    }

    pub fn can_undo(&self) -> bool {
        self.applied > 0
    }

    pub fn can_redo(&self) -> bool {
        self.applied < self.entries.len()
    }

    pub fn undo_depth(&self) -> usize {
        self.applied
    }

    pub fn redo_depth(&self) -> usize {
        self.entries.len() - self.applied
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn descriptions(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.description())
    }

    pub fn next_undo_description(&self) -> Option<&str> {
        self.can_undo().then(|| self.entries[self.applied - 1].description())
    }

    pub fn next_redo_description(&self) -> Option<&str> {
        self.can_redo().then(|| self.entries[self.applied].description())
    }

    pub fn memory_size(&self) -> usize {
        self.entries.iter().map(|e| e.memory_size()).sum()
    }

    pub fn push(&mut self, entry: impl HistoryEntry + 'static) {
        self.push_boxed(Box::new(entry));
    }

    pub fn push_boxed(&mut self, entry: Box<dyn HistoryEntry>) {
        self.entries.truncate(self.applied);
        self.entries.push(entry);
        self.applied += 1;
        if self.entries.len() > self.capacity {
            let excess = self.entries.len() - self.capacity;
            self.entries.drain(..excess);
            self.applied -= excess;
        }
        self.revision += 1;
    }

    pub fn undo(&mut self, doc: &mut Document) {
        if !self.can_undo() {
            return;
        }
        self.entries[self.applied - 1].undo(doc);
        self.applied -= 1;
        self.revision += 1;
    }

    pub fn redo(&mut self, doc: &mut Document) {
        if !self.can_redo() {
            return;
        }
        self.entries[self.applied].redo(doc);
        self.applied += 1;
        self.revision += 1;
    }

    pub fn jump_to(&mut self, doc: &mut Document, applied: usize) {
        let target = applied.min(self.entries.len());
        if target == self.applied {
            return;
        }
        while self.applied > target {
            self.entries[self.applied - 1].undo(doc);
            self.applied -= 1;
        }
        while self.applied < target {
            self.entries[self.applied].redo(doc);
            self.applied += 1;
        }
        self.revision += 1;
    }

    pub fn clear(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        self.entries.clear();
        self.applied = 0;
        self.revision += 1;
    }
}
