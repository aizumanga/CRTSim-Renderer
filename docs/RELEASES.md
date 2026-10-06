# Portable releases (Phase 4)

## Unreleased

- **Flip artifact pattern** (Signal) draws the composite artifact pattern upside down, each tick blending the row above,
  as Super Win the Game's Linux build does; with it, the composite signal matches that build's frames within 2 steps.
  Off by default, as in the public source. RetroArch presets carry it.
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
The Linux job also writes `CRTSim-Renderer-vX.Y.Z-retroarch.zip`, the RetroArch presets its CLI exports. Other pull requests are built and tested by the validation workflow only.
A `v*` tag builds the same packages and creates a **draft** GitHub release after all three packaging jobs succeed.
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
There are no automatic runtime downloads. Personal presets remain in the normal per-user app-data directory;
updating or replacing a portable package does not delete them. Use `CRTSIM_DATA_DIR` for an explicit alternate directory.

## Local packaging

Build with `cargo build --release --locked -p crtsim-app -p crtsim-cli`, then run
`python tools/package.py --platform linux-x86_64 --version preview-local` (or the matching Windows/macOS platform).
Run on the matching native OS. A clean `dist/` staging area is required. The workflow contains the additional AppImage build command.
