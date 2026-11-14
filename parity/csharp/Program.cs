// Parity runner: interprets the .pdnscript language (see rust/parity/README.md).
using System.Diagnostics;
using System.Globalization;
using OpenPDN;
using OpenPDN.History;
using OpenPDN.Tools;

if (args.Length < 2)
{
    Console.Error.WriteLine("usage: ParityRunner <script> <outdir> [--bench N]");
    return 2;
}
var bench = 1;
for (var i = 2; i < args.Length; i++)
    if (args[i] == "--bench") bench = int.Parse(args[++i], CultureInfo.InvariantCulture);

var lines = File.ReadAllLines(args[0]);
Directory.CreateDirectory(args[1]);
var times = new List<double>();
for (var run = 0; run < bench; run++)
{
    var sw = Stopwatch.StartNew();
    new Interp(args[1], run == bench - 1) { BaseDir = Path.GetDirectoryName(Path.GetFullPath(args[0]))! }.Run(lines);
    sw.Stop();
    times.Add(sw.Elapsed.TotalMilliseconds);
}
if (bench > 1)
{
    times.Sort();
    Console.WriteLine($"{Path.GetFileName(args[0])}: median {times[times.Count / 2]:F2} ms, min {times[0]:F2} ms ({bench} runs)");
}
return 0;

sealed class Interp(string outDir, bool write)
{
    public string BaseDir { get; init; } = ".";
    private Document _doc = null!;
    private int _active;
    private readonly ToolManager _tools = new();
    private readonly HistoryStack _history = new(1000);
    private float _pressure = 1f;
    private double _handleRadius = 4;

    private BitmapLayer Active => (BitmapLayer)_doc.Layers[_active];

    public void Run(string[] lines)
    {
        var n = 0;
        foreach (var raw in lines)
        {
            n++;
            var line = raw.Split('#')[0].Trim();
            if (line.Length == 0) continue;
            var t = line.Split(' ', StringSplitOptions.RemoveEmptyEntries);
            var sw = Stopwatch.StartNew();
            try { Exec(t); }
            catch (Exception e) { throw new InvalidOperationException($"line {n}: {line}: {e.Message}", e); }
            if (write && Environment.GetEnvironmentVariable("OPENPDN_TRACE") is not null)
                Console.Error.WriteLine($"{sw.Elapsed.TotalMilliseconds,9:F1} ms  {line}");
        }
    }

    private static int I(string s) => int.Parse(s, CultureInfo.InvariantCulture);
    private static double D(string s) => double.Parse(s, CultureInfo.InvariantCulture);

    private static ColorBgra Hex(string s) =>
        ColorBgra.TryParseHex(s, out var c) ? c : throw new FormatException($"bad colour {s}");

    private static ToolModifiers Mods(string[] t, int i)
    {
        var m = ToolModifiers.None;
        if (t.Length <= i) return m;
        foreach (var ch in t[i])
            m |= ch switch { 'S' => ToolModifiers.Shift, 'C' => ToolModifiers.Control, 'A' => ToolModifiers.Alt, '-' => 0, _ => throw new FormatException("mods") };
        return m;
    }

    private static PointerButton Btn(string s) => s switch
    {
        "L" => PointerButton.Left,
        "R" => PointerButton.Right,
        "M" => PointerButton.Middle,
        _ => throw new FormatException("button"),
    };

    private ToolContext Ctx() => new(_doc, Active, _tools, _history, () => { })
    {
        Pressure = _pressure,
        HandleRadius = _handleRadius,
    };

    // xorshift32, shared with the Rust interpreter.
    private static uint Next(ref uint s)
    {
        s ^= s << 13; s ^= s >> 17; s ^= s << 5;
        return s;
    }

    private Selection NewSel() => new(_doc.Width, _doc.Height);

