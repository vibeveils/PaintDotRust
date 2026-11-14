//! Legacy Paint.NET (2.1 – 3.x) `.pdn` reading.
//!
//! The file is `PDN3`, a 24-bit header length, an XML header, then a .NET
//! BinaryFormatter (MS-NRBF) object graph — either raw (`00 01`) followed by
//! "deferred" pixel blocks, or gzip-compressed with pixels inline.

use crate::{err, FormatError, Result};
use miniz_oxide::inflate::decompress_to_vec;
use openpdn_core::{BlendMode, Document, Layer, Surface};
use std::collections::HashMap;

// ------------------------------------------------------------------ NRBF

#[derive(Debug, Clone)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    /// Reference to a record by object id.
    Ref(i32),
    /// Inline object (class or array) by object id.
    Obj(i32),
}

#[derive(Debug)]
pub enum Record {
    Class { name: String, members: Vec<(String, Value)> },
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Str(String),
}

struct ClassMeta {
    name: String,
    members: Vec<String>,
    /// Per member: Some(primitive type) when values are raw primitives.
    prim: Option<Vec<Option<u8>>>,
}

struct Reader<'a> {
    d: &'a [u8],
    p: usize,
    meta: HashMap<i32, std::rc::Rc<ClassMeta>>,
    objects: HashMap<i32, Record>,
    depth: u32,
}

