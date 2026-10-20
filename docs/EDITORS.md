# In video editors and compositors

The renderer runs as an effect over a clip in two kinds of programs:

- **OpenFX** hosts, such as DaVinci Resolve, Natron and Nuke: the `CRTSim.ofx.bundle` effect.
- **frei0r** hosts, such as Kdenlive, MLT's `melt` and FFmpeg: the `crtsim` filter, which
  Kdenlive shows through the description in `kdenlive/crtsim.xml`.

Each draws every frame through the CRT at the clip's own size, with a look from the app's
presets or a look the app saved, and carries the glow from frame to frame as the app does for a
video. They render on the computer's graphics card through Vulkan, DX12 or Metal, as the app
does; one renderer serves every instance of the effect in a program.

The packages are on the [releases page](https://github.com/aizumanga/CRTSim-Renderer/releases),
one per system: `CRTSim-Renderer-<version>-editors-<system>.zip`.

## Installing

### OpenFX: DaVinci Resolve, Natron, Nuke

Copy the `CRTSim.ofx.bundle` folder, as it is, into the folder OpenFX hosts look in, and start
the program again:

| System | Folder |
| --- | --- |
| Linux | `/usr/OFX/Plugins` |
| Windows | `C:\Program Files\Common Files\OFX\Plugins` |
| macOS | `/Library/OFX/Plugins` |

Natron and Nuke also look in the folders the `OFX_PLUGIN_PATH` environment variable names. The
effect is called **CRTSim Renderer**, in the **CRTSim** group: in Resolve among the OpenFX
filters of the Effects library, in Natron's CRTSim menu, and by name in Nuke's node search.

The macOS package is built for Apple Silicon and signed ad hoc, as the app is.

### frei0r: Kdenlive

1. Copy the filter where MLT, the engine Kdenlive renders with, finds frei0r filters:
   - Linux: `crtsim.so` into `~/.frei0r-1/lib` (make the folder if it is not there).
   - Windows: `crtsim.dll` into the `frei0r-1` folder of Kdenlive's installation, beside
     Kdenlive's own frei0r filters.
   - On any system, a folder named by the `FREI0R_PATH` environment variable is searched
     instead of the usual ones.
2. Copy `kdenlive/crtsim.xml` into Kdenlive's effects folder: `~/.local/share/kdenlive/effects`
   on Linux, `%LOCALAPPDATA%\kdenlive\effects` on Windows, and
   `~/Library/Application Support/kdenlive/effects` on macOS.
3. Start Kdenlive again. **CRTSim Renderer** is in the effects list; search for it by name.

Without the description, Kdenlive does not list the filter; `melt` and other MLT programs run
it as `frei0r.crtsim` all the same.

### frei0r: FFmpeg and melt

Point `FREI0R_PATH` at the folder holding the filter, and name its parameters in order,
separated by `|` for FFmpeg:

```sh
FREI0R_PATH=~/crtsim/frei0r ffmpeg -i clip.mp4 -vf "frei0r=crtsim:NTSC 480i||16x9" crt.mp4
FREI0R_PATH=~/crtsim/frei0r melt clip.mp4 -filter frei0r.crtsim "Look=NTSC 480i" Tube=16:9 \
  -consumer avformat:crt.mp4
```

FFmpeg takes a colon as the end of an option, so the tube's shape can be written `16x9` or
`16/9` there. On macOS FFmpeg looks for `crtsim.dylib`: copy `crtsim.so` to that name.

For a whole video, the command line's own `crtsim render` does the same work with no editor,
and knows the frame rate from the start.

## Parameters

| Parameter | What it does |
| --- | --- |
| Look | One of the app's presets, `General image` by default. |
| Look file | A preset the app saved (**Save preset…**), or a look the command line wrote (`crtsim config`), used in place of Look. |
| Tube | The picture tube's shape: the look's own, or 4:3, 16:10 or 16:9. |
| Timing | Glow carried between frames with the composite artifacts still, the app's default for video; or alternating each frame, as an NTSC set's did at 60 a second; or each frame on its own. |
| Scene-linear input | OpenFX only: whether the clip's values are scene-linear, as a compositor's are. The effect then encodes them as sRGB for the CRT and decodes its picture again. On by default in Natron and Nuke, off in Resolve and other editors, whose timelines hold gamma-encoded video. |

The look is drawn at the clip's own size, whatever size the look was saved with. Its output is
opaque: the bezel and the dark around the tube are part of the picture.

## How frames follow each other

The CRT's phosphors glow on after each frame, so a frame's picture depends on the frames before
it.

**OpenFX.** Editors render frames in any order: while someone scrubs, from a cache, or split
between machines. A frame that follows the last one the effect rendered goes on with its glow.
Any other frame asks the host for the frames before it, as many as its glow needs to fade below
half an 8-bit step, at most 60, and renders those first. Either way the frame comes out the
same, to within one step, so a jump costs those frames' time and nothing else. Alternating
artifacts keep in step with the clip's first frame.

**frei0r.** A frei0r filter is handed one frame at a time and cannot ask for others. A frame
that follows the last goes on with its glow, which is how an export runs; a frame after a jump
starts afresh, warmed up as a still is, so scrubbed previews can show less glow than the export.
The filter learns the frame rate from the gap between the first two frames, and draws the first
as if at 30 a second.

## When it does not work

- **"CRTSim Renderer could not start"** means the effect found no graphics device it can use:
  it needs a Vulkan driver on Linux, Vulkan or DX12 on Windows, or Metal on macOS. The rest of
  the message says what the graphics library found. A frei0r host passes the frame through
  unchanged and prints the reason once on its error output; an OpenFX host shows it as the
  effect's error.
- **Natron on Linux** carries an older C++ library than current graphics drivers need, and the
  effect then finds no device. Start Natron with the system's library in its place:

  ```sh
  LD_PRELOAD=/usr/lib/x86_64-linux-gnu/libstdc++.so.6 ./Natron
  ```

  or delete `lib/libstdc++.so.6` from Natron's folder.
- **A frame is too large** when it is larger than the graphics device's largest texture,
  16384 pixels on a side on most current cards.

## How it is checked

`tools/check_editors.py` renders a clip through the frei0r filter in FFmpeg and in `melt`, and
through the OpenFX effect in Natron 2.3.14, on Linux with software Vulkan, and compares every
frame with the command line's render of the same clip. Natron's frames match it to within one
step, including a frame rendered on its own; the frei0r filter's, after its first two, to within
four. CI runs it on every change to the effects (`.github/workflows/editors.yml`). A unit test
keeps Kdenlive's description in step with the filter's parameters and the presets.

DaVinci Resolve and Nuke are not run in CI. They load the same effect through the same OpenFX
interface Natron does; reports of how it behaves in them are welcome.

## Building

```sh
cargo build --release -p crtsim-ofx -p crtsim-frei0r
```

The OpenFX effect is `target/release/libcrtsim_ofx.so` (`crtsim_ofx.dll` on Windows,
`libcrtsim_ofx.dylib` on macOS), renamed to `CRTSim.ofx` inside the bundle:

```
CRTSim.ofx.bundle/Contents/Linux-x86-64/CRTSim.ofx
CRTSim.ofx.bundle/Contents/Win64/CRTSim.ofx
CRTSim.ofx.bundle/Contents/MacOS/CRTSim.ofx
```

The frei0r filter is `libcrtsim_frei0r.so`, renamed `crtsim.so` (`crtsim.dll` on Windows).
`python tools/package.py --platform editors-<system> --version <version>` lays both out as the
release does.
