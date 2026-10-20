# CRTSim Renderer for Godot

The CRT of [CRTSim Renderer](https://github.com/aizumanga/CRTSim-Renderer) in Godot 4: the
composite signal with its artifacts and glow, the shadow mask, the curved glass, its bezel and
their lighting, and bloom, drawn over a camera's picture as the game runs. It is a port of
J. Kyle Pittman's CRTSim, the effect from *Super Win the Game*, and like it is dedicated to the
public domain (CC0 1.0).

It needs Godot 4.3 or later with the Forward+ or Mobile renderer. Godot's Compatibility
renderer, which its web exports use, runs no compositor effects.

## Using it

1. Copy the `addons` folder into your project.
2. Give a `WorldEnvironment` (or a `Camera3D`) a **Compositor**, and add a **CRTSimEffect** to
   its effects, or drag in one of the looks from `addons/crtsim_renderer/looks`.
3. In the environment, set **Tonemap → Mode** to **Linear** and turn **Glow** off. The effect
   draws the tube before Godot tonemaps the picture, so any other tonemapper or Godot's own
   glow changes the look. Anti-aliasing and 3D resolution scaling are best off too.

A **2D game** draws through it from a `Camera3D` whose environment sets **Background → Mode**
to **Canvas**: the canvas layers up to **Canvas Max Layer** are then the 3D picture the effect
draws over. Layers above it are drawn over the tube, which suits a menu meant to sit on the
glass rather than behind it.

Every setting is a property of the effect in the inspector, with the names and ranges the app
gives them, and a script can change any of them while the game runs. **Signal rows** sets the
resolution the CRT simulates, which the camera's picture is resized to; 0 keeps its own. The
glow fades by each frame's real length, so it looks the same at any frame rate.

## Making your own

Any look made in the CRTSim Renderer app exports as an effect with **Export → Godot
effect…**, or from the command line:

```sh
crtsim export-godot --look my-look=my-look.json --output-dir my-game-addon
```

Each look is a saved effect, `looks/<name>.tres`, with its LUT or NES palette beside it as
`<name>-table.png`, a strip of the table's slices. **Screen only** turns the bezel off, as it
does in the app.

The shaders are written whole from the passes every port of CRTSim Renderer shares, so they
are best changed there and exported again.
