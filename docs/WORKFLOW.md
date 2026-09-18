# v0.2 workflow

The toolbar groups commands under **File**, **Presets**, **View** and **Export**, followed by Credits.
File contains media and project commands; Presets contains the gallery and imports; View contains interface themes.
Export contains PNG, video and batch queue commands.

## Edit and compare

**Source & framing** provides independent crop edges, rotation, source zoom and horizontal/vertical pan.
Crop percentages remove pixels before signal sizing. Rotation and pan operate inside the resulting canvas;
rotated corners can be clipped. Nearest filtering retains hard pixels; transformed smooth sampling uses bilinear interpolation.
Neutral framing retains the existing Lanczos path. Output resolution remains independent of the crop.

Choose black, white, a custom color or a checkerboard for transparent pixels and uncovered source areas.
The background becomes part of the rendered image, including exports. This is not alpha-channel export.
**Screen only · no bezel** omits the physical frame mesh while retaining the curved glass, its lighting and output canvas.

**Color LUT** imports a 3D `.cube` table (2–65 samples per axis, up to 16 MB). Trilinear interpolation respects
DOMAIN_MIN/MAX. The LUT runs before the CRT signal simulation and hue/chroma grade. Tables are embedded in
presets/projects, so moving the original LUT file does not break the look. 1D LUTs and ICC transforms are outside this version.

Choose **Compare · drag divider** and drag anywhere across the image to reveal the original on the left and CRT on the right.
Each image keeps its aspect ratio; the curved tube is not geometrically registered with the source.

## Play video

**Play / Pause** and Space control playback. Startup shows **Buffering**, warms a persistent CRT sequence,
and buffers up to three frames before starting the presentation clock. Resume includes up to 200 ms of preceding footage
to rebuild temporal history. Buffer size is bounded independently of clip length and reduced for large sources.
If rendering falls behind, playback shows Buffering again. Lower Preview quality for demanding footage or effects.
Editing, seeking, opening a dialog or starting an export stops playback and cancels its decoder.

Video previews are silent. The earlier separate ffplay audio process could drift and restart on buffer underruns,
so it has been removed. Exported audio is configured in the video export window and still uses FFmpeg's stream-preservation pipeline.

Playback normalizes variable-rate footage to the selected constant rate, as export does. During playback the frame
counter is an estimate from media time; manual frame navigation still selects exact decoded ordinals.
Frame export renders the displayed source as a settled still, independently of playback history.

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
Each job captures the current render settings and video options. Images use PNG; videos use MKV for broader stream preservation.
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

Draft, Balanced, High and Archival select software CRF 26/18/14/10 for H.264 and 32/24/20/16 for VP9.
Archival is a high-quality lossy profile. Hardware H.264 choices are NVIDIA NVENC, Intel Quick Sync, AMD AMF and Apple VideoToolbox.
They use resolution/frame-rate-scaled bitrate targets; the selected encoder is tested before rendering starts.
Availability depends on FFmpeg, driver and hardware. Unsupported selections produce an error; choose Software to retry.
WebM uses software VP9. Actual hardware throughput and visual quality require testing on the target machine.

Preservation defaults on: compatible audio tracks are copied, with AAC/Opus fallback and per-track offset correction.
MKV copies subtitles and attachments; MP4 converts supported text subtitles to mov_text; WebM converts them to WebVTT.
Bitmap subtitles cannot be copied into MP4/WebM, and text conversion can lose styling. Container-supported chapters,
global metadata, audio/subtitle language tags and dispositions are retained where FFmpeg can represent them.
Data tracks and additional video streams are omitted. MP4/WebM attachment and subtitle limitations appear beside export controls.
The renderer preset occupies the output comment; the original comment is retained as `source_comment` where supported.
Mute deliberately drops audio. Turning preservation off retains only the first audio track (unless muted).

Old version-1 presets still load with neutral framing, no LUT and their original look. New keys require v0.2 when sharing presets.
