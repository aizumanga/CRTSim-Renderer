# 1. Find FFmpeg; never download it

Status: accepted for v0.7. The open question below needs the maintainer's decision.

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

## Open question: a one-click download

A **Download FFmpeg** button in the setup window, run only when clicked, would remove the last
manual step. Before building it, decide:

1. **Whether to change the promise.** A download the person asks for is not automatic, but the
   README currently says there are *no* runtime downloads.
2. **Where from.** Windows and Linux builds come from gyan.dev or BtbN's GitHub releases, and
   macOS builds from evermeet.cx or osxexperts. Each release must be pinned by URL and SHA-256
   in the source, and updated deliberately.
3. **What it adds.** An HTTPS client, and ZIP and `.tar.xz` extraction, would all be new
   dependencies, as would a folder for the download in app data, which the lookup above would
   also search.
4. **Licence notices.** They would be shown next to the button, because the download is GPL
   software.

Recommendation: keep the guided setup as the default. Add the button only if the maintainer
accepts points 1 and 4, pinning one build per platform and verifying its checksum before
extracting it.