    private void Exec(string[] t)
    {
        switch (t[0])
        {
            case "doc":
                _doc = new Document(I(t[1]), I(t[2]));
                _doc.Layers.Add(new BitmapLayer(_doc.Width, _doc.Height, "Background"));
                _active = 0;
                _history.Clear();
                break;
            case "load":
            {
                // load FILE: replaces the document with a single layer decoded from FILE.
                var bytes = File.ReadAllBytes(Path.Combine(BaseDir, t[1]));
                var surf = OpenPDN.Formats.PngCodec.Decode(bytes);
                _doc = Document.FromSingleLayer(surf);
                _active = 0;
                _history.Clear();
                break;
            }
            case "load-pdn":
            {
                using var f = File.OpenRead(Path.Combine(BaseDir, t[1]));
                _doc = new OpenPDN.Formats.PdnLegacyFormat().Read(f);
                _active = 0;
                _history.Clear();
                break;
            }
            case "save-png":
            {
                var bytes = OpenPDN.Formats.FastPngEncoder.Encode(Compositor.Composite(_doc), _doc.Metadata.TextChunks);
                if (write) File.WriteAllBytes(Path.Combine(outDir, t[1]), bytes);
                break;
            }
            case "save-openpdn":
                if (write)
                    using (var f = File.Create(Path.Combine(outDir, t[1])))
                        new OpenPDN.Formats.OpenPdnFormat().Write(_doc, f);
                break;
            case "load-openpdn-env":
            {
                // Path comes from an environment variable so a script can read the other engine's output.
                using var f = File.OpenRead(Environment.GetEnvironmentVariable(t[1])!);
                _doc = new OpenPDN.Formats.OpenPdnFormat().Read(f);
                _active = 0;
                _history.Clear();
                break;
            }
            case "meta":
                _doc.Metadata.TextChunks.Add(new PngTextChunk(t[2], string.Join(' ', t.Skip(3)), Enum.Parse<PngTextKind>(t[1], true),
                    t[1] == "InternationalText" ? "en" : "", "", t[1] != "Text"));
                break;
            case "dump-meta":
                if (write)
                    File.WriteAllText(Path.Combine(outDir, t[1] + ".txt"), string.Join("\n", _doc.Layers.Select(l => $"{l.Name}|{l.Visible}|{l.Opacity}|{l.BlendMode}"))
                        + "\n" + string.Join("\n", _doc.Metadata.TextChunks.Select(c => $"{c.Keyword}|{c.Text}|{c.Kind}|{c.LanguageTag}|{c.Compressed}")) + "\n");
                break;
            case "layer-add":
                _doc.Layers.Add(new BitmapLayer(_doc.Width, _doc.Height, $"Layer {_doc.Layers.Count + 1}"));
                _active = _doc.Layers.Count - 1;
                break;
            case "layer-active":
                _active = I(t[1]);
                break;
            case "layer-visible":
                _doc.Layers[I(t[1])].Visible = t[2] == "1";
                break;
            case "layer-opacity":
                _doc.Layers[I(t[1])].Opacity = (byte)I(t[2]);
                break;
            case "layer-blend":
                _doc.Layers[I(t[1])].BlendMode = Enum.Parse<BlendMode>(t[2], true);
                break;
            case "noise":
            {
                // noise SEED [alpha-mode: opaque|random]
                uint s = (uint)I(t[1]) | 1u;
                var opaque = t.Length < 3 || t[2] == "opaque";
                var px = Active.Surface.AsSpan();
                for (var i = 0; i < px.Length; i++)
                {
                    var v = Next(ref s);
                    px[i] = ColorBgra.FromBgra((byte)v, (byte)(v >> 8), (byte)(v >> 16), opaque ? (byte)255 : (byte)(v >> 24));
                }
                break;
            }
            case "pattern":
            {
                // Smooth-ish deterministic content: bands and blocks, good for flood fills.
                var s = Active.Surface;
                var k = I(t[1]);
                for (var y = 0; y < s.Height; y++)
                    for (var x = 0; x < s.Width; x++)
                    {
                        var v = ((x / k) + (y / k)) % 4;
                        s[x, y] = v switch
                        {
                            0 => ColorBgra.FromBgra(255, 0, 0, 255),
                            1 => ColorBgra.FromBgra(0, 255, 0, 255),
                            2 => ColorBgra.FromBgra((byte)(x * 255 / Math.Max(1, s.Width - 1)), (byte)(y * 255 / Math.Max(1, s.Height - 1)), 128, 255),
                            _ => ColorBgra.FromBgra(0, 0, 0, 0),
                        };
                    }
                break;
            }
            case "clear":
                Active.Surface.Clear(Hex(t[1]));
                break;
            case "fg": _tools.ForegroundColor = Hex(t[1]); break;
            case "bg": _tools.BackgroundColor = Hex(t[1]); break;
            case "set": Set(t[1], t[2]); break;
            case "tool":
            {
                var name = string.Join(' ', t.Skip(1));
                _tools.ActiveTool = _tools.Tools.First(x => x.Name.Equals(name, StringComparison.OrdinalIgnoreCase));
                break;
            }
            case "pressure": _pressure = (float)D(t[1]); break;
            case "handle-radius": _handleRadius = D(t[1]); break;
            case "down": _tools.ActiveTool.OnPointerDown(Ctx(), I(t[1]), I(t[2]), Btn(t[3]), Mods(t, 4)); break;
            case "move": _tools.ActiveTool.OnPointerMove(Ctx(), I(t[1]), I(t[2]), Mods(t, 3)); break;
            case "up": _tools.ActiveTool.OnPointerUp(Ctx(), I(t[1]), I(t[2]), Btn(t[3]), Mods(t, 4)); break;
            case "drag":
            {
                // drag BTN x0 y0 x1 y1 steps [mods]: down, `steps` interpolated moves, up.
                var b = Btn(t[1]);
                int x0 = I(t[2]), y0 = I(t[3]), x1 = I(t[4]), y1 = I(t[5]), steps = I(t[6]);
                var m = Mods(t, 7);
                _tools.ActiveTool.OnPointerDown(Ctx(), x0, y0, b, m);
                for (var i = 1; i <= steps; i++)
                    _tools.ActiveTool.OnPointerMove(Ctx(), x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps, m);
                _tools.ActiveTool.OnPointerUp(Ctx(), x1, y1, b, m);
                break;
            }
            case "undo": _history.Undo(); break;
            case "redo": _history.Redo(); break;
            case "sel-rect": { var s = NewSel(); s.SetRectangle(I(t[1]), I(t[2]), I(t[3]), I(t[4])); _doc.SetSelection(s); break; }
            case "sel-ellipse": { var s = NewSel(); s.SetEllipse(I(t[1]), I(t[2]), I(t[3]), I(t[4])); _doc.SetSelection(s); break; }
            case "sel-all": { var s = NewSel(); s.SelectAll(); _doc.SetSelection(s); break; }
            case "sel-none": _doc.SetSelection(null); break;
            case "sel-invert": _doc.Selection!.Invert(); break;
            case "sel-feather": _doc.Selection!.Feather(I(t[1])); break;
            case "sel-grow": _doc.Selection!.Grow(I(t[1])); break;
            case "sel-shrink": _doc.Selection!.Shrink(I(t[1])); break;
            case "sel-erase": SelectionOps.EraseSelection(Active.Surface, _doc.Selection!); break;
            case "sel-fill": SelectionOps.FillSelection(Active.Surface, _doc.Selection!, Hex(t[1])); break;
            case "resize": Swap(DocumentTransforms.Resize(_doc, I(t[1]), I(t[2]), Enum.Parse<ResampleMode>(t[3], true))); break;
            case "rotate90cw": Swap(DocumentTransforms.Rotate90CW(_doc)); break;
            case "rotate90ccw": Swap(DocumentTransforms.Rotate90CCW(_doc)); break;
            case "rotate180": foreach (var l in _doc.Layers) SurfaceTransforms.Rotate180(((BitmapLayer)l).Surface); break;
            case "flip-h": SurfaceTransforms.FlipHorizontal(Active.Surface); break;
            case "flip-v": SurfaceTransforms.FlipVertical(Active.Surface); break;
            case "canvas": Swap(DocumentTransforms.CanvasSize(_doc, I(t[1]), I(t[2]), Enum.Parse<CanvasAnchor>(t[3], true))); break;
            case "crop": Swap(DocumentTransforms.CropToSelection(_doc, _doc.Selection!)); break;
            case "autocrop": { var d = DocumentTransforms.AutoCrop(_doc); if (d is not null) Swap(d); break; }
            case "flatten": Swap(DocumentTransforms.Flatten(_doc)); _active = 0; break;
            case "rotate-zoom":
            {
                var src = Active.Surface;
                var copy = new Surface(src.Width, src.Height);
                src.AsSpan().CopyTo(copy.AsSpan());
                LayerTransform.RotateZoom(copy, src, D(t[1]), D(t[2]), D(t[3]), D(t[4]), t[5] == "1");
                break;
            }
            case "adjust": Adjust(t); break;
            case "effect": Fx(t); break;
            case "composite": Compositor.Composite(_doc); break;
            case "dump": Dump(t[1], Compositor.Composite(_doc)); break;
            case "dump-layer": Dump(t[1], ((BitmapLayer)_doc.Layers[I(t[2])]).Surface); break;
            case "colors":
            {
                if (!write) break;
                File.WriteAllText(Path.Combine(outDir, t[1] + ".txt"),
                    $"fg {_tools.ForegroundColor.ToHexString()}\nbg {_tools.BackgroundColor.ToHexString()}\nundo {_history.UndoDepth}\nredo {_history.RedoDepth}\n");
                break;
            }
            default: throw new FormatException($"unknown command {t[0]}");
        }
    }

