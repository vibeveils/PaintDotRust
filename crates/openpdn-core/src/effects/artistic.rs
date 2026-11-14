//! Artistic effects: linocut, pointillism, stained glass, mosaic.

use super::math::{average_rect, box_blur, hash01, luma_plane, mix, smooth_step};
use super::{rows, Effect};
use crate::color::Bgra;
use crate::rect::PixelRect;
use crate::surface::Surface;

#[inline]
fn scale(c: Bgra, k: f32) -> Bgra {
    let s = |v: u8| (v as f32 * k).clamp(0.0, 255.0) as u8;
    Bgra::new(s(c.b), s(c.g), s(c.r), c.a)
}

pub struct Linocut {
    pub ink: Bgra,
    pub paper: Bgra,
    pub threshold_percent: i32,
    pub detail_radius: i32,
    pub edge_percent: i32,
    pub line_spacing: i32,
}

impl Effect for Linocut {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let (w, h) = (src.width() as usize, src.height() as usize);
        let luma = luma_plane(src);
        let tone = box_blur(&luma, w, h, self.detail_radius.max(1));
        let coarse = box_blur(&luma, w, h, (self.detail_radius * 3).max(2));
        let t = (self.threshold_percent as f32 / 100.0).clamp(0.01, 0.99);
        let edge_k = 0.01f32 + (1.0 - (self.edge_percent as f32 / 100.0).clamp(0.0, 1.0)) * 0.15;
        let spacing = self.line_spacing.max(2) as f32;
        let aa = 1.5f32 / spacing;
        rows(dst, roi, |y, drow| {
            let srow = src.row(y);
            for x in roi.x..roi.right() {
                let i = y as usize * w + x as usize;
                let tn = tone[i];
                let phase = (y as f32 + 2.5 * (x as f32 * 0.045 + coarse[i] * 7.0).sin()) / spacing;
                let dist = (phase - phase.floor() - 0.5).abs() * 2.0;
                let cover = smooth_step(t + 0.3, t - 0.3, tn);
                let hatch = if cover <= 0.0 { 0.0 } else { smooth_step(cover + aa, cover - aa, dist) * 1f32.min(cover * 8.0) };
                let high = luma[i] - tn;
                let edge = smooth_step(-edge_k * 0.5, -edge_k * 1.5, high);
                let amount = hatch.max(edge);
                let c = mix(self.paper, self.ink, amount);
                drow[x as usize] = c.with_alpha((c.a as i32 * srow[x as usize].a as i32 / 255) as u8);
            }
        });
    }
}

pub struct Pointillism {
    pub dot_size: i32,
    pub density_percent: i32,
    pub color_jitter_percent: i32,
    pub seed: i32,
    pub paper: Bgra,
}

#[derive(Clone, Copy)]
struct Dot {
    x: f32,
    y: f32,
    r: f32,
    color: Bgra,
    order: f32,
}

