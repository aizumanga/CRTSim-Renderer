"""The game's composite.fx in numpy, to test which inputs and texture orientation reproduce a
captured composite frame. Images are HxWx3 float32 in 0-1, in the row order their textures
hold; the artifact texture as uploaded, its second sample one row further on."""
import numpy as np
from PIL import Image
def f32(a): return a.astype(np.float32)
def composite(cur, prev, art_mem, lerp, sharp=0.8, pers=(0.7, 0.525, 0.42), bleed=0.5, ntsc=0.5):
    """cur, prev: HxWx3 0-1 in memory order; art_mem: artifact texture rows as uploaded."""
    H, W = cur.shape[:2]
    clampx = lambda a, dx: a[:, np.clip(np.arange(W) + dx, 0, W - 1)]
    a1 = art_mem
    a2 = art_mem[(np.arange(H) + 1) % art_mem.shape[0]]
    art = a1 + (a2 - a1) * np.float32(lerp)
    cl, cr = clampx(cur, -1), clampx(cur, 1)
    tot = art * np.float32(ntsc)
    c = np.clip(cur + ((cl - cur) + (cr - cur)) * tot, 0, 1)
    lum = lambda v: v @ np.array([0.299, 0.587, 0.114], np.float32)
    brt = lum(c); off = np.zeros((H, W), np.float32)
    for i, w in enumerate([1.0, -0.3162277, 0.1]):
        off += ((brt - lum(clampx(cur, -(i + 1)))) + (brt - lum(clampx(cur, i + 1)))) * np.float32(w)
    c = np.clip(c + off[..., None] * np.float32(sharp) * (1 + (art - 1) * np.float32(ntsc)), 0, 1)
    pl, pr = clampx(prev, -1), clampx(prev, 1)
    p = np.array(pers, np.float32) * np.float32(1 / (1 + 2 * bleed)) * (prev + (pl + pr) * np.float32(bleed))
    return np.clip(np.maximum(c, p), 0, 1)
