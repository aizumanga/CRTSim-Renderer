#!/usr/bin/env python3
"""racer: an original pseudo-3D (scanline road) arcade racing scene at dusk.

320x224, 240 frames (8 s at 30 fps). Usage: python3 racer.py OUT_DIR
numpy + Pillow only, deterministic.
"""
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw

W, H = 320, 224
CY = 100                 # projection centre row (horizon of a flat road)
FRAMES = 240
FPS = 30

SEG_LEN = 200
RUMBLE_LEN = 3
ROAD_W = 1500            # half width of the road in world units
CAM_H = 1000
CAM_DEPTH = 1.0 / math.tan(math.radians(100 / 2))
PLAYER_Z = CAM_H * CAM_DEPTH
DRAW_DIST = 260
FOG_DENSITY = 5.0
KMH_TO_U = 40.0          # world units per second per km/h

COURSE = "VESPERA COAST"

# ----------------------------------------------------------------------------- palette
HAZE = np.array([150, 62, 112], np.float32)
GRASS = [np.array([30, 84, 74], np.float32), np.array([22, 66, 62], np.float32)]
ROAD = [np.array([82, 72, 104], np.float32), np.array([74, 65, 96], np.float32)]
RUMBLE = [np.array([232, 36, 72], np.float32), np.array([236, 222, 200], np.float32)]
LANE = np.array([236, 222, 200], np.float32)
EDGE = np.array([250, 200, 90], np.float32)

BAYER = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.float32) / 16.0


