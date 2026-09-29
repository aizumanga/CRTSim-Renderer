# CRTSim Renderer for RetroArch

The CRT of [CRTSim Renderer](https://github.com/aizumanga/CRTSim-Renderer) as a RetroArch shader
preset: the composite signal with its artifacts and glow, the shadow mask, the curved glass and
its lighting, and bloom, drawn over whatever a core shows. It is a port of J. Kyle Pittman's
CRTSim, the effect from *Super Win the Game*, and like it is dedicated to the public domain
(CC0 1.0).

## Using it

1. Copy this folder into RetroArch's `shaders` folder.
2. In RetroArch, set **Settings → Video → Scaling → Aspect Ratio** to **Full** and turn
   **Integer Scale** off. The preset draws the whole tube and places the picture on it, so it
   needs the whole screen.
3. Load a `.slangp` from **Quick Menu → Shaders → Load**.

Every setting is a shader parameter in **Quick Menu → Shaders → Shader Parameters**, with the
names and ranges the app gives them. A `-linear` preset renders in linear light and keeps float
buffers between passes; switch **Linear light** only in a preset made for it.

The picture's shape comes from the core on RetroArch 1.20 and later, and from **Picture
aspect** on older versions. On 1.20 and later the glow also fades by real time, so it looks the
same at any frame rate; older versions fade it by frames.

## Making your own

Any look made in the CRTSim Renderer app exports as a preset with **Export → RetroArch
shader…**, or from the command line:

```sh
crtsim export-retroarch --look my-look=my-look.json --output-dir my-shaders
```

A look's LUT or NES palette travels with it as `<name>-table.png`, a strip of the table's
slices. The bezel is not ported yet, so every look is drawn screen-only.
