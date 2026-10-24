# CRTSim-Renderer

A native, local renderer based on [J. Kyle Pittman's public CRTSim implementation](https://github.com/MinorKeyGames/CRTSim).
The desktop renders **images and video with audio**, with portable release builds for Windows, Linux and provisional macOS.
See [portable downloads and setup](docs/RELEASES.md).
The goal is to preserve the shared effect and make it useful for images and video.
It is not an official product or an exact reconstruction of a commercial game's shader.

## What works in this prototype

- PNG/JPEG/WebP/BMP/GIF input and PNG export; an original test card when input is omitted.
- Animated GIF and WebP open as animations, decoded without FFmpeg; videos, GIFs and animated WebPs export to MP4, MKV, WebM, GIF, animated WebP or animated PNG.
- Lossless outputs: animated PNG, written without FFmpeg in the web app, and **Lossless** MKV video in FFV1, both keeping every pixel of the CRT exactly.
- **File → Video test card** opens ten seconds of original side-scrolling pixel-art gameplay at 256x224 and 60 frames per second, drawn by the app, to see a look on moving pixels before exporting it. It opens and plays without a file or FFmpeg, and exports like an opened animation.
- The original curved screen, ray-traced as the sphere its mesh was cut from, with the same UVs and normals, and the mesh's own outline and edge shading baked across its UVs; the original bezel, baked from its mesh with its colors, normals, UVs and reflection weights and traced the same way, so both draw in one full-screen pass.
- Composite artifacts, horizontal ringing, separate RGB persistence, shadow mask, lighting, edge reflections and bloom.
- Deterministic still jobs, cleared feedback buffers, phase A/B/stable/alternating selection and configurable warm-up.
- Logical signal and output resolution presets; custom dimensions; JSON settings.
- Clean/signal/full-CRT debugging and mesh validation/export without a GPU.
- Native wgpu backends: Vulkan on Linux, DX12 on Windows and Metal on macOS.
- A web app on WebGPU that lays itself out for phones, and the CRT in RetroArch, ReShade, OBS Studio, Godot and Bevy, each look exported for them.

Installers and files from Super Win the Game are not included; its NTSC palette is made by the app, as the game makes it. A separate CC0 collection of 38 NES LUTs from MAME Goodies is included under **Color & LUT → LUT gallery…**. Platform compilation does not prove visual parity between drivers.
See [the implementation plan](docs/PLAN.md) and [validation notes](docs/VALIDATION.md).

## Desktop app

On first launch, the app honestly describes this AI-assisted project as “what some would call \"vibe-coded slop\", built based on J. Kyle Pittman's public CRTSim”.
Acknowledging that message is remembered on this computer. **Help → Credits & support** always reopens the attribution and links to
[J. Kyle Pittman's itch.io page](https://piratehearts.itch.io/) and [Minor Key Games on Steam](https://store.steampowered.com/developer/MinorKeyGames).
Please support the original developer by buying and playing their games. This port is unofficial and is not endorsed by him.
The welcome and credits also link to Pittman's
[CRT Simulation in Super Win the Game](https://www.gamedeveloper.com/programming/crt-simulation-in-super-win-the-game),
an interesting technical account of how the effect developed.
Right after the welcome, the first time, the app offers three looks to start from, Super Win the Game, General image and
Soft television, each as a thumbnail of the picture open; choosing one applies it, and closing the window keeps the
look in use.

**Preset gallery** includes Super Win the Game, the preset the app starts with, Super Win the Game NES palette, General image, Soft television, Clean RGB, Pixel art 240p, NTSC 240p, NTSC 480i, Widescreen television, PAL 288p, PAL 576i, Warm analog and Linear light.
The NTSC and PAL presets set the line count, interlacing and composite phase. The artifact model is the original NTSC-derived one, so the PAL presets approximate PAL's line-alternating color with gentler, stable artifacts rather than simulating it. Video frame rate is chosen separately in Export → Video.
Each preset shows a thumbnail rendered from your current image. Point at a preset to preview it on the full image without applying it (nothing is changed or added to Undo), and click to apply it.
Enter a name and choose **Save current** to add your exact settings to **My presets**; they reappear after restarting. Use **Edit description** on any personal preset to add, change or clear its description, and **Delete** to remove it, after confirming.
Descriptions are saved beside gallery JSON files as UTF-8 `.txt` files; the JSON stays CLI-compatible.
To add an existing JSON, load it, then use Save current in the gallery. JSON export remains available for sharing.
**Presets → Copy link to this look** copies a link to the web app that holds the settings in use, compressed into the
link's `#look=` fragment (about 800 characters); opening it in a browser applies the look in the web app, and
**Presets → Open look from link…** applies one in either app, as a change Undo reverses. No server keeps the look,
and a fragment is never sent to the site. An included NES LUT travels by name; a look with a LUT imported from a file
is too large for a link, so share that one as a JSON preset.
Saving under a name already in My presets, in any letter case, asks first, then replaces that preset's settings and keeps its name and description.
The included presets' names are refused, in any letter case, so an included preset is never replaced or shadowed. Malformed files are skipped with an explanation. Personal JSON files live outside the checkout,
in the app data directory shown in the gallery. `CRTSIM_DATA_DIR` can override that directory with an absolute path.
The desktop also saves a recoverable session every two seconds and on exit. **Project** opens/saves `.crtsim`
projects and lists the ten most recent projects. Recovery restores edits and leaves the export queue paused.
See the [v0.2 workflow guide](docs/WORKFLOW.md) for playback, framing, LUTs, projects and batch exports.

The desktop includes five interface themes: **CRT Dark**, **Paper Light**, **Luna Blue**, **Classic Platinum**
and **Sky Diary**. The two classic desktop palettes are operating-system-inspired rather than pixel-perfect replicas.
Sky Diary adapts the blue/pink window language of the aizumanga-neocities site without its animated background effects.
Themes change the application chrome only: render presets, previews and exported pixels are unaffected.

Full-resolution export shows stage-based progress and can be cancelled; routine live-preview updates stay unobtrusive. Warm-up advances after completed GPU batches,
followed by surface rendering/readback and PNG saving.
Percentages represent weighted work stages, not remaining seconds; PNG encoding stays at its stage until the file is completely saved.
Exported PNGs embed the exact JSON preset in a private PNG text chunk without changing the pixels. Use **Import preset from Image/Video…**
to recover settings from a rendered PNG, MP4, MKV or WebM. Video metadata also restores timing/audio choices.
Files without this metadata are rejected with an explanatory message; third-party transcoding or uploads may strip it.

**Filter mask when shrinking** samples the shadow mask from box-filtered mipmaps with trilinear filtering, as the original
game did, so the mask stays smooth where it is drawn smaller than it is. It is on by default; presets saved with it off keep
their unfiltered sampling. Display resizing can still introduce moiré.
**Mask follows the signal** gives the mask a column for every two signal columns and a row for every signal row, as the
original did, so a finer signal gets a finer mask. Super Win the Game and the line-count presets (Pixel art 240p, NTSC and PAL)
use it; the general-image presets keep a fixed 128 × 224 mask. Presets saved with fixed columns and rows keep them.

**Super Win the Game** uses the game's own CRT options on top of the public reference: a 30° field of view, **NTSC blending**
at 0.35, the artifact pattern as its Windows build draws it, and the bezel reflecting the picture as the screen shows it (**Reflection as on the screen**), and the game's grey backdrop past the bezel in wide outputs (**Backdrop color**). Its **NTSC palette** is off, since it
recolours only art drawn in the NES palette, so whatever is opened keeps its colours; **Super Win the Game NES palette**
is the same look with it on, as the game shows its own art. The game saves its screenshots with the two artifact patterns averaged; set **Phase** to **Stable** to compare with one. NTSC blending moves the two composite artifact patterns towards each other in the game's
units, so alternating ticks mix them 17.5/82.5 and 82.5/17.5 instead of switching cleanly; 0, the default, is the public
source's switch, and 1 shows their average on every tick. **NES palette** (Color & LUT) makes the palette from the game's
**Tint**, **Tint I** and **Tint Q**, one of two ways. **Super Win the Game's** makes it exactly as the game does, matching
the game's own output bit for bit, and recolours art drawn in the NES palette the game's art uses (the one beginning
`7C7C7C 0000FC 0000BC`); its table has the game's 32 steps per channel, each taking the colour of the nearest art colour.
**From the composite signal** decodes the NES's measured signal levels instead, fitted to FirebrandX's Composite Direct
capture, and recolours art drawn in MAME's NES palette, as the included NES LUTs expect. **LUT strength**, the game's NTSC
Palette slider, mixes any LUT or the palette with the original colours. The game's table is read as the game reads it,
nearest in red and green and blended in blue, here and in every shader port; other tables blend in all three.
**Optional color grade** rotates hue and changes chroma in YIQ before the composite simulation. Neutral values leave the prepared signal unchanged.
It is an artistic grade, not Super Win the Game's NES palette or a full NTSC decoder.
**Linear light (experimental)** decodes the SDR signal for glass sampling, performs lighting and bloom with RGBA16Float intermediates,
then encodes the final SDR PNG. Composite/persistence remains gamma-space RGBA8. It uses more GPU memory and changes the look;
it is not a full linear-light pipeline, HDR export or an exact match to the game.
Old presets load with reference processing and neutral grading. Newly saved presets require the updated app/CLI.

With stable Rust installed, run from the repository folder:

```sh
cargo run --release --locked -p crtsim-app
```

Use **Open File** or drag one PNG/JPEG/WebP/BMP/GIF into the window. Adjust the controls on the left;
with **Live preview** on, the preview follows a slider as you drag it, the way a game's picture follows its
options menu, then renders exactly once you let go. **Export PNG** renders using the export size,
even when the preview is smaller. The window remains responsive while loading, rendering and exporting.
Settings changed during an export apply to the next export. The native save dialog asks for confirmation when a desktop PNG or JSON preset
already exists; after confirmation, the app writes a complete temporary file and atomically replaces the destination.

- **Super Win the Game**, the preset the app starts with and **Reset** returns to, is the public-reference defaults,
  including 256x224 signal resampling and saturation 1.35, with the game's own CRT options on top, except its NES
  palette.
- **General image** starts with square pixels, smooth resizing, contain fitting and saturation 1.0.
- Under the source's name, **Open…** opens a file and **Export PNG…**, or **Export video…** for a video, saves the
  result. Below them, **Preset** names the preset the settings are, or are closest to, with how many settings differ
  from it, listed when pointed at; **Changed only** beside it hides every setting that matches that preset, and
  **Presets…** opens the gallery. With **Live preview** off, **Refresh** renders the edits waiting; on, the preview
  already follows them and Refresh is not offered. Notes that hold whatever the settings are sit behind **ℹ Notes**.
- The status bar's **notices** say what failed and what an export saved, one per topic: opening, saving, playback,
  the preview, presets, app data and the renderer. News of a topic replaces that topic's older news and leaves the
  rest, and each notice has its own **Dismiss**. A saved file's notice stays until the next export, with **Show in
  folder**, which shows it in Explorer or Finder, or opens its folder on Linux; an FFmpeg failure offers
  **FFmpeg setup…**.
- Closing the window while an export runs, or while the batch queue has jobs to go, asks first: **Keep exporting**,
  **Close when it finishes**, or **Stop and close**. Waiting to close stays open instead if the export fails, so its
  notice can say why. The web app asks the browser to ask before the page is left while it exports.
- **Original / CRT / Compare** compares the source with the rendered tube. Drag the comparison divider. The CRT bends the image, so the two views are not geometrically registered.
- **Pin look** keeps the settled picture and its settings; **Compare** then shows the pinned look left of the divider and
  the settings in use right of it, with the settings changed since listed above the picture. **Unpin look**, or opening
  another source, returns Compare to the original. The pinned picture is a still: after moving to another frame of a
  video, pin again to compare that one.
- **Auto / Fast / Balanced / Export resolution** changes preview canvas resolution only. **Auto**, the default, starts
  at Balanced (1280 px) and steps down to Fast (800 px) after two previews of a dragged slider take over 0.5 s, as on
  integrated graphics, and back up after five Fast ones take under 0.15 s; the list says which it is on. Settled
  stills do not count, since their warm-up is slow at any size. Choosing a size keeps it. Use Export resolution and turn off Fit view at 1x zoom to inspect mask sampling.
- **Load / Save preset** uses the same version-1 JSON format as the CLI. **Undo / Redo / Reset** acts on settings, not source files or exported files.
  `Ctrl+Z` and `Ctrl+Shift+Z` provide undo and redo shortcuts (`Command` equivalents are also accepted on macOS).
- Signal and export size boxes accept named presets or custom `WIDTHxHEIGHT`. Resolved dimensions and crop/mask warnings are shown in the window.
- `Ctrl+O` opens a file, `Ctrl+S` saves the project (asking where, the first time) and `Ctrl+E` exports a PNG;
  the menus show them, with `Command` on macOS.
- Dragging a slider moves it a quarter as far as the pointer, so it can be set finely; hold **Shift** to move it as far
  as the pointer, or **Alt** for a tenth as far again. A click still jumps it to the pointer.
- Click or drag a slider to give it the keyboard: **←/→** step it by 0.1% of its range, **Shift** by 1%, **Alt** by 0.01%, and **Delete** returns it to its default. **Esc** or a click elsewhere lets go.
- The status bar shows the app's version in its lower right corner. **Help → Check for updates…** asks GitHub
  whether a newer release is out and shows its notes and download page; turn on **Check when the app starts**
  there to be told without asking. The app never downloads or installs a release itself; package managers and
  AppImage tools do that (see [updating](docs/RELEASES.md#with-a-package-manager) and
  [the decision](docs/adr/0004-check-for-updates-never-download-them.md)).

The original-image display is limited to a 2048-pixel thumbnail; export always uses the loaded source.
While a control is being changed, each preview runs the CRT on screen on with the new settings, so the
glow of the old ones fades over a few frames; the exact still follows once the change settles. Video playback streams frames with persistent CRT history and a short buffer, and keeps
playing while you edit: the new settings show once the few frames already buffered have played. A change of
signal or preview size, or of colour mode, plays on again from the frame on screen.
The CRT mask can look different at different preview sizes; exports retain the requested resolution.
Recovery does not overwrite explicitly saved presets or project files.

Linux needs Vulkan for the CRT renderer and an X11 or Wayland session. The window shares that same
device, falling back to OpenGL where no Vulkan driver is present, so the application still opens and
reports the missing driver rather than failing to start.
X11 also needs the xkbcommon X11 library (`libxkbcommon-x11` on Arch, `libxkbcommon-x11-0` on Debian/Ubuntu).
The native file picker uses the desktop portal. On Arch/KDE, ensure `xdg-desktop-portal` and `xdg-desktop-portal-kde`
are installed and working in your logged-in desktop session. Drag-and-drop or passing an image path also works:

```sh
cargo run --release --locked -p crtsim-app -- "image.png" --backend vulkan
```

Windows uses the system file picker and normally DX12 for rendering; macOS uses its system picker and Metal.
The window draws on the same device as the renderer, through eframe's wgpu backend.
macOS runtime support remains provisional until tested on a real Mac.
For Linux source-build errors about windowing libraries, install your distribution's Wayland and xkbcommon development packages
(Debian/Ubuntu: `libwayland-dev libxkbcommon-dev libegl1-mesa-dev`; Arch: `wayland libxkbcommon`).

### Graphics-card compatibility

The renderer is not NVIDIA-specific. It requests no optional wgpu device features and is expected to work on AMD, Intel, NVIDIA and Apple GPUs
when the operating system has a working Vulkan, DX12 or Metal driver. Integrated graphics should also work, although large exports and the
experimental linear-light mode need more memory and may be slower. The adapter's maximum texture dimension is checked before rendering.

CI compiles the application for Windows, Linux and macOS and executes the GPU pipeline through Linux software Vulkan. That proves the shaders
do not depend on one vendor, but it is not a substitute for runtime tests on several physical GPUs and drivers. Older hardware without a usable
Vulkan/DX12/Metal implementation will show a compatible-adapter error; it is not silently switched to a CPU renderer.

## Web app

The same app runs in a browser with WebGPU, as the web app. It opens images, animated GIF or
WebP, and MP4, MOV, WebM or MKV videos you pick or drop on the page (the videos decoded by the
browser, so the codecs are the browser's), previews and plays them, and saves PNGs, MP4 and WebM videos (encoded by the browser, so the
formats offered are those it can write), GIF, animated WebP and animated PNG exports and presets as downloads;
its settings, presets and session stay in the browser. Its MP4 and WebM exports embed the
preset as the desktop's do, and **Import preset from Image/Video…** reads one back from a PNG,
MP4 or WebM, made by either app. Projects and batch export are
desktop-only for now (see [the web plan](docs/WEB_PLAN.md)). On a phone, or any screen under 640
points wide, the settings sit below the picture.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked   # the version Cargo.lock pins
python3 tools/build_web.py                                  # writes web/dist
python3 -m http.server --directory web/dist 8000            # then open http://localhost:8000
```

Browsers without WebGPU get a page that says how to turn it on. Firefox and LibreWolf on Linux
need `dom.webgpu.enabled` set to `true` in `about:config`.

## In games and streams: RetroArch, ReShade, OBS, Godot and Bevy

The CRT also runs in other programs, as shader ports of the same passes (see
[the decision](docs/adr/0003-games-get-shader-ports-not-the-renderer.md)), which every port shares
([ADR 5](docs/adr/0005-the-ports-share-one-translation-of-the-passes.md)), and in Bevy as the
renderer itself ([ADR 6](docs/adr/0006-bevy-embeds-the-renderer-godot-gets-a-compositor-port.md)).
The glass and its bezel draw as the app draws them, from the same baked bezel.

- **RetroArch**, over any core as it plays. Each release has a `-retroarch.zip` of presets for the
  shipped looks; **Export → RetroArch shader…**, or `crtsim export-retroarch`, writes any look as
  one. See [`ports/retroarch`](ports/retroarch).
- **ReShade**, over almost any PC game. Each release has a `-reshade.zip` of the effect and
  presets for the shipped looks; **Export → ReShade effect…**, or `crtsim export-reshade`, writes
  any look as one. See [`ports/reshade`](ports/reshade).
- **OBS Studio**, as a filter on any source, keeping the glow between frames. Each release has
  an `-obs-linux-x86_64.zip` of the plugin built for Linux; elsewhere it builds from
  [`ports/obs`](ports/obs). **Export → OBS filter…**, or `crtsim export-obs`, writes any look for
  it.
- **Godot 4**, as a compositor effect over a camera's picture, 2D games included, keeping the
  glow between frames. It is in the Godot Asset Library, and each release has a `-godot.zip` of the
  add-on with the shipped looks; **Export → Godot effect…**, or `crtsim export-godot`, writes the add-on
  with any look. See [`ports/godot`](ports/godot).
- **Bevy**, as the renderer itself on Bevy's own device: the
  [`crtsim-bevy`](crates/crtsim-bevy) crate, published to crates.io with each release, draws any
  camera holding a `Crt` with any look.

The shipped looks are the public reference (`crtsim-renderer`), the same in linear light
(`-linear`), General image (`-general`), the reference with the NES palette (`-nes-palette`), and
Super Win the Game as the app starts with it (`-super-win-the-game`) and with the game's NES
palette (`-super-win-the-game-nes-palette`).

Every port is checked against the renderer, picture for picture: the RetroArch port through
librashader and in RetroArch itself, the ReShade effect in vkBasalt, the OBS filter in OBS itself
and the Godot effect in Godot itself; and the ReShade effect is compiled by ReShade's own compiler
as each shipped look sets it. Bevy draws the renderer's own picture, which a headless Bevy app
checks.

## In video editors and Aseprite

The renderer also runs as an effect over a clip, at the clip's own size, with the app's looks or a preset it saved,
carrying the glow from frame to frame in whatever order the editor renders them:

- **DaVinci Resolve, Natron, Nuke** and other OpenFX hosts, as `CRTSim.ofx.bundle`.
- **Kdenlive**, MLT's `melt` and FFmpeg, as the `crtsim` frei0r filter, with a description that lists it in Kdenlive.

Each release has an `-editors-<system>.zip` of both. See [the editors' guide](docs/EDITORS.md) for where they go and
how they handle frames rendered out of order. They are checked in FFmpeg, `melt` and Natron against the command line's
render of the same clip.

**Aseprite** has an extension, `.aseprite-extension` in each release, that draws a sprite's frame or its animation
through the CRT with the command line and opens the result as a new sprite. See [the Aseprite guide](docs/ASEPRITE.md).

## As a service

`crtsim-service` renders pictures, animations and videos sent over HTTP, for a chat bot or a batch of renders on a
server with no screen, one at a time on one GPU, or on software Vulkan in its container, with a queue and limits on
every request:

```sh
docker build -f packaging/service/Dockerfile -t crtsim-service .
docker run --rm -p 127.0.0.1:8080:8080 -e CRTSIM_SERVICE_TOKEN=<a long secret> crtsim-service
curl -H "Authorization: Bearer <a long secret>" --data-binary @shot.png -o crt.png 'localhost:8080/render?preset=NTSC%20240p'
```

See [the service's documentation](docs/SERVICE.md) for its options, limits and how to expose it.

## In Rust programs: crtsim-core

The renderer itself is the [`crtsim-core`](crates/crtsim-core) crate, ready for crates.io: stills, video sequences
that keep the glow from frame to frame, and `Renderer::encode`, which records a game's or an engine's frame into its
own command encoder, from its texture into its target, on its own device, waiting for nothing. Its settings are the
same JSON the app saves as a preset. See [publishing it](docs/RELEASES.md#crtsim-core-on-cratesio).

## Build and try

### Video

Video needs `ffmpeg` and `ffprobe`; the image renderer does not. **Export → FFmpeg setup…** shows whether they
were found, which export formats they can write, and the install command for your system with a Copy button
(`winget install --id Gyan.FFmpeg -e` on Windows, `brew install ffmpeg` on macOS, your package manager on Linux).
Opening a video without FFmpeg opens that window instead of failing.
The app looks for the programs, in order, where `CRTSIM_FFMPEG` and `CRTSIM_FFPROBE` point; beside the app, or in an
`ffmpeg` or `ffmpeg/bin` folder next to it; in Homebrew's and MacPorts' folders on macOS, and winget's, Scoop's and
Chocolatey's on Windows, which apps started from the desktop may not have on their PATH; and finally on PATH.
FFmpeg must include `libx264` for MP4/MKV, `libvpx-vp9` for WebM, `libwebp` for animated WebP export, and AAC/Opus encoders when converting audio.
Animated GIF and WebP, and a sprite sheet's JSON as Aseprite writes it, open and play without FFmpeg; exporting any video or animation from the app needs it.
Nothing is downloaded automatically.

1. Choose **Open File…**, drop a video or an animated GIF/WebP, or pass its path on the command line.
   A GIF or WebP with a single frame opens as a still image. Batches keep turning animated WebP into a PNG of its first frame, as before; animated GIFs batch as MKV video.
2. Use **Previous frame / Next frame**, arrow keys (when no slider has the keyboard), the frame number field or the wide slider beneath the preview.
   Selection loads when you release the slider or choose **Go**. Frame numbers start at 1 and follow actual decoded frames, including VFR sources.
   **Play / Pause** (Space) starts buffered playback with CRT history. **Loop** plays a short clip or GIF again from its start when it ends, with no pause, and the CRT's glow carries over from its last frame to its first.
   **Subtitles**, shown for a video that has subtitle tracks, draws one of them into the picture so the CRT filters it with everything else, in the preview and in exports. **Export frame** saves the displayed source frame as a full-resolution, settled CRT PNG.
   When a valid container frame count exists it is used immediately. Other files require one decoded-frame count on open. Exact seeks decode by ordinal from the start and can take time on long videos; loading/seeking is cancellable.
3. Choose output resolution and video timing: source-rate stable artifacts (default), 60 Hz alternating artifacts, or persistence off.
4. Choose **Export → Video…**. A separate window explains MP4/H.264, MKV/H.264 and WebM/VP9, with quality/audio controls and optional advanced encoding settings,
   and offers GIF, animated WebP and animated PNG with their own size, frame-rate and length settings and a file-size estimate.
   For MKV, **Picture quality → Lossless** writes FFV1 that keeps every pixel exactly. Then choose the destination.

The compact toolbar groups media/projects under **File**, gallery/imports under **Presets**, themes under **View**, rendering under **Export**, and credits and the update check under **Help**. Video previews are silent; audio is retained in exports unless explicitly muted.

Exports retain CRT history between ordered frames. Variable-frame-rate input is normalized to the detected average frame rate,
or to 60 FPS in 60 Hz mode. Source-rate mode corrects persistence decay by media time; this is an approximation of the spatial feedback effect.
Audio defaults to copying compatible tracks, with AAC/Opus fallback; delayed tracks are re-encoded to preserve their timing.
**Re-encode** and **Mute** are also available. Video output dimensions must be even.

The progress bar reports frames, rendering speed and an approximate remaining time. **Cancel** stops image export, video loading and video export;
closing the app also terminates its FFmpeg processes. Existing destinations are replaced only after a successful export.
Temporary encoded files require space on the destination drive, but decoded frames are streamed rather than saved as PNGs.
Settings and the source are captured for each export; edits during export apply to the next job, and the preview keeps up with them while the export runs.

Output is opaque SDR with software or optional hardware H.264 encoding. Video is explicitly converted from full-range RGB to limited-range BT.709 and tagged accordingly, except Lossless, which keeps full-range RGB in FFV1 and is tagged as RGB. HDR input uses FFmpeg's `zscale`/`tonemap` filters when available; HDR output remains outside this version.
Audio tracks, supported subtitles, chapters and source metadata are preserved by default; MKV also supports attachments. The export window lists the subtitle tracks, to keep each as a track of its own or not, and to draw one into the picture, through the CRT, instead of leaving it for the player to draw on top. See the [workflow guide](docs/WORKFLOW.md) for container limits and quality profiles.
MP4/MKV/WebM exports embed an importable preset in a container comment, including the original controls and video timing/audio settings.
PNG exports still embed importable presets. Full details and validation commands are in [VIDEO_PIPELINE.md](docs/VIDEO_PIPELINE.md).

### Images and CLI

Install stable Rust through [rustup](https://rustup.rs/), then:

```sh
cargo run -p crtsim-cli -- presets
cargo run -p crtsim-cli -- inspect-meshes
cargo run --release -p crtsim-cli -- render --output test-crt.png
```

The last command uses the built-in test card and the public-reference defaults, with stable artifact phase.
`--preset "NTSC 240p"` renders with an included preset instead; `presets` lists them.

```sh
cargo run --release -p crtsim-cli -- contact-sheet --input image.png --output sheet.png
cargo run --release -p crtsim-cli -- contact-sheet --preset "Clean RGB" --look Mine=mine.json --tile 640x480 --columns 2 --output mine.png
```

`render` also makes videos and animations, the output's extension choosing which: `.mp4`, `.mkv` or `.webm` a video,
`.gif` or `.webp` an animation, and `.png` a still, or an animated PNG when the input is a video or animation. Without
`--input`, a moving output renders the video test card. They need FFmpeg, as the desktop's do, except an animation
(a GIF, a lossless WebP or an animated PNG) of an animated GIF or WebP, a sprite sheet or the video test card, which is
written without it where FFmpeg is not installed.

A sprite sheet's JSON, as Aseprite's **Export Sprite Sheet** and TexturePacker write it, renders as an animation of its
frames, each shown as long as the JSON says; see [the Aseprite extension](docs/ASEPRITE.md), which renders a sprite
this way from inside Aseprite.

```sh
cargo run --release -p crtsim-cli -- render -i clip.mp4 -o crt.mp4 --preset "NTSC 240p" --size 1080p
cargo run --release -p crtsim-cli -- render -i clip.mp4 -o crt.mkv --quality lossless --timing ntsc60
cargo run --release -p crtsim-cli -- render -i clip.mp4 -o crt.gif --max-side 480 --fps 24 --seconds 5
cargo run --release -p crtsim-cli -- render -i walk.json -o walk-crt.webp --lossless --fps 12 --seconds 0
```

`--quality` (draft, balanced, high, archival or lossless), `--timing` (stable, ntsc60 or off), `--audio` and
`--encoder` set a video as the desktop's Export → Video does; `--fps`, `--max-side`, `--start`, `--seconds` and
`--lossless` (WebP) set an animation. Progress is printed as it goes.

A **contact sheet** renders one image through many looks, every included preset unless `--preset` or
`--look NAME=SETTINGS.json` choose some, and lays them out in a grid with each look's name beneath its tile
(`--tile` sets each tile's size, 480x360 by default, and `--columns` how many go in a row). In the desktop and web
apps, **Export → Contact sheet…** does the same with the picture on screen, the settings in use and any of the
gallery's presets, your own included.
Files are **never overwritten**: choose a new output name if it already exists. Parent directories must exist.
All assets are embedded; the executable downloads nothing at runtime. The desktop reaches the network only to check for a
newer release, when asked ([ADR 4](docs/adr/0004-check-for-updates-never-download-them.md)); the CLI never does.

### Your own image

For modern illustrations, photos or screenshots, start with square pixels and contain fitting:

```sh
cargo run -p crtsim-cli -- config --general --output general.json
cargo run --release -p crtsim-cli -- render --input "image.png" --config general.json --output rendered.png
```

Edit `general.json` to adjust the effect. It is not necessary to rebuild after editing settings.
Transparency is composited onto black before filtering; output is opaque SDR PNG. HDR output is not implemented.
The default path follows the original gamma-space UNORM behavior, not linear-light rendering.

### Colour profiles

The CRT works in sRGB. A picture whose file carries a colour profile (an ICC profile in a PNG, JPEG or WebP,
including CMYK JPEGs from print work, or PNG's cICP, gAMA and cHRM chunks) has its colours converted into sRGB as
it opens, and the status line says from what. A profile that is sRGB in effect, as many programs embed, leaves
every value as it is, so art in exact NES palette values still matches the palette. **View → Pictures' colour
profiles → Ignore**, or `--ignore-profile`, reads every picture's numbers as sRGB whatever it carries.

PNG stills and contact sheets are written in sRGB, tagged as such, unless **Export → PNG colour space** or
`--color-space` chooses Display P3, Adobe RGB, Rec. 2020, ProPhoto RGB or an RGB `.icc` profile from a file;
then the PNG is converted at 16 bits, with that profile embedded, so colour-managed programs show the same picture.
Videos and animations stay sRGB and BT.709. Conversions use [moxcms](https://github.com/awxkee/moxcms), checked
against lcms in [the validation notes](docs/VALIDATION.md#colour-management).

```sh
cargo run --release -p crtsim-cli -- render -i photo.jpg -o print.png --color-space adobe-rgb
cargo run --release -p crtsim-cli -- render -i sprite.png -o crt.png --ignore-profile
```

### Resolutions

Signal resolution controls the image entering the simulation. Output resolution controls the final CRT picture, including its frame.

| Signal preset | Meaning |
|---|---|
| `original` | 256x224; explicit retro resampling |
| `auto` | Preserve aspect; at most 480 rows, never upscale height |
| `native` | Keep input dimensions |
| `240p`, `360p`, `480p` | Set logical height, preserve input aspect |
| `WIDTHxHEIGHT` | Explicit size; can change aspect |

| Output preset | Dimensions |
|---|---|
| `reference` | 1600x900 |
| `720p` | 1280x720 |
| `1080p` | 1920x1080 |
| `1440p` | 2560x1440 |
| `4k` | 3840x2160 |
| `match-input` | Original input dimensions |
| `WIDTHxHEIGHT` | Explicit canvas |

```sh
cargo run --release -p crtsim-cli -- render --input image.png --config general.json --size 4k --output rendered-4k.png
```

`--signal` changes dimensions only: it does not silently change pixel aspect, filtering or fit settings.
Use `config --general` for normal images: it writes the desktop's General image preset. Plain `config` generates the original-style configuration (8:7 pixel aspect, nearest filtering).
`fit` controls placement on the tube: `reference`, `contain`, `cover`, or `stretch`.
Contain mode prevents rectangular aspect cropping, but the rounded glass and barrel distortion can still hide extreme corners.
A widescreen output canvas does not turn the tube itself into a widescreen tube; **Tube** does.

**Tube** (`"tube_aspect"`, the tube's width over its height, under Image & output and Glass) makes the tube the
original's 4:3, or wider, to the 16:9 of the last sets, or narrower, to a portrait 3:4 such as an arcade cabinet's
turned monitor; anything from 0.75 to 2. The glass is the original's sphere stretched across, keeping its rounded
corners, and the bezel keeps its corners and sides, its top and bottom running between them. A tube wider than the
output is framed as a 4:3 tube is in an output of the tube's shape. **Widescreen television** is a 16:9 tube showing
480 interlaced rows. The shader ports carry it.

Hard safeguards: maximum 64 megapixels and 16384 pixels per side, additionally limited by the adapter and a conservative 1.5 GB job estimate.
This is a prototype safety policy, not a universal GPU limit or a promise that allocations cannot fail. Very large inputs are rejected before decoding;
resize those in an image editor first. Smaller supported inputs can be downscaled automatically before uploading to the GPU.

### Inspect intermediate results

```sh
cargo run --release -p crtsim-cli -- render --output crt.png --debug-dir debug-new --phase stable --warmup 16
cargo run -p crtsim-cli -- inspect-meshes --export-dir meshes-new
```

Debug output contains `clean.png`, `signal.png`, and the resolved `settings.json`. Mesh export writes portable JSON with every original vertex attribute.
Export directories must be new. Sixteen warm-up ticks means 17 total ticks; it is deterministic, not an assertion of complete convergence.
`alternating` advances phase once per tick; the final phase depends on warm-up parity. Stable mode is recommended for still images.

**Interlaced fields** (`"interlace": true`) makes each tick scan every other signal row, alternating fields, while the rows it skips only fade by persistence, as on an interlaced set. Use it with a 480- or 576-row signal. On a still, the last tick's field is the bright one, so warm-up parity picks which; in a video each frame is one field.
**Flip artifact pattern** (`"flip_artifacts": true`) turns the composite artifact pattern upside down against the picture, each tick blending in the row above rather than the one below, as Super Win the Game's Linux build draws it; compared with that build's own frames, the composite signal then matches within 2 steps. Off, the default, is the public CRTSim source's orientation, which the game's Windows build, on Direct3D 9, also draws; the **Super Win the Game** preset leaves it off. The Mac build draws through OpenGL as the Linux build does, so it likely flips too; turn this on to match those. The shader ports carry it.

**Backdrop color** (`"backdrop_color"`, under Frame & lighting) fills what neither the glass nor the bezel covers, at the sides of an output wider than 4:3; black by default. Super Win the Game draws a grey backdrop there at 1/16, which its preset uses. The shader ports carry it.

**Reflection as on the screen** (`"reflection_as_screen": true`, under Bloom & reflections) has the bezel reflect the picture with the screen's overscan and mask density, as Super Win the Game's monitor pass does. Off, the default, is the public CRTSim source, which reflects it with the overscan the other way round and half the mask's rows. At an overscan of 1 the two differ only faintly; with the overscan changed, the bezel's reflection then matches the game's within a fraction of a step. The **Super Win the Game** preset turns it on, and the shader ports carry it.

### GPU troubleshooting

- Linux: use your distribution's Vulkan driver for your GPU. On Arch/NVIDIA this normally comes with the Vulkan loader and matching NVIDIA userspace driver.
- Windows: use a DX12-capable GPU/driver. `--backend dx12` selects it explicitly.
- macOS: `--backend metal`; runtime visual validation still needs a Mac.
- `--backend vulkan` is useful on Linux or a Windows Vulkan setup.
- No display/window is needed. Software Vulkan is useful for CI but not representative of GPU performance.

```sh
cargo test --workspace --locked
cargo test -p crtsim-core gpu_smoke -- --ignored --nocapture
```

Only the second command requires a GPU or software Vulkan driver. Normal tests include WGSL validation through Naga and asset/configuration checks.

## Credits

Based on the CC0 CRTSim reference implementation by **J. Kyle Pittman**.
Original assets and shader source remain unmodified in `crates/crtsim-core/assets/original-crtsim/`; provenance is in [SOURCES.md](crates/crtsim-core/assets/original-crtsim/SOURCES.md).
The WGSL translation and application additions use this repository's CC0 license. Dependencies keep their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
