# Interface polish plan

The questions a review of the desktop's everyday use raised, with the answer recommended for
each. Done items are in this branch; the rest are proposals in priority order. Terms are the
ones in [CONTEXT.md](../CONTEXT.md).

## Done

| Question | Answer |
| --- | --- |
| What does a person without FFmpeg see? | **FFmpeg setup**: versions, writable formats and this system's install command. Opening a video opens it, and the export window warns before a format FFmpeg cannot write. See [ADR 1](adr/0001-find-ffmpeg-never-download-it.md). |
| Why does the app not find an FFmpeg the person installed? | It now also looks beside itself and in the package managers' folders, which Finder-launched Mac apps and already-running Windows apps do not have on PATH. |
| Should a drop-down's closed box and its list name options differ ("Cover" / "Cover (crop)")? | No. One table per drop-down names each option once (`widgets::choice`). |
| Which file commands deserve shortcuts? | Open (`Ctrl+O`), Save project (`Ctrl+S`, asking where the first time) and Export PNG (`Ctrl+E`), shown in the menus; `Command` on macOS. |

## Proposed

1. **Show only what I changed.** The settings panel is long and the answer to "what did I
   change?" already exists: `model::differences` powers the gallery's comparison. A **Changed
   only** toggle at the top of the panel would hide every slider at its default. It would need
   no new state beyond the toggle.
2. **Adaptive preview quality.** Each preview reports how long it took. Stepping down from
   Balanced to Fast after previews slower than 0.5 s, and back up after fast ones, would keep
   dragging responsive on integrated graphics. An explicit choice would still win.
3. **One place for messages.** `status`, `error` and `preview_error` are three strings that
   overwrite each other, and **Dismiss** clears both errors. A small notices list, where each
   notice has a source, a severity and its own dismiss button, and an FFmpeg error carries a
   **FFmpeg setup…** button, would stop a preview failure from hiding an export failure.
4. **Dropping several files** onto the window adds them to the **Batch queue**, where today
   only the first file opens.
5. **A first-run look picker.** After the welcome, offer three large thumbnails of the test
   card (General image, Super Win the Game, Soft television) instead of opening on the
   General preset with nothing chosen. The preset gallery's thumbnails already render these.
6. **Ask before closing during an export.** Closing mid-export currently cancels it silently.
   The session is recovered, but the export is not.

## Not planned

- Automatic FFmpeg downloads, until ADR 1's open question is decided.
- Room reflections, VHS noise and sprite flicker (see the plan's *Later work*).
