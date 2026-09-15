# Phase 0 validation

This is a runnable prototype, not a claim of completed cross-platform visual parity.

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
