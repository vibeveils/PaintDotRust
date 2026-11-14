//! Numeric helpers that reproduce .NET semantics exactly.

/// `Math.Round(double)`: .NET rounds midpoints to even.
#[inline(always)]
pub(crate) fn round_even(v: f64) -> f64 {
    v.round_ties_even()
}

/// `MathF.Round(float)`: midpoints to even.
#[inline(always)]
pub(crate) fn round_even_f(v: f32) -> f32 {
    v.round_ties_even()
}

/// `(byte)Math.Clamp(v, 0, 255)` for doubles: clamp then truncate.
#[inline(always)]
pub(crate) fn clamp_byte_f64(v: f64) -> u8 {
    v.clamp(0.0, 255.0) as u8
}

/// `(byte)Math.Clamp(v, 0, 255)` for floats.
#[inline(always)]
pub(crate) fn clamp_byte_f32(v: f32) -> u8 {
    v.clamp(0.0, 255.0) as u8
}

/// `(byte)Math.Clamp(i, 0, 255)` for ints.
#[inline(always)]
pub(crate) fn clamp_byte_i(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}