fn eof<T>() -> Result<T> {
    err("truncated .pdn object graph")
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.d.get(self.p).ok_or(()).or_else(|_| eof())?;
        self.p += 1;
        Ok(v)
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.p + n > self.d.len() {
            return eof();
        }
        let s = &self.d[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let mut len = 0usize;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            len |= ((b & 0x7F) as usize) << shift;
            if b & 0x80 == 0 {
                break;
            }
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    fn primitive(&mut self, t: u8) -> Result<Value> {
        let mut le = |n: usize| -> Result<[u8; 8]> {
            let mut b = [0u8; 8];
            b[..n].copy_from_slice(self.take(n)?);
            Ok(b)
        };
        Ok(match t {
            1 => Value::Bool(le(1)?[0] != 0),
            2 => Value::UInt(le(1)?[0] as u64),
            3 => {
                // UTF-8 char: read 1–4 bytes.
                let first = self.u8()?;
                let extra = if first < 0x80 { 0 } else if first < 0xE0 { 1 } else if first < 0xF0 { 2 } else { 3 };
                self.take(extra)?;
                Value::Int(first as i64)
            }
            5 => Value::Str(self.string()?), // Decimal as string
            6 => Value::Float(f64::from_le_bytes(le(8)?)),
            7 => Value::Int(i16::from_le_bytes(le(2)?[..2].try_into().unwrap()) as i64),
            8 => Value::Int(i32::from_le_bytes(le(4)?[..4].try_into().unwrap()) as i64),
            9 | 12 | 13 => Value::Int(i64::from_le_bytes(le(8)?)),
            10 => Value::Int(le(1)?[0] as i8 as i64),
            11 => Value::Float(f32::from_le_bytes(le(4)?[..4].try_into().unwrap()) as f64),
            14 => Value::UInt(u16::from_le_bytes(le(2)?[..2].try_into().unwrap()) as u64),
            15 => Value::UInt(u32::from_le_bytes(le(4)?[..4].try_into().unwrap()) as u64),
            16 => Value::UInt(u64::from_le_bytes(le(8)?)),
            17 => Value::Null,
            18 => Value::Str(self.string()?),
            _ => return err(format!("unknown NRBF primitive type {t}")),
        })
    }

    /// Reads one record in a value position. Returns the value plus how many
    /// array slots it fills (ObjectNullMultiple fills several).
    fn value_record(&mut self) -> Result<(Value, usize)> {
        loop {
            let rt = self.u8()?;
            return Ok(match rt {
                12 => {
                    // BinaryLibrary may precede any record.
                    self.i32()?;
                    self.string()?;
                    continue;
                }
                6 => {
                    let id = self.i32()?;
                    let s = self.string()?;
                    self.objects.insert(id, Record::Str(s.clone()));
                    (Value::Str(s), 1)
                }
                8 => {
                    let t = self.u8()?;
                    (self.primitive(t)?, 1)
                }
                9 => (Value::Ref(self.i32()?), 1),
                10 => (Value::Null, 1),
                13 => (Value::Null, self.u8()? as usize),
                14 => (Value::Null, self.i32()?.max(0) as usize),
                1..=5 | 7 | 15..=17 => (Value::Obj(self.object(rt)?), 1),
                _ => return err(format!("unexpected NRBF record type {rt}")),
            });
        }
    }

    fn class_info(&mut self) -> Result<(i32, String, Vec<String>)> {
        let id = self.i32()?;
        let name = self.string()?;
        let n = self.i32()?;
        if !(0..=100_000).contains(&n) {
            return err("bad NRBF member count");
        }
        let members = (0..n).map(|_| self.string()).collect::<Result<Vec<_>>>()?;
        Ok((id, name, members))
    }

    /// MemberTypeInfo: per member, `Some(prim)` for primitive members.
    fn member_types(&mut self, n: usize) -> Result<Vec<Option<u8>>> {
        let kinds = self.take(n)?.to_vec();
        let mut out = Vec::with_capacity(n);
        for k in kinds {
            out.push(match k {
                0 => Some(self.u8()?),
                1 | 2 | 5 | 6 => None,
                3 => {
                    self.string()?;
                    None
                }
                4 => {
                    self.string()?;
                    self.i32()?;
                    None
                }
                7 => {
                    self.u8()?;
                    None
                }
                _ => return err(format!("bad NRBF binary type {k}")),
            });
        }
        Ok(out)
    }

    fn members(&mut self, meta: &ClassMeta) -> Result<Vec<(String, Value)>> {
        let mut vals = Vec::with_capacity(meta.members.len());
        for (i, name) in meta.members.iter().enumerate() {
            let v = match meta.prim.as_ref().and_then(|p| p[i]) {
                Some(t) => self.primitive(t)?,
                None => self.value_record()?.0,
            };
            vals.push((name.clone(), v));
        }
        Ok(vals)
    }

    fn elements(&mut self, len: usize) -> Result<Vec<Value>> {
        let mut out = Vec::with_capacity(len.min(1 << 16));
        while out.len() < len {
            let (v, n) = self.value_record()?;
            for _ in 0..n.min(len - out.len()) {
                out.push(v.clone());
            }
        }
        Ok(out)
    }

    fn prim_array(&mut self, id: i32, len: usize, t: u8) -> Result<i32> {
        if t == 2 {
            let b = self.take(len)?.to_vec();
            self.objects.insert(id, Record::Bytes(b));
        } else {
            let v = (0..len).map(|_| self.primitive(t)).collect::<Result<Vec<_>>>()?;
            self.objects.insert(id, Record::Array(v));
        }
        Ok(id)
    }

    /// Parses an object record of type `rt` (already consumed); returns its id.
    fn object(&mut self, rt: u8) -> Result<i32> {
        self.depth += 1;
        if self.depth > 512 {
            return err(".pdn object graph nested too deeply");
        }
        let r = self.object_inner(rt);
        self.depth -= 1;
        r
    }

    fn object_inner(&mut self, rt: u8) -> Result<i32> {
        match rt {
            1 => {
                let id = self.i32()?;
                let mid = self.i32()?;
                let meta = self.meta.get(&mid).cloned().ok_or_else(|| FormatError("unknown NRBF metadata id".into()))?;
                let members = self.members(&meta)?;
                self.objects.insert(id, Record::Class { name: meta.name.clone(), members });
                Ok(id)
            }
            2..=5 => {
                let (id, name, member_names) = self.class_info()?;
                let prim = if rt == 4 || rt == 5 { Some(self.member_types(member_names.len())?) } else { None };
                if rt == 3 || rt == 5 {
                    self.i32()?; // library id
                }
                let meta = std::rc::Rc::new(ClassMeta { name: name.clone(), members: member_names, prim });
                self.meta.insert(id, meta.clone());
                let members = self.members(&meta)?;
                self.objects.insert(id, Record::Class { name, members });
                Ok(id)
            }
            7 => {
                let id = self.i32()?;
                let kind = self.u8()?;
                let rank = self.i32()?;
                if !(1..=32).contains(&rank) {
                    return err("bad NRBF array rank");
                }
                let mut len = 1usize;
                for _ in 0..rank {
                    len = len.saturating_mul(self.i32()?.max(0) as usize);
                }
                if kind >= 3 {
                    for _ in 0..rank {
                        self.i32()?;
                    }
                }
                let t = self.member_types(1)?;
                match t[0] {
                    Some(p) => self.prim_array(id, len, p),
                    None => {
                        let v = self.elements(len)?;
                        self.objects.insert(id, Record::Array(v));
                        Ok(id)
                    }
                }
            }
            15 => {
                let id = self.i32()?;
                let len = self.i32()?.max(0) as usize;
                let t = self.u8()?;
                self.prim_array(id, len, t)
            }
            16 | 17 => {
                let id = self.i32()?;
                let len = self.i32()?.max(0) as usize;
                let v = self.elements(len)?;
                self.objects.insert(id, Record::Array(v));
                Ok(id)
            }
            _ => err(format!("unexpected NRBF record type {rt}")),
        }
    }
}

/// A decoded object graph.
pub struct Graph {
    pub root: i32,
    pub objects: HashMap<i32, Record>,
}

/// Decodes an NRBF stream starting at `data[0]` (the header record). Returns
/// the graph and the number of bytes consumed.
pub fn decode_nrbf(data: &[u8]) -> Result<(Graph, usize)> {
    let mut r = Reader { d: data, p: 0, meta: HashMap::new(), objects: HashMap::new(), depth: 0 };
    if r.u8()? != 0 {
        return err("missing NRBF header");
    }
    let root = r.i32()?;
    r.take(12)?; // header id, major, minor
    loop {
        let rt = r.u8()?;
        match rt {
            11 => break,
            12 => {
                r.i32()?;
                r.string()?;
            }
            6 => {
                let id = r.i32()?;
                let s = r.string()?;
                r.objects.insert(id, Record::Str(s));
            }
            _ => {
                r.object(rt)?;
            }
        }
    }
    Ok((Graph { root, objects: r.objects }, r.p))
}

impl Graph {
    fn class(&self, v: &Value) -> Option<(&str, &[(String, Value)])> {
        let id = match v {
            Value::Ref(i) | Value::Obj(i) => *i,
            _ => return None,
        };
        match self.objects.get(&id)? {
            Record::Class { name, members } => Some((name, members)),
            _ => None,
        }
    }

    fn record(&self, v: &Value) -> Option<&Record> {
        match v {
            Value::Ref(i) | Value::Obj(i) => self.objects.get(i),
            _ => None,
        }
    }
}

/// Member by short name: exact, or `Base+name` for inherited fields.
fn member<'g>(members: &'g [(String, Value)], short: &str) -> Option<&'g Value> {
    members
        .iter()
        .find(|(n, _)| n == short || n.strip_suffix(short).is_some_and(|p| p.ends_with('+')))
        .map(|(_, v)| v)
}

