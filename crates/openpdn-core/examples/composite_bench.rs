//! Times compositing strategies on a 12 MP, 4-layer document.
use openpdn_core::{BlendMode, Bgra, Document, Layer, Surface};
use std::time::Instant;

fn noise(l: &mut Layer, mut s: u32, opaque: bool) {
    for p in l.surface.pixels_mut() {
        s ^= s << 13; s ^= s >> 17; s ^= s << 5;
        *p = Bgra::new(s as u8, (s >> 8) as u8, (s >> 16) as u8, if opaque { 255 } else { (s >> 24) as u8 });
    }
}

fn main() {
    let (w, h) = (4000, 3000);
    let mut d = Document::new(w, h);
    for (i, (mode, op, opaque)) in [(BlendMode::Normal, 255, false), (BlendMode::Multiply, 255, false), (BlendMode::Overlay, 128, false), (BlendMode::Normal, 60, true)].into_iter().enumerate() {
        let mut l = Layer::new(w, h, "l");
        noise(&mut l, i as u32 * 7 + 1, opaque);
        l.blend_mode = mode;
        l.opacity = op;
        d.layers.push(l);
    }
    let mut out = Surface::new(w, h);
    for _ in 0..2 { d.composite_into(&mut out); }
    let t = Instant::now();
    for _ in 0..5 { d.composite_into(&mut out); }
    println!("banded:        {:.1} ms", t.elapsed().as_secs_f64() * 200.0);
    let t = Instant::now();
    for _ in 0..5 {
        out.clear(Bgra::TRANSPARENT);
        for l in &d.layers { openpdn_core::blend::apply_layer(&mut out, &l.surface, l.blend_mode, l.opacity); }
    }
    println!("layer passes:  {:.1} ms", t.elapsed().as_secs_f64() * 200.0);
    for l in &d.layers {
        let t = Instant::now();
        for _ in 0..5 { openpdn_core::blend::apply_layer(&mut out, &l.surface, l.blend_mode, l.opacity); }
        println!("  {:?}@{}: {:.1} ms", l.blend_mode, l.opacity, t.elapsed().as_secs_f64() * 200.0);
    }
}
