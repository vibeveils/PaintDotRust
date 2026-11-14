//! Interpreter for `.pdnscript` files — the same language the C# parity
//! runner (`rust/parity/csharp`) understands — so both engines can replay an
//! identical sequence of operations and their outputs can be compared.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use openpdn_core::brushes::FillPattern;
use openpdn_core::history::HistoryStack;
use openpdn_core::resample::TransformResampling;
use openpdn_core::tools::{GradientColorMode, GradientType, Modifiers, PointerButton, ShapeDrawMode, ToolManager};
use openpdn_core::transforms::{self, CanvasAnchor, ResampleMode};
use openpdn_core::{selection_ops, BlendMode, Bgra, Document, Layer, Selection, SelectionCombineMode, Surface};

pub struct Interp {
    out_dir: PathBuf,
    /// Directory relative paths in scripts resolve against.
    pub base_dir: PathBuf,
    write: bool,
    doc: Option<Document>,
    active: usize,
    tools: ToolManager,
    history: HistoryStack,
    pressure: f32,
    handle_radius: f64,
    trace: bool,
}

type R<T> = Result<T, String>;

fn int(s: &str) -> R<i32> {
    s.parse().map_err(|_| format!("bad int {s}"))
}

fn float(s: &str) -> R<f64> {
    s.parse().map_err(|_| format!("bad number {s}"))
}

fn hex(s: &str) -> R<Bgra> {
    Bgra::parse_hex(s).ok_or_else(|| format!("bad colour {s}"))
}

fn arg<'a>(t: &[&'a str], i: usize) -> R<&'a str> {
    t.get(i).copied().ok_or_else(|| format!("missing argument {i}"))
}

fn mods(t: &[&str], i: usize) -> R<Modifiers> {
    let mut m = Modifiers::NONE;
    if let Some(s) = t.get(i) {
        for c in s.chars() {
            m = m | match c {
                'S' => Modifiers::SHIFT,
                'C' => Modifiers::CONTROL,
                'A' => Modifiers::ALT,
                '-' => Modifiers::NONE,
                _ => return Err("bad modifiers".into()),
            };
        }
    }
    Ok(m)
}

fn button(s: &str) -> R<PointerButton> {
    Ok(match s {
        "L" => PointerButton::Left,
        "R" => PointerButton::Right,
        "M" => PointerButton::Middle,
        _ => return Err("bad button".into()),
    })
}

/// Case-insensitive enum lookup by C# member name.
fn pick<T: Copy>(v: &str, opts: &[(&str, T)]) -> R<T> {
    opts.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)).map(|&(_, t)| t).ok_or_else(|| format!("unknown value {v}"))
}

fn xorshift(s: &mut u32) -> u32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    *s
}

impl Interp {
    pub fn new(out_dir: impl Into<PathBuf>, write: bool) -> Interp {
        Interp {
            out_dir: out_dir.into(),
            base_dir: PathBuf::from("."),
            write,
            doc: None,
            active: 0,
            tools: ToolManager::new(),
            history: HistoryStack::new(1000),
            pressure: 1.0,
            handle_radius: 4.0,
            trace: std::env::var_os("OPENPDN_TRACE").is_some() && write,
        }
    }

    pub fn document(&self) -> Option<&Document> {
        self.doc.as_ref()
    }

