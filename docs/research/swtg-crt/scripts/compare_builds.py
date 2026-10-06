#!/usr/bin/env python3
"""Which artifact orientation a build of the game draws: its screen frames against this
renderer's pictures of the same scene with the pattern unflipped and flipped.

usage: compare_builds.py FRAMES_DIR RENDER_UNFLIPPED RENDER_FLIPPED [--half-pixel]
  FRAMES_DIR        PNG screen grabs of the game, 1280x960 (play_windows.sh, or ffmpeg's
                    x11grab beside play_to_gameplay.sh)
  RENDER_*          render_probe output folders: PALETTE=0 render_probe of the scene's NTSC
                    frame, which capture_shim records, with FLIP_ARTIFACTS=0 and =1
  --half-pixel      move the renders half a pixel right and down first, as Direct3D 9 draws
                    the Windows build's final pass
Prints, for each orientation, the closest pair of frames inside the glass: mean of the worst
channel and the share of pixels past 16 steps, in 8-bit steps. Run with ARTIFACTS=0 renders
and a capture with NTSC 0 for the control both should then come close to."""
import glob, sys
import numpy as np
from PIL import Image

load = lambda f: np.asarray(Image.open(f).convert('RGB')).astype(float)


def distinct(folder):
    out = []
    for f in sorted(glob.glob(f'{folder}/*.png')):
        a = load(f)
        if a.mean() > 1 and not any(np.array_equal(a, b) for b in out):
            out.append(a)
    return out


def shift(a, dy, dx):
    """`a` moved by (dy, dx) pixels, blended bilinearly."""
    y0, x0 = int(np.floor(dy)), int(np.floor(dx))
    fy, fx = dy - y0, dx - x0
    r = lambda yy, xx: np.roll(np.roll(a, yy, 0), xx, 1)
    return (r(y0, x0) * (1 - fx) + r(y0, x0 + 1) * fx) * (1 - fy) + \
           (r(y0 + 1, x0) * (1 - fx) + r(y0 + 1, x0 + 1) * fx) * fy


frames, unflipped, flipped = sys.argv[1:4]
half = '--half-pixel' in sys.argv
glass = (slice(120, 840), slice(120, 1160))
game = distinct(frames)
print(f'{len(game)} distinct frames of the game')
for name, folder in (('unflipped', unflipped), ('flipped', flipped)):
    renders = [load(f) for f in glob.glob(f'{folder}/*_final.png')]
    renders = [shift(r, 0.5, 0.5) if half else r for r in renders]
    d = min((np.abs(g[glass] - r[glass]).max(2) for g in game for r in renders),
            key=lambda d: d.mean())
    print(f'  {name:9s} mean {d.mean():.3f}  past 16: {(d > 16).mean() * 100:.3f}%')
