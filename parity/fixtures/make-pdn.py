#!/usr/bin/env python3
"""Writes legacy Paint.NET .pdn fixtures into fixtures/pdn/ (git-ignored).

The object graph follows Paint.NET 3.x's BinaryFormatter layout: Document
{layers, width, height, userMetaData, savedWith} → LayerList (ArrayList
fields `_items`/`_size`/`_version` plus `parent`) → BitmapLayer
{Layer+width/height/properties, properties, surface} → Surface {width,
height, stride, scan0} → MemoryBlock {length64, hasParent, deferred |
pointerData}. Pixels are either deferred (chunked, optionally gzip,
written out of order) or inline in a gzip-compressed graph.
"""
import gzip, os, struct

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pdn")
os.makedirs(OUT, exist_ok=True)

def lps(s):
    b = s.encode("utf-8"); n = len(b); o = bytearray()
    while True:
        c = n & 0x7F; n >>= 7
        o.append(c | (0x80 if n else 0))
        if not n: break
    return bytes(o) + b

i32 = lambda v: struct.pack("<i", v)
i64 = lambda v: struct.pack("<q", v)

class W:
    def __init__(self):
        self.b = bytearray(); self.next_id = 1; self.libs = {}
    def id(self):
        v = self.next_id; self.next_id += 1; return v
    def lib(self, name):
        if name not in self.libs:
            lid = self.id(); self.libs[name] = lid
            self.b += b"\x0c" + i32(lid) + lps(name)
        return self.libs[name]

PRIM = {"bool": (1, lambda v: bytes([v])), "byte": (2, lambda v: bytes([v])), "int": (8, i32), "long": (9, i64)}