    pub fn run(&mut self, text: &str) -> R<()> {
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap().trim();
            if line.is_empty() {
                continue;
            }
            let t: Vec<&str> = line.split_whitespace().collect();
            let start = std::time::Instant::now();
            self.exec(&t).map_err(|e| format!("line {}: {line}: {e}", n + 1))?;
            if self.trace {
                eprintln!("{:9.1} ms  {line}", start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        Ok(())
    }

    fn doc(&mut self) -> R<&mut Document> {
        self.doc.as_mut().ok_or_else(|| "no document".to_string())
    }

    fn active_surface(&mut self) -> R<&mut Surface> {
        let a = self.active;
        Ok(&mut self.doc()?.layers[a].surface)
    }

    fn sel_mut(&mut self) -> R<&mut Selection> {
        self.doc()?.selection_mut().ok_or_else(|| "no selection".to_string())
    }

    fn new_sel(&mut self) -> R<Selection> {
        let d = self.doc()?;
        Ok(Selection::new(d.width(), d.height()))
    }

    fn swap(&mut self, d: Document) {
        self.active = self.active.min(d.layers.len() - 1);
        self.doc = Some(d);
    }

    fn down(&mut self, x: i32, y: i32, b: PointerButton, m: Modifiers) -> R<()> {
        let doc = self.doc.as_mut().ok_or("no document")?;
        let id = doc.layers[self.active].id();
        self.tools.pointer_down(doc, &mut self.history, id, x, y, b, m, self.pressure, self.handle_radius);
        Ok(())
    }

    fn mv(&mut self, x: i32, y: i32, m: Modifiers) -> R<()> {
        let doc = self.doc.as_mut().ok_or("no document")?;
        let id = doc.layers[self.active].id();
        self.tools.pointer_move(doc, &mut self.history, id, x, y, m, self.pressure, self.handle_radius);
        Ok(())
    }

    fn up(&mut self, x: i32, y: i32, b: PointerButton, m: Modifiers) -> R<()> {
        let doc = self.doc.as_mut().ok_or("no document")?;
        let id = doc.layers[self.active].id();
        self.tools.pointer_up(doc, &mut self.history, id, x, y, b, m, self.pressure, self.handle_radius);
        Ok(())
    }

    fn exec(&mut self, t: &[&str]) -> R<()> {
        match t[0] {
            "doc" => {
                let mut d = Document::new(int(arg(t, 1)?)?, int(arg(t, 2)?)?);
                d.layers.push(Layer::new(d.width(), d.height(), "Background"));
                self.doc = Some(d);
                self.active = 0;
                self.history.clear();
            }
            "load" => {
                let path = self.base_dir.join(arg(t, 1)?);
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let png = openpdn_formats::png::decode(&bytes).map_err(|e| e.to_string())?;
                self.doc = Some(Document::from_single_layer(png.surface, "Background"));
                self.active = 0;
                self.history.clear();
            }
            "load-pdn" => {
                let path = self.base_dir.join(arg(t, 1)?);
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                self.doc = Some(openpdn_formats::pdn::read(&bytes).map_err(|e| e.to_string())?);
                self.active = 0;
                self.history.clear();
            }
            "save-png" => {
                let d = self.doc()?;
                let bytes = openpdn_formats::png::encode(&d.composite(), &d.metadata.text_chunks, openpdn_formats::png::Compression::Fast);
                if self.write {
                    fs::write(self.out_dir.join(arg(t, 1)?), bytes).map_err(|e| e.to_string())?;
                }
            }
            "save-openpdn" => {
                if self.write {
                    let bytes = openpdn_formats::openpdn::write(self.doc()?).map_err(|e| e.to_string())?;
                    fs::write(self.out_dir.join(arg(t, 1)?), bytes).map_err(|e| e.to_string())?;
                }
            }
            "load-openpdn-env" => {
                let path = std::env::var(arg(t, 1)?).map_err(|_| "environment variable not set")?;
                let bytes = fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
                self.doc = Some(openpdn_formats::openpdn::read(&bytes).map_err(|e| e.to_string())?);
                self.active = 0;
                self.history.clear();
            }
            "meta" => {
                use openpdn_core::document::{PngTextChunk, PngTextKind};
                let kind = pick(
                    arg(t, 1)?,
                    &[("Text", PngTextKind::Text), ("CompressedText", PngTextKind::CompressedText), ("InternationalText", PngTextKind::InternationalText)],
                )?;
                let chunk = PngTextChunk {
                    keyword: arg(t, 2)?.to_owned(),
                    text: t[3..].join(" "),
                    kind,
                    language_tag: if kind == PngTextKind::InternationalText { "en".into() } else { String::new() },
                    translated_keyword: String::new(),
                    compressed: kind != PngTextKind::Text,
                };
                self.doc()?.metadata.text_chunks.push(chunk);
            }
            "dump-meta" => {
                if self.write {
                    use openpdn_core::document::PngTextKind;
                    let d = self.doc()?;
                    let cap = |b: bool| if b { "True" } else { "False" };
                    let mut s: String = d
                        .layers
                        .iter()
                        .map(|l| format!("{}|{}|{}|{}", l.name, cap(l.visible), l.opacity, l.blend_mode.name()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    s.push('\n');
                    s += &d
                        .metadata
                        .text_chunks
                        .iter()
                        .map(|c| {
                            let k = match c.kind {
                                PngTextKind::Text => "Text",
                                PngTextKind::CompressedText => "CompressedText",
                                PngTextKind::InternationalText => "InternationalText",
                            };
                            format!("{}|{}|{}|{}|{}", c.keyword, c.text, k, c.language_tag, cap(c.compressed))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    s.push('\n');
                    let name = format!("{}.txt", arg(t, 1)?);
                    fs::write(self.out_dir.join(name), s).map_err(|e| e.to_string())?;
                }
            }
            "layer-add" => {
                let d = self.doc()?;
                let n = d.layers.len() + 1;
                let l = Layer::new(d.width(), d.height(), format!("Layer {n}"));
                d.layers.push(l);
                self.active = n - 1;
            }
            "layer-active" => self.active = int(arg(t, 1)?)? as usize,
            "layer-visible" => {
                let i = int(arg(t, 1)?)? as usize;
                self.doc()?.layers[i].visible = arg(t, 2)? == "1";
            }
            "layer-opacity" => {
                let i = int(arg(t, 1)?)? as usize;
                self.doc()?.layers[i].opacity = int(arg(t, 2)?)? as u8;
            }
            "layer-blend" => {
                let i = int(arg(t, 1)?)? as usize;
                let m = BlendMode::from_name(arg(t, 2)?).ok_or("bad blend mode")?;
                self.doc()?.layers[i].blend_mode = m;
            }
            "noise" => {
                let mut s = int(arg(t, 1)?)? as u32 | 1;
                let opaque = t.get(2).is_none_or(|v| *v == "opaque");
                for p in self.active_surface()?.pixels_mut() {
                    let v = xorshift(&mut s);
                    *p = Bgra::new(v as u8, (v >> 8) as u8, (v >> 16) as u8, if opaque { 255 } else { (v >> 24) as u8 });
                }
            }
            "pattern" => {
                let k = int(arg(t, 1)?)?;
                let s = self.active_surface()?;
                let (w, h) = (s.width(), s.height());
                for y in 0..h {
                    for x in 0..w {
                        let c = match ((x / k) + (y / k)) % 4 {
                            0 => Bgra::new(255, 0, 0, 255),
                            1 => Bgra::new(0, 255, 0, 255),
                            2 => Bgra::new((x * 255 / (w - 1).max(1)) as u8, (y * 255 / (h - 1).max(1)) as u8, 128, 255),
                            _ => Bgra::TRANSPARENT,
                        };
                        s.set(x, y, c);
                    }
                }
            }
            "clear" => {
                let c = hex(arg(t, 1)?)?;
                self.active_surface()?.clear(c);
            }
            "fg" => self.tools.settings.foreground = hex(arg(t, 1)?)?,
            "bg" => self.tools.settings.background = hex(arg(t, 1)?)?,
            "set" => self.set(arg(t, 1)?, arg(t, 2)?)?,
            "tool" => {
                let name = t[1..].join(" ");
                if !self.tools.set_active_by_name(&name) {
                    return Err(format!("unknown tool {name}"));
                }
            }
            "pressure" => self.pressure = float(arg(t, 1)?)? as f32,
            "handle-radius" => self.handle_radius = float(arg(t, 1)?)?,
            "down" => self.down(int(arg(t, 1)?)?, int(arg(t, 2)?)?, button(arg(t, 3)?)?, mods(t, 4)?)?,
            "move" => self.mv(int(arg(t, 1)?)?, int(arg(t, 2)?)?, mods(t, 3)?)?,
            "up" => self.up(int(arg(t, 1)?)?, int(arg(t, 2)?)?, button(arg(t, 3)?)?, mods(t, 4)?)?,
            "drag" => {
                let b = button(arg(t, 1)?)?;
                let (x0, y0, x1, y1) = (int(arg(t, 2)?)?, int(arg(t, 3)?)?, int(arg(t, 4)?)?, int(arg(t, 5)?)?);
                let steps = int(arg(t, 6)?)?;
                let m = mods(t, 7)?;
                self.down(x0, y0, b, m)?;
                for i in 1..=steps {
                    self.mv(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps, m)?;
                }
                self.up(x1, y1, b, m)?;
            }
            "undo" => {
                let d = self.doc.as_mut().ok_or("no document")?;
                self.history.undo(d);
            }
            "redo" => {
                let d = self.doc.as_mut().ok_or("no document")?;
                self.history.redo(d);
            }
            "sel-rect" | "sel-ellipse" => {
                let mut s = self.new_sel()?;
                let (x, y, w, h) = (int(arg(t, 1)?)?, int(arg(t, 2)?)?, int(arg(t, 3)?)?, int(arg(t, 4)?)?);
                if t[0] == "sel-rect" {
                    s.set_rectangle(x, y, w, h);
                } else {
                    s.set_ellipse(x, y, w, h);
                }
                self.doc()?.set_selection(Some(s));
            }
            "sel-all" => {
                let mut s = self.new_sel()?;
                s.select_all();
                self.doc()?.set_selection(Some(s));
            }
            "sel-none" => self.doc()?.set_selection(None),
            "sel-invert" => self.sel_mut()?.invert(),
            "sel-feather" => {
                let r = int(arg(t, 1)?)?;
                self.sel_mut()?.feather(r);
            }
            "sel-grow" => {
                let r = int(arg(t, 1)?)?;
                self.sel_mut()?.grow(r);
            }
            "sel-shrink" => {
                let r = int(arg(t, 1)?)?;
                self.sel_mut()?.shrink(r);
            }
            "sel-erase" | "sel-fill" => {
                let c = if t[0] == "sel-fill" { Some(hex(arg(t, 1)?)?) } else { None };
                let a = self.active;
                let d = self.doc()?;
                let sel = d.selection().ok_or("no selection")?.clone();
                let s = &mut d.layers[a].surface;
                match c {
                    Some(c) => selection_ops::fill_selection(s, &sel, c),
                    None => selection_ops::erase_selection(s, &sel),
                };
            }
            "resize" => {
                let m = pick(
                    arg(t, 3)?,
                    &[
                        ("Best", ResampleMode::Best),
                        ("Bilinear", ResampleMode::Bilinear),
                        ("NearestNeighbor", ResampleMode::NearestNeighbor),
                        ("SuperSampling", ResampleMode::SuperSampling),
                    ],
                )?;
                let (w, h) = (int(arg(t, 1)?)?, int(arg(t, 2)?)?);
                let d = transforms::document_resize(self.doc()?, w, h, m);
                self.swap(d);
            }
            "rotate90cw" => {
                let d = transforms::document_rotate_90_cw(self.doc()?);
                self.swap(d);
            }
            "rotate90ccw" => {
                let d = transforms::document_rotate_90_ccw(self.doc()?);
                self.swap(d);
            }
            "rotate180" => {
                for l in &mut self.doc()?.layers {
                    transforms::rotate_180(&mut l.surface);
                }
            }
            "flip-h" => transforms::flip_horizontal(self.active_surface()?),
            "flip-v" => transforms::flip_vertical(self.active_surface()?),
            "canvas" => {
                use CanvasAnchor::*;
                let a = pick(
                    arg(t, 3)?,
                    &[
                        ("TopLeft", TopLeft),
                        ("TopCenter", TopCenter),
                        ("TopRight", TopRight),
                        ("MiddleLeft", MiddleLeft),
                        ("MiddleCenter", MiddleCenter),
                        ("MiddleRight", MiddleRight),
                        ("BottomLeft", BottomLeft),
                        ("BottomCenter", BottomCenter),
                        ("BottomRight", BottomRight),
                    ],
                )?;
                let (w, h) = (int(arg(t, 1)?)?, int(arg(t, 2)?)?);
                let d = transforms::canvas_size(self.doc()?, w, h, a);
                self.swap(d);
            }
            "crop" => {
                let doc = self.doc()?;
                let sel = doc.selection().ok_or("no selection")?;
                let d = transforms::crop_to_selection(doc, sel).ok_or("empty selection")?;
                self.swap(d);
            }
            "autocrop" => {
                if let Some(d) = transforms::auto_crop(self.doc()?) {
                    self.swap(d);
                }
            }
            "flatten" => {
                let d = transforms::flatten(self.doc()?);
                self.swap(d);
                self.active = 0;
            }
            "rotate-zoom" => {
                let (a, z, ox, oy) = (float(arg(t, 1)?)?, float(arg(t, 2)?)?, float(arg(t, 3)?)?, float(arg(t, 4)?)?);
                let tile = arg(t, 5)? == "1";
                let s = self.active_surface()?;
                let copy = s.clone();
                transforms::rotate_zoom(&copy, s, a, z, ox, oy, tile);
            }
            "adjust" => self.adjust(t)?,
            "effect" => self.effect(t)?,
            "composite" => {
                std::hint::black_box(self.doc()?.composite());
            }
            "dump" => {
                let s = self.doc()?.composite();
                self.dump(arg(t, 1)?, &s)?;
            }
            "dump-layer" => {
                let i = int(arg(t, 2)?)? as usize;
                let s = self.doc()?.layers[i].surface.clone();
                self.dump(arg(t, 1)?, &s)?;
            }
            "colors" => {
                if self.write {
                    let s = &self.tools.settings;
                    let text = format!(
                        "fg {}\nbg {}\nundo {}\nredo {}\n",
                        s.foreground.to_hex(),
                        s.background.to_hex(),
                        self.history.undo_depth(),
                        self.history.redo_depth()
                    );
                    fs::write(self.out_dir.join(format!("{}.txt", arg(t, 1)?)), text).map_err(|e| e.to_string())?;
                }
            }
            other => return Err(format!("unknown command {other}")),
        }
        Ok(())
    }

    fn adjust(&mut self, t: &[&str]) -> R<()> {
        use openpdn_core::adjust::*;
        let pts = |s: &str| -> R<Vec<(i32, i32)>> {
            s.split(',')
                .map(|p| {
                    let (a, b) = p.split_once(':').ok_or("bad point")?;
                    Ok((int(a)?, int(b)?))
                })
                .collect()
        };
        let a = self.active;
        let doc = self.doc.as_mut().ok_or("no document")?;
        let op: Box<dyn PixelOp> = match arg(t, 1)? {
            "invert" => Box::new(InvertColors),
            "desaturate" => Box::new(Desaturate),
            "sepia" => Box::new(Sepia::default()),
            "brightness-contrast" => Box::new(BrightnessContrast::new(int(arg(t, 2)?)?, int(arg(t, 3)?)?)),
            "hue-sat" => Box::new(HueSaturation::new(int(arg(t, 2)?)?, int(arg(t, 3)?)?, int(arg(t, 4)?)?)),
            "posterize" => Box::new(posterize(int(arg(t, 2)?)?, int(arg(t, 3)?)?, int(arg(t, 4)?)?)),
            "levels" => Box::new(levels(
                hex(arg(t, 2)?)?,
                hex(arg(t, 3)?)?,
                [float(arg(t, 4)?)? as f32, float(arg(t, 5)?)? as f32, float(arg(t, 6)?)? as f32],
                hex(arg(t, 7)?)?,
                hex(arg(t, 8)?)?,
            )),
            "levels-auto" => Box::new(HistogramRgb::new(&doc.layers[a].surface, doc.selection()).auto_levels()),
            "curves-lum" => Box::new(LuminosityCurve(Spline::build_lut(&pts(arg(t, 2)?)?))),
            "curves-rgb" => Box::new(channel_curve(
                Spline::build_lut(&pts(arg(t, 2)?)?),
                Spline::build_lut(&pts(arg(t, 3)?)?),
                Spline::build_lut(&pts(arg(t, 4)?)?),
            )),
            "red-eye" => Box::new(RedEyeRemove::new(int(arg(t, 2)?)?, int(arg(t, 3)?)?)),
            other => return Err(format!("unknown adjustment {other}")),
        };
        let sel = doc.selection().cloned();
        apply_op(op.as_ref(), &mut doc.layers[a].surface, sel.as_ref());
        Ok(())
    }

    fn effect(&mut self, t: &[&str]) -> R<()> {
        use openpdn_core::effects::{artistic::*, blur::*, distort::*, print::*, render::*, stylize::*, Effect};
        let i = |n: usize| -> R<i32> { int(arg(t, n)?) };
        let f = |n: usize| -> R<f64> { float(arg(t, n)?) };
        let c = |n: usize| -> R<Bgra> { hex(arg(t, n)?) };
        let b = |n: usize| -> R<bool> { Ok(arg(t, n)? == "1") };
        let e: Box<dyn Effect> = match arg(t, 1)? {
            "gaussian" => Box::new(GaussianBlur(i(2)?)),
            "motion" => Box::new(MotionBlur::new(f(2)?, i(3)?, b(4)?)),
            "sharpen" => Box::new(Sharpen(i(2)?)),
            "median" => Box::new(Median { radius: i(2)?, percentile: i(3)? }),
            "noise" => Box::new(AddNoise { intensity: i(2)?, saturation: i(3)?, coverage_percent: f(4)?, seed: i(5)? }),
            "pixelate" => Box::new(Pixelate(i(2)?)),
            "emboss" => Box::new(Emboss(f(2)?)),
            "edge" => Box::new(ColorDifference::edge_detect(f(2)?)),
            "relief" => Box::new(ColorDifference::relief(f(2)?)),
            "oil" => Box::new(OilPainting { brush_size: i(2)?, coarseness: i(3)? }),
            "unfocus" => Box::new(Unfocus(i(2)?)),
            "fragment" => Box::new(Fragment::new(i(2)?, i(3)?, f(4)?)),
            "radial" => Box::new(RadialBlur { angle_degrees: f(2)?, offset_x: f(3)?, offset_y: f(4)?, quality: i(5)? }),
            "twist" => Box::new(Twist::new(f(2)?, f(3)?, f(4)?, f(5)?, i(6)?)),
            "bulge" => Box::new(Bulge { amount: f(2)?, offset_x: f(3)?, offset_y: f(4)? }),
            "polar" => Box::new(PolarInversion {
                amount: f(2)?,
                offset_x: f(3)?,
                offset_y: f(4)?,
                edge: pick(
                    arg(t, 5)?,
                    &[("Clamp", WarpEdge::Clamp), ("Wrap", WarpEdge::Wrap), ("Reflect", WarpEdge::Reflect), ("Original", WarpEdge::Original)],
                )?,
                quality: i(6)?,
            }),
            "frosted" => Box::new(FrostedGlass { min_radius: f(2)?, max_radius: f(3)?, sample_count: i(4)?, seed: i(5)? }),
            "tile" => Box::new(TileReflection::new(f(2)?, f(3)?, f(4)?, i(5)?)),
            "clouds" => Box::new(Clouds {
                scale: i(2)?,
                power: f(3)?,
                seed: i(4)?,
                from: c(5)?,
                to: c(6)?,
                blend_mode: BlendMode::from_name(arg(t, 7)?).ok_or("bad blend mode")?,
            }),
            "julia" => Box::new(JuliaFractal { factor: f(2)?, zoom: f(3)?, angle_degrees: f(4)?, quality: i(5)? }),
            "mandelbrot" => Box::new(MandelbrotFractal { factor: i(2)?, zoom: f(3)?, angle_degrees: f(4)?, quality: i(5)?, invert_colors: b(6)? }),
            "ink" => Box::new(InkSketch { ink_outline: i(2)?, coloring: i(3)? }),
            "pencil" => Box::new(PencilSketch { tip_size: i(2)?, color_range: i(3)? }),
            "soften" => Box::new(SoftenPortrait { softness: i(2)?, lighting: i(3)?, warmth: i(4)? }),
            "reduce-noise" => Box::new(ReduceNoise::new(i(2)?, f(3)?)),
            "glow" => Box::new(Glow { radius: i(2)?, brightness: i(3)?, contrast: i(4)? }),
            "linocut" => Box::new(Linocut {
                ink: c(2)?,
                paper: c(3)?,
                threshold_percent: i(4)?,
                detail_radius: i(5)?,
                edge_percent: i(6)?,
                line_spacing: i(7)?,
            }),
            "pointillism" => Box::new(Pointillism { dot_size: i(2)?, density_percent: i(3)?, color_jitter_percent: i(4)?, seed: i(5)?, paper: c(6)? }),
            "stained" => Box::new(StainedGlass {
                cell_size: i(2)?,
                edge_width: i(3)?,
                randomness_percent: i(4)?,
                seed: i(5)?,
                shading: b(6)?,
                leading: c(7)?,
            }),
            "mosaic" => Box::new(Mosaic { tile_size: i(2)?, grout_width: i(3)?, bevel_width: i(4)?, bevel_strength_percent: i(5)?, grout: c(6)? }),
            "spherize" => Box::new(Spherize {
                amount_percent: i(2)?,
                radius_percent: i(3)?,
                offset_x_percent: i(4)?,
                offset_y_percent: i(5)?,
                quality: i(6)?,
            }),
            "waves" => {
                use WaveType::*;
                Box::new(Waves {
                    kind: pick(
                        arg(t, 2)?,
                        &[("Ripples", Ripples), ("Spiral", Spiral), ("Horizontal", Horizontal), ("Vertical", Vertical), ("Grid", Grid), ("Zigzag", Zigzag), ("Organic", Organic)],
                    )?,
                    wavelength: i(3)?,
                    amplitude: i(4)?,
                    phase_degrees: i(5)?,
                    offset_x_percent: i(6)?,
                    offset_y_percent: i(7)?,
                    quality: i(8)?,
                })
            }
            "halftone" => Box::new(Halftone {
                mode: pick(arg(t, 2)?, &[("Cmyk", HalftoneMode::Cmyk), ("Monochrome", HalftoneMode::Monochrome)])?,
                cell_size: i(3)?,
                angle_degrees: i(4)?,
                ink: c(5)?,
                paper: c(6)?,
            }),
            "dither" => {
                use DitherPattern::*;
                let p = pick(
                    arg(t, 2)?,
                    &[
                        ("Bayer2", Bayer2),
                        ("Bayer4", Bayer4),
                        ("Bayer8", Bayer8),
                        ("Bayer16", Bayer16),
                        ("Spiral", Spiral),
                        ("DualSpiral", DualSpiral),
                        ("BlueNoise", BlueNoise),
                        ("WhiteNoise", WhiteNoise),
                    ],
                )?;
                Box::new(OrderedDither::new(p, i(3)?, b(4)?, i(5)?))
            }
            other => return Err(format!("unknown effect {other}")),
        };
        let a = self.active;
        let doc = self.doc.as_mut().ok_or("no document")?;
        let sel = doc.selection().cloned();
        openpdn_core::effects::apply(e.as_ref(), &mut doc.layers[a].surface, sel.as_ref());
        Ok(())
    }

    fn set(&mut self, key: &str, v: &str) -> R<()> {
        let s = &mut self.tools.settings;
        match key {
            "brush" => s.set_brush_size(int(v)?),
            "aa" => s.antialiasing = v == "1",
            "alphablend" => s.alpha_blending = v == "1",
            "shape" => {
                s.shape_mode = pick(
                    v,
                    &[("Outline", ShapeDrawMode::Outline), ("Fill", ShapeDrawMode::Fill), ("FillOutline", ShapeDrawMode::FillOutline)],
                )?
            }
            "gradient" => {
                use GradientType::*;
                s.gradient_type = pick(
                    v,
                    &[("Linear", Linear), ("LinearReflected", LinearReflected), ("Diamond", Diamond), ("Radial", Radial), ("Conical", Conical)],
                )?
            }
            "gradmode" => {
                s.gradient_color_mode =
                    pick(v, &[("Color", GradientColorMode::Color), ("Transparency", GradientColorMode::Transparency)])?
            }
            "sampleall" => s.sample_all_layers = v == "1",
            "eraser" => {
                use FillPattern::*;
                s.eraser_pattern = pick(
                    v,
                    &[
                        ("Solid", Solid),
                        ("Horizontal", Horizontal),
                        ("Vertical", Vertical),
                        ("Cross", Cross),
                        ("ForwardDiagonal", ForwardDiagonal),
                        ("BackwardDiagonal", BackwardDiagonal),
                        ("DiagonalCross", DiagonalCross),
                        ("Percent50", Percent50),
                        ("Percent25", Percent25),
                        ("DottedGrid", DottedGrid),
                        ("Checkerboard", Checkerboard),
                    ],
                )?
            }
            "resampling" => {
                use TransformResampling::*;
                s.resampling = pick(
                    v,
                    &[
                        ("NearestNeighbor", NearestNeighbor),
                        ("NearestNeighborAntialiased", NearestNeighborAntialiased),
                        ("Bilinear", Bilinear),
                    ],
                )?
            }
            "pressure" => s.pressure_sensitivity = v == "1",
            "tolerance" => s.set_wand_tolerance(int(v)?),
            "combine" => {
                use SelectionCombineMode::*;
                s.selection_combine_mode =
                    pick(v, &[("Replace", Replace), ("Union", Union), ("Subtract", Subtract), ("Intersect", Intersect)])?
            }
            _ => return Err(format!("unknown setting {key}")),
        }
        Ok(())
    }

    fn dump(&mut self, name: &str, s: &Surface) -> R<()> {
        if !self.write {
            return Ok(());
        }
        let mut buf = Vec::with_capacity(16 + s.as_bytes().len());
        buf.extend_from_slice(&(s.width() as u32).to_le_bytes());
        buf.extend_from_slice(&(s.height() as u32).to_le_bytes());
        buf.extend_from_slice(s.as_bytes());
        match self.doc.as_ref().and_then(|d| d.selection()) {
            Some(sel) if sel.width() == s.width() && sel.height() == s.height() => {
                buf.extend_from_slice(&(sel.mask().len() as u32).to_le_bytes());
                buf.extend_from_slice(sel.mask());
            }
            _ => buf.extend_from_slice(&0u32.to_le_bytes()),
        }
        let path = self.out_dir.join(format!("{name}.bin"));
        fs::File::create(&path).and_then(|mut f| f.write_all(&buf)).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Compares two dump files; returns a description of the first difference.
pub fn compare_dumps(a: &Path, b: &Path) -> Result<(), String> {
    let x = fs::read(a).map_err(|e| format!("{}: {e}", a.display()))?;
    let y = fs::read(b).map_err(|e| format!("{}: {e}", b.display()))?;
    if x == y {
        return Ok(());
    }
    if x.len() < 8 || y.len() < 8 || x[..8] != y[..8] {
        return Err("dimensions differ".into());
    }
    let w = u32::from_le_bytes(x[0..4].try_into().unwrap()) as usize;
    let h = u32::from_le_bytes(x[4..8].try_into().unwrap()) as usize;
    let np = w * h * 4;
    let (px, py) = (&x[8..8 + np], &y[8..8 + np]);
    let mut diff = 0usize;
    let mut maxd = 0u8;
    let mut first = None;
    for i in 0..np {
        let d = px[i].abs_diff(py[i]);
        if d != 0 {
            if first.is_none() {
                first = Some(i / 4);
            }
            diff += 1;
            maxd = maxd.max(d);
        }
    }
    let mx = &x[8 + np..];
    let my = &y[8 + np..];
    let mask_diff = if mx.len() != my.len() {
        "selection presence differs".to_string()
    } else {
        let n = mx.iter().zip(my).skip(4).filter(|(a, b)| a != b).count();
        format!("{n} mask bytes differ")
    };
    let loc = first.map(|p| format!(" first at ({}, {})", p % w, p / w)).unwrap_or_default();
    Err(format!("{diff} channel bytes differ (max Δ{maxd}){loc}; {mask_diff}"))
}
