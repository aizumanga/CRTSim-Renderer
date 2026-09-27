# Web app plan

How the **Web app** ([CONTEXT.md](../CONTEXT.md)) gets built and reaches the site. The decisions
behind it are in [ADR 2](adr/0002-the-web-app-is-the-whole-app-on-webgpu.md). Steps are in
order; the polish track can run alongside.

## Steps

| Step | Work | Proven by |
| --- | --- | --- |
| 1 | `App::new` takes its app-data store. `main` passes the user's folder; tests pass a temporary one. | No test can write to the real app data. |
| 2 | Upgrade wgpu and eframe to current releases. | The golden images are unchanged. |
| 3 | One frame interface for the renderer: callers drive a sequence, which knows its timing; a still is a sequence of one frame; nothing blocks inside. | The golden images, reached through the new interface. |
| 4 | `crtsim-desktop` becomes `crtsim-app`, a library, with a small native `main`. | The desktop builds and behaves as before. |
| 5 | The render worker speaks renders and typed failures on one stream, with two adapters: a thread and an async task. | The worker's tests run against both adapters. |
| 6 | The web entry point: WebGPU check, files by picker and drop, downloads, app data in IndexedDB. | Opens, previews and saves a PNG in Chrome and Firefox. |
| 7 | Web encoders: WebCodecs for MP4 and WebM with a muxer, and Rust encoders for GIF and animated WebP. | Each format opens in the browser that wrote it. |
| 8 | Release and site workflows (below). | A published release reaches the site through a pull request. |

## Polish track

These help both hosts, so they are worth doing before or during steps 4 to 6:

- **Notices**: one module for what the app tells the person, one notice per source, each with
  its own dismiss and an optional action (proposal 3 in the [UX plan](UX_PLAN.md)).
- **Source**: one module for the open image or video, which also gives the preview's config.
  This fixes playback ignoring an audition.
- **Settings drawn from core's description**, so a setting is added in one place.

## From a release to the site

1. Tagging `vX.Y.Z` runs `releases.yml`. Besides the desktop packages, a web job builds
   `CRTSim-Renderer-vX.Y.Z-web.zip` and `SHA256SUMS-web.txt` into the draft release.
2. The maintainer publishes the draft.
3. In `aizumanga/aizumanga-neocities`, an **Update CRTSim** workflow runs daily and on demand.
   When the latest published release is newer than the one on the site, it downloads the web
   zip and checks its checksum. It then replaces the web app's folder, sets `VERSION` in
   `scripts/crtsim-downloads.js` to match, and opens a pull request.
4. Merging the pull request runs the site's existing **Deploy to Neocities**, which uploads the
   changed files. The downloads dialog and the web app then name the same release.
