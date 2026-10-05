//! The RetroArch port against the renderer it was ported from: each case is a look the golden
//! images already hold, exported as a preset, run through librashader for as many frames as
//! the renderer's still ticks, and compared with the renderer's own picture.
//!
//! ```text
//! cargo test -p crtsim-ports --test renderer_parity -- --ignored --nocapture
//! ```
//!
//! The renderer is held to its goldens bit for bit; this only guards the translation. The port
//! cannot be exact everywhere: its colour tables are 8-bit strips, and RetroArch builds the
//! mask's mipmaps itself, rounding each texel its own way, which moves a third of a filtered
//! frame's pixels by a step or two. So every case samples the mask unfiltered, where only a
//! real translation error can move a pixel, and one case keeps the mipmaps under a limit of its
//! own. The limits were set from the first measurement and frozen; `--nocapture` prints each
//! case's margin.

use crtsim_core::config::{self, ColorMode, Config, Phase};
use crtsim_core::{nes_luts, retroarch, Renderer, Sequence};
use crtsim_ports::{Port, RetroArch};
use image::RgbaImage;
use std::path::PathBuf;

/// The goldens' size.
const OUTPUT: (u32, u32) = (320, 180);
/// The largest channel difference allowed on any pixel, and the mean over the frame, in 8-bit
/// steps: with the mask unfiltered, and with its mipmaps.
const UNFILTERED: Limits = Limits {
    worst: 2,
    mean: 0.01,
};
const MIPMAPPED: Limits = Limits {
    worst: 4,
    mean: 0.3,
};
/// A colour table computed in floats, as the NES palette is, which the port reads as an 8-bit
/// strip.
const FLOAT_TABLE: Limits = Limits {
    worst: 3,
    mean: 0.08,
};

/// Pixels allowed past a case's worst: where the bezel meets the glass, both lie at nearly the
/// same depth, and the last digit GLSL and WGSL round differently can pick either.
const SEAM_PIXELS: usize = 4;

#[derive(Clone, Copy)]
struct Limits {
    worst: u8,
    mean: f64,
}

fn base() -> Config {
    Config {
        output: format!("{}x{}", OUTPUT.0, OUTPUT.1),
        mask_antialias: false,
        ..Config::default()
    }
}

struct Case {
    name: &'static str,
    config: Config,
    /// What reaches the renderer: resized by its prepare step to the signal.
    source: RgbaImage,
    /// What a core would hand RetroArch, in turn, and for how many frames: the source already
    /// at signal size, before any colour table, since the port applies the table itself.
    steps: Vec<(RgbaImage, u32)>,
    limits: Limits,
    /// How the renderer paces its frames, and how long RetroArch says each one lasts. Newer
    /// RetroArch decays glow by that time; older by a tick a frame, which is exactly the
    /// renderer's 60 a second.
    timing: Timing,
}

#[derive(Clone, Copy)]
enum Timing {
    /// A still's ticks, on RetroArch without frame times.
    Ticks,
    /// A video at this many frames a second, on RetroArch that reports each frame's length.
    Video(f64),
}

fn cases() -> Vec<Case> {
    let card = config::test_card();
    let still = |name, config: Config| {
        let signal = config::prepare(
            &card,
            &Config {
                lut: None,
                palette: None,
                hue: 0.,
                chroma: 1.,
                ..config.clone()
            },
        )
        .expect("the signal");
        let frames = config.warmup + 1;
        let limits = if config.mask_antialias {
            MIPMAPPED
        } else if config.palette.is_some() {
            FLOAT_TABLE
        } else {
            UNFILTERED
        };
        Case {
            name,
            config,
            source: card.clone(),
            steps: vec![(signal, frames)],
            limits,
            timing: Timing::Ticks,
        }
    };
    let mut cases = vec![
        still("reference", base()),
        still(
            "phase-a",
            Config {
                phase: Phase::A,
                ..base()
            },
        ),
        still(
            "alternating-blended",
            Config {
                phase: Phase::Alternating,
                ntsc_blending: 0.35,
                ..base()
            },
        ),
        still(
            "linear-light",
            Config {
                color_mode: ColorMode::LinearLight,
                ..base()
            },
        ),
        still(
            "screen-only",
            Config {
                screen_only: true,
                ..base()
            },
        ),
        still(
            "mipmapped-mask",
            Config {
                mask_antialias: true,
                ..base()
            },
        ),
        still(
            "interlaced",
            Config {
                signal: "480p".into(),
                interlace: true,
                ..base()
            },
        ),
        still(
            "nes-lut",
            Config {
                lut: Some(std::sync::Arc::new(nes_luts::load(0).expect("bundled LUT"))),
                ..base()
            },
        ),
        // The game's palette, read nearest in red and green as the game reads it.
        still(
            "nes-palette-graded",
            Config {
                palette: Some(Default::default()),
                hue: 20.,
                chroma: 1.2,
                ..base()
            },
        ),
        // The signal's, a table in floats read blended in all three.
        still(
            "nes-palette-signal",
            Config {
                palette: Some(crtsim_core::palette::NesPalette {
                    model: crtsim_core::palette::Model::Signal,
                    ..Default::default()
                }),
                ..base()
            },
        ),
        still(
            "general",
            Config {
                signal: "native".into(),
                ..Config {
                    output: base().output,
                    mask_antialias: false,
                    ..Config::general()
                }
            },
        ),
    ];
    // Frame-to-frame feedback: a frame of white, then one of black keeping its trail; then the
    // same at 30 frames a second, each frame keeping the glow for twice as long.
    let white = RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]));
    let black = RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]));
    let trail = |name, persistence, timing| Case {
        name,
        config: Config {
            signal: "64x64".into(),
            warmup: 0,
            persistence: [persistence; 3],
            ..base()
        },
        source: white.clone(),
        steps: vec![(white.clone(), 1), (black.clone(), 1)],
        limits: UNFILTERED,
        timing,
    };
    cases.push(trail("persistence-trail", 0.9, Timing::Ticks));
    // Not 0.9 here: 0.9 of white is 229.5 steps, where a frame of 33,333 microseconds, a
    // hair short of a thirtieth of a second, rounds the other way.
    cases.push(trail("persistence-trail-30fps", 0.8, Timing::Video(30.)));
    cases
}

