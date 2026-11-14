//! PNG decoding and encoding, with tEXt / zTXt / iTXt metadata.

use crate::checksum::{adler32, adler32_combine, crc32_update};
use crate::{err, Result};
use miniz_oxide::deflate::core::{compress, create_comp_flags_from_zip_params, CompressorOxide, TDEFLFlush, TDEFLStatus};
use openpdn_core::document::{PngTextChunk, PngTextKind};
use openpdn_core::{Bgra, Surface};

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

pub fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && data[..8] == SIGNATURE
}

/// Iterates `(type, data)` for each well-formed chunk up to IEND.
pub fn chunks(png: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = if is_png(png) { 8 } else { png.len() };
    let mut done = false;
    std::iter::from_fn(move || {
        if done || pos + 12 > png.len() {
            return None;
        }
        let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
        if pos + 12 + len > png.len() {
            return None;
        }
        let ty: [u8; 4] = png[pos + 4..pos + 8].try_into().unwrap();
        let data = &png[pos + 8..pos + 8 + len];
        pos += 12 + len;
        if &ty == b"IEND" {
            done = true;
        }
        Some((ty, data))
    })
}

fn write_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32_update(crc32_update(0, ty), data).to_be_bytes());
}

// ------------------------------------------------------------------ text

fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

fn inflate_zlib(data: &[u8]) -> Option<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib(data).ok()
}

fn deflate_zlib(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec_zlib(data, 9)
}

fn parse_text_chunk(ty: &[u8; 4], d: &[u8]) -> Option<PngTextChunk> {
    let nul = d.iter().position(|&b| b == 0)?;
    if nul == 0 {
        return None;
    }
    let keyword = latin1(&d[..nul]);
    let mk = |text: String, kind, lang: String, trans: String, compressed| PngTextChunk {
        keyword: keyword.clone(),
        text,
        kind,
        language_tag: lang,
        translated_keyword: trans,
        compressed,
    };
    match ty {
        b"tEXt" => Some(mk(latin1(&d[nul + 1..]), PngTextKind::Text, String::new(), String::new(), false)),
        b"zTXt" => {
            if nul + 2 > d.len() || d[nul + 1] != 0 {
                return None;
            }
            let text = latin1(&inflate_zlib(&d[nul + 2..])?);
            Some(mk(text, PngTextKind::CompressedText, String::new(), String::new(), true))
        }
        b"iTXt" => {
            if nul + 3 > d.len() {
                return None;
            }
            let compressed = d[nul + 1] == 1;
            let lang_end = nul + 3 + d[nul + 3..].iter().position(|&b| b == 0)?;
            let trans_end = lang_end + 1 + d[lang_end + 1..].iter().position(|&b| b == 0)?;
            let lang = String::from_utf8_lossy(&d[nul + 3..lang_end]).into_owned();
            let trans = String::from_utf8_lossy(&d[lang_end + 1..trans_end]).into_owned();
            let body = &d[trans_end + 1..];
            let text = if compressed {
                String::from_utf8_lossy(&inflate_zlib(body)?).into_owned()
            } else {
                String::from_utf8_lossy(body).into_owned()
            };
            Some(mk(text, PngTextKind::InternationalText, lang, trans, compressed))
        }
        _ => None,
    }
}

/// All text chunks of a PNG, skipping malformed ones.
pub fn read_text_chunks(png: &[u8]) -> Vec<PngTextChunk> {
    chunks(png).filter_map(|(ty, d)| parse_text_chunk(&ty, d)).collect()
}

fn sanitize_keyword(k: &str) -> String {
    let kept: String = k.chars().filter(|&c| matches!(c as u32, 32..=126 | 161..=255)).collect();
    let mut cleaned = kept.split(' ').filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        cleaned = "Comment".into();
    }
    cleaned.chars().take(79).collect()
}

fn to_latin1(s: &str) -> Option<Vec<u8>> {
    s.chars().map(|c| u8::try_from(c as u32).ok()).collect()
}

