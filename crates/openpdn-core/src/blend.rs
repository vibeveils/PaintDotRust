//! Layer blend modes and the compositing operator (C# `Compositor`).

use crate::color::Bgra;
use crate::surface::Surface;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum BlendMode {
    #[default]
    Normal = 0,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    Difference,
    Add,
    ColorBurn,
    ColorDodge,
    Reflect,
    Glow,
    Negation,
    Xor,
}

impl BlendMode {
    pub const ALL: [BlendMode; 14] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Add,
        BlendMode::ColorBurn,
        BlendMode::ColorDodge,
        BlendMode::Reflect,
        BlendMode::Glow,
        BlendMode::Negation,
        BlendMode::Xor,
    ];

    pub fn from_index(i: u8) -> Option<BlendMode> {
        Self::ALL.get(i as usize).copied()
    }

    /// Name as it appears in the UI and the C# enum.
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Normal => "Normal",
            BlendMode::Multiply => "Multiply",
            BlendMode::Screen => "Screen",
            BlendMode::Overlay => "Overlay",
            BlendMode::Darken => "Darken",
            BlendMode::Lighten => "Lighten",
            BlendMode::Difference => "Difference",
            BlendMode::Add => "Add",
            BlendMode::ColorBurn => "ColorBurn",
            BlendMode::ColorDodge => "ColorDodge",
            BlendMode::Reflect => "Reflect",
            BlendMode::Glow => "Glow",
            BlendMode::Negation => "Negation",
            BlendMode::Xor => "Xor",
        }
    }

    pub fn from_name(s: &str) -> Option<BlendMode> {
        Self::ALL.iter().copied().find(|m| m.name().eq_ignore_ascii_case(s))
    }
}

/// Straight-alpha "over" with a layer opacity (C# `Compositor.OverNormal`).
#[inline(always)]
pub fn over_normal(top: Bgra, bottom: Bgra, layer_opacity: u8) -> Bgra {
    let a_top = (top.a as u32 * layer_opacity as u32 + 127) / 255;
    if a_top == 0 {
        return bottom;
    }
    if a_top == 255 {
        return top;
    }
    let w_top = a_top * 255;
    if bottom.a == 255 {
        // Opaque destination: the weight sum is the constant 65025, so the
        // divisions below compile to multiplies.
        const SUM: u32 = 255 * 255;
        let w_bot = 255 * (255 - a_top);
        let ch = |t: u8, b: u8| ((t as u32 * w_top + b as u32 * w_bot + SUM / 2) / SUM) as u8;
        return Bgra::new(ch(top.b, bottom.b), ch(top.g, bottom.g), ch(top.r, bottom.r), 255);
    }
    let w_bot = bottom.a as u32 * (255 - a_top);
    let sum = w_top + w_bot; // > 0 because a_top > 0
    let a_out = (sum + 127) / 255;
    let half = sum / 2;
    // Exact floor(n / sum): n < 2^24 is exact in f32, so a float reciprocal
    // estimate is off by at most one and a single correction step fixes it.
    let inv = 1.0f32 / sum as f32;
    let ch = |t: u8, b: u8| {
        let n = t as u32 * w_top + b as u32 * w_bot + half;
        let mut q = (n as f32 * inv) as u32;
        if q * sum > n {
            q -= 1;
        } else if (q + 1) * sum <= n {
            q += 1;
        }
        q as u8
    };
    Bgra::new(ch(top.b, bottom.b), ch(top.g, bottom.g), ch(top.r, bottom.r), a_out as u8)
}

/// Branch-free [`over_normal`] (identical results) for vectorised loops.
///
/// With a_top = 255 the general formula already yields `top`, and with
/// a_top = 0 it yields `bottom` except when both alphas are 0, so only that
/// one case needs a select.
#[inline(always)]
fn over_normal_bf(top: Bgra, bottom: Bgra, opacity: i32) -> Bgra {
    let a_top = (top.a as i32 * opacity + 127) / 255;
    let w_top = a_top * 255;
    let w_bot = bottom.a as i32 * (255 - a_top);
    let sum = (w_top + w_bot).max(1);
    let half = sum >> 1;
    let inv = 1.0f32 / sum as f32;
    let div = |n: i32| {
        let q = (n as f32 * inv) as i32;
        let q = q - (q * sum > n) as i32;
        q + ((q + 1) * sum <= n) as i32
    };
    let ch = |t: u8, b: u8| div(t as i32 * w_top + b as i32 * w_bot + half);
    let (b, g, r) = (ch(top.b, bottom.b), ch(top.g, bottom.g), ch(top.r, bottom.r));
    let a = (sum + 127) / 255;
    let keep = a_top == 0;
    if keep {
        bottom
    } else {
        Bgra::new(b as u8, g as u8, r as u8, a as u8)
    }
}

