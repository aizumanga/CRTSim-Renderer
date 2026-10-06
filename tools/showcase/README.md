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

## Animations and the reel

`animations/` holds six more original animations, each 240 frames (8 s at 30 fps) at its own
resolution, made to be seen through the CRT. Each script runs as `animations/NAME.py OUT_DIR`.

| Animation | Size | Look in `clips.json` |
|---|---|---|
| `arcade`: a space shooter's attract mode | 256×224 | Original CRTSim with longer persistence |
| `terminal`: a green-phosphor terminal booting and running a program | 640×480 | Monochrome, no artifacts, long glow |
| `demoscene`: copper bars, plasma, a 3D solid and a sine scroller | 320×240 | PAL 288p |
| `rpg`: a 16-bit overworld and a dialog window | 256×224 | Super Win the Game |
| `racer`: a pseudo-3D road at dusk | 320×224 | NTSC 240p |
| `broadcast`: snow, a test card, a station ident and a TV's on-screen display | 640×480 | NTSC 480i |

`clips.py` renders each through the CRT as a 1440×1080 clip with music from `musicgen.py`, which
synthesizes a chiptune in a mood per clip. `reel.py` edits four seconds of each into a vertical reel,
between the title and outro shots that `make.sh` renders.

```sh
tools/showcase/clips.py /tmp/showcase tools/showcase/clips.json     # writes crtsim-NAME.mp4
tools/showcase/reel.py /tmp/showcase tools/showcase/clips.json /tmp/showcase/crtsim-reel.mp4
```

