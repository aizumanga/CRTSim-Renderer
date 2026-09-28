# Web app plan

How the **Web app** ([CONTEXT.md](../CONTEXT.md)) gets built and reaches the site. The decisions
behind it are in [ADR 2](adr/0002-the-web-app-is-the-whole-app-on-webgpu.md). Steps are in
order; the polish track can run alongside.

## Steps

| Step | Work | Proven by |
| --- | --- | --- |
| 1 ✓ | `App::new` takes its app-data store. `main` passes the user's folder; tests pass a temporary one. | No test can write to the real app data. |
| 2 ✓ | Upgrade wgpu and eframe to current releases (wgpu 30, eframe 0.36). | The golden images are unchanged. |
| 3 ✓ | One frame interface for the renderer: callers drive a sequence, which knows its timing; a still is a sequence of one frame; nothing blocks inside. | The golden images, reached through the new interface. |
| 4 ✓ | `crtsim-desktop` becomes `crtsim-app`, a library, with a small native `main` (the executable keeps its name). | The desktop builds and behaves as before. |
| 5 ✓ | The render worker speaks renders and typed failures on one stream, with two adapters: a thread and an async task. | The worker's tests run against both adapters. |
| 6 ✓ | The web entry point: WebGPU check, files by picker and drop, downloads, app data in IndexedDB. | Opens, previews and saves a PNG in Chrome and Firefox. (Chromium checked; Firefox by hand.) |
| 7 ✓ | Web encoders: WebCodecs for MP4 and WebM with a muxer, and Rust encoders for GIF and animated WebP. | Each format opens in the browser that wrote it. (Chromium checked for GIF, WebP and WebM; MP4 needs a browser with H.264, by hand.) |
| 8 ✓ | Release and site workflows (below). | A published release reaches the site through a pull request. |

## The frame interface (step 3, done)

- **A sequence owns its timing.** It is made as a still, or as a video with a timing (stable,
  NTSC 60 or no persistence) and a frame rate. `render_config` moves from crtsim-media into core
  and stops being something every caller must remember to apply.
- **A still is a sequence of one frame**, after warm-up. It replaces `render`, `render_frame`
  on a fresh sequence, and `render_preview`.
- **The renderer keeps the output.** A frame can be shown where it is, on a shared device, or read
  back asynchronously. Drawing into a caller's own target waits until a game or OBS host needs it.
- **One uniform buffer per sequence**, with a slot per tick in a batch reached by dynamic offset,
  and bind groups cached with the workspace. A single slot would not do: `write_buffer` lands at
  submit, so every tick in a batch would see the last tick's settings.
- **Frames are `async`.** Between batches of 8 ticks a frame awaits the submitted work, reports
  progress and checks for cancelling. The desktop's worker thread blocks on it; the browser
  awaits it. Read back is `async` too.
- **Core reports typed progress stages**, and the app words them.
- **The CLI asks for the signal and clean images only with `--debug-dir`.**
- **The web app declines an image larger than the GPU allows**, with a notice naming the limit.
  The desktop keeps preparing such images on the CPU, which one browser thread cannot afford.
  (Done with the web entry point, step 6.)

## The worker's two runtimes (step 5, done)

- **The lanes are async.** The preview lane (previews before thumbnails, stale thumbnails
  dropped) and the work lane (loading, exports, playback) are each one `async fn`. On the
  desktop, `Runtime::Threads` runs each on a thread of its own, as before. In the browser,
  `Runtime::Tasks` hands each to the page as a task on its one thread.
- **Jobs ring a bell.** Job channels stay `std::mpsc`; sending one wakes the lane waiting for
  it, whether that lane sleeps on a thread or is a task that gave its thread back.
- **A render yields between batches**, so a long export on one lane lets the other lane's
  preview through when both share a thread. The worker's test proves it on both runtimes, and
  fails without the yield.
- **Failures are typed.** Events carry the error itself, not text made from it, and the app
  words it. Playback's frames keep their own bounded channel: its back-pressure paces playback.
- **No lock around building a renderer.** wgpu 30 keeps error scopes per thread, and a renderer
  opens and closes its scope with no await between. A blocking lock held across awaits could
  have deadlocked the browser's one thread.

## The web entry point (step 6, done)

- **One app, two hosts.** `crtsim_app::web::start` runs the same `App` in a page with eframe's
  web runner, on WebGPU only, with the worker's lanes as tasks. Clocks are `web_time`'s, which
  are std's on the desktop.
- **Files in as bytes, out as downloads.** The browser's own picker, opened straight from the
  click or key press that asked for it, and drops give a name and bytes: images open
  (`Job::LoadBytes`), presets and LUTs load, and a PNG's embedded preset imports. Exported PNGs
  and saved presets download under the name the save would have used. rfd stays the desktop's:
  on the web it shows its own overlay with a second button to press.
- **App data in IndexedDB.** The store has two places: the desktop's folder, and the browser's
  storage for the site, read whole when the page starts and written through on each change.
  A browser session keeps the settings; the picture is picked again.