#[inline(always)]
fn mul(t: i32, b: i32) -> i32 {
    (t * b + 127) / 255
}

#[inline(always)]
fn blend_channel(t: u8, b: u8, mode: BlendMode) -> u8 {
    let (t, b) = (t as i32, b as i32);
    let v = match mode {
        BlendMode::Multiply => mul(t, b),
        BlendMode::Screen => 255 - ((255 - t) * (255 - b) + 127) / 255,
        BlendMode::Overlay => {
            if b < 128 {
                (2 * t * b + 127) / 255
            } else {
                255 - (2 * (255 - t) * (255 - b) + 127) / 255
            }
        }
        BlendMode::Darken => t.min(b),
        BlendMode::Lighten => t.max(b),
        BlendMode::Difference => (t - b).abs(),
        BlendMode::Add => (t + b).min(255),
        BlendMode::ColorBurn => {
            if t == 0 { 0 } else { (255 - (255 - b) * 255 / t).max(0) }
        }
        BlendMode::ColorDodge => {
            if t == 255 { 255 } else { (b * 255 / (255 - t)).min(255) }
        }
        BlendMode::Reflect => {
            if t == 255 { 255 } else { (b * b / (255 - t)).min(255) }
        }
        BlendMode::Glow => {
            if b == 255 { 255 } else { (t * t / (255 - b)).min(255) }
        }
        BlendMode::Negation => 255 - (255 - b - t).abs(),
        BlendMode::Xor => b ^ t,
        BlendMode::Normal => t,
    };
    v as u8
}

#[inline(always)]
fn blend_channels(t: Bgra, b: Bgra, mode: BlendMode) -> Bgra {
    Bgra::new(
        blend_channel(t.b, b.b, mode),
        blend_channel(t.g, b.g, mode),
        blend_channel(t.r, b.r, mode),
        t.a,
    )
}

/// Blends one pixel: mode-mix the colour channels, then "over" with opacity.
#[inline]
pub fn blend(top: Bgra, bottom: Bgra, mode: BlendMode, opacity: u8) -> Bgra {
    let t = if mode == BlendMode::Normal { top } else { blend_channels(top, bottom, mode) };
    over_normal(t, bottom, opacity)
}

/// Composites `src` onto `dst` in place (same dimensions).
pub fn apply_layer(dst: &mut Surface, src: &Surface, mode: BlendMode, opacity: u8) {
    assert!(dst.width() == src.width() && dst.height() == src.height());
    let w = dst.width() as usize;
    apply_layer_rows(dst.pixels_mut(), src.pixels(), w, mode, opacity);
}

pub(crate) fn apply_layer_rows(dst: &mut [Bgra], src: &[Bgra], width: usize, mode: BlendMode, opacity: u8) {
    crate::par::for_rows2(dst, src, width, |_, d, s| blend_row(d, s, mode, opacity));
}

/// Composites one row of `src` onto `dst` (serial; callers parallelise).
///
/// Picks the branch-free kernel where it vectorises well (AVX2, wasm
/// SIMD128, NEON) and the branchy one otherwise; both are exact.
pub(crate) fn blend_row(d: &mut [Bgra], s: &[Bgra], mode: BlendMode, opacity: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            // SAFETY: AVX2 support was just checked.
            unsafe { blend_row_avx2(d, s, mode, opacity) };
            return;
        }
    }
    #[cfg(any(target_arch = "aarch64", all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        blend_row_impl::<true>(d, s, mode, opacity);
        return;
    }
    #[allow(unreachable_code)]
    blend_row_impl::<false>(d, s, mode, opacity)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn blend_row_avx2(d: &mut [Bgra], s: &[Bgra], mode: BlendMode, opacity: u8) {
    blend_row_impl::<true>(d, s, mode, opacity)
}

