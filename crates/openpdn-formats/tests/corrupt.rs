//! Decoders must reject corrupt input with an error, never panic.

use openpdn_core::{Bgra, Document, Layer, Surface};
use openpdn_formats::{openpdn, pdn, png};
use std::panic::catch_unwind;

fn rng(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn sample_doc() -> Document {
    let mut s = Surface::new(40, 30);
    for (i, p) in s.pixels_mut().iter_mut().enumerate() {
        *p = Bgra::new(i as u8, (i * 3) as u8, (i * 7) as u8, (i * 11) as u8);
    }
    let mut d = Document::from_single_layer(s, "a");
    d.layers.push(Layer::new(40, 30, "b"));
    d
}

fn mutate(base: &[u8], seed: u64) -> Vec<u8> {
    let mut s = seed | 1;
    let mut v = base.to_vec();
    match rng(&mut s) % 4 {
        0 => {
            let n = (rng(&mut s) % 8 + 1) as usize;
            for _ in 0..n {
                let i = (rng(&mut s) as usize) % v.len();
                v[i] = rng(&mut s) as u8;
            }
        }
        1 => v.truncate((rng(&mut s) as usize) % v.len()),
        2 => {
            let i = (rng(&mut s) as usize) % v.len();
            let n = ((rng(&mut s) % 64) as usize).min(v.len() - i);
            v.drain(i..i + n);
        }
        _ => {
            // Big-endian/little-endian length fields set to huge values.
            let i = (rng(&mut s) as usize) % v.len().saturating_sub(4).max(1);
            let end = (i + 4).min(v.len());
            v[i..end].fill(0xFF);
        }
    }
    v
}

fn fuzz(name: &str, base: &[u8], f: impl Fn(&[u8]) + std::panic::RefUnwindSafe) {
    for seed in 1..3000u64 {
        let m = mutate(base, seed * 0x9E3779B97F4A7C15);
        if catch_unwind(|| f(&m)).is_err() {
            panic!("{name}: decoder panicked on mutation seed {seed}");
        }
    }
}

#[test]
fn png_never_panics() {
    let d = sample_doc();
    let base = png::encode(&d.layers[0].surface, &[], png::Compression::Fast);
    fuzz("png", &base, |b| {
        let _ = png::decode(b);
        let _ = png::read_text_chunks(b);
    });
}

#[test]
fn openpdn_never_panics() {
    let base = openpdn::write(&sample_doc()).unwrap();
    fuzz("openpdn", &base, |b| {
        let _ = openpdn::read(b);
    });
}

#[test]
fn pdn_never_panics() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../parity/fixtures/pdn");
    let Ok(entries) = std::fs::read_dir(dir) else {
        eprintln!("skipping: run parity/fixtures/make-pdn.py first");
        return;
    };
    for e in entries.flatten() {
        let base = std::fs::read(e.path()).unwrap();
        fuzz(&e.file_name().to_string_lossy(), &base, |b| {
            let _ = pdn::read(b);
        });
    }
}
