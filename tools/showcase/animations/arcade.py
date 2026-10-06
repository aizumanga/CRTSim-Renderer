#!/usr/bin/env python3
"""Original 1980s arcade space-shooter "attract mode" at 256x224.

Parallax starfield over a dithered nebula, a player ship that auto-targets
swooping enemy formations, pixel explosions, a HUD with a rolling score that
overtakes the HI score, a STAGE CLEAR chain reaction, a hyperspace warp and a
made-up title ("QUASAR KESTREL") with a flashing INSERT COIN.

    python3 arcade.py OUT_DIR      -> OUT_DIR/00000.png .. 00239.png
numpy + Pillow only, deterministic.
"""
import math
import os
import sys

import numpy as np
from PIL import Image

W, H = 256, 224
NFRAMES = 240
T0 = -84  # simulation warm-up so frame 0 is already busy

# ---------------------------------------------------------------- palette
BG = (3, 2, 10)
WHITE = (244, 244, 255)
LGREY = (176, 188, 232)
YELLOW = (255, 226, 32)
ORANGE = (255, 132, 16)
RED = (240, 30, 44)
DRED = (130, 8, 30)
MAGENTA = (236, 48, 226)
PINK = (255, 96, 180)
CYAN = (40, 226, 255)
BLUE = (48, 76, 255)
DBLUE = (22, 26, 120)
PURPLE = (120, 30, 200)

BAYER4 = np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]],
                  float) / 16.0 + 1 / 32.0

# ---------------------------------------------------------------- fonts
FONT5x7 = {
    'A': ".###.#...##...#######...##...##...#",
    'B': "####.#...##...#####.#...##...#####.",
    'C': ".###.#...##....#....#....#...#.###.",
    'D': "####.#...##...##...##...##...#####.",
    'E': "######....#....####.#....#....#####",
    'F': "######....#....####.#....#....#....",
    'G': ".###.#...##....#.####...##...#.####",
    'H': "#...##...##...#######...##...##...#",
    'I': ".###...#....#....#....#....#...###.",
    'J': "..###...#....#....#.#..#.#..#..##..",
    'K': "#...##..#.#.#..##...#.#..#..#.#...#",
    'L': "#....#....#....#....#....#....#####",
    'M': "#...###.###.#.##.#.##...##...##...#",
    'N': "#...##...###..##.#.##..###...##...#",
    'O': ".###.#...##...##...##...##...#.###.",
    'P': "####.#...##...#####.#....#....#....",
    'Q': ".###.#...##...##...##.#.##..#..##.#",
    'R': "####.#...##...#####.#.#..#..#.#...#",
    'S': ".#####....#.....###.....#....#####.",
    'T': "#####..#....#....#....#....#....#..",
    'U': "#...##...##...##...##...##...#.###.",
    'V': "#...##...##...##...##...#.#.#...#..",
    'W': "#...##...##...##.#.##.#.###.###...#",
    'X': "#...##...#.#.#...#...#.#.#...##...#",
    'Y': "#...##...#.#.#...#....#....#....#..",
    'Z': "#####....#...#...#...#...#....#####",
    '0': ".###.#...##..###.#.###..##...#.###.",
    '1': "..#...##....#....#....#....#...###.",
    '2': ".###.#...#....#..##..#...#....#####",
    '3': "#####...#...#.....#.....##...#.###.",
    '4': "...#...##..#.#.#..#.#####...#....#.",
    '5': "######....####.....#....##...#.###.",
    '6': "..##..#...#....####.#...##...#.###.",
    '7': "#####....#...#...#...#....#....#...",
    '8': ".###.#...##...#.###.#...##...#.###.",
    '9': ".###.#...##...#.####....#...#..##..",
    '-': "...............###...............",
    ' ': "." * 35,
    '.': "." * 30 + "..#..",
    '!': "..#....#....#....#....#.........#..",
    ':': ".......#.........................",
}
FONT5x7['-'] = "." * 15 + "#####" + "." * 15
FONT5x7[':'] = "." * 5 + "..#.." + "." * 10 + "..#.." + "." * 10
GLYPH = {k: np.array([c == '#' for c in v], bool).reshape(7, 5) for k, v in FONT5x7.items()}

FONT3x5 = {
    '0': "####.##.##.####", '1': ".#.##..#..#.###", '2': "###..#####..###",
    '3': "###..#.##..####", '4': "#.##.####..#..#", '5': "####..###..####",
    '6': "####..####.####", '7': "###..#..#.#..#.", '8': "####.#####.####",
    '9': "####.####..####",
}
GLYPH3 = {k: np.array([c == '#' for c in v], bool).reshape(5, 3) for k, v in FONT3x5.items()}