    private static List<(int X, int Y)> Pts(string s) =>
        s.Split(',').Select(p => p.Split(':')).Select(p => (I(p[0]), I(p[1]))).ToList();

    private void Adjust(string[] t)
    {
        OpenPDN.Adjustments.UnaryPixelOp op = t[1] switch
        {
            "invert" => new OpenPDN.Adjustments.InvertColorsOp(),
            "desaturate" => new OpenPDN.Adjustments.DesaturateOp(),
            "sepia" => new OpenPDN.Adjustments.SepiaOp(),
            "brightness-contrast" => new OpenPDN.Adjustments.BrightnessContrastOp(I(t[2]), I(t[3])),
            "hue-sat" => new OpenPDN.Adjustments.HueSaturationOp(I(t[2]), I(t[3]), I(t[4])),
            "posterize" => new OpenPDN.Adjustments.PosterizeOp(I(t[2]), I(t[3]), I(t[4])),
            "levels" => new OpenPDN.Adjustments.LevelOp(Hex(t[2]), Hex(t[3]),
                [(float)D(t[4]), (float)D(t[5]), (float)D(t[6])], Hex(t[7]), Hex(t[8])),
            "levels-auto" => new OpenPDN.Adjustments.HistogramRgb(Active.Surface, _doc.Selection).MakeLevelsAuto(),
            "curves-lum" => new OpenPDN.Adjustments.LuminosityCurveOp(OpenPDN.Adjustments.SplineInterpolator.BuildLut(Pts(t[2]))),
            "curves-rgb" => new OpenPDN.Adjustments.ChannelCurveOp(
                OpenPDN.Adjustments.SplineInterpolator.BuildLut(Pts(t[2])),
                OpenPDN.Adjustments.SplineInterpolator.BuildLut(Pts(t[3])),
                OpenPDN.Adjustments.SplineInterpolator.BuildLut(Pts(t[4]))),
            "red-eye" => new OpenPDN.Adjustments.RedEyeRemoveOp(I(t[2]), I(t[3])),
            _ => throw new FormatException($"unknown adjustment {t[1]}"),
        };
        op.Apply(Active.Surface, _doc.Selection);
    }

