//! The ReShade effect against the renderer, in vkBasalt: each case's look set on the effect,
//! which vkBasalt draws over the frames of a window showing the case's pictures, on Vulkan in a
//! window of Xvfb's, and the window compared with the renderer's own picture. See
//! `crtsim_ports::parity` for the cases and their limits.
//!
//! ```text
//! CRTSIM_VKBASALT=/usr/share/vulkan/implicit_layer.d/vkBasalt.json xvfb-run -a cargo test -p crtsim-ports --test reshade_vkbasalt -- --ignored --nocapture
//! ```

use crtsim_core::config::{Config, Filter as Resize};
use crtsim_ports::parity::{self, Case, Limits, SEAM_PIXELS, UNFILTERED};
use crtsim_ports::vkbasalt::Layer;
use std::path::{Path, PathBuf};

/// The resize computes its Lanczos weights in the shader, where the renderer takes the image
/// crate's.
const RESIZED: Limits = Limits {
    worst: 2,
    mean: 0.02,
    past: SEAM_PIXELS,
};

/// ReShade hands the effect the game's whole picture and draws the effect at its size, so the
/// renderer is asked for that size too.
fn at_buffer_size(mut case: Case) -> Case {
    let (width, height) = case.steps.last().expect("a step").0.dimensions();
    case.config.output = format!("{width}x{height}");
    case
}

/// A game's picture of any size, which the effect resizes to the signal itself: the test card,
/// as the renderer is given it, resized by each filter.
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
#[ignore = "requires vkBasalt, its layer named by CRTSIM_VKBASALT, Vulkan and an X display"]
fn vkbasalt_draws_the_effect_as_the_renderer_draws() {
    let manifest = PathBuf::from(
        std::env::var_os("CRTSIM_VKBASALT").expect("CRTSIM_VKBASALT names vkBasalt's layer"),
    );
    let layer = Layer::new(&manifest, Path::new(env!("CARGO_BIN_EXE_present"))).expect("vkBasalt");
    let mut cases: Vec<_> = parity::cases().into_iter().map(at_buffer_size).collect();
    cases.push(at_buffer_size(resized(
        "resize-lanczos",
        "160x140",
        Resize::Lanczos,
    )));
    cases.push(at_buffer_size(resized(
        "resize-nearest",
        "100x80",
        Resize::Nearest,
    )));
    parity::check(cases, &parity::output_dir("vkbasalt"), |case, folder| {
        layer
            .render(
                &case.config,
                &case.steps,
                case.timing.frame_seconds(),
                folder,
            )
            .expect("vkBasalt's frame")
    });
}
