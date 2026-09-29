# Glossary

Terms the desktop's code and its reviews use, so a module is named after the concept it holds.

**Preview**: the CRT picture on screen while editing, rendered at the preview quality's size
unless that is Export resolution. It never stands in for an export, which renders again at full
resolution from the settings captured when it was asked for.

**Sequence**: frames rendered one after another that share the CRT's history, so the glow of
one frame persists into the next. A video is a sequence with a timing and a frame rate.

**Still**: a sequence of one frame, rendered after warm-up.

**Warm-up**: the ticks a new sequence runs before the frame it keeps, so a still or a video's
first frame already has the glow of a picture that has been on screen.

**Host sequence**: a sequence paced by its host's own frames rather than a video's rate. It
starts without warm-up, so the glow builds as a set's would, and each frame's glow decays by how
long that frame lasted. A new output size keeps its history; a new signal size or colour mode
starts it again.
_Avoid_: live sequence, since live preview means something else.

**Host**: the program whose picture the CRT is drawn from and into, such as an emulator or a
game, as opposed to the app, which renders files.

**Shader port**: the effect rewritten in a host's own shader language, as passes the host runs
on every frame of its picture, rather than this renderer running inside the host. See
[the decision](docs/adr/0003-games-get-shader-ports-not-the-renderer.md).

**Bezel**: the moulded surround of the picture tube, lit and reflecting the glass.
_Avoid_: frame, which already means a picture in a sequence.

**Preview schedule**: decides whether the preview on screen is out of date and when to render
the next one (`crates/crtsim-app/src/schedule.rs`). One preview renders at a time.

**Revision**: counts the changes to what the preview should show. A preview is of the revision
it was asked for, and one that comes back after a newer change is not shown.

**Settle**: a change settles once it has stayed unchanged for 180 ms with the pointer up. Then
it becomes an undo step and, with live preview on, is previewed.

**Live preview**: preview every edit once it settles. With it off, edits wait for Refresh.

**Audition**: previewing a preset or LUT by pointing at it in a gallery, without applying it.
An audition is previewed whether or not live preview is on, and never reaches the settings,
their undo history or an export.

**Timeline**: where the editor is in an open video: the frame on screen, the frame the
controls have picked to go to, and the time playing starts from
(`crates/crtsim-app/src/timeline.rs`). There is one only while a video or animation is open.

**Playback**: playing a video in the preview (`crates/crtsim-app/src/playback.rs`). The
worker renders frames ahead into a small buffer, and each is shown when its time comes. Frames
already overdue are skipped rather than shown late.

**Preroll**: the frames playback waits for before its clock starts: three, or all the buffer
holds when large frames make it smaller. When the renderer falls behind, the clock stops and
playback prerolls again.

**Colour table**: the LUT or the NES palette a look maps colours through before the CRT. A look
has one at most; choosing either replaces the other (`Config::set_lut`, `Config::set_palette`).

**Preset gallery** and **LUT gallery**: the windows offering looks to audition and apply
(`gallery.rs` and `lut_gallery.rs`). The preset gallery also saves the settings in use as a
personal preset; the LUT gallery decodes each included LUT once and shares it.

**Batch queue**: exports that each render one file with the settings in use when they were
added, one at a time and in order (`crates/crtsim-app/src/batch.rs`). Each output is named
after its source and never replaces a file; cancelling a job pauses the queue.

**Project**: a source, its settings and the batch queue, saved as a `.crtsim` file to pick up
later (`crates/crtsim-app/src/project.rs`).

**Session**: the project the app saves to its app data every two seconds, and offers to
recover on the next start (`crates/crtsim-app/src/session.rs`, which also keeps the project
file open and the recent ones). It is never saved over a session still on offer, nor while a
load or a project being restored leaves the settings half applied.

**App data**: the folder holding everything the app remembers between runs: the welcome
acknowledged, the theme, tool window placement, personal presets, recent projects and the
session (`crates/crtsim-app/src/app_data.rs`). Only that module knows the files in it.

**FFmpeg setup**: whether the FFmpeg programs video work needs start, which export formats
they can write, and how to install them (`crates/crtsim-app/src/ffmpeg_setup.rs`). The
programs are found, never downloaded (`crates/crtsim-media/src/tools.rs`); see
[the decision](docs/adr/0001-find-ffmpeg-never-download-it.md).

**Web app**: the whole app, with editing, the galleries, playback and exports, running in a
browser from the author's Neocities site. It is not a cut-down demo: what the desktop app does
with a look, the web app does too, within what a browser allows. It needs WebGPU and never
runs FFmpeg; see [the decision](docs/adr/0002-the-web-app-is-the-whole-app-on-webgpu.md).
