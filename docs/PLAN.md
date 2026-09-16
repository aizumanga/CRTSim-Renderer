# Implementation plan

## Scope and authority

Start with the public CC0 reference, not an exact clone of Super Win the Game's controls or unpublished final shaders.
Do not contact the upstream author or extract commercial game resources as a development dependency.
The video demonstrates adjustable effects; it is not an interface specification.

## Phase 0: current implementation

- Rust core library plus CLI, no UI or video.
- Embedded, pinned upstream meshes and textures; validated M3D reader and lossless attribute export to JSON.
- WGSL composite -> curved screen/frame -> bloom downsample/upsample -> present.
- Gamma-space RGBA8 intermediate targets match the public reference's broad numerical behavior.
- Source-sized horizontal samples replace the hardcoded 1/256 step.
- The original NTSC texture tiles in signal pixels; mask density is separately configurable.
- Black-border bilinear sampling is implemented explicitly, avoiding an optional GPU border-sampler feature.
- Feedback textures start cleared; one immutable uniform buffer per tick keeps phase ordering deterministic.
- Stable phase default; A/B and alternating available. Warm-up is user-controlled, not proof of convergence.
- Original-style and general-image config defaults, named signal/output resolutions, fit/filter/aspect options.
- Unit tests and explicit GPU smoke tests; platform build checks separate from visual parity checks.

## Deliberate corrections to preliminary planning

- No 25-control game-menu recreation is required.
- Do not automatically infer pixel art from image height. Users select nearest vs Lanczos.
- No RGB-to-YIQ round trip is presented as the missing game palette/LUT.
- Gamma-space and 8-bit reference behavior remain the compatibility default. Phase 2 adds an experimental linear-light surface/bloom mode.
- 240p/480p signal presets are logical progressive resolutions, not interlacing or a PAL/NTSC broadcast emulator.
- NES pixel aspect is preset-specific, not a rule for all images. Generic content uses square pixels.
- The article's overscan discussion describes a chosen presentation, not a universal guarantee that every CRT always hides exactly eight rows.
- No claim that a four-frame warm-up converges. Alternating phase can form a two-state cycle.
- Published memory/performance estimates must be measured before being advertised as guarantees.

## Reference vs adaptations

Public reference defaults are preserved where practical, but stable phase, safety validation, dynamic dimensions, black-luma division protection,
and the new GPU API are intentional adaptations. The camera retains a 4:3 physical tube on an independently sized output canvas.
Mask mipmapping/minification and exact D3D9 rasterization differ; visual equivalence must be reviewed before naming the port faithful.
Reference screenshots from the commercial game are not golden test data.

## Phase 1: desktop implementation

Native eframe desktop preview uses the same core. Image loading, rendering and PNG export run on a background worker.
Only one preview can be in flight; revision numbers reject stale results and the next request uses the latest settings.
Preview requests are debounced and wait until a slider drag finishes. Preview resolution is independent of export resolution.
Exports capture the image and settings when the file dialog completes and publish complete PNGs without overwriting files.

Implemented controls include before/after side-by-side views, zoom, named/custom resolutions, fitting, filtering, pixel aspect,
all existing shader parameters, temporal warm-up, phase selection, shared JSON presets and settings undo/redo/reset.
General-image defaults use saturation 1.0; the original reference preset remains explicit. Color management remains future work.
The UI displays resolved sizes and warnings for cropping, insufficient mask sampling and large jobs.
This phase does not promise continuous real-time frame rates, cancellable GPU submissions or packaged installers.

## Phase 2: still-image expansion and polish

- First-launch acknowledgement and permanent credits place J. Kyle Pittman's original work first, with the requested honest AI-assisted-project wording and game-store links.
- Included presets and personal JSON presets stored outside the checkout; current settings can be saved directly into the gallery.
- Progress based on completed warm-up batches plus surface/readback/save stages, independently labeled for previews and exports.
- Optional mipmapped mask filtering; legacy sampling is retained for old/reference presets.
- Optional YIQ hue/chroma grade, explicitly not the unpublished NES LUT or a complete NTSC decoder.
- Experimental linear-light glass/lighting/bloom with float intermediates and SDR output. Composite/history stays in gamma space.
- Existing frame lighting, reflection attributes, mask density, geometry and resolution controls remain shared with the CLI.

## Phase 3 and later

- Video streaming through FFmpeg with audio remux, cancellation and timestamp-based simulation.
- Independent simulation clock, fixed 60 Hz mode and a stable-artifact source-rate mode. Do not advance persistence according to wall-clock export speed.
- Decay correction alone is not an exact correction for the spatial feedback filter; test 24/25/30/50/59.94/60 FPS and seeking/preroll.
- Full color management, optional LUT import, batch jobs, wider tube geometry and packaging remain later work.
- No default synthetic interlacing, VHS noise, sprite flicker or room reflection.

## Sources

- [Public reference](https://github.com/MinorKeyGames/CRTSim/tree/dbbe9d1bc2512288f5b1747e6be35ff44a7baae1)
- [Pittman's technical article](https://www.gamedeveloper.com/programming/crt-simulation-in-super-win-the-game)
