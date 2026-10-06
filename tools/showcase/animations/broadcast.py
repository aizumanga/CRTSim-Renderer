#!/usr/bin/env python3
"""broadcast: late-night analog TV from an invented local station.

Snow resolves into an original test card for "CHANNEL 7½ / HALCYON BAY", which cuts to a
chrome flying-logo station ident, finished with the TV's own green on-screen display
(channel number and a rising volume bar).

    python3 broadcast.py OUT_DIR      -> 240 RGB PNGs, 640x480, 30 fps
"""
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

W, H = 640, 480
N_FRAMES = 240

FONT_DIR = "/usr/share/fonts/opentype/inter/"
F_BLACK = FONT_DIR + "Inter-Black.otf"
F_BLACK_I = FONT_DIR + "Inter-BlackItalic.otf"
F_XB_I = FONT_DIR + "Inter-ExtraBoldItalic.otf"
F_BOLD = FONT_DIR + "Inter-Bold.otf"
F_BOLD_I = FONT_DIR + "Inter-BoldItalic.otf"
F_SEMI = FONT_DIR + "Inter-SemiBold.otf"
F_MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"

# timeline (frames)
T_SNOW_END = 12        # pure-ish snow until here
T_LOCK = 34            # picture fully resolved/locked
T_CUT = 102            # test card -> ident
T_IDENT = 104
T_LAND = 150           # logo lands
T_OSD = 186            # "CH 07" appears
T_VOL = 192            # volume bar slides up


def font(path, size):
    return ImageFont.truetype(path, size)


def smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0.0, 1.0)
    return t * t * (3 - 2 * t)


def sstep(a, b, x):
    t = min(max((x - a) / (b - a), 0.0), 1.0)
    return t * t * (3 - 2 * t)


def ease_out(t):
    t = min(max(t, 0.0), 1.0)
    return 1 - (1 - t) ** 3


def to_np(img):
    return np.asarray(img, dtype=np.float32) / 255.0


def over(dst, rgb, a):
    """alpha-composite colour/array rgb with alpha array a onto dst (in place)."""
    a = a[..., None]
    dst *= (1 - a)
    dst += np.asarray(rgb, dtype=np.float32) * a


def text_mask(txt, fnt, size, xy, anchor="mm", ss=1, stroke=0):
    """Anti-aliased text alpha mask (H,W) float."""
    im = Image.new("L", (size[0] * ss, size[1] * ss), 0)
    d = ImageDraw.Draw(im)
    d.text((xy[0] * ss, xy[1] * ss), txt, font=fnt, fill=255, anchor=anchor,
           stroke_width=stroke * ss, stroke_fill=255)
    if ss > 1:
        im = im.reduce(ss)
    return to_np(im)


def shift(a, dx, dy):
    """Shift 2D array by integer offsets, filling with zeros."""
    out = np.zeros_like(a)
    h, w = a.shape[:2]
    xs0, xs1 = max(0, -dx), min(w, w - dx)
    ys0, ys1 = max(0, -dy), min(h, h - dy)
    if xs1 > xs0 and ys1 > ys0:
        out[ys0 + dy:ys1 + dy, xs0 + dx:xs1 + dx] = a[ys0:ys1, xs0:xs1]
    return out


def dilate(a, r):
    im = Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8))
    return to_np(im.filter(ImageFilter.MaxFilter(2 * r + 1)))


def blur(a, r):
    im = Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8))
    return to_np(im.filter(ImageFilter.GaussianBlur(r)))


YY, XX = np.mgrid[0:H, 0:W].astype(np.float32)

# ---------------------------------------------------------------------------
# 5x7 bitmap font for the TV's on-screen display
# ---------------------------------------------------------------------------
GLYPHS = {
    "C": [" ### ", "#   #", "#    ", "#    ", "#    ", "#   #", " ### "],
    "H": ["#   #", "#   #", "#   #", "#####", "#   #", "#   #", "#   #"],
    "V": ["#   #", "#   #", "#   #", "#   #", "#   #", " # # ", "  #  "],
    "O": [" ### ", "#   #", "#   #", "#   #", "#   #", "#   #", " ### "],
    "L": ["#    ", "#    ", "#    ", "#    ", "#    ", "#    ", "#####"],
    "U": ["#   #", "#   #", "#   #", "#   #", "#   #", "#   #", " ### "],
    "M": ["#   #", "## ##", "# # #", "# # #", "#   #", "#   #", "#   #"],
    "E": ["#####", "#    ", "#    ", "#### ", "#    ", "#    ", "#####"],
    "S": [" ####", "#    ", "#    ", " ### ", "    #", "    #", "#### "],
    "T": ["#####", "  #  ", "  #  ", "  #  ", "  #  ", "  #  ", "  #  "],
    "R": ["#### ", "#   #", "#   #", "#### ", "# #  ", "#  # ", "#   #"],
    "0": [" ### ", "#   #", "#   #", "#   #", "#   #", "#   #", " ### "],
    "1": ["  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### "],
    "2": [" ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####"],
    "3": ["#####", "   # ", "  #  ", "   # ", "    #", "#   #", " ### "],
    "4": ["   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # "],
    "5": ["#####", "#    ", "#### ", "    #", "    #", "#   #", " ### "],
    "6": ["  ## ", " #   ", "#    ", "#### ", "#   #", "#   #", " ### "],
    "7": ["#####", "    #", "   # ", "  #  ", " #   ", " #   ", " #   "],
    "8": [" ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### "],
    "9": [" ### ", "#   #", "#   #", " ####", "    #", "   # ", " ##  "],
    " ": ["     "] * 7,
}


