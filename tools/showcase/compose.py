# Composites the rendered CRT shots into a vertical 1080x1920 social video and pipes it to ffmpeg.
import math, os, subprocess, sys
from functools import lru_cache
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

S = sys.argv[1]
OUT = sys.argv[2]
LIMIT = int(sys.argv[3]) if len(sys.argv) > 3 else None   # render only some frames (preview)
STILLS = sys.argv[4:] if len(sys.argv) > 4 else None       # frame numbers to save as PNG instead
R = f"{S}/renders"
W, H, FPS, TOTAL = 1080, 1920, 30, 1020
BOX = (0, 590, 1080, 810)   # x, y, w, h of the CRT picture
INTER = "/usr/share/fonts/opentype/inter/"

def font(name, size): return ImageFont.truetype(INTER + name, size)

def ease_out(t): t = min(max(t, 0), 1); return 1 - (1 - t) ** 3
def smooth(t): t = min(max(t, 0), 1); return t * t * (3 - 2 * t)
def keyed(keys, i):
    """Smooth interpolation through (frame, value...) keyframes."""
    if i <= keys[0][0]: return keys[0][1:]
    for a, b in zip(keys, keys[1:]):
        if a[0] <= i <= b[0]:
            k = smooth((i - a[0]) / (b[0] - a[0]))
            return tuple(x + (y - x) * k for x, y in zip(a[1:], b[1:]))
    return keys[-1][1:]

@lru_cache(maxsize=64)
def load(path, size=None):
    im = Image.open(path).convert("RGB")
    return im.resize(size, Image.LANCZOS) if size else im

def frame_path(shot, i): return f"{R}/{shot}/{i:05}.png"

# ---------- CRT content per segment ----------
MONTAGE = [("m1_original", "Original CRTSim"), ("m2_superwin", "Super Win the Game"),
           ("m3_soft", "Soft television"), ("m4_ntsc240", "NTSC 240p"),
           ("m5_pal576i", "PAL 576i"), ("m6_warm", "Warm analog"),
           ("m7_clean", "Clean RGB"), ("m8_linear", "Linear light")]
SEGMENTS = [(0, "title"), (120, "compare"), (300, "macro"), (420, "montage"),
            (660, "pullback"), (780, "ui"), (900, "outro"), (TOTAL, None)]

def segment(f):
    for (a, name), (b, _) in zip(SEGMENTS, SEGMENTS[1:]):
        if a <= f < b: return name, f - a, b - a
    return "outro", f - 900, 120

COMPARE_KEYS = [(0, 0.0), (36, 0.88), (70, 0.16), (104, 0.72), (140, 0.5), (179, 0.5)]
MACRO_KEYS = [(0, 1440, 1080, 1.0), (22, 1440, 1080, 1.0), (40, 640, 150, 3.4), (62, 700, 160, 3.2),
              (86, 800, 1050, 3.0), (119, 820, 1070, 2.7)]

def ui_index(i):
    return 150 + 2 * i if i < 60 else 390 + 8 * (i - 60)

