# Showcase video

Makes a 34-second vertical (1080×1920, 30 fps) video of the renderer for social media. All of it is
made here, and all of it is original: the gameplay is the app's video test card, the title cards and
synthwave scene are drawn by `gen_sources.py`, and the chiptune is synthesized by `music.py`.

| Time | Shows |
|---|---|
| 0–4 s | The tube switching on to a title card rendered through the CRT |
| 4–10 s | Raw pixels against the CRT, with a Compare divider sweeping across |
| 10–14 s | A zoom into a 2880×2160 render: shadow mask, composite artifacts, glow |
| 14–22 s | Eight built-in presets, one per two beats |
| 22–26 s | The glass's curvature and the light moving across it |
| 26–30 s | The desktop app: playback, Compare, and its five themes |
| 30–34 s | Outro card, repository link, credit to J. Kyle Pittman's CRTSim, power-off |

- `shotgen/` renders a shot as a sequence sharing CRT history, through `crtsim-core`, with settings
  that can be animated from frame to frame. It is not a workspace member.
- `mkshots.py` lists the shots, `compose.py` edits them into the video, and `make.sh` runs everything.
- `record_ui.sh` records the desktop app on a virtual X display with `xdotool`. Its click positions
  assume the default window and theme.

```sh
tools/showcase/record_ui.sh /tmp/showcase     # needs Xvfb, xdotool, ffmpeg, a Vulkan driver
tools/showcase/make.sh /tmp/showcase          # writes /tmp/showcase/crtsim-showcase-vertical.mp4
```

The renders take about 15 minutes on lavapipe, the software Vulkan driver, and much less on a GPU.
Python needs Pillow and NumPy. The fonts are Inter and DejaVu, which the scripts load from
`/usr/share/fonts`.
