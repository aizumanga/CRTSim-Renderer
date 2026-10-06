"""Compares the passes capture_shim.c read back from the game with render_probe's output, frame
for frame by the artifact-pattern lerp each used: worst and mean differences in 8-bit steps,
and the share of pixels past 2 and 8, with each difference image (x8) written to OUTDIR.

usage: compare_passes.py GAME_DIR TAG RENDER_DIR OUT_DIR"""
import sys, glob, re, numpy as np
from PIL import Image
game, tag, ren, out = sys.argv[1:5]
def game_img(frame, p):
    t = open(f'{game}/{tag}_{frame:03d}_{p}.txt').read()
    w, h = map(int, re.search(r'size (\d+) (\d+)', t).groups())
    a = np.fromfile(f'{game}/{tag}_{frame:03d}_{p}.rgba', np.uint8).reshape(h, w, 4)[::-1, :, :3]
    lerp = re.search(r'NTSCLerp = ([\d.]+)', open(f'{game}/{tag}_{frame:03d}_composite.txt').read()).group(1)
    return a, float(lerp)
def ren_img(lerp, what):
    f = sorted(glob.glob(f'{ren}/frame*_lerp{lerp}_{what}.png'))[-1]
    return np.array(Image.open(f).convert('RGB'))
def stats(name, a, b):
    d = np.abs(a.astype(int) - b.astype(int))
    print(f'{name:28s} max {d.max():3d}  mean {d.mean():6.3f}  mean/ch {d.mean((0,1)).round(2)}  >2: {100*(d.max(2)>2).mean():5.1f}%  >8: {100*(d.max(2)>8).mean():5.1f}%')
    Image.fromarray(np.clip(d * 8, 0, 255).astype(np.uint8)).save(f'{out}/diff_{name}.png')
    return d
import os; os.makedirs(out, exist_ok=True)
for frame in (0, 1):
    g_ntsc, lerp = game_img(frame, 'ntsc')
    key = '0.175' if lerp < 0.5 else '0.825'
    stats(f'ntsc_f{frame}', g_ntsc, ren_img(key, 'prepared'))
    g_comp, _ = game_img(frame, 'composite')
    stats(f'composite_lerp{key}', g_comp, ren_img(key, 'composite'))
    g_final, _ = game_img(frame, 'compose')
    r_final = ren_img(key, 'final')
    stats(f'final_lerp{key}', g_final, r_final)
    Image.fromarray(g_final).save(f'{out}/game_final_lerp{key}.png')
    Image.fromarray(r_final).save(f'{out}/ren_final_lerp{key}.png')
