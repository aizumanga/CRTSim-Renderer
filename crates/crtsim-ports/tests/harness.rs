//! Proves the harness runs a preset as RetroArch would, before any port's test relies on it.
//!
//! ```text
//! cargo test -p crtsim-ports -- --ignored --nocapture
//! ```
//!
//! With `CRTSIM_SLANG_SHADERS` set to a checkout of libretro's slang-shaders, it also renders
//! their port of the original CRTSim, and with `CRTSIM_TEST_OUTPUT` set it saves that render.

use crtsim_ports::{Port, RetroArch};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};
use std::time::Instant;

const SIZE: (u32, u32) = (8, 4);
/// 0.8 / 4 and 40 ms / 100 ms are whole 8-bit steps: 51 and 102.
const ASPECT: f32 = 0.8;
const FRAME_TIME_US: u32 = 40_000;
const NEWER: Option<[u8; 2]> = Some([51, 102]);

fn feedback() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/feedback.slangp")
}

/// Every texel's red differs, so a flip or a shift shows.
fn input() -> RgbaImage {
    RgbaImage::from_fn(4, 2, |x, y| {
        Rgba([(10 + 40 * x + 100 * y) as u8, 7, 7, 255])
    })
}

/// The fixture's output of `input` scaled 2x: red inverted, green as accumulated, the frame
/// count in blue, and opaque though the fixture writes no alpha. With newer RetroArch's
/// uniforms, blue in the bottom half is `OriginalAspect` on the left and `FrameTimeDelta` on
/// the right.
fn expected(green: u8, frame: u8, uniforms: Option<[u8; 2]>) -> RgbaImage {
    let input = input();
    RgbaImage::from_fn(SIZE.0, SIZE.1, |x, y| {
        let red = 255 - input.get_pixel(x / 2, y / 2)[0];
        let blue = match uniforms {
            Some(uniforms) if y >= SIZE.1 / 2 => uniforms[usize::from(x >= SIZE.0 / 2)],
            _ => frame,
        };
        Rgba([red, green, blue, 255])
    })
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn feedback_frame_count_and_newer_uniforms_are_exact() {
    let mut port = Port::load(&feedback(), RetroArch::Newer).unwrap();
    eprintln!("adapter: {}", port.adapter().name);
    let input = input();
    for frame in 0..4u8 {
        let got = port.render(&input, SIZE, 1, FRAME_TIME_US, ASPECT).unwrap();
        assert_eq!(
            got,
            expected(16 * (frame + 1), frame, NEWER),
            "frame {frame}"
        );
    }
    // Several frames in one call carry on the sequence and return the last.
    let got = port.render(&input, SIZE, 3, FRAME_TIME_US, ASPECT).unwrap();
    assert_eq!(got, expected(16 * 7, 6, NEWER));
    port.set("STEP", 64.0).unwrap();
    let got = port.render(&input, SIZE, 1, FRAME_TIME_US, ASPECT).unwrap();
    assert_eq!(got, expected(16 * 7 + 64, 7, NEWER));
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn older_retroarch_leaves_the_newer_uniforms_undefined() {
    let mut port = Port::load(&feedback(), RetroArch::Older).unwrap();
    let got = port
        .render(&input(), SIZE, 1, FRAME_TIME_US, ASPECT)
        .unwrap();
    assert_eq!(got, expected(16, 0, None));
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn parameters_the_preset_does_not_declare_are_refused() {
    let mut port = Port::load(&feedback(), RetroArch::Newer).unwrap();
    let error = port.set("STPE", 1.0).unwrap_err().to_string();
    assert!(error.contains("STEP"), "{error}");
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn libretro_crtsim_port_renders() {
    let Some(shaders) = std::env::var_os("CRTSIM_SLANG_SHADERS") else {
        eprintln!("skipped: set CRTSIM_SLANG_SHADERS to a checkout of libretro/slang-shaders");
        return;
    };
    let preset = Path::new(&shaders).join("crt/crtsim.slangp");
    let start = Instant::now();
    let mut port = Port::load(&preset, RetroArch::Newer).unwrap();
    let loaded = start.elapsed();
    let start = Instant::now();
    let got = port
        .render(
            &crtsim_core::config::test_card(),
            (640, 480),
            20,
            16_667,
            4.0 / 3.0,
        )
        .unwrap();
    eprintln!(
        "{}: loaded in {loaded:.2?}, 20 frames in {:.2?}",
        port.adapter().name,
        start.elapsed()
    );
    if let Some(dir) = std::env::var_os("CRTSIM_TEST_OUTPUT") {
        std::fs::create_dir_all(&dir).unwrap();
        got.save(Path::new(&dir).join("libretro-crtsim.png"))
            .unwrap();
    }
    let luma: Vec<u32> = got
        .pixels()
        .map(|p| (u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])) / 3)
        .collect();
    let mean = luma.iter().sum::<u32>() as f32 / luma.len() as f32;
    let (darkest, brightest) = (luma.iter().min().unwrap(), luma.iter().max().unwrap());
    eprintln!("mean {mean:.1}, range {darkest}..={brightest}");
    assert!(mean > 8.0, "the picture is black");
    assert!(brightest - darkest > 64, "the picture is flat");
}