impl Effect for Pointillism {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let seed = self.seed;
        let base_r = 1f32.max(self.dot_size as f32 / 2.0);
        let spacing = 1f32.max(base_r * 1.6 / (self.density_percent.clamp(10, 400) as f32 / 100.0).sqrt());
        let jitter = self.color_jitter_percent.clamp(0, 100) as f32 / 100.0 * 0.35;
        let cols = (src.width() as f32 / spacing).ceil() as i32 + 2;
        let rows_n = (src.height() as f32 / spacing).ceil() as i32 + 2;
        let mut dots = vec![Dot { x: 0.0, y: 0.0, r: 0.0, color: Bgra::TRANSPARENT, order: 0.0 }; (cols * rows_n) as usize];
        crate::par::for_rows(&mut dots, cols as usize, |gy, row| {
            let gy = gy as i32;
            for (gx, d) in row.iter_mut().enumerate() {
                let gx = gx as i32;
                let cx = (gx as f32 - 0.5 + hash01(gx, gy, seed)) * spacing;
                let cy = (gy as f32 - 0.5 + hash01(gx, gy, seed + 1)) * spacing;
                let r = base_r * (0.75 + 0.4 * hash01(gx, gy, seed + 2));
                let sx = (cx as i32).clamp(0, src.width() - 1);
                let sy = (cy as i32).clamp(0, src.height() - 1);
                let k = 1.0 + (hash01(gx, gy, seed + 3) - 0.5) * 2.0 * jitter;
                *d = Dot { x: cx, y: cy, r, color: scale(src.get(sx, sy), k), order: hash01(gx, gy, seed + 4) };
            }
        });
        let reach = (base_r * 1.15 / spacing).ceil() as i32 + 1;
        let paper = self.paper;
        rows(dst, roi, |y, drow| {
            let mut hits: Vec<usize> = Vec::with_capacity(((2 * reach + 1) * (2 * reach + 1)) as usize);
            let gy0 = (y as f32 / spacing + 0.5).floor() as i32;
            for x in roi.x..roi.right() {
                let gx0 = (x as f32 / spacing + 0.5).floor() as i32;
                hits.clear();
                for gy in gy0 - reach..=gy0 + reach {
                    if (gy as u32) >= rows_n as u32 {
                        continue;
                    }
                    for gx in gx0 - reach..=gx0 + reach {
                        if (gx as u32) >= cols as u32 {
                            continue;
                        }
                        let idx = (gy * cols + gx) as usize;
                        let d = &dots[idx];
                        let dx = x as f32 + 0.5 - d.x;
                        let dy = y as f32 + 0.5 - d.y;
                        if dx * dx + dy * dy < (d.r + 1.0) * (d.r + 1.0) {
                            hits.push(idx);
                        }
                    }
                }
                // Painter's order (stable insertion sort, as in C#).
                for a in 1..hits.len() {
                    let v = hits[a];
                    let mut b = a as isize - 1;
                    while b >= 0 && dots[hits[b as usize]].order > dots[v].order {
                        hits[b as usize + 1] = hits[b as usize];
                        b -= 1;
                    }
                    hits[(b + 1) as usize] = v;
                }
                let mut c = paper;
                for &k in &hits {
                    let d = &dots[k];
                    let ex = x as f32 + 0.5 - d.x;
                    let ey = y as f32 + 0.5 - d.y;
                    let dist = (ex * ex + ey * ey).sqrt();
                    let cov = (d.r + 0.5 - dist).clamp(0.0, 1.0) * d.color.a as f32 / 255.0;
                    if cov > 0.0 {
                        c = mix(c, d.color.with_alpha(255), cov);
                    }
                }
                drow[x as usize] = c;
            }
        });
    }
}

pub struct StainedGlass {
    pub cell_size: i32,
    pub edge_width: i32,
    pub randomness_percent: i32,
    pub seed: i32,
    pub shading: bool,
    pub leading: Bgra,
}

impl Effect for StainedGlass {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let (w, h) = (src.width(), src.height());
        let size = self.cell_size.max(3);
        let sizef = size as f32;
        let rnd = self.randomness_percent.clamp(0, 100) as f32 / 100.0;
        let cols = w / size + 2;
        let rows_n = h / size + 2;
        let n = (cols * rows_n) as usize;
        let mut sx = vec![0f32; n];
        let mut sy = vec![0f32; n];
        for gy in 0..rows_n {
            for gx in 0..cols {
                let i = (gy * cols + gx) as usize;
                sx[i] = (gx as f32 + 0.5 + (hash01(gx, gy, self.seed) - 0.5) * rnd) * sizef;
                sy[i] = (gy as f32 + 0.5 + (hash01(gx, gy, self.seed + 7) - 0.5) * rnd) * sizef;
            }
        }
        // Pass 1: nearest and second-nearest seed per pixel.
        let mut cell = vec![(0i32, 0f32); (w * h) as usize];
        crate::par::for_rows(&mut cell, w as usize, |y, row| {
            for (x, out) in row.iter_mut().enumerate() {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let gx0 = (px / sizef) as i32;
                let gy0 = (py / sizef) as i32;
                let (mut best, mut second) = (-1i32, -1i32);
                let (mut bd, mut sd) = (f32::MAX, f32::MAX);
                for gy in gy0 - 2..=gy0 + 2 {
                    if (gy as u32) >= rows_n as u32 {
                        continue;
                    }
                    for gx in gx0 - 2..=gx0 + 2 {
                        if (gx as u32) >= cols as u32 {
                            continue;
                        }
                        let i = (gy * cols + gx) as usize;
                        let dx = px - sx[i];
                        let dy = py - sy[i];
                        let d = dx * dx + dy * dy;
                        if d < bd {
                            sd = bd;
                            second = best;
                            bd = d;
                            best = i as i32;
                        } else if d < sd {
                            sd = d;
                            second = i as i32;
                        }
                    }
                }
                let ed = if second < 0 {
                    f32::MAX
                } else {
                    let (b, s) = (best as usize, second as usize);
                    let ex = sx[s] - sx[b];
                    let ey = sy[s] - sy[b];
                    let len = (ex * ex + ey * ey).sqrt();
                    if len <= 0.0 { 0.0 } else { (sd - bd) / (2.0 * len) }
                };
                *out = (best, ed);
            }
        });
        let mut sum = vec![[0i64; 4]; n];
        let mut cnt = vec![0i64; n];
        for (&c, &(l, _)) in src.pixels().iter().zip(&cell) {
            let a = c.a as i64;
            let s = &mut sum[l as usize];
            s[0] += c.b as i64 * a;
            s[1] += c.g as i64 * a;
            s[2] += c.r as i64 * a;
            s[3] += a;
            cnt[l as usize] += 1;
        }
        let avg: Vec<Bgra> = (0..n)
            .map(|i| {
                let a = sum[i][3];
                if cnt[i] == 0 || a == 0 {
                    Bgra::TRANSPARENT
                } else {
                    Bgra::new((sum[i][0] / a) as u8, (sum[i][1] / a) as u8, (sum[i][2] / a) as u8, (a / cnt[i]) as u8)
                }
            })
            .collect();
        let half = self.edge_width as f32 / 2.0;
        let glow = sizef * 0.3;
        rows(dst, roi, |y, drow| {
            for x in roi.x..roi.right() {
                let (l, ed) = cell[(y * w + x) as usize];
                let l = l as usize;
                let mut c = avg[l];
                if self.shading && ed < glow {
                    let dx = x as f32 + 0.5 - sx[l];
                    let dy = y as f32 + 0.5 - sy[l];
                    let len = 1e-3f32.max((dx * dx + dy * dy).sqrt());
                    let facing = -(dx + dy) / (len * 1.4142);
                    let k = 1.0 + 0.35 * facing * (1.0 - ed / glow);
                    c = scale(c, k);
                }
                if self.edge_width > 0 {
                    let lead = (half + 0.5 - ed).clamp(0.0, 1.0);
                    if lead > 0.0 {
                        c = mix(c, self.leading, lead);
                    }
                }
                drow[x as usize] = c;
            }
        });
    }
}