#[inline(always)]
fn blend_row_impl<const BRANCH_FREE: bool>(d: &mut [Bgra], s: &[Bgra], mode: BlendMode, opacity: u8) {
    // Monomorphise per mode so the inner loop has no per-pixel match.
    let op = opacity as i32;
    macro_rules! run {
        ($m:expr) => {
            for (dp, &sp) in d.iter_mut().zip(s) {
                let t = if $m == BlendMode::Normal { sp } else { blend_channels(sp, *dp, $m) };
                *dp = if BRANCH_FREE { over_normal_bf(t, *dp, op) } else { over_normal(t, *dp, opacity) };
            }
        };
    }
    match mode {
        BlendMode::Normal if opacity == 255 && s.iter().all(|p| p.a == 0 || p.a == 255) => {
            // Hard-edged content (the usual case): copy opaque pixels only.
            for (dp, &sp) in d.iter_mut().zip(s) {
                if sp.a != 0 {
                    *dp = sp;
                }
            }
        }
        BlendMode::Normal => run!(BlendMode::Normal),
        BlendMode::Multiply => run!(BlendMode::Multiply),
        BlendMode::Screen => run!(BlendMode::Screen),
        BlendMode::Overlay => run!(BlendMode::Overlay),
        BlendMode::Darken => run!(BlendMode::Darken),
        BlendMode::Lighten => run!(BlendMode::Lighten),
        BlendMode::Difference => run!(BlendMode::Difference),
        BlendMode::Add => run!(BlendMode::Add),
        BlendMode::ColorBurn => run!(BlendMode::ColorBurn),
        BlendMode::ColorDodge => run!(BlendMode::ColorDodge),
        BlendMode::Reflect => run!(BlendMode::Reflect),
        BlendMode::Glow => run!(BlendMode::Glow),
        BlendMode::Negation => run!(BlendMode::Negation),
        BlendMode::Xor => run!(BlendMode::Xor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn over_extremes() {
        let top = Bgra::new(10, 20, 30, 255);
        let bot = Bgra::new(200, 200, 200, 255);
        assert_eq!(over_normal(top, bot, 255), top);
        assert_eq!(over_normal(top, bot, 0), bot);
        assert_eq!(over_normal(Bgra::TRANSPARENT, bot, 255), bot);
        // Half-opaque white over transparent stays white at half alpha.
        assert_eq!(over_normal(Bgra::new(255, 255, 255, 128), Bgra::TRANSPARENT, 255), Bgra::new(255, 255, 255, 128));
    }

    #[test]
    fn exhaustive_over_matches_reference() {
        // Reference: the formula exactly as written in C#.
        fn reference(top: Bgra, bottom: Bgra, op: u8) -> Bgra {
            let a_top = (top.a as i32 * op as i32 + 127) / 255;
            if a_top == 0 { return bottom; }
            if a_top == 255 { return top; }
            let a_bot = bottom.a as i32;
            let w_top = a_top * 255;
            let w_bot = a_bot * (255 - a_top);
            let sum = w_top + w_bot;
            if sum == 0 { return Bgra::TRANSPARENT; }
            let half = sum / 2;
            Bgra::new(
                ((top.b as i32 * w_top + bottom.b as i32 * w_bot + half) / sum) as u8,
                ((top.g as i32 * w_top + bottom.g as i32 * w_bot + half) / sum) as u8,
                ((top.r as i32 * w_top + bottom.r as i32 * w_bot + half) / sum) as u8,
                ((sum + 127) / 255) as u8,
            )
        }
        // All alpha pairs × several opacities × extreme and mid channel values.
        let chans = [0u8, 1, 77, 127, 128, 200, 254, 255];
        for ta in 0..=255u8 {
            for ba in 0..=255u8 {
                for op in [1u8, 128, 255] {
                    for &c1 in &chans {
                        for &c2 in &chans {
                            let t = Bgra::new(c1, c2, 255 - c1, ta);
                            let b = Bgra::new(c2, c1, 255 - c2, ba);
                            let r = reference(t, b, op);
                            assert_eq!(over_normal(t, b, op), r, "{t:?} over {b:?} @ {op}");
                            assert_eq!(over_normal_bf(t, b, op as i32), r, "bf {t:?} over {b:?} @ {op}");
                        }
                    }
                }
            }
        }
    }
}
