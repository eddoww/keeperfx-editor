#!/usr/bin/env python3
# The application icon is part of the KeeperFX Map Editor project and is
# licensed under GPL-2.0-or-later like the rest of it. It is an original
# drawing produced entirely by this script; it uses no game artwork.
"""Draw the KeeperFX Map Editor application icon and write it in every format.

The mark is an isometric block built from 2x2x2 tiles with the front top tile
dug out, in gold on a dark rounded-square plate. Sizes up to 32 px get a
simplified variant (larger block, no tile grid) so it stays readable.

Run from anywhere (standard library only, output is byte-for-byte deterministic):

    python3 -I tools/make_icon.py [output_dir]

The output directory defaults to packaging/icons in the repository root. Writes
kfx-editor-<N>.png for N in 16, 24, 32, 48, 64, 128, 256, 512, 1024, plus
kfx-editor.ico (16-256), kfx-editor.icns (16-1024) and kfx-editor.svg.
"""
import math
import struct
import sys
import zlib
from pathlib import Path

PNG_SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
# (ICNS type, pixel size). PNG payloads; the ic1x types are the @2x variants.
ICNS_TYPES = (
    (b"icp4", 16), (b"icp5", 32), (b"icp6", 64), (b"ic07", 128), (b"ic08", 256),
    (b"ic09", 512), (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64), (b"ic13", 256),
    (b"ic14", 512),
)
SVG_SIZE = 256

# The editor's palette (RGB).
BACKGROUND = (18, 16, 14)
PANEL = (26, 23, 20)
WIDGET = (38, 34, 30)
GOLD = (214, 170, 72)

# Face shades of the block, lit from the upper left.
TOP = GOLD
LEFT = (166, 127, 53)
RIGHT = (104, 79, 34)  # below the UI's dim gold (120, 92, 40) so the sides separate at 16 px
# The dug-out tile: same light direction as the outside, one step darker.
PIT_FLOOR = (170, 133, 56)
PIT_WALL_Y = (128, 97, 41)  # wall looking +y, like LEFT
PIT_WALL_X = (70, 53, 23)  # wall looking +x, like RIGHT
EDGE = BACKGROUND


# ---------------------------------------------------------------------------
# Geometry


def iso(x, y, z, cx, cy, s):
    """Project block coordinates (x right-forward, y left-forward, z up) to pixels."""
    c = math.sqrt(3) / 2
    return (cx + (x - y) * c * s, cy + ((x + y) * 0.5 - z) * s)


def offset_polygon(poly, d):
    """Grow (d > 0) or shrink (d < 0) a simple polygon by moving each edge by d (miter joins)."""
    n = len(poly)
    area2 = sum(poly[i][0] * poly[(i + 1) % n][1] - poly[(i + 1) % n][0] * poly[i][1] for i in range(n))
    flip = 1.0 if area2 > 0 else -1.0  # for area2 > 0 the normal (dy, -dx) points outward
    lines = []
    for i in range(n):
        (x0, y0), (x1, y1) = poly[i], poly[(i + 1) % n]
        length = math.hypot(x1 - x0, y1 - y0)
        nx, ny = (y1 - y0) / length * flip, (x0 - x1) / length * flip
        lines.append(((x0 + nx * d, y0 + ny * d), (x1 - x0, y1 - y0)))
    out = []
    for i in range(n):
        (p, r), (q, t) = lines[i - 1], lines[i]
        cross = r[0] * t[1] - r[1] * t[0]
        u = ((q[0] - p[0]) * t[1] - (q[1] - p[1]) * t[0]) / cross
        out.append((p[0] + r[0] * u, p[1] + r[1] * u))
    return out


def rounded_square(x0, y0, x1, y1, radius, steps=24):
    """Polygon of a rounded rectangle, corners approximated by `steps` segments each."""
    corners = ((x1 - radius, y0 + radius, -90), (x1 - radius, y1 - radius, 0),
               (x0 + radius, y1 - radius, 90), (x0 + radius, y0 + radius, 180))
    poly = []
    for cx, cy, start in corners:
        for k in range(steps + 1):
            a = math.radians(start + 90 * k / steps)
            poly.append((cx + radius * math.cos(a), cy + radius * math.sin(a)))
    return poly


