//! Runs the shader ports headless, so tests can compare their pixels with the renderer's: the
//! RetroArch port through librashader and in RetroArch itself, the ReShade effect in vkBasalt,
//! the OBS filter through OBS itself, and the Godot effect through Godot itself. See
//! docs/adr/0003-games-get-shader-ports-not-the-renderer.md.
//!
//! This crate is for tests and benchmarks and never ships, and nothing that ships may depend
//! on it.

pub mod godot;
pub mod obs;
pub mod parity;
pub mod retroarch;
pub mod retroarch_real;
pub mod vkbasalt;
