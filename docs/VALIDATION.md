# Renderer validation

This is a runnable prototype, not a claim of completed cross-platform visual parity.

## Phase 2 checks

CPU tests cover old JSON defaults, exact neutral grading, gray chroma output, gallery persistence, welcome acknowledgement,
duplicate/path-name protection, corrupt-file isolation and all built-in configuration validation.
The GPU test covers linear-light float targets, filtered masks and monotonic progress through 100%.
CI captures the actual welcome window, desktop and gallery, plus linear-light and filtered-mask diagnostics.
Native file dialogs, real GPU performance and cross-driver visual equivalence remain manual acceptance work.

## Phase 1 desktop checks

Five desktop tests cover aspect-preserving preview resolution without changing signal dimensions,
settings undo/redo, JSON preset round-trips, atomic PNG/preset replacement, rejection of stale preview results,
and export snapshots preserving the original source and full output resolution while settings change.
The stale-result test also checks that a successful preview does not erase an image-loading error.
The existing six core tests and upstream asset hashes remain applicable. Local tests, formatting and strict Clippy pass.

CI builds/tests the desktop crate on Linux, Windows and macOS. The Vulkan job also opens an actual window under Xvfb,
waits for a rendered preview, captures `desktop.png`, and fails on render/screenshot timeout.
The screenshot is part of the same render-fixtures artifact. Platform run results are recorded in the Phase 1 PR.

[Run 35053287376](https://github.com/aizumanga/CRTSim-Renderer/actions/runs/35053287376)
passed all three platform jobs and the Vulkan/desktop runtime job for commit `6c746e77d071f341af8e05f0e8480d9541aa1620`.
The actual 1280x850 window screenshot was inspected: controls and the 1280x720 CRT preview are visible without overlapping panels.
An initial Xvfb launch failed because the runner lacked `libxkbcommon-x11.so.0`; provisioning that runtime dependency fixed it.
A subsequent change keeps file errors separate from preview errors; its checks are available on the PR.

Manual acceptance on real desktops still includes native open/save dialogs, drag-and-drop, rapid slider changes,
high-DPI resizing/zoom, image replacement during rendering, and export of a user image at the selected resolution.
macOS runtime behavior, Wayland portal integration and real-GPU performance are not established by an Xvfb run.

## Recorded result

[CI run 35018126875](https://github.com/aizumanga/CRTSim-Renderer/actions/runs/35018126875)
passed all four jobs for implementation commit `5881e60337066359d78bb1bbd1f39e7b1a0d6b3a`:

- Linux, Windows and macOS: compilation, six unit tests, formatting, strict Clippy and original-asset hashes.
- Linux software Vulkan: GPU smoke test and all image exports, including 3840x2160 output.
- The smoke test exercised odd-width readback, repeatable still jobs, distinct artifact phases and disabled-effect identity.

The reference, 720p, 1080p, 4K and automatic-resize images were visually inspected on 2026-09-16.
They show the expected test-card layout, curved glass, mask, bloom and frame reflections, with no gross channel swap or vertical inversion.
Fine mask patterns change with output sampling; this inspection does not establish correct minification or pixel equivalence to D3D9.
The automatic-resize fixture reuses the generated 4K image as input, so its nested CRT appearance is intentional.
The run's `phase0-render-fixtures` artifact contains the images and resolved settings (14-day retention).
These are reproducible diagnostics, not approved golden images or a real-GPU performance benchmark.

## Checks

- `cargo test --workspace --locked`: configuration, aspect, alpha, malformed meshes, attribute preservation, WGSL parsing/validation.
- `cargo test -p crtsim-core gpu_smoke -- --ignored --nocapture`: actual pipelines and readback, odd-width row padding,
  deterministic resets, phase differences, and clean-signal identity when temporal/artifact/sharpness effects are disabled.
- `python3 tools/verify_assets.py`: every vendored file matches the pinned original digest.
- CI compiles/tests on Linux, Windows and macOS, with a separate Linux software-Vulkan render job.
- CI uploads reference/720p/1080p/4K/A/B fixtures and debug intermediates for visual inspection.

## Remaining fidelity checks

- Compare the same input against a running D3D9 upstream reference, not a compressed game video.
- Check mask aliasing at all output/display sizes; reference mode samples mip zero, while the optional filter chooses mip levels from screen-space derivatives.
- Check mesh projection, channel order, UV orientation and reflected corners on multiple adapters.
- Compare Vulkan and DX12 captures of the same preset using perceptual tolerance, not exact hashes.
- Test Metal on actual Mac hardware. A CI build alone is insufficient.
- Expand temporal tests with moving bright objects, dark decay sequences and high persistence.
- Benchmark on real GPUs; software-Vulkan speed is not a consumer performance estimate.

## Environment

The development workspace can compile Rust and validate shader source, but initially has no configured Vulkan ICD.
Driver package installation was blocked by workspace permissions. No attempt is made to bypass that restriction.
GPU runtime validation is delegated to the repository's explicit CI job or a user's configured GPU machine.
