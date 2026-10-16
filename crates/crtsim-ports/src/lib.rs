//! Runs the shader ports headless, so tests can compare their pixels with the renderer's: the
//! RetroArch port through librashader, the OBS filter through OBS itself. See
//! docs/adr/0003-games-get-shader-ports-not-the-renderer.md.
//!
//! This crate is for tests and benchmarks and never ships, and nothing that ships may depend
//! on it.

pub mod obs;
pub mod parity;
pub mod retroarch;