def text_mask(s, scale=1, spacing=1, font=None):
    font = font or GLYPH
    gh, gw = next(iter(font.values())).shape
    m = np.zeros((gh * scale, max(1, len(s) * (gw + spacing) * scale - spacing * scale)), bool)
    for i, ch in enumerate(s):
        g = font.get(ch.upper(), font.get(' '))
        if g is None:
            continue
        g = np.kron(g, np.ones((scale, scale), bool))
        x = i * (gw + spacing) * scale
        m[:, x:x + gw * scale] |= g
    return m


def blit_mask(img, m, x, y, color):
    """Paint boolean mask m at (x,y) with a colour (tuple) or an HxWx3 array."""
    x, y = int(round(x)), int(round(y))
    h, w = m.shape
    x0, y0 = max(0, x), max(0, y)
    x1, y1 = min(W, x + w), min(H, y + h)
    if x0 >= x1 or y0 >= y1:
        return
    sub = m[y0 - y:y1 - y, x0 - x:x1 - x]
    tgt = img[y0:y1, x0:x1]
    if isinstance(color, np.ndarray):
        tgt[sub] = color[y0 - y:y1 - y, x0 - x:x1 - x][sub]
    else:
        tgt[sub] = color


def draw_text(img, s, x, y, color, scale=1, font=None):
    m = text_mask(s, scale, font=font)
    blit_mask(img, m, x, y, color)
    return m.shape[1]


def text_w(s, scale=1):
    return len(s) * 6 * scale - scale


# ---------------------------------------------------------------- sprites
CMAP = {'W': WHITE, 'G': LGREY, 'Y': YELLOW, 'O': ORANGE, 'R': RED, 'D': DRED,
        'M': MAGENTA, 'P': PINK, 'C': CYAN, 'B': BLUE, 'K': DBLUE, 'U': PURPLE}


def sprite(rows):
    h, w = len(rows), len(rows[0])
    rgb = np.zeros((h, w, 3), np.uint8)
    m = np.zeros((h, w), bool)
    for j, r in enumerate(rows):
        for i, c in enumerate(r):
            if c in CMAP:
                rgb[j, i] = CMAP[c]
                m[j, i] = True
    return rgb, m


PLAYER = sprite([
    "......W......",
    "......W......",
    ".....GCG.....",
    ".....WCW.....",
    "....GCBCG....",
    "....WCBCW....",
    "..R.GCCCG.R..",
    "..R.WGWGW.R..",
    ".RRGWMWMWGRR.",
    "RRGWWGWGWWGRR",
    "RG.GW...WG.GR",
    "R...Y...Y...R",
])

# type A "vexwing": magenta pincers, cyan core
ENEMY_A = [sprite([
    "M.........M",
    "MM...C...MM",
    ".MM.CWC.MM.",
    "..MMCCCMM..",
    "...MRRRM...",
    "..MM.Y.MM..",
    ".MM.....MM.",
]), sprite([
    "...........",
    ".M...C...M.",
    ".MM.CWC.MM.",
    "MMMMCCCMMMM",
    "...MRRRM...",
    "...M.Y.M...",
    "..M.....M..",
])]
# type B "glowdisc": yellow saucer with chasing lights
ENEMY_B = [sprite([
    "...YYYYY...",
    ".YYOOWOOYY.",
    "YYOOOOOOOYY",
    "RRRRRRRRRRR",
    ".W.R.W.R.W.",
    "...RRRRR...",
]), sprite([
    "...YYYYY...",
    ".YYOWOOOYY.",
    "YYOOOOOOOYY",
    "RRRRRRRRRRR",
    ".R.W.R.W.R.",
    "...RRRRR...",
])]
# type C "lancet": cyan arrowhead diving down
ENEMY_C = [sprite([
    "C.........C",
    "CC.......CC",
    ".CCBBBBBCC.",
    "..CBWWWBC..",
    "...CBMBC...",
    "....CBC....",
    ".....P.....",
]), sprite([
    "C.........C",
    ".C.......C.",
    ".CCBBBBBCC.",
    "..CBWPWBC..",
    "...CBMBC...",
    "....CBC....",
    ".....Y.....",
])]
ENEMY = {'A': ENEMY_A, 'B': ENEMY_B, 'C': ENEMY_C}
POINTS = {'A': 150, 'B': 200, 'C': 300}
DEBRIS_COL = {'A': [MAGENTA, CYAN, PINK], 'B': [YELLOW, ORANGE, RED], 'C': [CYAN, BLUE, PINK]}

LIFE_ICON = sprite([
    "...W...",
    "..WCW..",
    ".RWBWR.",
    "RRWWWRR",
    "R..Y..R",
])
FLAG_ICON = sprite([
    "M....",
    "MMM..",
    "MMMMM",
    "M....",
    "M....",
    "M....",
])
FLAG_ICON2 = sprite([
    "C....",
    "CYC..",
    "CYYYC",
    "C....",
    "C....",
    "C....",
])


