# Further uses: games, browsers and other hosts

How the simulation could run outside this app, and what each route needs from the code as it is.

## What there is to reuse

- **The effect is a short chain of GPU passes** in `crates/crtsim-core/shaders/crtsim.wgsl`, run by `crtsim-core`:
  1. Prepare (`prepare.wgsl`): resize, crop, LUT or palette, grade.
  2. Composite, at signal size. It reads the previous tick's output, which gives persistence.
  3. The curved glass and its bezel, ray-traced together in one full-screen pass at output size.
  4. Bloom, down then up.
  5. Present.

  That makes six to eight passes a frame, of which only composite depends on the frame before.
- **The assets are small and CC0**: two meshes (180 KB), the mask and artifact textures, and 38
  NES LUTs, all embedded. Pittman's reference is CC0, as is this port, so any engine or store can
  use it; credit him as the app does.
- **Real time already works**: a `Sequence` carries history from frame to frame, which is how
  the desktop plays video. A game is the same loop with one tick per frame and no warm-up.

What stands in the way today, in `crtsim-core`:

| Obstacle | Where | Why it matters |
| --- | --- | --- |
| ~~The input is a CPU `RgbaImage`, uploaded each frame~~ Done: `Renderer::encode` reads the host's texture | `Renderer::frame` | A game's frame is already a GPU texture. The round trip costs more than the effect. |
| ~~The renderer submits and waits~~ Done: `Renderer::encode` records into the host's encoder and waits for nothing. `frame` and `read` still submit and wait, as the app's own renders need | `lib.rs`, `gpu.rs` | A host records into its own frame and never waits on the GPU. |
| ~~A new uniform buffer and bind group for every tick and pass~~ Done: one buffer per sequence, bind groups made with it | `Workspace` | This was fine offline, but allocated 60 or more times a second in a game. |
| ~~wgpu 0.19, pinned by eframe 0.27~~ Done: wgpu 30 and eframe 0.36 | `Cargo.toml` | Current Bevy and wgpu releases were far newer. |

## The enabling step, for every route

Done: one entry point records a frame into the host's encoder, from the host's texture, into the
host's target, submitting nothing and waiting for nothing:

```rust
let mut sequence = Sequence::host();
// Each frame, then submit the encoder as the host always does:
let frame = HostFrame { source: &game_texture, target: &surface_texture, seconds: 1. / 60. };
renderer.encode(&mut encoder, &mut sequence, &frame, &config)?;
```

A host's sequence starts without warm-up, so the glow builds as a set's would, runs one tick a
frame, and decays each frame's glow by how long the frame before it lasted. A target of another
size keeps the glow; a new signal size or color mode starts again. Textures of sRGB formats read
and write as the colors they encode. A test holds it to what `frame` draws, byte for byte.
`Renderer::with_device` builds the renderer on the host's own device.

## Route 1: in the browser

**Fit: excellent.** The shaders are already WGSL, WebGPU's own language, and wgpu compiles to
WebAssembly. Chrome and Edge ship WebGPU, as do Safari 26 and Firefox on Windows. wgpu's WebGL2
backend could cover older browsers for the gamma-space mode.

- A `crtsim-web` crate (wasm-bindgen) with `CrtSim.create(canvas)`, `setConfig(json)` taking
  the same preset JSON, and `render(source)`.
- Sources come straight from the browser, without decoding in Rust: images via
  `createImageBitmap`, and video, webcam or `<canvas>` via `copyExternalImageToTexture` each
  frame. A live CRT on a playing video or a webcam is the obvious demo.
- It draws to the canvas surface, so no readback is needed to view it. Saving a PNG uses one
  async `map_async`, not `poll(Wait)`.
- Video export in the browser would use WebCodecs `VideoEncoder` plus a small MP4 or WebM
  muxer, not FFmpeg. ffmpeg.wasm works, but it is tens of megabytes and slow.
- The whole egui interface could also build for the web (eframe supports it), with presets in
  IndexedDB and files through the browser's pickers. This is the route taken: see
  [ADR 2](adr/0002-the-web-app-is-the-whole-app-on-webgpu.md) and the [web plan](WEB_PLAN.md).

