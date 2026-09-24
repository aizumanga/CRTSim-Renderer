# Third-party notices

The root CC0 dedication applies to this project's original contributions, not to dependencies.

## CRTSim

J. Kyle Pittman, CC0 1.0. Original dedication: `assets/original-crtsim/COPYING.txt`.
Repository and exact source commit: `assets/original-crtsim/SOURCES.md`.
The CRT shader, `shaders/crtsim.wgsl`, is a translation/adaptation of the shared effect. `shaders/prepare.wgsl` is original to this project.

## Rust dependencies

Cargo.lock records the exact versions used by this prototype. Their own license terms remain applicable.
Direct dependencies include wgpu/Naga, image, glam, bytemuck, serde/serde_json, anyhow, clap, pollster,
eframe/egui, rfd, tempfile and directories. The desktop uses eframe's bundled fonts, which retain their upstream notices.
Before distributing binaries, collect the complete license texts for all transitive dependencies using a license-reporting tool such as cargo-about.
This document is a development inventory, not a completed binary-distribution license bundle.

## FFmpeg

Phase 3 invokes a user-installed FFmpeg and ffprobe as separate executables. Neither is bundled, linked or downloaded by this application.
The installed build determines codec availability and its license obligations; builds containing libx264 generally enable GPL components.
Before distributing FFmpeg with a future release, collect the exact build configuration, applicable licenses and corresponding source obligations.

## Included NES LUTs

NES LUT collection/update by **Wellington Uemura (wtuemura)**, distributed through
[MAME Goodies](https://github.com/mamedev/mame-goodies/tree/master/bgfx/lut/nes)
under **CC0 1.0**, as stated in the upstream NES README. Thanks to the MAME Goodies contributors.
Includes palettes by [FirebrandX (FBX)](https://www.firebrandx.com/nespalette.html)
and other creators identified in the original palette names, which are preserved.

[Author's announcement](https://www.reddit.com/r/emulation/comments/1oopf1i/updated_nes_luts_for_mame/).
The unmodified PNGs, upstream dedication, source notes and SHA-256 digests are in
`assets/nes-luts/`. Release packages include `NES_LUTS_README.md`, `NES_LUTS_SOURCES.md`
and `NES_LUTS_SHA256SUMS`; the LUT data is embedded in the executable.

## NES palette generator

`crtsim-core/src/palette.rs` reproduces MAME's NES palette (the formula in
`ppu2c0x_device::nespal_to_RGB`, [mamedev/mame](https://github.com/mamedev/mame),
BSD-3-Clause, copyright holders Ernesto Corvi, Brad Oliver and Fabio Priuli) as the input
side of its LUT, since the bundled NES LUTs expect that palette. The composite decode uses the
NES PPU's measured output levels as documented by the [NESdev Wiki](https://www.nesdev.org/wiki/NTSC_video),
with its chroma scale and phase fitted to the FirebrandX Composite Direct palette above.
