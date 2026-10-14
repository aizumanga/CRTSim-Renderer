# 4. Check for updates when asked; never download them

Status: accepted. It narrows the promise [ADR 1](0001-find-ffmpeg-never-download-it.md) relies
on, that nothing reaches the network; ADR 1's own decision, never to download FFmpeg, stands.

## Context

Releases come often, and a portable package has no way to say a newer one exists. People
learn of one only by visiting the releases page. The README and the release notes promised
that the app downloads nothing at runtime and that nothing reaches the network unless a person
runs a command.

A self-updating app would download a new build, check it and replace itself. Each system
replaces a running program differently (Windows cannot overwrite its own `.exe`, a Mac app run
from Downloads is translocated to a read-only folder, a Linux tarball may be unpacked where
its user cannot write), and the builds are unsigned, so a file the app wrote itself would skip
the warnings Windows and macOS give for a downloaded program. The app would then be the only
check on what runs. Settings saved by a newer version cannot be opened by an older one, so a
silent update is not easily undone either.

## Decision

The desktop app **tells** a person a newer release exists. It never downloads one.

- **Help → Check for updates…** asks GitHub for the latest published release
  (`api.github.com/repos/aizumanga/CRTSim-Renderer/releases/latest`), compares its tag with
  the app's own version, and shows the result: up to date, or the new version with its notes
  and a link to its release page. Drafts and pre-releases are never offered.
- The request is made only when the person chooses the command, or at start if they turned on
  **Check when the app starts** in that window. It is off until they do. It sends nothing but
  what any HTTPS request carries, with a User-Agent naming the app and its version.
- A failed check says why and changes nothing. No download, unpacking or replacing happens,
  so no new signing keys are needed.
- The web app does not check: the site serves the latest release.
- The release version is the one in `Cargo.toml`; packaging refuses a tag that names another,
  so the comparison holds.

Updating itself is left to tools people already trust to replace programs:

- **Scoop** (Windows) and **Homebrew** (macOS) install from manifests kept in this repository,
  under `bucket/` and `Casks/`, which a workflow updates when a release is published. The
  winget manifests are generated beside them, for submitting to winget's repository.
- **AppImage** carries its update information and a `.zsync` file is published with it, so
  AppImageUpdate, Gear Lever or AppImageLauncher update it in place, downloading only the
  parts that changed.

## Consequences

- The promise becomes: the app downloads nothing to run, and reaches the network only to
  check for a release, when a person asks it to. The README and release notes say so.
- One HTTP client joins the desktop's dependencies (`ureq`, with rustls). Its license texts
  are collected with the others.
- The GitHub API allows 60 unauthenticated requests an hour from one address, far more than
  one per start.

## Rejected: replacing the app in place

Downloading, verifying and swapping the binary was rejected for the reasons in the context:
three ways of replacing a running program, a signing key to keep and build into the app, a
Mac build no one has run on a Mac, and a step that cannot easily be undone.