fn as_i64(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Int(i) => Some(*i),
        Value::UInt(u) => Some(*u as i64),
        _ => None,
    }
}

// ------------------------------------------------------------------ document

const BLEND_OPS: [(&str, BlendMode); 14] = [
    ("MultiplyBlendOp", BlendMode::Multiply),
    ("AdditiveBlendOp", BlendMode::Add),
    ("ColorBurnBlendOp", BlendMode::ColorBurn),
    ("ColorDodgeBlendOp", BlendMode::ColorDodge),
    ("ReflectBlendOp", BlendMode::Reflect),
    ("GlowBlendOp", BlendMode::Glow),
    ("OverlayBlendOp", BlendMode::Overlay),
    ("DifferenceBlendOp", BlendMode::Difference),
    ("NegationBlendOp", BlendMode::Negation),
    ("LightenBlendOp", BlendMode::Lighten),
    ("DarkenBlendOp", BlendMode::Darken),
    ("ScreenBlendOp", BlendMode::Screen),
    ("XorBlendOp", BlendMode::Xor),
    ("NormalBlendOp", BlendMode::Normal),
];

fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 18 || data[0] != 0x1f || data[1] != 0x8b || data[2] != 8 {
        return err("bad gzip data in .pdn");
    }
    let flags = data[3];
    let mut p = 10;
    if flags & 4 != 0 {
        let xlen = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
        p += 2 + xlen;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            while p < data.len() && data[p] != 0 {
                p += 1;
            }
            p += 1;
        }
    }
    if flags & 2 != 0 {
        p += 2;
    }
    if p > data.len() {
        return err("bad gzip header in .pdn");
    }
    decompress_to_vec(&data[p..]).map_err(|_| FormatError("corrupt gzip data in .pdn".into()))
}

