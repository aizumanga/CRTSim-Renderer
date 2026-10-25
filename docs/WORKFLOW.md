# v0.2 workflow

The toolbar groups commands under **File**, **Presets**, **View**, **Export** and **Help**, which holds the credits and, on the desktop, the update check.
File contains media and project commands; Presets contains the gallery and imports; View contains interface themes and sounds.
Export contains PNG, video and batch queue commands.

The workbench uses beveled control groups, a shaded menu bar, a native monitor icon and a colored status lamp.
The preview has a dark surround and centers the image; less-used controls collapse to leave more space for editing.
Sky Diary is the default for new installs and matches the approved blue-gray/pink concept. Existing saved theme choices
are respected: choose **View → Sky Diary** for that palette. All five themes use the new layout and native controls.

## Interface effects and sounds

Sliders are meters: a well of cells that light up to the value, in the theme's own style, with a small
mark at the default. Dragging, the arrow keys, Delete and typed numbers work as before. Settings
switch with sliding switches whose lamp lights while on, and a section's lamp lights while any setting
in it differs from the preset. **Original / CRT / Compare** is a channel selector.

**Effects**, first in the settings panel, chooses how much the interface moves. **Full**, the
default, eases the meters and switches, and the preview's screen switches on like a set's when new
media first shows and flickers with static when the channel or the preset changes; **Reduced** keeps
the meters and switches moving but the screen still; **Off** moves nothing that need not. The screen's
effects are drawn over the preview only, never into exports.

The interface makes short sounds of its own: no audio files are shipped. They are on until you turn
them off with **Sounds**, beside Effects. **View → Interface sounds** chooses the volume and which
groups you hear: meters and switches (a tick for each cell, a click at the default, a thud at the ends,
a sweep on reset), buttons, channels and sections, and power-on, presets and finished exports. Pointing
at a button ticks very quietly, with Full effects only. The audio output opens the first time a sound
plays; if there is none, View → Interface sounds says why and the app stays silent until sounds are
turned off and on again. On Linux, sounds go to PulseAudio's server, which PipeWire serves too, or else
to ALSA. Building from source on Linux needs ALSA's development package (Debian/Ubuntu:
`libasound2-dev`).

## Edit and compare

**Source & framing** provides independent crop edges, rotation, source zoom and horizontal/vertical pan.
Crop percentages remove pixels before signal sizing. Rotation and pan operate inside the resulting canvas;
rotated corners can be clipped. Nearest filtering retains hard pixels; transformed smooth sampling uses bilinear interpolation.
Neutral framing retains the existing Lanczos path. Output resolution remains independent of the crop.

Choose black, white, a custom color or a checkerboard for transparent pixels and uncovered source areas.
The background becomes part of the rendered image, including exports. This is not alpha-channel export.
**Screen only · no bezel** omits the physical frame mesh while retaining the curved glass, its lighting and output canvas.

**Color & LUT → LUT gallery…** opens 38 included NES LUTs with a searchable list. Each
shows its color mapping on a thumbnail of the current image. Point at one to preview it on
the full image without applying it; move away to return to your settings.
Click a name to apply it while keeping your CRT and source framing settings. The gallery
stays open for comparisons; **Remove LUT** restores the ungraded input. Changes support
Undo/Redo and use Live preview when enabled. The collection is embedded and works offline.
These LUTs were designed to remap MAME's mathematically derived NES palette; arbitrary
images can use them as creative grades, but will not necessarily reproduce the named palette.
Credits and source/license links are available in the gallery and Credits & support.

**Color LUT** imports a 3D `.cube` table (2–65 samples per axis, up to 16 MB). Trilinear interpolation respects
DOMAIN_MIN/MAX. The LUT runs before the CRT signal simulation and hue/chroma grade. Tables are embedded in
presets/projects, so moving the original LUT file does not break the look. 1D LUTs are outside this version.

Choose **Compare · drag divider** and drag anywhere across the image to reveal the original on the left and CRT on the right.
Each image keeps its aspect ratio; the curved tube is not geometrically registered with the source.

## Play video

