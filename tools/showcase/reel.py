# Edits the animation clips (tools/showcase/animations) into a vertical reel, one CRT look per clip.
# Usage: reel.py WORKDIR CLIPS.json OUT.mp4 [stills frame...]
# CLIPS.json: {"intro": ["LINE 1", "LINE 2"], "outro": [...],
#              "clips": [{"name": "arcade", "start": 60, "headline": ["..", ".."],
#                         "look": "Original CRTSim", "era": "1980s arcade"}, ...]}
import json, subprocess, sys
import numpy as np
from PIL import Image, ImageDraw, ImageFilter
import compose as c

S, CLIPS, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
STILLS = [int(x) for x in sys.argv[4:]]
spec = json.load(open(CLIPS))
SEG = 120                      # frames per segment: 4 s, two bars at 120 BPM
clips = spec["clips"]
TOTAL = SEG * (len(clips) + 2)
OFF = TOTAL - 20               # power-off starts
bx, by, bw, bh = c.BOX
ACCENTS = [c.CYAN, c.PINK, c.YELLOW, c.GREEN, c.ORANGE, c.PINK, c.CYAN, c.YELLOW]

def heads(lines, k):
    return c.glow_text_layer(lines, colors=((255, 255, 255), ACCENTS[k % len(ACCENTS)]),
                             glow=(c.PINK, c.PINK) if k % 2 == 0 else (c.CYAN, c.CYAN))

HEAD = [c.glow_text_layer(spec["intro"])] + [heads(cl["headline"], k + 1) for k, cl in enumerate(clips)] \
    + [c.glow_text_layer(spec["outro"])]
LOOK = [c.chip(cl["look"], ACCENTS[k % len(ACCENTS)], size=50) for k, cl in enumerate(clips)]
ERA = [c.small_text(f'{k + 1:02} / {len(clips):02}  ·  {cl["era"].upper()}', 30, (190, 170, 255), "Inter-Bold.otf")
       for k, cl in enumerate(clips)]
INTRO_SUB = c.small_text(spec.get("intro_sub", "Original animations, rendered through CRTSim"), 34)
URL = c.chip("github.com/aizumanga/CRTSim-Renderer", c.YELLOW, size=38)
CREDIT = c.small_text("Built on J. Kyle Pittman's public CRTSim (CC0) · unofficial", 28, (200, 190, 235))

def picture(f):
    seg, i = divmod(f, SEG)
    if seg == 0:
        return c.load(f"{S}/renders/title/{i:05}.png")
    if seg > len(clips):
        return c.load(f"{S}/renders/outro/{i:05}.png")
    cl = clips[seg - 1]
    return c.load(f"{S}/clips/{cl['name']}/{cl['start'] + i:05}.png", (bw, bh))

def tube(f, pic):
    """The picture with the set switching on (first frames) or off (last frames)."""
    if 14 <= f < OFF:
        return pic
    if f < 4:
        v, h, boost, line = 0.004, c.ease_out((f + 1) / 4), 1, True
    elif f < 14:
        v = max(c.ease_out((f - 3) / 10), 0.004); h, boost, line = 1, 1 - v, False
    else:
        o = f - OFF
        if o < 8: v = max(1 - c.smooth((o + 1) / 8), 0.004); h, boost, line = 1, 1 - v, False
        elif o < 15: v, h, boost, line = 0.004, max(1 - c.smooth((o - 7) / 7), 0.01), 1, True
        else: return Image.new("RGB", (bw, bh))
    arr = np.asarray(pic).astype(np.float32)
    arr = arr + (255 - arr) * boost * 0.85
    if line: arr[:] = 255
    pw, ph = max(2, int(bw * h)), max(3, int(bh * v))
    small = Image.fromarray(arr.clip(0, 255).astype(np.uint8)).resize((pw, ph), Image.BILINEAR)
    out = Image.new("RGB", (bw, bh)); out.paste(small, ((bw - pw) // 2, (bh - ph) // 2))
    glow = out.filter(ImageFilter.GaussianBlur(12))
    return Image.fromarray(np.minimum(255, np.asarray(out).astype(np.int32) + np.asarray(glow).astype(np.int32) * 2).astype(np.uint8))

def render(f):
    seg, i = divmod(f, SEG)
    pic = picture(f)
    ambient = min(1, max(0, (f - 6) / 10)) if f < 30 else (1 if f < OFF else max(0, 1 - (f - OFF) / 8))
    frame = Image.fromarray(c.background(pic, ambient).clip(0, 255).astype(np.uint8)).convert("RGBA")
    since_cut = i if seg > 0 else 99
    shown = pic
    if since_cut < 7:
        z = 1 + 0.05 * (1 - c.ease_out(since_cut / 7)); cw, ch = bw / z, bh / z
        shown = pic.resize((bw, bh), Image.BICUBIC, box=((bw - cw) / 2, (bh - ch) / 2, (bw + cw) / 2, (bh + ch) / 2))
    frame.paste(tube(f, shown), (bx, by))
    if since_cut < 5:
        a = 0.45 * (1 - since_cut / 5)
        arr = np.asarray(frame).astype(np.float32)
        arr[by:by + bh, :, :3] += (255 - arr[by:by + bh, :, :3]) * a
        frame = Image.fromarray(arr.astype(np.uint8))
    if f < OFF + 4:
        cue = 18 if seg == 0 else 0
        for k, layer in enumerate(HEAD[seg]):
            c.paste_pop(frame, layer, c.W / 2, 290 + k * 140, i - cue - 4 * k)
        c.paste_pop(frame, c.BRAND, c.W / 2, 120, f - 10, 10)
    if seg == 0:
        c.paste_pop(frame, INTRO_SUB, c.W / 2, 1490, i - 40)
    elif seg <= len(clips):
        c.paste_pop(frame, LOOK[seg - 1], c.W / 2, 1515, i - 3, 6)
        c.paste_pop(frame, ERA[seg - 1], c.W / 2, 1625, i - 6, 6)
    elif f < OFF + 4:
        c.paste_pop(frame, URL, c.W / 2, 1500, i - 12)
        c.paste_pop(frame, CREDIT, c.W / 2, 1610, i - 24)
    arr = np.asarray(frame.convert("RGB"))
    if since_cut < 3:
        o = (3 - since_cut) * 6
        arr = arr.copy(); arr[..., 0] = np.roll(arr[..., 0], o, 1); arr[..., 2] = np.roll(arr[..., 2], -o, 1)
    if f >= TOTAL - 8:
        arr = (arr * max(0, 1 - (f - (TOTAL - 8)) / 7)).astype(np.uint8)
    return arr

if STILLS:
    for f in STILLS:
        Image.fromarray(render(f)).save(f"{OUT}_{f:04}.png")
    sys.exit()
subprocess.run([sys.executable, f"{sys.path[0]}/musicgen.py", "reel", str(TOTAL / c.FPS), f"{S}/reel.wav"], check=True)
cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgb24",
       "-s", f"{c.W}x{c.H}", "-r", str(c.FPS), "-i", "-", "-i", f"{S}/reel.wav",
       "-map", "0:v", "-map", "1:a", "-c:v", "libx264", "-preset", "slow", "-crf", "17",
       "-pix_fmt", "yuv420p", "-profile:v", "high", "-level", "4.2",
       "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
       "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart", OUT]
p = subprocess.Popen(cmd, stdin=subprocess.PIPE)
for f in range(TOTAL):
    p.stdin.write(render(f).tobytes())
    if f % 120 == 0: print("frame", f, flush=True)
p.stdin.close(); p.wait()
print("done", p.returncode)
