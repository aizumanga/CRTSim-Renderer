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
| Smaller fixes | The tooltips of Pin look and Copy link to this look lost a run of spaces from a missing line continuation; the playback hint names **Export → Current frame as PNG** as the menu does. |

## Proposed

1. **Show only what I changed.** The panel is long, and the preset line already counts the
   changes. A **Changed only** toggle at the top of the panel would hide every setting that
   matches the preset named there. It would need no new state beyond the toggle.
2. **Presets first in the gallery.** Move the save form under *My presets* and the storage
   path into a tooltip, so the gallery opens on looks rather than on five rows of chrome.
3. **Fine print into tooltips.** Keep the warnings that depend on the settings, such as large
   output or cropping, and move the standing notes ("SDR output; no ICC color management")
   behind a hover.
4. **Refresh only while Live preview is off**, when it has something to catch up on.
5. **A first-run look picker.** After the welcome, offer three large thumbnails of the test
   card (General image, Super Win the Game, Soft television) instead of opening on Super Win
   the Game with nothing chosen. The preset gallery's thumbnails already render these, and the
   preset line then names the choice.
6. **One place for messages.** `status`, `error` and `preview_error` are three strings that
   overwrite each other, and **Dismiss** clears both errors. A small notices list, where each
   notice has a source, a severity and its own dismiss button, and an FFmpeg error carries a
   **FFmpeg setup…** button, would stop a preview failure from hiding an export failure. The
   save notice would become one of them.
7. **Ask before closing during an export.** Closing mid-export currently cancels it silently.
   The session is recovered, but the export is not.
8. **Adaptive preview quality.** Each preview reports how long it took. Stepping down from
   Balanced to Fast after previews slower than 0.5 s, and back up after fast ones, would keep
   dragging responsive on integrated graphics. An explicit choice would still win.
9. **Dropping several files** onto the window adds them to the **Batch queue**, where today
   only the first file opens.

## Not planned

- Downloading or installing FFmpeg from the app, automatically or on request ([ADR 1](adr/0001-find-ffmpeg-never-download-it.md)).
- Room reflections, VHS noise and sprite flicker (see the plan's *Later work*).
