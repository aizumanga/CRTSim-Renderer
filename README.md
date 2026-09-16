# CRTSim-Renderer

A native, local renderer based on [J. Kyle Pittman's public CRTSim implementation](https://github.com/MinorKeyGames/CRTSim).
Phase 2 expands the **native desktop still-image application** with a persistent preset gallery, credits, progress and optional rendering refinements.
The goal is to preserve the shared effect and make it useful for images and, later, video.
It is not an official product or an exact reconstruction of a commercial game's shader.

## What works in this prototype

- PNG/JPEG/WebP/BMP input and PNG export; an original test card when input is omitted.
- The original curved-screen and frame meshes, including colors, normals, UVs and reflection weights.
- Composite artifacts, horizontal ringing, separate RGB persistence, shadow mask, lighting, edge reflections and bloom.
- Deterministic still jobs, cleared feedback buffers, phase A/B/stable/alternating selection and configurable warm-up.
- Logical signal and output resolution presets; custom dimensions; JSON settings.
- Clean/signal/full-CRT debugging and mesh validation/export without a GPU.
- Native wgpu backends: Vulkan on Linux, DX12 on Windows and Metal on macOS.

Video, audio, installers and the unpublished NES palette LUT are not included yet. Platform compilation does not prove visual parity between drivers.
See [the implementation plan](docs/PLAN.md) and [validation notes](docs/VALIDATION.md).

## Desktop app

On first launch, the app honestly describes this AI-assisted project as “what some would call \"vibe-coded slop\", built based on J. Kyle Pittman's public CRTSim”.
Acknowledging that message is remembered on this computer. **Credits** always reopens the attribution and links to
[J. Kyle Pittman's itch.io page](https://piratehearts.itch.io/) and [Minor Key Games on Steam](https://store.steampowered.com/developer/MinorKeyGames).
Please support the original developer by buying and playing their games. This port is unofficial and is not endorsed by him.
The welcome and credits also link to Pittman's
[CRT Simulation in Super Win the Game](https://www.gamedeveloper.com/programming/crt-simulation-in-super-win-the-game),
an interesting technical account of how the effect developed.

**Preset gallery** includes General image, Original CRTSim, Soft television, Clean RGB, Pixel art 240p, Warm analog and Linear light.
Enter a name and choose **Save current** to add your exact settings to **My presets**; they reappear after restarting.
To add an existing JSON, load it, then use Save current in the gallery. JSON export remains available for sharing.
Names are never overwritten; malformed files are skipped with an explanation. Personal JSON files live outside the checkout,
in the app data directory shown in the gallery. `CRTSIM_DATA_DIR` can override that directory with an absolute path.
Only explicitly saved presets and the welcome acknowledgement persist; unsaved edits are not automatically saved on exit.

Full-resolution export shows stage-based progress; routine live-preview updates stay unobtrusive. Warm-up advances after completed GPU batches,
followed by surface rendering/readback and PNG saving.
Percentages represent weighted work stages, not remaining seconds; PNG encoding stays at its stage until the file is completely saved.

**Filter mask when shrinking** uses a mipmapped mask to reduce minification aliasing. New general-image presets enable it;
Original CRTSim and old JSON files preserve the original sampling. Display resizing can still introduce moiré.
**Optional color grade** rotates hue and changes chroma in YIQ before the composite simulation. Neutral values leave the prepared signal unchanged.
It is an artistic grade, not the private NES palette LUT or a full NTSC decoder.
**Linear light (experimental)** decodes the SDR signal for glass sampling, performs lighting and bloom with RGBA16Float intermediates,
then encodes the final SDR PNG. Composite/persistence remains gamma-space RGBA8. It uses more GPU memory and changes the look;
it is not a full linear-light pipeline, HDR export, ICC color management or an exact match to the game.
Old presets load with reference processing and neutral grading. Newly saved presets require the updated app/CLI.

With stable Rust installed, run from the repository folder:

```sh
cargo run --release --locked -p crtsim-desktop
```

Use **Open image** or drag one PNG/JPEG/WebP/BMP into the window. Adjust the controls on the left;
the preview refreshes after you finish a drag or pause typing. **Export PNG** renders using the export size,
even when the preview is smaller. The window remains responsive while loading, rendering and exporting.
Settings changed during an export apply to the next export. The native save dialog asks for confirmation when a desktop PNG or JSON preset
already exists; after confirmation, the app writes a complete temporary file and atomically replaces the destination.

- **General image** starts with square pixels, smooth resizing, contain fitting and saturation 1.0.
- **Original CRTSim** restores the public-reference defaults, including 256x224 signal resampling and saturation 1.35.
- **Original / CRT / Side by side** compares the source with the rendered tube. Comparison is not geometrically aligned because the CRT bends the image.
- **Fast / Balanced / Export resolution** changes preview canvas resolution only. Use Export resolution and turn off Fit view at 1x zoom to inspect mask sampling.
- **Load / Save preset** uses the same version-1 JSON format as the CLI. **Undo / Redo / Reset** acts on settings, not source files or exported files.
  `Ctrl+Z` and `Ctrl+Shift+Z` provide undo and redo shortcuts (`Command` equivalents are also accepted on macOS).
- Signal and export size boxes accept named presets or custom `WIDTHxHEIGHT`. Resolved dimensions and crop/mask warnings are shown in the window.

The original-image display is limited to a 2048-pixel thumbnail; export always uses the loaded source.
Preview rendering shares the CLI core but is a debounced still-image render, not a continuous 60 FPS simulation.
The CRT mask can look different at different preview sizes; exports retain the requested resolution.
Preset changes are kept in memory until saved; the app does not silently write settings on exit.

Linux needs working OpenGL for the window and Vulkan for the CRT renderer, plus an X11 or Wayland session.
X11 also needs the xkbcommon X11 library (`libxkbcommon-x11` on Arch, `libxkbcommon-x11-0` on Debian/Ubuntu).
The native file picker uses the desktop portal. On Arch/KDE, ensure `xdg-desktop-portal` and `xdg-desktop-portal-kde`
are installed and working in your logged-in desktop session. Drag-and-drop or passing an image path also works:

```sh
cargo run --release --locked -p crtsim-desktop -- "image.png" --backend vulkan
```

Windows uses the system file picker and normally DX12 for rendering; macOS uses its system picker and Metal.
The window itself uses OpenGL through eframe. macOS runtime support remains provisional until tested on a real Mac.
For Linux source-build errors about windowing libraries, install your distribution's Wayland and xkbcommon development packages
(Debian/Ubuntu: `libwayland-dev libxkbcommon-dev libegl1-mesa-dev`; Arch: `wayland libxkbcommon`).

### Graphics-card compatibility

The renderer is not NVIDIA-specific. It requests no optional wgpu device features and is expected to work on AMD, Intel, NVIDIA and Apple GPUs
when the operating system has a working Vulkan, DX12 or Metal driver. Integrated graphics should also work, although large exports and the
experimental linear-light mode need more memory and may be slower. The adapter's maximum texture dimension is checked before rendering.

CI compiles the application for Windows, Linux and macOS and executes the GPU pipeline through Linux software Vulkan. That proves the shaders
do not depend on one vendor, but it is not a substitute for runtime tests on several physical GPUs and drivers. Older hardware without a usable
Vulkan/DX12/Metal implementation will show a compatible-adapter error; it is not silently switched to a CPU renderer.

## Build and try

Install stable Rust through [rustup](https://rustup.rs/), then:

```sh
cargo run -p crtsim-cli -- presets
cargo run -p crtsim-cli -- inspect-meshes
cargo run --release -p crtsim-cli -- render --output test-crt.png
```

The last command uses the built-in test card and the public-reference defaults, with stable artifact phase.
Files are **never overwritten**: choose a new output name if it already exists. Parent directories must exist.
All assets are embedded; the executable does not download anything at runtime.

### Your own image

For modern illustrations, photos or screenshots, start with square pixels and contain fitting:

```sh
cargo run -p crtsim-cli -- config --general --output general.json
cargo run --release -p crtsim-cli -- render --input "image.png" --config general.json --output rendered.png
```

Edit `general.json` to adjust the effect. It is not necessary to rebuild after editing settings.
Transparency is composited onto black before filtering; output is opaque SDR PNG. ICC/HDR color management is not implemented.
The default path follows the original gamma-space UNORM behavior, not linear-light rendering.

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
Use `config --general` for normal images; plain `config` generates the original-style configuration (8:7 pixel aspect, nearest filtering).
`fit` controls placement on a fixed 4:3 tube: `reference`, `contain`, `cover`, or `stretch`.
Contain mode prevents rectangular aspect cropping, but the rounded glass and barrel distortion can still hide extreme corners.
A widescreen output canvas does not turn the tube itself into a widescreen tube.

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
Original assets and shader source remain unmodified in `assets/original-crtsim/`; provenance is in [SOURCES.md](assets/original-crtsim/SOURCES.md).
The WGSL translation and application additions use this repository's CC0 license. Dependencies keep their own licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
