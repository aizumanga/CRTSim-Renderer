# Phase 0 validation

This is a runnable prototype, not a claim of completed cross-platform visual parity.

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
- Check mask aliasing at all output sizes; this prototype samples mask mip level zero.
- Check mesh projection, channel order, UV orientation and reflected corners on multiple adapters.
- Compare Vulkan and DX12 captures of the same preset using perceptual tolerance, not exact hashes.
- Test Metal on actual Mac hardware. A CI build alone is insufficient.
- Expand temporal tests with moving bright objects, dark decay sequences and high persistence.
- Benchmark on real GPUs; software-Vulkan speed is not a consumer performance estimate.

## Environment

The development workspace can compile Rust and validate shader source, but initially has no configured Vulkan ICD.
Driver package installation was blocked by workspace permissions. No attempt is made to bypass that restriction.
GPU runtime validation is delegated to the repository's explicit CI job or a user's configured GPU machine.
