//! Selection outlines for "marching ants": closed polygons along the
//! boundary of the half-covered region of a selection mask.

use openpdn_core::Selection;
use std::collections::HashMap;

/// Maximum boundary edges traced before falling back to the bounding box,
/// so pathological masks (noise wand selections) stay cheap to draw.
const MAX_EDGES: usize = 400_000;

/// Polygons as flat `[x0, y0, x1, y1, …]` vertex lists in pixel-corner coordinates.
pub fn outline(sel: &Selection) -> Vec<Vec<i32>> {
    if sel.is_empty() {
        return Vec::new();
    }
    if let Some(pts) = sel.outline() {
        // Lasso selections draw their polygon (pixel centres → +0.5 handled by the UI).
        return vec![pts.iter().flat_map(|&(x, y)| [x, y]).collect()];
    }
    let b = sel.bounds();
    let inside = |x: i32, y: i32| sel.coverage_at(x, y) >= 128;
    // Directed boundary edges with the inside on the right (clockwise in screen space).
    // Key: start vertex → list of end vertices.
    let mut edges: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    let mut count = 0usize;
    for y in b.y..=b.bottom() {
        for x in b.x..=b.right() {
            let here = inside(x, y);
            // Top edge of pixel (x, y): between (x, y-1) and (x, y).
            let above = inside(x, y - 1);
            if here != above {
                if here {
                    edges.entry((x, y)).or_default().push((x + 1, y));
                } else {
                    edges.entry((x + 1, y)).or_default().push((x, y));
                }
                count += 1;
            }
            let left = inside(x - 1, y);
            if here != left {
                if here {
                    edges.entry((x, y + 1)).or_default().push((x, y));
                } else {
                    edges.entry((x, y)).or_default().push((x, y + 1));
                }
                count += 1;
            }
            if count > MAX_EDGES {
                let (x0, y0, x1, y1) = (b.x, b.y, b.right(), b.bottom());
                return vec![vec![x0, y0, x1, y0, x1, y1, x0, y1]];
            }
        }
    }
    let mut polys = Vec::new();
    while let Some(&start) = edges.keys().next() {
        let mut poly: Vec<(i32, i32)> = vec![start];
        let mut cur = start;
        let mut prev_dir = (0, 0);
        loop {
            let list = edges.get_mut(&cur).unwrap();
            // At a saddle (two outgoing edges) prefer turning right to keep loops simple.
            let idx = if list.len() > 1 {
                let right = (-prev_dir.1, prev_dir.0);
                list.iter().position(|&e| (e.0 - cur.0, e.1 - cur.1) == right).unwrap_or(0)
            } else {
                0
            };
            let next = list.swap_remove(idx);
            if list.is_empty() {
                edges.remove(&cur);
            }
            let dir = (next.0 - cur.0, next.1 - cur.1);
            if dir == prev_dir {
                // Collinear: extend the last segment.
                *poly.last_mut().unwrap() = next;
            } else {
                poly.push(next);
            }
            prev_dir = dir;
            cur = next;
            if cur == start || !edges.contains_key(&cur) {
                break;
            }
        }
        if poly.len() > 1 && poly.last() == Some(&start) {
            poly.pop();
        }
        // Merge the closing collinear run into the first vertex.
        if poly.len() >= 3 {
            let n = poly.len();
            let (a, b0, c) = (poly[n - 1], poly[0], poly[1]);
            if (b0.0 - a.0) * (c.1 - b0.1) == (b0.1 - a.1) * (c.0 - b0.0) {
                poly.remove(0);
            }
        }
        polys.push(poly.into_iter().flat_map(|(x, y)| [x, y]).collect());
    }
    polys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_is_four_corners() {
        let mut s = Selection::new(20, 20);
        s.set_rectangle(2, 3, 5, 4);
        let o = outline(&s);
        assert_eq!(o.len(), 1);
        let mut v: Vec<(i32, i32)> = o[0].chunks(2).map(|c| (c[0], c[1])).collect();
        v.sort();
        assert_eq!(v, vec![(2, 3), (2, 7), (7, 3), (7, 7)]);
    }

    #[test]
    fn hole_gives_two_loops() {
        let mut s = Selection::new(20, 20);
        s.set_rectangle(0, 0, 10, 10);
        let mut hole = Selection::new(20, 20);
        hole.set_rectangle(3, 3, 2, 2);
        let s = Selection::combine(Some(&s), &hole, openpdn_core::SelectionCombineMode::Subtract);
        assert_eq!(outline(&s).len(), 2);
    }
}