- **Not in the browser yet**, and saying so: video (step 7), projects and batch export. Nothing
  is downloaded but the page itself.
- **The page** checks for WebGPU and for a graphics adapter first, and explains what to do for
  each, or to get the desktop app. If the browser takes the device away later, the app tells
  the page, which says so and offers a reload. `tools/build_web.py` builds it into `web/dist`;
  CI checks the browser build with clippy.
- **Checked in headless Chromium** on its software WebGPU (with `--use-angle=swiftshader`;
  without it, this container's Chromium loses any device that draws to a canvas, a plain
  WebGPU page included): the test card previews and exports a PNG within a few levels of the
  desktop's render of the same preset (PSNR 55 dB; the rim's edge pixels differ between the two
  software rasterisers); an image opens through the picker and another by dropping it, each
  exporting; a cancelled picker leaves the app usable; the app data is found again after a
  reload; the notices without WebGPU and after a lost device show. Firefox is not available in
  the build container, so it still needs a check by hand, with `dom.webgpu.enabled` on Linux.
- **Images larger than the browser's GPU allows are declined**, naming the limit and pointing to
  the desktop app, instead of being prepared on the page's only thread.
- **Still to do for the web:** the module is 20 MB, 7 MB of it the embedded NES LUTs, which
  could be fetched when first used.

## Video in the browser (step 7, done)

Every video path on the desktop runs through FFmpeg processes, so the browser needs its own
way in as well as out: exporting needs a video to export. Step 7 is three parts, each shipped
on its own.

1. **Animations out (done).** GIF and animated WebP are written in Rust (`crtsim-media`'s
   `gif_writer` and `webp_writer`), from sources the browser can already open: animated GIF
   and WebP are decoded in Rust, from the bytes the page was handed (`Video::contents`).
   - `crtsim_media::page` runs a video the way a page must: one frame at a time, each step
     awaited, the file made in memory and downloaded. Its frames come through `FrameSource`,
     which animations fill now and WebCodecs will fill next. Playback and the timeline's
     frames use it too. The worker takes this path for any video that came as bytes, so the
     native tests exercise it; the desktop's FFmpeg paths are unchanged.
   - A GIF gets 255 colors for the whole animation, by median cut over every frame's
     colors, and the desktop's three dithers (FFmpeg's Bayer at scale 3, Sierra-2-4A
     diffusion, none). Choosing colors first means its frames are rendered twice. Each frame
     stores only the rectangle that changed, unchanged pixels in it transparent, and a frame
     that changes nothing lengthens the one before.
   - Animated WebP frames are lossless from `image-webp`, or lossy from the browser's own
     still-WebP encoder (`OffscreenCanvas.convertToBlob`), since no lossy encoder is written
     in Rust. A browser that cannot write WebP (Safari) is told to choose Lossless or GIF.
     Only the changed rectangle is stored.
   - The export window offers the formats the host writes; in a browser, GIF and WebP
     for now.
   - Checked in headless Chromium: an animated GIF picked with Ctrl+O opens with its
     timeline, plays, and exports as a GIF and as a lossy WebP; Chromium's `ImageDecoder`
     reads both back with every frame and the full two seconds. An animated WebP opens and
     plays. (FFmpeg reads the GIF; it has no decoder for animated WebP.)
2. **Video in (done).** MP4, MOV, WebM and MKV open in the browser, scrub, play and export
   as GIF or WebP.
   - `crtsim_media::demux` reads the container in Rust: every frame's place in the file, when
     it shows and whether it is a keyframe, the codec as WebCodecs names it and the decoder's
     configuration, the track's rotation, and the audio track for step 3. MP4 and MOV are
     read by `re_mp4`; WebM and Matroska by a small EBML reader here, which keeps frames in
     place rather than copying them and follows clusters written without their size, as a
     recording browser writes them. VP9's profile and bit depth come from its first keyframe.
   - `Source::Demuxed` is a video whose frames the host decodes. In the browser, `web_video`
     feeds WebCodecs' `VideoDecoder` from the keyframe before the wanted time, a few samples
     ahead, and draws each frame upright into an `OffscreenCanvas` to read it as RGBA.
     WebCodecs is bound by hand, since web-sys has it only behind an unstable flag.
   - `page::Ticks` picks, from frames in showing order, the one each tick of a constant rate
     takes, by the same rule as the animation schedule and FFmpeg's `fps` filter.
   - A codec the browser cannot decode is refused by name, pointing to the desktop app.
   - Checked in headless Chromium, with clips whose frames each code their number in full
     red, green and blue: VP9 in WebM and in MP4 open; frames 1 and 4 export as PNGs showing
     those frames; a GIF of the whole clip at 24 per second holds each frame for exactly the
     ticks the rule gives; a clip rotated by its MP4 matrix shows upright as FFmpeg shows it;
     playback runs to the end. This Chromium build has no H.264 decoder (open-source builds
     leave it out), so H.264 was checked only as far as the refusal naming it; Chrome, Edge,
     Safari and Firefox decode it.
