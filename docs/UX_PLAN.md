# Interface polish plan

The questions reviews of the desktop's everyday use raised, with the answer recommended for
each. Done items are in this branch; the rest are proposals in priority order, the order
[the October review](UX_REVIEW.md) ranked them in by impact and effort. Terms are the ones in
[CONTEXT.md](../CONTEXT.md).

## Done

| Question | Answer |
| --- | --- |
| What does a person without FFmpeg see? | **FFmpeg setup**: versions, writable formats and this system's install command. Opening a video opens it, and the export window warns before a format FFmpeg cannot write. See [ADR 1](adr/0001-find-ffmpeg-never-download-it.md). |
| Why does the app not find an FFmpeg the person installed? | It now also looks beside itself and in the package managers' folders, which Finder-launched Mac apps and already-running Windows apps do not have on PATH. |
| Should a drop-down's closed box and its list name options differ ("Cover" / "Cover (crop)")? | No. One table per drop-down names each option once (`widgets::choice`). |
| Which file commands deserve shortcuts? | Open (`Ctrl+O`), Save project (`Ctrl+S`, asking where the first time) and Export PNG (`Ctrl+E`), shown in the menus; `Command` on macOS. |
| Why do the Pause button and "Export → Video" show boxes? | The bundled proportional fonts lack `Ⅱ`, `→` and `✘`. Hack, egui's monospace font, now backs them, Pause is `⏸` and a cross is `✖`; a test reads the fonts' character maps for every character the interface writes. |
| Where are Open and Export? | Under the source's name: **Open…**, and **Export PNG…** or, for a video, **Export video…**. |
| Which look is this? | **Preset** below them names the preset the settings are, or are closest to, and how many settings differ, listed when pointed at (`PresetGallery::closest`). **Presets…** opens the gallery, and **General image** says what it applies. |
| Where did my export go? | The status bar keeps "Saved *name*" until the next export, where the next preview used to replace it within a moment, with **Show in folder** on the desktop. |
| Can the small print be read? | It is 11 px, not egui's 9 px. |
| What does a first run start on? | The **first look**, right after the welcome: Super Win the Game, General image and Soft television, each a thumbnail of the picture open. Choosing one applies it; closing keeps the look in use. Not offered to someone with a session to recover. |
| Should Super Win the Game recolour every picture through its NES palette? | No. It stays the default with the palette off, since the palette recolours only art drawn in the NES palette. **Super Win the Game NES palette** is the game's look with it on. |
| What did I change? | **Changed only**, beside the preset line, hides every setting that matches the preset named there, and every section left empty; one being dragged stays until the pointer lets go (`widgets::Shown`). |
| Why does the gallery open on a form? | It opens on the presets. Saving, loading a JSON and refreshing sit under *My presets*, and the storage path is in Save current's tooltip. |
| Which notes are always on screen? | Only those that depend on the settings, such as large output or cropping. Standing notes ("SDR output; no ICC color management") sit behind **ℹ Notes**. |
| Why offer Refresh while Live preview is on? | It is offered only while Live preview is off, when edits wait for it. |
| Smaller fixes | The tooltips of Pin look and Copy link to this look lost a run of spaces from a missing line continuation; the playback hint names **Export → Current frame as PNG** as the menu does. |

## Proposed

1. **One place for messages.** `status`, `error` and `preview_error` are three strings that
   overwrite each other, and **Dismiss** clears both errors. A small notices list, where each
   notice has a source, a severity and its own dismiss button, and an FFmpeg error carries a
   **FFmpeg setup…** button, would stop a preview failure from hiding an export failure. The
   save notice would become one of them.
2. **Ask before closing during an export.** Closing mid-export currently cancels it silently.
   The session is recovered, but the export is not.
3. **Adaptive preview quality.** Each preview reports how long it took. Stepping down from
   Balanced to Fast after previews slower than 0.5 s, and back up after fast ones, would keep
   dragging responsive on integrated graphics. An explicit choice would still win.
4. **Dropping several files** onto the window adds them to the **Batch queue**, where today
   only the first file opens.

## Not planned

- Downloading or installing FFmpeg from the app, automatically or on request ([ADR 1](adr/0001-find-ffmpeg-never-download-it.md)).
- Room reflections, VHS noise and sprite flicker (see the plan's *Later work*).
