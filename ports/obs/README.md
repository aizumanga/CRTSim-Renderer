# CRTSim Renderer for OBS Studio

The CRT of [CRTSim Renderer](https://github.com/aizumanga/CRTSim-Renderer) as an OBS filter: the
composite signal with its artifacts and glow, the shadow mask, the curved glass, its bezel and
their lighting, and bloom, drawn over any source as it streams or records. It is a port of
J. Kyle Pittman's CRTSim, the effect from *Super Win the Game*, and like it is dedicated to the
public domain (CC0 1.0).

Unlike a shader in OBS's own filters, it keeps the signal from one frame to the next, so the
glow of a bright picture fades over the frames after it, as a picture tube's did.

## Installing it

On Linux, a release's `obs-linux-x86_64` package holds the plugin built for OBS 30 and later:
copy its `crtsim-renderer` folder into `~/.config/obs-studio/plugins`.

Elsewhere, or for another version of OBS, build it from `ports/obs` in the repository against
OBS's development files, with the data folder the command line writes:

```sh
crtsim obs-plugin-data --output-dir data
cmake -S ports/obs -B build -DCRTSIM_DATA_DIR=data
cmake --build build && cmake --install build --prefix ~/.config/obs-studio/plugins
```

## Using it

1. Right-click a source, choose **Filters**, and add **CRTSim Renderer** under Effect Filters.
2. Pick a look file under **Look** and press **Apply look**. The plugin's `looks` folder holds
   the shipped ones; its starting look is the app's for ordinary pictures.

Every setting is in the filter's properties, with the names and ranges the app gives them. The
filter draws at the source's own size, and resizes its picture to the signal first: **Signal
rows** 0 keeps the picture's own, and **Signal columns** 0 keeps its shape. The glow fades by
how long each frame lasts, so it looks the same at any frame rate, and moves on once a frame
however many scenes show the source.

## Making your own

Any look made in the CRTSim Renderer app exports for the filter with **Export → OBS filter…**, or
from the command line:

```sh
crtsim export-obs --look my-look=my-look.json --output-dir my-obs
```

A look is a `.json` of the filter's settings, with its LUT or NES palette beside it as
`<name>-table.png`. **Screen only** turns the bezel off, as it does in the app.