def crt_content(f):
    name, i, n = segment(f)
    size = BOX[2:]
    if name in ("title", "outro", "pullback"):
        return load(frame_path(name, i)), name
    if name == "compare":
        crt = np.asarray(load(frame_path("compare", i)))
        raw = np.asarray(load(frame_path("compare_raw", i)).resize(size, Image.NEAREST))
        x = int(keyed(COMPARE_KEYS, i)[0] * size[0])
        out = crt.copy(); out[:, :x] = raw[:, :x]
        return Image.fromarray(out), ("compare", x)
    if name == "macro":
        cx, cy, z = keyed(MACRO_KEYS, i)
        im = load(frame_path("macro", i))
        w, h = 2880 / z, 2160 / z
        x0 = min(max(cx - w / 2, 0), 2880 - w); y0 = min(max(cy - h / 2, 0), 2160 - h)
        return im.resize(size, Image.BICUBIC if z > 2.67 else Image.LANCZOS,
                         box=(x0, y0, x0 + w, y0 + h)), name
    if name == "montage":
        shot = MONTAGE[i // 30][0]
        return load(frame_path(shot, i % 30)), name
    if name == "ui":
        im = load(f"{S}/uiframes/{ui_index(i) + 1:04}.png")
        if i < 60:  # close on the preview while the Compare divider is dragged
            return im.resize((1060, 703), Image.LANCZOS, box=(300, 30, 1280, 680)), name
        return im.resize((1060, 704), Image.LANCZOS), name
    raise ValueError(name)

# ---------- text ----------
def glow_text_layer(lines, size=124, colors=((255, 255, 255), (255, 225, 77)), glow=((255, 60, 170), (255, 60, 170))):
    """Big headline lines as separate RGBA layers (text + soft neon glow)."""
    layers = []
    for k, line in enumerate(lines):
        s = size
        while font("InterDisplay-Black.otf", s).getlength(line) > 980: s -= 4
        f = font("InterDisplay-Black.otf", s)
        w = int(f.getlength(line)) + 120; h = s + 90
        txt = Image.new("RGBA", (w, h)); d = ImageDraw.Draw(txt)
        d.text((w / 2, h / 2), line, font=f, fill=colors[k % 2] + (255,), anchor="mm")
        g = Image.new("RGBA", (w, h)); dg = ImageDraw.Draw(g)
        dg.text((w / 2, h / 2), line, font=f, fill=glow[k % 2] + (255,), anchor="mm",
                stroke_width=4, stroke_fill=glow[k % 2] + (255,))
        g = g.filter(ImageFilter.GaussianBlur(18))
        a = np.asarray(g).astype(np.float32); a[..., 3] *= 0.9
        g = Image.fromarray(a.clip(0, 255).astype(np.uint8))
        # hard drop shadow for readability
        sh = Image.new("RGBA", (w, h)); ds = ImageDraw.Draw(sh)
        ds.text((w / 2 + 5, h / 2 + 7), line, font=f, fill=(20, 0, 40, 200), anchor="mm")
        layer = Image.alpha_composite(Image.alpha_composite(g, sh), txt)
        layers.append(layer)
    return layers

def chip(text, accent=(41, 230, 255), size=40, fill=(18, 10, 40, 210)):
    f = font("Inter-Bold.otf", size)
    w = int(f.getlength(text)) + 2 * 34; h = size + 36
    im = Image.new("RGBA", (w + 40, h + 40)); d = ImageDraw.Draw(im)
    gl = Image.new("RGBA", im.size); dg = ImageDraw.Draw(gl)
    dg.rounded_rectangle((20, 20, 20 + w, 20 + h), h // 2, outline=accent + (255,), width=6)
    gl = gl.filter(ImageFilter.GaussianBlur(9))
    im = Image.alpha_composite(im, gl); d = ImageDraw.Draw(im)
    d.rounded_rectangle((20, 20, 20 + w, 20 + h), h // 2, fill=fill, outline=accent + (255,), width=3)
    d.text((20 + w / 2, 20 + h / 2), text, font=f, fill=(255, 255, 255, 255), anchor="mm")
    return im

def small_text(text, size=34, color=(220, 210, 255), fnt="Inter-SemiBold.otf"):
    f = font(fnt, size)
    w = int(f.getlength(text)) + 20; h = size + 24
    im = Image.new("RGBA", (w, h)); d = ImageDraw.Draw(im)
    d.text((w / 2 + 2, h / 2 + 3), text, font=f, fill=(0, 0, 0, 180), anchor="mm")
    d.text((w / 2, h / 2), text, font=f, fill=color + (255,), anchor="mm")
    return im

PINK, CYAN, YELLOW, GREEN, ORANGE = (255, 60, 170), (41, 230, 255), (255, 225, 77), (0, 228, 120), (255, 163, 0)
HEADLINES = {
    "title":    (["YOUR PIXELS", "DESERVE A CRT"], 18),
    "compare":  (["FLAT PIXELS", "→ GLOWING TUBE"], 0),
    "macro":    (["EVERY DETAIL", "SIMULATED"], 0),
    "montage":  (["12 BUILT-IN", "CRT LOOKS"], 0),
    "pullback": (["REAL CURVED", "GLASS"], 0),
    "ui":       (["RUNS ON YOUR", "DESKTOP & WEB"], 0),
    "outro":    (["FREE &", "OPEN SOURCE"], 0),
}
HEAD_LAYERS = {k: glow_text_layer(v[0]) for k, v in HEADLINES.items()}

CHIPS = {  # segment -> list of (text, accent, frame offset, (cx, cy))
    "title":    [("Images  ·  Video  ·  GIF", CYAN, 45, (540, 1530))],
    "macro":    [("Shadow mask", CYAN, 22, (540, 1490)), ("Composite artifacts", PINK, 37, (540, 1590)),
                 ("Phosphor persistence & bloom", YELLOW, 52, (540, 1690))],
    "pullback": [("Ray-traced screen", CYAN, 10, (540, 1490)), ("Original bezel", PINK, 30, (540, 1590)),
                 ("Moving reflections", YELLOW, 50, (540, 1690))],
    "ui":       [("Windows · Linux · macOS", CYAN, 8, (540, 1480)), ("WebGPU in your browser", PINK, 24, (540, 1580)),
                 ("RetroArch shader presets", YELLOW, 40, (540, 1680))],
}
CHIP_LAYERS = {k: [(chip(t, a), o, c) for t, a, o, c in v] for k, v in CHIPS.items()}
PRESET_CHIPS = [chip(name, [CYAN, PINK, YELLOW, GREEN, ORANGE, PINK, CYAN, YELLOW][k], size=52)
                for k, (_, name) in enumerate(MONTAGE)]
COUNTERS = [small_text(f"PRESET {k + 1:02} / {len(MONTAGE):02}", 30, (190, 170, 255), "Inter-Bold.otf")
            for k in range(len(MONTAGE))]
SUBS = {
    "title":   small_text("A native CRT renderer for images & video", 36),
    "compare": small_text("Before / after — the same Compare view the app has", 32),
    "outro":   None,
}
BRAND = small_text("C R T S I M  —  R E N D E R E R", 28, (180, 160, 240), "Inter-Bold.otf")
URL = chip("github.com/aizumanga/CRTSim-Renderer", YELLOW, size=38)
CREDIT1 = small_text("Built on J. Kyle Pittman's public CRTSim (CC0)", 30, (210, 200, 240))
CREDIT2 = small_text("Unofficial · Images · Video · GIF · RetroArch", 30, (170, 160, 210))
TAG_RAW = chip("RAW", (200, 200, 200), size=34)
TAG_CRT = chip("CRTSIM", PINK, size=34)

def paste_pop(canvas, layer, cx, cy, t, dur=8):
    """Paste `layer` centred at cx, cy, popping in t frames after its cue."""
    if t < 0: return
    k = ease_out(t / dur)
    s = 1.35 - 0.35 * k
    a = min(1, t / 4)
    im = layer
    if abs(s - 1) > 1e-3:
        im = layer.resize((max(1, int(layer.width * s)), max(1, int(layer.height * s))), Image.BICUBIC)
    if a < 1:
        arr = np.asarray(im).copy(); arr[..., 3] = (arr[..., 3] * a).astype(np.uint8); im = Image.fromarray(arr)
    canvas.alpha_composite(im, (int(cx - im.width / 2), int(cy - im.height / 2)))

# ---------- background ----------
yy = np.linspace(0, 1, H)[:, None, None]
BASE = (np.array([14, 6, 32]) * (1 - yy) + np.array([4, 2, 12]) * yy) * np.ones((1, W, 1))
SCAN = np.ones((H, 1, 1)); SCAN[::4] = 0.8
gx, gy = np.meshgrid(np.linspace(-1, 1, W), np.linspace(-1, 1, H))
VIG = (1 - 0.55 * np.clip(gx ** 2 * 0.6 + (gy * 0.9) ** 2, 0, 1))[..., None]

def background(content, strength):
    small = content.resize((27, 20), Image.BILINEAR).filter(ImageFilter.GaussianBlur(2))
    amb = np.asarray(small.resize((W, H), Image.BICUBIC)).astype(np.float32)
    # ambient light strongest around the tube
    dist = np.abs(np.linspace(0, 1, H) - (BOX[1] + BOX[3] / 2) / H)[:, None, None]
    falloff = np.exp(-(dist / 0.28) ** 2)
    bg = BASE + amb * (0.55 * falloff + 0.12) * strength
    return bg * SCAN * VIG

# ---------- per-frame ----------
CUTS = {120, 300, 420, 660, 780, 900} | {420 + 30 * k for k in range(1, 8)}

def power_scale(f):
    """(vertical, horizontal, brightness boost) of the tube's picture while switching on or off."""
    if f < 4: return 0.004, ease_out((f + 1) / 4), 1.0
    if f < 14: v = ease_out((f - 3) / 10); return max(v, 0.004), 1, 1 - v
    off = f - 1000
    if off >= 0:
        if off < 8: v = 1 - smooth((off + 1) / 8); return max(v, 0.004), 1, 1 - v
        if off < 15: return 0.004, max(1 - smooth((off - 7) / 7), 0.01), 1
        return 0.004, 0.01, 1
    return 1, 1, 0

def render(f):
    name, i, n = segment(f)
    content, info = crt_content(f)
    v, h, boost = power_scale(f)
    on = f >= 4 and f < 1015
    amb_strength = min(1, max(0, (f - 6) / 10)) if f < 30 else (1 if f < 1000 else max(0, 1 - (f - 1000) / 8))
    frame = Image.fromarray(background(content, amb_strength).clip(0, 255).astype(np.uint8)).convert("RGBA")
    # tube picture, with punch-in on cuts and power on/off squash
    pic = content
    since_cut = min([f - c for c in CUTS if f >= c] or [99])
    punch = 1 + 0.05 * (1 - ease_out(since_cut / 7)) if since_cut < 7 else 1
    bx, by, bw, bh = BOX
    if name == "ui":
        # app window floating on the ambient glow
        shadow = Image.new("RGBA", (W, H)); ds = ImageDraw.Draw(shadow)
        ux, uy = (W - pic.width) // 2, by + (bh - pic.height) // 2
        ds.rounded_rectangle((ux - 6, uy - 6, ux + pic.width + 6, uy + pic.height + 6), 18, fill=(255, 60, 170, 160))
        frame.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(26)))
        if punch != 1:
            pic = pic.resize((int(pic.width * punch), int(pic.height * punch)), Image.BICUBIC)
        mask = Image.new("L", pic.size, 0); ImageDraw.Draw(mask).rounded_rectangle((0, 0, *pic.size), 14, fill=255)
        frame.paste(pic, ((W - pic.width) // 2, by + (bh - pic.height) // 2), mask)
    else:
        if punch != 1:
            cw, ch = bw / punch, bh / punch
            pic = pic.resize((bw, bh), Image.BICUBIC, box=((bw - cw) / 2, (bh - ch) / 2, (bw + cw) / 2, (bh + ch) / 2))
        if (v, h) != (1, 1):
            arr = np.asarray(pic).astype(np.float32)
            arr = arr + (255 - arr) * boost * 0.85 if boost > 0 else arr
            if f < 4 or f >= 1007: arr[:] = 255  # bright scan line / dot
            pw, ph = max(2, int(bw * h)), max(3, int(bh * v))
            pic = Image.fromarray(arr.clip(0, 255).astype(np.uint8)).resize((pw, ph), Image.BILINEAR)
            tube = Image.new("RGB", (bw, bh), (0, 0, 0))
            tube.paste(pic, ((bw - pw) // 2, (bh - ph) // 2))
            if f >= 1015: tube = Image.new("RGB", (bw, bh));
            glow = tube.filter(ImageFilter.GaussianBlur(12))
            tube = Image.fromarray(np.minimum(255, np.asarray(tube).astype(np.int32) + np.asarray(glow).astype(np.int32) * 2).astype(np.uint8))
            pic = tube
        frame.paste(pic, (bx, by))
    # flash on cuts
    if since_cut < 5 and f > 30:
        a = 0.45 * (1 - since_cut / 5)
        arr = np.asarray(frame).astype(np.float32)
        arr[by:by + bh, :, :3] += (255 - arr[by:by + bh, :, :3]) * a
        frame = Image.fromarray(arr.astype(np.uint8))
    # headline
    lines, cue = HEADLINES[name]
    if f < 1004:
        for k, layer in enumerate(HEAD_LAYERS[name]):
            paste_pop(frame, layer, W / 2, 300 + k * 140 - (10 if len(lines) == 2 else 0), i - cue - 4 * k)
    paste_pop(frame, BRAND, W / 2, 120, f - 10, 10) if f < 1004 else None
    # segment extras
    if name == "compare" and isinstance(info, tuple):
        x = info[1]
        d = Image.new("RGBA", (W, H)); dd = ImageDraw.Draw(d)
        dd.rectangle((x - 3, by, x + 3, by + bh), fill=(255, 255, 255, 255))
        g = d.filter(ImageFilter.GaussianBlur(10)); frame.alpha_composite(g); frame.alpha_composite(d)
        hd = Image.new("RGBA", (W, H)); dh = ImageDraw.Draw(hd)
        dh.ellipse((x - 34, by + bh / 2 - 34, x + 34, by + bh / 2 + 34), fill=(255, 255, 255, 255))
        dh.text((x, by + bh / 2), "◀ ▶", font=font("Inter-Bold.otf", 22), fill=(30, 10, 60, 255), anchor="mm")
        frame.alpha_composite(hd)
        if x > 170: frame.alpha_composite(TAG_RAW, (20, by + bh + 14))
        if x < 860: frame.alpha_composite(TAG_CRT, (W - TAG_CRT.width - 20, by + bh + 14))
        paste_pop(frame, SUBS["compare"], W / 2, 1590, i - 10)
    if name == "title":
        paste_pop(frame, SUBS["title"], W / 2, 1450, i - 35)
    for layer, off, (cx, cy) in CHIP_LAYERS.get(name, []):
        paste_pop(frame, layer, cx, cy, i - off)
    if name == "montage":
        k = i // 30
        paste_pop(frame, PRESET_CHIPS[k], W / 2, 1520, i % 30, 6)
        paste_pop(frame, COUNTERS[k], W / 2, 1625, i % 30 - 2, 6)
    if name == "outro" and f < 1004:
        paste_pop(frame, URL, W / 2, 1500, i - 12)
        paste_pop(frame, CREDIT1, W / 2, 1610, i - 24)
        paste_pop(frame, CREDIT2, W / 2, 1660, i - 30)
    # chromatic glitch right after cuts
    arr = np.asarray(frame.convert("RGB"))
    if since_cut < 3 and f > 30:
        o = (3 - since_cut) * 6
        arr = arr.copy(); arr[..., 0] = np.roll(arr[..., 0], o, 1); arr[..., 2] = np.roll(arr[..., 2], -o, 1)
    # fade out at the end
    if f >= 1012:
        arr = (arr * max(0, 1 - (f - 1012) / 7)).astype(np.uint8)
    return arr

frames = range(TOTAL if LIMIT is None else LIMIT)
if STILLS:
    for s in STILLS:
        Image.fromarray(render(int(s))).save(f"{OUT}_{int(s):04}.png")
    sys.exit()
cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24",
       "-s", f"{W}x{H}", "-r", str(FPS), "-i", "-", "-i", f"{S}/music.wav",
       "-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-preset", "slow", "-crf", "17",
       "-pix_fmt", "yuv420p", "-profile:v", "high", "-level", "4.2",
       "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
       "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart", OUT]
p = subprocess.Popen(cmd, stdin=subprocess.PIPE)
for f in frames:
    p.stdin.write(render(f).tobytes())
    if f % 60 == 0: print("frame", f, flush=True)
p.stdin.close(); p.wait()
print("done", p.returncode)
