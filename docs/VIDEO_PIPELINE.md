# Video pipeline

The `crtsim-media` crate invokes local FFmpeg/ffprobe executables without a shell. The desktop worker owns each job and captures its settings.
Environment overrides `CRTSIM_FFMPEG` and `CRTSIM_FFPROBE` support executable paths containing spaces.

## Processing

1. Probe the first non-cover-art video stream and collect audio/subtitle/attachment tracks and global metadata. Validate dimensions, duration, frame rate, pixel aspect and right-angle rotation.
2. Normalize decoded video timestamps to zero, sample them at the selected constant frame rate, apply HDR-to-SDR tone mapping when required, and normalize rotation/pixel aspect.
3. Read one RGBA frame at a time. A `Sequence` owns two feedback textures and a simulation tick counter on one renderer. Warm-up occurs only on the first frame; later frames retain history.
4. Stream rendered RGBA to the encoder at the same rational frame rate. Balanced defaults to H.264 CRF 18/medium for MP4/MKV or VP9 CRF 24 for WebM. Other profiles and optional hardware H.264 encoders are described in [WORKFLOW.md](WORKFLOW.md). Full-range RGB converts explicitly to limited-range BT.709, tagged as BT.709, and stored as 8-bit yuv420p. Output dimensions must be even.
5. Remux source audio tracks, supported subtitles, chapters and metadata. Auto mode tries audio stream copy without material offsets, then falls back to AAC/Opus if needed. Offset audio is trimmed/padded per track. MKV also copies attachments. Unsupported bitmap subtitles in MP4/WebM and data streams are omitted with UI guidance. Text subtitle conversions may lose styling.
6. Publish a complete temporary output using atomic replacement. Any error or cancellation leaves an existing destination untouched.

The output duration follows the sampled video frames, with up to one frame of CFR rounding. Audio is limited to that duration.
No frame interpolation is performed: conversion to 60 Hz holds/drops source frames using their timestamps.
Stable mode uses each channel's original persistence weight raised to `60 / output_fps`; spatial persistence still makes this an approximation.
Each export starts a fresh sequence, independent of all previews and prior jobs.

## Resource ownership and cancellation

Each FFmpeg process has a bounded stderr collector and a monitor that can kill the process while frame pipe I/O is blocked.
Dropping a process kills and reaps it; closing the desktop sets cancellation and joins the worker.
GPU work already submitted must finish before the worker can return. PNG cancellation is checked between bounded warm-up batches and before saving. Frames are never queued without bounds.
Only a silent encoded clip and the final encoded container are stored temporarily, on the destination filesystem.
Signal, surface, bloom, depth, readback and temporal targets persist for the duration of a video sequence. Performance is not guaranteed to be real time.

## Scope and limits

- Exact frame seeking produces a settled still. Continuous playback retains history, has up to 200 ms of preroll on resume, and uses bounded buffering. See [WORKFLOW.md](WORKFLOW.md) for audio-preview and CFR navigation limits.
- Only local files with a finite probed duration (up to seven days) and a usable 1–240 FPS rate are accepted. Existing core dimension/memory limits apply.
- Sources are normalized to square pixels before the CRT preset's own pixel-aspect setting. Rotation is limited to multiples of 90 degrees.
- HDR transfer flags trigger `zscale`/`tonemap`; the installed FFmpeg must provide those filters. ICC management, Dolby Vision reconstruction and HDR output are outside this version.
- Preservation is enabled by default, subject to container support. Renderer presets are embedded in PNG and exported video; original comments use `source_comment` where supported.
- Optional hardware H.264 encoders are probed before rendering. Video export is exposed through the desktop and media library; the image CLI also supports source edits/LUTs via JSON.
- The original video cannot be its own export destination. Native save dialogs confirm other replacements.

## Verification

Normal workspace tests validate rate parsing, rotation/pixel aspect, settings, shaders and metadata safety.
Run the FFmpeg integration suite explicitly with the codecs listed in the README:

```sh
cargo test --locked -p crtsim-media --test video ffmpeg_streaming -- --ignored --nocapture
cargo test --locked -p crtsim-media --test video frame_rates -- --ignored --nocapture
```

It checks MP4/MKV/WebM, frame counts, stream copy and re-encoding, duration, fixed rates, variable-rate timestamp normalization, audio delay, mute, atomic replacement, cancellation and temporary-file cleanup.
With a Vulkan adapter, also run:

```sh
cargo test --locked -p crtsim-core video_history -- --ignored --nocapture
cargo test --locked -p crtsim-media --test video video_gpu -- --ignored --nocapture
```

CI runs both suites with software Vulkan, exports a short audio/video sample, and captures the desktop with video controls visible.
Native Windows/macOS dialogs, physical-GPU throughput and long-form A/V synchronization still require manual testing.

## Frame navigation and embedded presets

Desktop frame controls operate on decoded frame ordinals, starting at 1 in the UI.
The container's frame count is reused when present and valid. Otherwise FFprobe counts decoded frames once on opening; the worker reuses that count while navigating.
FFmpeg selects the requested decoded frame from the start of the stream, preserving exact ordering for CFR and VFR.
This favors accuracy over seek speed. Long clips may take time; loading/seeking can be cancelled.
Frame PNG export renders the currently loaded frame as a settled still, without reconstructing motion history.

MP4/MKV/WebM exports write `CRTSim-Renderer-Preset:` plus versioned JSON into the container comment.
The JSON contains the original Config and video Options (timing/audio), not the adjusted persistence coefficients.
The importer validates the schema, version and dimensions, runs ffprobe on a background worker and limits the response to 32 MB for embedded LUTs. Export metadata goes through an FFmetadata file to avoid command-line length limits.
Container tags are matched case-insensitively because Matroska/WebM uppercases comment names.
The renderer preset contains no source filename/path. Preserved source metadata may contain information from the source container. Audio fallback and mute retain the renderer preset; external services/editors may strip it.
