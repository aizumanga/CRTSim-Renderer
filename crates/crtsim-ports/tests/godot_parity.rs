//! The Godot effect against the renderer: each case's look applied to the effect in Godot
//! itself, on Vulkan in a window of Xvfb's, and compared with the renderer's own picture. See
//! `crtsim_ports::parity` for the cases and their limits.
//!
//! ```text
//! CRTSIM_GODOT=path/to/godot xvfb-run -a cargo test -p crtsim-ports --test godot_parity -- --ignored --nocapture
//! ```

use crtsim_core::config::{Config, Filter as Resize};
use crtsim_ports::godot::Engine;
use crtsim_ports::parity::{self, Case, Limits, UNFILTERED};
use std::path::PathBuf;

/// The resize computes its Lanczos weights in the shader, where the renderer takes the image
/// crate's.
const RESIZED: Limits = Limits {
    worst: 2,
    mean: 0.02,
};

/// Godot hands the effect its picture, and takes it back, in linear light in half floats, and
/// draws the canvas into it and the window from it with its own sRGB curves, which moves a
/// channel a step on a few percent of pixels: measured at 0.0002 to 0.036 of a step on average
/// past each case's limit, never more than its worst.
const ROUND_TRIP: f64 = 0.04;

/// The effect draws at the camera's size, so the renderer is asked for that size too.
fn at_camera_size(mut case: Case) -> Case {
    let (width, height) = case.steps.last().expect("a step").0.dimensions();
    case.config.output = format!("{width}x{height}");
    case.limits.mean += ROUND_TRIP;
    case
}

/// Godot hands the effect a picture of any size, which it resizes to the signal itself: the
/// test card, as the renderer is given it, resized by each filter.
fn resized(name: &'static str, signal: &str, filter: Resize) -> Case {
    let config = Config {
        signal: signal.into(),
        filter,
        ..parity::base()
    };
    let card = crtsim_core::config::test_card();
    Case {
        name,
        steps: vec![(card.clone(), config.warmup + 1)],
        source: card,
        config,
        limits: if filter == Resize::Lanczos {
            RESIZED
        } else {
            UNFILTERED
        },
        timing: parity::Timing::Ticks,
    }
}

#[test]
#[ignore = "requires Godot 4.3 or later, named by CRTSIM_GODOT, a Vulkan adapter and an X display"]
fn the_effect_draws_what_the_renderer_draws() {
    let godot = PathBuf::from(std::env::var_os("CRTSIM_GODOT").expect("CRTSIM_GODOT names Godot"));
    let engine = Engine::new(&godot).expect("Godot");
    let dir = parity::output_dir("godot");
    let mut cases: Vec<_> = parity::cases().into_iter().map(at_camera_size).collect();
    cases.push(at_camera_size(resized(
        "resize-lanczos",
        "160x140",
        Resize::Lanczos,
    )));
    cases.push(at_camera_size(resized(
        "resize-nearest",
        "100x80",
        Resize::Nearest,
    )));
    parity::check(cases, &dir, |case, folder| {
        engine
            .render(
                &case.config,
                &case.steps,
                case.timing.frame_seconds(),
                folder,
            )
            .expect("the effect's frame")
    });
}