/// Serialises one text chunk, choosing iTXt when the text is not Latin-1.
pub fn write_text_chunk(out: &mut Vec<u8>, t: &PngTextChunk) {
    let keyword = to_latin1(&sanitize_keyword(&t.keyword)).unwrap();
    match (t.kind, to_latin1(&t.text)) {
        (PngTextKind::Text, Some(text)) => {
            write_chunk(out, b"tEXt", &[keyword, vec![0], text].concat());
        }
        (PngTextKind::CompressedText, Some(text)) => {
            write_chunk(out, b"zTXt", &[keyword, vec![0, 0], deflate_zlib(&text)].concat());
        }
        _ => {
            let utf8 = t.text.as_bytes();
            let body = if t.compressed { deflate_zlib(utf8) } else { utf8.to_vec() };
            let data = [
                keyword,
                vec![0, t.compressed as u8, 0],
                t.language_tag.bytes().filter(|b| b.is_ascii()).collect(),
                vec![0],
                t.translated_keyword.as_bytes().to_vec(),
                vec![0],
                body,
            ]
            .concat();
            write_chunk(out, b"iTXt", &data);
        }
    }
}

/// Inserts text chunks right after IHDR of an existing PNG (e.g. one the browser encoded).
pub fn insert_text_chunks(png: &[u8], text: &[PngTextChunk]) -> Vec<u8> {
    if text.is_empty() || !is_png(png) || png.len() < 33 {
        return png.to_vec();
    }
    let ihdr_len = u32::from_be_bytes(png[8..12].try_into().unwrap()) as usize;
    let after = 8 + 12 + ihdr_len;
    let mut out = Vec::with_capacity(png.len() + 1024);
    out.extend_from_slice(&png[..after]);
    for t in text {
        write_text_chunk(&mut out, t);
    }
    out.extend_from_slice(&png[after..]);
    out
}

// ------------------------------------------------------------------ decode

pub struct DecodedPng {
    pub surface: Surface,
    pub text: Vec<PngTextChunk>,
}

struct Header {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    interlace: bool,
}

impl Header {
    fn channels(&self) -> usize {
        match self.color {
            0 | 3 => 1,
            4 => 2,
            2 => 3,
            _ => 4,
        }
    }
    /// Bytes per complete pixel, at least 1 (the filter unit).
    fn bpp(&self) -> usize {
        ((self.channels() * self.depth as usize).div_ceil(8)).max(1)
    }
    fn row_bytes(&self, w: u32) -> usize {
        (w as usize * self.channels() * self.depth as usize).div_ceil(8)
    }
}

#[inline]
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

