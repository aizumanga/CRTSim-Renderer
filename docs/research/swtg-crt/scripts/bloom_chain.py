#!/usr/bin/env python3
"""The game's bloom chain modelled on its own PreHUD image (monitor-pass readback), against its
captured downsample, upsample and compose (final, before HUD) outputs:
  level0   downsample taps read level 0 (what this renderer's blur() does)
  mips     downsample taps read mip level 4, the mips made as Mesa's glGenerateMipmap makes them
           here: horizontal pairs averaged rounding half up, then vertical pairs likewise
           (bit-exact against the game's dumped levels, run_title/bloom_000_prehud_L*.rgba)
  box      as mips, each level a plain 2x2 average rounded to nearest (a driver-neutral box)
The upsample (taps .yx) and compose (scene + ColorPow(blur, BloomPower) * BloomScalar) follow
post_Low.fx and Valkyrie_FXAA_None.fx; every pass writes 8 bits rounded to nearest.
usage: bloom_chain.py RUN_DIR TAG [FRAME]"""
import re, sys
import numpy as np

run, tag = sys.argv[1:3]
frame = int(sys.argv[3]) if len(sys.argv) > 3 else 0
POISSON = [(0, 0), (0, 1), (0, -1), (-0.866025, 0.5), (-0.866025, -0.5), (0.866025, 0.5), (0.866025, -0.5)]


def load(p):
    t = open(f'{run}/{tag}_{frame:03d}_{p}.txt').read()
    w, h = map(int, re.search(r'size (\d+) (\d+)', t).groups())
    return np.fromfile(f'{run}/{tag}_{frame:03d}_{p}.rgba', np.uint8).reshape(h, w, 4)[..., :3].astype(np.float64), t


def bilinear(img, u, v):
    h, w = img.shape[:2]
    x = np.clip(u * w - 0.5, 0, w - 1); y = np.clip(v * h - 0.5, 0, h - 1)
    x0 = np.floor(x).astype(int); y0 = np.floor(y).astype(int)
    x1 = np.minimum(x0 + 1, w - 1); y1 = np.minimum(y0 + 1, h - 1)
    fx = (x - x0)[..., None]; fy = (y - y0)[..., None]
    return (img[y0, x0] * (1 - fx) + img[y0, x1] * fx) * (1 - fy) + (img[y1, x0] * (1 - fx) + img[y1, x1] * fx) * fy


def taps(img, w, h, scale, swap=False):
    v, u = np.mgrid[0:h, 0:w]
    u = (u + 0.5) / w; v = (v + 0.5) / h
    total = sum(bilinear(img, u + (py if swap else px) * scale[0], v + (px if swap else py) * scale[1])
                for px, py in POISSON)
    return np.floor(total / 7 + 0.5)


def mip(a, how):
    a = a.astype(np.int64)
    a0, a1, a2, a3 = a[0::2, 0::2], a[0::2, 1::2], a[1::2, 0::2], a[1::2, 1::2]
    if how == 'mips':
        return ((((a0 + a1 + 1) // 2) + ((a2 + a3 + 1) // 2) + 1) // 2).astype(np.float64)
    return np.floor((a0 + a1 + a2 + a3) / 4 + 0.5)


def colorpow(c, p):
    luma = c @ np.array([0.299, 0.587, 0.114])
    return c / np.maximum(luma, 1e-6)[..., None] * (np.maximum(luma, 0) ** p)[..., None]


pre, _ = load('monitor')
down_g, tdown = load('bloomdown')
up_g, tup = load('bloomup')
comp_g, tcomp = load('compose')
scale = tuple(map(float, re.search(r'BloomScale = ([\d.e-]+) ([\d.e-]+)', tdown).groups()))
bscalar = float(re.search(r'BloomScalar = ([\d.e-]+)', tcomp).group(1))
bpower = float(re.search(r'BloomPower = ([\d.e-]+)', tcomp).group(1))
H, W = pre.shape[:2]; h, w = down_g.shape[:2]
lvl = int(round(np.log2(W / w)))

def stats(name, m, g):
    d = np.abs(m - g); worst = d.max(2)
    return f'{name:34s} mean {d.mean():6.3f}  worst-ch {worst.mean():6.3f}  past 2 {100*(worst>2).mean():6.2f}%  max {worst.max():4.0f}'

print(f'{run}/{tag} frame {frame}: PreHUD {W}x{H}, downsample {w}x{h} (level {lvl}), BloomScale {scale}, BloomScalar {bscalar}, BloomPower {bpower}')
# the game's own upsample from its own downsample: checks the upsample model
up_from_g = taps(down_g, W, H, scale, swap=True)
print(' ', stats('upsample model on game downsample', up_from_g, up_g))
comp_from_g = np.clip(np.floor(np.clip(pre / 255 + colorpow(up_g / 255, bpower) * bscalar, 0, 1) * 255 + 0.5), 0, 255)
print(' ', stats('compose model on game upsample', comp_from_g, comp_g))
for how in ('level0', 'mips', 'box'):
    if how == 'level0':
        src = pre
    else:
        src = pre
        for _ in range(lvl):
            src = mip(src, how)
    down = taps(src, w, h, scale)
    up = taps(down, W, H, scale, swap=True)
    comp = np.clip(np.floor(np.clip(pre / 255 + colorpow(up / 255, bpower) * bscalar, 0, 1) * 255 + 0.5), 0, 255)
    print(f'  [{how}]')
    print('   ', stats('downsample vs game', down, down_g))
    print('   ', stats('upsample vs game', up, up_g))
    print('   ', stats('compose (final) vs game', comp, comp_g))
