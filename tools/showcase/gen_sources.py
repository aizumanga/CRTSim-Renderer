import os, sys, math
import numpy as np
from PIL import Image, ImageDraw, ImageFont
S = sys.argv[1]
W, H = 256, 224
BOLD = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono-Bold.ttf"
BAYER = np.array([[0,8,2,10],[12,4,14,6],[3,11,1,9],[15,7,13,5]])

def font(path, size):
    return ImageFont.truetype(path, size)

def sky(top, bot, h=H):
    # dithered vertical gradient in 4 steps
    img = np.zeros((h, W, 3), np.uint8)
    ys, xs = np.mgrid[0:h, 0:W]
    t = ys / (h - 1) * 6
    lvl = np.floor(t).astype(int)
    frac = ((t - lvl) * 16).astype(int)
    lvl = lvl + (BAYER[ys % 4, xs % 4] < frac)
    cols = [tuple(int(top[c] + (bot[c] - top[c]) * k / 6) for c in range(3)) for k in range(8)]
    for k in range(8):
        img[lvl == k] = cols[min(k, 7)]
    return img

def text(d, xy, s, f, fill, shadow=(0,0,0), anchor="mm", outline=None):
    x, y = xy
    if outline:
        for dx in (-1,0,1):
            for dy in (-1,0,1):
                d.text((x+dx, y+dy), s, font=f, fill=outline, anchor=anchor)
    if shadow:
        d.text((x+2, y+2), s, font=f, fill=shadow, anchor=anchor)
    d.text((x, y), s, font=f, fill=fill, anchor=anchor)