/// Length of a complete gzip member at the start of `data` is not needed:
/// the compressed graph runs to the end of the file.
fn read_deferred_block(d: &[u8], p: &mut usize, length: usize) -> Result<Vec<u8>> {
    let mut take = |n: usize| -> Result<&[u8]> {
        if *p + n > d.len() {
            return eof();
        }
        let s = &d[*p..*p + n];
        *p += n;
        Ok(s)
    };
    let version = take(1)?[0];
    if version > 1 {
        return err(format!("unknown deferred format version {version}"));
    }
    let chunk_size = u32::from_be_bytes(take(4)?.try_into().unwrap()) as usize;
    if chunk_size == 0 {
        return err("zero chunk size in .pdn");
    }
    let mut out = vec![0u8; length];
    let count = length.div_ceil(chunk_size);
    let mut seen = vec![false; count];
    for _ in 0..count {
        let n = u32::from_be_bytes(take(4)?.try_into().unwrap()) as usize;
        if n >= count || seen[n] {
            return err(format!("bad chunk number {n} in .pdn"));
        }
        seen[n] = true;
        let size = u32::from_be_bytes(take(4)?.try_into().unwrap()) as usize;
        let data = take(size)?;
        let off = n * chunk_size;
        let this = chunk_size.min(length - off);
        if version == 0 {
            let raw = gunzip(data)?;
            if raw.len() < this {
                return err("short pixel chunk in .pdn");
            }
            out[off..off + this].copy_from_slice(&raw[..this]);
        } else {
            if data.len() < this {
                return err("short pixel chunk in .pdn");
            }
            out[off..off + this].copy_from_slice(&data[..this]);
        }
    }
    Ok(out)
}

fn fill_surface(s: &mut Surface, bgra: &[u8], stride: i64) {
    let row = s.width() as usize * 4;
    let stride = if stride <= 0 { row } else { stride as usize };
    let dst = s.as_bytes_mut();
    for y in 0..dst.len() / row {
        let o = y * stride;
        if o + row > bgra.len() {
            break;
        }
        dst[y * row..(y + 1) * row].copy_from_slice(&bgra[o..o + row]);
    }
}

