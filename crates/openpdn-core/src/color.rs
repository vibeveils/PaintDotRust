//! 32-bit BGRA colour, byte-compatible with the C# `ColorBgra`.

use std::fmt;

/// Straight (non-premultiplied) 8-bit BGRA pixel. Memory order is B, G, R, A,
/// so a little-endian `u32` view is `0xAARRGGBB`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(C, align(4))]
pub struct Bgra {
    pub b: u8,
    pub g: u8,
    pub r: u8,
    pub a: u8,
}

impl Bgra {
    pub const TRANSPARENT: Bgra = Bgra::new(0, 0, 0, 0);
    pub const BLACK: Bgra = Bgra::new(0, 0, 0, 255);
    pub const WHITE: Bgra = Bgra::new(255, 255, 255, 255);

    #[inline(always)]
    pub const fn new(b: u8, g: u8, r: u8, a: u8) -> Self {
        Bgra { b, g, r, a }
    }

    #[inline(always)]
    pub const fn bgr(b: u8, g: u8, r: u8) -> Self {
        Bgra { b, g, r, a: 255 }
    }

    #[inline(always)]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Bgra { b, g, r, a }
    }

    #[inline(always)]
    pub const fn from_u32(packed: u32) -> Self {
        let [b, g, r, a] = packed.to_le_bytes();
        Bgra { b, g, r, a }
    }

    #[inline(always)]
    pub const fn to_u32(self) -> u32 {
        u32::from_le_bytes([self.b, self.g, self.r, self.a])
    }

    #[inline(always)]
    pub const fn with_alpha(self, a: u8) -> Self {
        Bgra { a, ..self }
    }

    /// Alpha-weighted average of `colors` (C# `ColorBgra.BlendAverage`).
    pub fn blend_average(colors: &[Bgra]) -> Bgra {
        if colors.is_empty() {
            return Bgra::TRANSPARENT;
        }
        let a_sum: u64 = colors.iter().map(|c| c.a as u64).sum();
        let a = (a_sum / colors.len() as u64) as u8;
        if a_sum == 0 {
            return Bgra::new(0, 0, 0, a);
        }
        let (mut b, mut g, mut r) = (0u64, 0u64, 0u64);
        for c in colors {
            b += c.a as u64 * c.b as u64;
            g += c.a as u64 * c.g as u64;
            r += c.a as u64 * c.r as u64;
        }
        Bgra::new((b / a_sum) as u8, (g / a_sum) as u8, (r / a_sum) as u8, a)
    }

    /// Rec.601-ish luma as a byte (C# `GetIntensityByte`).
    #[inline(always)]
    pub fn intensity_byte(self) -> u8 {
        ((7471 * self.b as u32 + 38470 * self.g as u32 + 19595 * self.r as u32) >> 16) as u8
    }

    #[inline(always)]
    pub fn intensity(self) -> f64 {
        (7471 * self.b as u32 + 38470 * self.g as u32 + 19595 * self.r as u32) as f64 / 16711680.0
    }

    /// `RRGGBB`, or `RRGGBBAA` when not opaque.
    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// Parses `#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA` (`#`/`0x` optional).
    pub fn parse_hex(text: &str) -> Option<Bgra> {
        let mut s = text.trim();
        if let Some(rest) = s.strip_prefix('#') {
            s = rest;
        } else if s.len() >= 2 && s[..2].eq_ignore_ascii_case("0x") {
            s = &s[2..];
        }
        let s = s.trim();
        if !matches!(s.len(), 3 | 4 | 6 | 8) {
            return None;
        }
        let mut v = [0u8; 8];
        for (i, c) in s.bytes().enumerate() {
            v[i] = (c as char).to_digit(16)? as u8;
        }
        let (r, g, b, mut a) = if s.len() <= 4 {
            (v[0] * 17, v[1] * 17, v[2] * 17, 255)
        } else {
            (v[0] * 16 + v[1], v[2] * 16 + v[3], v[4] * 16 + v[5], 255)
        };
        if s.len() == 4 {
            a = v[3] * 17;
        } else if s.len() == 8 {
            a = v[6] * 16 + v[7];
        }
        Some(Bgra::new(b, g, r, a))
    }
}

impl fmt::Debug for Bgra {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BGRA({}, {}, {}, {})", self.b, self.g, self.r, self.a)
    }
}

/// Reinterprets a pixel slice as bytes (B, G, R, A order).
#[inline]
pub fn as_bytes(px: &[Bgra]) -> &[u8] {
    // SAFETY: Bgra is repr(C) of four u8s with no padding.
    unsafe { std::slice::from_raw_parts(px.as_ptr().cast::<u8>(), px.len() * 4) }
}

#[inline]
pub fn as_bytes_mut(px: &mut [Bgra]) -> &mut [u8] {
    // SAFETY: as above; every byte pattern is a valid Bgra.
    unsafe { std::slice::from_raw_parts_mut(px.as_mut_ptr().cast::<u8>(), px.len() * 4) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let c = Bgra::parse_hex("#336699").unwrap();
        assert_eq!(c, Bgra::rgba(0x33, 0x66, 0x99, 255));
        assert_eq!(c.to_hex(), "336699");
        assert_eq!(Bgra::parse_hex("0xF0A8").unwrap(), Bgra::rgba(0xFF, 0x00, 0xAA, 0x88));
        assert_eq!(Bgra::parse_hex("12345678").unwrap().to_hex(), "12345678");
        assert!(Bgra::parse_hex("12345").is_none());
        assert!(Bgra::parse_hex("zzz").is_none());
    }

    #[test]
    fn packed_layout() {
        let c = Bgra::new(1, 2, 3, 4);
        assert_eq!(c.to_u32(), 0x04030201);
        assert_eq!(Bgra::from_u32(0x04030201), c);
        assert_eq!(std::mem::size_of::<Bgra>(), 4);
    }
}
