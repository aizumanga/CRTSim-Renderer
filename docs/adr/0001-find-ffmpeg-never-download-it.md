# 1. Find FFmpeg; never download it

Status: accepted. The maintainer ruled out a download button too (see the end).

## Context

Opening videos and exporting any video or animation run FFmpeg and ffprobe as subprocesses.
Before this change, a missing FFmpeg showed only as an error when a video failed to open, and
the fix was to read the README. The README and the release notes promise that the app downloads
nothing at runtime, and the release packages do not bundle FFmpeg.

A missing FFmpeg is often an FFmpeg the app could not see rather than one never installed:

- Apps opened from Finder on macOS do not get Homebrew's folders on their PATH, so
  `brew install ffmpeg` alone is not enough.
- On Windows, a program already running does not see PATH change after `winget install`.
- Portable users unzip FFmpeg next to the app and expect it to be picked up.

## Decision

The app **finds** FFmpeg and **guides** its installation. It does not download it.

- `crtsim_media::Tool::executable` looks where `CRTSIM_FFMPEG` or `CRTSIM_FFPROBE` point; then
  beside the app and in an `ffmpeg` or `ffmpeg/bin` folder next to it; then in the package
  managers' folders (Homebrew and MacPorts; winget, Scoop and Chocolatey); then on PATH.
- The desktop checks the programs in the background at startup. **Export → FFmpeg setup…**
  shows each program's version, which export formats its encoders allow, and this system's
  install command with a Copy button. Opening a video without FFmpeg opens that window instead.
  The export window warns before a format the installed FFmpeg cannot write.

## Consequences

- The promise stays true: nothing reaches the network unless a person runs a command.
- Licensing stays simple: the CC0 app never redistributes FFmpeg, whose usual builds with
  libx264 are GPL.
- A release could still ship FFmpeg in an `ffmpeg` folder beside the app, and it would be found,
  but that bundle would carry GPL source-offer obligations.

## Rejected: a one-click download

A **Download FFmpeg** button, even one that runs only when clicked, was considered and rejected.
It would break the promise of no runtime downloads, and it would need three things: pinned
third-party builds with their checksums, new download and unpacking dependencies, and GPL
notices. Guided setup is the whole answer: find FFmpeg, and tell the person how to install it.