def build(layers, w, h, deferred, chunk_version=0, chunk_size=4096):
    """layers: list of dicts {name, visible, opacity, blend, pixels(bytes BGRA), bg}"""
    wr = W(); b = wr.b
    b += b"\x00" + i32(1) + i32(-1) + i32(1) + i32(0)  # header, root id 1
    root = wr.id()  # the header names object 1 as the root
    lib = wr.lib("PaintDotNet.Data, Version=3.36.0.0, Culture=neutral, PublicKeyToken=null")
    ids = {"layers": wr.id(), "meta": wr.id(), "ver": wr.id()}
    # Document (ClassWithMembersAndTypes)
    b += b"\x05" + i32(root) + lps("PaintDotNet.Document") + i32(5)
    for m in ("layers", "width", "height", "userMetaData", "savedWith"): b += lps(m)
    b += bytes([4, 0, 0, 3, 3]) + lps("PaintDotNet.LayerList") + i32(lib) + bytes([8, 8])
    b += lps("System.Collections.Specialized.NameValueCollection") + lps("System.Version") + i32(lib)
    b += b"\x09" + i32(ids["layers"]) + i32(w) + i32(h) + b"\x09" + i32(ids["meta"]) + b"\x09" + i32(ids["ver"])
    # LayerList : ArrayList
    items_id = wr.id()
    b += b"\x05" + i32(ids["layers"]) + lps("PaintDotNet.LayerList") + i32(4)
    for m in ("ArrayList+_items", "ArrayList+_size", "ArrayList+_version", "parent"): b += lps(m)
    b += bytes([5, 0, 0, 4]) + bytes([8, 8]) + lps("PaintDotNet.Document") + i32(lib) + i32(lib)
    b += b"\x09" + i32(items_id) + i32(len(layers)) + i32(len(layers) * 3) + b"\x09" + i32(root)  # cycle back to Document
    # NameValueCollection (system class, members we never read)
    b += b"\x04" + i32(ids["meta"]) + lps("System.Collections.Specialized.NameValueCollection") + i32(3)
    for m in ("ReadOnly", "Count", "Keys"): b += lps(m)
    b += bytes([0, 0, 6]) + bytes([1, 8]) + bytes([0]) + i32(1)
    keys_id = wr.id()
    b += b"\x09" + i32(keys_id)
    b += b"\x11" + i32(keys_id) + i32(1) + b"\x06" + i32(wr.id()) + lps("$exif.tag0")
    # System.Version
    b += b"\x04" + i32(ids["ver"]) + lps("System.Version") + i32(4)
    for m in ("_Major", "_Minor", "_Build", "_Revision"): b += lps(m)
    b += bytes([0, 0, 0, 0]) + bytes([8, 8, 8, 8]) + i32(3) + i32(36) + i32(-1) + i32(-1)
    # items array: object[] with capacity > count (trailing nulls)
    cap = len(layers) + 2
    b += b"\x10" + i32(items_id) + i32(cap)
    meta_id = None; lp_meta = None; bp_meta = None; surf_meta = None; mb_meta = None
    deferred_blocks = []
    pending = []  # referenced objects, written at top level after the array
    for li, L in enumerate(layers):
        lid = wr.id()
        if meta_id is None:
            meta_id = lid
            b += b"\x05" + i32(lid) + lps("PaintDotNet.BitmapLayer") + i32(5)
            for m in ("Layer+width", "Layer+height", "Layer+properties", "properties", "surface"): b += lps(m)
            b += bytes([0, 0, 4, 4, 4]) + bytes([8, 8])
            b += lps("PaintDotNet.Layer+LayerProperties") + i32(lib)
            b += lps("PaintDotNet.BitmapLayer+BitmapLayerProperties") + i32(lib)
            b += lps("PaintDotNet.Surface") + i32(lib) + i32(lib)
        else:
            b += b"\x01" + i32(lid) + i32(meta_id)  # ClassWithId reuses metadata
        b += i32(w) + i32(h)
        # Layer+properties inline (ISerializable: name, userMetaData, visible, isBackground, opacity)
        pid = wr.id()
        if lp_meta is None:
            lp_meta = pid
            b += b"\x05" + i32(pid) + lps("PaintDotNet.Layer+LayerProperties") + i32(5)
            for m in ("name", "userMetaData", "visible", "isBackground", "opacity"): b += lps(m)
            b += bytes([1, 3, 0, 0, 0]) + lps("System.Collections.Specialized.NameValueCollection") + bytes([1, 1, 2]) + i32(lib)
        else:
            b += b"\x01" + i32(pid) + i32(lp_meta)
        b += b"\x06" + i32(wr.id()) + lps(L["name"]) + b"\x0a" + bytes([L["visible"], li == 0, L["opacity"]])
        # BitmapLayer properties: blendOp (class named after the op) + opacity (older files)
        bpid = wr.id()
        if bp_meta is None:
            bp_meta = bpid
            b += b"\x05" + i32(bpid) + lps("PaintDotNet.BitmapLayer+BitmapLayerProperties") + i32(1)
            b += lps("blendOp") + bytes([2]) + i32(lib)
        else:
            b += b"\x01" + i32(bpid) + i32(bp_meta)
        b += b"\x05" + i32(wr.id()) + lps("PaintDotNet.UserBlendOps+" + L["blend"] + "BlendOp") + i32(0) + bytes([]) + i32(lib)
        # Surface
        sid = wr.id()
        if surf_meta is None:
            surf_meta = sid
            b += b"\x05" + i32(sid) + lps("PaintDotNet.Surface") + i32(4)
            for m in ("width", "height", "stride", "scan0"): b += lps(m)
            b += bytes([0, 0, 0, 4]) + bytes([8, 8, 8]) + lps("PaintDotNet.MemoryBlock") + i32(lib) + i32(lib)
        else:
            b += b"\x01" + i32(sid) + i32(surf_meta)
        stride = w * 4 + (8 if L.get("padded") else 0)
        px = L["pixels"]
        if stride != w * 4:
            px = b"".join(px[y * w * 4:(y + 1) * w * 4] + b"\xEE" * (stride - w * 4) for y in range(h))
        b += i32(w) + i32(h) + i32(stride)
        # MemoryBlock (ISerializable; members differ between deferred and inline)
        mid = wr.id()
        if deferred:
            b += b"\x05" + i32(mid) + lps("PaintDotNet.MemoryBlock") + i32(3)
            for m in ("length64", "hasParent", "deferred"): b += lps(m)
            b += bytes([0, 0, 0, 9, 1, 1]) + i32(lib) + i64(len(px)) + bytes([0, 1])
            deferred_blocks.append(px)
        else:
            arr = wr.id()
            b += b"\x05" + i32(mid) + lps("PaintDotNet.MemoryBlock") + i32(3)
            for m in ("length64", "hasParent", "pointerData"): b += lps(m)
            b += bytes([0, 0, 7, 9, 1, 2]) + i32(lib) + i64(len(px)) + bytes([0]) + b"\x09" + i32(arr)
            pending.append(b"\x0f" + i32(arr) + i32(len(px)) + bytes([2]) + px)  # ArraySinglePrimitive<byte>, top level later
    b += b"\x0d" + bytes([cap - len(layers)])  # ObjectNullMultiple256 fills the spare capacity
    for rec in pending:
        b += rec
    b += b"\x0b"
    graph = bytes(b)
    header = b'<pdnImage width="%d" height="%d" layers="%d" savedWithVersion="3.36"><custom><thumb png="" /></custom></pdnImage>' % (w, h, len(layers))
    head = b"PDN3" + bytes([len(header) & 255, (len(header) >> 8) & 255, len(header) >> 16]) + header
    if not deferred:
        return head + gzip.compress(graph)
    out = bytearray(head + b"\x00\x01" + graph)
    for px in deferred_blocks:
        out += bytes([chunk_version]) + struct.pack(">I", chunk_size)
        count = (len(px) + chunk_size - 1) // chunk_size
        order = list(range(count))[::-1]  # out of order on purpose
        for n in order:
            raw = px[n * chunk_size:(n + 1) * chunk_size]
            data = gzip.compress(raw) if chunk_version == 0 else raw
            out += struct.pack(">II", n, len(data)) + data
    return bytes(out)

def pixels(w, h, seed, opaque=False):
    s = seed | 1; o = bytearray()
    for _ in range(w * h):
        s ^= (s << 13) & 0xFFFFFFFF; s ^= s >> 17; s ^= (s << 5) & 0xFFFFFFFF
        o += bytes([s & 255, (s >> 8) & 255, (s >> 16) & 255, 255 if opaque else s >> 24])
    return bytes(o)

W_, H_ = 61, 47
ls = [
    dict(name="Background", visible=1, opacity=255, blend="Normal", pixels=pixels(W_, H_, 1, True)),
    dict(name="Überlagerung ✓", visible=1, opacity=150, blend="Overlay", pixels=pixels(W_, H_, 2)),
    dict(name="Hidden", visible=0, opacity=255, blend="Difference", pixels=pixels(W_, H_, 3), padded=True),
    dict(name="Glow", visible=1, opacity=90, blend="Glow", pixels=pixels(W_, H_, 4)),
]
open(os.path.join(OUT, "deferred-gzip.pdn"), "wb").write(build(ls, W_, H_, True, 0, 1000))
open(os.path.join(OUT, "deferred-raw.pdn"), "wb").write(build(ls[:2], W_, H_, True, 1, 4096))
open(os.path.join(OUT, "inline-gzip.pdn"), "wb").write(build(ls, W_, H_, False))
open(os.path.join(OUT, "single.pdn"), "wb").write(build([dict(ls[0], pixels=pixels(1, 1, 9, True))], 1, 1, True))