def bitmap_mask(txt, x, y, s, mask=None):
    """Draw 5x7 bitmap text into an (H,W) mask at pixel scale s. Returns mask."""
    if mask is None:
        mask = np.zeros((H, W), np.float32)
    cx = x
    for ch in txt:
        g = GLYPHS[ch]
        for r, row in enumerate(g):
            for c, px in enumerate(row):
                if px == "#":
                    y0, x0 = y + r * s, cx + c * s
                    if 0 <= y0 and y0 + s <= H and 0 <= x0 and x0 + s <= W:
                        mask[y0:y0 + s, x0:x0 + s] = 1.0
        cx += 6 * s
    return mask


# ---------------------------------------------------------------------------
# Test card (static layer built at 3x and box-filtered down)
# ---------------------------------------------------------------------------
TC_CX, TC_CY, TC_R = 320, 240, 200
RAINBOW = [(235, 40, 50), (250, 140, 20), (240, 225, 30), (40, 200, 70),
           (20, 200, 220), (40, 90, 235), (130, 50, 220), (220, 40, 190)]


def build_test_card():
    S = 3
    w3, h3 = W * S, H * S
    bg = Image.new("RGB", (w3, h3), (46, 52, 74))
    d = ImageDraw.Draw(bg)
    # edge columns: hue ramp on the left, luma ramp on the right
    for i in range(12):
        hue = i / 12.0
        r, g, b = [int(255 * (0.5 + 0.42 * math.cos(2 * math.pi * (hue + k / 3.0)))) for k in range(3)]
        d.rectangle([0, i * 40 * S, 40 * S, (i + 1) * 40 * S], fill=(r, g, b))
        v = int(20 + 190 * i / 11.0)
        d.rectangle([600 * S, i * 40 * S, 640 * S, (i + 1) * 40 * S], fill=(v, v, v))
    # top & bottom rows: alternating dark/teal cells
    for i in range(1, 15):
        c = (24, 28, 44) if i % 2 else (30, 96, 110)
        d.rectangle([i * 40 * S, 0, (i + 1) * 40 * S, 40 * S], fill=c)
        c = (24, 28, 44) if (i + 1) % 2 else (110, 40, 90)
        d.rectangle([i * 40 * S, 440 * S, (i + 1) * 40 * S, 480 * S], fill=c)
    # convergence grid: 1 px lines every 40 px
    for gx in range(0, W + 1, 40):
        d.rectangle([gx * S - 1, 0, gx * S + 1, h3], fill=(205, 208, 215))
    for gy in range(0, H + 1, 40):
        d.rectangle([0, gy * S - 1, w3, gy * S + 1], fill=(205, 208, 215))
    # corner targets
    for (cx, cy) in [(80, 80), (560, 80), (80, 400), (560, 400)]:
        r = 26
        d.ellipse([(cx - r) * S, (cy - r) * S, (cx + r) * S, (cy + r) * S], fill=(16, 18, 30),
                  outline=(225, 225, 230), width=4)
        d.ellipse([(cx - 9) * S, (cy - 9) * S, (cx + 9) * S, (cy + 9) * S], outline=(225, 225, 230), width=3)

    # ---- inner circle content
    inner = Image.new("RGB", (w3, h3), (0, 0, 0))
    di = ImageDraw.Draw(inner)
    x0, x1 = 120, 520
    bw = (x1 - x0) / 8.0
    for i, c in enumerate(RAINBOW):
        di.rectangle([int((x0 + i * bw) * S), 40 * S, int((x0 + (i + 1) * bw) * S), 130 * S], fill=c)
    # timing checker strip
    for i in range(0, 400 // 8):
        c = (200, 200, 200) if i % 2 == 0 else (10, 10, 10)
        di.rectangle([(x0 + i * 8) * S, 130 * S, (x0 + i * 8 + 8) * S - 1, 136 * S], fill=c)
    # station name panel
    di.rectangle([0, 136 * S, w3, 196 * S], fill=(8, 8, 20))
    f = font(F_BLACK, 40 * S)
    di.text((320 * S, 166 * S), "CHANNEL 7½", font=f, fill=(236, 228, 205), anchor="mm")
    # grey steps
    steps = 8
    for i in range(steps):
        v = int(14 + 186 * i / (steps - 1))
        di.rectangle([int((x0 + i * 50) * S), 196 * S, int((x0 + (i + 1) * 50) * S), 284 * S], fill=(v, v, v))
    # multiburst gratings
    periods = [10, 8, 6, 4, 3, 2]
    gw = 400 // len(periods)
    di.rectangle([0, 284 * S, w3, 338 * S], fill=(100, 100, 100))
    for i, p in enumerate(periods):
        gx0 = x0 + i * gw + 4
        for k in range(0, gw - 8, p):
            if (k // p) % 2 == 0:
                continue
        for xx in range(gx0, gx0 + gw - 8):
            on = ((xx - gx0) // (p // 2 if p > 2 else 1)) % 2 == 0
            if on:
                di.rectangle([xx * S, 288 * S, xx * S + S - 1, 334 * S], fill=(205, 205, 205))
            else:
                di.rectangle([xx * S, 288 * S, xx * S + S - 1, 334 * S], fill=(12, 12, 12))
    # lower panel with location
    di.rectangle([0, 338 * S, w3, 386 * S], fill=(14, 22, 80))
    f2 = font(F_BOLD, 22 * S)
    di.text((320 * S, 356 * S), "HALCYON BAY  •  UHF 47", font=f2, fill=(250, 210, 60), anchor="mm")
    f3 = font(F_SEMI, 11 * S)
    di.text((320 * S, 376 * S), "TEST TRANSMISSION  —  PROGRAMMES RESUME AT 06:00", font=f3,
            fill=(150, 190, 255), anchor="mm")
    # luma ramp bottom
    ramp = np.linspace(0, 1, 400 * S)
    ramp_rgb = np.stack([30 + 200 * ramp, 10 + 60 * ramp, 120 + 100 * (1 - ramp)], -1)
    ramp_img = np.repeat(ramp_rgb[None], (440 - 386) * S, 0).astype(np.uint8)
    inner.paste(Image.fromarray(ramp_img), (120 * S, 386 * S))
    # centre cross + circle
    di.rectangle([320 * S - 1, 196 * S, 320 * S + 1, 284 * S], fill=(240, 60, 60))
    di.rectangle([270 * S, 240 * S - 1, 370 * S, 240 * S + 1], fill=(240, 60, 60))
    di.ellipse([(320 - 22) * S, (240 - 22) * S, (320 + 22) * S, (240 + 22) * S], outline=(240, 60, 60), width=4)

    mask = Image.new("L", (w3, h3), 0)
    ImageDraw.Draw(mask).ellipse([(TC_CX - TC_R) * S, (TC_CY - TC_R) * S, (TC_CX + TC_R) * S,
                                  (TC_CY + TC_R) * S], fill=255)
    card = Image.composite(inner, bg, mask)
    dc = ImageDraw.Draw(card)
    dc.ellipse([(TC_CX - TC_R) * S, (TC_CY - TC_R) * S, (TC_CX + TC_R) * S, (TC_CY + TC_R) * S],
               outline=(230, 230, 235), width=5)
    # header strip
    dc.rounded_rectangle([150 * S, 8 * S, 490 * S, 32 * S], radius=6 * S, fill=(10, 12, 26),
                         outline=(120, 130, 170), width=3)
    dc.text((320 * S, 20 * S), "HALCYON BAY TELEVISION", font=font(F_BOLD, 15 * S),
            fill=(225, 230, 245), anchor="mm")
    # footer strip for ON AIR / tone / clock
    dc.rounded_rectangle([46 * S, 446 * S, 594 * S, 474 * S], radius=6 * S, fill=(8, 10, 22),
                         outline=(120, 130, 170), width=3)
    card = card.reduce(S)
    return to_np(card)


def draw_test_overlay(img, f):
    """Animated parts of the test card: ON AIR lamp, 1 kHz tone scope, meter, clock, sweeps."""
    S = 3
    t = f / 30.0
    # ON AIR lamp (blinks)
    on = (f // 12) % 2 == 0 or f < T_LOCK
    patch = Image.new("RGBA", (110 * S, 22 * S), (0, 0, 0, 0))
    d = ImageDraw.Draw(patch)
    if on:
        d.rounded_rectangle([0, 0, 110 * S - 1, 22 * S - 1], radius=6 * S, fill=(235, 30, 40, 255),
                            outline=(255, 150, 140, 255), width=S)
        d.text((55 * S, 11 * S), "ON AIR", font=font(F_BLACK, 14 * S), fill=(255, 240, 220, 255), anchor="mm")
    else:
        d.rounded_rectangle([0, 0, 110 * S - 1, 22 * S - 1], radius=6 * S, fill=(70, 10, 16, 255),
                            outline=(120, 40, 40, 255), width=S)
        d.text((55 * S, 11 * S), "ON AIR", font=font(F_BLACK, 14 * S), fill=(130, 60, 60, 255), anchor="mm")
    patch = patch.reduce(S)
    img.alpha_composite(patch, (52, 449))
    if on:  # glow halo
        glow = Image.new("RGBA", (150, 60), (0, 0, 0, 0))
        ImageDraw.Draw(glow).rounded_rectangle([18, 18, 132, 42], radius=8, fill=(255, 40, 40, 110))
        glow = glow.filter(ImageFilter.GaussianBlur(7))
        img.alpha_composite(glow, (32, 430))
        img.alpha_composite(patch, (52, 449))

    # tone label + scope
    d = ImageDraw.Draw(img)
    d.text((172, 460), "TONE 1 kHz", font=font(F_BOLD, 12), fill=(150, 230, 170), anchor="lm")
    sx0, sy0, sw, sh = 252, 450, 150, 20
    d.rectangle([sx0, sy0, sx0 + sw, sy0 + sh], fill=(0, 22, 8), outline=(40, 120, 60))
    sc = Image.new("RGBA", (sw * S, sh * S), (0, 0, 0, 0))
    ds = ImageDraw.Draw(sc)
    for gx in range(1, 6):
        ds.line([(gx * sw * S // 6, 0), (gx * sw * S // 6, sh * S)], fill=(20, 70, 35, 255), width=1)
    pts = []
    for i in range(sw * S + 1):
        u = i / (sw * S)
        y = 0.5 + 0.38 * math.sin(2 * math.pi * (7 * u - 1.3 * t))
        pts.append((i, y * sh * S))
    ds.line(pts, fill=(120, 255, 140, 255), width=2 * S)
    img.alpha_composite(sc.reduce(S), (sx0, sy0))
    # level meter: steady tone at -18 with tiny flicker
    lv = 13 + (1 if (f * 7) % 5 == 0 else 0)
    for i in range(18):
        x = 412 + i * 5
        col = (60, 220, 90) if i < 12 else ((240, 210, 40) if i < 15 else (240, 60, 50))
        if i >= lv:
            col = tuple(c // 5 for c in col)
        d.rectangle([x, 452, x + 3, 467], fill=col)
    # clock
    secs = 2 * 3600 + 47 * 60 + 13 + f // 30
    hh, mm, ss = secs // 3600, (secs // 60) % 60, secs % 60
    d.text((586, 460), "%02d:%02d:%02d" % (hh, mm, ss), font=font(F_MONO, 15), fill=(250, 210, 60), anchor="rm")

    # sweep hands in the corner targets
    sw_img = Image.new("RGBA", (W * 2, H * 2), (0, 0, 0, 0))
    dd = ImageDraw.Draw(sw_img)
    for k, (cx, cy) in enumerate([(80, 80), (560, 80), (80, 400), (560, 400)]):
        ang = 2 * math.pi * (t * 0.5 + k * 0.25) * (1 if k % 2 == 0 else -1)
        for j in range(10):
            a2 = ang - j * 0.06 * (1 if k % 2 == 0 else -1)
            ex, ey = cx + 23 * math.cos(a2), cy + 23 * math.sin(a2)
            alpha = int(230 * (1 - j / 10.0) ** 2)
            dd.line([(cx * 2, cy * 2), (ex * 2, ey * 2)], fill=(90, 255, 200, alpha), width=3)
    img.alpha_composite(sw_img.reduce(2))


# ---------------------------------------------------------------------------
# Snow
# ---------------------------------------------------------------------------
def snow(rng, f):
    n = rng.random((H, W), dtype=np.float32)
    # horizontal smear like real RF noise
    n = 0.55 * n + 0.3 * np.roll(n, 1, axis=1) + 0.15 * np.roll(n, 2, axis=1)
    n = (n - 0.5) * 1.9 + 0.5
    line = rng.normal(0, 0.06, (H, 1)).astype(np.float32)
    n = n + line
    # slow hum bar
    hum = 0.12 * np.sin(2 * np.pi * (YY / H * 1.0 - f * 0.07))
    n = n + hum
    n = np.clip(n, 0, 1)
    chroma = rng.normal(0, 0.05, (H, W // 4, 3)).astype(np.float32).repeat(4, axis=1)
    out = n[..., None] * np.array([0.9, 0.92, 1.0], np.float32) + chroma
    return np.clip(out, 0, 1)


def rolled_card(card, f):
    """Card with vertical-hold roll and horizontal jitter that settle by T_LOCK."""
    k = max(0.0, (T_LOCK - 4 - f) / (T_LOCK - 4))
    roll = int((k ** 2) * 1500) % (H + 24)
    out = np.zeros((H + 24, W, 3), np.float32)
    out[24:] = card
    out = np.roll(out, roll, axis=0)[24:]
    if k > 0:
        jit = (np.sin(YY[:, 0] * 0.09 + f * 1.7) * 9 * k + np.sin(YY[:, 0] * 0.31 + f) * 3 * k).astype(int)
        idx = (XX.astype(int) - jit[:, None]) % W
        out = np.take_along_axis(out, idx[..., None].repeat(3, 2), axis=1)
    return out


# ---------------------------------------------------------------------------
# Ident: background
# ---------------------------------------------------------------------------
HZ = 300  # horizon line
LOGO_C = (320, 212)
ANG = np.arctan2(YY - LOGO_C[1], XX - LOGO_C[0])
RAD = np.hypot(XX - LOGO_C[0], YY - LOGO_C[1])


def gradient_rows(stops, n):
    ys = np.linspace(0, 1, n)
    out = np.zeros((n, 3), np.float32)
    for c in range(3):
        out[:, c] = np.interp(ys, [s[0] for s in stops], [s[1][c] / 255.0 for s in stops])
    return out


SKY = gradient_rows([(0, (6, 3, 28)), (0.45, (30, 8, 84)), (0.8, (130, 24, 130)), (0.95, (240, 80, 120)),
                     (1.0, (255, 170, 110))], HZ)
FLOOR = gradient_rows([(0, (60, 6, 70)), (0.25, (24, 2, 44)), (1.0, (10, 0, 24))], H - HZ)
BG_BASE = np.zeros((H, W, 3), np.float32)
BG_BASE[:HZ] = SKY[:, None, :]
BG_BASE[HZ:] = FLOOR[:, None, :]

_srng = np.random.default_rng(77)
STARS = [(int(_srng.uniform(0, W)), int(_srng.uniform(0, HZ - 60)), _srng.uniform(0.3, 1.0),
          _srng.uniform(0, 6.28)) for _ in range(90)]


def ident_background(f):
    t = (f - T_IDENT) / 30.0
    bg = BG_BASE.copy()
    # sunburst behind the logo (sky only)
    rays = 0.5 + 0.5 * np.sin(14 * ANG + t * 0.9)
    rays = smoothstep(0.35, 0.65, rays)
    fall = np.exp(-RAD / 260.0)
    sky = (YY < HZ).astype(np.float32)
    intro = sstep(T_IDENT, T_IDENT + 25, f)
    burst = rays * fall * sky * 0.42 * intro
    bg += burst[..., None] * np.array([1.0, 0.45, 0.8], np.float32)
    # stars
    for (sx, sy, b, ph) in STARS:
        v = b * (0.55 + 0.45 * math.sin(t * 5 + ph))
        bg[sy, sx] += v * 0.9
        bg[sy, max(sx - 1, 0)] += v * 0.25
        bg[sy, min(sx + 1, W - 1)] += v * 0.25
    # perspective grid floor
    dy = np.maximum(YY[HZ:] - HZ, 0.5)
    xs = XX[HZ:] - 320
    u = xs / dy * 6.0
    fu = 6.0 / dy                    # du per pixel in x
    du = np.abs(u - np.round(u)) / fu
    lu = np.clip(1.4 - du, 0, 1)
    v = 900.0 / dy + t * 2.2
    fv = 900.0 / dy ** 2             # dv per pixel in y
    dv = np.abs(v - np.round(v)) / fv
    lv = np.clip(1.3 - dv, 0, 1) * np.clip(1.2 - fv * 1.6, 0, 1)
    lines = np.maximum(lu, lv) * np.clip(dy / 40.0, 0.25, 1)
    depth = np.clip(dy / (H - HZ), 0, 1)
    col = np.stack([0.95 - 0.8 * depth, 0.25 + 0.6 * depth, np.full_like(depth, 0.95)], -1)
    bg[HZ:] += lines[..., None] * col * intro
    # horizon glow line
    glow = np.exp(-np.abs(YY - HZ) / 3.0)[..., None] * np.array([1.0, 0.45, 0.6], np.float32) * 0.7
    bg += glow * intro
    return bg


# ---------------------------------------------------------------------------
# Ident: chrome logo sprite
# ---------------------------------------------------------------------------
def env_map(t):
    """Classic chrome: sky gradient, hard horizon, warm ground."""
    stops = [(0.0, (30, 40, 140)), (0.30, (90, 160, 250)), (0.49, (235, 245, 255)),
             (0.50, (70, 30, 30)), (0.62, (190, 90, 40)), (0.85, (255, 200, 120)), (1.0, (255, 245, 220))]
    out = np.zeros(t.shape + (3,), np.float32)
    for c in range(3):
        out[..., c] = np.interp(t, [s[0] for s in stops], [s[1][c] / 255.0 for s in stops])
    return out


def build_logo():
    LW, LH = 840, 560
    pad = 40
    f = font(F_BLACK_I, 470)
    face = text_mask("7½", f, (LW, LH), (LW // 2, LH // 2 + 10), anchor="mm")
    rim = dilate(face, 7)
    line = dilate(face, 3)
    ys = np.linspace(0, 1, LH, dtype=np.float32)[:, None] * np.ones((1, LW), np.float32)
    b = blur(face, 6)
    gy, gx = np.gradient(b)
    t = ys * 0.95 + 0.04 - gy * 7.0 - gx * 2.0
    chrome = env_map(np.clip(t, 0, 1))
    # bevel lighting: light from top-left
    light = np.clip(-(gx + gy) * 9.0, -1, 1)
    chrome = np.clip(chrome + light[..., None] * 0.35, 0, 1)
    # gold rim with vertical gradient
    gold = env_map(np.clip(ys * 0.3 + 0.62, 0, 1)) * np.array([1.0, 0.9, 0.55], np.float32)
    gold = np.clip(gold + 0.15, 0, 1)
    rgb = gold * rim[..., None]
    a = rim.copy()
    over(rgb, (0.06, 0.02, 0.12), line)
    over(rgb, chrome, face)
    a = np.maximum(a, line)
    # crop to bbox
    nz = np.argwhere(a > 0.01)
    y0, x0 = nz.min(0) - pad
    y1, x1 = nz.max(0) + pad
    y0, x0 = max(y0, 0), max(x0, 0)
    rgb, a, face = rgb[y0:y1, x0:x1], a[y0:y1, x0:x1], face[y0:y1, x0:x1]
    prem = rgb * a[..., None]
    mips = []
    pim = Image.fromarray((np.clip(prem, 0, 1) * 255).astype(np.uint8))
    aim = Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8))
    fim = Image.fromarray((np.clip(face, 0, 1) * 255).astype(np.uint8))
    while True:
        mips.append((pim, aim, fim))
        if pim.width < 60:
            break
        pim = pim.reduce(2)
        aim = aim.reduce(2)
        fim = fim.reduce(2)
    return mips


def homography(src, dst):
    """Coefficients mapping dst (output) points -> src (input) points, PIL PERSPECTIVE order."""
    A, bvec = [], []
    for (x, y), (u, v) in zip(dst, src):
        A.append([x, y, 1, 0, 0, 0, -x * u, -y * u])
        bvec.append(u)
        A.append([0, 0, 0, x, y, 1, -x * v, -y * v])
        bvec.append(v)
    return np.linalg.solve(np.array(A, float), np.array(bvec, float)).tolist()


def render_logo(mips, theta, phi, tx, ty, tz):
    """Project the sprite plane in 3D. Returns premult rgb, alpha, face alpha at native res."""
    SS = 2
    Fz = 1000.0 * SS
    base_w, base_h = mips[0][0].size
    ww = 3.0
    hh = ww * base_h / base_w
    corners = [(-ww / 2, -hh / 2), (ww / 2, -hh / 2), (ww / 2, hh / 2), (-ww / 2, hh / 2)]
    dst = []
    ct, st, cp, sp = math.cos(theta), math.sin(theta), math.cos(phi), math.sin(phi)
    for X, Y in corners:
        Y2, Z2 = Y * cp, Y * sp
        X3, Z3 = X * ct, X * st + Z2
        Z = tz + Z3
        dst.append((W * SS / 2 + Fz * (X3 + tx) / Z, H * SS / 2 + Fz * (Y2 + ty) / Z))
    screen_w = Fz * ww / tz
    lvl = 0
    while lvl + 1 < len(mips) and mips[lvl + 1][0].width >= screen_w * 1.1:
        lvl += 1
    pim, aim, fim = mips[lvl]
    sw, sh = pim.size
    src = [(0, 0), (sw, 0), (sw, sh), (0, sh)]
    co = homography(src, dst)
    size = (W * SS, H * SS)
    p = pim.transform(size, Image.PERSPECTIVE, co, Image.BICUBIC).reduce(SS)
    a = aim.transform(size, Image.PERSPECTIVE, co, Image.BICUBIC).reduce(SS)
    fa = fim.transform(size, Image.PERSPECTIVE, co, Image.BICUBIC).reduce(SS)
    return to_np(p), to_np(a), to_np(fa), [(x / SS, y / SS) for x, y in dst]


def star_glint(img, x, y, size, strength, col=(1.0, 0.95, 0.85)):
    """Additive 4-point star with soft core."""
    r = int(size * 1.2) + 2
    x0, x1 = max(int(x) - r, 0), min(int(x) + r + 1, W)
    y0, y1 = max(int(y) - r, 0), min(int(y) + r + 1, H)
    if x1 <= x0 or y1 <= y0:
        return
    dx = XX[y0:y1, x0:x1] - x
    dy = YY[y0:y1, x0:x1] - y
    ax, ay = np.abs(dx), np.abs(dy)
    spike = np.exp(-ay / 0.9) * np.clip(1 - ax / size, 0, 1) ** 2 + \
        np.exp(-ax / 0.9) * np.clip(1 - ay / size, 0, 1) ** 2
    d2 = (dx + dy) / 1.414
    e2 = (dx - dy) / 1.414
    diag = (np.exp(-np.abs(e2) / 0.8) * np.clip(1 - np.abs(d2) / (size * 0.45), 0, 1) ** 2 +
            np.exp(-np.abs(d2) / 0.8) * np.clip(1 - np.abs(e2) / (size * 0.45), 0, 1) ** 2) * 0.5
    core = np.exp(-(dx * dx + dy * dy) / (size * 0.18) ** 2)
    v = (spike + diag + core * 1.4) * strength
    img[y0:y1, x0:x1] += v[..., None] * np.array(col, np.float32)


def flare(img, lx, ly, strength):
    """Anamorphic streak + ghost discs along the axis through screen centre."""
    if strength <= 0.01:
        return
    streak = np.exp(-np.abs(YY - ly) / 1.6) * np.exp(-np.abs(XX - lx) / 220.0)
    img += (streak * strength * 0.9)[..., None] * np.array([0.55, 0.75, 1.0], np.float32)
    cx, cy = W / 2, H / 2
    ghosts = [(-0.5, 18, (0.3, 1.0, 0.6)), (-0.9, 30, (0.9, 0.4, 1.0)), (0.4, 10, (1.0, 0.7, 0.2)),
              (-1.4, 46, (0.3, 0.6, 1.0))]
    for k, rr, col in ghosts:
        gx = cx + (lx - cx) * k
        gy = cy + (ly - cy) * k
        d = np.hypot(XX - gx, YY - gy)
        disc = np.clip(1 - np.abs(d - rr * 0.7) / (rr * 0.45), 0, 1) * 0.5 + np.clip(1 - d / rr, 0, 1) * 0.25
        img += (disc * strength * 0.5)[..., None] * np.array(col, np.float32)


def styled_text(img, txt, fnt, cx, cy, top, bot, alpha=1.0, shadow=(5, 5), dx=0):
    """Gradient-filled text with black outline and chunky drop shadow."""
    m = text_mask(txt, fnt, (W, H), (cx + dx, cy), ss=2)
    o = text_mask(txt, fnt, (W, H), (cx + dx, cy), ss=2, stroke=3)
    sh = shift(o, *shadow)
    over(img, (0.08, 0.0, 0.12), sh * 0.85 * alpha)
    over(img, (0.0, 0.0, 0.0), o * alpha)
    tt = np.clip((YY - (cy - 18)) / 36.0, 0, 1)[..., None]
    grad = np.array(top, np.float32) * (1 - tt) + np.array(bot, np.float32) * tt
    a = m * alpha
    img *= (1 - a[..., None])
    img += grad * a[..., None]


def draw_ident(f, mips, fonts):
    img = ident_background(f)
    t_fly = (f - T_IDENT) / float(T_LAND - T_IDENT)
    e = ease_out(t_fly)
    theta = -3.0 * math.pi * (1 - e) + 0.12 * math.sin(max(0, f - T_LAND) * 0.08) * sstep(T_LAND, T_LAND + 20, f)
    phi = 0.35 * (1 - e) - 0.06
    tz = 4.0 + 46.0 * (1 - e) ** 2
    tx = -2.5 * (1 - e) ** 2
    ty = -0.11 - 1.5 * (1 - e) ** 2 + 0.03 * math.sin(max(0, f - T_LAND) * 0.11)
    prem, a, fa, quad = render_logo(mips, theta, phi, tx, ty, tz)
    shade = 0.45 + 0.55 * abs(math.cos(theta))
    nz = np.argwhere(a > 0.004)
    if len(nz):
        y0, x0 = np.maximum(nz.min(0) - 40, 0)
        y1, x1 = np.minimum(nz.max(0) + 40, [H, W])
        sub = img[y0:y1, x0:x1]
        aa = a[y0:y1, x0:x1]
        # chunky drop shadow
        sd = shift(blur(aa, 3), 16, 18)
        over(sub, (0.02, 0.0, 0.05), sd * 0.7)
        # extrusion: depth direction follows rotation
        depth = 18 * (tz and 4.0 / tz)
        ex, ey = 0.55 + 0.9 * math.sin(theta), 0.75
        n = 14
        for i in range(n, 0, -1):
            k = i / n
            sx, sy = int(round(ex * depth * k)), int(round(ey * depth * k))
            col = np.array([0.55, 0.06, 0.45]) * (1 - k) + np.array([0.12, 0.0, 0.22]) * k
            over(sub, col, shift(aa, sx, sy))
        sub *= (1 - aa[..., None])
        sub += prem[y0:y1, x0:x1] * shade
        img[y0:y1, x0:x1] = sub

    lcx = (quad[0][0] + quad[2][0]) / 2
    lcy = (quad[0][1] + quad[2][1]) / 2
    lw = quad[1][0] - quad[0][0]
    # landing flash
    if T_LAND - 2 <= f <= T_LAND + 10:
        k = 1 - abs(f - (T_LAND + 1)) / 9.0
        star_glint(img, lcx - lw * 0.18, lcy - 70, 120, 0.9 * k)
    # sweeping light streak across the face
    s0, s1 = T_LAND + 4, T_LAND + 26
    if s0 <= f <= s1 + 4:
        u = (f - s0) / float(s1 - s0)
        bx = quad[0][0] - 60 + u * (lw + 160)
        band = np.exp(-((XX - bx) + 0.45 * (YY - lcy)) ** 2 / (2 * 14.0 ** 2))
        band2 = np.exp(-((XX - bx + 34) + 0.45 * (YY - lcy)) ** 2 / (2 * 5.0 ** 2)) * 0.6
        img += ((band + band2) * fa * 0.95)[..., None] * np.array([1.0, 0.97, 0.9], np.float32)
        flare(img, bx, lcy - 10, 0.8 * math.sin(min(u, 1.0) * math.pi))
        if u <= 1:
            star_glint(img, bx - 0.45 * (quad[0][1] + 40 - lcy), quad[0][1] + 40, 46, 0.9)
    # twinkles after landing
    pts = [(-0.30, -0.42), (0.36, -0.40), (0.05, 0.38), (-0.12, -0.05), (0.40, 0.30)]
    if f > s1:
        for i, (px, py) in enumerate(pts):
            ph = (f - s1 - i * 9) % 46
            if ph < 14:
                k = math.sin(ph / 14.0 * math.pi)
                star_glint(img, lcx + px * lw, lcy + py * lw * 0.62, 26 + 10 * (i % 2), 0.85 * k)

    # "CHANNEL" slides in from the left
    if f >= T_LAND - 10:
        k = ease_out((f - (T_LAND - 10)) / 14.0)
        styled_text(img, "CHANNEL", fonts["chan"], 320, 60, (1.0, 0.95, 0.4), (1.0, 0.45, 0.1),
                    alpha=min(1.0, k * 1.5), dx=int(-420 * (1 - k)))
    # tagline banner wipes out from centre
    if f >= T_LAND + 8:
        k = ease_out((f - (T_LAND + 8)) / 12.0)
        half = 230 * k
        by0, by1 = 350, 380
        sl = 10
        band = np.zeros((H, W), np.float32)
        inb = (YY >= by0) & (YY < by1)
        xl = 320 - half + (YY - by0) * 0 - sl * (1 - (YY - by0) / (by1 - by0))
        xr = 320 + half + sl * ((YY - by0) / (by1 - by0))
        band = (np.clip(XX - xl, 0, 1) * np.clip(xr - XX, 0, 1) * inb).astype(np.float32)
        over(img, (0, 0, 0), shift(band, 6, 6) * 0.7)
        tt = np.clip((XX - 90) / 460.0, 0, 1)[..., None]
        grad = np.array([0.95, 0.15, 0.55], np.float32) * (1 - tt) + np.array([1.0, 0.55, 0.1], np.float32) * tt
        img *= (1 - band[..., None])
        img += grad * band[..., None]
        if k > 0.6:
            m = text_mask("HALCYON BAY  •  ALL NIGHT", fonts["tag"], (W, H), (320, 365), ss=2)
            m *= band
            over(img, (0.1, 0.0, 0.15), shift(m, 2, 2))
            over(img, (1.0, 0.98, 0.92), m)
    return img


# ---------------------------------------------------------------------------
# TV on-screen display
# ---------------------------------------------------------------------------
OSD_GREEN = np.array([0.35, 1.0, 0.35], np.float32)


def osd_layer(img, m):
    """Green glyphs with black edging and a drop shadow, like a TV's character generator."""
    edge = dilate(m, 2)
    over(img, (0, 0, 0), shift(edge, 3, 3) * 0.6)
    over(img, (0, 0, 0), edge)
    over(img, OSD_GREEN, m)


def draw_osd(img, f):
    if f < T_OSD:
        return
    m = bitmap_mask("CH 07", 640 - 26 - (5 * 6 * 7 - 7), 24, 7)
    if f >= T_OSD + 4:
        bitmap_mask("STEREO", 640 - 26 - (6 * 6 * 2 - 2), 24 + 7 * 7 + 8, 2, m)
    osd_layer(img, m)
    if f >= T_VOL:
        k = ease_out((f - T_VOL) / 9.0)
        yb = int(416 + 90 * (1 - k))
        level = int(round(np.interp(f, [T_VOL + 8, T_VOL + 38], [8, 30])))
        mv = np.zeros((H, W), np.float32)
        bitmap_mask("VOLUME", 36, yb, 3, mv)
        nseg = 34
        bx = 152
        for i in range(nseg):
            x = bx + i * 11
            if i < level:
                hgt = 21
                if 0 <= yb and yb + hgt <= H:
                    mv[yb:yb + hgt, x:x + 8] = 1.0
            else:
                if 0 <= yb and yb + 21 <= H:
                    mv[yb + 9:yb + 12, x:x + 8] = 1.0
        bitmap_mask("%2d" % level, bx + nseg * 11 + 6, yb, 3, mv)
        osd_layer(img, mv)


# ---------------------------------------------------------------------------
def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else "frames"
    os.makedirs(out_dir, exist_ok=True)
    rng = np.random.default_rng(1977)
    card = build_test_card()
    mips = build_logo()
    fonts = {"chan": font(F_XB_I, 44), "tag": font(F_BOLD_I, 20)}
    card_img = Image.fromarray((card * 255).astype(np.uint8)).convert("RGBA")

    for f in range(N_FRAMES):
        if f < T_CUT:
            ci = card_img.copy()
            draw_test_overlay(ci, f)
            live = to_np(ci.convert("RGB"))
            if f < T_LOCK:
                pic = rolled_card(live, f)
                p = 0.22 + 0.78 * sstep(T_SNOW_END - 6, T_LOCK - 4, f)
                nz = 1.0 - sstep(T_SNOW_END, T_LOCK, f)
                sn = snow(rng, f)
                img = pic * p + sn * nz * (1 - p * 0.55)
                img = np.clip(img, 0, 1)
            else:
                img = live
        elif f < T_IDENT:
            # the cut: torn sync then a dark frame with a hint of noise
            if f == T_CUT:
                ci = card_img.copy()
                draw_test_overlay(ci, f)
                live = to_np(ci.convert("RGB"))
                tear = (np.sin(YY[:, 0] * 0.05) * 60 + np.sin(YY[:, 0] * 0.21) * 25).astype(int)
                idx = (XX.astype(int) - tear[:, None]) % W
                img = np.take_along_axis(live, idx[..., None].repeat(3, 2), axis=1) * 1.15
                img = np.clip(img + snow(rng, f) * 0.25, 0, 1)
            else:
                img = draw_ident(T_IDENT, mips, fonts) * 0.35 + snow(rng, f) * 0.18
        else:
            img = draw_ident(f, mips, fonts)
            draw_osd(img, f)
        img = np.clip(img, 0, 1)
        Image.fromarray((img * 255 + 0.5).astype(np.uint8), "RGB").save(os.path.join(out_dir, "%05d.png" % f))


if __name__ == "__main__":
    main()
