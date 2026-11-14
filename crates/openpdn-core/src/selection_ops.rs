//! Pixel operations driven by a selection (C# `SelectionOps`).

use crate::blend::over_normal;
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::selection::Selection;
use crate::surface::Surface;
use crate::transforms::blit_clipped;

fn clipped(s: &Surface, b: PixelRect) -> (i32, i32) {
    (s.width().min(b.right()), s.height().min(b.bottom()))
}

/// Erases selected pixels (partial coverage reduces alpha).
pub fn erase_selection(s: &mut Surface, sel: &Selection) -> PixelRect {
    let b = sel.bounds();
    if b.is_empty() {
        return PixelRect::EMPTY;
    }
    let (x1, y1) = clipped(s, b);
    for y in b.y..y1 {
        let row = s.row_mut(y);
        for x in b.x..x1 {
            let m = sel.coverage_at(x, y);
            if m == 0 {
                continue;
            }
            let c = row[x as usize];
            row[x as usize] = if m == 255 {
                Bgra::TRANSPARENT
            } else {
                c.with_alpha((c.a as i32 * (255 - m as i32) / 255) as u8)
            };
        }
    }
    PixelRect::new(b.x, b.y, (x1 - b.x).max(0), (y1 - b.y).max(0))
}

/// Fills selected pixels with `color` (partial coverage blends over).
pub fn fill_selection(s: &mut Surface, sel: &Selection, color: Bgra) -> PixelRect {
    let b = sel.bounds();
    if b.is_empty() {
        return PixelRect::EMPTY;
    }
    let (x1, y1) = clipped(s, b);
    for y in b.y..y1 {
        let row = s.row_mut(y);
        for x in b.x..x1 {
            let m = sel.coverage_at(x, y);
            if m == 0 {
                continue;
            }
            row[x as usize] = if m == 255 {
                color
            } else {
                let a = ((color.a as i32 * m as i32 + 127) / 255) as u8;
                over_normal(color.with_alpha(a), row[x as usize], 255)
            };
        }
    }
    PixelRect::new(b.x, b.y, (x1 - b.x).max(0), (y1 - b.y).max(0))
}

/// Copies the selected pixels into a new surface the size of the selection bounds.
pub fn copy_from_selection(s: &Surface, sel: &Selection) -> Option<Surface> {
    let b = sel.bounds();
    if b.is_empty() {
        return None;
    }
    let mut result = Surface::new(b.width, b.height);
    let mask = sel.clip_mask();
    let x0 = b.x.max(0);
    let x1 = s.width().min(b.right());
    let sw = sel.width() as usize;
    for y in 0..b.height {
        let sy = b.y + y;
        if (sy as u32) >= s.height() as u32 {
            continue;
        }
        let src_row = s.row(sy);
        let dst_row = result.row_mut(y);
        if x1 > x0 && mask[sy as usize * sw + x0 as usize..sy as usize * sw + x1 as usize].iter().all(|&m| m == 255) {
            let d0 = (x0 - b.x) as usize;
            dst_row[d0..d0 + (x1 - x0) as usize].copy_from_slice(&src_row[x0 as usize..x1 as usize]);
            continue;
        }
        for x in 0..b.width {
            let sx = b.x + x;
            if (sx as u32) >= s.width() as u32 {
                continue;
            }
            let m = sel.coverage_at(sx, sy);
            let c = src_row[sx as usize];
            dst_row[x as usize] = match m {
                0 => Bgra::TRANSPARENT,
                255 => c,
                _ => c.with_alpha((c.a as i32 * m as i32 / 255) as u8),
            };
        }
    }
    Some(result)
}

/// Overwrites `dst` with `src` at (ox, oy), clipped.
pub fn paste_into(dst: &mut Surface, src: &Surface, ox: i32, oy: i32) -> PixelRect {
    blit_clipped(src, dst, ox, oy);
    let x0 = ox.max(0);
    let y0 = oy.max(0);
    let x1 = dst.width().min(ox + src.width());
    let y1 = dst.height().min(oy + src.height());
    if x1 <= x0 || y1 <= y0 { PixelRect::EMPTY } else { PixelRect::new(x0, y0, x1 - x0, y1 - y0) }
}
