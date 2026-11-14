//! Transform handles for the move tools (C# `TransformHandles`).

use crate::rect::RectF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandleKind {
    None,
    Move,
    N,
    S,
    E,
    W,
    NE,
    NW,
    SE,
    SW,
}

/// Handle positions in hit-test priority order.
pub fn positions(r: RectF) -> [(HandleKind, f64, f64); 8] {
    let cx = r.x + r.w / 2.0;
    let cy = r.y + r.h / 2.0;
    [
        (HandleKind::NW, r.x, r.y),
        (HandleKind::N, cx, r.y),
        (HandleKind::NE, r.right(), r.y),
        (HandleKind::E, r.right(), cy),
        (HandleKind::SE, r.right(), r.bottom()),
        (HandleKind::S, cx, r.bottom()),
        (HandleKind::SW, r.x, r.bottom()),
        (HandleKind::W, r.x, cy),
    ]
}

pub fn hit_test(r: RectF, x: f64, y: f64, radius: f64) -> HandleKind {
    let mut best = HandleKind::None;
    let mut best_d = radius * radius;
    for (k, hx, hy) in positions(r) {
        let d = (x - hx) * (x - hx) + (y - hy) * (y - hy);
        if d <= best_d {
            best_d = d;
            best = k;
        }
    }
    if best != HandleKind::None {
        return best;
    }
    if x >= r.x && x < r.right() && y >= r.y && y < r.bottom() { HandleKind::Move } else { HandleKind::None }
}

/// New rectangle after dragging `handle` by (dx, dy); `constrain` keeps `aspect`.
pub fn resize(start: RectF, handle: HandleKind, dx: f64, dy: f64, constrain: bool, aspect: f64) -> RectF {
    use HandleKind::*;
    let (mut left, mut top, mut right, mut bottom) = (start.x, start.y, start.right(), start.bottom());
    let move_l = matches!(handle, W | NW | SW);
    let move_r = matches!(handle, E | NE | SE);
    let move_t = matches!(handle, N | NW | NE);
    let move_b = matches!(handle, S | SW | SE);
    if move_l {
        left = (start.x + dx).min(right - 1.0);
    }
    if move_r {
        right = (start.right() + dx).max(left + 1.0);
    }
    if move_t {
        top = (start.y + dy).min(bottom - 1.0);
    }
    if move_b {
        bottom = (start.bottom() + dy).max(top + 1.0);
    }
    if constrain && aspect > 0.0 {
        let mut w = right - left;
        let mut h = bottom - top;
        let horizontal = move_l || move_r;
        let vertical = move_t || move_b;
        if horizontal && vertical {
            if w / start.w >= h / start.h {
                h = w / aspect;
            } else {
                w = h * aspect;
            }
            if move_l {
                left = right - w;
            } else {
                right = left + w;
            }
            if move_t {
                top = bottom - h;
            } else {
                bottom = top + h;
            }
        } else if horizontal {
            h = w / aspect;
            let cy = start.y + start.h / 2.0;
            top = cy - h / 2.0;
            bottom = cy + h / 2.0;
        } else if vertical {
            w = h * aspect;
            let cx = start.x + start.w / 2.0;
            left = cx - w / 2.0;
            right = cx + w / 2.0;
        }
    }
    RectF::new(left, top, right - left, bottom - top)
}