def stars(d, frame, seed=3, n=40, h=H):
    rng = np.random.default_rng(seed)
    for i in range(n):
        x, y = rng.integers(0, W), rng.integers(0, h)
        tw = (frame // 6 + i) % 5
        c = (255,255,255) if tw == 0 else (120,140,200)
        d.point((int(x), int(y)), fill=c)

def rainbow(d, y, frame, h=6):
    bars = [(255,0,77),(255,163,0),(255,236,39),(0,228,54),(41,173,255),(131,118,156),(255,119,168)]
    for x in range(W):
        c = bars[((x + frame * 2) // 12) % len(bars)]
        d.line([(x, y), (x, y + h - 1)], fill=c)

def title(frame, out):
    img = Image.fromarray(sky((10,10,40), (60,20,90)))
    d = ImageDraw.Draw(img)
    d.fontmode = "1"
    stars(d, frame)
    rainbow(d, 30, frame, 5)
    rainbow(d, 190, -frame, 5)
    big = font(BOLD, 44)
    # drop in letter by letter
    word = "CRTSIM"
    ws = [big.getlength(c) for c in word]
    gap = 3
    x = 128 - (sum(ws) + gap * (len(ws) - 1)) / 2
    xs = []
    for w_ in ws:
        xs.append(x + w_ / 2); x += w_ + gap
    for i, ch in enumerate(word):
        t = frame - 6 - i * 3
        if t < 0: continue
        y = 82 - max(0, 30 - t * 6)
        bounce = int(3 * math.sin(frame * 0.25 + i)) if frame > 50 else 0
        text(d, (xs[i], y + bounce), ch, big, (255, 236, 39), shadow=(180, 40, 90), outline=(40, 0, 60))
    if frame >= 30:
        med = font(BOLD, 20)
        txt = "RENDERER"[: max(0, (frame - 30) // 2)]
        text(d, (128, 128), txt, med, (41, 230, 255), shadow=(20, 40, 120), anchor="mm")
    if frame >= 55 and (frame // 8) % 2 == 0:
        sm = font(MONO, 10)
        text(d, (128, 162), "PRESS START", sm, (255, 255, 255), shadow=(60,60,60))
    img.save(out)

def outro(frame, out):
    img = Image.fromarray(sky((5,5,25), (40,10,70)))
    d = ImageDraw.Draw(img)
    d.fontmode = "1"
    stars(d, frame, seed=9)
    rainbow(d, 22, frame, 4)
    big = font(BOLD, 30)
    text(d, (128, 58), "CRTSIM", big, (255,236,39), shadow=(180,40,90), outline=(40,0,60))
    text(d, (128, 90), "RENDERER", font(BOLD, 18), (41,230,255), shadow=(20,40,120))
    sm = font(MONO, 11)
    lines = ["IMAGES  VIDEO  GIF", "DESKTOP  WEB  RETROARCH", "FREE & OPEN - CC0"]
    for i, l in enumerate(lines):
        if frame >= 8 + i * 8:
            col = [(255,255,255),(255,163,0),(0,228,54)][i]
            text(d, (128, 124 + i * 16), l, sm, col, shadow=(40,40,40))
    if frame >= 40 and (frame // 8) % 2 == 0:
        text(d, (128, 186), "* INSERT PIXELS *", sm, (255,119,168), shadow=(60,0,40))
    rainbow(d, 200, -frame, 4)
    img.save(out)

def synthwave(frame, out, w=640, h=480):
    t = frame / 30.0
    img = Image.new("RGB", (w, h))
    a = np.zeros((h, w, 3), np.float32)
    ys = np.linspace(0, 1, h)[:, None]
    hor = int(h * 0.58)
    top = np.array([20, 5, 60.]); mid = np.array([255, 60, 140.]); 
    k = np.clip(ys / 0.58, 0, 1)
    skyc = top * (1 - k[..., None]) + mid * (k[..., None] ** 2)
    a[:] = skyc[:, :, :] if skyc.shape[1] == w else np.repeat(skyc, w, axis=1)
    img = Image.fromarray(a.clip(0,255).astype(np.uint8))
    d = ImageDraw.Draw(img)
    # stars
    rng = np.random.default_rng(1)
    for i in range(120):
        x, y = rng.integers(0, w), rng.integers(0, int(hor*0.6))
        b = 150 + int(100 * math.sin(t * 4 + i))
        d.point((int(x), int(y)), fill=(b, b, 255))
    # sun with stripes
    cx, cy, r = w // 2, hor - 70, 115
    sun = Image.new("L", (w, h), 0); sd = ImageDraw.Draw(sun)
    sd.ellipse([cx - r, cy - r, cx + r, cy + r], fill=255)
    for i in range(7):
        yy = cy - 50 + i * 12 - int((t * 20) % 12)
        thick = 1 + i
        sd.rectangle([0, yy, w, yy + thick], fill=0)
    grad = np.zeros((h, w, 3), np.uint8)
    gy = np.clip((np.arange(h) - (cy - r)) / (2 * r), 0, 1)[:, None]
    grad[:] = (np.array([255, 230, 60]) * (1 - gy) + np.array([255, 40, 150]) * gy).astype(np.uint8)[:, None, :]
    img.paste(Image.fromarray(grad), (0, 0), sun)
    # mountains
    pts = [(0, hor)]
    for x in range(0, w + 20, 20):
        yy = hor - 30 - 40 * abs(math.sin(x * 0.013 + 1.3)) - 25 * abs(math.sin(x * 0.041))
        pts.append((x, int(yy)))
    pts.append((w, hor))
    d.polygon(pts, fill=(40, 10, 70), outline=(0, 230, 255))
    # floor
    d.rectangle([0, hor, w, h], fill=(15, 0, 35))
    for i in range(-14, 15):
        x0 = cx + i * 30; x1 = cx + i * 260
        d.line([(x0, hor), (x1, h)], fill=(255, 40, 200), width=2)
    for j in range(12):
        z = (j + (t * 2.0) % 1.0)
        yy = hor + int((h - hor) * (z / 12) ** 2.2)
        d.line([(0, yy), (w, yy)], fill=(255, 40, 200), width=2)
    d.line([(0, hor), (w, hor)], fill=(0, 230, 255), width=2)
    img.save(out)

for name, fn, n in [("title", title, 120), ("outro", outro, 120), ("synth", synthwave, 90)]:
    os.makedirs(f"{S}/src/{name}", exist_ok=True)
    for i in range(n):
        fn(i, f"{S}/src/{name}/{i:05}.png")
print("ok")