def paint_sprite(img, spr, x, y):
    """Draw sprite with top-left at (x, y)."""
    rgb, m = spr
    x, y = int(round(x)), int(round(y))
    h, w = m.shape
    x0, y0 = max(0, x), max(0, y)
    x1, y1 = min(W, x + w), min(H, y + h)
    if x0 >= x1 or y0 >= y1:
        return
    sm = m[y0 - y:y1 - y, x0 - x:x1 - x]
    sr = rgb[y0 - y:y1 - y, x0 - x:x1 - x]
    img[y0:y1, x0:x1][sm] = sr[sm]


def put(img, x, y, c):
    x, y = int(round(x)), int(round(y))
    if 0 <= x < W and 0 <= y < H:
        img[y, x] = c


def add_px(img, x, y, c, a=1.0):
    x, y = int(round(x)), int(round(y))
    if 0 <= x < W and 0 <= y < H:
        v = img[y, x].astype(np.int32) + (np.array(c) * a).astype(np.int32)
        img[y, x] = np.clip(v, 0, 255)


# ---------------------------------------------------------------- paths
def make_path(pts, n=48):
    p = np.array(pts, float)
    P = np.vstack([2 * p[0] - p[1], p, 2 * p[-1] - p[-2]])
    out = []
    for i in range(1, len(P) - 2):
        p0, p1, p2, p3 = P[i - 1:i + 3]
        for u in np.linspace(0, 1, n, endpoint=False):
            out.append(0.5 * (2 * p1 + (-p0 + p2) * u + (2 * p0 - 5 * p1 + 4 * p2 - p3) * u * u
                              + (-p0 + 3 * p1 - 3 * p2 + p3) * u ** 3))
    out.append(p[-1])
    out = np.array(out)
    d = np.r_[0, np.cumsum(np.hypot(*np.diff(out, axis=0).T))]
    return out, d


def mirror(pts):
    return [(W - x, y) for x, y in pts]


P_LOOP = [(-16, 36), (50, 56), (110, 104), (160, 128), (196, 104), (186, 62), (140, 46),
          (92, 66), (60, 108), (40, 150), (10, 190), (-24, 214)]
P_DIVE = [(118, -14), (124, 40), (96, 84), (60, 78), (62, 44), (110, 46), (168, 92),
          (214, 132), (276, 150)]
P_SNAKE = [(36, -14), (70, 34), (150, 52), (214, 76), (206, 112), (120, 112), (52, 126),
           (42, 160), (90, 196), (150, 240)]
P_ARC = [(-16, 120), (40, 84), (100, 64), (156, 64), (210, 84), (240, 120), (272, 150)]
PATHS = {
    'loopL': make_path(P_LOOP), 'loopR': make_path(mirror(P_LOOP)),
    'diveL': make_path(P_DIVE), 'diveR': make_path(mirror(P_DIVE)),
    'snakeL': make_path(P_SNAKE), 'snakeR': make_path(mirror(P_SNAKE)),
    'arcL': make_path(P_ARC), 'arcR': make_path(mirror(P_ARC)),
}
# (spawn frame, path, type, count, spacing)
WAVES = [
    (-84, 'loopL', 'A', 6, 9),
    (-66, 'loopR', 'B', 6, 9),
    (-8, 'diveL', 'C', 5, 8),
    (6, 'diveR', 'C', 5, 8),
    (40, 'snakeL', 'A', 6, 8),
    (56, 'snakeR', 'B', 6, 8),
    (88, 'arcL', 'C', 5, 8),
    (98, 'loopR', 'A', 6, 8),
    (112, 'loopL', 'B', 6, 8),
    (132, 'diveR', 'A', 5, 8),
]
ESPEED = 2.35


def path_pos(name, s):
    pts, d = PATHS[name]
    L = s * ESPEED
    if L < 0 or L > d[-1]:
        return None
    x = np.interp(L, d, pts[:, 0])
    y = np.interp(L, d, pts[:, 1])
    return x, y