    private void Fx(string[] t)
    {
        OpenPDN.Effects.Effect e = t[1] switch
        {
            "gaussian" => new OpenPDN.Effects.GaussianBlurEffect(I(t[2])),
            "motion" => new OpenPDN.Effects.MotionBlurEffect(D(t[2]), I(t[3]), t[4] == "1"),
            "sharpen" => new OpenPDN.Effects.SharpenEffect(I(t[2])),
            "median" => new OpenPDN.Effects.MedianEffect(I(t[2]), I(t[3])),
            "noise" => new OpenPDN.Effects.AddNoiseEffect(I(t[2]), I(t[3]), D(t[4]), I(t[5])),
            "pixelate" => new OpenPDN.Effects.PixelateEffect(I(t[2])),
            "emboss" => new OpenPDN.Effects.EmbossEffect(D(t[2])),
            "edge" => new OpenPDN.Effects.EdgeDetectEffect(D(t[2])),
            "relief" => new OpenPDN.Effects.ReliefEffect(D(t[2])),
            "oil" => new OpenPDN.Effects.OilPaintingEffect(I(t[2]), I(t[3])),
            "unfocus" => new OpenPDN.Effects.UnfocusEffect(I(t[2])),
            "fragment" => new OpenPDN.Effects.FragmentEffect(I(t[2]), I(t[3]), D(t[4])),
            "radial" => new OpenPDN.Effects.RadialBlurEffect(D(t[2]), D(t[3]), D(t[4]), I(t[5])),
            "twist" => new OpenPDN.Effects.TwistEffect(D(t[2]), D(t[3]), D(t[4]), D(t[5]), I(t[6])),
            "bulge" => new OpenPDN.Effects.BulgeEffect(D(t[2]), D(t[3]), D(t[4])),
            "polar" => new OpenPDN.Effects.PolarInversionEffect(D(t[2]), D(t[3]), D(t[4]), Enum.Parse<OpenPDN.Effects.WarpEdgeBehavior>(t[5], true), I(t[6])),
            "frosted" => new OpenPDN.Effects.FrostedGlassEffect(D(t[2]), D(t[3]), I(t[4]), I(t[5])),
            "tile" => new OpenPDN.Effects.TileReflectionEffect(D(t[2]), D(t[3]), D(t[4]), I(t[5])),
            "clouds" => new OpenPDN.Effects.CloudsEffect(I(t[2]), D(t[3]), I(t[4]), Hex(t[5]), Hex(t[6]), Enum.Parse<BlendMode>(t[7], true)),
            "julia" => new OpenPDN.Effects.JuliaFractalEffect(D(t[2]), D(t[3]), D(t[4]), I(t[5])),
            "mandelbrot" => new OpenPDN.Effects.MandelbrotFractalEffect(I(t[2]), D(t[3]), D(t[4]), I(t[5]), t[6] == "1"),
            "ink" => new OpenPDN.Effects.InkSketchEffect(I(t[2]), I(t[3])),
            "pencil" => new OpenPDN.Effects.PencilSketchEffect(I(t[2]), I(t[3])),
            "soften" => new OpenPDN.Effects.SoftenPortraitEffect(I(t[2]), I(t[3]), I(t[4])),
            "reduce-noise" => new OpenPDN.Effects.ReduceNoiseEffect(I(t[2]), D(t[3])),
            "glow" => new OpenPDN.Effects.GlowEffect(I(t[2]), I(t[3]), I(t[4])),
            "linocut" => new OpenPDN.Effects.LinocutEffect(Hex(t[2]), Hex(t[3]), I(t[4]), I(t[5]), I(t[6]), I(t[7])),
            "pointillism" => new OpenPDN.Effects.PointillismEffect(I(t[2]), I(t[3]), I(t[4]), I(t[5]), Hex(t[6])),
            "stained" => new OpenPDN.Effects.StainedGlassEffect(I(t[2]), I(t[3]), I(t[4]), I(t[5]), t[6] == "1", Hex(t[7])),
            "mosaic" => new OpenPDN.Effects.MosaicEffect(I(t[2]), I(t[3]), I(t[4]), I(t[5]), Hex(t[6])),
            "spherize" => new OpenPDN.Effects.SpherizeEffect(I(t[2]), I(t[3]), I(t[4]), I(t[5]), I(t[6])),
            "waves" => new OpenPDN.Effects.WavesEffect(Enum.Parse<OpenPDN.Effects.WaveType>(t[2], true), I(t[3]), I(t[4]), I(t[5]), I(t[6]), I(t[7]), I(t[8])),
            "halftone" => new OpenPDN.Effects.HalftoneEffect(Enum.Parse<OpenPDN.Effects.HalftoneMode>(t[2], true), I(t[3]), I(t[4]), Hex(t[5]), Hex(t[6])),
            "dither" => new OpenPDN.Effects.OrderedDitherEffect(Enum.Parse<OpenPDN.Effects.DitherPattern>(t[2], true), I(t[3]), t[4] == "1", I(t[5])),
            _ => throw new FormatException($"unknown effect {t[1]}"),
        };
        e.Apply(Active.Surface, _doc.Selection);
    }

