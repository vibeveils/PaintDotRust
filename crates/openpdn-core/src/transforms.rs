//! Whole-surface and whole-document transforms (C# `SurfaceTransforms`,
//! `DocumentTransforms`, `LayerTransform`).

use crate::color::Bgra;
use crate::document::{Document, Layer};
use crate::num::{clamp_byte_i, round_even};
use crate::rect::PixelRect;
use crate::sampling;
use crate::selection::Selection;
use crate::surface::Surface;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResampleMode {
    #[default]
    Best,
    Bilinear,
    NearestNeighbor,
    SuperSampling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum CanvasAnchor {
    TopLeft = 0,
    TopCenter,
    TopRight,
    MiddleLeft,
    #[default]
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

pub fn flip_horizontal(s: &mut Surface) {
    let w = s.width() as usize;
    crate::par::for_rows(s.pixels_mut(), w, |_, row| row.reverse());
}

pub fn flip_vertical(s: &mut Surface) {
    let w = s.width() as usize;
    let h = s.height() as usize;
    let px = s.pixels_mut();
    let (top, bottom) = px.split_at_mut(w * (h / 2));
    let bottom_start = bottom.len() - w * (h / 2);
    let bottom = &mut bottom[bottom_start..];
    for y in 0..h / 2 {
        let t = &mut top[y * w..(y + 1) * w];
        let by = h / 2 - 1 - y;
        let b = &mut bottom[by * w..(by + 1) * w];
        t.swap_with_slice(b);
    }
}

pub fn rotate_180(s: &mut Surface) {
    s.pixels_mut().reverse();
}

pub fn rotate_90_cw(s: &Surface) -> Surface {
    let (w, h) = (s.width(), s.height());
    let mut dst = Surface::new(h, w);
    let src = s.pixels();
    // dst(x', y') with x' = h-1-y, y' = x  ⇒  dst row y' = column x of src, read bottom-up.
    let hw = h as usize;
    crate::par::for_rows(dst.pixels_mut(), hw, |x, row| {
        for (i, d) in row.iter_mut().enumerate() {
            let y = hw - 1 - i;
            *d = src[y * w as usize + x];
        }
    });
    dst
}

pub fn rotate_90_ccw(s: &Surface) -> Surface {
    let (w, h) = (s.width(), s.height());
    let mut dst = Surface::new(h, w);
    let src = s.pixels();
    // dst(y, w-1-x) = src(x, y)  ⇒  dst row r = column (w-1-r) of src, top-down.
    let hw = h as usize;
    let wu = w as usize;
    crate::par::for_rows(dst.pixels_mut(), hw, |r, row| {
        let x = wu - 1 - r;
        for (y, d) in row.iter_mut().enumerate() {
            *d = src[y * wu + x];
        }
    });
    dst
}

pub fn resize(src: &Surface, width: i32, height: i32, mode: ResampleMode) -> Surface {
    assert!(width > 0 && height > 0);
    if width == src.width() && height == src.height() {
        return src.clone();
    }
    match mode {
        ResampleMode::NearestNeighbor => resize_nearest(src, width, height),
        ResampleMode::Bilinear => resize_bilinear(src, width, height),
        ResampleMode::SuperSampling => resize_supersampling(src, width, height),
        ResampleMode::Best => {
            if width < src.width() && height < src.height() {
                resize_supersampling(src, width, height)
            } else {
                resize_bilinear(src, width, height)
            }
        }
    }
}

fn resize_nearest(src: &Surface, width: i32, height: i32) -> Surface {
    let mut dst = Surface::new(width, height);
    let (sw, sh) = (src.width() as i64, src.height() as i64);
    crate::par::for_rows(dst.pixels_mut(), width as usize, |y, row| {
        let sy = (sh - 1).min(y as i64 * sh / height as i64) as i32;
        let srow = src.row(sy);
        for (x, d) in row.iter_mut().enumerate() {
            *d = srow[(sw - 1).min(x as i64 * sw / width as i64) as usize];
        }
    });
    dst
}

fn resize_bilinear(src: &Surface, width: i32, height: i32) -> Surface {
    let mut dst = Surface::new(width, height);
    let sx = src.width() as f64 / width as f64;
    let sy = src.height() as f64 / height as f64;
    let (sw, sh) = (src.width(), src.height());
    crate::par::for_rows(dst.pixels_mut(), width as usize, |y, row| {
        let fy = (y as f64 + 0.5) * sy - 0.5;
        let y0 = fy.floor() as i32;
        let ty = fy - y0 as f64;
        let y1 = (y0 + 1).clamp(0, sh - 1);
        let y0 = y0.clamp(0, sh - 1);
        let row0 = src.row(y0);
        let row1 = src.row(y1);
        for (x, d) in row.iter_mut().enumerate() {
            let fx = (x as f64 + 0.5) * sx - 0.5;
            let x0 = fx.floor() as i32;
            let tx = fx - x0 as f64;
            let x1 = (x0 + 1).clamp(0, sw - 1) as usize;
            let x0 = x0.clamp(0, sw - 1) as usize;
            let (c00, c10, c01, c11) = (row0[x0], row0[x1], row1[x0], row1[x1]);
            let w00 = (1.0 - tx) * (1.0 - ty);
            let w10 = tx * (1.0 - ty);
            let w01 = (1.0 - tx) * ty;
            let w11 = tx * ty;
            let a = c00.a as f64 * w00 + c10.a as f64 * w10 + c01.a as f64 * w01 + c11.a as f64 * w11;
            if a <= 0.0001 {
                *d = Bgra::TRANSPARENT;
                continue;
            }
            // C# multiplies channel × alpha as ints before the double weight.
            let ch = |p: fn(Bgra) -> u8| {
                ((p(c00) as i32 * c00.a as i32) as f64 * w00
                    + (p(c10) as i32 * c10.a as i32) as f64 * w10
                    + (p(c01) as i32 * c01.a as i32) as f64 * w01
                    + (p(c11) as i32 * c11.a as i32) as f64 * w11)
                    / a
            };
            let b = ch(|c| c.b);
            let g = ch(|c| c.g);
            let r = ch(|c| c.r);
            *d = Bgra::new(
                clamp_byte_i(round_even(b) as i32),
                clamp_byte_i(round_even(g) as i32),
                clamp_byte_i(round_even(r) as i32),
                clamp_byte_i(round_even(a) as i32),
            );
        }
    });
    dst
}

fn resize_supersampling(src: &Surface, width: i32, height: i32) -> Surface {
    let mut dst = Surface::new(width, height);
    let sx = src.width() as f64 / width as f64;
    let sy = src.height() as f64 / height as f64;
    let (sw, sh) = (src.width(), src.height());
    crate::par::for_rows(dst.pixels_mut(), width as usize, |y, row| {
        let src_y0 = y as f64 * sy;
        let src_y1 = (sh as f64).min((y + 1) as f64 * sy);
        let iy0 = src_y0.floor() as i32;
        let iy1 = src_y1.ceil() as i32;
        for (x, d) in row.iter_mut().enumerate() {
            let src_x0 = x as f64 * sx;
            let src_x1 = (sw as f64).min((x + 1) as f64 * sx);
            let ix0 = src_x0.floor() as i32;
            let ix1 = src_x1.ceil() as i32;
            let (mut bs, mut gs, mut rs, mut as_, mut area) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
            for yy in iy0..iy1 {
                let h_cov = src_y1.min((yy + 1) as f64) - src_y0.max(yy as f64);
                if h_cov <= 0.0 {
                    continue;
                }
                let srow = src.row(yy.clamp(0, sh - 1));
                for xx in ix0..ix1 {
                    let w_cov = src_x1.min((xx + 1) as f64) - src_x0.max(xx as f64);
                    if w_cov <= 0.0 {
                        continue;
                    }
                    let w = h_cov * w_cov;
                    let c = srow[xx.clamp(0, sw - 1) as usize];
                    area += w;
                    as_ += c.a as f64 * w;
                    bs += (c.b as i32 * c.a as i32) as f64 * w;
                    gs += (c.g as i32 * c.a as i32) as f64 * w;
                    rs += (c.r as i32 * c.a as i32) as f64 * w;
                }
            }
            if area <= 0.0 || as_ <= 0.0001 {
                *d = Bgra::TRANSPARENT;
                continue;
            }
            let a = as_ / area;
            *d = Bgra::new(
                clamp_byte_i(round_even(bs / as_) as i32),
                clamp_byte_i(round_even(gs / as_) as i32),
                clamp_byte_i(round_even(rs / as_) as i32),
                clamp_byte_i(round_even(a) as i32),
            );
        }
    });
    dst
}

/// Copies `src` into `dst` at (ox, oy), clipped (overwrite, no blending).
pub fn blit_clipped(src: &Surface, dst: &mut Surface, ox: i32, oy: i32) {
    let x0 = (-ox).max(0);
    let y0 = (-oy).max(0);
    let x1 = src.width().min(dst.width() - ox);
    let y1 = src.height().min(dst.height() - oy);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let w = (x1 - x0) as usize;
    for y in y0..y1 {
        let s = &src.row(y)[x0 as usize..x0 as usize + w];
        let d0 = (x0 + ox) as usize;
        dst.row_mut(y + oy)[d0..d0 + w].copy_from_slice(s);
    }
}

pub fn anchor_offset(anchor: CanvasAnchor, old_w: i32, old_h: i32, new_w: i32, new_h: i32) -> (i32, i32) {
    let a = anchor as i32;
    let (col, row) = (a % 3, a / 3);
    let x = match col {
        0 => 0,
        1 => (new_w - old_w) / 2,
        _ => new_w - old_w,
    };
    let y = match row {
        0 => 0,
        1 => (new_h - old_h) / 2,
        _ => new_h - old_h,
    };
    (x, y)
}

fn map(doc: &Document, new_w: i32, new_h: i32, f: impl Fn(&Surface) -> Surface) -> Document {
    let mut result = Document::new(new_w, new_h);
    result.metadata = doc.metadata.clone();
    for l in &doc.layers {
        result.layers.push(l.with_surface(f(&l.surface)));
    }
    result
}

pub fn document_rotate_90_cw(doc: &Document) -> Document {
    map(doc, doc.height(), doc.width(), rotate_90_cw)
}

pub fn document_rotate_90_ccw(doc: &Document) -> Document {
    map(doc, doc.height(), doc.width(), rotate_90_ccw)
}

pub fn document_resize(doc: &Document, width: i32, height: i32, mode: ResampleMode) -> Document {
    map(doc, width, height, |s| resize(s, width, height, mode))
}

pub fn canvas_size(doc: &Document, width: i32, height: i32, anchor: CanvasAnchor) -> Document {
    let (ox, oy) = anchor_offset(anchor, doc.width(), doc.height(), width, height);
    map(doc, width, height, |s| {
        let mut d = Surface::new(width, height);
        blit_clipped(s, &mut d, ox, oy);
        d
    })
}

pub fn crop_to_selection(doc: &Document, sel: &Selection) -> Option<Document> {
    let b = sel.bounds();
    if b.is_empty() {
        return None;
    }
    Some(map(doc, b.width, b.height, |s| {
        let mut d = Surface::new(b.width, b.height);
        for y in 0..b.height {
            let srow = s.row(b.y + y);
            let drow = d.row_mut(y);
            for x in 0..b.width {
                let m = sel.coverage_at(b.x + x, b.y + y);
                let c = srow[(b.x + x) as usize];
                drow[x as usize] = match m {
                    0 => Bgra::TRANSPARENT,
                    255 => c,
                    _ => c.with_alpha((c.a as i32 * m as i32 / 255) as u8),
                };
            }
        }
        d
    }))
}

/// Bounds of non-transparent pixels over all visible layers.
pub fn content_bounds(doc: &Document) -> PixelRect {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for l in doc.layers.iter().filter(|l| l.visible) {
        for y in 0..l.height() {
            let row = l.surface.row(y);
            let Some(first) = row.iter().position(|c| c.a != 0) else { continue };
            let last = row.iter().rposition(|c| c.a != 0).unwrap();
            min_x = min_x.min(first as i32);
            max_x = max_x.max(last as i32);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    PixelRect::from_inclusive(min_x, min_y, max_x, max_y)
}

/// Crops to visible content; `None` if empty or already tight.
pub fn auto_crop(doc: &Document) -> Option<Document> {
    let b = content_bounds(doc);
    if b.is_empty() || b == doc.bounds() {
        return None;
    }
    Some(map(doc, b.width, b.height, |s| Surface::from_pixels(b.width, b.height, s.read_rect(b))))
}

pub fn flatten(doc: &Document) -> Document {
    let mut result = Document::new(doc.width(), doc.height());
    result.metadata = doc.metadata.clone();
    result.layers.push(Layer::from_surface(doc.composite(), "Background"));
    result
}

/// Rotate/zoom a layer around its centre (C# `LayerTransform.RotateZoom`).
#[allow(clippy::too_many_arguments)]
pub fn rotate_zoom(src: &Surface, dst: &mut Surface, angle_degrees: f64, zoom: f64, offset_x: f64, offset_y: f64, tile: bool) {
    let theta = -angle_degrees * std::f64::consts::PI / 180.0;
    let (sin, cos) = theta.sin_cos();
    let inv_zoom = 1.0 / zoom.max(0.01);
    let cx = (src.width() - 1) as f64 / 2.0 + offset_x * src.width() as f64 / 2.0;
    let cy = (src.height() - 1) as f64 / 2.0 + offset_y * src.height() as f64 / 2.0;
    let (sw, sh) = (src.width() as f64, src.height() as f64);
    let w = dst.width() as usize;
    crate::par::for_rows(dst.pixels_mut(), w, |y, row| {
        let dy = y as f64 - cy;
        for (x, d) in row.iter_mut().enumerate() {
            let dx = x as f64 - cx;
            let sx = (dx * cos - dy * sin) * inv_zoom + cx;
            let sy = (dx * sin + dy * cos) * inv_zoom + cy;
            *d = if tile {
                sampling::bilinear_wrapped(src, sx as f32, sy as f32)
            } else if sx >= -0.5 && sy >= -0.5 && sx <= sw - 0.5 && sy <= sh - 0.5 {
                sampling::bilinear_clamped(src, sx as f32, sy as f32)
            } else {
                Bgra::TRANSPARENT
            };
        }
    });
}

