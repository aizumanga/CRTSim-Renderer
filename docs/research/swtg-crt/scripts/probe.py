#!/usr/bin/env python3
"""A 256x224 probe card in the NES palette the game's art uses: its 56 colours, a grey ramp
through every 8-bit level, black/white edges and 1- and 2-pixel stripes, isolated bright
pixels and short lines, and saturated colour patches on grey.

usage: probe.py NES_PALETTE.pal OUT  (writes OUT.png and OUT.rgba, raw RGBA rows top first)"""
import sys
from PIL import Image

data = open(sys.argv[1], 'rb').read()               # RIFF PAL: 24-byte header, then RGBX
nes = [tuple(data[24 + 4 * i: 27 + 4 * i]) for i in range(64)]
img = Image.new('RGBA', (256, 224), (0, 0, 0, 255))
px = img.load()
for row in range(4):
    for hue in range(14):
        for y in range(row * 14, row * 14 + 14):
            for x in range(hue * 18, hue * 18 + 18):
                px[x, y] = nes[hue + 16 * row] + (255,)
for x in range(256):
    for y in range(60, 76):
        px[x, y] = (x, x, x, 255)
for y in range(80, 112):
    for x in range(256):
        v = 255 if (x // 32) % 2 else 0
        if 128 <= x < 192:
            v = 255 if x % 2 else 0
        if 192 <= x < 256:
            v = 255 if (x // 2) % 2 else 0
        px[x, y] = (v, v, v, 255)
for i, c in enumerate([(255, 0, 0), (0, 255, 0), (0, 0, 255), (255, 255, 255), (252, 252, 252),
                       (248, 56, 0)]):
    px[20 + i * 40, 130] = c + (255,)
    for x in range(10 + i * 40, 34 + i * 40):
        px[x, 150] = c + (255,)
for i, index in enumerate([0x16, 0x2A, 0x12, 0x28, 0x14, 0x2C, 0x01, 0x3D]):
    for y in range(170, 220):
        for x in range(i * 32, i * 32 + 32):
            inner = 8 <= x - i * 32 < 24 and 180 <= y < 210
            px[x, y] = (nes[index] if inner else (124, 124, 124)) + (255,)
img.save(sys.argv[2] + '.png')
open(sys.argv[2] + '.rgba', 'wb').write(img.tobytes())