/// The renderer's picture: a frame of the source, then one of each later step's input.
fn render(renderer: &Renderer, case: &Case) -> RgbaImage {
    pollster::block_on(async {
        let mut sequence = match case.timing {
            Timing::Ticks => Sequence::still(),
            Timing::Video(fps) => Sequence::video(crtsim_core::Timing::Stable, fps),
        };
        renderer
            .frame(&mut sequence, &case.source, &case.config, None, |_| {})
            .await?;
        for (input, _) in &case.steps[1..] {
            renderer
                .frame(&mut sequence, input, &case.config, None, |_| {})
                .await?;
        }
        renderer.read(&mut sequence).await
    })
    .expect("render")
}

/// The picture's shape as a core would report it: its signal widened by its pixel aspect.
fn aspect(case: &Case) -> f32 {
    let (width, height) = case.steps[0].0.dimensions();
    width as f32 / height as f32 * case.config.pixel_aspect
}

fn port(case: &Case, dir: &std::path::Path) -> RgbaImage {
    let files = retroarch::export(&[("look", &case.config)]).expect("export");
    let folder = dir.join(case.name);
    retroarch::write(&folder, &files).expect("write the preset");
    let (retroarch, frame_time_us) = match case.timing {
        Timing::Ticks => (RetroArch::Older, 0),
        Timing::Video(fps) => (RetroArch::Newer, (1e6 / fps).round() as u32),
    };
    let mut port = Port::load(&folder.join("look.slangp"), retroarch).expect("load");
    // Older RetroArch has no `OriginalAspect`, so the preset's own aspect stands in for it.
    if retroarch == RetroArch::Older {
        port.set("CRTSIM_ASPECT", aspect(case))
            .expect("the aspect parameter");
    }
    let mut picture = None;
    for (input, frames) in &case.steps {
        picture = Some(
            port.render(input, OUTPUT, *frames, frame_time_us, aspect(case))
                .expect("port render"),
        );
    }
    picture.expect("a step")
}

/// The worst channel difference and the mean, in 8-bit steps.
/// The worst channel difference once the `SEAM_PIXELS` worst pixels are set aside, the mean
/// over every pixel, and how many pixels differ by more than `worst`, all in 8-bit steps.
fn compare(a: &RgbaImage, b: &RgbaImage, worst: u8) -> (u8, f64, usize) {
    assert_eq!(a.dimensions(), b.dimensions());
    let mut differences: Vec<u8> = a
        .pixels()
        .zip(b.pixels())
        .map(|(p, q)| (0..3).map(|c| p[c].abs_diff(q[c])).max().unwrap_or(0))
        .collect();
    let total: u64 = a
        .pixels()
        .zip(b.pixels())
        .flat_map(|(p, q)| (0..3).map(move |c| u64::from(p[c].abs_diff(q[c]))))
        .sum();
    let past = differences.iter().filter(|&&d| d > worst).count();
    differences.sort_unstable();
    let kept = differences.len().saturating_sub(SEAM_PIXELS + 1);
    (
        differences[kept],
        total as f64 / (a.pixels().len() * 3) as f64,
        past,
    )
}

fn output_dir() -> PathBuf {
    match std::env::var_os("CRTSIM_TEST_OUTPUT") {
        Some(dir) => PathBuf::from(dir).join("port-parity"),
        None => std::env::temp_dir().join(format!("crtsim-port-parity-{}", std::process::id())),
    }
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn the_port_draws_what_the_renderer_draws() {
    let renderer =
        pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).expect("Vulkan renderer");
    let dir = output_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("output folder");
    let mut failures = vec![];
    for case in cases() {
        let expected = render(&renderer, &case);
        let got = port(&case, &dir);
        let limits = case.limits;
        let (worst, mean, past) = compare(&expected, &got, limits.worst);
        eprintln!(
            "{}: worst channel {worst}/{}, mean {mean:.4}/{} steps, {past} pixels past the worst",
            case.name, limits.worst, limits.mean
        );
        if worst > limits.worst || mean > limits.mean {
            let _ = expected.save(dir.join(format!("{}-renderer.png", case.name)));
            let _ = got.save(dir.join(format!("{}-port.png", case.name)));
            failures.push(case.name);
        }
    }
    assert!(
        failures.is_empty(),
        "the port drifted from the renderer in {failures:?}; images in {}",
        dir.display()
    );
}
