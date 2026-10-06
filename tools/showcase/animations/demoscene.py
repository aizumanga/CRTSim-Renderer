#!/usr/bin/env python3
"""Original late-80s/early-90s style demoscene intro, 320x240, 240 frames (8 s @ 30 fps).

Three parts cut on a 120 BPM beat (one beat = 15 frames), joined by dithered wipes:
  1. copper bars bouncing in a sine train behind a wobbling chrome "CRTSIM" logo
  2. palette-cycled plasma with a flat-shaded, dithered icosahedron with a glowing outline
  3. palette-cycled checker tunnel with a glowing wireframe torus
A chunky bevelled bitmap-font sine scroller runs across the bottom throughout.
Everything is periodic over 240 frames, so the clip loops seamlessly.

Usage: python3 demoscene.py OUT_DIR
"""
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

W, H = 320, 240
FRAMES = 240
BEAT = 15  # frames per beat at 120 BPM / 30 fps
TAU = 2.0 * math.pi

# --------------------------------------------------------------------------- font
FONT_SRC = {
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "B": ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
    "C": [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."],
    "D": ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
    "E": ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
    "F": ["#####", "#....", "#....", "####.", "#....", "#....", "#...."],
    "G": [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".####"],
    "H": ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "I": [".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."],
    "J": ["..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##.."],
    "K": ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"],
    "L": ["#....", "#....", "#....", "#....", "#....", "#....", "#####"],
    "M": ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"],
    "N": ["#...#", "#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#"],
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "P": ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
    "Q": [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "S": [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "U": ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "V": ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
    "W": ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "#.#.#", ".#.#."],
    "X": ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"],
    "Y": ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
    "Z": ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"],
    ".": [".....", ".....", ".....", ".....", ".....", ".##..", ".##.."],
    "!": ["..#..", "..#..", "..#..", "..#..", "..#..", ".....", "..#.."],
    "*": [".....", "#.#.#", ".###.", "#####", ".###.", "#.#.#", "....."],
    "-": [".....", ".....", ".....", ".###.", ".....", ".....", "....."],
    " ": ["....."] * 7,
}
FONT = {k: np.array([[c == "#" for c in row] for row in v], dtype=bool) for k, v in FONT_SRC.items()}


def text_mask(text, scale, gap=1):
    """Boolean mask of `text` with each font pixel a scale x scale block; also returns
    per-pixel 'bevel' shading (+1 highlight, -1 shade, 0 face)."""
    cols = []
    for i, ch in enumerate(text):
        g = FONT[ch]
        cols.append(g)
        if i != len(text) - 1:
            cols.append(np.zeros((7, gap), bool))
    m = np.concatenate(cols, axis=1)
    big = np.kron(m, np.ones((scale, scale), bool))
    bev = np.zeros(big.shape, np.int8)
    yy, xx = np.mgrid[0:big.shape[0], 0:big.shape[1]]
    ly, lx = yy % scale, xx % scale
    bev[(ly == 0) | (lx == 0)] = 1
    bev[(ly == scale - 1) | (lx == scale - 1)] = -1
    bev[~big] = 0
    return big, bev


def dilate(m, r=1):
    out = m.copy()
    for dy in range(-r, r + 1):
        for dx in range(-r, r + 1):
            out |= np.roll(np.roll(m, dy, 0), dx, 1)
    return out


# --------------------------------------------------------------------------- helpers
def bayer(n):
    m = np.array([[0]])
    while m.shape[0] < n:
        m = np.block([[4 * m, 4 * m + 2], [4 * m + 3, 4 * m + 1]])
    return (m + 0.5) / (n * n)


B8 = np.tile(bayer(8), (H // 8, W // 8))  # thresholds in (0,1), shape (H,W)
B4 = np.tile(bayer(4), (H // 4, W // 4))
YY, XX = np.mgrid[0:H, 0:W].astype(np.float32)


def lerp(a, b, t):
    return a + (b - a) * t


def ramp(stops, n):
    """Palette of n colours interpolated through (pos, (r,g,b)) stops, pos in [0,1]."""
    out = np.zeros((n, 3), np.float32)
    ps = [s[0] for s in stops]
    cs = [np.array(s[1], np.float32) for s in stops]
    for i in range(n):
        p = i / (n - 1) if n > 1 else 0
        for k in range(len(ps) - 1):
            if ps[k] <= p <= ps[k + 1]:
                f = (p - ps[k]) / max(1e-6, ps[k + 1] - ps[k])
                out[i] = lerp(cs[k], cs[k + 1], f)
                break
    return out


def cyc_ramp(stops, n):
    """Cyclic palette: stops evenly spaced, wrapping last->first."""
    cs = [np.array(c, np.float32) for c in stops]
    k = len(cs)
    out = np.zeros((n, 3), np.float32)
    for i in range(n):
        p = i / n * k
        a = int(p) % k
        f = p - int(p)
        f = f * f * (3 - 2 * f)
        out[i] = lerp(cs[a], cs[(a + 1) % k], f)
    return out


def beat_pulse(t):
    return math.exp(-(t % BEAT) / 3.5)


def glow_add(base, lines, strength=1.3, radius=2.5):
    """Add `lines` (float RGB) plus a blurred halo of it onto base."""
    img = Image.fromarray(np.clip(lines, 0, 255).astype(np.uint8))
    g1 = np.asarray(img.filter(ImageFilter.GaussianBlur(radius)), np.float32)
    g2 = np.asarray(img.filter(ImageFilter.GaussianBlur(radius * 2.4)), np.float32)
    return base + lines + g1 * strength + g2 * strength * 0.8


def rot(ax, ay, az):
    ca, sa = math.cos(ax), math.sin(ax)
    cb, sb = math.cos(ay), math.sin(ay)
    cc, sc = math.cos(az), math.sin(az)
    rx = np.array([[1, 0, 0], [0, ca, -sa], [0, sa, ca]])
    ry = np.array([[cb, 0, sb], [0, 1, 0], [-sb, 0, cb]])
    rz = np.array([[cc, -sc, 0], [sc, cc, 0], [0, 0, 1]])
    return rz @ ry @ rx


def project(v, cx, cy, scale, dist=4.0):
    z = v[:, 2] + dist
    f = scale * dist / z
    return np.stack([cx + v[:, 0] * f, cy + v[:, 1] * f], 1), z


# --------------------------------------------------------------------------- part 1: copper bars + logo
STAR_RNG = np.random.default_rng(1989)
STARS = []
for layer, (n, speed, col) in enumerate([(50, 4 / 3, (70, 70, 120)), (30, 8 / 3, (130, 130, 200)), (16, 4.0, (210, 210, 255))]):
    xs = STAR_RNG.uniform(0, W, n)
    ys = STAR_RNG.integers(0, H, n)
    STARS.append((xs, ys, speed, np.array(col, np.float32)))

BAR_HUES = [(255, 40, 60), (255, 140, 0), (255, 230, 20), (40, 230, 60), (0, 200, 255), (60, 80, 255), (210, 40, 255)]
BAR_H = 21
_prof = np.cos(np.linspace(-math.pi / 2, math.pi / 2, BAR_H))
BAR_PROFILE = np.round(_prof * 7) / 7.0  # quantised to 8 copper steps
BAR_SHEEN = np.clip((_prof - 0.75) / 0.25, 0, 1) * 0.45

LOGO_M, LOGO_B = text_mask("CRTSIM", 8, gap=1)
LOGO_H, LOGO_W = LOGO_M.shape
LOGO_PAD = 3
LOGO_PM = np.pad(LOGO_M, LOGO_PAD)
LOGO_PH = LOGO_PM.shape[0]
LOGO_OUT = dilate(LOGO_PM, 2)
# chrome-style per-row gradient: sky above horizon, warm ground below
_lh = LOGO_H
LOGO_GRAD = np.concatenate([
    ramp([(0, (40, 90, 255)), (0.6, (120, 220, 255)), (1, (225, 250, 255))], _lh // 2),
    ramp([(0, (90, 20, 60)), (0.35, (230, 60, 90)), (0.75, (255, 170, 40)), (1, (255, 240, 120))], _lh - _lh // 2),
])
SUB_M, SUB_B = text_mask("PHOSPHOR FORCE", 2, gap=1)
SUB_M, SUB_B = np.pad(SUB_M, 2), np.pad(SUB_B, 2)
SUB_OUT = dilate(SUB_M, 1)


def stamp(img, mask, outline, colors, x0, y0, shadow=(0, 0, 0)):
    """Draw mask at (x0,y0) clipped to screen; colors is (h,w,3) or (h,1,3)."""
    h, w = mask.shape
    xa, ya = max(0, x0), max(0, y0)
    xb, yb = min(W, x0 + w), min(H, y0 + h)
    if xa >= xb or ya >= yb:
        return
    sm = mask[ya - y0:yb - y0, xa - x0:xb - x0]
    so = outline[ya - y0:yb - y0, xa - x0:xb - x0]
    c = np.broadcast_to(colors, (h, w, 3))[ya - y0:yb - y0, xa - x0:xb - x0]
    reg = img[ya:yb, xa:xb]
    reg[so & ~sm] = shadow
    reg[sm] = c[sm]


def part_copper(t):
    img = np.zeros((H, W, 3), np.float32)
    # dark navy background gradient, dithered into 4 steps
    g = (YY / H) * 3 + B4
    lvl = np.floor(g) / 3.0
    img[..., 0] = 6 + lvl * 10
    img[..., 1] = 2 + lvl * 4
    img[..., 2] = 18 + lvl * 26
    # starfield (periodic: every layer wraps an integer number of times in 240 frames)
    for xs, ys, speed, col in STARS:
        x = ((xs + speed * t) % W).astype(int)
        img[ys, x] = col
    bp = beat_pulse(t)
    # copper bars: a sine train, drawn back to front
    bars = []
    for i, hue in enumerate(BAR_HUES):
        ph = TAU * 3 * t / FRAMES - i * 0.48 + math.pi
        y = 104 + 84 * math.sin(ph)
        z = math.cos(ph)
        bars.append((z, y, np.array(hue, np.float32)))
    bars.sort(key=lambda b: b[0])
    for z, y, hue in bars:
        depth = 0.55 + 0.45 * (z + 1) / 2
        y0 = int(round(y)) - BAR_H // 2
        for k in range(BAR_H):
            yy = y0 + k
            if 0 <= yy < H:
                c = hue * BAR_PROFILE[k] * depth * (1 + 0.25 * bp)
                c = c + (255 - c) * BAR_SHEEN[k] * depth
                img[yy, :] = c
    # logo: bounces, landing on every beat; per-scanline sine wobble
    bounce = abs(math.sin(math.pi * t / BEAT))
    ly = 22 + int(round(-12 * bounce)) + 14
    lx = (W - LOGO_W) // 2
    shade = np.where(LOGO_B > 0, 1.25, np.where(LOGO_B < 0, 0.6, 1.0))[..., None]
    cols = np.clip(LOGO_GRAD[:, None, :] * shade, 0, 255)
    for r in range(LOGO_PH):
        yy = ly + r - LOGO_PAD
        if not (0 <= yy < H):
            continue
        dx = int(round(3 * math.sin(r * 0.075 + TAU * 4 * t / FRAMES)))
        x0 = lx + dx - LOGO_PAD
        mrow, orow = LOGO_PM[r], LOGO_OUT[r]
        xs = np.nonzero(orow & ~mrow)[0]
        xd = xs + x0
        ok = (xd >= 0) & (xd < W)
        img[yy, xd[ok]] = (0, 0, 0)
        xs = np.nonzero(mrow)[0]
        if len(xs) == 0:
            continue
        xd = xs + x0
        ok = (xd >= 0) & (xd < W)
        img[yy, xd[ok]] = cols[r - LOGO_PAD, xs[ok] - LOGO_PAD]
    # subtitle, rainbow cycling per column
    sh, sw = SUB_M.shape
    sx, sy = (W - sw) // 2, ly + LOGO_H + 12
    hue = (np.arange(sw) / 40.0 - t / 30.0) % 1.0
    sub_pal = cyc_ramp([(255, 60, 200), (255, 220, 40), (40, 255, 200), (90, 120, 255)], 64)
    scol = sub_pal[(hue * 64).astype(int) % 64][None, :, :] * np.ones((sh, 1, 1), np.float32)
    stamp(img, SUB_M, SUB_OUT, scol, sx, sy)
    return img


# --------------------------------------------------------------------------- part 2: plasma + icosahedron
PLASMA_PAL = cyc_ramp([(20, 0, 50), (110, 0, 120), (220, 20, 110), (255, 110, 30), (150, 20, 60), (40, 0, 90)], 32)
ICO_PAL = ramp([(0, (4, 10, 50)), (0.3, (10, 60, 170)), (0.65, (20, 170, 240)), (1, (180, 250, 255))], 12)

_phi = (1 + 5 ** 0.5) / 2
ICO_V = np.array([[-1, _phi, 0], [1, _phi, 0], [-1, -_phi, 0], [1, -_phi, 0],
                  [0, -1, _phi], [0, 1, _phi], [0, -1, -_phi], [0, 1, -_phi],
                  [_phi, 0, -1], [_phi, 0, 1], [-_phi, 0, -1], [-_phi, 0, 1]], float)
ICO_V /= np.linalg.norm(ICO_V[0])
ICO_F = []
for a in range(12):
    for b in range(a + 1, 12):
        for c in range(b + 1, 12):
            d = [np.linalg.norm(ICO_V[a] - ICO_V[b]), np.linalg.norm(ICO_V[b] - ICO_V[c]), np.linalg.norm(ICO_V[a] - ICO_V[c])]
            if max(d) < 1.1:
                n = np.cross(ICO_V[b] - ICO_V[a], ICO_V[c] - ICO_V[a])
                if np.dot(n, ICO_V[a]) < 0:
                    b_, c_ = c, b
                else:
                    b_, c_ = b, c
                ICO_F.append((a, b_, c_))
LIGHT = np.array([-0.5, -0.7, -0.9])
LIGHT /= np.linalg.norm(LIGHT)


def part_plasma(t):
    a = TAU * t / FRAMES
    v = (np.sin(XX * 0.045 + a * 3)
         + np.sin(YY * 0.06 - a * 2)
         + np.sin((XX + YY) * 0.03 + a * 4)
         + np.sin(np.sqrt((XX - 160 - 60 * math.sin(a * 2)) ** 2 + (YY - 120 - 40 * math.cos(a * 3)) ** 2) * 0.07 - a * 5))
    idx = np.floor(v * 6 + t * 0.9 + B8).astype(int) % 32
    img = PLASMA_PAL[idx] * (0.85 + 0.25 * beat_pulse(t))
    # icosahedron
    bp = beat_pulse(t)
    R = rot(a * 2 + 0.3, a * 3, a * 1 + 0.2)
    vv = ICO_V @ R.T
    cx = 160 + 34 * math.sin(a * 2)
    cy = 92 + 10 * math.sin(a * 4)
    p2, z = project(vv, cx, cy, 64 * (1 + 0.10 * bp))
    fid = Image.new("L", (W, H), 0)
    dr = ImageDraw.Draw(fid)
    shades = np.zeros(256, np.float32)
    vis = []
    for k, (i, j, m) in enumerate(ICO_F):
        n = np.cross(vv[j] - vv[i], vv[m] - vv[i])
        n /= np.linalg.norm(n)
        if n[2] >= 0:  # facing away (camera looks down +z)
            continue
        vis.append((i, j, m))
        dr.polygon([tuple(p2[i]), tuple(p2[j]), tuple(p2[m])], fill=k + 1)
        shades[k + 1] = 0.12 + 0.88 * max(0.0, float(np.dot(n, LIGHT)))
    f = np.asarray(fid)
    mask = f > 0
    # dark drop shadow halo so the solid separates from the plasma
    halo = np.asarray(Image.fromarray((mask * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(6)), np.float32) / 255
    img *= (1 - 0.75 * np.clip(halo * 1.6, 0, 1))[..., None]
    s = shades[f]
    q = np.clip(np.floor(s * (len(ICO_PAL) - 1) + B4), 0, len(ICO_PAL) - 1).astype(int)
    img[mask] = ICO_PAL[q][mask]
    # glowing outline
    lines = Image.new("RGB", (W, H), (0, 0, 0))
    ld = ImageDraw.Draw(lines)
    edges = set()
    for i, j, m in vis:
        for e in ((i, j), (j, m), (m, i)):
            edges.add(tuple(sorted(e)))
    for i, j in edges:
        ld.line([tuple(p2[i]), tuple(p2[j])], fill=(140, 255, 255), width=1)
    for i in {x for e in edges for x in e}:
        x, y = p2[i]
        ld.rectangle([x - 1, y - 1, x + 1, y + 1], fill=(255, 255, 210))
    ln = np.asarray(lines, np.float32)
    img = glow_add(img, ln * np.array([1.0, 1.0, 1.0]), strength=0.9 + 0.6 * bp, radius=2.0)
    return img


# --------------------------------------------------------------------------- part 3: tunnel + torus
TUN_PAL = cyc_ramp([(0, 40, 140), (0, 190, 255), (40, 255, 170), (0, 120, 200), (90, 30, 220), (200, 60, 255)], 32)
TOR_PAL = ramp([(0, (90, 0, 40)), (0.4, (230, 40, 70)), (0.75, (255, 160, 20)), (1, (255, 245, 150))], 32)
_tu, _tv = np.meshgrid(np.linspace(0, TAU, 20, endpoint=False), np.linspace(0, TAU, 10, endpoint=False), indexing="ij")
TOR_V = np.stack([(1 + 0.42 * np.cos(_tv)) * np.cos(_tu), (1 + 0.42 * np.cos(_tv)) * np.sin(_tu), 0.42 * np.sin(_tv)], -1).reshape(-1, 3)
TOR_E = []
for i in range(20):
    for j in range(10):
        a = i * 10 + j
        TOR_E.append((a, ((i + 1) % 20) * 10 + j))
        TOR_E.append((a, i * 10 + (j + 1) % 10))


def part_tunnel(t):
    a = TAU * t / FRAMES
    bp = beat_pulse(t)
    cx = 160 + 46 * math.sin(a * 2)
    cy = 100 + 28 * math.sin(a * 3 + 0.7)
    dx, dy = XX - cx, YY - cy
    d = np.sqrt(dx * dx + dy * dy) + 1e-3
    ang = np.arctan2(dy, dx)
    u = ang / TAU * 16 + a * 2 * 3  # twist (integer turns of the pattern per loop)
    v = 900.0 / d + t * 0.55
    chk = ((np.floor(u) + np.floor(v / 4)) % 2).astype(np.float32)
    idx = (np.floor(v / 2 + B8 * 0.999) + int(t * 0.8)).astype(int) % 32
    fog = np.clip((d - 8) / 150.0, 0, 1) ** 0.8
    fq = np.floor(fog * 7 + B8) / 7.0
    img = TUN_PAL[idx] * ((0.38 + 0.62 * chk) * fq * (0.9 + 0.3 * bp))[..., None]
    # torus wireframe, depth-cued
    R = rot(a * 2 + 0.9, a * 3 + 0.4, a)
    vv = TOR_V @ R.T
    tx = 160 + 20 * math.sin(a * 4)
    ty = 94 + 8 * math.cos(a * 2)
    p2, z = project(vv, tx, ty, 62 * (1 + 0.08 * bp))
    zn = (z - z.min()) / max(1e-6, z.max() - z.min())
    lines = Image.new("RGB", (W, H), (0, 0, 0))
    ld = ImageDraw.Draw(lines)
    order = sorted(TOR_E, key=lambda e: -(z[e[0]] + z[e[1]]))
    for i, j in order:
        dep = 1 - (zn[i] + zn[j]) / 2
        c = TOR_PAL[int(dep * 31)]
        ld.line([tuple(p2[i]), tuple(p2[j])], fill=tuple(int(x) for x in c), width=1)
    ln = np.asarray(lines, np.float32)
    # darken behind the torus a little
    m = np.asarray(lines.convert("L").filter(ImageFilter.GaussianBlur(5)), np.float32) / 255
    img *= (1 - np.clip(m * 2.5, 0, 0.7))[..., None]
    return glow_add(img, ln, strength=0.8 + 0.6 * bp, radius=2.0)


# --------------------------------------------------------------------------- scroller
SCROLL_TEXT = "CRTSIM PRESENTS ... GREETINGS TO EVERY PIXEL ... "
SCROLL_TEXT = SCROLL_TEXT.ljust(50)  # 50 chars * 24 px = 1200 px = 5 px/frame * 240 frames
SC_SCALE = 4
_cols = []
for ch in SCROLL_TEXT:
    _cols.append(FONT[ch])
    _cols.append(np.zeros((7, 1), bool))
SC_M = np.kron(np.concatenate(_cols, 1), np.ones((SC_SCALE, SC_SCALE), bool))
SC_M = np.pad(SC_M, ((2, 3), (0, 0)))
SC_HH, SC_W = SC_M.shape
assert SC_W == 1200
_yy, _xx = np.mgrid[0:SC_HH, 0:SC_W]
_ly, _lx = (_yy - 2) % SC_SCALE, _xx % SC_SCALE
SC_OUT = dilate(SC_M, 1) | np.roll(np.roll(dilate(SC_M, 1), 2, 0), 2, 1)
_rowpal = ramp([(0, (255, 250, 140)), (0.25, (255, 190, 30)), (0.5, (255, 70, 60)), (0.75, (230, 30, 200)), (1, (90, 60, 255))], 28)
SC_COL = np.zeros((SC_HH, SC_W, 3), np.float32)
SC_COL[2:30] = _rowpal[:, None, :]
SC_COL *= np.where((_ly == 0) | (_lx == 0), 1.2, np.where((_ly == SC_SCALE - 1) | (_lx == SC_SCALE - 1), 0.55, 1.0))[..., None]
SC_COL = np.clip(SC_COL, 0, 255)
SPEED = 5


def draw_scroller(img, t):
    base = 190
    bp = beat_pulse(t)
    for x in range(W):
        sx = int((x + SPEED * t - 8) % SC_W)
        y0 = base + int(round(13 * math.sin(x * 0.028 + TAU * 8 * t / FRAMES)))
        y0 -= SC_HH // 2
        col_m = SC_M[:, sx]
        col_o = SC_OUT[:, sx] & ~col_m
        ys = np.arange(SC_HH) + y0
        ok = (ys >= 0) & (ys < H)
        img[ys[ok & col_o], x] = (0, 0, 8)
        sel = ok & col_m
        img[ys[sel], x] = SC_COL[sel, sx] * (1 + 0.15 * bp)


# --------------------------------------------------------------------------- sequencing
# beats: 0..74 copper | 75..89 wipe | 90..149 plasma | 150..164 wipe | 165..224 tunnel | 225..239 wipe back
SEQ = [(0, part_copper), (75, part_plasma), (150, part_tunnel), (225, part_copper)]


def wipe_mask(p):
    """Dithered diagonal wipe; p in (0,1)."""
    w = 0.45
    g = (YY / H) * 0.7 + (XX / W) * 0.3
    a = np.clip((p * (1 + w) - g) / w, 0, 1)
    return B8 < a


def render(t):
    cur = None
    for i, (start, fn) in enumerate(SEQ[1:]):
        if start <= t < start + BEAT:
            p = (t - start + 1) / (BEAT + 1)
            a = SEQ[i][1](t)
            b = fn(t)
            m = wipe_mask(p)
            cur = np.where(m[..., None], b, a)
            break
    if cur is None:
        fn = [f for s, f in SEQ if s <= t][-1]
        cur = fn(t)
    draw_scroller(cur, t)
    return np.clip(cur, 0, 255).astype(np.uint8)


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(1)
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for t in range(FRAMES):
        Image.fromarray(render(t), "RGB").save(os.path.join(out, f"{t:05d}.png"))


if __name__ == "__main__":
    main()
