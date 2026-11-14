#!/usr/bin/env python3
"""Writes PNG fixtures covering every colour type, bit depth, filter type,
tRNS form and Adam7 interlacing into fixtures/png/ (git-ignored)."""
import os, random, struct, zlib

out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "png")
os.makedirs(out, exist_ok=True)

def chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)

def pack_row(samples, depth):
    if depth == 8:
        return bytes(samples)
    if depth == 16:
        return b"".join(struct.pack(">H", s) for s in samples)
    o, acc, n = bytearray(), 0, 0
    for s in samples:
        acc, n = (acc << depth) | s, n + depth
        if n == 8:
            o.append(acc); acc, n = 0, 0
    if n:
        o.append(acc << (8 - n))
    return bytes(o)

def write(name, w, h, color, depth, pix, interlace=False, plte=None, trns=None):
    raw = bytearray()
    passes = [(0,0,8,8),(4,0,8,8),(0,4,4,8),(2,0,4,4),(0,2,2,4),(1,0,2,2),(0,1,1,2)] if interlace else [(0,0,1,1)]
    rnd = random.Random(name)
    bpp = max(1, (len(pix(0, 0)) * depth + 7) // 8)
    for (x0, y0, dx, dy) in passes:
        if x0 >= w or y0 >= h:
            continue
        xs, prev = list(range(x0, w, dx)), None
        for y in range(y0, h, dy):
            row = pack_row([s for x in xs for s in pix(x, y)], depth)
            ft = rnd.randrange(5)  # exercise every filter
            pb = prev if prev is not None else bytes(len(row))
            f = bytearray(len(row))
            for i in range(len(row)):
                a = row[i - bpp] if i >= bpp else 0
                b = pb[i]
                c = pb[i - bpp] if i >= bpp else 0
                if ft == 0: p = 0
                elif ft == 1: p = a
                elif ft == 2: p = b
                elif ft == 3: p = (a + b) // 2
                else:
                    q = a + b - c
                    pa, pb_, pc = abs(q - a), abs(q - b), abs(q - c)
                    p = a if pa <= pb_ and pa <= pc else (b if pb_ <= pc else c)
                f[i] = (row[i] - p) & 255
            raw += bytes([ft]) + f
            prev = row
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, color, 0, 0, int(interlace)))
    if plte: data += chunk(b"PLTE", plte)
    if trns: data += chunk(b"tRNS", trns)
    data += chunk(b"tEXt", b"Comment\0fixture " + name.encode())
    data += chunk(b"IDAT", zlib.compress(bytes(raw))) + chunk(b"IEND", b"")
    open(os.path.join(out, name), "wb").write(data)

W, H = 37, 29
for d in (1, 2, 4, 8, 16):
    m = (1 << d) - 1
    write(f"gray{d}.png", W, H, 0, d, lambda x, y, m=m: ((x * 3 + y * 5) % (m + 1),))
    write(f"gray{d}-i.png", W, H, 0, d, lambda x, y, m=m: ((x * 7 + y) % (m + 1),), interlace=True)
write("gray16-trns.png", W, H, 0, 16, lambda x, y: ((x * 1000 + y * 77) % 65536,), trns=struct.pack(">H", 3000))
write("gray8-trns.png", W, H, 0, 8, lambda x, y: ((x * 7 + y) % 256,), trns=struct.pack(">H", 14))
write("gray4-trns.png", W, H, 0, 4, lambda x, y: ((x + y) % 16,), trns=struct.pack(">H", 5))
write("ga8.png", W, H, 4, 8, lambda x, y: ((x * 7) % 256, (y * 9) % 256))
write("ga8-i.png", W, H, 4, 8, lambda x, y: ((x * 7) % 256, (y * 9) % 256), interlace=True)
write("ga16.png", W, H, 4, 16, lambda x, y: ((x * 1777) % 65536, (y * 2311 + x) % 65536))
write("rgb8.png", W, H, 2, 8, lambda x, y: (x * 7 % 256, y * 9 % 256, (x * y) % 256))
write("rgb8-trns.png", W, H, 2, 8, lambda x, y: (x * 7 % 256, y * 9 % 256, 0), trns=struct.pack(">HHH", 14, 18, 0))
write("rgb16.png", W, H, 2, 16, lambda x, y: ((x * 900) % 65536, (y * 1300) % 65536, 500))
write("rgb16-trns.png", W, H, 2, 16, lambda x, y: ((x * 900) % 65536, (y * 1300) % 65536, 500), trns=struct.pack(">HHH", 1800, 3900, 500))
write("rgba8.png", W, H, 6, 8, lambda x, y: (x * 7 % 256, y * 9 % 256, (x * y) % 256, (x * 13 + y * 5) % 256))
write("rgba8-i.png", W, H, 6, 8, lambda x, y: (x * 7 % 256, y * 9 % 256, (x * y) % 256, (x * 13 + y * 5) % 256), interlace=True)
write("rgba16.png", W, H, 6, 16, lambda x, y: ((x * 900 + 7) % 65536, (y * 1300) % 65536, (x * y * 11) % 65536, (x * 2000 + y * 300) % 65536))
write("rgba16-i.png", W, H, 6, 16, lambda x, y: ((x * 900 + 7) % 65536, (y * 1300) % 65536, (x * y * 11) % 65536, (x * 2000 + y * 300) % 65536), interlace=True)
pal = bytes(sum(([i * 16 % 256, (255 - i * 8) % 256, i * 3 % 256] for i in range(256)), []))
write("pal1.png", W, H, 3, 1, lambda x, y: ((x + y) % 2,), plte=pal[:6], trns=bytes([100]))
write("pal2.png", W, H, 3, 2, lambda x, y: ((x * 3 + y) % 4,), plte=pal[:12])
write("pal4-i.png", W, H, 3, 4, lambda x, y: ((x * 3 + y) % 16,), plte=pal[:48], trns=bytes([0, 50, 255, 128]), interlace=True)
write("pal8-trns.png", W, H, 3, 8, lambda x, y: ((x * 31 + y * 7) % 200,), plte=pal[:600], trns=bytes((i * 37) % 256 for i in range(150)))
write("tiny-i.png", 3, 2, 6, 8, lambda x, y: (x * 80, y * 100, 50, 200), interlace=True)
write("one.png", 1, 1, 2, 8, lambda x, y: (1, 2, 3))