def bayer(h, w):
    return np.tile(BAYER, (h // 4 + 1, w // 4 + 1))[:h, :w]


# ----------------------------------------------------------------------------- font
FONT = {
    'A': ".###.#...##...#######...##...##...#", 'B': "####.#...##...#####.#...##...#####.",
    'C': ".###.#...##....#....#....#...#.###.", 'D': "####.#...##...##...##...##...#####.",
    'E': "######....#....####.#....#....#####", 'F': "######....#....####.#....#....#....",
    'G': ".###.#...##....#.####...##...#.####", 'H': "#...##...##...#######...##...##...#",
    'I': ".###...#....#....#....#....#...###.", 'J': "..###...#....#....#.#..#.#..#..##..",
    'K': "#...##..#.#.#..##...#.#..#..#.#...#", 'L': "#....#....#....#....#....#....#####",
    'M': "#...###.###.#.##.#.##...##...##...#", 'N': "#...##...###..##.#.##..###...##...#",
    'O': ".###.#...##...##...##...##...#.###.", 'P': "####.#...##...#####.#....#....#....",
    'Q': ".###.#...##...##...##.#.##..#..##.#", 'R': "####.#...##...#####.#.#..#..#.#...#",
    'S': ".###.#...##.....###.....##...#.###.", 'T': "#####..#....#....#....#....#....#..",
    'U': "#...##...##...##...##...##...#.###.", 'V': "#...##...##...##...##...#.#.#...#..",
    'W': "#...##...##...##.#.##.#.##.#..#.#.", 'X': "#...##...#.#.#...#...#.#.#...##...#",
    'Y': "#...##...#.#.#...#....#....#....#..", 'Z': "#####....#...#...#...#...#....#####",
    '0': ".###.#...##..###.#.###..##...#.###.", '1': "..#...##....#....#....#....#...###.",
    '2': ".###.#...#....#...#...#...#...#####", '3': "#####...#...#.....#....##...#.###.",
    '4': "...#...##..#.#.#..#.#####...#....#.", '5': "######....####.....#....##...#.###.",
    '6': "..##..#...#....####.#...##...#.###.", '7': "#####....#...#...#...#....#....#...",
    '8': ".###.#...##...#.###.#...##...#.###.", '9': ".###.#...##...#.####....#...#..##..",
    "'": "..#....#...#.........................", '"': ".#.#..#.#.#.#........................",
    ':': "......##...##.........##...##......", '.': ".........................##...##..",
    '/': "....#...#....#...#...#....#...#....", '-': "...............###...............",
    '+': ".......#....#..#####..#....#.......", '!': "..#....#....#....#....#.........#..",
    '>': ".#.....#.....#.....#...#...#...#...", '<': "...#...#...#...#.....#.....#.....#.",
    ' ': "." * 35,
}
GLYPH = {}
for _k, _v in FONT.items():
    _v = (_v + "." * 35)[:35]
    GLYPH[_k] = np.array([[c == '#' for c in _v[r * 5:r * 5 + 5]] for r in range(7)], bool)


def text_mask(text, scale=1, spacing=1):
    w = len(text) * (5 + spacing) - spacing
    m = np.zeros((7, max(w, 1)), bool)
    for i, ch in enumerate(text):
        g = GLYPH.get(ch, GLYPH[' '])
        m[:, i * (5 + spacing):i * (5 + spacing) + 5] |= g
    if scale > 1:
        m = m.repeat(scale, 0).repeat(scale, 1)
    return m


def blit_text(img, x, y, text, colors, scale=1, outline=(10, 6, 24), spacing=1):
    """colors: one RGB, or a list of per-row RGB (top to bottom, len == 7*scale)."""
    m = text_mask(text, scale, spacing)
    mh, mw = m.shape
    pad = 1
    o = np.zeros((mh + 2 * pad + 1, mw + 2 * pad + 1), bool)
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            o[pad + dy:pad + dy + mh, pad + dx:pad + dx + mw] |= m
    o[pad + 1:pad + 1 + mh, pad + 1:pad + 1 + mw] |= m  # drop shadow
    x0, y0 = x - pad, y - pad
    _stamp(img, x0, y0, o, np.array(outline, np.float32) if outline is not None else None)
    col = np.array(colors, np.float32)
    if col.ndim == 1:
        _stamp(img, x, y, m, col)
    else:
        for r in range(mh):
            _stamp(img, x, y + r, m[r:r + 1], col[min(r, len(col) - 1)])
    return mw


def _stamp(img, x, y, mask, col):
    if col is None:
        return
    mh, mw = mask.shape
    xa, ya = max(x, 0), max(y, 0)
    xb, yb = min(x + mw, img.shape[1]), min(y + mh, img.shape[0])
    if xa >= xb or ya >= yb:
        return
    sub = mask[ya - y:yb - y, xa - x:xb - x]
    img[ya:yb, xa:xb][sub] = col


def grad_rows(stops, n):
    stops = np.array(stops, np.float32)
    t = np.linspace(0, 1, n)
    pos = t * (len(stops) - 1)
    i = np.minimum(pos.astype(int), len(stops) - 2)
    f = (pos - i)[:, None]
    return stops[i] * (1 - f) + stops[i + 1] * f


# ----------------------------------------------------------------------------- sprites
def rgba(w, h):
    return Image.new("RGBA", (w, h), (0, 0, 0, 0))


def make_palm(seed):
    rng = np.random.RandomState(seed)
    w, h = 56, 104
    im = rgba(w, h)
    d = ImageDraw.Draw(im)
    lean = rng.uniform(-8, 10)
    base = (w / 2 - 2, h - 1)
    top = (w / 2 + lean, 26)
    n = 34
    pts = []
    for i in range(n + 1):
        t = i / n
        x = base[0] + (top[0] - base[0]) * (t ** 1.6)
        y = base[1] + (top[1] - base[1]) * t
        pts.append((x, y))
    for i in range(n):
        t = i / n
        wd = 5.5 - 2.5 * t
        x, y = pts[i]
        band = (i % 3 == 0)
        c = (58, 36, 52) if band else (84, 52, 64)
        d.ellipse([x - wd / 2, y - 2, x + wd / 2, y + 1], fill=c + (255,))
        d.point((x - wd / 2 + 0.5, y), fill=(255, 150, 96, 255))  # sun rim light
    cx, cy = top
    nf = 8
    for k in range(nf):
        ang = math.pi * (k / (nf - 1)) + rng.uniform(-0.15, 0.15)
        dirx = -math.cos(ang)
        L = rng.uniform(20, 27)
        droop = rng.uniform(14, 22)
        prev = None
        for i in range(0, 25):
            t = i / 24
            x = cx + dirx * L * t
            y = cy - math.sin(ang) * 10 * t + droop * t * t
            if prev is not None:
                d.line([prev, (x, y)], fill=(26, 70, 58, 255), width=2)
                # leaflets
                if i % 2 == 0:
                    ll = 5 * (1 - t) + 2
                    d.line([(x, y), (x + dirx * 1.5, y + ll)], fill=(20, 54, 50, 255), width=1)
                    d.line([(x, y), (x - dirx * 0.5, y + ll * 0.8)], fill=(34, 92, 70, 255), width=1)
            prev = (x, y)
        d.point((cx + dirx * L * 0.5, cy - math.sin(ang) * 5 + droop * 0.25 - 1),
                fill=(255, 170, 110, 255))
    for k in range(3):
        d.ellipse([cx - 3 + k * 2, cy + 1, cx + k * 2, cy + 4], fill=(70, 40, 30, 255))
    return im, 0.5 - 2 / w


def make_lamp(right_side):
    w, h = 40, 120
    im = rgba(w, h)
    d = ImageDraw.Draw(im)
    px = 6
    d.rectangle([px - 1, 10, px + 1, h - 1], fill=(70, 66, 100, 255))
    d.line([(px - 1, 10), (px - 1, h - 1)], fill=(150, 140, 190, 255))
    d.rectangle([px - 3, h - 8, px + 3, h - 1], fill=(56, 50, 80, 255))
    # arm
    d.arc([px - 1, 6, px + 30, 30], 180, 270, fill=(110, 104, 150, 255), width=2)
    d.line([(px + 14, 6), (px + 28, 6)], fill=(110, 104, 150, 255), width=2)
    # head + glow
    glow = rgba(w, h)
    gd = ImageDraw.Draw(glow)
    for r, a in ((12, 40), (8, 70), (5, 120)):
        gd.ellipse([px + 27 - r, 10 - r, px + 27 + r, 10 + r], fill=(255, 190, 110, a))
    im = Image.alpha_composite(glow, im)
    d = ImageDraw.Draw(im)
    d.rectangle([px + 21, 5, px + 33, 8], fill=(60, 56, 84, 255))
    d.rectangle([px + 22, 9, px + 32, 11], fill=(255, 238, 170, 255))
    d.line([(px + 24, 10), (px + 30, 10)], fill=(255, 252, 220, 255))
    if not right_side:
        return im, px / w
    return im.transpose(Image.FLIP_LEFT_RIGHT), (w - 1 - px) / w


def make_billboard(word, bg1, bg2, fg):
    tm = text_mask(word, 2, 2)
    pw = max(64, tm.shape[1] + 14)
    w, h = pw, 52
    im = rgba(w, h)
    d = ImageDraw.Draw(im)
    for x in (pw // 5, pw - pw // 5):
        d.rectangle([x - 2, 26, x + 1, h - 1], fill=(64, 58, 86, 255))
        d.line([(x - 2, 26), (x - 2, h - 1)], fill=(130, 120, 170, 255))
    d.rectangle([0, 0, pw - 1, 29], fill=(30, 22, 44, 255))
    g = grad_rows([bg1, bg2], 26)
    a = np.array(im)
    a[2:28, 2:pw - 2, :3] = g[:, None, :].astype(np.uint8)
    a[2:28, 2:pw - 2, 3] = 255
    # text
    ty, tx = 8, (pw - tm.shape[1]) // 2
    sh = np.zeros_like(tm)
    a[ty + 1:ty + 1 + tm.shape[0], tx + 1:tx + 1 + tm.shape[1], :3][tm] = (30, 14, 40)
    a[ty:ty + tm.shape[0], tx:tx + tm.shape[1], :3][tm] = fg
    del sh
    # bulbs along the top
    for x in range(4, pw - 3, 6):
        a[0, x, :] = (255, 230, 150, 255)
    return Image.fromarray(a), 0.5


def make_chevron(pointing_right):
    w, h = 24, 30
    im = rgba(w, h)
    d = ImageDraw.Draw(im)
    d.rectangle([11, 16, 12, h - 1], fill=(90, 84, 120, 255))
    d.rectangle([0, 0, w - 1, 17], fill=(20, 14, 34, 255))
    d.rectangle([1, 1, w - 2, 16], fill=(236, 48, 80, 255))
    for k in (0, 8):
        pts = [(4 + k, 3), (9 + k, 3), (14 + k, 8.5), (9 + k, 14), (4 + k, 14), (9 + k, 8.5)]
        d.polygon(pts, fill=(250, 214, 120, 255))
    if not pointing_right:
        im = im.transpose(Image.FLIP_LEFT_RIGHT)
    return im, 0.5


def make_gate():
    w, h = 176, 96
    im = rgba(w, h)
    a = np.array(im)
    # pillars
    for x0 in (0, w - 12):
        a[0:h, x0:x0 + 12] = (52, 44, 80, 255)
        a[0:h, x0 + 1] = (130, 116, 180, 255)
        a[0:h, x0 + 10] = (30, 24, 50, 255)
        for y in range(30, h - 4, 8):
            a[y:y + 3, x0 + 4:x0 + 8] = (255, 196, 90, 255)
    # banner
    a[2:30, 6:w - 6] = (20, 16, 52, 255)
    for x in range(6, w - 6):
        for y in (3, 4, 26, 27):
            on = ((x // 3) + (y // 2)) % 2
            a[y, x] = (236, 222, 200, 255) if on else (16, 12, 30, 255)
    tm = text_mask(COURSE, 1, 2)
    ty, tx = 11, (w - tm.shape[1]) // 2
    a[ty + 1:ty + 8, tx + 1:tx + 1 + tm.shape[1], :3][tm] = (60, 10, 60)
    g = grad_rows([(255, 240, 150), (255, 110, 150)], 7)
    for r in range(7):
        a[ty + r, tx:tx + tm.shape[1], :3][tm[r]] = g[r].astype(np.uint8)
    return Image.fromarray(a), 0.5


MINI = {  # 3x5 glyphs for the licence plate
    'C': "####..#..#..###", 'R': "####.###.#.##.#", 'T': "###.#..#..#..#.",
    '5': "####..###..####", 'I': "###.#..#..#.###", 'M': "#.####.#.#.#.#.",
}


def make_car(body, lean=0.0, brake=False, frame=0, flame=True):
    """Rear view coupe. body: base RGB. lean in [-1, 1] (+ = turning right)."""
    Wc, Hc = 104, 52
    cx = Wc // 2
    b = np.array(body, np.float32)

    def sh(k):
        return tuple(int(v) for v in np.clip(b * k, 0, 255)) + (255,)

    hi = tuple(int(v) for v in np.clip(b * 1.25 + 40, 0, 255)) + (255,)

    under = rgba(Wc, Hc)
    du = ImageDraw.Draw(under)
    du.ellipse([cx - 48, 41, cx + 48, 51], fill=(8, 6, 18, 210))
    for sx in (-1, 1):
        x0 = cx + sx * 34 - 7
        du.rectangle([x0, 31, x0 + 13, 47], fill=(20, 18, 28, 255))
        for y in range(31, 48):
            if (y + frame) % 3 == 0:
                du.line([(x0 + 1, y), (x0 + 12, y)], fill=(48, 44, 60, 255))
        du.line([(x0 + (12 if sx < 0 else 1), 32), (x0 + (12 if sx < 0 else 1), 46)],
                fill=(70, 66, 86, 255))

    top = rgba(Wc, Hc)
    d = ImageDraw.Draw(top)
    flank = int(round(abs(lean) * 6))
    side = -1 if lean > 0 else 1   # we see the flank on the outside of the turn
    # flank (side panel when yawing)
    if flank:
        if side < 0:
            d.polygon([(cx - 46 - flank, 16), (cx - 44, 14), (cx - 42, 38), (cx - 42 - flank, 37)], fill=sh(0.45))
        else:
            d.polygon([(cx + 46 + flank, 16), (cx + 44, 14), (cx + 42, 38), (cx + 42 + flank, 37)], fill=sh(0.45))
    # lower body / bumper
    d.polygon([(cx - 45, 26), (cx + 45, 26), (cx + 42, 40), (cx - 42, 40)], fill=sh(0.62))
    d.rectangle([cx - 24, 35, cx + 24, 40], fill=(26, 22, 38, 255))
    for x in range(cx - 22, cx + 23, 4):
        d.line([(x, 36), (x, 40)], fill=(54, 48, 70, 255))
    for ex in (-17, 17):
        d.ellipse([cx + ex - 3, 34, cx + ex + 3, 39], fill=(150, 146, 160, 255))
        d.ellipse([cx + ex - 2, 35, cx + ex + 2, 38], fill=(34, 16, 10, 255))
        if flame and frame % 4 < 2:
            d.point((cx + ex, 37), fill=(255, 170, 70, 255))
    # main body
    d.polygon([(cx - 47, 15), (cx + 47, 15), (cx + 46, 26), (cx - 46, 26)], fill=sh(1.0))
    d.line([(cx - 46, 15), (cx + 46, 15)], fill=hi)
    d.line([(cx - 46, 26), (cx + 46, 26)], fill=sh(0.4))
    # taillight band
    d.rectangle([cx - 44, 19, cx + 44, 23], fill=(90, 8, 26, 255))
    core = (255, 170, 160, 255) if brake else (255, 44, 72, 255)
    mid = (255, 90, 100, 255) if brake else (200, 20, 52, 255)
    for sx in (-1, 1):
        x0, x1 = sorted((cx + sx * 43, cx + sx * 24))
        d.rectangle([x0, 20, x1, 22], fill=mid)
        d.line([(x0 + 1, 21), (x1 - 1, 21)], fill=core)
    d.line([(cx - 22, 21), (cx + 22, 21)], fill=(170, 16, 46, 255) if not brake else (230, 60, 80, 255))
    # plate
    d.rectangle([cx - 10, 27, cx + 10, 33], fill=(222, 214, 188, 255))
    a = np.array(top)
    for i, ch in enumerate("CRT"):
        g = MINI[ch]
        for r in range(5):
            for c in range(3):
                if g[r * 3 + c] == '#':
                    a[28 + r, cx - 7 + i * 5 + c] = (24, 20, 44, 255)
    top = Image.fromarray(a)
    d = ImageDraw.Draw(top)
    # cabin / rear deck
    d.polygon([(cx - 31, 3), (cx + 31, 3), (cx + 40, 15), (cx - 40, 15)], fill=sh(0.78))
    d.line([(cx - 30, 3), (cx + 30, 3)], fill=hi)
    win = grad_rows([(46, 24, 84), (110, 46, 120), (232, 110, 110)], 9)
    a = np.array(top)
    for r in range(9):
        y = 5 + r
        half = 27 + int(r * 0.8)
        a[y, cx - half:cx + half + 1, :3] = win[r].astype(np.uint8)
        a[y, cx - half:cx + half + 1, 3] = 255
        # diagonal highlight streak
        for k in (0, 1, 2):
            xs = cx - 10 + (8 - r) + k
            a[y, xs, :3] = np.minimum(255, win[r] + 70).astype(np.uint8)
        a[y, cx + 14 + (8 - r), :3] = np.minimum(255, win[r] + 45).astype(np.uint8)
    top = Image.fromarray(a)
    d = ImageDraw.Draw(top)
    # mirrors
    for sx in (-1, 1):
        d.rectangle(sorted([cx + sx * 40, cx + sx * 45]) and [min(cx + sx * 40, cx + sx * 45), 8,
                                                                max(cx + sx * 40, cx + sx * 45), 11],
                    fill=sh(0.7))
    # wing
    for sx in (-18, 18):
        d.rectangle([cx + sx - 1, 12, cx + sx + 1, 16], fill=(30, 26, 44, 255))
    d.rectangle([cx - 46, 9, cx + 46, 12], fill=sh(0.55))
    d.line([(cx - 46, 9), (cx + 46, 9)], fill=hi)
    d.rectangle([cx - 47, 8, cx - 44, 13], fill=sh(0.4))
    d.rectangle([cx + 44, 8, cx + 47, 13], fill=sh(0.4))

    # lean: shear the body rows toward the turn
    ta = np.array(top)
    out = np.zeros_like(ta)
    for y in range(Hc):
        s = int(round(lean * 4.0 * max(0.0, (34 - y)) / 30.0))
        out[y] = np.roll(ta[y], s, axis=0)
    top = Image.fromarray(out)
    # body sits one pixel lower on the outside when leaning hard
    res = Image.alpha_composite(under, top)
    return res


# ----------------------------------------------------------------------------- track
def ease_in(a, b, p):
    return a + (b - a) * p * p


def ease_io(a, b, p):
    return a + (b - a) * ((-math.cos(p * math.pi) / 2) + 0.5)


class Track:
    def __init__(self):
        self.curve = []
        self.y = [0.0]   # y at segment start (len = n+1)

    def add(self, enter, hold, leave, curve, hill):
        y0 = self.y[-1]
        y1 = y0 + hill * SEG_LEN
        tot = enter + hold + leave
        for n in range(enter):
            self.curve.append(ease_in(0, curve, n / enter))
            self.y.append(ease_io(y0, y1, (n + 1) / tot))
        for n in range(hold):
            self.curve.append(curve)
            self.y.append(ease_io(y0, y1, (enter + n + 1) / tot))
        for n in range(leave):
            self.curve.append(ease_io(curve, 0, n / leave))
            self.y.append(ease_io(y0, y1, (enter + hold + n + 1) / tot))

    def done(self):
        self.n = len(self.curve)
        self.curve = np.array(self.curve)
        self.y = np.array(self.y)


def smooth_keys(keys, f):
    for (f0, v0), (f1, v1) in zip(keys, keys[1:]):
        if f0 <= f <= f1:
            p = (f - f0) / (f1 - f0) if f1 > f0 else 1
            return ease_io(v0, v1, p)
    return keys[-1][1]


# ----------------------------------------------------------------------------- scene
def build_scene():
    tr = Track()
    tr.add(5, 40, 10, 0, 0)          # 0..55 straight
    tr.add(20, 40, 20, 3.2, 14)      # right sweeper, climbing
    tr.add(15, 20, 15, 0, 26)        # climb
    tr.add(15, 25, 20, -2.4, -46)    # crest, drop with a gentle left
    tr.add(15, 25, 15, -5.2, 0)      # left hander
    tr.add(15, 25, 15, 5.0, 12)      # right hander
    tr.add(10, 70, 10, 0, -8)        # straight with the gate
    tr.add(25, 60, 25, 2.2, 22)
    tr.add(25, 60, 25, -3.0, -26)
    tr.add(20, 200, 20, 0, 0)
    tr.done()
    return tr


SPEED_KEYS = [(0, 236), (24, 262), (30, 255), (78, 287), (132, 292), (160, 256), (172, 258),
              (230, 301), (239, 302)]


def speed_at(f):
    return smooth_keys(SPEED_KEYS, f)


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "frames"
    os.makedirs(out, exist_ok=True)
    rng = np.random.RandomState(1987)
    tr = build_scene()

    # positions per frame
    kmh = np.array([speed_at(f) for f in range(FRAMES + 1)])
    pos = np.zeros(FRAMES + 1)
    pos[0] = 6 * SEG_LEN
    for f in range(FRAMES):
        pos[f + 1] = pos[f] + kmh[f] * KMH_TO_U / FPS
    braking = np.r_[kmh[1:] - kmh[:-1], 0] < -0.25

    def seg_at(z):
        return int(z // SEG_LEN)

    # ---- sprites
    palms = [make_palm(s) for s in range(4)]
    lampL, lampR = make_lamp(False), make_lamp(True)
    boards = [make_billboard("ZEPHA", (255, 180, 70), (240, 70, 110), (30, 10, 60)),
              make_billboard("CRTSIM", (30, 200, 220), (40, 70, 200), (255, 240, 170)),
              make_billboard("ORLO 24", (150, 60, 220), (60, 20, 120), (120, 255, 200)),
              make_billboard("LUMAVEN", (250, 230, 120), (250, 120, 60), (120, 20, 70))]
    chevR, chevL = make_chevron(True), make_chevron(False)
    gate = make_gate()
    # (image, anchor, world width)
    seg_sprites = [[] for _ in range(tr.n)]

    def put(seg, spr, off, ww):
        if 0 <= seg < tr.n:
            seg_sprites[seg].append((off, spr[0], spr[1], ww))

    gate_seg = None
    for s in range(tr.n):
        c = tr.curve[s]
        if s % 5 == 0 and not (150 <= s < 260):
            side = 1 if (s // 5) % 2 else -1
            put(s, palms[(s // 5) % 4], side * (1.35 + 0.5 * rng.rand()), 2000)
            if rng.rand() < 0.6:
                put(s, palms[(s // 5 + 1) % 4], -side * (1.9 + 0.9 * rng.rand()), 2100)
        if 150 <= s < 260 and s % 6 == 0:
            put(s, lampL, -1.2, 800)
            put(s, lampR, 1.2, 800)
        if 150 <= s < 260 and s % 6 == 3 and rng.rand() < 0.5:
            put(s, palms[s % 4], (1 if rng.rand() < 0.5 else -1) * 2.2, 2000)
        if abs(c) > 2.0 and s % 3 == 0:
            if c > 0:
                put(s, chevL, -1.2, 450)   # outside of a right curve is the left
            else:
                put(s, chevR, 1.2, 450)
    for s, bi, side in ((30, 0, 1), (64, 1, -1), (118, 2, 1), (172, 3, -1), (228, 0, 1),
                        (300, 1, 1), (352, 2, -1), (420, 3, 1), (470, 1, -1), (530, 0, 1)):
        put(s, boards[bi], side * 1.75, 1900)

    # gate where the player is at ~frame 196
    gf = 196
    gate_seg = seg_at(pos[gf] + PLAYER_Z) + 1
    put(gate_seg, gate, 0.0, ROAD_W * 2 * 1.32)
    gate_frame = next(f for f in range(FRAMES) if pos[f] + PLAYER_Z >= gate_seg * SEG_LEN)

    # ---- traffic (body colour, lane x, speed kmh, overtake frame)
    traffic_spec = [((250, 196, 40), 0.62, 182, 58),
                    ((40, 170, 240), 0.0, 176, 118),
                    ((170, 90, 250), -0.6, 168, 176),
                    ((60, 230, 160), 0.6, 190, 236)]
    traffic = []
    for col, lx, sp, fo in traffic_spec:
        v = sp * KMH_TO_U / FPS
        z0 = pos[fo] + PLAYER_Z - v * fo
        traffic.append((lx, z0, v, col))
    car_imgs = {}

    def traffic_img(col, f):
        k = (col, f % 3)
        if k not in car_imgs:
            car_imgs[k] = make_car(col, 0.0, False, f % 3, flame=False)
        return car_imgs[k]

    # player lateral script
    px_keys = [(0, 0.05), (40, -0.12), (70, -0.18), (96, -0.05), (104, -0.55), (130, -0.55),
               (142, -0.1), (160, 0.18), (190, 0.12), (215, -0.05), (239, -0.1)]
    player_x = np.array([smooth_keys(px_keys, f) for f in range(FRAMES + 2)])

    # ---- backdrop
    MW = 640
    xs = np.arange(MW)
    mh = np.zeros(MW)
    for k, a_, ph in ((1, 7, 0.3), (2, 6, 1.1), (3, 4, 2.0), (5, 3, 0.7), (8, 2.2, 2.6), (13, 1.2, 1.4),
                      (21, 0.7, 0.2)):
        mh += a_ * np.sin(2 * np.pi * k * xs / MW + ph)
    mh = 20 + mh
    # valley where the sun sets at the start
    mh = mh * (1 - 0.75 * np.exp(-((((xs - 160) + MW / 2) % MW - MW / 2) / 40.0) ** 2))
    mh = np.maximum(mh, 2)

    CW, CH = 512, 48
    city = np.zeros((CH, CW, 4), np.uint8)
    windows = []
    x = 0
    lights = []
    while x < CW:
        bw = rng.randint(6, 18)
        bh = rng.randint(6, 30) if rng.rand() < 0.8 else rng.randint(28, 44)
        if rng.rand() < 0.15:
            bh = rng.randint(2, 5)
        top = CH - bh
        c = np.array([30, 20, 56]) + rng.randint(0, 10)
        city[top:, x:min(x + bw, CW), :3] = c
        city[top:, x:min(x + bw, CW), 3] = 255
        city[top, x:min(x + bw, CW), :3] = c + 30
        for wy in range(top + 2, CH - 2, 3):
            for wx in range(x + 2, min(x + bw, CW) - 1, 2):
                if rng.rand() < 0.3:
                    col = [(255, 206, 120), (120, 220, 255), (255, 120, 190), (255, 230, 170)][rng.randint(4)]
                    windows.append((wy, wx, col, rng.randint(0, 997)))
        if bh > 26 and rng.rand() < 0.8:
            ax = x + bw // 2
            ah = rng.randint(3, 7)
            city[top - ah:top, ax, :3] = (60, 50, 90)
            city[top - ah:top, ax, 3] = 255
            lights.append((top - ah - 1, ax, rng.randint(0, 30)))
        x += bw
    # lower part of the city fades into the haze
    fade = np.linspace(0, 1, CH)[:, None] ** 3

    stars = [(rng.randint(0, W * 3), rng.randint(0, 46), rng.rand()) for _ in range(70)]

    sky_stops = [(10, 6, 38), (26, 10, 66), (62, 18, 104), (120, 28, 128), (186, 48, 124),
                 (234, 88, 104), (255, 136, 84), (255, 186, 100)]

    car_cache = {}
    bgx = 0.0
    yy, xx = np.mgrid[0:H, 0:W]
    BY = bayer(H, W)

    for f in range(FRAMES):
        position = pos[f]
        speed = kmh[f]
        base_seg = seg_at(position)
        base_pct = (position % SEG_LEN) / SEG_LEN
        pz = position + PLAYER_Z
        ps = seg_at(pz)
        ppct = (pz % SEG_LEN) / SEG_LEN
        player_y = tr.y[ps] + (tr.y[ps + 1] - tr.y[ps]) * ppct
        pcurve = tr.curve[ps]
        bgx += pcurve * speed / 300.0 * 2.2
        bg_dy = int(np.clip(-(player_y - 0) * 0.0025, -10, 14))
        pX = player_x[f]

        img = np.zeros((H, W, 3), np.float32)

        # --- sky (dithered bands), sun, stars
        hz = CY + bg_dy + 6
        nb = 22
        t = np.clip(np.arange(H) / max(hz, 1), 0, 1)[:, None]
        t = t * (1 - f / FRAMES * 0.05) + f / FRAMES * 0.0
        q = np.floor(t * nb + BY * 0.999) / nb
        q = np.clip(q, 0, 1)
        idx = q * (len(sky_stops) - 1)
        i0 = np.minimum(idx.astype(int), len(sky_stops) - 2)
        fr = (idx - i0)[..., None]
        ss = np.array(sky_stops, np.float32)
        img[:] = ss[i0] * (1 - fr) + ss[i0 + 1] * fr
        for sx, sy, ph in stars:
            x = int((sx - bgx * 0.1) % (W * 3))
            if x < W:
                tw = 0.5 + 0.5 * math.sin(f * 0.35 + ph * 20)
                if tw > 0.25:
                    img[sy, x] = np.array([255, 230, 255]) * (0.4 + 0.6 * tw) * (1 - sy / 60)

        sun_x = 160 - bgx * 0.18
        sun_y = CY + bg_dy - 16 + f * 0.04
        R = 32
        dy_ = yy - sun_y
        dx_ = xx - sun_x
        dsun = dx_ * dx_ + dy_ * dy_
        disc = dsun <= R * R
        st = np.clip((dy_ + R) / (2 * R), 0, 1)
        scol = grad_rows([(255, 246, 150), (255, 200, 100), (255, 110, 96), (250, 50, 140)], 64)
        sc = scol[np.clip((st * 63).astype(int), 0, 63)]
        # moving gaps in the lower half
        gp = (dy_ - f * 0.35) % 8
        gap_h = np.clip((dy_ / R) * 5.0, 0, 5)
        disc &= ~((dy_ > 0) & (gp < gap_h))
        img[disc] = sc[disc]
        halo = np.clip(1 - np.sqrt(dsun) / (R * 2.4), 0, 1) ** 2 * (~disc)
        img += halo[..., None] * np.array([90, 40, 30]) * (yy < hz)[..., None]

        # --- far mountains
        mx = (np.arange(W) + bgx * 0.3).astype(int) % MW
        mtop = (CY + bg_dy + 4 - mh[mx])[None, :]
        mm = (yy >= mtop) & (yy < CY + bg_dy + 8)
        depth_ = np.clip((yy - mtop) / 26.0, 0, 1)
        mcol = np.array([120, 52, 132], np.float32) * (1 - depth_[..., None]) + \
            np.array([74, 30, 96], np.float32) * depth_[..., None]
        rim = mm & (yy < mtop + 1.5)
        slope = np.gradient(mh[mx])[None, :] * np.ones((H, 1))
        img[mm] = mcol[mm]
        rimc = rim & (slope > 0)
        img[rimc] = np.array([255, 130, 120])
        img[rim & ~(slope > 0)] = np.array([170, 70, 140])

        # --- city skyline
        cx_ = (np.arange(W) + bgx * 0.6).astype(int) % CW
        cl = city[:, cx_].astype(np.float32)
        # windows
        wl = np.zeros((CH, CW, 3), np.float32)
        wm = np.zeros((CH, CW), bool)
        for wy, wx, col, ph in windows:
            if ((f // 6) * 7 + ph) % 23 != 0:
                wl[wy, wx] = col
                wm[wy, wx] = True
        for ly, lx, ph in lights:
            if (f + ph) % 30 < 12:
                wl[ly, lx] = (255, 40, 60)
                wm[ly, lx] = True
        top_y = CY + bg_dy + 8 - CH
        for r in range(CH):
            y = top_y + r
            if 0 <= y < H:
                a = cl[r, :, 3] > 0
                row = cl[r, :, :3] * (1 - fade[r] * 0.0)
                row = row * (1 - fade[r]) + HAZE * fade[r] * 0.8
                img[y, a] = row[a]
                wrow = wm[r, cx_]
                img[y, wrow] = wl[r, cx_][wrow] * (1 - fade[r] * 0.6)
        # ground below the skyline
        gy0 = CY + bg_dy + 8
        if gy0 < H:
            gfar = HAZE * 0.8 + GRASS[0] * 0.2
            img[gy0:] = gfar

        # --- road
        row_n = np.full(H, -1, int)
        row_cx = np.zeros(H)
        row_hw = np.zeros(H)
        row_seg = np.zeros(H, int)
        maxy = float(H)
        x = 0.0
        dx = -(tr.curve[base_seg] * base_pct)
        cam_x = pX * ROAD_W
        cam_y = player_y + CAM_H
        segdata = []
        for n in range(DRAW_DIST):
            s = base_seg + n
            if s >= tr.n:
                break
            z1 = s * SEG_LEN - position
            z2 = z1 + SEG_LEN
            c1x = 0 - (cam_x - x)
            c2x = 0 - (cam_x - x - dx)
            x += dx
            dx += tr.curve[s]
            clip = maxy
            if z1 <= 1:
                sc1 = None
            else:
                sc1 = CAM_DEPTH / z1
            sc2 = CAM_DEPTH / z2
            y2 = CY - sc2 * (tr.y[s + 1] - cam_y) * H / 2
            x2 = W / 2 + sc2 * c2x * W / 2
            w2 = sc2 * ROAD_W * W / 2
            if sc1 is not None:
                y1 = CY - sc1 * (tr.y[s] - cam_y) * H / 2
                x1 = W / 2 + sc1 * c1x * W / 2
                w1 = sc1 * ROAD_W * W / 2
            else:
                y1, x1, w1 = None, None, None
            segdata.append((s, n, x1, y1, w1, sc1, clip, x2, y2, w2, sc2))
            if sc1 is None or z1 <= CAM_DEPTH:
                continue
            if y2 >= y1 or y2 >= maxy:
                continue
            ra = max(0, int(math.ceil(y2)))
            rb = int(min(maxy, math.ceil(y1), H))
            if rb > ra:
                rr = np.arange(ra, rb)
                tt = (rr - y1) / (y2 - y1)
                row_n[ra:rb] = n
                row_seg[ra:rb] = s
                row_cx[ra:rb] = x1 + (x2 - x1) * tt
                row_hw[ra:rb] = w1 + (w2 - w1) * tt
            maxy = y2
        rows = np.nonzero(row_n >= 0)[0]
        if len(rows):
            rn = row_n[rows]
            par = (row_seg[rows] // RUMBLE_LEN) % 2
            cxr = row_cx[rows][:, None]
            hwr = np.maximum(row_hw[rows][:, None], 0.3)
            X = np.arange(W)[None, :] + 0.5
            ad = np.abs(X - cxr)
            rumw = hwr / 6.0
            lanew = np.maximum(hwr / 32.0, 0.5)
            p = par[:, None]
            col = np.where((p == 0)[..., None], GRASS[0], GRASS[1]) * np.ones((1, W, 1))
            # grass dither stripes for texture
            road_m = ad < hwr
            rum_m = (ad >= hwr) & (ad < hwr + rumw)
            col[road_m] = np.where((p == 0)[..., None], ROAD[0], ROAD[1]).repeat(W, 1)[road_m]
            rcol = np.where((p == 0)[..., None], RUMBLE[0], RUMBLE[1]).repeat(W, 1)
            col[rum_m] = rcol[rum_m]
            edge_m = (ad < hwr - lanew * 1.0) & (ad >= hwr - lanew * 2.2)
            col[edge_m] = EDGE
            lane_off = hwr / 3.0
            lane_m = (np.abs(ad - lane_off) < lanew / 2 + 0.25) & (p == 0)
            col[lane_m] = LANE
            # fog by distance
            d = rn / DRAW_DIST
            fogk = (1.0 / np.exp(d * d * FOG_DENSITY))[:, None, None]
            col = col * fogk + HAZE * (1 - fogk)
            img[rows] = col

        # --- sprites, far to near
        ptx = None
        draw_list = []
        for (s, n, x1, y1, w1, sc1, clip, x2, y2, w2, sc2) in reversed(segdata):
            if sc1 is None or n == 0 and False:
                continue
            d = n / DRAW_DIST
            fogk = 1.0 / math.exp(d * d * FOG_DENSITY)
            for off, im, anc, ww in seg_sprites[s]:
                draw_list.append((x1 + sc1 * off * ROAD_W * W / 2, y1, sc1, im, anc, ww, clip, fogk, s))
            for lx, z0, v, col_ in traffic:
                cz = z0 + v * f
                if s * SEG_LEN <= cz < (s + 1) * SEG_LEN:
                    pc = (cz - s * SEG_LEN) / SEG_LEN
                    zz = cz - position
                    if zz <= CAM_DEPTH * 2:
                        continue
                    scc = CAM_DEPTH / zz
                    xc = x1 + (x2 - x1) * pc + scc * lx * ROAD_W * W / 2
                    ycar = y1 + (y2 - y1) * pc
                    draw_list.append((xc, ycar, scc, traffic_img(col_, f), 0.5, 650, clip, fogk, s))
        frame_img = Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).convert("RGBA")
        for sx, sy, sc, im, anc, ww, clip, fogk, s in draw_list:
            dw = ww * sc * W / 2
            dh = dw * im.height / im.width
            if dh < 1.5 or dw < 1:
                continue
            if dh > 600:
                continue
            iw, ih = max(1, int(round(dw))), max(1, int(round(dh)))
            x0 = int(round(sx - anc * dw))
            y0 = int(round(sy - dh))
            if x0 > W or x0 + iw < 0 or y0 > H:
                continue
            cut = int(math.ceil(clip)) - y0
            if cut <= 0:
                continue
            spr = im.resize((iw, ih), Image.NEAREST)
            if fogk < 0.98:
                a = np.array(spr).astype(np.float32)
                a[..., :3] = a[..., :3] * fogk + HAZE * (1 - fogk)
                spr = Image.fromarray(a.astype(np.uint8))
            if cut < ih:
                spr = spr.crop((0, 0, iw, cut))
            frame_img.alpha_composite(spr, (0, 0), (0, 0)) if False else None
            _paste(frame_img, spr, x0, y0)

        # --- player car
        steer_v = (player_x[f + 1] - player_x[f]) * 30
        lean = float(np.clip(pcurve / 4.5 * speed / 290 - steer_v * 0.9, -1, 1))
        lq = round(lean * 4) / 4
        brk = bool(braking[f])
        key = (lq, brk, f % 3, f % 4 < 2)
        if key not in car_cache:
            car_cache[key] = make_car((228, 26, 64), lq, brk, f % 3, flame=True)
        car = car_cache[key]
        bounce = 1 if (f % 4 == 0 and speed > 250) else 0
        _paste(frame_img, car, W // 2 - car.width // 2, H - car.height - 3 + bounce)

        img = np.array(frame_img.convert("RGB")).astype(np.float32)
        # brake / tail light glow on the road
        hud(img, f, speed, gate_frame, ps)
        Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).save(os.path.join(out, "%05d.png" % f))


def _paste(dst, spr, x, y):
    # clip manually for alpha_composite
    x0, y0 = max(x, 0), max(y, 0)
    x1, y1 = min(x + spr.width, dst.width), min(y + spr.height, dst.height)
    if x1 <= x0 or y1 <= y0:
        return
    sub = spr.crop((x0 - x, y0 - y, x1 - x, y1 - y))
    dst.alpha_composite(sub, (x0, y0))


GOLD = grad_rows([(255, 250, 170), (255, 214, 80), (255, 140, 40), (240, 70, 60)], 14)
GOLD3 = grad_rows([(255, 250, 170), (255, 214, 80), (255, 140, 40), (240, 70, 60)], 21)
CYAN = (110, 230, 255)
PINK = (255, 110, 190)


def fmt_time(cs):
    m = cs // 6000
    s = (cs // 100) % 60
    c = cs % 100
    return "%d'%02d\"%02d" % (m, s, c)


def hud(img, f, speed, gate_frame, ps):
    # lap timer
    start_cs = 4812
    if f < gate_frame:
        cs = start_cs + int(f * 100 / FPS)
        lap = 1
    else:
        cs = int((f - gate_frame) * 100 / FPS)
        lap = 2
    blit_text(img, 8, 6, "TIME", CYAN)
    blit_text(img, 8, 16, fmt_time(cs), GOLD, scale=2)
    blit_text(img, 8, 34, "LAP %d/3" % lap, PINK)

    # speed
    sp = int(round(speed + (1 if (f // 3) % 2 and speed > 280 else 0)))
    s = "%3d" % sp
    wsp = len(s) * 18 - 3
    xr = W - 8
    blit_text(img, xr - 29 - wsp, 6, "SPEED", CYAN)
    blit_text(img, xr - 29 - wsp + 1, 16, s, GOLD3, scale=3)
    blit_text(img, xr - 23, 30, "KM/H", PINK)
    # tach bar
    nseg = 16
    gears = [(0, 120), (120, 175), (175, 220), (220, 258), (258, 295), (295, 340)]
    g = next(i for i, (lo, hi) in enumerate(gears) if speed < hi)
    lo, hi = gears[g]
    rpm = 0.35 + 0.65 * (speed - lo) / (hi - lo)
    lit = int(rpm * nseg + 0.5)
    bx = xr - 29 - wsp
    for i in range(nseg):
        x = bx + i * 4
        hgt = 2 + i // 3
        y1 = 46
        if i < lit:
            c = (60, 255, 140) if i < 9 else ((255, 220, 60) if i < 13 else (255, 50, 70))
        else:
            c = (40, 30, 70)
        img[y1 - hgt:y1, x:x + 3] = (10, 6, 24)
        img[y1 - hgt:y1 - 0, x:x + 2] = c
    blit_text(img, bx + nseg * 4 + 2, 40, "%d" % (g + 1), (255, 255, 255) if False else (236, 222, 200))

    # course name plate
    name = COURSE
    tw = len(name) * 6 - 1
    x0 = (W - tw) // 2
    img[4:15, x0 - 6:x0 + tw + 6] = img[4:15, x0 - 6:x0 + tw + 6] * 0.35 + np.array([20, 6, 40]) * 0.65
    img[4, x0 - 6:x0 + tw + 6] = PINK
    img[14, x0 - 6:x0 + tw + 6] = PINK
    blit_text(img, x0, 6, name, (255, 236, 190), outline=None)
    blit_text(img, (W - (len("BEST 1'02\"88") * 6 - 1)) // 2, 18, "BEST 1'02\"88", CYAN)

    # lap event
    if gate_frame <= f < gate_frame + 44:
        k = f - gate_frame
        if (k // 4) % 2 == 0 or k > 28:
            t = "LAP 2"
            tw = len(t) * 24 - 4
            blit_text(img, (W - tw) // 2, 60, t, grad_rows([(180, 255, 255), (60, 200, 255), (120, 80, 255)], 28),
                      scale=4)
        lt = fmt_time(4812 + int(gate_frame * 100 / FPS))
        tw2 = len(lt) * 12 - 2
        blit_text(img, (W - tw2) // 2, 94, lt, GOLD, scale=2)
        blit_text(img, (W - (len("NEW RECORD!") * 6 - 1)) // 2, 112, "NEW RECORD!",
                  PINK if (k // 3) % 2 else (255, 236, 190))


if __name__ == "__main__":
    main()
