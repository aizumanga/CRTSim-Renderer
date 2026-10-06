#!/usr/bin/env python3
"""Green-phosphor terminal: an invented early-80s machine powers up, someone lists a disk and runs
a program that spins a character-art wireframe globe, then it returns to a blinking READY prompt.

    python3 terminal.py OUT_DIR [--amber]

Writes 240 RGB PNG frames (640x480, 8 s at 30 fps). numpy + Pillow only; deterministic.
The text grid is 64x24 cells of 10x20 pixels, from a bitmap font rasterised once from DejaVu Sans
Mono (supersampled, then thresholded, so every glyph is a clean 1-bit bitmap).
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

W, H = 640, 480
CW, CH = 10, 20
COLS, ROWS = W // CW, H // CH  # 64 x 24
NFRAMES = 240
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"

# Special glyph codes (private, below space).
FULL, UPPER, LOWER = 1, 2, 3

# ----------------------------------------------------------------------------- font

def build_font():
    atlas = np.zeros((128, CH, CW), np.float32)
    S = 8
    font = ImageFont.truetype(FONT, int(16.6 * S))
    for code in range(32, 127):
        im = Image.new("L", (CW * S, CH * S))
        d = ImageDraw.Draw(im)
        d.text((0, 1 * S), chr(code), font=font, fill=255)
        a = np.asarray(im, np.float32).reshape(CH, S, CW, S).mean(axis=(1, 3))
        atlas[code] = a > 96
        # Embolden by a pixel, as terminals did, so 1-pixel stems survive the CRT's blur.
        atlas[code, :, 1:] = np.maximum(atlas[code, :, 1:], atlas[code, :, :-1])
    atlas[FULL] = 1
    atlas[UPPER, : CH // 2] = 1
    atlas[LOWER, CH // 2:] = 1
    return atlas


# Big 5x7 logo letters, drawn with half-block cells (each cell = 1x2 logo pixels).
LOGO_GLYPHS = {
    "C": [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "S": [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
    "I": [".###.", "..#..", "..#..", "..#..", "..#..", "..#..", ".###."],
    "M": ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"],
    "-": [".....", ".....", ".....", "####.", ".....", ".....", "....."],
    "8": [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."],
    "0": [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."],
}


def logo_cells(text):
    cols = []
    for ch in text:
        g = LOGO_GLYPHS[ch]
        for x in range(5):
            cols.append([g[y][x] == "#" for y in range(7)] + [False])
        cols.append([False] * 8)
    cols = cols[:-1]
    bits = np.array(cols).T  # 8 x n
    rows = []
    for r in range(4):
        top, bot = bits[2 * r], bits[2 * r + 1]
        rows.append([FULL if t and b else UPPER if t else LOWER if b else 32 for t, b in zip(top, bot)])
    return rows


# ----------------------------------------------------------------------------- terminal

class Term:
    def __init__(self):
        self.ch = np.full((ROWS, COLS), 32, np.int32)
        self.lv = np.zeros((ROWS, COLS), np.float32)
        self.inv = np.zeros((ROWS, COLS), bool)
        self.x = self.y = 0

    def scroll(self):
        self.ch[:-1] = self.ch[1:].copy(); self.ch[-1] = 32
        self.lv[:-1] = self.lv[1:].copy(); self.lv[-1] = 0
        self.inv[:-1] = self.inv[1:].copy(); self.inv[-1] = False

    def newline(self):
        self.x = 0
        self.y += 1
        if self.y >= ROWS:
            self.scroll()
            self.y = ROWS - 1

    def putc(self, c, level=0.85):
        if c == "\n":
            self.newline()
            return
        if self.x >= COLS:
            self.newline()
        self.ch[self.y, self.x] = ord(c) if isinstance(c, str) else c
        self.lv[self.y, self.x] = level
        self.x += 1

    def put_at(self, row, col, text, level=0.85, inv=False):
        for i, c in enumerate(text):
            if 0 <= col + i < COLS:
                self.ch[row, col + i] = ord(c)
                self.lv[row, col + i] = level
                self.inv[row, col + i] = inv

    def clear(self):
        self.ch[:] = 32; self.lv[:] = 0; self.inv[:] = False
        self.x = self.y = 0


# ----------------------------------------------------------------------------- rendering

class Out:
    def __init__(self, out_dir, amber):
        self.dir = out_dir
        self.n = 0
        self.atlas = build_font()
        if amber:
            self.tint = np.array([255, 176, 40], np.float32)
            self.bg = np.array([7, 4, 1], np.float32)
        else:
            self.tint = np.array([70, 255, 120], np.float32)
            self.bg = np.array([1, 7, 3], np.float32)
        self.blink_from = 0

    def frame(self, t, cursor="blink", count=1):
        for _ in range(count):
            if self.n >= NFRAMES:
                return
            on = cursor == "solid" or (cursor == "blink" and ((self.n - self.blink_from) // 9) % 2 == 0)
            ch, lv, inv = t.ch.copy(), t.lv.copy(), t.inv.copy()
            if on and 0 <= t.y < ROWS and 0 <= t.x < COLS:
                inv[t.y, t.x] = not inv[t.y, t.x]
                lv[t.y, t.x] = 0.95
            g = self.atlas[ch]  # R C h w
            g = np.where(inv[..., None, None], 1 - g, g) * lv[..., None, None]
            img = g.transpose(0, 2, 1, 3).reshape(H, W)
            rgb = self.bg + img[..., None] * (self.tint - self.bg)
            Image.fromarray(np.clip(rgb + 0.5, 0, 255).astype(np.uint8), "RGB").save(
                os.path.join(self.dir, f"{self.n:05d}.png"))
            self.n += 1


# ----------------------------------------------------------------------------- globe

LIGHT = np.array([-0.55, 0.45, 0.70]); LIGHT /= np.linalg.norm(LIGHT)
_rng = np.random.default_rng(1983)
LAND_K = _rng.normal(size=(7, 3)) * np.array([[2.2], [2.6], [3.1], [3.7], [4.6], [5.5], [7.0]])
LAND_P = _rng.uniform(0, 2 * np.pi, 7)
LAND_A = np.array([1.0, 0.9, 0.75, 0.6, 0.45, 0.35, 0.25])


def slope_char(gx, gy):
    """Character for a line whose function gradient (screen px, y down) is (gx, gy)."""
    ang = (np.degrees(np.arctan2(gx, gy)) + 180.0) % 180.0  # direction of the line, y up
    out = np.full(ang.shape, ord("-"), np.int32)
    out[(ang >= 30) & (ang < 76)] = ord("/")
    out[(ang >= 76) & (ang < 104)] = ord("|")
    out[(ang >= 104) & (ang < 150)] = ord("\\")
    return out


def globe(spin, tilt, cx, cy, R, rows, cols):
    """Character art of a wireframe globe. Returns (codes, levels) for a rows x cols grid."""
    # sample each cell at 3x3 points (corners+edges+centre) to find line crossings
    sx = np.array([0.0, 0.5, 1.0]); sy = np.array([0.0, 0.5, 1.0])
    X = (np.arange(cols)[None, :, None, None] + sx[None, None, None, :]) * CW
    Y = (np.arange(rows)[:, None, None, None] + sy[None, None, :, None]) * CH
    X, Y = np.broadcast_arrays(X, Y)  # r c sy sx
    nx = (X - cx) / R
    ny = -(Y - cy) / R
    rr = nx * nx + ny * ny
    inside = rr < 1
    nz = np.sqrt(np.clip(1 - rr, 0, 1))

    ct, st = np.cos(tilt), np.sin(tilt)
    cs, ss = np.cos(spin), np.sin(spin)
    roll = np.radians(-14); cr, sr = np.cos(roll), np.sin(roll)

    def to_globe(z):
        x, y = nx * cr - ny * sr, nx * sr + ny * cr
        y2 = y * ct - z * st
        z2 = y * st + z * ct
        x3 = x * cs + z2 * ss
        z3 = -x * ss + z2 * cs
        return x3, y2, z3

    codes = np.full((rows, cols), 32, np.int32)
    levels = np.zeros((rows, cols), np.float32)
    inc = inside.any(axis=(2, 3))
    allin = inside.all(axis=(2, 3))
    ctr_in = inside[:, :, 1, 1]
    lam = np.clip(nx * LIGHT[0] + ny * LIGHT[1] + nz * LIGHT[2], 0, 1)[:, :, 1, 1]

    def lines(gx, gy, gz, front):
        lat = np.arcsin(np.clip(gy, -1, 1))
        lon = np.arctan2(gx, gz)
        fm = np.sin(4 * lon) * np.where(np.abs(lat) < np.radians(72), 1, np.nan)
        fp = np.sin(4.5 * lat)
        res = []
        for f in (fm, fp):
            f = np.where(inside, f, np.nan)
            fmax = np.nanmax(np.where(np.isnan(f), -9, f), axis=(2, 3))
            fmin = np.nanmin(np.where(np.isnan(f), 9, f), axis=(2, 3))
            hit = (fmax > 0) & (fmin < 0) & ctr_in & (nz[:, :, 1, 1] > 0.22)
            # gradient from cell edges
            gxs = (f[:, :, 1, 2] - f[:, :, 1, 0]) / CW
            gys = (f[:, :, 2, 1] - f[:, :, 0, 1]) / CH
            ok = hit & np.isfinite(gxs) & np.isfinite(gys)
            res.append((ok, slope_char(np.nan_to_num(gxs), np.nan_to_num(gys))))
        land = None
        if front:
            p = np.stack([gx[:, :, 1, 1], gy[:, :, 1, 1], gz[:, :, 1, 1]], -1)
            v = (np.sin(p @ LAND_K.T + LAND_P) * LAND_A).sum(-1)
            v += 0.9 * np.abs(p[..., 1]) ** 3  # ice caps
            land = v > 0.30
        return res, land

    (bm, bp), _ = lines(*to_globe(-nz), front=False)
    (fm_, fp_), land = lines(*to_globe(nz), front=True)

    # back-side grid, dim
    for ok, cc in (bp, bm):
        codes = np.where(ok, ord("."), codes); levels = np.where(ok, 0.45, levels)
    # land, shaded
    ramp = np.array([ord(c) for c in ":-=+*#%@"], np.int32)
    li = np.clip((lam * 1.25 * len(ramp)).astype(int), 0, len(ramp) - 1)
    landc = land & ctr_in
    codes = np.where(landc, ramp[li], codes)
    levels = np.where(landc, 0.36 + 0.6 * lam, levels)
    # front grid, bright on the lit side
    for ok, cc in (fp_, fm_):
        codes = np.where(ok, cc, codes)
        levels = np.where(ok, 0.62 + 0.38 * np.sqrt(lam), levels)
    # limb outline
    limb = inc & ~allin
    r2 = rr
    gx = (r2[:, :, 1, 2] - r2[:, :, 1, 0]) / CW
    gy = (r2[:, :, 2, 1] - r2[:, :, 0, 1]) / CH
    lc = slope_char(gx, gy)
    codes = np.where(limb, lc, codes)
    levels = np.where(limb, 0.95, levels)
    return codes, levels


# ----------------------------------------------------------------------------- script

def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    out_dir = sys.argv[1]
    os.makedirs(out_dir, exist_ok=True)
    amber = "--amber" in sys.argv[2:]
    o = Out(out_dir, amber)
    t = Term()
    rng = np.random.default_rng(80)

    BAUD = 34  # characters per frame (about 9600 baud)

    def emit(text, level=0.85, cps=BAUD, cursor="solid"):
        """Print text at line speed, one frame per cps characters."""
        n = 0
        for c in text:
            t.putc(c, level)
            if c != "\n":
                n += 1
            if n >= cps:
                o.frame(t, cursor); n = 0
        if n:
            o.frame(t, cursor)

    def type_cmd(cmd):
        """A person typing: irregular gaps, a hesitation before Enter."""
        for i, c in enumerate(cmd):
            gap = int(rng.choice([1, 2, 2, 3, 3, 3, 4, 6]))
            if c == " ":
                gap += 3
            o.frame(t, "solid", gap)
            t.putc(c, 0.95)
        o.frame(t, "solid", 4)

    def idle(n):
        o.blink_from = o.n
        o.frame(t, "blink", n)

    # --- power-on: logo already lit (frame 0 is the thumbnail), memory count ticking
    logo = logo_cells("CRTSIM-80")
    lx = (COLS - len(logo[0])) // 2
    for r, row in enumerate(logo):
        for c, code in enumerate(row):
            t.ch[1 + r, lx + c] = code
            t.lv[1 + r, lx + c] = 1.0
    t.put_at(5, lx, "=" * len(logo[0]), 0.4)
    t.put_at(6, 4, "PHOSPHOR DATA TERMINAL  *  MONITOR ROM V2.3", 0.9)
    t.put_at(7, 4, "(C) 1982 KOVAREL DATAWORKS.  ALL RIGHTS RESERVED.", 0.55)
    t.y, t.x = 9, 0
    label = " MEMORY CHECK ...... "
    t.put_at(9, 0, label, 0.85)
    mem = 0
    steps = [1024, 2048, 3072, 2048, 4096, 2048, 3072, 4096, 2048, 3072, 4096, 3072, 4096,
             2048, 4096, 3072, 4096, 2048, 4096, 4096, 3072]
    for s in steps:
        mem = min(65536, mem + s)
        t.put_at(9, len(label), f"{mem:06d} BYTES", 1.0)
        t.x = len(label) + 12
        o.frame(t, "none")
    t.put_at(9, len(label), "065536 BYTES", 1.0)
    t.y, t.x = 9, len(label) + 12
    emit(" OK", 1.0)
    o.frame(t, "none", 2)
    t.newline()
    emit(" DISK UNIT D1 ...... ")
    o.frame(t, "none", 3)
    emit("ONLINE", 1.0)
    t.newline()
    emit(" CLOCK ............. 2.048 MHZ\n\n")
    emit(" SYSTEM READY.\n", 0.95)
    emit(">", 0.95)
    idle(9)

    # --- list the disk
    type_cmd("FILES")
    t.newline()
    listing = [
        (" VOL D1:WORKDISK        9 FILES       214 BLOCKS FREE", 0.95),
        (" NAME      TYPE   SIZE   DATE", 0.5),
        (" BOOTSTRP  SYS    2048   09-14-82", 0.85),
        (" ORBIT     PRG    6144   03-02-83", 1.0),
        (" LEDGER    DAT   12800   02-17-83", 0.85),
        (" STARMAP   DAT    9216   01-05-83", 0.85),
        (" FONTGEN   PRG    2560   12-12-82", 0.85),
        (" GLYPHS    BIN    4096   12-12-82", 0.85),
        (" SCRATCH   TMP     256   03-04-83", 0.85),
    ]
    for text, lv in listing:
        emit(text + "\n", lv, cps=36)
    emit("\n>", 0.95)
    idle(4)

    # --- run the program
    type_cmd("GO ORBIT")
    t.newline()
    emit(" LOADING ORBIT.PRG ", 0.85)
    for _ in range(4):
        t.putc(".", 0.85)
        o.frame(t, "none", 2)
    emit(" 6144 BYTES\n", 0.85)
    o.frame(t, "none", 2)
    # clear screen, top to bottom
    for r in range(0, ROWS, 8):
        t.ch[r:r + 8] = 32; t.lv[r:r + 8] = 0; t.inv[r:r + 8] = False
        o.frame(t, "none")
    t.clear()

    # --- the globe program
    end_tail = 24  # READY + blinking cursor at the end
    gl_frames = NFRAMES - o.n - end_tail
    print("globe starts at", o.n, "frames", gl_frames, file=sys.stderr)
    g_rows, g_cols = 21, 42
    gcx, gcy, gR = 21 * CW, 1 * CH + 10.5 * CH, 10.2 * CH
    tilt = np.radians(24)
    spin = 0.0
    px0 = 45  # right panel column
    for k in range(gl_frames):
        u = k / max(gl_frames - 1, 1)
        speed = np.radians(5.2) * (1 - u ** 3)  # spins down as the program ends
        spin += speed
        codes, levels = globe(spin, tilt, gcx, gcy - CH, gR, g_rows, g_cols)
        # program draws the globe top to bottom at first
        reveal = min(g_rows, k * 2 + 1)
        t.ch[1:1 + g_rows, 0:g_cols] = 32
        t.lv[1:1 + g_rows, 0:g_cols] = 0
        t.ch[1:1 + reveal, 0:g_cols] = codes[:reveal]
        t.lv[1:1 + reveal, 0:g_cols] = levels[:reveal]
        if reveal < g_rows:
            t.lv[reveal, 0:g_cols] = np.maximum(t.lv[reveal, 0:g_cols], 0.0)
        # header bar
        lon = (np.degrees(spin) + 180) % 360 - 180
        head = f" ORBIT/3  SPHERE PROJECTOR          LON {lon:+07.1f}  F{k:04d} "
        t.put_at(0, 0, head.ljust(COLS), 0.85, inv=True)
        # right panel
        t.put_at(2, px0, "+" + "-" * 17 + "+", 0.5)
        for r in range(3, 21):
            t.put_at(r, px0, "|", 0.5); t.put_at(r, px0 + 18, "|", 0.5)
        t.put_at(21, px0, "+" + "-" * 17 + "+", 0.5)
        t.put_at(3, px0 + 2, "TELEMETRY", 1.0)
        t.put_at(5, px0 + 2, f"TILT   {np.degrees(tilt):5.1f}", 0.8)
        t.put_at(6, px0 + 2, f"RATE   {np.degrees(speed):5.2f}", 0.8)
        t.put_at(7, px0 + 2, f"REV    {spin / (2 * np.pi):5.2f}", 0.8)
        t.put_at(8, px0 + 2, f"GRID   45/40", 0.8)
        t.put_at(10, px0 + 2, "SIGNAL", 1.0)
        # character sine plot, scrolling
        pw, ph, prow = 15, 9, 11
        for r in range(ph):
            t.put_at(prow + r, px0 + 2, " " * pw, 0.3)
        ph_ = spin * 1.6
        ys = []
        for c in range(pw + 1):
            a = ph_ + c * 0.42
            v = 0.7 * np.sin(a) + 0.25 * np.sin(2.7 * a + 1.0)
            ys.append(int(round((1 - (v + 1) / 2) * (ph - 1))))
        t.put_at(prow + (ph - 1) // 2, px0 + 2, "." * pw, 0.3)
        for c in range(pw):
            r0, r1 = ys[c], ys[c + 1]
            if r0 == r1:
                t.put_at(prow + r0, px0 + 2 + c, "-", 1.0)
            else:
                ch = "\\" if r1 > r0 else "/"
                for r in range(min(r0, r1), max(r0, r1) + 1):
                    t.put_at(prow + r, px0 + 2 + c, ch, 1.0)
        # status line
        if u < 0.93:
            t.put_at(22, 0, " RUNNING - PRESS BRK TO HALT".ljust(COLS), 0.5)
        else:
            t.put_at(0, 37, "*HALTED*", 0.85, inv=False)
            t.put_at(22, 0, " ORBIT/3 ENDED.  0 ERRORS.".ljust(COLS), 0.9)
        t.y, t.x = 23, 0
        o.frame(t, "none")

    # --- back to the prompt (the program's last newline scrolls the screen)
    t.y, t.x = 22, 27
    emit("\nREADY\n>", 0.95)
    idle(NFRAMES - o.n)
    assert o.n == NFRAMES, o.n


if __name__ == "__main__":
    main()
