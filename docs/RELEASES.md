# Portable releases (Phase 4)

## Unreleased

Super Win the Game's look comes closer to the game's own: its bloom reads the picture as the game
reads it, and its black holds where the game's 8-bit glow holds it.

- **Bloom from mipmaps.** The bloom's first blur reads the picture through its mipmaps, at the
  level a target a sixteenth its size reads, as the game does, so each tap averages 16×16 pixels
  and the mask no longer flickers into the glow. On in the Super Win the Game looks; the public
  reference reads the picture itself, as before.
- **Glow floor.** The glow never fades below the level 8-bit history rounds it back to, 1 of 255
  in red and green at the default persistence, as every pixel stays once the game's title screen
  has lit it. On in the Super Win the Game looks.
- The shader ports ship the Super Win the Game looks, as the app starts with them and with the
  game's NES palette (`crtsim-renderer-super-win-the-game` and
  `crtsim-renderer-super-win-the-game-nes-palette`). RetroArch before 1.20, which says no
  picture shape, takes a look's own for a fixed signal, such as the game's 8:7 pixels.
- Settings take every value the game's own sliders do: sharpness and saturation up to 4, edge
  dimming and mask opacity up to 2, specular light up to 5, rim light up to 10, and a field of
  view down to 0.05°.
- The ReShade, OBS and Godot ports size the bloom's first target as the renderer and the game
  do, a sixteenth of the picture rounded down.
- The glass's rim light is squared by multiplying, as the game squares it.
- [The research notes](research/swtg-crt/README.md) add session 11: the article, every shipped
  shader and render state, the rest of the engine around the CRT, and the two changes above
  checked against the running game.

## v1.6.0

The CRT runs in games made with Bevy, as the renderer itself, and in Godot 4, as a compositor effect; and a video
playing goes on through a new preview size with its glow.

- **Bevy.** The [`crtsim-bevy`](../crates/crtsim-bevy) crate, published on crates.io with each release, draws every
  camera holding a `Crt` through the CRT with the renderer itself, on Bevy's own device, after Bevy's post-processing
  and its interface. Any included look, saved settings or `Config` will do, HDR cameras included. It is for Bevy 0.20
  ([ADR 6](adr/0006-bevy-embeds-the-renderer-godot-gets-a-compositor-port.md)).
- **Godot.** `CRTSim-Renderer-<version>-godot.zip` holds the Godot add-on with the shipped looks: a `CRTSimEffect` for a
  camera's or an environment's compositor, every setting in the inspector, keeping the glow between frames. A 2D game
  draws through it with the environment's Canvas background. **Export → Godot effect…** and `crtsim export-godot` write
  the add-on with any look as a saved effect. It needs Godot 4.3 or later with Forward+ or Mobile, and is checked
  against the renderer in Godot itself. See [`ports/godot`](../ports/godot).
- **Playback** goes on through a new output size, such as another preview quality, without stopping and without the
  glow starting over, as long as frames of the new size fit its buffer. Another signal size or colour mode still plays
  again from the frame on screen.
- `crtsim-core`: a host frame reads and draws a float texture, as an engine's HDR target is, as linear light, as it
  does an sRGB one; a host frame of no time, as an engine's first can be, holds the glow rather than being refused; and
  a video sequence goes on with its glow through a new output size.

## v1.5.0

The CRT goes where pictures are made: into video editors as an OpenFX effect and a frei0r filter, into Aseprite as an
extension, and onto a server as a render service. Pictures keep their colours from their profiles, and the tube can
be widescreen.

- **Colour profiles.** A picture's ICC profile (PNG, JPEG including CMYK, WebP), or PNG's colour chunks, say what its
  colours are, and they are converted into sRGB, the CRT's own. A profile that is sRGB in effect leaves every value as
  it is, so palette art still matches its palette. **View → Pictures' colour profiles → Ignore**, or
  `--ignore-profile`, reads the numbers as sRGB instead. PNG stills and contact sheets can be written in Display P3,
  Adobe RGB, Rec. 2020, ProPhoto RGB or a profile from a file, at 16 bits with the profile embedded
  (**Export → PNG colour space**, or `--color-space`).