    private void Swap(Document d)
    {
        _doc = d;
        _active = Math.Min(_active, d.Layers.Count - 1);
    }

    private void Set(string key, string v)
    {
        switch (key)
        {
            case "brush": _tools.BrushSize = I(v); break;
            case "aa": _tools.Antialiasing = v == "1"; break;
            case "alphablend": _tools.AlphaBlending = v == "1"; break;
            case "shape": _tools.ShapeMode = Enum.Parse<ShapeDrawMode>(v, true); break;
            case "gradient": _tools.GradientType = Enum.Parse<GradientType>(v, true); break;
            case "gradmode": _tools.GradientColorMode = Enum.Parse<GradientColorMode>(v, true); break;
            case "sampleall": _tools.SampleAllLayers = v == "1"; break;
            case "eraser": _tools.EraserPattern = Enum.Parse<FillPattern>(v, true); break;
            case "resampling": _tools.Resampling = Enum.Parse<TransformResampling>(v, true); break;
            case "pressure": _tools.PressureSensitivity = v == "1"; break;
            case "tolerance": _tools.WandTolerance = I(v); break;
            case "combine": _tools.SelectionCombineMode = Enum.Parse<SelectionCombineMode>(v, true); break;
            default: throw new FormatException($"unknown setting {key}");
        }
    }

    private void Dump(string name, Surface s)
    {
        if (!write) return;
        // Format: u32 width, u32 height, BGRA pixels, u32 mask length, mask bytes.
        using var f = File.Create(Path.Combine(outDir, name + ".bin"));
        using var w = new BinaryWriter(f);
        w.Write((uint)s.Width);
        w.Write((uint)s.Height);
        w.Write(s.AsByteSpan());
        var sel = _doc.Selection;
        if (sel is null || sel.Width != s.Width || sel.Height != s.Height)
        {
            w.Write(0u);
        }
        else
        {
            w.Write((uint)sel.Mask.Length);
            w.Write(sel.Mask);
        }
    }
}