# ---------------------------------------------------------------- background
def build_nebula(rng):
    noise = rng.standard_normal((H, W))
    F = np.fft.fft2(noise)
    fy = np.fft.fftfreq(H)[:, None]
    fx = np.fft.fftfreq(W)[None, :]
    f = np.sqrt(fx ** 2 + fy ** 2)
    F *= np.exp(-(f / 0.018) ** 2) + 0.25 * np.exp(-(f / 0.06) ** 2)
    n = np.real(np.fft.ifft2(F))
    n = (n - n.mean()) / n.std()
    # second, independent field for hue variation
    F2 = np.fft.fft2(rng.standard_normal((H, W))) * np.exp(-(f / 0.02) ** 2)
    h = np.real(np.fft.ifft2(F2))
    h = (h - h.mean()) / h.std()
    v = np.clip((n - 0.35) / 1.9, 0, 1)  # cloud density 0..1
    # quantise to 3 levels with ordered dither (tileable since 224,256 % 4 == 0)
    th = np.tile(BAYER4, (H // 4, W // 4))
    lev = np.floor(v * 3 + th).clip(0, 3).astype(int)
    pal_a = np.array([BG, (14, 6, 38), (30, 10, 64), (52, 14, 86)], np.uint8)  # violet
    pal_b = np.array([BG, (6, 12, 40), (10, 24, 70), (16, 38, 96)], np.uint8)  # blue
    neb = np.where((h > 0)[..., None], pal_a[lev], pal_b[lev])
    return neb


def build_stars(rng):
    layers = []
    spec = [(90, 1, [(60, 60, 120), (80, 70, 150), (50, 80, 140)]),
            (46, 2, [(120, 140, 220), (170, 120, 210), (110, 190, 220)]),
            (20, 4, [WHITE, CYAN, YELLOW, PINK])]
    for n, mult, cols in spec:
        xs = rng.integers(0, W, n)
        ys = rng.uniform(0, H, n)
        cs = [cols[i] for i in rng.integers(0, len(cols), n)]
        ks = rng.integers(1, 7, n)          # twinkle cycles per loop (seamless)
        ph = rng.uniform(0, 2 * np.pi, n)
        layers.append((xs, ys, cs, ks, ph, mult))
    return layers


WARP_A, WARP_B = 176, 214  # warp window (extra full screen scroll)


def warp_disp(t):
    """Extra displacement (in screens) of the warp, 0 before, 1 after."""
    if t <= WARP_A:
        return 0.0
    if t >= WARP_B:
        return 1.0
    u = (t - WARP_A) / (WARP_B - WARP_A)
    return u - math.sin(2 * math.pi * u) / (2 * math.pi)


def warp_vel(t):
    if t <= WARP_A or t >= WARP_B:
        return 0.0
    u = (t - WARP_A) / (WARP_B - WARP_A)
    return (1 - math.cos(2 * math.pi * u)) / (WARP_B - WARP_A)  # screens / frame


def scroll(mult, t):
    # base speed mult*H per 240 frames, plus mult extra screens during warp -> loops
    return mult * H * (t / NFRAMES + warp_disp(t))


def draw_background(img, t, neb, stars):
    off = int(round(scroll(1, t))) % H
    img[:] = np.roll(neb, off, axis=0)
    for xs, ys, cs, ks, ph, mult in stars:
        y = (ys + scroll(mult, t)) % H
        vel = mult * H * (1.0 / NFRAMES + warp_vel(t))
        streak = int(min(46, max(1, vel * 0.9)))
        tw = 0.65 + 0.35 * np.sin(2 * np.pi * ks * t / NFRAMES + ph)
        for i in range(len(xs)):
            c = np.array(cs[i], float) * tw[i]
            if mult == 4 and streak < 3:
                streak_i = 2
            else:
                streak_i = streak
            for k in range(streak_i):
                a = 1.0 if k == 0 else max(0.15, 1 - k / streak_i)
                yy = int(y[i] - k) % H
                img[yy, xs[i]] = np.maximum(img[yy, xs[i]], (c * a).astype(np.uint8))


# ---------------------------------------------------------------- simulation
class Particle:
    __slots__ = ('x', 'y', 'vx', 'vy', 'age', 'life', 'kind', 'col')

    def __init__(self, x, y, vx, vy, life, kind, col=None):
        self.x, self.y, self.vx, self.vy = x, y, vx, vy
        self.age, self.life, self.kind, self.col = 0, life, kind, col


def fire_color(f):
    if f < 0.08:
        return WHITE
    if f < 0.25:
        return YELLOW
    if f < 0.45:
        return ORANGE
    if f < 0.72:
        return RED
    return DRED


class Game:
    def __init__(self):
        self.rng = np.random.default_rng(1984)
        self.enemies = []
        for wi, (t0, path, typ, cnt, sp) in enumerate(WAVES):
            for k in range(cnt):
                self.enemies.append(dict(spawn=t0 + k * sp, path=path, typ=typ,
                                         alive=True, idx=k, x=-99, y=-99))
        self.px, self.py = 128.0, 192.0
        self.pvx = 0.0
        self.bolts = []
        self.eshots = []
        self.parts = []
        self.rings = []
        self.popups = []
        self.score = 21350
        self.shown = self.score
        self.cool = 0
        self.target = None
        self.target_t = 0
        self.chain = []
        self.player_on = True
        self.trail = []

    # -- explosions
    def explode(self, x, y, typ, big=False):
        r = self.rng
        n = 34 if big else 22
        for _ in range(n):
            a = r.uniform(0, 2 * np.pi)
            s = r.uniform(0.4, 2.6 if not big else 3.4)
            self.parts.append(Particle(x, y, math.cos(a) * s, math.sin(a) * s,
                                       int(r.integers(14, 30)), 'fire'))
        for _ in range(7 if big else 5):
            a = r.uniform(0, 2 * np.pi)
            s = r.uniform(3.0, 5.2)
            self.parts.append(Particle(x, y, math.cos(a) * s, math.sin(a) * s,
                                       int(r.integers(8, 15)), 'spark',
                                       [WHITE, CYAN, YELLOW][int(r.integers(0, 3))]))
        cols = DEBRIS_COL[typ]
        for _ in range(6):
            a = r.uniform(0, 2 * np.pi)
            s = r.uniform(0.8, 2.0)
            self.parts.append(Particle(x, y, math.cos(a) * s, math.sin(a) * s - 0.3,
                                       int(r.integers(20, 34)), 'chunk',
                                       cols[int(r.integers(0, len(cols)))]))
        self.rings.append([x, y, 0, big])

    def kill(self, e, t, chain=False):
        e['alive'] = False
        pts = POINTS[e['typ']] * (2 if chain else 1)
        self.score += pts
        self.explode(e['x'] + 5, e['y'] + 3, e['typ'], big=chain)
        self.popups.append([e['x'] + 5, e['y'] - 2, 0, str(pts)])

    def on_screen(self, e):
        return e['alive'] and -4 < e['x'] < W - 7 and 20 < e['y'] < 176

    def step(self, t):
        r = self.rng
        # enemies
        for e in self.enemies:
            if not e['alive']:
                continue
            p = path_pos(e['path'], t - e['spawn'])
            if p is None:
                if t - e['spawn'] > 0:
                    e['alive'] = False
                e['x'], e['y'] = -99, -99
                continue
            e['x'], e['y'] = p[0] - 5, p[1] - 3
        visible = [e for e in self.enemies if self.on_screen(e)]

        # stage-clear chain reaction
        if t == 158:
            rest = [e for e in self.enemies if e['alive']]
            vis = sorted([e for e in rest if self.on_screen(e)],
                         key=lambda e: -(e['y']))
            for e in rest:
                if e not in vis:
                    e['alive'] = False
            self.chain = vis
        if 158 <= t and self.chain and (t - 158) % 3 == 0:
            e = self.chain.pop(0)
            if e['alive']:
                self.kill(e, t, chain=True)

        if t == 170:
            self.score += 5000
        # player AI
        if self.player_on:
            if t < 158:
                if self.target is not None and (not self.on_screen(self.target)
                                                or t - self.target_t > 40):
                    self.target = None
                if self.target is None and visible:
                    best, bc = None, 1e9
                    for e in visible:
                        c = abs(e['x'] + 5 - self.px) * 0.8 + (180 - e['y']) * 0.35
                        if c < bc:
                            best, bc = e, c
                    self.target, self.target_t = best, t
                if self.target is not None:
                    e = self.target
                    lead = (self.py - e['y']) / 7.0
                    p = path_pos(e['path'], t + lead - e['spawn'])
                    gx = (p[0] if p is not None else e['x'] + 5)
                else:
                    gx = 128 + 70 * math.sin(t * 0.045)
                gx = min(232, max(24, gx))
                d = gx - self.px
                self.pvx += np.clip(d * 0.18 - self.pvx * 0.35, -0.9, 0.9)
                self.pvx = float(np.clip(self.pvx, -3.0, 3.0))
                self.px += self.pvx
                if self.cool > 0:
                    self.cool -= 1
                elif self.target is not None and abs(d) < 9:
                    self.bolts.append([self.px - 4, self.py - 2])
                    self.bolts.append([self.px + 4, self.py - 2])
                    self.cool = 6
            elif t < 186:
                # settle to centre for the warp
                d = 128 - self.px
                self.pvx += np.clip(d * 0.15 - self.pvx * 0.4, -0.8, 0.8)
                self.px += self.pvx
                if self.cool > 0:
                    self.cool -= 1
                elif t < 172:
                    self.bolts.append([self.px - 4, self.py - 2])
                    self.bolts.append([self.px + 4, self.py - 2])
                    self.cool = 5
            else:
                # warp out
                self.trail.append((self.px, self.py))
                self.py -= 0.6 + 0.55 * (t - 186) ** 1.35
                if self.py < -40:
                    self.player_on = False
        if self.trail and not self.player_on:
            self.trail.pop(0)
        self.trail = self.trail[-10:]

        # bolts
        nb = []
        for b in self.bolts:
            y_old = b[1]
            b[1] -= 7
            hit = None
            for e in visible:
                if not e['alive']:
                    continue
                if e['x'] - 1 <= b[0] <= e['x'] + 11 and b[1] - 1 <= e['y'] + 7 and y_old >= e['y']:
                    hit = e
                    break
            if hit is not None and t < 158:
                self.kill(hit, t)
                visible = [e for e in visible if e['alive']]
                continue
            if b[1] > -8:
                nb.append(b)
        self.bolts = nb

        # enemy shots
        if t < 150:
            for e in visible:
                if e['alive'] and 30 < e['y'] < 130 and r.random() < 0.013:
                    dx = self.px - (e['x'] + 5)
                    dy = self.py - e['y']
                    n = math.hypot(dx, dy)
                    self.eshots.append([e['x'] + 5, e['y'] + 7, dx / n * 2.1, dy / n * 2.1])
        ns = []
        for s in self.eshots:
            s[0] += s[2]
            s[1] += s[3]
            if -4 < s[0] < W + 4 and s[1] < H + 4 and not (t > 158 and r.random() < 0.15):
                ns.append(s)
            elif t > 158 and s[1] < H:
                # shots fizzle during stage clear
                self.parts.append(Particle(s[0], s[1], 0, 0, 6, 'spark', PINK))
        self.eshots = ns

        # particles
        np_ = []
        for p in self.parts:
            p.age += 1
            p.x += p.vx
            p.y += p.vy
            drag = 0.9 if p.kind == 'fire' else (0.84 if p.kind == 'spark' else 0.95)
            p.vx *= drag
            p.vy = p.vy * drag + (0.04 if p.kind == 'chunk' else 0.015)
            if p.age < p.life:
                np_.append(p)
        self.parts = np_
        for rg in self.rings:
            rg[2] += 1
        self.rings = [rg for rg in self.rings if rg[2] < (11 if rg[3] else 8)]
        for pp in self.popups:
            pp[2] += 1
            pp[1] -= 0.35
        self.popups = [pp for pp in self.popups if pp[2] < 34]

        # rolling score display
        if self.shown < self.score:
            self.shown = min(self.score, self.shown + 30)


# ---------------------------------------------------------------- drawing
def draw_game(img, g, t):
    # enemy shots
    for s in g.eshots:
        c = WHITE if (t // 2) % 2 else PINK
        put(img, s[0], s[1], c)
        put(img, s[0] - 1, s[1], MAGENTA)
        put(img, s[0] + 1, s[1], MAGENTA)
        put(img, s[0], s[1] - 1, MAGENTA)
        put(img, s[0], s[1] + 1, MAGENTA)
    # enemies
    for e in g.enemies:
        if e['alive'] and e['x'] > -20:
            fr = ((t + e['idx'] * 3) // 6) % 2
            paint_sprite(img, ENEMY[e['typ']][fr], e['x'], e['y'])
    # bolts
    for b in g.bolts:
        x, y = int(round(b[0])), int(round(b[1]))
        for k, c in enumerate([WHITE, CYAN, CYAN, CYAN, BLUE, DBLUE]):
            put(img, x, y + k, c)
    # rings (dithered shockwave)
    for x, y, age, big in g.rings:
        rad = 2 + age * (3.2 if big else 2.4)
        col = [WHITE, YELLOW, YELLOW, ORANGE, ORANGE, RED, RED, DRED, DRED, DRED, DRED][age]
        n = int(2 * np.pi * rad / 2) + 4
        for k in range(n):
            if (k + age) % 2:
                continue
            a = 2 * np.pi * k / n
            put(img, x + rad * math.cos(a), y + rad * math.sin(a) * 0.9, col)
        if age < 3:  # core flash: 4-point star
            ln = 6 - age * 2 + (3 if big else 0)
            for k in range(-ln, ln + 1):
                cc = WHITE if abs(k) < 2 else YELLOW
                put(img, x + k, y, cc)
                put(img, x, y + k, cc)
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    put(img, x + dx, y + dy, WHITE)
    # particles
    for p in g.parts:
        f = p.age / p.life
        if p.kind == 'fire':
            put(img, p.x, p.y, fire_color(f))
        elif p.kind == 'spark':
            c = p.col if f < 0.6 else RED
            put(img, p.x, p.y, c)
            put(img, p.x - p.vx * 0.7, p.y - p.vy * 0.7, tuple(int(v * 0.6) for v in c))
        else:
            c = p.col if f < 0.7 else DRED
            if (p.age // 3) % 2 == 0:
                put(img, p.x, p.y, c)
                put(img, p.x + 1, p.y, c)
            else:
                put(img, p.x, p.y, c)
                put(img, p.x, p.y + 1, c)
    # player
    if g.player_on:
        warping = t >= 186
        for i, (tx, ty) in enumerate(g.trail[-8:]):
            a = (i + 1) / 9
            for k in range(-1, 2):
                for yy in range(int(ty), int(ty) + 12):
                    add_px(img, tx + k, yy, CYAN if k == 0 else BLUE, a * 0.5)
        x, y = int(round(g.px)) - 6, int(round(g.py)) - 6
        paint_sprite(img, PLAYER, x, y)
        fl = 1 + (t * 7 + 3) % 3 if not warping else 4 + (t % 3) * 2
        for ex in (x + 4, x + 8):
            for k in range(fl):
                c = [YELLOW, YELLOW, ORANGE, ORANGE, RED, RED, DRED, DRED, DRED][min(k, 8)]
                put(img, ex, y + 12 + k, c)
    # popups
    for x, y, age, s in g.popups:
        if age < 4:
            continue
        c = [CYAN, YELLOW, MAGENTA, WHITE][(age // 3) % 4]
        m = text_mask(s, 1, font=GLYPH3)
        blit_mask(img, m, x - m.shape[1] // 2, y, c)


def draw_hud(img, g, t, hi, title_mode):
    if title_mode or (t // 16) % 3 != 2:
        draw_text(img, "1UP", 22, 3, RED)
    draw_text(img, "%06d" % g.shown, 16, 12, WHITE)
    lbl = "HI SCORE"
    draw_text(img, lbl, (W - text_w(lbl)) // 2, 3, RED)
    hv = max(hi, g.shown)
    hc = YELLOW if g.shown >= hi and (t // 4) % 2 == 0 else WHITE
    draw_text(img, "%06d" % hv, (W - text_w("000000")) // 2, 12, hc)
    draw_text(img, "2UP", W - 22 - text_w("2UP"), 3, BLUE)
    draw_text(img, "000000", W - 16 - text_w("000000"), 12, (70, 80, 150))
    if not title_mode:
        # lives and stage flags along the bottom
        paint_sprite(img, LIFE_ICON, 6, 214)
        paint_sprite(img, LIFE_ICON, 15, 214)
        for i in range(6):
            paint_sprite(img, FLAG_ICON if i % 2 == 0 else FLAG_ICON2, W - 10 - i * 7, 213)
    else:
        s = "CREDIT 00"
        draw_text(img, s, W - 8 - text_w(s), 214, CYAN)
        paint_sprite(img, LIFE_ICON, 6, 214)


# ---------------------------------------------------------------- title
TITLE_LINES = [("QUASAR", 46), ("KESTREL", 90)]
TS = 5  # title pixel scale


def build_title_letter(ch):
    """Return (rgb, letter mask, outline mask, shadow mask) for one big letter."""
    g = np.kron(GLYPH[ch], np.ones((TS, TS), bool))
    h, w = g.shape
    # 80s logo cut-lines in the lower half
    for r in (19, 23, 26, 29, 31, 33):
        g[r, :] = False
    pad = 4
    m = np.zeros((h + pad * 2, w + pad * 2), bool)
    m[pad:pad + h, pad:pad + w] = g
    # outline: dilate by 1 (8-connected)
    o = m.copy()
    for dy in (-1, 0, 1):
        for dx in (-1, 0, 1):
            o |= np.roll(np.roll(m, dy, 0), dx, 1)
    sh = np.zeros_like(m)
    sh[3:, 3:] = o[:-3, :-3]
    # dithered vertical gradient
    rows = np.arange(m.shape[0])[:, None] - pad
    v = np.clip(rows / (h - 1), 0, 1) * 3
    yy, xx = np.mgrid[0:m.shape[0], 0:m.shape[1]]
    th = BAYER4[yy % 4, xx % 4]
    band = np.floor(v).clip(0, 2).astype(int)
    frac = v - band
    pal = np.array([YELLOW, ORANGE, RED, MAGENTA], np.uint8)
    idx = band + (frac > th)
    rgb = pal[idx.clip(0, 3)]
    # bright top edge
    top = m & ~np.roll(m, 1, 0)
    rgb[top] = (255, 250, 190)
    return rgb, m, o & ~m, sh & ~o


TITLE_GLYPHS = {}


def title_letter(ch):
    if ch not in TITLE_GLYPHS:
        TITLE_GLYPHS[ch] = build_title_letter(ch)
    return TITLE_GLYPHS[ch]


def ease_drop(t, ts):
    u = (t - ts) / 11.0
    if u <= 0:
        return None
    if u >= 1:
        k = t - ts - 11
        return -abs(math.sin(k * 0.6)) * 4 * math.exp(-k * 0.25) if k < 14 else 0.0
    return -(1 - u) ** 2 * 120


def draw_title(img, t):
    TA = 190
    shine_t = t - 214
    letters_drawn = []
    for li, (word, y0) in enumerate(TITLE_LINES):
        pitch = 6 * TS
        x0 = (W - (len(word) * pitch - TS)) // 2
        for i, ch in enumerate(word):
            ts = TA + li * 5 + i * 2
            dy = ease_drop(t, ts)
            if dy is None:
                continue
            rgb, m, out, sh = title_letter(ch)
            x = x0 + i * pitch - 4
            y = int(round(y0 + dy)) - 4
            blit_mask(img, sh, x, y, (70, 8, 90))
            blit_mask(img, out, x, y, (20, 30, 150))
            # fresh-landing flash
            if 0 <= t - ts - 11 < 2:
                blit_mask(img, m, x, y, WHITE)
            else:
                full = np.zeros((H, W, 3), np.uint8)
                hh, ww = m.shape
                xa, ya = max(0, x), max(0, y)
                xb, yb = min(W, x + ww), min(H, y + hh)
                if xa < xb and ya < yb:
                    full[ya:yb, xa:xb] = rgb[ya - y:yb - y, xa - x:xb - x]
                    mm = np.zeros((H, W), bool)
                    mm[ya:yb, xa:xb] = m[ya - y:yb - y, xa - x:xb - x]
                    img[mm] = full[mm]
                    letters_drawn.append(mm)
    # diagonal shine sweep
    if letters_drawn and 0 <= shine_t < 26:
        allm = np.logical_or.reduce(letters_drawn)
        yy, xx = np.mgrid[0:H, 0:W]
        pos = -60 + shine_t * 16
        d = (xx + (yy - 46) * 0.6) - pos
        band = (d >= 0) & (d < 7) & allm
        img[band] = (255, 255, 220)
        band2 = ((d >= 7) & (d < 10) | (d >= -3) & (d < 0)) & allm
        img[band2] = YELLOW
    # glints
    for gx, gy, gt in ((40, 50, 222), (214, 96, 228), (70, 126, 234), (196, 44, 216)):
        k = t - gt
        if 0 <= k < 8:
            ln = [1, 2, 3, 4, 3, 2, 1, 1][k]
            for j in range(-ln, ln + 1):
                c = WHITE if abs(j) < 2 else CYAN
                put(img, gx + j, gy, c)
                put(img, gx, gy + j, c)


def draw_overlays(img, t):
    # STAGE CLEAR
    if 160 <= t < 192:
        s = "STAGE CLEAR"
        k = t - 160
        cols = [CYAN, YELLOW, MAGENTA, WHITE]
        c = cols[(k // 2) % 4]
        sc = 2
        w = text_w(s, sc)
        if k < 6:  # stretch in
            m = text_mask(s, sc)
            cut = int(m.shape[1] * (k + 1) / 6)
            m = m.copy()
            m[:, cut:] = False
            blit_mask(img, m, (W - w) // 2, 100, c)
        elif k < 28 or (k // 2) % 2 == 0:
            draw_text(img, s, (W - w) // 2 + 2, 102, (60, 10, 80), sc)
            draw_text(img, s, (W - w) // 2, 100, c, sc)
        if 8 <= k < 30:
            s2 = "BONUS 5000"
            draw_text(img, s2, (W - text_w(s2)) // 2, 122, WHITE if (k // 3) % 2 else YELLOW)
    if t >= 190:
        draw_title(img, t)
    if t >= 210:
        s = "INSERT COIN"
        if ((t - 210) // 10) % 2 == 0:
            w = text_w(s, 1)
            # double-width pixels for punch
            m = np.repeat(text_mask(s, 1), 2, axis=1)
            x = (W - m.shape[1]) // 2
            blit_mask(img, m, x + 1, 151, (90, 30, 0))
            blit_mask(img, m, x, 150, YELLOW if ((t - 210) // 20) % 2 == 0 else WHITE)
    if t >= 204:
        s = "1984 CRTSIM"
        c = CYAN
        draw_text(img, s, (W - text_w(s)) // 2, 182, c)
        s = "ALL RIGHTS RESERVED"
        draw_text(img, s, (W - text_w(s)) // 2, 193, (110, 90, 230))


# ---------------------------------------------------------------- main
def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "frames"
    os.makedirs(out, exist_ok=True)
    rng = np.random.default_rng(7)
    neb = build_nebula(rng)
    stars = build_stars(rng)
    g = Game()
    # pre-compute final score -> pick a HI that gets beaten late in the run
    sim = Game()
    for t in range(T0, NFRAMES):
        sim.step(t)
    start = 21350
    gain = sim.score - start
    hi = int(round((start + gain * 0.72) / 50.0)) * 50
    img = np.zeros((H, W, 3), np.uint8)
    for t in range(T0, NFRAMES):
        g.step(t)
        if t < 0:
            continue
        draw_background(img, t, neb, stars)
        draw_game(img, g, t)
        draw_hud(img, g, t, hi, t >= 192)
        draw_overlays(img, t)
        Image.fromarray(img).save(os.path.join(out, "%05d.png" % t))


if __name__ == "__main__":
    main()
