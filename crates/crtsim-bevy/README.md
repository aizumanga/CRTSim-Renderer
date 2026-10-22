# crtsim-bevy

J. Kyle Pittman's CRTSim in [Bevy](https://bevy.org): each camera holding a `Crt` has its
picture drawn through the CRT every frame, with composite artifacts, persistence, a
ray-traced curved screen and bezel, shadow mask and bloom. It runs
[`crtsim-core`](https://crates.io/crates/crtsim-core)'s own renderer on Bevy's own device,
recording into Bevy's frame, so a game shows exactly the picture the
[CRTSim Renderer app](https://github.com/aizumanga/CRTSim-Renderer) shows, with every look
and setting it has.

```rust,no_run
use bevy::prelude::*;
use crtsim_bevy::{Crt, CrtPlugin};

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, CrtPlugin))
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((Camera2d, Crt::preset("NTSC 480i").unwrap()));
        })
        .run();
}
```

`cargo run -p crtsim-bevy --example tube` shows a few moving blocks under the CRT; the
arrow keys go through the included looks.

## Looks

- `Crt::preset(name)` takes any included look, as the app's gallery and
  `crtsim render --preset` name them (`crtsim_core::presets::included()` lists them).
- `Crt::from_json(bytes)` takes a look the app or the command line saved.
- `Crt::new(config)` takes any `crtsim_core::config::Config`. Change the component's `config`
  at any time and the next frame draws with it. A settings screen can offer
  `crtsim_core::settings::SETTINGS`, which names every setting with its range and default.

The look's output size is the camera's own. A wide window shows the original 4:3 tube in its
middle; set `config.tube_aspect` to `16. / 9.` for a wide tube filling it.

## How it draws

- The CRT draws after Bevy's own post-processing (tonemapping and anti-aliasing), and with the
  default `ui` feature after the camera's interface too, so a HUD is on the tube as a set
  shows it. Without that feature the interface is drawn over the tube.
- Each camera is a host sequence: no warm-up, so the glow builds as a set's would, and each
  frame's glow decays by how long the frame before lasted. A new window size goes on with the
  glow; a new signal size or colour mode starts it again.
- HDR cameras work as others do: the CRT reads and draws Bevy's float target as linear light.
- A frame the CRT cannot draw, such as one with settings it refuses, is shown as Bevy drew it,
  and the error is logged once.

## Versions

| crtsim-bevy | Bevy | wgpu |
| --- | --- | --- |
| 1.6 | 0.20 | 30 |

`crtsim-bevy` is versioned with the app and `crtsim-core`, and published with each release.
It runs on Bevy's own wgpu device, so it follows a Bevy release only once the renderer is on
that release's wgpu.

Licensed CC0-1.0, as the rest of the renderer is.
