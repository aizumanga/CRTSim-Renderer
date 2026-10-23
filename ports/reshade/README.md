# CRTSim Renderer for ReShade

The CRT of [CRTSim Renderer](https://github.com/aizumanga/CRTSim-Renderer) as a ReShade effect:
the composite signal with its artifacts and glow, the shadow mask, the curved glass, its bezel and
their lighting, and bloom, drawn over a game as it plays. It is a port of J. Kyle Pittman's
CRTSim, the effect from *Super Win the Game*, and like it is dedicated to the public domain
(CC0 1.0).

## Using it

1. Install [ReShade](https://reshade.me) for the game.
2. Copy the `reshade-shaders` folder into the game's folder, beside the one ReShade made, so
   their `Shaders` and `Textures` folders merge.
3. Copy a look's `.ini` beside the game's executable, open ReShade's overlay (**Home**) and pick
   it as the preset. Or turn on **CRTSim Renderer** in any preset.

The effect draws the whole tube and places the game's picture on it, so it covers the screen.

## Settings

Most settings are in the effect's list in the overlay, with the names and ranges the app gives
them. Three are preprocessor definitions, since the effect's textures depend on them; a look's
preset sets them, and **Edit global preprocessor definitions** changes them, which recompiles the
effect:

- `CRTSIM_SIGNAL_HEIGHT` and `CRTSIM_SIGNAL_WIDTH`: the signal the game's picture is resized to,
  as the app's **Signal** is. 0 rows keeps the picture's own; 0 columns keeps its shape.
- `CRTSIM_LINEAR`: 1 renders in linear light, with float buffers.
- `CRTSIM_TABLE_FILE` and `CRTSIM_TABLE_SIZE`: a look's colour table, a LUT or the NES palette,
  as an image of its slices side by side, and how many there are.

**Pixel aspect** says how wide each of the game's pixels is shown; a game drawn for square
pixels wants 1. The glow fades by real time, so it looks the same at any frame rate.

The effect is made for games on Direct3D 10 and later, OpenGL and Vulkan. Every change to it is
compiled with ReShade's own compiler as each shipped look sets it, and it shares its passes, line
for line, with the RetroArch port, the OBS filter and the Godot effect, which are tested against
the app's renderer. The shipped looks include Super Win the Game's, with and without the game's
NES palette.

## Making your own

Any look made in the CRTSim Renderer app exports as a preset with **Export → ReShade effect…**,
or from the command line:

```sh
crtsim export-reshade --look my-look=my-look.json --output-dir my-reshade
```

A look's LUT or NES palette travels with it as `Textures/crtsim-renderer/<name>-table.png`.
**Screen only** turns the bezel off, as it does in the app.
