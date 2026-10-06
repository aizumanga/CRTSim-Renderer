#!/usr/bin/env python3
"""Measures CRTSim-Renderer's glass and bezel against the meshes the game draws (surfaces.py):
where each shows which surface, how far apart their UVs, normals, shades and reflections are,
and what that does to the picture when both are shaded alike, as the game shades them.

usage: measure_surfaces.py BEZEL_MAPS_DIR COMPOSITE.png OUT_DIR [WIDTH HEIGHT FOV]
  BEZEL_MAPS_DIR  what bezel_maps writes
  GLASS=formula   measures the glass as the renderer drew it until session 7
  COMPOSITE.png   a 256x224 composite frame to shade with (rows top first)
Writes OUT_DIR/report.txt and maps of each error."""
import sys, os
import numpy as np
from PIL import Image
import surfaces as s

maps, comp_path, out = sys.argv[1:4]
W, H, FOV = (int(sys.argv[4]), int(sys.argv[5]), float(sys.argv[6])) if len(sys.argv) > 6 else (1280, 960, 30.)
assets = os.path.join(os.path.dirname(__file__), '../../../../assets/original-crtsim')
os.makedirs(out, exist_ok=True)
ref = s.rasterize(s.read_m3d(f'{assets}/screen.m3d'), s.read_m3d(f'{assets}/frame.m3d'), W, H, FOV)
app = s.approximate(s.Bezel(maps), W, H, FOV, formula=os.environ.get('GLASS') == 'formula')
comp = np.asarray(Image.open(comp_path).convert('RGB')) / 255.
mask = np.asarray(Image.open(f'{assets}/mask.bmp').convert('RGB')) / 255.
levels = s.mask_levels(mask)
lines = []
say = lambda t='': (lines.append(t), print(t))

def stats(v):
    v = np.asarray(v)
    return f'mean {v.mean():.4f}  p95 {np.percentile(v, 95):.4f}  p99 {np.percentile(v, 99):.4f}  max {v.max():.4f}'

def save(name, v, scale):
    Image.fromarray(np.clip(v * scale, 0, 255).astype(np.uint8)).save(f'{out}/{name}.png')

names = {0: 'nothing', 1: 'glass', 2: 'bezel'}
say(f'{W}x{H}, vertical field of view {FOV}')
say('Coverage: rows the meshes, columns the approximation (pixels)')
for a in (0, 1, 2):
    say(f'  {names[a]:8s}' + ''.join(f'{((ref["kind"] == a) & (app["kind"] == b)).sum():>10d}' for b in (0, 1, 2)))
disagree = ref['kind'] != app['kind']
save('coverage_disagreement', disagree.astype(float), 255)

signal = np.array([256., 224. * s.GAME['uv_scale']])
angle = lambda a, b: np.degrees(np.arccos(np.clip((a * b).sum(-1), -1, 1)))
for k in (1, 2):
    m = (ref['kind'] == k) & (app['kind'] == k)
    say()
    say(f'{names[k].capitalize()}, where both show it ({m.sum()} pixels)')
    uv_err = np.linalg.norm((ref['uv'] - app['uv']) * signal, axis=-1)
    say(f'  UV, in signal pixels:       {stats(uv_err[m])}')
    say(f'  normal, degrees:            {stats(angle(ref["normal"], app["normal"])[m])}')
    say(f'  shade, 8-bit steps:         {stats(np.abs(ref["shade"] - app["shade"])[m] * 255)}')
    if k == 2:
        say(f'  reflection, 8-bit steps:    {stats(np.abs(ref["reflection"] - app["reflection"])[m] * 255)}')
    say(f'  depth (x), mesh units:      {stats(np.abs(ref["point"][..., 0] - app["point"][..., 0])[m])}')
    save(f'{names[k]}_uv_error', np.where(m, uv_err, 0), 255 / 2)

say()
say('Picture, both shaded alike (mask on), 8-bit steps per pixel (worst channel)')
a = s.shade(ref, comp, levels, FOV).astype(int)
b = s.shade(app, comp, levels, FOV).astype(int)
Image.fromarray(a.astype(np.uint8)).save(f'{out}/meshes.png')
Image.fromarray(b.astype(np.uint8)).save(f'{out}/approximation.png')
d = np.abs(a - b).max(-1)
save('picture_difference', d, 16)
# The glass's outline: within 4 pixels of a pixel the two show differently.
near_edge = np.zeros_like(disagree)
for dy in range(-4, 5):
    for dx in range(-4, 5):
        near_edge |= np.roll(np.roll(disagree, dy, 0), dx, 1)
regions = {
    'whole picture': np.ones_like(disagree),
    'glass, inside': (ref['kind'] == 1) & ~near_edge,
    'bezel, inside': (ref['kind'] == 2) & ~near_edge,
    'outlines (4 px)': near_edge,
}
for name, r in regions.items():
    say(f'  {name:16s} {r.sum():>8d} px  mean {d[r].mean():6.3f}  >2: {(d[r] > 2).mean() * 100:6.2f}%  '
        f'>8: {(d[r] > 8).mean() * 100:6.2f}%  max {d[r].max()}')

say()
say('What each difference costs, inside the glass and bezel: the meshes with one thing taken')
say('from the approximation, mean 8-bit steps (worst channel)')
inside = (ref['kind'] == app['kind']) & ~near_edge
for part in ('uv', 'normal', 'color', 'reflection', 'point'):
    mixed = {k: v.copy() for k, v in ref.items()}
    mixed[part][inside] = app[part][inside]
    c = s.shade(mixed, comp, levels, FOV).astype(int)
    e = np.abs(c - a).max(-1)
    say(f'  {part:10s} glass {e[inside & (ref["kind"] == 1)].mean():.3f}  '
        f'bezel {e[inside & (ref["kind"] == 2)].mean():.3f}')
open(f'{out}/report.txt', 'w').write('\n'.join(lines) + '\n')
