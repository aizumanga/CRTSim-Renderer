# crtsim-core

[J. Kyle Pittman's CRTSim](https://github.com/MinorKeyGames/CRTSim) on [wgpu](https://wgpu.rs): the CRT of
*Super Win the Game*, as its public CC0 source describes it. A picture goes through composite artifacts and
ringing, a signal whose glow persists from frame to frame, the curved glass and its bezel (ray-traced, from the
original meshes), the shadow mask, lighting, edge reflections and bloom.

This is the renderer behind [CRTSim Renderer](https://github.com/aizumanga/CRTSim-Renderer), its desktop and
web apps and its command line. It is not an official product and is not endorsed by Pittman; please support
him by buying [his games](https://store.steampowered.com/developer/MinorKeyGames).

```rust,no_run
use crtsim_core::{config::Config, input, presets, Renderer};

fn main() -> anyhow::Result<()> {
    let renderer = pollster::block_on(Renderer::new(wgpu::Backends::PRIMARY))?;
    let picture = input::load_image("game.png".as_ref())?;
    // The original's settings; `presets::included()` lists the app's looks.
    let look = presets::named("NTSC 240p").map_or_else(Config::default, |p| p.config);
    let crt = pollster::block_on(renderer.still(&picture, &look, None, |_| {}))?;
    crt.save("crt.png")?;
    Ok(())
}
```

- **Stills**: `Renderer::still`, from cleared history after a warm-up, so the picture already has the glow
  of one that has been on screen.
- **Video**: a `Sequence::video` carries the CRT's history from frame to frame through `Renderer::frame`, and
  `Renderer::read` reads each back.
- **In a game or an engine**: `Renderer::encode` records a `Sequence::host` frame into your own command
  encoder, from your texture into your target, waiting for nothing. Build the renderer on your device with
  `Renderer::with_device`, requested with at least `Renderer::limits`.
- **Settings** are one `Config`, the same JSON the app saves as a preset. `settings::SETTINGS` describes each
  one once, with its name and limits, for a settings screen.

It needs a GPU with Vulkan, DX12 or Metal, or WebGPU in a browser; it requests no optional features.

## Licence

CC0 1.0, as Pittman's reference is. The original meshes, mask and artifact textures are embedded unmodified
(`assets/original-crtsim`, with their provenance), as are 38 CC0 NES LUTs from MAME Goodies
(`assets/nes-luts`). Dependencies keep their own licences.
