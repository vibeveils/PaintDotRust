//! The `.openpdn` layered document: a zip of one PNG per layer plus
//! `manifest.json` (version, size, layers bottom→top, optional PNG text).

use crate::json::Json;
use crate::png::{self, Compression};
use crate::zip::{self, ZipWriter};
use crate::{err, Result};
use openpdn_core::document::{PngTextChunk, PngTextKind};
use openpdn_core::{BlendMode, Document, Layer};

pub const CURRENT_VERSION: i64 = 1;
const MANIFEST: &str = "manifest.json";

fn kind_name(k: PngTextKind) -> &'static str {
    match k {
        PngTextKind::Text => "Text",
        PngTextKind::CompressedText => "CompressedText",
        PngTextKind::InternationalText => "InternationalText",
    }
}

fn kind_from(v: Option<&Json>) -> PngTextKind {
    // System.Text.Json's string enum converter also accepts the numeric value.
    match v {
        Some(Json::Str(s)) if s.eq_ignore_ascii_case("CompressedText") => PngTextKind::CompressedText,
        Some(Json::Str(s)) if s.eq_ignore_ascii_case("InternationalText") => PngTextKind::InternationalText,
        Some(Json::Num(n)) if *n == 1.0 => PngTextKind::CompressedText,
        Some(Json::Num(n)) if *n == 2.0 => PngTextKind::InternationalText,
        _ => PngTextKind::Text,
    }
}

fn blend_from(v: Option<&Json>) -> BlendMode {
    match v {
        Some(Json::Str(s)) => BlendMode::from_name(s).unwrap_or_default(),
        Some(Json::Num(n)) => BlendMode::from_index(*n as u8).unwrap_or_default(),
        _ => BlendMode::Normal,
    }
}

pub fn read(data: &[u8]) -> Result<Document> {
    let entries = zip::read(data)?;
    let find = |name: &str| entries.iter().find(|e| e.name == name);
    let manifest = find(MANIFEST).ok_or_else(|| crate::FormatError("missing manifest.json — not a .openpdn layered image".into()))?;
    let text = String::from_utf8(manifest.data()?).map_err(|_| crate::FormatError("manifest is not UTF-8".into()))?;
    let m = Json::parse(&text)?;
    let version = m.get("Version").and_then(Json::as_i64).unwrap_or(0);
    if version > CURRENT_VERSION {
        return err(format!("document version {version} is newer than supported ({CURRENT_VERSION})"));
    }
    let width = m.get("Width").and_then(Json::as_i64).unwrap_or(0);
    let height = m.get("Height").and_then(Json::as_i64).unwrap_or(0);
    crate::check_size(width, height)?;
    let mut doc = Document::new(width as i32, height as i32);
    if let Some(chunks) = m.get("Text").and_then(Json::as_array) {
        for c in chunks {
            let s = |k: &str| c.get(k).and_then(Json::as_str).unwrap_or("").to_owned();
            doc.metadata.text_chunks.push(PngTextChunk {
                keyword: s("Keyword"),
                text: s("Text"),
                kind: kind_from(c.get("Kind")),
                language_tag: s("LanguageTag"),
                translated_keyword: s("TranslatedKeyword"),
                compressed: c.get("Compressed").and_then(Json::as_bool).unwrap_or(false),
            });
        }
    }
    for lm in m.get("Layers").and_then(Json::as_array).unwrap_or(&[]) {
        let file = lm.get("File").and_then(Json::as_str).unwrap_or("");
        let entry = find(file).ok_or_else(|| crate::FormatError(format!("missing layer image '{file}'")))?;
        let surface = png::decode(&entry.data()?)?.surface;
        if surface.width() != doc.width() || surface.height() != doc.height() {
            return err(format!("layer image '{file}' has the wrong size"));
        }
        let mut layer = Layer::from_surface(surface, lm.get("Name").and_then(Json::as_str).unwrap_or("Layer"));
        layer.visible = lm.get("Visible").and_then(Json::as_bool).unwrap_or(true);
        layer.opacity = lm.get("Opacity").and_then(Json::as_i64).unwrap_or(255).clamp(0, 255) as u8;
        layer.blend_mode = blend_from(lm.get("BlendMode"));
        doc.layers.push(layer);
    }
    Ok(doc)
}

pub fn write(doc: &Document) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new();
    let mut layers = Vec::new();
    for (i, l) in doc.layers.iter().enumerate() {
        let file = format!("layer_{i:04}.png");
        // PNG is already deflated, so the zip entry is stored.
        zip.add(&file, &png::encode(&l.surface, &[], Compression::Fast), false)?;
        layers.push(Json::Obj(vec![
            ("Name".into(), Json::Str(l.name.clone())),
            ("File".into(), Json::Str(file)),
            ("Visible".into(), Json::Bool(l.visible)),
            ("Opacity".into(), Json::Num(l.opacity as f64)),
            ("BlendMode".into(), Json::Str(l.blend_mode.name().into())),
        ]));
    }
    let mut manifest = vec![
        ("Version".into(), Json::Num(CURRENT_VERSION as f64)),
        ("Width".into(), Json::Num(doc.width() as f64)),
        ("Height".into(), Json::Num(doc.height() as f64)),
        ("Layers".into(), Json::Arr(layers)),
    ];
    if !doc.metadata.text_chunks.is_empty() {
        let chunks = doc
            .metadata
            .text_chunks
            .iter()
            .map(|t| {
                Json::Obj(vec![
                    ("Keyword".into(), Json::Str(t.keyword.clone())),
                    ("Text".into(), Json::Str(t.text.clone())),
                    ("Kind".into(), Json::Str(kind_name(t.kind).into())),
                    ("LanguageTag".into(), Json::Str(t.language_tag.clone())),
                    ("TranslatedKeyword".into(), Json::Str(t.translated_keyword.clone())),
                    ("Compressed".into(), Json::Bool(t.compressed)),
                ])
            })
            .collect();
        manifest.push(("Text".into(), Json::Arr(chunks)));
    }
    zip.add(MANIFEST, Json::Obj(manifest).to_string_pretty().as_bytes(), true)?;
    Ok(zip.finish())
}