Expect wgpu plus the embedded assets to come to a few megabytes of wasm. Measure it before
promising anything.

## Route 2: in games

Three depths, from least to most work per engine:

**a. Rust engines that use wgpu (Bevy, custom renderers).** These are the most direct. A Bevy
plugin adds a post-processing node that calls `encode` with the camera's output. Each camera has
its own `Sequence`, which must be reset on resize. Players then get the same presets JSON as the
app, and a settings screen can reuse `crtsim_core::settings::SETTINGS`: names, ranges and
defaults, already described once.

**b. Emulators and any PC game: RetroArch slang and ReShade.** This is where most CRT-shader
users are. Neither can draw arbitrary meshes. The glass needs none: it is a cap of a sphere, which the
renderer now ray-traces in a full-screen pass. The bezel is an irregular mesh, so its surface is
baked into textures, seen straight on, and each pixel steps a ray across it. Every stage then
becomes a full-screen pass. naga,
already a dependency, translates WGSL to GLSL and HLSL as a starting point. The history target is
slang's `PassFeedback` and a persistent texture in ReShade.

- RetroArch: a `.slangp` preset. Its reach is every libretro core, which suits the NES LUTs and
  Super Win the Game's palette.
- ReShade: an `.fx` file that works in almost any DirectX, Vulkan or OpenGL game.

**c. Other engines (Godot, Unity, Unreal, GameMaker).** Port the baked full-screen passes to
the engine's shader language:

- Godot 4: `CompositorEffect`.
- Unity: a URP `ScriptableRendererFeature`.
- Unreal: a post-process material, with a render target for history.

A native C API (`crtsim-ffi`) over `encode` is possible, but sharing GPU textures between an
engine's device and wgpu's needs platform-specific interop (Vulkan external memory, DX12 shared
handles). Porting the shaders is less work and more robust.

## Other hosts worth considering

- **OBS Studio**, for streamers. A native filter plugin keeps history between frames, which the
  generic shader-filter plugin cannot. Done: as a shader port running the shared passes, not the
  renderer ([ADR 5](adr/0005-the-ports-share-one-translation-of-the-passes.md)).
- **Video editors via OpenFX** (DaVinci Resolve, Natron, Nuke). This matches the app's strength:
  deterministic, high-quality offline renders. Editors do not render frames in order, so a
  plugin must warm up from a few frames earlier each time, as stills already do.
- **A headless service**: the CLI with software Vulkan (lavapipe), as CI already runs it, can
  power a chat bot or batch service. Only resource limits and a queue are missing.
- **Publishing `crtsim-core` to crates.io**, so Rust projects can depend on it directly. Ready:
  it packages and builds from its package alone, and `publish-core.yml` publishes a version
  tagged for it (see [releases](RELEASES.md#crtsim-core-on-cratesio)).

## Suggested order

Decided since: games get shader ports, RetroArch first and then ReShade, with the renderer's
glass made full-screen as their reference ([ADR 3](adr/0003-games-get-shader-ports-not-the-renderer.md)),
and every port shares one translation of the passes, OBS's filter included
([ADR 5](adr/0005-the-ports-share-one-translation-of-the-passes.md)).

| Step | Work | Unlocks |
| --- | --- | --- |
| 1 | ~~`Renderer::encode`, a persistent uniform buffer, no waits; upgrade wgpu and eframe~~ Done | Everything below. The app's preview gets faster too. |
| 2 | `crtsim-web` with an image and webcam demo page | The widest audience, with WGSL as it is. |
| 3 | ~~Baked mesh textures and a full-screen-only variant, checked against the goldens~~ Done: the glass is ray-traced and the bezel baked, in one full-screen pass | Every engine without mesh support. |
| 4 | ~~RetroArch slang preset, then ReShade~~ Done, sharing their passes | Emulators and existing PC games. |
| 5 | Bevy plugin; ~~OBS filter~~ Done, as a shader port | Rust games; live streams. |

Visual parity with the mesh renderer has to be checked for step 3. The golden-image tests
already compare renders pixel by pixel, so the baked variant can be held to the same fixtures
before it ships anywhere.
