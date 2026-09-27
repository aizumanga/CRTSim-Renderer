# 2. The web app is the whole app, on WebGPU, on one thread, without FFmpeg

Status: accepted.

The **Web app** is the whole app compiled for the browser: the same egui interface, galleries,
playback and exports. It is not a cut-down demo, and it is not a separate page written in
JavaScript. It is served from the author's Neocities supporter site, as `.wasm`.

## Decision

- **WebGPU only.** Browsers without WebGPU see a page that says how to turn it on (Firefox and
  LibreWolf on Linux: `dom.webgpu.enabled` in `about:config`) and offers the downloads.
- **One thread.** The app runs on the page's main thread. Rendering is already asynchronous GPU
  work; CPU-heavy steps are handed to the browser (`createImageBitmap` to decode,
  `OffscreenCanvas.convertToBlob` for PNGs, WebCodecs for video), which runs them off the page.
- **No FFmpeg.** Video is decoded and encoded with WebCodecs and a small muxer. GIF and animated
  WebP are encoded in Rust. The formats offered are those the browser can write.
- **Files in, downloads out.** People open files by picking or dropping them. Every save,
  batch outputs included, is a download. App data lives in IndexedDB.

## Considered options

- **A WebGL2 fallback** would reach Linux Firefox today. It was rejected because the linear-light
  passes render to 32-bit float targets, so it would cover only the gamma-space mode and need
  golden checks of its own, for a gap that closes when Firefox enables WebGPU on Linux.
- **Wasm threads** need a cross-origin-isolated page, and Neocities cannot send the
  `Cross-Origin-Opener-Policy` and `Cross-Origin-Embedder-Policy` headers. A service worker
  that adds them (coi-serviceworker) works but needs nightly Rust and fragile tooling. A Web
  Worker with its own GPU device would have to copy every preview back to the page.
- **ffmpeg.wasm** would match the desktop's formats. It was rejected for the same reasons as
  [ADR 1](0001-find-ffmpeg-never-download-it.md): it is GPL, and it is tens of megabytes. It is
  also slow without threads.
- **The File System Access API** would let saves write into a folder, but Firefox and LibreWolf
  do not have it.

## Consequences

- wgpu and eframe must be upgraded first. wgpu 0.19 asks for `maxInterStageShaderComponents`,
  which Chrome 135 removed, so it cannot create a device in current Chrome. (Done: wgpu 30 and
  eframe 0.36, with every golden image unchanged.)
- The renderer may not block. `device.poll(Maintain::Wait)` and waiting on a channel for
  `map_async` never finish on a browser's main thread.
- The render worker gets two adapters: a thread on the desktop and an async task in the browser.
  (Done: `worker::Runtime`.)
- `crtsim-desktop` becomes a library holding the whole app (`crtsim-app`), with a small native
  `main` and a web entry point. (Done for the native side: the executable keeps its name.)
