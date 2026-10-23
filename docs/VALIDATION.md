# Renderer validation

This is a runnable prototype, not a claim of completed cross-platform visual parity.

## Golden-image rendering regression

`cargo test -p crtsim-core --test golden -- --ignored --nocapture` compares thirty-six rendered
images against images committed in `crates/crtsim-core/tests/golden/`. The CI fixtures show
that a render finished; these show that it still produces the same picture. Twenty-four take one
route each through the renderer: the reference frame and its signal, both composite phases,
NTSC blending and the flipped artifact pattern in the signal, linear-light, the unfiltered mask,
screen-only, the bezel's reflection as on the screen, a backdrop, a widescreen and a portrait
tube, interlaced scanning, a bundled NES LUT, the bloom from mipmaps, the glow floor, FXAA at
its finest, a brighter picture, the picture turned for tate, the CRT off, TV static in the
signal, Super Win the Game's look as a whole, drawn as the game's screenshots, and the second
frame of a sequence, which is the only case that sees frame-to-frame persistence.

The other twelve capture the prepare step's output -- the clean signal, before the simulation --
since the test card reaches it only as an identity: Lanczos down, up and along one axis, Nearest
down and up, the alpha composite, source edits with each sampler, the grade, the NES palette,
a LUT at half strength, and a LUT with the grade. They were rendered by the CPU prepare step and still pass bit-exactly now that it runs on
the GPU.

Comparison is effectively bit-exact (max one 0-255 step on any channel, mean under 0.002).
That is not a cosmetic choice: perceptual limits loose enough to survive a driver change were
measured against a deliberate regression -- luma weights rounded to 0.30/0.59/0.11 -- and
missed it in every case it was tried on, because that edit moves the mean by only 0.015-0.039 steps. A
single driver renders a frame bit-exactly, so tight limits are what makes the test able to
fail at all.

The driver that produced the goldens is recorded in `tests/golden/driver.txt`, and a different
driver version fails with that message rather than with pixel differences. A Mesa upgrade in CI
is therefore a deliberate review point, not a silent shift.

## GPU prepare step

Resize, alpha composite, source edits, LUT and grade run on the GPU (`crates/crtsim-core/shaders/prepare.wgsl`).
The shader repeats the CPU implementation's arithmetic rather than improving on it -- the image
crate's separable resize with weights computed on the CPU exactly as that crate computes them,
and the rounding to 8 bits between stages -- so the move changes where the work runs and not
the picture. `config::prepare` stays as the reference, and as the fallback for an input too
large to prepare on the device.

- `gpu_prepare::tests::kernels_reproduce_the_image_crates_resize_exactly` (no GPU) checks the
  weight tables against `image::imageops::resize` for both filters.
- `gpu_prepare_matches_cpu_prepare` (Vulkan) renders 48 varied settings both ways and allows at
  most one step on any channel. On lavapipe no channel differs at all. Elsewhere, float rounding
  can put a value on the other side of a half step. A source edit sampled Nearest can also pick
  the neighbouring pixel where a sample falls within float error of a pixel edge.
- `crtsim render --prepare cpu` renders with the CPU implementation for a direct comparison on
  real hardware.

After an intended rendering change, refresh and review the image diff:

```sh
CRTSIM_UPDATE_GOLDEN=1 cargo test -p crtsim-core --test golden -- --ignored
```

A mismatch writes the rendered frame and a 64x-amplified difference image into
`$CRTSIM_TEST_OUTPUT/golden-failures/`, which the Vulkan job uploads with the other fixtures
even when it fails. When the goldens have to be regenerated against a driver only CI has, an
update run there leaves them in the same uploaded directory to download and commit.

## Colour management

A picture is read into sRGB from the profile its file carries, with moxcms (`crtsim_core::color`).
`python3 tools/check_icc.py` converts the same pictures with lcms 2.14, through ImageMagick, and
with the renderer, and compares them. Recorded on 2026-10-19:

| Profile | Most steps apart | Within 1 step | Within 2 |
| --- | --- | --- | --- |
| Adobe RGB (1998), colord's | 2 | 99.9% | 100% |
| ProPhoto RGB | 2 | 99.9% | 100% |
| Apple RGB, gamma 1.8 | 2 | 100% | 100% |
| Rec. 709 | 1 | 100% | 100% |
| FOGRA39 coated, a CMYK JPEG | 7 | 91.0% | 99.7% |

The CMYK differences are where a colour lies outside sRGB. On a picture with more saturated cyans
than the script's, a few pixels in a thousand came out up to 18 steps apart: lcms clips their red
to 0, moxcms leaves it at up to 18 (a linear 0.006). In-gamut colours agree.

Unit tests (`color::tests`, `input::tests`) check without either program that a file's profile is
found in PNG, JPEG and WebP and in PNG's order of chunks, that a program's own copy of sRGB and a
gAMA of 1/2.2 leave every value as it is, that 16-bit and grey pictures convert through their
own layouts, and that a still written in Display P3 reads back as the sRGB picture rendered.

## Phase 2 checks

CPU tests cover old JSON defaults, exact neutral grading, gray chroma output, gallery persistence, welcome acknowledgement,
duplicate/path-name protection, corrupt-file isolation and all built-in configuration validation.
Theme tests cover unique stable identifiers, every built-in palette applying to an egui context, and app-data persistence.
The GPU test covers linear-light float targets, filtered masks and monotonic progress through 100%.
CI captures the default dark desktop plus Sky Diary welcome/gallery windows, along with linear-light and filtered-mask diagnostics.
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
  The Super Win the Game look is compared with the game itself, its Linux build pass by pass
  and its Windows build under Wine (`docs/research/swtg-crt`).
- Check mask aliasing at all output/display sizes. The filtered mask, the default as in the original, chooses mip levels from screen-space derivatives; with filtering off it samples only mip zero.
- Check mesh projection, channel order, UV orientation and reflected corners on multiple adapters.
- Compare Vulkan and DX12 captures of the same preset using perceptual tolerance, not exact hashes.
- Test Metal on actual Mac hardware. A CI build alone is insufficient.
- Expand temporal tests with moving bright objects, dark decay sequences and high persistence.
- Benchmark on real GPUs; software-Vulkan speed is not a consumer performance estimate.

## Environment

A development container without a GPU runs the GPU tests on Mesa's software Vulkan
(`apt-get install mesa-vulkan-drivers`, with `XDG_RUNTIME_DIR` set), as CI does; the goldens
record the driver they were made with. Otherwise GPU runtime validation is delegated to the
repository's explicit CI job or a user's configured GPU machine.