fn unfilter(ft: u8, row: &mut [u8], prev: &[u8], bpp: usize) -> Result<()> {
    match ft {
        0 => {}
        1 => {
            for i in bpp..row.len() {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for (r, &p) in row.iter_mut().zip(prev) {
                *r = r.wrapping_add(p);
            }
        }
        3 => {
            for i in 0..row.len() {
                let left = if i >= bpp { row[i - bpp] as u16 } else { 0 };
                row[i] = row[i].wrapping_add(((left + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..row.len() {
                let (a, c) = if i >= bpp { (row[i - bpp], prev[i - bpp]) } else { (0, 0) };
                row[i] = row[i].wrapping_add(paeth(a, prev[i], c));
            }
        }
        _ => return err(format!("bad PNG filter type {ft}")),
    }
    Ok(())
}

/// 16-bit → 8-bit by keeping the high byte, as Skia (the C# port's decoder) does.
#[inline]
fn scale16(v: u16) -> u8 {
    (v >> 8) as u8
}

struct Converter<'a> {
    h: &'a Header,
    palette: Vec<Bgra>,
    trns_gray: Option<u16>,
    trns_rgb: Option<(u16, u16, u16)>,
}

impl Converter<'_> {
    /// Converts one unfiltered row of `w` pixels; calls `put(i, px)`.
    fn row(&self, raw: &[u8], w: usize, mut put: impl FnMut(usize, Bgra)) {
        let d = self.h.depth;
        let sample = |i: usize| -> u16 {
            // i-th sample in the row, for sub-byte depths.
            let bit = i * d as usize;
            let byte = raw[bit / 8];
            let shift = 8 - d as usize - bit % 8;
            ((byte >> shift) as u16) & ((1u16 << d) - 1)
        };
        let s16 = |i: usize| u16::from_be_bytes([raw[2 * i], raw[2 * i + 1]]);
        match (self.h.color, d) {
            (0, 16) => (0..w).for_each(|x| {
                let v = s16(x);
                let a = if self.trns_gray == Some(v) { 0 } else { 255 };
                let g = scale16(v);
                put(x, Bgra::new(g, g, g, a))
            }),
            (0, _) => {
                let max = (1u16 << d) - 1;
                (0..w).for_each(|x| {
                    let v = if d == 8 { raw[x] as u16 } else { sample(x) };
                    let a = if self.trns_gray == Some(v) { 0 } else { 255 };
                    let g = (v as u32 * 255 / max as u32) as u8;
                    put(x, Bgra::new(g, g, g, a))
                })
            }
            (2, 8) => (0..w).for_each(|x| {
                let (r, g, b) = (raw[3 * x], raw[3 * x + 1], raw[3 * x + 2]);
                let a = if self.trns_rgb == Some((r as u16, g as u16, b as u16)) { 0 } else { 255 };
                put(x, Bgra::new(b, g, r, a))
            }),
            (2, _) => (0..w).for_each(|x| {
                let (r, g, b) = (s16(3 * x), s16(3 * x + 1), s16(3 * x + 2));
                let a = if self.trns_rgb == Some((r, g, b)) { 0 } else { 255 };
                put(x, Bgra::new(scale16(b), scale16(g), scale16(r), a))
            }),
            (3, _) => (0..w).for_each(|x| {
                let i = if d == 8 { raw[x] as usize } else { sample(x) as usize };
                put(x, self.palette.get(i).copied().unwrap_or(Bgra::BLACK))
            }),
            (4, 8) => (0..w).for_each(|x| {
                let g = raw[2 * x];
                put(x, Bgra::new(g, g, g, raw[2 * x + 1]))
            }),
            (4, _) => (0..w).for_each(|x| {
                let g = scale16(s16(2 * x));
                put(x, Bgra::new(g, g, g, scale16(s16(2 * x + 1))))
            }),
            (6, 8) => (0..w).for_each(|x| {
                let p = &raw[4 * x..4 * x + 4];
                put(x, Bgra::new(p[2], p[1], p[0], p[3]))
            }),
            _ => (0..w).for_each(|x| {
                put(x, Bgra::new(scale16(s16(4 * x + 2)), scale16(s16(4 * x + 1)), scale16(s16(4 * x)), scale16(s16(4 * x + 3))))
            }),
        }
    }
}

/// Decodes a PNG to straight-alpha BGRA8 plus its text chunks.
pub fn decode(png: &[u8]) -> Result<DecodedPng> {
    if !is_png(png) {
        return err("not a PNG file");
    }
    let mut header = None;
    let mut palette = Vec::new();
    let mut trns: Option<&[u8]> = None;
    let mut idat = Vec::new();
    let mut text = Vec::new();
    let mut ended = false;
    for (ty, d) in chunks(png) {
        match &ty {
            b"IHDR" => {
                if d.len() != 13 {
                    return err("bad IHDR");
                }
                header = Some(Header {
                    width: u32::from_be_bytes(d[0..4].try_into().unwrap()),
                    height: u32::from_be_bytes(d[4..8].try_into().unwrap()),
                    depth: d[8],
                    color: d[9],
                    interlace: d[12] == 1,
                });
            }
            b"PLTE" => {
                palette = d.chunks_exact(3).map(|c| Bgra::new(c[2], c[1], c[0], 255)).collect();
            }
            b"tRNS" => trns = Some(d),
            b"IDAT" => idat.extend_from_slice(d),
            b"IEND" => ended = true,
            _ => {
                if let Some(t) = parse_text_chunk(&ty, d) {
                    text.push(t);
                }
            }
        }
    }
    let h = header.ok_or_else(|| crate::FormatError("missing IHDR".into()))?;
    let valid = matches!(
        (h.color, h.depth),
        (0, 1 | 2 | 4 | 8 | 16) | (2, 8 | 16) | (3, 1 | 2 | 4 | 8) | (4, 8 | 16) | (6, 8 | 16)
    );
    if !valid {
        return err("unsupported PNG header");
    }
    crate::check_size(h.width as i64, h.height as i64)?;
    let _ = ended; // truncated files decode as far as their data goes
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&idat)
        .or_else(|e| if e.output.is_empty() { Err(()) } else { Ok(e.output) })
        .map_err(|_| crate::FormatError("corrupt PNG image data".into()))?;
    let mut conv = Converter { h: &h, palette, trns_gray: None, trns_rgb: None };
    if let Some(t) = trns {
        match h.color {
            0 if t.len() >= 2 => conv.trns_gray = Some(u16::from_be_bytes([t[0], t[1]])),
            2 if t.len() >= 6 => {
                conv.trns_rgb = Some((
                    u16::from_be_bytes([t[0], t[1]]),
                    u16::from_be_bytes([t[2], t[3]]),
                    u16::from_be_bytes([t[4], t[5]]),
                ))
            }
            3 => {
                for (p, &a) in conv.palette.iter_mut().zip(t) {
                    p.a = a;
                }
            }
            _ => {}
        }
    }
    let (w, ht) = (h.width as usize, h.height as usize);
    let mut surface = Surface::new(w as i32, ht as i32);
    let bpp = h.bpp();
    let passes: &[(usize, usize, usize, usize)] = if h.interlace {
        &[(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]
    } else {
        &[(0, 0, 1, 1)]
    };
    let mut pos = 0usize;
    let px = surface.pixels_mut();
    for &(x0, y0, dx, dy) in passes {
        if x0 >= w || y0 >= ht {
            continue;
        }
        let pw = (w - x0).div_ceil(dx);
        let ph = (ht - y0).div_ceil(dy);
        let rb = h.row_bytes(pw as u32);
        let mut prev = vec![0u8; rb];
        let mut cur = vec![0u8; rb];
        for r in 0..ph {
            if pos + 1 + rb > raw.len() {
                // Truncated: leave the rest transparent.
                return Ok(DecodedPng { surface, text });
            }
            let ft = raw[pos];
            cur.copy_from_slice(&raw[pos + 1..pos + 1 + rb]);
            pos += 1 + rb;
            unfilter(ft, &mut cur, &prev, bpp)?;
            let y = y0 + r * dy;
            conv.row(&cur, pw, |i, c| px[y * w + x0 + i * dx] = c);
            std::mem::swap(&mut prev, &mut cur);
        }
    }
    Ok(DecodedPng { surface, text })
}

// ------------------------------------------------------------------ encode

/// Raw-deflates `data`; non-final pieces end with a sync flush so pieces
/// compressed independently can be concatenated into one stream.
fn deflate_piece(data: &[u8], last: bool, level: u8) -> Vec<u8> {
    let flags = create_comp_flags_from_zip_params(level as i32, -15, 0);
    let mut c = CompressorOxide::new(flags);
    let mut out = vec![0u8; data.len() / 2 + 1024];
    let mut out_pos = 0;
    let mut in_pos = 0;
    let flush = if last { TDEFLFlush::Finish } else { TDEFLFlush::Sync };
    loop {
        let (status, n_in, n_out) = compress(&mut c, &data[in_pos..], &mut out[out_pos..], flush);
        in_pos += n_in;
        out_pos += n_out;
        match status {
            TDEFLStatus::Done => break,
            TDEFLStatus::Okay if in_pos >= data.len() && out_pos < out.len() => break,
            TDEFLStatus::Okay => {
                let n = out.len();
                out.resize(n * 2, 0);
            }
            _ => panic!("deflate failed"),
        }
    }
    out.truncate(out_pos);
    out
}

/// "Sub" filter + BGRA→RGBA for one row.
fn filter_sub_row(src: &[Bgra], dst: &mut [u8]) {
    dst[0] = 1;
    let o = &mut dst[1..];
    let mut prev = [0u8; 4];
    for (p, d) in src.iter().zip(o.chunks_exact_mut(4)) {
        let cur = [p.r, p.g, p.b, p.a];
        for k in 0..4 {
            d[k] = cur[k].wrapping_sub(prev[k]);
        }
        prev = cur;
    }
}

/// Encoding speed / size trade-off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    /// Fast (deflate level 1), the default; what the C# FastPngEncoder does.
    Fast,
    /// Smaller files (level 6).
    Default,
    /// Smallest (level 9), slowest.
    Best,
}

/// Encodes RGBA8 PNG in parallel strips (pigz-style), with text chunks after IHDR.
pub fn encode(s: &Surface, text: &[PngTextChunk], compression: Compression) -> Vec<u8> {
    let level = match compression {
        Compression::Fast => 1,
        Compression::Default => 6,
        Compression::Best => 9,
    };
    let (w, h) = (s.width() as usize, s.height() as usize);
    let row_bytes = 1 + w * 4;
    let threads = {
        #[cfg(feature = "parallel")]
        {
            rayon::current_num_threads()
        }
        #[cfg(not(feature = "parallel"))]
        {
            1
        }
    };
    let rows_per_strip = 32.max(h.div_ceil(threads * 4));
    let strips = h.div_ceil(rows_per_strip);
    let work = |i: usize| -> (Vec<u8>, u32, u64) {
        let y0 = i * rows_per_strip;
        let y1 = h.min(y0 + rows_per_strip);
        let mut raw = vec![0u8; (y1 - y0) * row_bytes];
        for y in y0..y1 {
            filter_sub_row(s.row(y as i32), &mut raw[(y - y0) * row_bytes..(y - y0 + 1) * row_bytes]);
        }
        (deflate_piece(&raw, i == strips - 1, level), adler32(&raw), raw.len() as u64)
    };
    #[cfg(feature = "parallel")]
    let pieces: Vec<(Vec<u8>, u32, u64)> = {
        use rayon::prelude::*;
        (0..strips).into_par_iter().map(work).collect()
    };
    #[cfg(not(feature = "parallel"))]
    let pieces: Vec<(Vec<u8>, u32, u64)> = (0..strips).map(work).collect();

    let mut adler = pieces[0].1;
    for p in &pieces[1..] {
        adler = adler32_combine(adler, p.1, p.2);
    }
    let total: usize = pieces.iter().map(|p| p.0.len() + 12).sum::<usize>() + 64;
    let mut out = Vec::with_capacity(total + 1024);
    out.extend_from_slice(&SIGNATURE);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&(w as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(h as u32).to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    write_chunk(&mut out, b"IHDR", &ihdr);
    for t in text {
        write_text_chunk(&mut out, t);
    }
    let n = pieces.len();
    for (i, (data, _, _)) in pieces.iter().enumerate() {
        let mut chunk = Vec::with_capacity(data.len() + 6);
        if i == 0 {
            chunk.extend_from_slice(&[0x78, 0x01]);
        }
        chunk.extend_from_slice(data);
        if i == n - 1 {
            chunk.extend_from_slice(&adler.to_be_bytes());
        }
        write_chunk(&mut out, b"IDAT", &chunk);
    }
    write_chunk(&mut out, b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(w: i32, h: i32) -> Surface {
        let mut s = Surface::new(w, h);
        let mut v = 12345u32;
        for p in s.pixels_mut() {
            v ^= v << 13;
            v ^= v >> 17;
            v ^= v << 5;
            *p = Bgra::new(v as u8, (v >> 8) as u8 & 0xF0, (v >> 16) as u8, (v >> 24) as u8);
        }
        s
    }

    #[test]
    fn round_trip_with_text() {
        let s = sample(300, 517);
        let text = vec![
            PngTextChunk {
                keyword: "Title".into(),
                text: "Hello".into(),
                kind: PngTextKind::Text,
                language_tag: String::new(),
                translated_keyword: String::new(),
                compressed: false,
            },
            PngTextChunk {
                keyword: "Comment".into(),
                text: "Ünïcödé ✓".into(),
                kind: PngTextKind::InternationalText,
                language_tag: "en".into(),
                translated_keyword: "Kommentar".into(),
                compressed: true,
            },
        ];
        for c in [Compression::Fast, Compression::Best] {
            let png = encode(&s, &text, c);
            let d = decode(&png).unwrap();
            assert_eq!(d.surface, s);
            assert_eq!(d.text, text);
        }
    }

    #[test]
    fn tiny_images() {
        for (w, h) in [(1, 1), (1, 40), (40, 1), (3, 100)] {
            let s = sample(w, h);
            assert_eq!(decode(&encode(&s, &[], Compression::Fast)).unwrap().surface, s);
        }
    }
}
