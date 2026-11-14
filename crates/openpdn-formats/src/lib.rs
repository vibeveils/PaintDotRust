//! OpenPDN file formats.
//!
//! Only the formats the engine must understand byte-for-byte live here: PNG
//! (including tEXt/zTXt/iTXt preservation), the layered `.openpdn` container
//! and legacy Paint.NET `.pdn`. JPEG, WebP, AVIF, HEIC, GIF and friends are
//! decoded and encoded by the host (the browser's codecs), which keeps the
//! WebAssembly module small.

pub mod checksum;
pub mod json;
pub mod openpdn;
pub mod pdn;
pub mod png;
pub mod zip;

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError(pub String);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

pub type Result<T> = std::result::Result<T, FormatError>;

/// Largest image any decoder will allocate: 2²⁸ pixels (1 GiB of BGRA),
/// so a hostile header can't request an allocation that aborts the process.
pub const MAX_PIXELS: u64 = 1 << 28;

pub fn check_size(w: i64, h: i64) -> Result<()> {
    if w <= 0 || h <= 0 || w > 1 << 20 || h > 1 << 20 || (w as u64) * (h as u64) > MAX_PIXELS {
        return err(format!("image size {w}×{h} is not supported"));
    }
    Ok(())
}

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(FormatError(msg.into()))
}

/// Identifies a file from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sniffed {
    Png,
    OpenPdn,
    PdnLegacy,
    Other,
}

pub fn sniff(data: &[u8]) -> Sniffed {
    if png::is_png(data) {
        Sniffed::Png
    } else if data.starts_with(b"PDN3") {
        Sniffed::PdnLegacy
    } else if data.starts_with(b"PK\x03\x04") {
        Sniffed::OpenPdn
    } else {
        Sniffed::Other
    }
}
