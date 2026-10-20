//! The RetroArch port against the renderer it was ported from: each case exported as a
//! preset, run through librashader, and compared with the renderer's own picture. See
//! `crtsim_ports::parity` for the cases and their limits.
//!
//! ```text
//! cargo test -p crtsim-ports --test retroarch_parity -- --ignored --nocapture
//! ```

use crtsim_ports::parity::{self, Case, Timing};
use crtsim_ports::retroarch::{Port, RetroArch};
use crtsim_shader_ports as ports;
use image::RgbaImage;
use std::path::Path;

fn port(case: &Case, folder: &Path) -> RgbaImage {
    let files = ports::retroarch::export(&[("look", &case.config)]).expect("export");
    ports::write(folder, &files).expect("write the preset");
    // Older RetroArch decays glow by a tick a frame, which is exactly the renderer's 60 a
    // second; newer by how long RetroArch says each frame lasts.
    let (retroarch, frame_time_us) = match case.timing {
        Timing::Ticks => (RetroArch::Older, 0),
        Timing::Video(fps) => (RetroArch::Newer, (1e6 / fps).round() as u32),
    };
    let mut port = Port::load(&folder.join("look.slangp"), retroarch).expect("load");
    // Older RetroArch has no `OriginalAspect`, so the preset's own aspect stands in for it.
    if retroarch == RetroArch::Older {
        port.set("CRTSIM_ASPECT", parity::aspect(case))
            .expect("the aspect parameter");
    }
    let mut picture = None;
    for (input, frames) in &case.steps {
        picture = Some(
            port.render(
                input,
                parity::output(case),
                *frames,
                frame_time_us,
                parity::aspect(case),
            )
            .expect("port render"),
        );
    }
    picture.expect("a step")
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn the_port_draws_what_the_renderer_draws() {
    parity::check(parity::cases(), &parity::output_dir("retroarch"), port);
}
