//! OpenPDN engine core.
//!
//! A Rust reimplementation of the OpenPDN C# `OpenPDN.Core` library: pixel
//! surfaces, layers and blend modes, selections, undo history, resampling,
//! document transforms and the painting/selection tools. It has no platform
//! dependencies, so the same code runs natively (tests, benchmarks, a future
//! desktop shell) and in the browser as WebAssembly.
//!
//! Pixel results are intended to match the C# port bit-for-bit; the parity
//! harness in `rust/parity` checks this against golden outputs.

#![forbid(unsafe_op_in_unsafe_fn)]

pub mod color;
pub mod rect;
pub mod surface;
pub mod blend;
pub mod document;
pub mod selection;
pub mod selection_ops;
pub mod history;
pub mod resample;
pub mod transforms;
pub mod sampling;
pub mod draw;
pub mod brushes;
pub mod tools;
pub mod adjust;
pub mod effects;
mod par;
mod num;

pub use blend::BlendMode;
pub use color::Bgra;
pub use document::{Document, Layer, LayerId};
pub use history::HistoryStack;
pub use rect::{PixelRect, RectF};
pub use selection::{Selection, SelectionCombineMode, SelectionShape};
pub use surface::Surface;
