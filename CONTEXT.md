# Glossary

Terms the desktop's code and its reviews use, so a module is named after the concept it holds.

**Preview**: the CRT picture on screen while editing, rendered at the preview quality's size
unless that is Export resolution. It never stands in for an export, which renders again at full
resolution from the settings captured when it was asked for.

**Preview schedule**: decides whether the preview on screen is out of date and when to render
the next one (`crates/crtsim-desktop/src/schedule.rs`). One preview renders at a time.

**Revision**: counts the changes to what the preview should show. A preview is of the revision
it was asked for, and one that comes back after a newer change is not shown.

**Settle**: a change settles once it has stayed unchanged for 180 ms with the pointer up. Then
it becomes an undo step and, with live preview on, is previewed.

**Live preview**: preview every edit once it settles. With it off, edits wait for Refresh.

**Audition**: previewing a preset or LUT by pointing at it in a gallery, without applying it.
An audition is previewed whether or not live preview is on, and never reaches the settings,
their undo history or an export.

**Playback**: playing a video in the preview (`crates/crtsim-desktop/src/playback.rs`). The
worker renders frames ahead into a small buffer, and each is shown when its time comes. Frames
already overdue are skipped rather than shown late.

**Preroll**: the frames playback waits for before its clock starts: three, or all the buffer
holds when large frames make it smaller. When the renderer falls behind, the clock stops and
playback prerolls again.

**Batch queue**: exports that each render one file with the settings in use when they were
added, one at a time and in order (`crates/crtsim-desktop/src/batch.rs`). Each output is named
after its source and never replaces a file; cancelling a job pauses the queue.