**Play / Pause** and Space control playback. Startup shows **Buffering**, warms a persistent CRT sequence,
and buffers up to three frames before starting the presentation clock. Resume includes up to 200 ms of preceding footage
to rebuild temporal history. Buffer size is bounded independently of clip length and reduced for large sources.
If rendering falls behind, playback shows Buffering again. Lower Preview quality for demanding footage or effects.
Editing, seeking, opening a dialog or starting an export stops playback and cancels its decoder.

**Loop** plays the video again from its start when it ends, for the video playing now and the next. The next lap is read before this
one ends and the CRT keeps its history, so a short clip or GIF goes round without a pause, its last frame's glow fading into its first.
Turning Loop off lets the lap in progress play out. A video that stops on an error does not start again by itself.

Video previews are silent. The earlier separate ffplay audio process could drift and restart on buffer underruns,
so it has been removed. Exported audio is configured in the video export window and still uses FFmpeg's stream-preservation pipeline.

Playback normalizes variable-rate footage to the selected constant rate, as export does. During playback the frame
counter is an estimate from media time; manual frame navigation still selects exact decoded ordinals.
Frame export renders the displayed source as a settled still, independently of playback history.

## Subtitles

A video with subtitle tracks shows a **Subtitles** drop-down under the preview controls. Choosing a track draws it into every
frame before the CRT sees it, so scanlines, mask, glow and curvature affect it as they do the picture; **Off** returns the plain
frame. The frame on screen loads again, or playback goes on from where it is, with the subtitle. Text subtitles (SubRip, ASS/SSA,
WebVTT, MP4 text) are typeset by FFmpeg's libass at the times the file gives, styling and embedded fonts included. Picture subtitles
(Blu-ray PGS, DVD VobSub, DVB) are laid over the frames; to show one that began before a frame being loaded, FFmpeg reads up to ten
seconds of footage before it. Reading a text subtitle takes FFmpeg through the whole file each time a frame is loaded or playback
starts, which takes a moment on large files. Closed captions inside the video stream cannot be drawn. A file a browser opens in the web
app has no subtitle tracks to choose from.

## Projects and recovery

**Project → Save project as…** stores a `.crtsim` file containing source location, selected frame, rendering settings,
embedded LUT, video export options and the queue. Media is referenced, not copied. Opening projects uses absolute source
paths or resolves relative paths beside the project. Keep the source available; if it is missing, recovered settings remain
available and **Open File** relinks a replacement. The last ten project paths appear in the Project menu.

The app saves a separate recovery snapshot every two seconds and on exit. Next launch offers Restore session or Start fresh.
An explicit startup media path bypasses that offer. Named project files and gallery presets change only when explicitly saved.
Running queue jobs recover as pending, and the queue always restores paused. Project files are bounded to 64 MB / 256 jobs;
large embedded LUTs repeated across many jobs can reach this limit. Storage errors are shown in the status area.

## Batch exports

Open **Batch queue → Add files…**, select images/videos, then choose a destination folder.
Or drop several images or videos on the window: the desktop asks for the destination folder, starting
where the first file is, and adds them with the settings on screen. One dropped file opens, as before;
the web app opens the first of several, since it has no batch export yet.
Each job captures the current render settings and video options. Images use PNG; videos and animated GIFs use MKV for broader
stream preservation. Animated WebPs still become a PNG of their first frame, as they did before animations could be opened.
Generated names include `-crt` and a numeric suffix when needed. Existing destinations are refused when a job starts.
Use Start / resume, Pause after current, Cancel current, Retry, Move up or Remove. Failed jobs retain their errors while subsequent
jobs can continue. No temporary PNG frame sequences are written. Cancelling or closing the app reaps FFmpeg subprocesses.

## Video quality and preservation

**Export → Video…** opens a dedicated settings window before the destination chooser. MP4/H.264 is the recommended default;
MKV/H.264 favors stream preservation, and WebM/VP9 targets web playback. Each format and encoding method has a short explanation.
**Advanced** exposes optional software CRF (0–51), compression speed, or hardware target bitrate (1–200 Mbps).
Leave overrides off to use the quality profile. Selecting another profile resets custom CRF/bitrate. Format switches reset CRF
because H.264 and VP9 use different quality scales. WebM always uses software VP9.
The batch queue has its own **Video settings…** window for new jobs; existing jobs keep their settings.

## GIF, animated WebP and animated PNG

