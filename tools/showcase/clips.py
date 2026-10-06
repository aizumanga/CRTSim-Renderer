#!/usr/bin/env python3
# Renders each animation in CLIPS.json through the CRT and encodes it as a 4:3 clip with its music.
# Usage: clips.py WORKDIR CLIPS.json [name...]
# Each clip: {"name": .., "base": "default"|"general", "config": {..}, "mask_signal": bool, ...}
import json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
S, spec = sys.argv[1], json.load(open(sys.argv[2]))
only = set(sys.argv[3:])
SIZE = "1440x1080"
for clip in spec["clips"]:
    name = clip["name"]
    if only and name not in only:
        continue
    frames, out = f"{S}/frames/{name}", f"{S}/clips/{name}"
    if not os.path.exists(f"{frames}/00239.png"):
        subprocess.run([sys.executable, f"{HERE}/animations/{name}.py", frames], check=True)
    if not os.path.exists(f"{out}/00239.png"):
        shot = {"base": clip.get("base", "default"), "config": dict(clip.get("config", {}), output=SIZE),
                "mask_signal": clip.get("mask_signal", False), "source": {"dir": frames},
                "frames": 240, "fps": 30, "timing": "stable", "out": out}
        os.makedirs(f"{S}/shots", exist_ok=True)
        json.dump(shot, open(f"{S}/shots/clip_{name}.json", "w"), indent=1)
        subprocess.run([f"{ROOT}/target/release/shotgen", f"{S}/shots/clip_{name}.json"], check=True)
    subprocess.run([sys.executable, f"{HERE}/musicgen.py", clip.get("music", name), "8", f"{S}/clips/{name}.wav"], check=True)
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-framerate", "30",
                    "-i", f"{out}/%05d.png", "-i", f"{S}/clips/{name}.wav",
                    "-c:v", "libx264", "-preset", "slow", "-crf", "17", "-pix_fmt", "yuv420p",
                    "-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709",
                    "-c:a", "aac", "-b:a", "192k", "-shortest", "-movflags", "+faststart",
                    f"{S}/crtsim-{name}.mp4"], check=True)
    print("clip", name, "done", flush=True)
