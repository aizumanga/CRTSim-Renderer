//! Times a preset on the fastest adapter, as a game would run it every frame:
//!
//! ```text
//! cargo run --release -p crtsim-ports --bin bench -- <preset.slangp> [--size 1920x1080] [--frames 2000]
//! ```
//!
//! `WGPU_BACKEND` picks the backend; the input is the renderer's test card at 60 frames a second.

use anyhow::{bail, Context, Result};
use crtsim_ports::{Port, RetroArch};
use std::path::PathBuf;
use std::time::Duration;

/// Frames rendered and thrown away first, so pipeline creation and the first frame's
/// allocations are not timed.
const WARM_UP: u32 = 60;
const FRAME_TIME_US: u32 = 16_667;

fn main() -> Result<()> {
    let mut preset = None;
    let (mut size, mut frames) = ((1920, 1080), 2000);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--size" => {
                let value = args.next().context("--size needs WIDTHxHEIGHT")?;
                let (width, height) = value.split_once('x').context("--size is WIDTHxHEIGHT")?;
                size = (width.parse()?, height.parse()?);
            }
            "--frames" => frames = args.next().context("--frames needs a count")?.parse()?,
            _ if preset.is_none() && !arg.starts_with('-') => preset = Some(PathBuf::from(arg)),
            _ => bail!("usage: bench <preset.slangp> [--size WIDTHxHEIGHT] [--frames N]"),
        }
    }
    let preset =
        preset.context("usage: bench <preset.slangp> [--size WIDTHxHEIGHT] [--frames N]")?;
    anyhow::ensure!(frames > 0, "--frames must be at least 1");

    let backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY);
    let mut port = Port::load_on(backends, &preset, RetroArch::Newer)?;
    let adapter = port.adapter();
    println!(
        "{} ({:?}, {:?}, driver {})",
        adapter.name, adapter.backend, adapter.device_type, adapter.driver_info
    );
    let input = crtsim_core::config::test_card();
    let aspect = 4.0 / 3.0;
    port.time(&input, size, WARM_UP, FRAME_TIME_US, aspect)?;
    let timing = port.time(&input, size, frames, FRAME_TIME_US, aspect)?;

    println!(
        "{} at {}x{}, {frames} frames",
        preset.display(),
        size.0,
        size.1
    );
    match timing.gpu {
        Some(mut gpu) => {
            gpu.sort();
            let at = |q: f64| ms(gpu[((gpu.len() - 1) as f64 * q).round() as usize]);
            let mean = ms(gpu.iter().sum::<Duration>() / frames);
            println!(
                "GPU per frame: mean {mean:.3} ms, median {:.3}, p95 {:.3}, min {:.3}, max {:.3}",
                at(0.5),
                at(0.95),
                at(0.0),
                at(1.0)
            );
        }
        None => println!("GPU time: the adapter has no timestamp queries"),
    }
    println!(
        "wall-clock per frame, submitting one at a time: {:.3} ms",
        ms(timing.wall)
    );
    Ok(())
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}