- **Tube** sits beside Fit: 4:3 as before, 16:10, 16:9, a portrait 3:4 or anything between. The glass and bezel keep
  their shape, stretched; a 4:3 tube is the original's bit for bit. **Widescreen television** is a 16:9 tube showing
  480 interlaced rows. The RetroArch, ReShade and OBS ports take the tube's shape too.
- **Video editors.** An OpenFX effect for DaVinci Resolve, Natron, Nuke and other OpenFX hosts, and a frei0r filter
  for Kdenlive, `melt` and FFmpeg, in an `-editors-<system>.zip` for each system. Frames rendered out of order come out
  as they would in order. See [the editors' guide](EDITORS.md).
- **Aseprite.** The `.aseprite-extension` adds **File → Export → CRTSim Renderer...**, drawing a sprite's frame, its
  animation or a tag through the command line and opening the result as a new sprite. See [the guide](ASEPRITE.md).
- **Sprite sheets.** A sprite sheet's JSON, as Aseprite and TexturePacker write it, opens in the app and renders from
  the command line as an animation, each frame shown as long as the JSON says.
- The command line writes a GIF, lossless WebP or animated PNG of an animated GIF or WebP, a sprite sheet or the video
  test card without FFmpeg, when FFmpeg is not installed.
- **Render service.** `crtsim-service` renders pictures, animations and videos sent over HTTP, with a token, a
  bounded queue and limits on every request, and a container image on software Vulkan. See [the service](SERVICE.md).
- `crtsim-core` records a game's or an engine's frames into its own command encoder (`Renderer::encode`), and is
  published on crates.io with each release, at the release's version.
- Releases are pushed to itch.io too, a channel for each package, once the itch.io page is set up.

## v1.4.0

The desktop and web app are easier to start with and to finish an export in: Open and Export sit under the source,
the panel names the preset in use, the first run offers three looks, and messages no longer hide each other.
Super Win the Game stays the default look, with its NES palette off. [The review](UX_REVIEW.md) behind these changes
and [the plan](UX_PLAN.md) that ranked them are in the docs.

- **Super Win the Game**, the look the app starts with and Reset returns to, leaves its NES palette off, since the
  palette recolours only art drawn in the NES palette. **Super Win the Game NES palette** is the game's look with it
  on, by that name in the gallery and in `crtsim render --preset`.
- Right after the welcome, the first run offers three looks, Super Win the Game, General image and Soft television,
  each a thumbnail of the picture open.
- **Open…** and **Export PNG…** or **Export video…** sit under the source's name. **Preset** below them names the
  preset the settings are, or are closest to, with how many settings differ, listed when pointed at, and
  **Presets…** opens the gallery. **Changed only** hides every setting that matches it.
- The status bar's notices say what failed and what an export saved, one per topic, each with its own Dismiss. A
  preview that fails no longer hides an export that failed. A saved file's notice offers **Show in folder**, and an
  FFmpeg failure **FFmpeg setup…**.
- Closing the window during an export asks first: keep exporting, close when it finishes, or stop and close. The web
  app has the browser ask before the page is left while it exports.
- **Auto** preview quality, the default, steps from Balanced down to Fast while dragged previews are slow, as on
  integrated graphics, and back up once they are quick. Choosing a size keeps it.
- On the desktop, several images or videos dropped on the window become batch jobs, once a folder for their exports
  is chosen.
- The preset gallery opens on its presets, with saving under My presets. Standing notes sit behind **ℹ Notes**, small
  print is 11 px rather than 9, and Refresh shows only while Live preview is off.
- The Pause button, arrows and crosses that drew as boxes draw properly, and a test checks the fonts hold every
  character the interface writes.

## v1.3.0

The CRT runs in ReShade, over almost any PC game, and in OBS Studio, as a filter that keeps the glow between frames,
and the web app fits a phone.

- `CRTSim-Renderer-<version>-reshade.zip` holds the ReShade effect and presets for the shipped looks: copy its
  `reshade-shaders` folder into the game's folder and pick a preset in ReShade's overlay. **Export → ReShade effect…**
  and `crtsim export-reshade` write any look as a preset. The effect resizes the game's picture to the signal itself;
  the signal's size, linear light and a look's colour table are preprocessor definitions the preset sets.
- `CRTSim-Renderer-<version>-obs-linux-x86_64.zip` holds the OBS filter built for OBS 30 on Linux, with its data and
  the shipped looks; elsewhere it builds from `ports/obs`. Add **CRTSim Renderer** to a source's filters and apply a
  look. **Export → OBS filter…** and `crtsim export-obs` write any look for it.
- RetroArch, ReShade and OBS run one translation of the passes, `ports/passes`
  ([ADR 5](adr/0005-the-ports-share-one-translation-of-the-passes.md)). RetroArch presets draw as before: the parity
  test measures the same differences from the renderer, case for case. RetroArch's **Mask columns** and **Mask rows**
  say they follow the signal, which is what they always followed.
- The OBS filter is tested against the renderer in OBS itself, and the ReShade effect compiled with ReShade's own
  compiler as each shipped look sets it.
- The web app fits a phone: under 640 points wide the settings sit below the picture, in a panel you can drag taller,
  and every window fits the screen. The desktop app does the same in a narrow window.

## v1.2.0

The app tells you when a release is out and package managers install and update it, looks travel as links, two looks
compare side by side, contact sheets show many looks at once, exports can be lossless, and the CLI renders video.

- **Help → Check for updates…** asks GitHub for the latest release and shows its notes and download page. **Check when
  the app starts**, off until you turn it on, does the same at each start. The app never downloads or installs a
  release itself, and reaches the network for nothing else ([ADR 4](adr/0004-check-for-updates-never-download-them.md)).
  **Credits** moves into the new **Help** menu.
- Install and update with **Scoop** or **Homebrew** from this repository, and with **winget** once its manifests are
  accepted (see [With a package manager](#with-a-package-manager)). The AppImage carries its update information, and a
  `.zsync` file is published beside it, so AppImageUpdate or Gear Lever update it in place.
- **Presets → Copy link to this look** copies a link that opens the settings in use in the web app; **Open look from
  link…** applies one in either app, as a change Undo reverses. A look with an imported `.cube` LUT is too large for a
  link and is shared as a preset instead.
- **Pin look** keeps the picture on screen and its settings; **Compare** then shows them beside the settings in use,
  with the settings changed listed above the picture.
- **Export → Contact sheet…** renders the picture through the settings in use and any of the gallery's presets into one
  labelled PNG. `crtsim contact-sheet` does the same from the command line, and `crtsim render --preset NAME` renders
  with an included preset.
- **Animated PNG** joins GIF and WebP as an animation export, every pixel exact, written without FFmpeg in the web app.
  MKV video has a **Lossless** quality that keeps the CRT's RGB exactly in FFV1.
- `crtsim render` makes videos and animations, by the output's extension: `.mp4`, `.mkv`, `.webm`, `.gif`, `.webp`,
  or `.png` from a moving input. Without `--input` a moving output renders the video test card.
- The macOS app reports its own version; it said 0.1.1 since that release. Packaging refuses a tag that does not match
  the version in `Cargo.toml`.
- Presets, projects and sessions are written as before. A video exported with Lossless quality carries a preset that
  earlier versions cannot import.

## v1.1.0

Super Win the Game is the default look, sliders move finely, the web app's videos carry their preset, and
the app's preview, video, source, work and file handling were each rebuilt as one module, with the bugs that
turned up fixed.

- **Super Win the Game** is the preset the app starts with, and the one **Reset** and each setting's own reset return
  to. It takes the place of Original CRTSim, which was the same public reference without the game's options.
- Sliders move finely: dragging moves one a quarter as far as the pointer (**Shift** as far, **Alt** a tenth as far
  again), and the arrow keys step it by 0.1% of its range (**Shift** 1%, **Alt** 0.01%).
- The NES palette made as the game makes it is called **Super Win the Game's** rather than "the game's".
- The status bar shows the app's version in its lower right corner.
- The web app's MP4 and WebM exports embed their preset as the desktop's do, in the container's
  comment, so either app imports it. The web app's **Import preset from Image/Video…** now reads
  a preset from an MP4 or WebM as well as a PNG.
- On the desktop, an animation opened from a browser's bytes exports as a video too.
- LUTs and presets are read the same way in both apps: the web app now refuses a LUT over 16 MB as
  the desktop does, and both say "Loaded preset", "Imported preset from" and "Cannot import
  preset" with the file's name.
- A project or recovered session whose source cannot be opened, or whose opening is cancelled,
  still restores its settings and export queue, as one whose source is missing already did; they
  were dropped before. The source panel names a missing or unopened source and says to relink it
  with Open File, and the session keeps it, so the next start tries it again.

## v1.0.1

The preview now follows the controls as Super Win the Game's picture follows its options menu: with **Live preview** on,
a slider shows its effect while it is being dragged, and a video keeps playing while its settings are edited.

- While a control is being changed, each preview runs the CRT already on screen on with the new settings, so an edit
  shows at once and the glow of the old settings fades over a few frames. Once the change settles it becomes one undo
  step and is rendered again exactly, as an export renders it. Previews used to wait until the pointer was let go.
- Editing while a video plays no longer pauses it: the new settings show once the few frames already buffered have
  played. A new signal or preview size, or colour mode, plays on again from the frame on screen.
- Deleting a personal preset removes its description first, so a description can no longer be left behind and picked
  up by the next preset saved under that name. Presets are found in any letter case when deleted or described.
- The included presets' names cannot be used for personal presets, in any letter case, so an included preset is never
  replaced or shown twice. A personal preset saved under one before keeps working and can be deleted.
- `tools/showcase` makes a vertical showcase video of the renderer from six original animations, with per-clip music
  and a reel editor. It is a development tool and is not part of the packages.

## v1.0.0

The **Super Win the Game** preset now draws the CRT as the shipped game does, checked against the game's own frames:
its palette and colour table, its NTSC blending, the screen mesh's outline and shading, the bezel's reflection and the
backdrop around it. All but the screen mesh are settings of their own, off in the other looks; the
screen's outline and shading move every look by a fraction of a step. Presets and projects saved before open as they
looked. The research behind it, and how to reproduce it from a copy of the game, is in
`docs/research/swtg-crt/README.md`.

- **Backdrop color** (Frame & lighting) fills the sides of an output wider than 4:3, past the bezel, where the
  renderer drew black. Black by default; the **Super Win the Game** preset uses the game's grey backdrop, 1/16,
  which brings a 16:9 frame from 1.76 steps off the game's on average to 0.85. RetroArch presets carry it.
- **Reflection as on the screen** (Bloom & reflections) has the bezel reflect the picture with the screen's overscan
  and mask density, as Super Win the Game does, where the public source reverses the overscan and halves the mask's
  rows there. The **Super Win the Game** preset turns it on; with the overscan changed, the bezel's reflection now
  matches the game's within a fraction of a step. Off by default. RetroArch presets carry it.
- The screen's outline and darkened edges are the original mesh's, baked across the glass's UVs as the bezel is baked:
  its 192-sided outline and its 15 greys blended across each triangle, where a formula drew a smooth outline and
  shading before. Measured against the mesh as Super Win the Game draws it, the glass's shade moves from 0.83 steps
  off on average to 0.01, and the picture from 0.46 to 0.30. Renders move by about a tenth of a step on average, and
  by more on the outline's pixels. RetroArch exports carry the image as `glass.png`.
- **My presets** can be deleted, after confirming. Saving under a name already there, in any letter case, asks
  before replacing that preset, which keeps its name and description and takes the settings in use.
- **Flip artifact pattern** (Signal) draws the composite artifact pattern upside down, each tick blending the row above,
  as Super Win the Game's Linux build does; with it, the composite signal matches that build's frames within 2 steps.
  Off by default, as in the public source and the game's Windows build, which draws the pattern unflipped; the
  **Super Win the Game** preset leaves it off. RetroArch presets carry it.
- **NES palette** can be made **the game's** way, exactly as Super Win the Game makes it: every colour its Tint, Tint I and
  Tint Q give matches the game's own output, as does its 32-step table, which takes the NES palette the game's art is
  drawn in (beginning `7C7C7C 0000FC 0000BC`) onto them. The **Super Win the Game** preset uses it. **From the composite
  signal**, the palette until now, stays for art in MAME's NES palette, and looks saved with a palette keep it. The
  game's table is read as the game reads it, nearest in red and green and blended in blue, in the app and in RetroArch.
- **NTSC blending** works as Super Win the Game's NTSC Blending does, which the game's code shows: each tick shows half
  the setting of the other pattern, so the game's 0.35 mixes them 17.5/82.5 rather than 35/65, and 1 is their average.
  The **Super Win the Game** preset keeps 0.35 and now blends as the game does.
- Presets, projects, sessions and files with an embedded look are saved as settings version 2. Those saved before,
  as version 1 or with no version, open with their NTSC blending doubled, so they look as they did; a value past 0.5,
  which blended beyond the average, opens above the slider's 1 and keeps its picture too. Earlier versions of the app
  cannot open settings saved by this one. RetroArch presets read the setting the same way.

## v0.9.0

- The RetroArch presets draw the bezel, as the app does: the curved glass and its bezel, lit, with the screen's
  picture reflected in the bezel's edge. **Screen only**, the bezel's colour and its edge reflection are preset
  settings now.
- **File → Video test card** plays ten seconds of original pixel-art gameplay, drawn by the app, to see a look on
  moving pixels before exporting it to RetroArch. It needs no file and no FFmpeg, in the desktop and web apps alike.
- The app draws the bezel from the same baked surface as RetroArch, in one pass with the glass. Renders move by a
  fraction of a step across the bezel, and by more on a few pixels of its outline and the screen's corners.
- The bezel's colour setting is called **Bezel color**; presets keep its `frame_color` key and load as before.
- A project that holds the video test card opens only in this version or later.

## v0.8.0

- The CRT runs in RetroArch, over any core as it plays: `CRTSim-Renderer-v0.8.0-retroarch.zip` holds presets for
  the original's settings, the same in linear light, the general look and the NES palette. Copy its folder into
  RetroArch's `shaders` folder and set **Video → Scaling → Aspect Ratio** to **Full**. The bezel is not ported yet,
  so the presets draw the screen alone.
- **Export → RetroArch shader…** saves the look in use as such a preset, in the desktop and web apps alike, and
  `crtsim export-retroarch` does the same from the command line.
- The screen's glass is ray-traced as the sphere its mesh was cut from, as the RetroArch port draws it. Renders move
  by at most 3 steps where the mesh's flat triangles shaded its edges, and its corners are true curves now.
- Presets, projects and exports are written as before.

## v0.6.0

- Animated GIF and WebP open as animations, with frame navigation, playback and export. They are decoded without FFmpeg.
  Single-frame GIFs open as still images.
- Export to GIF and animated WebP with small defaults (640 px, 24 FPS, 10 seconds), a file-size estimate, and a
  confirmation above 25 MB.
- Sliders step with the arrow keys once clicked: Shift for larger steps, Alt for finer ones, Delete to reset.
- Nothing that worked before changes: presets, projects and exported videos are written as before, and batches still
  turn animated WebP into PNG.

## v0.1.1 hardening

- Video sequences reuse their signal, surface, bloom, depth and readback GPU resources instead of reallocating them for every frame.
- PNG exports can be cancelled between bounded GPU batches and before the atomic destination replacement.
- Videos use a valid container frame count immediately when one is available; formats without one retain the exact decoded-frame fallback.
- Video RGB-to-YUV conversion and stream metadata explicitly use limited-range BT.709.
- Export checks the required H.264 or VP9 encoder before starting and reports the executable or missing encoder clearly.
- JSON preset loading now uses one version-aware entry point so future migrations can be implemented consistently.

The preset schema remains version 1 and is compatible with v0.1.0 files.

The **Portable packages** workflow builds downloadable Actions artifacts for each manual workflow run, and for pull requests that change packaging: the workflow itself, `tools/package.py`, `tools/build_web.py`, `packaging/`, `ports/`, `web/` or `Cargo.lock`.
The Linux job also writes `CRTSim-Renderer-vX.Y.Z-retroarch.zip` and `CRTSim-Renderer-vX.Y.Z-reshade.zip`, the RetroArch
and ReShade presets its CLI exports, and the OBS job builds the filter into `CRTSim-Renderer-vX.Y.Z-obs-linux-x86_64.zip`.
Other pull requests are built and tested by the validation workflow only.
A `v*` tag builds the same packages and creates a **draft** GitHub release after every packaging job succeeds.
Review and publish the draft manually. This workflow does not create tags or merge pull requests.
The same workflow builds the web app into `CRTSim-Renderer-vX.Y.Z-web.zip` and `SHA256SUMS-web.txt`. Running it by
hand with `release_tag` set, such as `v0.7.0`, adds those two files to that existing release without rebuilding the
desktop packages. Once the release is published, the site's **Update CRTSim** workflow opens a pull request that
brings the web app and the new version to the site.

## Packages

| Platform | Package | Launch |
| --- | --- | --- |
| Windows x86_64 | Portable ZIP, desktop and CLI | Extract the entire ZIP; `crtsim-desktop.exe` is directly inside the extracted folder |
| Linux x86_64 | AppImage | Make executable, then open it |
| Linux x86_64 | Portable tar.gz, desktop and CLI | Extract, run `./crtsim-desktop` |
| macOS Apple Silicon | tar.gz with `.app`, desktop and CLI | Extract, open `CRTSim Renderer.app` |
| Web | ZIP of the web app, for a website to serve | Serve the files over HTTPS in a browser with WebGPU; the site does this at `aizumanga.neocities.org/crtsim/` |
| RetroArch | ZIP of shader presets for the shipped looks | Copy its `crtsim-renderer` folder into RetroArch's `shaders` folder; see the README inside |
| ReShade | ZIP of the effect and presets for the shipped looks | Copy its `reshade-shaders` folder into the game's folder and pick a preset in ReShade's overlay; see the README inside |
| OBS Studio 30, Linux x86_64 | ZIP of the filter plugin, its data and the shipped looks | Install it as the README inside says, then add **CRTSim Renderer** to a source's filters |

### With a package manager

Scoop and Homebrew install a release, and their own update commands bring it to the next one:

```sh
# Windows, Scoop
scoop bucket add crtsim https://github.com/aizumanga/CRTSim-Renderer
scoop install crtsim-renderer          # later: scoop update crtsim-renderer

# macOS (Apple Silicon), Homebrew
brew tap aizumanga/crtsim https://github.com/aizumanga/CRTSim-Renderer
brew install --cask crtsim-renderer    # later: brew upgrade --cask crtsim-renderer
```

This repository is the Scoop bucket (`bucket/`) and the Homebrew tap (`Casks/`). Once a release is published,
the **Package managers** workflow opens a pull request bringing both to it, written by `tools/manifests.py` from the
release's SHA256SUMS files; merging it offers the release. The same workflow writes the release's winget manifests
(`aizumanga.CRTSimRenderer`, a portable ZIP with `crtsim-desktop` and `crtsim` commands) and, when the repository
has a `WINGET_TOKEN` secret, submits them to winget's repository, after which `winget install aizumanga.CRTSimRenderer`
and `winget upgrade` work too. Without the secret, the manifests are kept as the run's `winget-manifests` artifact.

No Rust installation or repository checkout is required. Effects/assets are embedded.
Each archive includes the README, project license, original asset provenance, dependency license texts/index and commit identifier.
SHA256SUMS files accompany downloads. Builds are not claimed to be byte-for-byte reproducible.

### Linux

AppImages are built on Ubuntu 22.04 (glibc 2.35 baseline). They bundle selected windowing libraries;
your host must still provide a working display session, graphics drivers and desktop file portal.
The tarball uses the host windowing libraries as described in the README. These are portable binaries, not fully static builds.
The AppImage builder is the upstream linuxdeploy continuous release; its downloaded SHA-256 is recorded in the build artifacts.
Graphics drivers, FFmpeg and ffprobe are not bundled. The host FFmpeg runs outside the AppImage's library search path.

```sh
chmod +x CRTSim-Renderer-*-linux-x86_64.AppImage
./CRTSim-Renderer-*-linux-x86_64.AppImage
```

If FUSE is unavailable, use `APPIMAGE_EXTRACT_AND_RUN=1 ./CRTSim-Renderer-...AppImage`
or `--appimage-extract` and launch `squashfs-root/AppRun`.
Pass `--cli` to the AppImage to run the command-line renderer, e.g. `./renderer.AppImage --cli inspect-meshes`.
The AppImage carries its update information, and each release publishes its `.AppImage.zsync` beside it, so
[AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate), Gear Lever or AppImageLauncher can bring it to
the latest release in place, downloading only what changed. The app itself never does
([ADR 4](adr/0004-check-for-updates-never-download-them.md)).
The AppImage is launched under software Vulkan/Xvfb in CI before its artifact is uploaded.

### Windows and macOS

Windows builds statically link the C runtime; no separate Visual C++ redistributable is needed for that runtime.
Windows ZIP contents are stored at the archive root so extraction does not create a redundant second package folder.
The application is unsigned and Windows may display a download warning.
macOS is optional/provisional: Apple Silicon only, ad-hoc signed for execution, without Developer ID signing or notarization.
Gatekeeper may require approval through System Settings → Privacy & Security. Real Mac runtime testing remains necessary.
A Windows installer, Intel/universal Mac builds and Apple signing are deferred.

### Video setup and user data

Install FFmpeg and ffprobe separately; **Export → FFmpeg setup…** gives the command for your system. Put both on PATH,
in a folder named `ffmpeg` next to the app (the Windows ZIP and Linux tarball), or set `CRTSIM_FFMPEG` and
`CRTSIM_FFPROBE` to their full paths. On macOS, Homebrew's folders are searched even when the app is opened from Finder.
They must include the codecs described in [VIDEO_PIPELINE.md](VIDEO_PIPELINE.md). Image rendering works without FFmpeg.
There are no runtime downloads; the desktop reaches the network only when asked to check for a newer release. Personal presets remain in the normal per-user app-data directory;
updating or replacing a portable package does not delete them. Use `CRTSIM_DATA_DIR` for an explicit alternate directory.

## crtsim-core and crtsim-bevy on crates.io

The renderer is published as the [`crtsim-core`](https://crates.io/crates/crtsim-core) crate, for Rust programs to
depend on: stills, video sequences, and `Renderer::encode` for a game's or an engine's own frames. It is versioned
with the app, from v1.5.0: each release's crate is the renderer that release's app draws with. Semantic versioning
then holds for both, so a change that breaks a caller of the crate needs a new major version of the app too.

The Bevy plugin is published beside it as [`crtsim-bevy`](https://crates.io/crates/crtsim-bevy), at the same
version, asking for that release's `crtsim-core`; a test fails when the two versions part. It runs on Bevy's own wgpu
device, so it follows a Bevy release only once the renderer is on that release's wgpu.
Everything it embeds, its shaders, the original meshes and textures, and the NES LUTs, lives in its folder,
`crates/crtsim-core`, so the crate builds from its package alone; CI packages it on every change to prove that.

A crate version cannot be taken back, so they are published when a person publishes the release, not when its tag
is pushed: the `publish-core.yml` workflow runs on the release's publication, checks out its tag, checks the tag
names both crates' version, tests them and publishes the renderer and then the plugin, with a token crates.io issues
for that run alone. A version already on crates.io is left as it is. To set it up, for each crate:

1. The first version only: trusted publishing cannot make a crate, so the workflow leaves a crate crates.io does
   not have yet with a warning. Make a crates.io API token (Account Settings → API Tokens) with the `publish-new`
   scope, limited to that crate and expiring soon. Either put it in the `crates-io` environment's secrets as
   `CRATES_IO_FIRST_PUBLISH_TOKEN` and run the workflow by hand with the release's tag, or, from the release's tag,
   run `cargo login` with it and `cargo publish -p <crate>` (`--no-verify` skips the build CI has already checked).
   Then delete the token, and the secret.
2. Add this repository's `publish-core.yml` workflow as the crate's trusted publisher on crates.io (crate settings →
   Trusted Publishing, environment `crates-io`), and make a `crates-io` environment in the repository's settings.
3. From then on, publishing a release publishes its crates with no token. Running the workflow by hand with a
   release's tag publishes that one's, where crates.io does not have them already.

## The Godot Asset Library

The [Godot Asset Library](https://godotengine.org/asset-library/), which Godot's AssetLib tab searches, does not
host files: it downloads a commit of a repository as GitHub zips it. So publishing a release also runs the
`godot-asset.yml` workflow, which commits the release's `-godot.zip` to the `godot-asset-library` branch: the
add-on's `addons/crtsim_renderer` folder, with its licence, a README and the library's icon. Its `.gitattributes`
leaves everything but `addons` out of the download, so the add-on installs into a project as it is. The run's
summary names the commit. The library takes a new version from a person, reviewed by its moderators:

1. The first time only: log in at the Asset Library, choose **Submit Asset**, and fill it in:

   | Field | Value |
   | --- | --- |
   | Asset Name | CRTSim Renderer |
   | Category | 3D Tools (an Addon category; Shaders also fits) |
   | Godot version | 4.3 |
   | Version | The release's, without its `v`, such as `1.6.0` |
   | Repository host | GitHub |
   | Repository URL | `https://github.com/aizumanga/CRTSim-Renderer` |
   | Issues URL | Empty: the repository's own |
   | Download Commit | The commit the workflow's summary names |
   | Icon URL | `https://raw.githubusercontent.com/aizumanga/CRTSim-Renderer/godot-asset-library/icon.png` |
   | License | CC0 1.0 Universal |
   | Description | `ports/godot/README.md`'s opening paragraphs and its **Using it**, as plain text |

   Up to three previews can show the effect, as images by their raw.githubusercontent.com address.
2. Each release: on the asset's page, **Edit**, and set Version and Download Commit to the new release's. The edit
   waits for a moderator's approval, as the first submission does.

Running the workflow by hand with a published release's tag commits that release's add-on, if the branch does not
hold it already.

## itch.io

Publishing a release also pushes its packages to itch.io, with [butler](https://itch.io/docs/butler/), from the
`itch.yml` workflow: each package to a channel of its own, unpacked as people use it, so the itch app installs,
launches and updates the desktop app, and butler uploads only what changed since the last release. Each channel's
version on itch.io is the release's, without its `v`. `tools/itch.py` lays the channels out from the release's
downloads, after their SHA256SUMS are checked:

| Channel | From | Holds |
| --- | --- | --- |
| `windows` | `-windows-x86_64.zip` | The app and command line; the itch app launches `crtsim-desktop.exe` |
| `mac` | `-macos-arm64.tar.gz` | `CRTSim Renderer.app` and the command line; the itch app launches the app |
| `linux` | `-linux-x86_64.AppImage` | The AppImage, which the itch app launches |
| `web` | `-web.zip` | The web app, to play in the browser |
| `editors-windows`, `editors-mac`, `editors-linux` | `-editors-<system>.zip` | The OpenFX effect and the frei0r filter |
| `aseprite` | `.aseprite-extension` | The Aseprite extension, as the file Aseprite installs |
| `retroarch`, `reshade`, `obs-linux`, `godot` | Their ZIPs | The shader presets, the OBS filter and the Godot add-on |

To set it up, once:

1. Make the project on itch.io, as a tool, and make an API key for butler in itch.io's settings (API keys).
2. In the repository's **Settings → Secrets and variables → Actions**: on the **Secrets** tab, a repository secret
   `BUTLER_API_KEY` holding the key; on the **Variables** tab, a variable `ITCH_GAME` naming the project as
   `user/project`, as its page's address `https://user.itch.io/project` has it. A variable `ITCH_CHANNELS`, such as
   `windows mac linux`, pushes only those channels. The workflow runs in an `itch-io` environment, which GitHub makes
   the first time; the secret can go there instead, to limit which branches and tags may use it.
3. After the first push, edit the project on itch.io: mark the `web` upload **This file will be played in the
   browser** if the page should run the web app (it needs a browser with WebGPU), and the others' descriptions as
   wanted. Later pushes keep those settings.

Without `ITCH_GAME` the workflow does nothing; pre-releases are left off itch.io. Running it by hand with a published
release's tag pushes that release, as when a push failed or for the releases before it, whose packages it has.

## Local packaging

Build with `cargo build --release --locked -p crtsim-app -p crtsim-cli`, then run
`python tools/package.py --platform linux-x86_64 --version preview-local` (or the matching Windows/macOS platform).
Run on the matching native OS. A clean `dist/` staging area is required. The workflow contains the additional AppImage build command.