/// Reads a Paint.NET 2.1+ `.pdn` file.
pub fn read(data: &[u8]) -> Result<Document> {
    if !data.starts_with(b"PDN3") {
        return err("not a Paint.NET 2.1+ file (missing PDN3 magic); pre-2.1 files are not supported");
    }
    if data.len() < 9 {
        return eof();
    }
    let header_len = data[4] as usize | (data[5] as usize) << 8 | (data[6] as usize) << 16;
    let mut p = 7 + header_len;
    let marker = data.get(p..p + 2).ok_or(()).or_else(|_| eof())?;
    let (graph_bytes, deferred): (std::borrow::Cow<[u8]>, bool) = match marker {
        [0x00, 0x01] => {
            p += 2;
            (std::borrow::Cow::Borrowed(&data[p..]), true)
        }
        [0x1f, 0x8b] => (std::borrow::Cow::Owned(gunzip(&data[p..])?), false),
        _ => return err("unrecognised .pdn data section marker"),
    };
    let (g, used) = decode_nrbf(&graph_bytes)?;
    let mut dp = p + used; // deferred blocks follow the graph in the file

    let (_, doc_m) = g.class(&Value::Obj(g.root)).ok_or_else(|| FormatError("unexpected root object in .pdn".into()))?;
    let width = as_i64(member(doc_m, "width")).ok_or_else(|| FormatError("document has no width".into()))?;
    let height = as_i64(member(doc_m, "height")).ok_or_else(|| FormatError("document has no height".into()))?;
    crate::check_size(width, height)?;
    let mut doc = Document::new(width as i32, height as i32);

    let (_, list) = member(doc_m, "layers").and_then(|v| g.class(v)).ok_or_else(|| FormatError("document has no layers".into()))?;
    let items = member(list, "_items").and_then(|v| g.record(v));
    let size = as_i64(member(list, "_size")).unwrap_or(i64::MAX);
    let Some(Record::Array(items)) = items else { return err("layer list has no items array") };

    let mut pending: Vec<(usize, i64, usize)> = Vec::new(); // (layer index, stride, length)
    for item in items.iter().filter_map(|v| g.class(v)).take(size.max(0) as usize) {
        let (_, lm) = item;
        let (_, sm) = member(lm, "surface")
            .and_then(|v| g.class(v))
            .ok_or_else(|| FormatError("layer has no surface (only bitmap layers are supported)".into()))?;
        let stride = as_i64(member(sm, "stride")).unwrap_or(0);
        let (_, block) = member(sm, "scan0").and_then(|v| g.class(v)).ok_or_else(|| FormatError("surface has no scan0".into()))?;
        let mut layer = Layer::new(doc.width(), doc.height(), "Layer");
        // Layer+properties (name/visible/opacity) and BitmapLayer's own properties (blendOp).
        for (name, v) in lm {
            if !(name == "properties" || name.ends_with("+properties")) {
                continue;
            }
            let Some((_, pm)) = g.class(v) else { continue };
            if member(pm, "name").is_some() {
                if let Some(Value::Str(s)) = member(pm, "name") {
                    layer.name = s.clone();
                } else if let Some(Record::Str(s)) = member(pm, "name").and_then(|v| g.record(v)) {
                    layer.name = s.clone();
                }
                if let Some(Value::Bool(b)) = member(pm, "visible") {
                    layer.visible = *b;
                }
                if let Some(o) = as_i64(member(pm, "opacity")) {
                    if (0..=255).contains(&o) {
                        layer.opacity = o as u8;
                    }
                }
            } else if member(pm, "blendOp").is_some() {
                if let Some((type_name, _)) = member(pm, "blendOp").and_then(|v| g.class(v)) {
                    layer.blend_mode =
                        BLEND_OPS.iter().find(|(m, _)| type_name.contains(m)).map(|&(_, b)| b).unwrap_or(BlendMode::Normal);
                }
                if let Some(o) = as_i64(member(pm, "opacity")) {
                    if (0..=255).contains(&o) {
                        layer.opacity = o as u8;
                    }
                }
            }
        }
        let idx = doc.layers.len();
        match member(block, "pointerData").and_then(|v| g.record(v)) {
            Some(Record::Bytes(b)) => fill_surface(&mut layer.surface, b, stride),
            _ => {
                let len = as_i64(member(block, "length64"))
                    .or_else(|| as_i64(member(block, "length")))
                    .ok_or_else(|| FormatError("memory block has no length".into()))?;
                if len < 0 || len as u64 > 1 << 32 {
                    return err("bad memory block length");
                }
                pending.push((idx, stride, len as usize));
            }
        }
        doc.layers.push(layer);
    }
    if !pending.is_empty() {
        if !deferred {
            return err("deferred pixel data expected but the file has none");
        }
        // Deferred blocks follow in graph order (bottom layer first).
        for (idx, stride, len) in pending {
            let bytes = read_deferred_block(data, &mut dp, len)?;
            fill_surface(&mut doc.layers[idx].surface, &bytes, stride);
        }
    }
    Ok(doc)
}
