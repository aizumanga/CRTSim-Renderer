//! The RetroArch port against the renderer, in RetroArch itself: each case's look exported as
//! a preset and drawn over its picture, which RetroArch's image viewer shows, on Xvfb, then
//! compared with the renderer's own picture. See `crtsim_ports::parity` for the cases and their
//! limits.
//!
//! ```text
//! CRTSIM_RETROARCH=retroarch xvfb-run -a cargo test -p crtsim-ports --test retroarch_real -- --ignored --nocapture
//! ```

use crtsim_core::config::Phase;
use crtsim_ports::parity::{self, Case, Timing};
use crtsim_ports::retroarch_real::{Driver, Frontend};
use std::path::PathBuf;

/// Whether the case's picture depends on `FrameCount`: the composite phase that alternates
/// each frame, the field an interlaced picture scans, which the port takes as interlaced from
/// 400 rows unless the look says, and TV static, whose noise is the frame's.
fn counts_frames(case: &Case) -> bool {
    case.config.phase == Phase::Alternating
        || case.config.interlace
        || case.steps[0].0.height() >= 400
        || case.config.static_amount > 0.
}

fn check(driver: Driver) {
    let retroarch = PathBuf::from(
        std::env::var_os("CRTSIM_RETROARCH").expect("CRTSIM_RETROARCH names RetroArch"),
    );
    let dir = parity::output_dir(&format!("retroarch-{}", driver.name()));
    let frontend = Frontend::new(&retroarch, driver, &dir.join("counting")).expect("RetroArch");
    let counting = frontend.counting();
    eprintln!(
        "RetroArch {:?} on {}: {counting:?}",
        frontend.version(),
        driver.name()
    );
    let cases = parity::cases().into_iter().filter(|case| {
        if case.steps.len() > 1 || !matches!(case.timing, Timing::Ticks) {
            eprintln!("{}: skipped, the image viewer shows one picture", case.name);
            return false;
        }
        // RetroArch's Vulkan driver starts the passes' feedback again after the first frame
        // while `FrameCount` runs on, so the frames the screenshot carries are counted from 1,
        // where the renderer's ticks count from 0.
        if counting.first != 0 && counts_frames(case) {
            eprintln!(
                "{}: skipped, {} counts the frames from {}",
                case.name,
                driver.name(),
                counting.first
            );
            return false;
        }
        true
    });
    parity::check(cases.collect(), &dir, |case, folder| {
        let (picture, frames) = &case.steps[0];
        frontend
            .render(
                &case.config,
                picture,
                *frames,
                parity::output(case),
                parity::aspect(case),
                folder,
            )
            .expect("RetroArch's screenshot")
    });
}

#[test]
#[ignore = "requires RetroArch before 1.20, named by CRTSIM_RETROARCH, and an X display"]
fn retroarch_on_glcore_draws_what_the_renderer_draws() {
    check(Driver::Glcore);
}

#[test]
#[ignore = "requires RetroArch before 1.20, named by CRTSIM_RETROARCH, Vulkan and an X display"]
fn retroarch_on_vulkan_draws_what_the_renderer_draws() {
    check(Driver::Vulkan);
}