3. **Video out (done).** MP4 and WebM export in the browser, with their sound.
   - `crtsim_media::mux` writes MP4 (H.264 or VP9; AAC or Opus) with its index before the
     frames, in half-second chunks of picture and sound in turn, and WebM (VP8 or VP9; Opus or
     Vorbis) with a cluster per keyframe, cues and a seek head, so both play and seek as they
     load. Decoding times are the showing times in order, with signed offsets if an encoder
     reorders frames.
   - `page::export_video` renders at the export's rate into the host's `VideoEncoding`, with a
     keyframe every two seconds and a bit rate from the quality, then adds the sound: copied
     when the container holds its codec, and otherwise, or when asked, converted to Opus by
     the host. A browser that cannot convert it says so and suggests No audio or another
     format, rather than dropping the sound.
   - `web_encode` binds WebCodecs' `VideoEncoder`, `AudioDecoder` and `AudioEncoder` by hand.
     When the page starts it asks which of H.264 and VP9 the browser encodes, and the export
     window offers MP4 and WebM only where it can. The desktop's encoder, CRF and track
     settings are not shown in a browser.
   - The MP4 reader now finds Opus sound, which `re_mp4` leaves unnamed, and sound in WebM
     keeps in step with a video that does not start at zero.
   - Checked natively: the fixtures' frames and sound written as MP4 and WebM read back byte
     for byte, and FFmpeg decodes each file without an error. Checked in headless Chromium:
     VP9 clips without sound, with Opus and with Vorbis export as WebM at 1920x1080 with every
     frame (10 and 50) and the sound copied; FFmpeg decodes them cleanly; Chromium plays each to
     the end showing every frame and decoding its sound, and seeks in them as in FFmpeg's own
     WebM. Asked to convert, Vorbis became 48 kHz Opus. AAC, which this Chromium cannot decode,
     stopped the export with the message above. This Chromium has no H.264 encoder, so it
     offered WebM only, as it should; MP4 from a browser needs a check by hand in Chrome, Edge,
     Safari or Firefox.

## Polish track

These help both hosts, so they are worth doing before or during steps 4 to 6:

- **Notices**: one module for what the app tells the person, one notice per source, each with
  its own dismiss and an optional action (proposal 3 in the [UX plan](UX_PLAN.md)).
- **Source**: one module for the open image or video, which also gives the preview's config.
  This fixes playback ignoring an audition.
- **Settings drawn from core's description**, so a setting is added in one place.

## From a release to the site (step 8, done)

1. Tagging `vX.Y.Z` runs `releases.yml`. Besides the desktop packages, its `web` job builds
   the web app with `tools/build_web.py` and packages it with `tools/package.py --platform web`
   as `CRTSim-Renderer-vX.Y.Z-web.zip` and `SHA256SUMS-web.txt`, which go into the draft
   release. The zip holds the page, its module and the module's JavaScript at its root, with
   the licenses and notices; the dependencies' license texts are one file, each text once.
   Running the workflow by hand with `release_tag` adds the web app to a release published
   before it existed, as v0.7.0 was, and leaves the desktop packages alone.
2. The maintainer publishes the draft.
3. In `aizumanga/aizumanga-neocities`, the **Update CRTSim** workflow runs daily and on demand.
   When the latest published release is newer than the one on the site,
   `tools/update-crtsim.mjs` downloads the web zip and checks its checksum. It refuses a zip
   holding folders or missing the app's files before touching anything. It then replaces
   `crtsim/`, sets `VERSION` and turns on `WEB_APP` in `scripts/crtsim-downloads.js`, and the
   workflow opens a pull request.
4. Merging the pull request runs the site's existing **Deploy to Neocities**, which uploads the
   changed files. The downloads dialog and the web app then name the same release.

The web app lives at `aizumanga.neocities.org/crtsim/`. The downloads dialog behind the site's
**CRTSim** button offers **Open in your browser** first, above the platform downloads. Neocities
accepts the `.wasm` module only from Supporter accounts, which the site is. The dialog's
no-JavaScript fallback links point at the latest release, so they need no edit per version.

Checked: the web zip built and packaged here, installed by the site's updater into a copy of
the site, served the app from `/crtsim/`, which rendered and exported the test pattern in
headless Chromium; the dialog offered it first and named the release. The updater run against
GitHub found v0.7.0 without a web zip, warned, and changed nothing. The site's own validation
passes with the workflow's test added to it.

## Home page showcase

With the next release, the site's home page gains a small `CRTSim_Renderer.exe` window below
**Welcome to my corner!**: a before/after slider over one sample at a time, with thumbnails to
switch between an image and a video sample, and a **Download** button that opens the existing
downloads dialog. **Open in your browser** joins it once the web app ships. The samples are the
maintainer's own source and CRT pairs at matching sizes; the page gets smaller display copies,
and videos load only when the window scrolls into view.
