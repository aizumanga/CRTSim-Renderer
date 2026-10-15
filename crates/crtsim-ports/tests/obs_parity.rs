//! The OBS filter against the renderer: each case's look applied to the filter in OBS itself,
//! headless on Xvfb, and compared with the renderer's own picture. See
//! `crtsim_ports::parity` for the cases and their limits.
//!
//! ```text
//! cmake -S ports/obs -B build/obs -DCRTSIM_OBS_TEST=ON && cmake --build build/obs
//! CRTSIM_OBS_BUILD=build/obs xvfb-run -a cargo test -p crtsim-ports --test obs_parity -- --ignored --nocapture
//! ```

use crtsim_core::config::{Config, Filter as Resize, Phase};
use crtsim_ports::obs::Filter;
use crtsim_ports::parity::{self, Case, Limits, UNFILTERED};
use std::path::PathBuf;

/// OBS draws with OpenGL here, whose arithmetic differs from Vulkan's in its last bits. Where
/// each tick's signal differs from the last -- alternating phase, interlaced fields -- the glow
/// carried between them settles a step apart on a few percent of pixels within ten ticks, and
/// stays there: measured at 0.014 and 0.036 of a step on average, never more than 2 on any
/// pixel. A steady picture stays within 0.0005.
const ALTERNATING: Limits = Limits {
    worst: 2,
    mean: 0.05,
};
/// The resize computes its Lanczos weights in the shader, where the renderer takes the image
/// crate's: measured at 0.0103 of a step on average.
const RESIZED: Limits = Limits {
    worst: 2,
    mean: 0.02,
};

/// The filter draws at its source's size, so the renderer is asked for that size too.
fn at_source_size(mut case: Case) -> Case {
    let (width, height) = case.steps.last().expect("a step").0.dimensions();
    case.config.output = format!("{width}x{height}");
    let alternating = case.config.phase == Phase::Alternating || case.config.interlace;
    if alternating {
        case.limits = ALTERNATING;
    }
    case
}

/// OBS hands the filter a picture of any size, which it resizes to the signal itself: the
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
#[ignore = "requires OBS's development files, the plugin built with CMake, and an X display"]
fn the_filter_draws_what_the_renderer_draws() {
    let build = PathBuf::from(
        std::env::var_os("CRTSIM_OBS_BUILD").expect("CRTSIM_OBS_BUILD names the plugin's build"),
    );
    let dir = parity::output_dir("obs");
    let filter = Filter::new(&build, &dir).expect("the plugin and its data");
    let mut cases: Vec<_> = parity::cases().into_iter().map(at_source_size).collect();
    cases.push(at_source_size(resized(
        "resize-lanczos",
        "160x140",
        Resize::Lanczos,
    )));
    cases.push(at_source_size(resized(
        "resize-nearest",
        "100x80",
        Resize::Nearest,
    )));
    parity::check(cases, &dir, |case, folder| {
        filter
            .render(
                &case.config,
                &case.steps,
                case.timing.frame_seconds(),
                folder,
            )
            .expect("the filter's frame")
    });
}