def design(size):
    """The icon at `size` px as a back-to-front list of (polygon, rgb) in pixel coordinates."""
    small = size <= 32
    shapes = []

    # Plate: a rounded square with a one-step lighter rim. Small icons use the full square.
    margin = 0.0 if small else size / 16
    radius = (size - 2 * margin) * 0.2
    rim = max(1.0, size / 64)
    shapes.append((rounded_square(margin, margin, size - margin, size - margin, radius), WIDGET))
    shapes.append((rounded_square(margin + rim, margin + rim, size - margin - rim, size - margin - rim,
                                  radius - rim), PANEL))

    # Block: 2x2x2 tiles, the front top tile (x, y, z in [1, 2]) removed.
    if small:
        # Snap the block's left and right sides to whole pixels; no line work,
        # the face shades alone carry the shape.
        half_width = round(size * 0.35)
        s = half_width / math.sqrt(3)  # one tile edge, in pixels
        edge = gap = 0.0
    else:
        s = size * 0.165
        edge = max(1.5, size * 0.018)  # dark outline around the block
        gap = edge  # dark line between faces / tiles
    cx, cy = size / 2, size / 2

    def p(x, y, z):
        return iso(x, y, z, cx, cy, s)

    silhouette = [p(0, 0, 2), p(2, 0, 2), p(2, 0, 0), p(2, 2, 0), p(0, 2, 0), p(0, 2, 2)]
    if edge:
        shapes.append((offset_polygon(silhouette, edge), EDGE))

    # Each face as its tiles (corners in block coordinates), drawn inset so the
    # dark silhouette shows through as the line work.
    def top(x, y, z=2):
        return [(x, y, z), (x + 1, y, z), (x + 1, y + 1, z), (x, y + 1, z)]

    def left(x, z, y=2):  # face looking +y
        return [(x, y, z), (x + 1, y, z), (x + 1, y, z + 1), (x, y, z + 1)]

    def right(y, z, x=2):  # face looking +x
        return [(x, y, z), (x, y + 1, z), (x, y + 1, z + 1), (x, y, z + 1)]

    if small:
        # Whole faces only (L-shaped), no tile grid.
        faces = [
            ([(0, 0, 2), (2, 0, 2), (2, 1, 2), (1, 1, 2), (1, 2, 2), (0, 2, 2)], TOP),
            ([(0, 2, 2), (1, 2, 2), (1, 2, 1), (2, 2, 1), (2, 2, 0), (0, 2, 0)], LEFT),
            ([(2, 0, 2), (2, 1, 2), (2, 1, 1), (2, 2, 1), (2, 2, 0), (2, 0, 0)], RIGHT),
        ]
    else:
        faces = [(top(0, 0), TOP), (top(1, 0), TOP), (top(0, 1), TOP),
                 (left(0, 0), LEFT), (left(1, 0), LEFT), (left(0, 1), LEFT),
                 (right(0, 0), RIGHT), (right(1, 0), RIGHT), (right(0, 1), RIGHT)]
    faces += [(top(1, 1, z=1), PIT_FLOOR), (left(1, 1, y=1), PIT_WALL_Y),
              (right(1, 1, x=1), PIT_WALL_X)]
    for corners, rgb in faces:
        poly = [p(*c) for c in corners]
        shapes.append((offset_polygon(poly, -gap / 2) if gap else poly, rgb))

    return shapes


# ---------------------------------------------------------------------------
# Rasteriser: exact horizontal coverage, `sub` sample rows per pixel row.


def coverage_rows(poly, width, height, sub):
    """Yield (y, x0, coverages) for each pixel row touched by `poly` (nonzero fill rule)."""
    n = len(poly)
    edges = []
    for i in range(n):
        (xa, ya), (xb, yb) = poly[i], poly[(i + 1) % n]
        if ya != yb:
            edges.append((xa, ya, xb, yb, 1) if ya < yb else (xb, yb, xa, ya, -1))
    xs = [x for x, _ in poly]
    ys = [y for _, y in poly]
    bx0, bx1 = max(0, math.floor(min(xs))), min(width, math.ceil(max(xs)))
    by0, by1 = max(0, math.floor(min(ys))), min(height, math.ceil(max(ys)))
    span = bx1 - bx0
    if span <= 0:
        return
    for py in range(by0, by1):
        acc = [0.0] * (span + 1)
        full = [0] * (span + 2)
        for k in range(sub):
            yc = py + (k + 0.5) / sub
            hits = []
            for xa, ya, xb, yb, wind in edges:
                if ya <= yc < yb:
                    hits.append((xa + (yc - ya) * (xb - xa) / (yb - ya), wind))
            hits.sort()
            winding = 0
            for x, wind in hits:
                if winding == 0:
                    start = x
                winding += wind
                if winding == 0:
                    lo, hi = max(start, bx0) - bx0, min(x, bx1) - bx0
                    if hi <= lo:
                        continue
                    ia, ib = int(lo), int(hi)
                    if ia == ib:
                        acc[ia] += hi - lo
                    else:
                        acc[ia] += ia + 1 - lo
                        full[ia + 1] += 1
                        full[ib] -= 1
                        acc[ib] += hi - ib
        run = 0
        row = []
        for i in range(span):
            run += full[i]
            row.append((acc[i] + run) / sub)
        yield py, bx0, row


