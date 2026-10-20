# In Aseprite

The CRTSim Renderer extension for [Aseprite](https://www.aseprite.org/) draws the sprite's
frame, or its animation, through the CRT, and opens what it makes as a new sprite: a picture
from a frame, an animation from all the frames or a tag's. It runs the `crtsim` command line
to do it, so it needs CRTSim Renderer's package for the system too.

## Installing

1. Get the package for the system from the
   [releases page](https://github.com/aizumanga/CRTSim-Renderer/releases) and unpack it. Its
   `crtsim` (`crtsim.exe` on Windows) is the command line the extension runs. The Linux
   AppImage will do as well: the extension runs its command line.
2. From the same release, get `CRTSim-Renderer-<version>.aseprite-extension` and open it with
   Aseprite, or add it in **Edit → Preferences → Extensions → Add Extension**.

It needs Aseprite 1.3 or later.

## Using it

**File → Export → CRTSim Renderer...**, beside Export Sprite Sheet, opens its dialog:

| Field | What it does |
| --- | --- |
| Render | This frame; or, for a sprite of more than one frame, all its frames, or a tag's. |
| Look | One of the app's included looks. Pixel art 240p to start with. |
| Look file | A preset the desktop app saved (**Save preset…**), used in place of Look. |
| Size | A frame's picture: 1080p, 720p, 1440p, 4k or the reference 1600x900. |
| Longest side | An animation's longest side, in pixels. |
| Open it in Aseprite | Opens the picture, or the animation, as a new sprite. |
| Save as | Saves it as well: a PNG, or an animation as GIF, WebP, animated PNG, or a video (`.mp4`, `.mkv`, `.webm`), which needs FFmpeg. |
| crtsim program | Where `crtsim` is. Empty, it is looked for by name, which a program started from the desktop often cannot do on macOS and Linux: choose the file. |

The dialog remembers its choices. Aseprite waits while `crtsim` renders; an animation of many
frames takes a while. Pictures and animations need no FFmpeg; only a video does.

An animation is drawn at enough frames a second for its shortest frame, at most 60 (30 for a
GIF), so each frame shows for as long as it does in Aseprite, to within a frame. A new sprite
opened from the CRT's picture is named after the sprite, with `-crt`, for when it is saved.

The first time, Aseprite asks whether the extension may run a command, write files and read
them: it writes the frame, or the sprite sheet, to a temporary folder, runs `crtsim` on it, and
reads what `crtsim` said if it failed. Tick **Don't show this specific alert again for this
script** to be asked once for each.

## How it hands the sprite over

A frame goes to `crtsim` as a PNG of the sprite as it shows, with every layer that is
visible. An animation goes as a sprite sheet with its JSON, made by Aseprite's own sprite-sheet
export, which keeps every pixel and each frame's duration; the extension's export leaves the
sprite's own sprite-sheet settings as they were.

The command line reads such a sheet itself, so one exported by hand renders the same way:
**File → Export Sprite Sheet**, with **JSON Data** ticked, as an array or a hash, and no
rotation:

```sh
crtsim render -i walk.json -o walk-crt.gif --preset "Pixel art 240p" --seconds 0
```

TexturePacker's JSON reads too; its frames, which carry no durations, show for 100 ms each.
A sheet's picture can be up to 16384 pixels on a side and 64 megapixels, and every frame must
be the size of the first.

## How it is checked

`extensions/aseprite/tests/run.lua` runs the extension in Lua 5.4, the version Aseprite runs,
against a stand-in for the parts of Aseprite's scripting API it uses. Given `crtsim`, it renders
a frame, an animation and a tag through it from a folder whose name needs quoting, checks the
dialog's answers reach the render and checks the extension's looks are the command line's:

```sh
lua5.4 extensions/aseprite/tests/run.lua target/debug/crtsim
```

CI runs it on software Vulkan (`.github/workflows/editors.yml`). Aseprite itself is not run
in CI; the calls the extension makes were checked against Aseprite's source.