The same window offers **GIF**, **Animated WebP** and **Animated PNG**, none with sound. All grow quickly with size, rate and length,
so their defaults are small: the CRT is rendered with its longest side at 640 pixels, at 24 frames per second, for the
first 10 seconds. The CRT is rendered at that size rather than scaled down afterwards, which would turn the mask and
scanlines into moiré. **Start at the frame shown** begins at the frame on screen; **Only the first … s** limits the length.
Timing is Stable or Persistence off: NTSC timing needs 60 frames per second.

GIF allows up to 30 frames per second. It stores each frame's time in hundredths of a second, so at 24 per second
frames alternate between 4 and 5 hundredths, averaging exactly 24. One palette of 256 colors is chosen from the whole
animation, so the frames are first encoded losslessly to a temporary file next to the destination; the window shows
how much space that needs. Dithering can be a pattern (smallest), diffusion (smoother) or none (banding).
Animated WebP allows up to 60 frames per second and is usually several times smaller, with full color. Lossy WebP
stores color at half resolution, softening the mask's colored stripes; **Lossless** keeps them at a larger size.
Animated PNG allows up to 60 frames per second and keeps every pixel exactly as rendered, in a `.png` that browsers
play and other viewers show as its first frame: the largest of the three, for editing or archiving a clip. FFmpeg's
APNG encoder is built in, so any FFmpeg writes it, and the web app writes it without FFmpeg. A PNG still opens as a
still image, not as an animation.

Below the settings the window shows the rendered size, the frame count and a likely size range, from still scenes
(low) to busy footage with room to spare (high). Above 25 MB the estimate turns amber and names what shrinks the file
most, and choosing a destination asks once more. GIF, WebP and animated PNG files do not store the renderer preset.

## Keyboard

Click or drag any slider to give it the keyboard, marked by its highlighted handle. **←/→** step it by 1% of its range,
**Shift+←/→** by 10% and **Alt+←/→** by 0.1%; logarithmic sliders step by the same proportion at any value, and
whole-number sliders by at least one. **Delete** or **Backspace** returns it to its default, as its ↺ button does.
**Esc** or a click elsewhere lets go; until then the arrows no longer step video frames. Holding a key is one undo step.

Draft, Balanced, High and Archival select software CRF 26/18/14/10 for H.264 and 32/24/20/16 for VP9.
Archival is a high-quality lossy profile. **Lossless**, offered for MKV on the desktop, writes the CRT's RGB exactly in
FFV1 (version 3, every frame a keyframe, checksummed slices), without the conversion to limited-range YUV 4:2:0 the
other profiles make: for editing and archiving, at many times Archival's size, and not every player plays FFV1.
Hardware H.264 choices are NVIDIA NVENC, Intel Quick Sync, AMD AMF and Apple VideoToolbox.
They use resolution/frame-rate-scaled bitrate targets; the selected encoder is tested before rendering starts.
Availability depends on FFmpeg, driver and hardware. Unsupported selections produce an error; choose Software to retry.
WebM uses software VP9. Actual hardware throughput and visual quality require testing on the target machine.

Preservation defaults on: compatible audio tracks are copied, with AAC/Opus fallback and per-track offset correction.
MKV copies subtitles and attachments; MP4 converts supported text subtitles to mov_text; WebM converts them to WebVTT.
Bitmap subtitles cannot be copied into MP4/WebM, and text conversion can lose styling. Container-supported chapters,
global metadata, audio/subtitle language tags and dispositions are retained where FFmpeg can represent them.
Data tracks and additional video streams are omitted. MP4/WebM attachment and subtitle limitations appear beside export controls.
When the video has subtitle tracks, the export window lists them: **Keep** copies a track into the file (while the additional tracks above
are kept), and **Draw in picture** puts it through the CRT with the frames instead, for MP4, MKV, WebM, GIF or animated WebP. A track
drawn in is unticked for keeping, since it would show twice, but can be ticked again. The window starts with the subtitle on screen in the
preview drawn in and every track kept. These choices are for the export in hand; batch jobs open files not yet known, so they keep every
subtitle track and draw none.
The renderer preset occupies the output comment; the original comment is retained as `source_comment` where supported.
Mute deliberately drops audio. Turning preservation off retains only the first audio track (unless muted).

Old version-1 presets still load with neutral framing, no LUT and their original look. New keys require v0.2 when sharing presets.