def render(shapes, size):
    """Composite the shapes (opaque colours, antialiased edges) into premultiplied RGBA floats."""
    sub = 16 if size <= 128 else 8
    pix = [[0.0, 0.0, 0.0, 0.0] for _ in range(size * size)]
    for poly, rgb in shapes:
        r, g, b = (c / 255 for c in rgb)
        for y, x0, row in coverage_rows(poly, size, size, sub):
            base = y * size + x0
            for i, a in enumerate(row):
                if a <= 0.0:
                    continue
                a = min(a, 1.0)
                px = pix[base + i]
                keep = 1.0 - a
                px[0] = r * a + px[0] * keep
                px[1] = g * a + px[1] * keep
                px[2] = b * a + px[2] * keep
                px[3] = a + px[3] * keep
    out = bytearray()
    for r, g, b, a in pix:
        if a <= 0.0:
            out += b"\0\0\0\0"
        else:
            out += bytes((round(min(1.0, r / a) * 255), round(min(1.0, g / a) * 255),
                          round(min(1.0, b / a) * 255), round(min(1.0, a) * 255)))
    return bytes(out)


# ---------------------------------------------------------------------------
# Encoders


def png(rgba, size):
    """Encode square 8-bit RGBA pixels as a PNG (filter Sub on every row, zlib level 9)."""
    stride = size * 4
    raw = bytearray()
    for y in range(size):
        row = rgba[y * stride:(y + 1) * stride]
        raw.append(1)
        raw += row[:4]
        raw += bytes((row[i] - row[i - 4]) & 0xFF for i in range(4, stride))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + chunk(b"IEND", b""))


def ico(pngs):
    """Windows icon with one PNG-compressed entry per size; `pngs` maps size -> PNG bytes."""
    sizes = sorted(pngs)
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = len(header) + 16 * len(sizes)
    directory, data = b"", b""
    for size in sizes:
        blob = pngs[size]
        dim = 0 if size >= 256 else size  # 0 means 256
        directory += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset + len(data))
        data += blob
    return header + directory + data


def icns(entries):
    """macOS icon family; `entries` is a list of (4-byte type, PNG bytes)."""
    body = b"".join(kind + struct.pack(">I", 8 + len(blob)) + blob for kind, blob in entries)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def svg(shapes, size):
    """The same drawing as an SVG document (polygons only)."""
    lines = ['<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" viewBox="0 0 %d %d">'
             % (size, size, size, size)]
    for poly, rgb in shapes:
        points = " ".join("%.2f,%.2f" % (x, y) for x, y in poly)
        lines.append('  <polygon fill="#%02x%02x%02x" points="%s"/>' % (rgb + (points,)))
    lines.append("</svg>")
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------


def main():
    repo = Path(__file__).resolve().parent.parent
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else repo / "packaging" / "icons"
    out.mkdir(parents=True, exist_ok=True)
    pngs = {}
    for size in PNG_SIZES:
        pngs[size] = png(render(design(size), size), size)
        (out / ("kfx-editor-%d.png" % size)).write_bytes(pngs[size])
    (out / "kfx-editor.ico").write_bytes(ico({s: pngs[s] for s in ICO_SIZES}))
    (out / "kfx-editor.icns").write_bytes(icns([(kind, pngs[s]) for kind, s in ICNS_TYPES]))
    (out / "kfx-editor.svg").write_text(svg(design(SVG_SIZE), SVG_SIZE))


if __name__ == "__main__":
    main()