pub struct Mosaic {
    pub tile_size: i32,
    pub grout_width: i32,
    pub bevel_width: i32,
    pub bevel_strength_percent: i32,
    pub grout: Bgra,
}

impl Effect for Mosaic {
    fn render(&self, src: &Surface, dst: &mut Surface, roi: PixelRect) {
        let size = self.tile_size.max(2);
        let g = self.grout_width.clamp(0, size - 1);
        let cols = (src.width() + size - 1) / size;
        let rows_n = (src.height() + size - 1) / size;
        let mut avg = vec![Bgra::TRANSPARENT; (cols * rows_n) as usize];
        crate::par::for_rows(&mut avg, cols as usize, |ty, row| {
            let ty = ty as i32;
            for (tx, a) in row.iter_mut().enumerate() {
                let tx = tx as i32;
                *a = average_rect(src, tx * size, ty * size, (tx + 1) * size, (ty + 1) * size);
            }
        });
        let g_lead = g as f32 / 2.0;
        let g_trail = g as f32 - g_lead;
        let bevel = self.bevel_width.max(0);
        let strength = self.bevel_strength_percent.clamp(0, 100) as f32 / 100.0 * 0.6;
        let sizef = size as f32;
        rows(dst, roi, |y, drow| {
            let ty = y / size;
            let ly = (y - ty * size) as f32 + 0.5;
            for x in roi.x..roi.right() {
                let tx = x / size;
                let lx = (x - tx * size) as f32 + 0.5;
                let mut c = avg[(ty * cols + tx) as usize];
                let dl = lx - g_lead;
                let dt = ly - g_lead;
                let dr = sizef - g_trail - lx;
                let db = sizef - g_trail - ly;
                let inside = dl.min(dr).min(dt.min(db));
                if bevel > 0 && inside < bevel as f32 && inside > -0.5 {
                    let t = 1.0 - (inside / bevel as f32).clamp(0.0, 1.0);
                    let lit = dl.min(dt) <= dr.min(db);
                    let k = 1.0 + (if lit { strength } else { -strength }) * t;
                    c = scale(c, k);
                }
                if g > 0 {
                    let gc = (0.5 - inside).clamp(0.0, 1.0);
                    if gc > 0.0 {
                        c = mix(c, self.grout, gc);
                    }
                }
                drow[x as usize] = c;
            }
        });
    }
}
