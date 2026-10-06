#!/usr/bin/env python3
"""Original 16-bit-style top-down RPG vignette at 256x224.

A hero walks along a stone path, over a little bridge, up to a hooded lamp-keeper
creature in front of a cottage at dusk. A dialog window slides up and a line of text
types out letter by letter in a bitmap font defined below.

Usage: python3 rpg.py OUT_DIR   -> writes 240 PNG frames 00000.png .. 00239.png
numpy + Pillow only, deterministic.
"""
import math
import os
import sys

import numpy as np
from PIL import Image

W, H = 256, 224
NFRAMES = 240
TILE = 16

BAYER4 = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]], np.float32) / 16.0 + 1 / 32.0
BAYER = np.tile(BAYER4, (H // 4, W // 4))
YY, XX = np.mgrid[0:H, 0:W]


def C(*rgb):
    return np.array(rgb, np.uint8)


# ----------------------------------------------------------------------------- palette
OUT = C(20, 16, 36)
GRASS = [C(30, 82, 58), C(44, 112, 66), C(66, 146, 72), C(108, 180, 84), C(160, 212, 100)]
LEAF = [C(14, 44, 40), C(28, 88, 58), C(48, 128, 66), C(86, 168, 78), C(146, 204, 96)]
STONE = [C(84, 70, 70), C(126, 108, 100), C(166, 148, 128), C(204, 188, 158)]
DIRT = C(128, 96, 64)
WATER = [C(20, 44, 120), C(30, 72, 170), C(44, 110, 214), C(92, 164, 246), C(196, 232, 255)]
WOOD = [C(58, 32, 26), C(104, 60, 36), C(148, 92, 52), C(192, 132, 72)]
ROOF = [C(70, 24, 36), C(132, 40, 44), C(184, 64, 52), C(226, 112, 72)]
PLASTER = [C(150, 126, 120), C(200, 176, 150), C(232, 212, 172)]
GLOW = C(255, 214, 96)


# ----------------------------------------------------------------------------- helpers
def value_noise(seed, cells_y, cells_x):
    r = np.random.default_rng(seed).random((cells_y, cells_x)).astype(np.float32)
    return np.asarray(Image.fromarray(r, "F").resize((W, H), Image.BICUBIC), np.float32)


def shift(m, dy, dx):
    p = max(abs(dy), abs(dx))
    pm = np.pad(m, p, mode="edge")
    return pm[p - dy:p - dy + m.shape[0], p - dx:p - dx + m.shape[1]]


def disk(r):
    return [(dy, dx) for dy in range(-r, r + 1) for dx in range(-r, r + 1) if dx * dx + dy * dy <= r * r + r * 0.6]


def dilate(m, r):
    out = np.zeros_like(m)
    for dy, dx in disk(r):
        out |= shift(m, dy, dx)
    return out


def erode(m, r):
    return ~dilate(~m, r)


def smooth_mask(m, r):
    m = erode(dilate(m, r), r)   # close concave corners
    return dilate(erode(m, r), r)  # open convex corners


def quantize(v, n):
    """v in 0..1 -> int levels 0..n-1 with ordered dithering."""
    return np.clip(np.floor(v * (n - 1) + BAYER), 0, n - 1).astype(np.int32)


def parse_sprite(rows, pal):
    h, w = len(rows), max(len(r) for r in rows)
    rgb = np.zeros((h, w, 3), np.uint8)
    a = np.zeros((h, w), bool)
    for y, row in enumerate(rows):
        for x, ch in enumerate(row):
            if ch in pal:
                rgb[y, x] = pal[ch]
                a[y, x] = True
    return rgb, a


def blit(img, spr, x, y, flip=False):
    rgb, a = spr
    if flip:
        rgb, a = rgb[:, ::-1], a[:, ::-1]
    h, w = a.shape
    x, y = int(round(x)), int(round(y))
    x0, y0, x1, y1 = max(x, 0), max(y, 0), min(x + w, W), min(y + h, H)
    if x0 >= x1 or y0 >= y1:
        return
    sub_a = a[y0 - y:y1 - y, x0 - x:x1 - x]
    img[y0:y1, x0:x1][sub_a] = rgb[y0 - y:y1 - y, x0 - x:x1 - x][sub_a]


def darken_ellipse(img, cx, cy, rx, ry, amt=0.55, dither=True):
    m = ((XX - cx) / rx) ** 2 + ((YY - cy) / ry) ** 2 <= 1.0
    if dither:
        m &= ((XX + YY) % 2 == 0) | (((XX - cx) / rx) ** 2 + ((YY - cy) / ry) ** 2 <= 0.45)
    img[m] = (img[m] * amt).astype(np.uint8)


def add_glow(acc, cx, cy, radius, color, strength):
    """additive soft glow into a float accumulator"""
    r = int(radius * 2.2) + 1
    x0, x1 = max(int(cx) - r, 0), min(int(cx) + r + 1, W)
    y0, y1 = max(int(cy) - r, 0), min(int(cy) + r + 1, H)
    if x0 >= x1 or y0 >= y1:
        return
    yy, xx = np.mgrid[y0:y1, x0:x1]
    d2 = ((xx - cx) ** 2 + (yy - cy) ** 2) / (radius * radius)
    f = np.exp(-d2) * strength
    acc[y0:y1, x0:x1] += f[..., None] * (np.asarray(color, np.float32) / 255)[None, None, :]


# ----------------------------------------------------------------------------- map
MAP = [
    "TTTTTTTTTTTTTTTT",
    "WWWWWggggg....gT",
    "WWWWWWgggg....gg",
    "WWWWWWgggg....gT",
    "gWWWWWgggg....gg",
    "ggggWWgggggPgggg",
    "ggggWWgggggPgggT",
    "PPPPBBPPPPPPPggg",
    "ggggWWgggggggggg",
    "gggWWWgggggggggT",
    "ggWWWggggggggggg",
    "gWWWgggggggggggg",
    "WWWggggggggggggT",
    "WWgggggggggggggg",
]


def tile_mask(chars):
    m = np.zeros((14, 16), bool)
    for r, row in enumerate(MAP):
        for c, ch in enumerate(row):
            if ch in chars:
                m[r, c] = True
    return np.kron(m, np.ones((TILE, TILE), bool))


def build_static():
    rng = np.random.default_rng(1234)
    img = np.zeros((H, W, 3), np.uint8)

    water = smooth_mask(tile_mask("WB"), 5)
    path = smooth_mask(tile_mask("PB"), 4) & ~water
    # path should stop at the cottage door and not run into the right edge
    path &= ~((XX > 210))
    path |= (np.abs(XX - 184) <= 7) & (YY >= 76) & (YY < 116)

    # --- grass: dithered value-noise shading
    n = 0.55 * value_noise(1, 8, 9) + 0.30 * value_noise(2, 18, 20) + 0.15 * value_noise(3, 60, 64)
    n = (n - n.min()) / (n.max() - n.min())
    lvl = quantize(0.18 + 0.62 * n, 4)  # levels into GRASS[0..3]
    lvl = np.clip(lvl + 0, 0, 3)
    pal = np.stack(GRASS[:4])
    img[:] = pal[lvl]
    # grass tufts
    for _ in range(170):
        x, y = rng.integers(2, W - 3), rng.integers(4, H - 2)
        if water[y - 3:y + 3, x - 3:x + 4].any() or path[y - 3:y + 3, x - 3:x + 4].any():
            continue
        img[y, x - 1] = GRASS[0]
        img[y, x + 1] = GRASS[0]
        img[y - 1, x - 1] = GRASS[1]
        img[y - 1, x] = GRASS[0]
        img[y - 1, x + 1] = GRASS[1]
        img[y - 2, x] = GRASS[4]
        img[y - 2, x - 2] = GRASS[3]

    # --- path: cobblestones via jittered voronoi
    sp = 7
    gy, gx = H // sp + 2, W // sp + 2
    jit = rng.uniform(-2.2, 2.2, (gy, gx, 2))
    shade = rng.integers(0, 3, (gy, gx))
    cy = (YY // sp).astype(int)
    cx = (XX // sp).astype(int)
    best = np.full((H, W), 1e9, np.float32)
    second = np.full((H, W), 1e9, np.float32)
    bvec = np.zeros((H, W, 2), np.float32)
    bsh = np.zeros((H, W), int)
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            ny, nx = np.clip(cy + dy, 0, gy - 1), np.clip(cx + dx, 0, gx - 1)
            sy = ny * sp + sp / 2 + jit[ny, nx, 0] + (nx % 2) * 2
            sx = nx * sp + sp / 2 + jit[ny, nx, 1]
            vy, vx = YY - sy, (XX - sx) * 0.8
            d = np.sqrt(vy * vy + vx * vx)
            closer = d < best
            second = np.where(closer, best, np.minimum(second, d))
            best = np.where(closer, d, best)
            bvec[closer] = np.stack([vy, vx], -1)[closer]
            bsh = np.where(closer, shade[ny, nx], bsh)
    edge = second - best
    lightdot = (bvec[..., 0] + bvec[..., 1])
    stone = np.where(bsh == 0, 2, np.where(bsh == 1, 2, 1))
    stone = np.where((edge < 2.4) & (lightdot > 0.5), stone - 1, stone)
    stone = np.where((edge < 2.4) & (lightdot < -1.0), np.minimum(stone + 1, 3), stone)
    stone = np.where(edge < 1.0, 0, stone)
    stone = np.clip(stone, 0, 3)
    stone_rgb = np.stack(STONE)[stone]
    # dirt between stones near edges
    img[path] = stone_rgb[path]
    rim = path & ~erode(path, 1)
    img[rim & ((XX + YY) % 3 != 0)] = DIRT
    outer = dilate(path, 1) & ~path & ~water
    img[outer] = (img[outer] * 0.62).astype(np.uint8)

    # --- shoreline on the grass side
    bank = dilate(water, 1) & ~water
    img[bank] = OUT
    bank2 = dilate(water, 2) & ~dilate(water, 1) & ~path
    img[bank2 & ((XX + YY) % 2 == 0)] = GRASS[0]

    return img, water, path


# ----------------------------------------------------------------------------- objects
def draw_tree(img, occ, bx, by, rng, size=1.0):
    """bx,by = trunk base. Canopy = union of shaded blobs."""
    darken_ellipse(img, bx + 2, by - 1, 12 * size, 4 * size, 0.6)
    # trunk
    for yy in range(int(by - 10 * size), by):
        for xx in range(bx - 3, bx + 4):
            if 0 <= yy < H and 0 <= xx < W:
                c = WOOD[1] if xx < bx - 1 else WOOD[2] if xx < bx + 2 else WOOD[0]
                if xx in (bx - 3, bx + 3):
                    c = OUT
                img[yy, xx] = c
    if 0 <= by < H:
        img[by - 1, bx - 4:bx + 5] = OUT
    cyc = by - 21 * size
    blobs = [(0, 0, 11), (-7, 3, 7.5), (7, 3, 7.5), (-4, -6, 7), (5, -6, 7), (0, -9, 6)]
    m = np.zeros((H, W), bool)
    light = np.full((H, W), -9.0, np.float32)
    for ox, oy, r in blobs:
        r = r * size
        ccx, ccy = bx + ox * size + rng.uniform(-1, 1), cyc + oy * size + rng.uniform(-1, 1)
        d = np.sqrt((XX - ccx) ** 2 + (YY - ccy) ** 2) / r
        inside = d <= 1.0
        # lambert-ish with light from upper-left
        nx, ny = (XX - ccx) / r, (YY - ccy) / r
        nz = np.sqrt(np.clip(1 - nx * nx - ny * ny, 0, 1))
        l = (-0.55 * nx - 0.65 * ny + 0.55 * nz)
        light = np.where(inside, np.maximum(light, l), light)
        m |= inside
    tex = value_noise(abs(int(bx * 7 + by)), 40, 44) * 0.45
    v = np.clip((light + 0.35) / 1.3 + tex - 0.22, 0, 1)
    lv = quantize(v, 4) + 1
    pal = np.stack(LEAF)
    img[m] = pal[lv][m]
    edge = m & ~erode(m, 1)
    img[edge] = LEAF[0]
    occ |= m


def draw_house(img):
    x0, x1 = 160, 224
    # shadow
    darken_ellipse(img, 194, 86, 40, 6, 0.55)
    # walls
    wy0, wy1 = 54, 86
    wall = np.zeros((H, W), bool)
    wall[wy0:wy1, x0 + 4:x1 - 4] = True
    v = 0.55 + 0.25 * (XX - x0) / 64.0 - 0.1 * (YY - wy0) / 32.0
    img[wall] = np.stack(PLASTER)[quantize(np.clip(v, 0, 1), 3)][wall]
    # timber frame
    for bx in (x0 + 4, x0 + 5, x1 - 6, x1 - 5, 194, 195):
        img[wy0:wy1, bx] = WOOD[1] if bx % 2 == 0 else WOOD[0]
    img[wy0:wy0 + 2, x0 + 4:x1 - 4] = WOOD[0]
    img[wy0 + 15, x0 + 4:x1 - 4] = WOOD[1]
    # stone foundation
    for yy in range(wy1 - 5, wy1):
        for xx in range(x0 + 4, x1 - 4):
            b = ((xx + (yy // 2 % 2) * 3) % 6 == 0) or yy == wy1 - 1 or yy == wy1 - 5
            img[yy, xx] = STONE[0] if b else STONE[2 if yy < wy1 - 3 else 1]
    # door
    dx0, dx1, dy0 = 178, 191, 63
    for yy in range(dy0, wy1 - 1):
        for xx in range(dx0, dx1):
            if yy < dy0 + 3 and (xx - (dx0 + dx1 - 1) / 2) ** 2 + (yy - dy0 - 4) ** 2 > 42:
                continue
            c = WOOD[2] if (xx - dx0) % 4 else WOOD[1]
            if xx in (dx0, dx1 - 1):
                c = WOOD[0]
            img[yy, xx] = c
    img[74, 188] = GLOW
    img[75, 188] = WOOD[0]
    # step
    img[wy1 - 1:wy1 + 1, dx0 - 2:dx1 + 2] = STONE[2]
    img[wy1 + 1, dx0 - 2:dx1 + 2] = STONE[0]
    # window frames (glass filled per-frame with glow)
    for (wx0, wy) in ((200, 62), (166, 62)):
        img[wy - 1:wy + 12, wx0 - 1:wx0 + 13] = WOOD[0]
    # roof
    ry0, ry1 = 18, 58
    roof = np.zeros((H, W), bool)
    for yy in range(ry0, ry1):
        inset = max(0, 6 - (yy - ry0)) if yy < ry0 + 6 else 0
        roof[yy, x0 - 2 + inset:x1 + 2 - inset] = True
    for yy in range(ry0, ry1):
        row = (yy - ry0) // 5
        ph = (yy - ry0) % 5
        for xx in range(W):
            if not roof[yy, xx]:
                continue
            tx = (xx + row * 4) % 8
            c = ROOF[2]
            if ph == 4:
                c = ROOF[0]
            elif ph == 3:
                c = ROOF[1]
            elif ph == 0 and tx < 6:
                c = ROOF[3]
            if tx == 7 and ph < 4:
                c = ROOF[1]
            img[yy, xx] = c
    # ridge and eaves
    img[ry0:ry0 + 3, x0 + 4:x1 - 4] = ROOF[0]
    img[ry0 + 1, x0 + 5:x1 - 5] = ROOF[3]
    edge = roof & ~erode(roof, 1)
    img[edge] = OUT
    img[ry1:ry1 + 2, x0 + 2:x1 - 2] = (img[ry1:ry1 + 2, x0 + 2:x1 - 2] * 0.5).astype(np.uint8)
    # chimney
    for yy in range(8, 28):
        for xx in range(206, 216):
            c = STONE[2] if ((xx + (yy // 3 % 2) * 2) % 5) else STONE[1]
            if yy % 3 == 0:
                c = STONE[1]
            if xx in (206, 215) or yy == 8:
                c = OUT
            img[yy, xx] = c
    img[9:11, 205:217] = STONE[0]
    img[8, 205:217] = OUT
    # mailbox / sign by the path start
    return


def draw_sign(img, x, y):
    # wooden signpost
    img[y:y + 14, x + 6:x + 8] = WOOD[1]
    img[y:y + 14, x + 8] = WOOD[0]
    img[y - 2:y + 6, x:x + 15] = WOOD[2]
    img[y - 2, x:x + 15] = WOOD[3]
    img[y + 5, x:x + 15] = WOOD[0]
    img[y - 3, x:x + 15] = OUT
    img[y - 2:y + 7, x - 1] = OUT
    img[y - 2:y + 7, x + 15] = OUT
    img[y + 6, x:x + 15] = OUT
    for i in range(3):  # carved squiggle (an arrow pointing right)
        img[y + 1, x + 3 + i * 3:x + 5 + i * 3] = WOOD[0]
    img[y + 2, x + 3:x + 12] = WOOD[0]
    img[y + 1, x + 11] = WOOD[0]
    img[y + 3, x + 11] = WOOD[0]
    darken_ellipse(img, x + 8, y + 14, 6, 2, 0.6)


def draw_rock(img, x, y):
    spr = parse_sprite([
        "..KKKK..",
        ".KLLMMK.",
        "KLMMMMdK",
        "KMMMMddK",
        ".KKKKKK.",
    ], {"K": OUT, "L": STONE[3], "M": STONE[2], "d": STONE[1]})
    darken_ellipse(img, x + 4, y + 5, 5, 2, 0.6)
    blit(img, spr, x, y)


def draw_bridge(img):
    x0, x1, y0, y1 = 60, 100, 108, 130
    for yy in range(y0, y1):
        for xx in range(x0, x1):
            k = (xx - x0) % 5
            c = WOOD[2] if k in (1, 2) else WOOD[3] if k == 0 else WOOD[1]
            if k == 4:
                c = WOOD[0]
            if (xx * 7 + yy * 3) % 23 == 0:
                c = WOOD[1]
            img[yy, xx] = c
    # rails
    for ry in (y0 - 3, y1 - 1):
        img[ry:ry + 3, x0 - 2:x1 + 2] = WOOD[2]
        img[ry, x0 - 2:x1 + 2] = WOOD[3]
        img[ry + 2, x0 - 2:x1 + 2] = WOOD[0]
        img[ry - 1, x0 - 2:x1 + 2] = OUT
        img[ry + 3, x0 - 2:x1 + 2] = OUT
        for px in (x0 - 2, x0 + 18, x1 - 1):
            img[ry - 3:ry + 4, px:px + 3] = WOOD[1]
            img[ry - 3, px:px + 3] = WOOD[3]
            img[ry - 4, px:px + 3] = OUT
            img[ry - 3:ry + 4, px - 1] = OUT
            img[ry - 3:ry + 4, px + 3] = OUT
    # shadow under bridge into water
    img[y1 + 3:y1 + 5, x0:x1] = (img[y1 + 3:y1 + 5, x0:x1] * 0.55).astype(np.uint8)


# ----------------------------------------------------------------------------- sprites
HERO_PAL = {
    "K": OUT, "H": C(236, 108, 44), "h": C(170, 56, 34), "L": C(255, 184, 96),
    "S": C(250, 196, 150), "s": C(206, 132, 96), "Y": C(255, 220, 64), "y": C(206, 140, 30),
    "T": C(96, 82, 214), "t": C(58, 44, 140), "B": C(118, 64, 34), "b": C(78, 42, 26),
    "P": C(70, 62, 110), "p": C(44, 38, 74), "w": C(240, 236, 220),
}
HERO_TOP = [
    "....KKKKKK......",
    "..KKHHLLLHKK....",
    ".KHHHHHHLLHHK...",
    ".KHHHHHHHHHHHK..",
    "KhHHHHHHHHHHHHK.",
    "KhhHHHHHHSHHHK..",
    "KhhHHHHSSSKSSK..",
    "KhhhHHSSSSKwSK..",
    ".KhhhKsSSSSSSSK.",
    "..KKKKKssSSSKK..",
]
HERO_BODY = [  # two arm-swing variants, scarf tail flutters
    [
        "...KYYYYYYYK....",
        ".KyyYYyTTTTTK...",
        "Kyy.KtTTTTSSK...",
        "....KtBBBBBK....",
    ],
    [
        "...KYYYYYYYK....",
        "KyyyYYyTTTTTK...",
        ".KK.KSTTTTTTK...",
        "....KtBBBBBK....",
    ],
]
HERO_LEGS = [
    ["...KPPK.KpPK....", "..KbbK...KbbbK..", "..KKK.....KKKK.."],
    ["....KPPpPK......", "....KbbbbbK.....", "....KKKKKKK....."],
    ["...KpPK.KPPK....", "..KbbK...KbbbK..", "..KKK.....KKKK.."],
    ["....KPPpPK......", "....KbbbbbK.....", "....KKKKKKK....."],
]


def hero_frame(step, idle=False, blink=False):
    top = list(HERO_TOP)
    if blink:
        top[6] = top[6].replace("K", "s", 2)
        top[6] = "K" + top[6][1:]
        top[6] = top[6][:10] + "S" + top[6][11:]
        top[7] = top[7][:10] + "K" + "S" + top[7][12:]
    body = HERO_BODY[0 if idle else (0 if step in (0, 1) else 1)]
    legs = HERO_LEGS[1] if idle else HERO_LEGS[step]
    return parse_sprite(top + body + legs, HERO_PAL)


WICK_PAL = {
    "K": OUT, "H": C(64, 150, 120), "h": C(36, 96, 92), "L": C(130, 210, 160), "d": C(18, 52, 58),
    "E": C(255, 236, 120), "e": C(255, 170, 60), "R": C(150, 74, 160), "r": C(98, 44, 116),
    "M": C(232, 196, 150), "G": C(255, 226, 110), "g": C(255, 150, 50), "F": C(60, 50, 52),
    "f": C(120, 100, 90), "Y": C(255, 250, 200),
}
WICK = [
    "..........KK..........",
    ".........KLHK.........",
    "........KLHHK.........",
    "........KHHHK.........",
    ".......KLHHHhK........",
    "......KLHHHHHhK.......",
    ".....KLHHHHHHHhK......",
    "....KLHHhhhhhHHhK.....",
    "....KHHhdddddhHHK.....",
    "...KLHhdEddEddhHhK....",
    "...KHHhdEddEddhHhK....",
    "...KHHhddddddhHHhK....",
    "...KHHHhddddhHHHhK....",
    "..KLHHHHhhhhHHHHhhK...",
    "..KRRRRRRRRRRRRRRrK...",
    ".K.KrRRRRRRRRRRRRrK...",
    ".fKMMKRRRRRRRRRRRrrK..",
    ".f.KKRRRRRRRRRRRRrrK..",
    "KfK.KRRRRRRRRRRRRrrK..",
    "KGgK.KRRRRRRRRRRrrrK..",
    "KGYgKKRrRRRRRRRRRrrK..",
    "KGGgK.KKKFFKKFFKKKK...",
    ".KKK....KFFK.KFFK.....",
    "........KKK...KKK.....",
]


def wick_frame(blink=False, squint=False):
    rows = list(WICK)
    if blink:
        rows[9] = rows[9].replace("E", "d")
        rows[10] = rows[10].replace("E", "e")
    elif squint:
        rows[9] = rows[9].replace("E", "d")
    return parse_sprite(rows, WICK_PAL)


BUBBLE = parse_sprite([
    ".KKKKKKK.",
    "KwwwwwwwK",
    "KwwwRwwwK",
    "KwwwRwwwK",
    "KwwwRwwwK",
    "KwwwRwwwK",
    "KwwwwwwwK",
    "KwwwRwwwK",
    "KwwwwwwwK",
    ".KKKwKKK.",
    "....KK...",
], {"K": OUT, "w": C(236, 232, 244), "R": C(230, 40, 64)})

FLOWER_PALS = [
    (C(255, 120, 170), C(255, 230, 120)),
    (C(255, 214, 72), C(220, 110, 40)),
    (C(170, 150, 255), C(255, 255, 210)),
    (C(240, 236, 250), C(255, 200, 70)),
]


# ----------------------------------------------------------------------------- font
def _g(*rows):
    return rows


FONT = {
    "A": _g(".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"),
    "B": _g("####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."),
    "C": _g(".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."),
    "D": _g("####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."),
    "E": _g("#####", "#....", "#....", "####.", "#....", "#....", "#####"),
    "F": _g("#####", "#....", "#....", "####.", "#....", "#....", "#...."),
    "G": _g(".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".####"),
    "H": _g("#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"),
    "I": _g("###", ".#.", ".#.", ".#.", ".#.", ".#.", "###"),
    "J": _g("..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##.."),
    "K": _g("#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"),
    "L": _g("#....", "#....", "#....", "#....", "#....", "#....", "#####"),
    "M": _g("#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"),
    "N": _g("#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"),
    "O": _g(".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."),
    "P": _g("####.", "#...#", "#...#", "####.", "#....", "#....", "#...."),
    "Q": _g(".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"),
    "R": _g("####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"),
    "S": _g(".####", "#....", "#....", ".###.", "....#", "....#", "####."),
    "T": _g("#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."),
    "U": _g("#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."),
    "V": _g("#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."),
    "W": _g("#...#", "#...#", "#...#", "#.#.#", "#.#.#", "#.#.#", ".#.#."),
    "X": _g("#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"),
    "Y": _g("#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."),
    "Z": _g("#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"),
    "a": _g(".....", ".....", ".###.", "....#", ".####", "#...#", ".####"),
    "b": _g("#....", "#....", "####.", "#...#", "#...#", "#...#", "####."),
    "c": _g(".....", ".....", ".###.", "#....", "#....", "#...#", ".###."),
    "d": _g("....#", "....#", ".####", "#...#", "#...#", "#...#", ".####"),
    "e": _g(".....", ".....", ".###.", "#...#", "#####", "#....", ".###."),
    "f": _g("..##", ".#..", ".#..", "###.", ".#..", ".#..", ".#.."),
    "g": _g(".....", ".....", ".####", "#...#", "#...#", "#...#", ".####", "....#", ".###."),
    "h": _g("#....", "#....", "#.##.", "##..#", "#...#", "#...#", "#...#"),
    "i": _g(".#.", "...", "##.", ".#.", ".#.", ".#.", "###"),
    "j": _g("...#", "....", "..##", "...#", "...#", "...#", "...#", "#..#", ".##."),
    "k": _g("#...", "#...", "#..#", "#.#.", "##..", "#.#.", "#..#"),
    "l": _g("##.", ".#.", ".#.", ".#.", ".#.", ".#.", "###"),
    "m": _g(".....", ".....", "##.#.", "#.#.#", "#.#.#", "#.#.#", "#.#.#"),
    "n": _g(".....", ".....", "#.##.", "##..#", "#...#", "#...#", "#...#"),
    "o": _g(".....", ".....", ".###.", "#...#", "#...#", "#...#", ".###."),
    "p": _g(".....", ".....", "####.", "#...#", "#...#", "#...#", "####.", "#....", "#...."),
    "q": _g(".....", ".....", ".####", "#...#", "#...#", "#...#", ".####", "....#", "....#"),
    "r": _g(".....", ".....", "#.##.", "##..#", "#....", "#....", "#...."),
    "s": _g(".....", ".....", ".####", "#....", ".###.", "....#", "####."),
    "t": _g(".#..", ".#..", "###.", ".#..", ".#..", ".#..", "..##"),
    "u": _g(".....", ".....", "#...#", "#...#", "#...#", "#..##", ".##.#"),
    "v": _g(".....", ".....", "#...#", "#...#", "#...#", ".#.#.", "..#.."),
    "w": _g(".....", ".....", "#...#", "#...#", "#.#.#", "#.#.#", ".#.#."),
    "x": _g(".....", ".....", "#...#", ".#.#.", "..#..", ".#.#.", "#...#"),
    "y": _g(".....", ".....", "#...#", "#...#", "#...#", "#...#", ".####", "....#", ".###."),
    "z": _g(".....", ".....", "#####", "...#.", "..#..", ".#...", "#####"),
    ".": _g(".", ".", ".", ".", ".", ".", "#"),
    ",": _g("..", "..", "..", "..", "..", ".#", ".#", "#."),
    "!": _g("#", "#", "#", "#", "#", ".", "#"),
    "?": _g(".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."),
    "'": _g("#", "#", "."),
    "-": _g("....", "....", "....", "####"),
    ":": _g(".", ".", "#", ".", ".", ".", "#"),
    " ": _g("...",),
}
FONT_BITS = {}
for ch, rows in FONT.items():
    w = max(len(r) for r in rows)
    a = np.zeros((9, w), bool)
    for y, r in enumerate(rows):
        for x, c in enumerate(r):
            a[y, x] = c == "#"
    FONT_BITS[ch] = a


def text_width(s):
    return sum(FONT_BITS[c].shape[1] + 1 for c in s) - 1


def draw_text(img, s, x, y, color, shadow, clip=None, hi_last=None):
    for i, ch in enumerate(s):
        a = FONT_BITS[ch]
        h, w = a.shape
        col = hi_last if (hi_last is not None and i == len(s) - 1) else color
        for (ox, oy, c) in ((1, 1, shadow), (0, 1, shadow), (0, 0, col)):
            ys, xs = np.nonzero(a)
            ys = ys + y + oy
            xs = xs + x + ox
            ok = (ys >= 0) & (ys < H) & (xs >= 0) & (xs < W)
            if clip is not None:
                ok &= ys < clip
            img[ys[ok], xs[ok]] = c
        x += w + 1


# ----------------------------------------------------------------------------- timeline
WALK_END = 82
HERO_X0, HERO_X1 = 6, 144
HERO_Y = 106
WICK_X, WICK_Y = 172, 100
BUBBLE_T0, BUBBLE_T1 = 84, 104
BOX_T0, BOX_T1 = 98, 112
TYPE_T0 = 114

NAME = "WICK"
LINES = ["The old tube still glows...", "want to see?"]


def build_type_schedule():
    """list of (frame, line_index, char_count) reveal times"""
    events = []
    t = TYPE_T0
    for li, line in enumerate(LINES):
        for ci, ch in enumerate(line):
            events.append((t, li, ci + 1))
            if ch == ".":
                t += 5
            elif ch == " ":
                t += 1
            else:
                t += 2
        t += 8
    return events, t


TYPE_EVENTS, TYPE_END = build_type_schedule()


def ease_out_back(u):
    u = min(max(u, 0.0), 1.0)
    c1 = 1.5
    return 1 + (c1 + 1) * (u - 1) ** 3 + c1 * (u - 1) ** 2


def hero_state(t):
    if t < WALK_END:
        u = t / WALK_END
        # constant speed with a short ease-out at the end
        k = 0.88
        if u < k:
            x = HERO_X0 + (HERO_X1 - HERO_X0) * (u / k) * 0.94
        else:
            v = (u - k) / (1 - k)
            x = HERO_X0 + (HERO_X1 - HERO_X0) * (0.94 + 0.06 * (1 - (1 - v) ** 2))
        dist = x - HERO_X0
        step = int(dist / 7) % 4
        bob = 1 if step in (1, 3) else 0
        return x, step, False, -bob
    return HERO_X1, 1, True, 0


# ----------------------------------------------------------------------------- dialog box
BOX_X0, BOX_X1 = 8, 248
BOX_H = 62
BOX_YF = 156


def draw_box(img, y0, t, wick_portrait):
    x0, x1 = BOX_X0, BOX_X1
    y1 = y0 + BOX_H
    ya, yb = max(y0, 0), min(y1, H)
    if ya >= yb:
        return
    # gradient fill with ordered dither between 5 blues
    blues = np.stack([C(10, 14, 60), C(18, 30, 104), C(28, 52, 150), C(40, 78, 190), C(64, 108, 220)])
    yy = YY[ya:yb, x0:x1]
    v = 1.0 - (yy - y0) / BOX_H
    v = 0.15 + 0.8 * v
    lv = quantize(v, 5)[ya - 0:yb, :] if False else np.clip(np.floor(v * 4 + BAYER[ya:yb, x0:x1]), 0, 4).astype(int)
    region = img[ya:yb, x0:x1]
    region[:] = blues[lv]
    # rounded corner mask + border (outer dark, 2px light frame with shading, inner dark)
    hh, ww = y1 - y0, x1 - x0
    ly, lx = np.mgrid[0:hh, 0:ww]
    cr = 5
    dx = np.maximum(np.maximum(cr - lx, lx - (ww - 1 - cr)), 0)
    dy = np.maximum(np.maximum(cr - ly, ly - (hh - 1 - cr)), 0)
    dist_in = np.where((dx > 0) & (dy > 0), cr - np.sqrt(dx * dx + dy * dy),
                       np.minimum(np.minimum(lx, ww - 1 - lx), np.minimum(ly, hh - 1 - ly)))
    full = np.zeros((hh, ww, 3), np.uint8)
    full[:] = 0
    sl = slice(ya - y0, yb - y0)
    di = dist_in[sl]
    reg = img[ya:yb, x0:x1]
    under = img[ya:yb, x0:x1].copy()
    border_lt = C(236, 238, 252)
    border_md = C(170, 180, 220)
    border_dk = C(96, 104, 160)
    reg[di < 0] = under[di < 0]  # outside corner: keep (already overwritten by fill, restore below)
    reg[(di >= 0) & (di < 1)] = OUT
    lyy = ly[sl]
    lxx = lx[sl]
    ring1 = (di >= 1) & (di < 2)
    ring2 = (di >= 2) & (di < 3)
    reg[ring1] = border_lt
    reg[ring2 & ((lyy > hh / 2) | (lxx > ww - 6))] = border_dk
    reg[ring2 & ~((lyy > hh / 2) | (lxx > ww - 6))] = border_md
    reg[(di >= 3) & (di < 4)] = OUT
    return dist_in, sl


def box_frame(img, y0, t, base_under):
    x0, x1 = BOX_X0, BOX_X1
    ya, yb = max(y0, 0), min(y0 + BOX_H, H)
    if ya >= yb:
        return
    under = img[ya:yb, x0:x1].copy()
    res = draw_box(img, y0, t, None)
    if res is None:
        return
    dist_in, sl = res
    di = dist_in[sl]
    reg = img[ya:yb, x0:x1]
    reg[di < 0] = under[di < 0]
    # drop shadow below box
    if y0 + BOX_H < H - 1:
        sy = y0 + BOX_H
        img[sy:sy + 2, x0 + 4:x1 - 2] = (img[sy:sy + 2, x0 + 4:x1 - 2] * 0.45).astype(np.uint8)


# ----------------------------------------------------------------------------- main
def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else "frames"
    os.makedirs(out_dir, exist_ok=True)
    rng = np.random.default_rng(42)

    base, water, path = build_static()
    occ = np.zeros((H, W), bool)

    # objects (drawn into the static layer, sorted by base y)
    objs = []
    for i, x in enumerate(range(-6, W + 16, 19)):
        objs.append(("tree", x + int(rng.integers(-3, 4)), 18 + (i % 2) * 8 + int(rng.integers(0, 3)), 1.0))
    objs += [("tree", 246, 62, 0.9), ("tree", 244, 104, 0.85), ("tree", 240, 152, 0.95),
             ("tree", 120, 52, 0.8), ("tree", 222, 202, 1.0), ("tree", 150, 212, 0.95), ("tree", 6, 102, 0.75)]
    objs.append(("house", 0, 86, 1.0))
    objs.append(("sign", 22, 92, 1.0))
    objs += [("rock", 112, 96, 1), ("rock", 136, 144, 1), ("rock", 34, 150, 1), ("rock", 226, 120, 1)]
    objs.sort(key=lambda o: o[2])
    for kind, x, y, s in objs:
        if kind == "tree":
            draw_tree(base, occ, x, y, rng, s)
        elif kind == "house":
            draw_house(base)
            occ[8:90, 156:230] = True
        elif kind == "sign":
            draw_sign(base, x, y)
            occ[y - 3:y + 15, x - 1:x + 16] = True
        elif kind == "rock":
            draw_rock(base, x, y)
    draw_bridge(base)
    occ[104:134, 56:104] = True
    water_vis = water & ~occ
    # remember which water pixels were overdrawn by bridge/trees
    water_vis &= ~((YY >= 104) & (YY < 134) & (XX >= 56) & (XX < 104))

    # shore shadow (grass overhang) and foam masks
    land = ~water
    shadow_m = water & (shift(land, 2, 0) | shift(land, 3, 0) | shift(land, 1, 0))
    foam_m = water & (shift(land, 1, 0) | shift(land, -1, 0) | shift(land, 0, 1) | shift(land, 0, -1))
    flow_bias = np.zeros((H, W), np.float32)

    # flowers
    flowers = []
    tries = 0
    while len(flowers) < 34 and tries < 4000:
        tries += 1
        x, y = int(rng.integers(4, W - 6)), int(rng.integers(30, H - 6))
        if dilate_cache_ok(x, y, water, path, occ):
            flowers.append((x, y, int(rng.integers(0, 4)), int(rng.integers(0, 40))))

    # fireflies
    flies = []
    for i in range(16):
        flies.append(dict(x=rng.uniform(10, W - 10), y=rng.uniform(20, 200), ax=rng.uniform(8, 22),
                          ay=rng.uniform(5, 12), wx=rng.uniform(0.02, 0.05), wy=rng.uniform(0.03, 0.07),
                          px=rng.uniform(0, 6.28), py=rng.uniform(0, 6.28), wb=rng.uniform(0.05, 0.12),
                          pb=rng.uniform(0, 6.28), dx=rng.uniform(-0.12, 0.12)))
    # water sparkles
    wy, wx = np.nonzero(water_vis & ~shadow_m & ~foam_m)
    sparkles = []
    for i in range(80):
        k = int(rng.integers(0, len(wy)))
        sparkles.append((i * 3 + int(rng.integers(0, 3)), int(wx[k]), int(wy[k])))
    # smoke puffs
    puffs = [(i * 9.0 + rng.uniform(0, 4), rng.uniform(-0.6, 0.6)) for i in range(-12, 30)]

    portrait_src = wick_frame()

    for t in range(NFRAMES):
        img = base.copy()
        tq = t // 3  # water animates at 10 fps like an old tile animation

        # ---- water
        ph = tq * 0.55
        v = (0.5 + 0.22 * np.sin(XX * 0.26 + np.sin(YY * 0.19 + ph * 0.6) * 1.8 + ph)
             + 0.20 * np.sin(YY * 0.42 - ph * 1.3 + np.sin(XX * 0.11) * 2.0))
        v = np.clip(v, 0, 1)
        lv = np.clip(np.floor(v * 3 + BAYER), 0, 3).astype(int)
        wcol = np.stack(WATER[:4])[lv]
        hl = (v > 0.86) & ((YY % 2) == 0)
        wcol[hl] = WATER[4]
        wcol[shadow_m] = (wcol[shadow_m] * 0.55).astype(np.uint8)
        foam = foam_m & (((XX + YY * 2 + tq) % 5) < 2)
        wcol[foam] = WATER[3]
        img[water_vis] = wcol[water_vis]

        # ---- flowers (sway)
        for (fx, fy, k, phs) in flowers:
            petal, centre = FLOWER_PALS[k]
            off = 1 if ((t + phs) // 20) % 2 else 0
            img[fy + 1, fx] = GRASS[0]
            img[fy, fx] = GRASS[1]
            cx, cy = fx + off, fy - 2
            for dx_, dy_ in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                img[cy + dy_, cx + dx_] = petal
            img[cy, cx] = centre

        # ---- windows glow (flicker)
        flick = 0.85 + 0.15 * math.sin(t * 0.7) * math.sin(t * 0.23 + 1)
        for (wx0, wyy) in ((200, 62), (166, 62)):
            for yy in range(wyy, wyy + 11):
                for xx in range(wx0, wx0 + 12):
                    if xx in (wx0 + 5, wx0 + 6) or yy == wyy + 5:
                        img[yy, xx] = WOOD[1]
                    else:
                        g = 1.0 - 0.35 * (yy - wyy) / 11.0
                        c = np.array([255, 200 * g * flick + 30, 90 * g * flick], np.float32)
                        img[yy, xx] = np.clip(c, 0, 255).astype(np.uint8)

        # ---- characters
        hx, step, idle, bob = hero_state(t)
        hero_blink = idle and (t % 70) in (40, 41, 42)
        darken_ellipse(img, hx + 7, HERO_Y + 17, 6, 2, 0.5, dither=False)
        blit(img, hero_frame(step, idle, hero_blink), hx, HERO_Y + bob)

        # wick: idle bob, a hop when the hero arrives, bob while talking
        talking = TYPE_T0 <= t < TYPE_END
        wbob = 0
        if t < BUBBLE_T0:
            wbob = -1 if (t // 14) % 2 else 0
        elif t < BUBBLE_T0 + 12:
            u = (t - BUBBLE_T0) / 12.0
            wbob = -int(round(7 * math.sin(math.pi * u)))
        elif talking:
            wbob = -1 if (t // 4) % 2 else 0
        else:
            wbob = -1 if (t // 14) % 2 else 0
        wblink = (t % 57) in (20, 21, 22) and not talking
        squint = talking and (t // 6) % 3 == 0
        darken_ellipse(img, WICK_X + 11, WICK_Y + 24, 8, 2, 0.5, dither=False)
        blit(img, wick_frame(wblink, squint), WICK_X, WICK_Y + wbob)

        if BUBBLE_T0 <= t < BUBBLE_T1:
            pop = t - BUBBLE_T0
            by = WICK_Y - 14 + wbob - (2 if pop < 2 else 0)
            blit(img, BUBBLE, WICK_X + 8, by)

        # ---- smoke from the chimney
        for (t0, drift) in puffs:
            age = t - t0
            if age < 0 or age > 60:
                continue
            u = age / 60.0
            px = 211 + drift * age * 0.3 + 6 * u * u * 10 * 0.4 + math.sin(age * 0.15 + t0) * 1.5
            py = 6 - age * 0.55
            r = 1.5 + u * 4.5
            m = ((XX - px) ** 2 + (YY - py) ** 2) <= r * r
            dens = 1 - u
            m &= BAYER < dens * 0.9
            col = np.array([150, 150, 190], np.float32) * (0.7 + 0.3 * dens)
            img[m] = (img[m] * 0.4 + col * 0.6).astype(np.uint8)

        # ---- additive glows: lantern, windows, fireflies, sparkles
        acc = np.zeros((H, W, 3), np.float32)
        lf = 0.8 + 0.2 * math.sin(t * 0.9) * math.cos(t * 0.37)
        add_glow(acc, WICK_X + 2, WICK_Y + wbob + 20, 6, (255, 190, 80), 90 * lf)
        add_glow(acc, 206, 67, 9, (255, 180, 70), 50 * flick)
        add_glow(acc, 172, 67, 9, (255, 180, 70), 50 * flick)
        add_glow(acc, 188, 74.5, 2, (255, 200, 90), 30)

        for f in flies:
            fx = f["x"] + f["ax"] * math.sin(t * f["wx"] + f["px"]) + f["dx"] * t
            fy = f["y"] + f["ay"] * math.sin(t * f["wy"] + f["py"])
            fx = (fx + 10) % (W + 20) - 10
            b = math.sin(t * f["wb"] + f["pb"])
            b = max(0.0, b) ** 0.7
            if b < 0.05:
                continue
            add_glow(acc, fx, fy, 3.0, (170, 255, 90), 140 * b)
            ix, iy = int(round(fx)), int(round(fy))
            if 1 <= ix < W - 1 and 1 <= iy < H - 1:
                core = np.array([230, 255, 150], np.float32)
                img[iy, ix] = np.clip(img[iy, ix] * (1 - b) + core * b, 0, 255)
                if b > 0.5:
                    for dx_, dy_ in ((1, 0), (-1, 0), (0, 1), (0, -1)):
                        img[iy + dy_, ix + dx_] = np.clip(img[iy + dy_, ix + dx_] * 0.5 + np.array([150, 220, 70]) * 0.5, 0, 255)

        for (t0, sx, sy) in sparkles:
            age = t - t0
            if age < 0 or age >= 8:
                continue
            size = [0, 1, 2, 3, 2, 1, 1, 0][age]
            img[sy, sx] = WATER[4]
            for k in range(1, size + 1):
                c = WATER[4] if k < size else WATER[3]
                for dx_, dy_ in ((k, 0), (-k, 0), (0, k), (0, -k)):
                    if 0 <= sx + dx_ < W and 0 <= sy + dy_ < H and water[sy + dy_, sx + dx_]:
                        img[sy + dy_, sx + dx_] = c

        img = np.clip(img.astype(np.float32) + acc, 0, 255).astype(np.uint8)

        # ---- dialog window
        if t >= BOX_T0:
            u = (t - BOX_T0) / float(BOX_T1 - BOX_T0)
            y0 = int(round(H + (BOX_YF - H) * ease_out_back(u)))
            box_frame(img, y0, t, None)
            # portrait inset
            px0, py0 = BOX_X0 + 8, y0 + 9
            inset = img[max(py0, 0):max(min(py0 + 44, H), 0), px0:px0 + 44]
            if inset.size:
                hh = inset.shape[0]
                g = np.linspace(0, 1, 44)[:hh]
                inset[:] = (np.array([12, 20, 70]) * (1 - g[:, None, None]) + np.array([30, 50, 120]) * g[:, None, None]).astype(np.uint8)
                inset[0, :] = C(96, 104, 160)
                if hh == 44:
                    inset[-1, :] = C(170, 180, 220)
                inset[:, 0] = C(96, 104, 160)
                inset[:, -1] = C(170, 180, 220)
            rgb, a = portrait_src
            prgb = np.repeat(np.repeat(rgb[1:21, 0:22], 2, 0), 2, 1)
            pa = np.repeat(np.repeat(a[1:21, 0:22], 2, 0), 2, 1)
            if talking and (t // 6) % 3 == 0:
                pr2, pa2 = wick_frame(squint=True)
                prgb = np.repeat(np.repeat(pr2[1:21, 0:22], 2, 0), 2, 1)
            # clip portrait inside inset
            pspr = (prgb[:42, :42], pa[:42, :42])
            tmp = img.copy()
            blit(tmp, pspr, px0 + 1, py0 + 1)
            clipm = np.zeros((H, W), bool)
            clipm[max(py0 + 1, 0):max(min(py0 + 43, H), 0), px0 + 1:px0 + 43] = True
            img[clipm] = tmp[clipm]

            tx = px0 + 52
            draw_text(img, NAME, tx, y0 + 9, C(255, 206, 72), C(8, 10, 40))
            nw = text_width(NAME)
            if y0 + 19 < H:  # the window slides up from below the screen
                img[y0 + 18, tx:tx + nw + 1] = C(255, 206, 72)
                img[y0 + 19, tx + 1:tx + nw + 2] = C(8, 10, 40)
            shown = [0, 0]
            fresh = None
            for (et, li, cc) in TYPE_EVENTS:
                if et <= t:
                    shown[li] = cc
                    if et == t or et == t - 1:
                        fresh = li
            for li, line in enumerate(LINES):
                s = line[:shown[li]]
                if s:
                    draw_text(img, s, tx, y0 + 24 + li * 13, C(244, 238, 222), C(8, 10, 40),
                              hi_last=C(255, 255, 160) if fresh == li else None)
            # blinking "next" arrow
            if t >= TYPE_END:
                if ((t - TYPE_END) // 8) % 2 == 0:
                    ay = y0 + BOX_H - 14 + (1 if (t // 4) % 2 else 0)
                    ax = BOX_X1 - 18
                    arrow = ["#######", ".#####.", "..###..", "...#..."]
                    for r, row in enumerate(arrow):
                        for c_, ch in enumerate(row):
                            if ch == "#":
                                img[ay + r + 1, ax + c_ + 1] = C(8, 10, 40)
                    for r, row in enumerate(arrow):
                        for c_, ch in enumerate(row):
                            if ch == "#":
                                img[ay + r, ax + c_] = C(255, 214, 80) if r < 2 else C(255, 160, 40)

        Image.fromarray(img, "RGB").save(os.path.join(out_dir, "%05d.png" % t))


def dilate_cache_ok(x, y, water, path, occ):
    y0, y1, x0, x1 = y - 5, y + 4, x - 4, x + 5
    if y0 < 0 or x0 < 0 or y1 > H or x1 > W:
        return False
    if water[y0:y1, x0:x1].any() or path[y0:y1, x0:x1].any() or occ[y0:y1, x0:x1].any():
        return False
    # keep clear of character feet line
    if 96 <= y <= 132 and 0 <= x <= 200:
        return False
    return True


if __name__ == "__main__":
    main()
