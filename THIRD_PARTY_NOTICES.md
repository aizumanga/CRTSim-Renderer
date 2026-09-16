# Third-party notices

The root CC0 dedication applies to this project's original contributions, not to dependencies.

## CRTSim

J. Kyle Pittman, CC0 1.0. Original dedication: `assets/original-crtsim/COPYING.txt`.
Repository and exact source commit: `assets/original-crtsim/SOURCES.md`.
The WGSL shader is a translation/adaptation of the shared effect.

## Rust dependencies

Cargo.lock records the exact versions used by this prototype. Their own license terms remain applicable.
Direct dependencies include wgpu/Naga, image, glam, bytemuck, serde/serde_json, anyhow, clap, pollster,
eframe/egui, rfd and tempfile. The desktop uses eframe's bundled fonts, which retain their upstream notices.
Before distributing binaries, collect the complete license texts for all transitive dependencies using a license-reporting tool such as cargo-about.
This document is a development inventory, not a completed binary-distribution license bundle.

FFmpeg is not bundled, linked, downloaded or used in Phases 0 or 1.
