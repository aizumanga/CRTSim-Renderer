//! The looks every port is held to the renderer with, and the measure of how far a port's
//! picture is from the renderer's. Each case is a look the golden images already hold, run
//! through the port for as many frames as the renderer's still ticks, or a few frames of
//! changing pictures for the glow between them.
//!
//! The renderer is held to its goldens bit for bit; this only guards the translations. A port
//! cannot be exact everywhere: its colour tables are 8-bit strips, and its host builds the
//! mask's mipmaps its own way, rounding each texel its own way, which moves a third of a
//! filtered frame's pixels by a step or two. So every case samples the mask unfiltered, where
//! only a real translation error can move a pixel, and one case keeps the mipmaps under a limit
//! of its own. The limits were set from the first measurement and frozen; `--nocapture`
//! prints each case's margin.

use crtsim_core::config::{self, ColorMode, Config, Phase};
use crtsim_core::{nes_luts, Renderer, Sequence};
use image::RgbaImage;
use std::path::{Path, PathBuf};

/// The goldens' size.
pub const OUTPUT: (u32, u32) = (320, 180);
/// The largest channel difference allowed on any pixel, and the mean over the frame, in 8-bit
/// steps: with the mask unfiltered, and with its mipmaps.
pub const UNFILTERED: Limits = Limits {
    worst: 2,
    mean: 0.01,
};
pub const MIPMAPPED: Limits = Limits {
    worst: 4,
    mean: 0.3,
};
/// A colour table computed in floats, as the NES palette is, which the ports read as an 8-bit
/// strip.
pub const FLOAT_TABLE: Limits = Limits {
    worst: 3,
    mean: 0.08,
};

/// Pixels allowed past a case's worst: where the bezel meets the glass, both lie at nearly the
/// same depth, and the last digit GLSL and WGSL round differently can pick either.
const SEAM_PIXELS: usize = 4;

#[derive(Clone, Copy)]
pub struct Limits {
    pub worst: u8,
    pub mean: f64,
}

/// The reference look at the goldens' size, its mask unfiltered.
pub fn base() -> Config {
    Config {
        output: format!("{}x{}", OUTPUT.0, OUTPUT.1),
        mask_antialias: false,
        ..Config::default()
    }
}

#[derive(Clone)]
pub struct Case {
    pub name: &'static str,
    pub config: Config,
    /// What reaches the renderer: resized by its prepare step to the signal.
    pub source: RgbaImage,
    /// What a host would hand the port, in turn, and for how many frames: the source already
    /// at signal size, before any colour table, since the port applies the table itself.
    pub steps: Vec<(RgbaImage, u32)>,
    pub limits: Limits,
    /// How the renderer paces its frames, and how long the host says each one lasts.
    pub timing: Timing,
}

#[derive(Clone, Copy)]
pub enum Timing {
    /// A still's ticks, each a sixtieth of a second, as a host that does not say how long a
    /// frame lasts decays the glow.
    Ticks,
    /// A video at this many frames a second, from a host that says how long each lasts.
    Video(f64),
}

impl Timing {
    /// How long a frame lasts, in seconds.
    pub fn frame_seconds(self) -> f64 {
        match self {
            Self::Ticks => 1. / 60.,
            Self::Video(fps) => 1. / fps,
        }
    }
}

/// A still of `config` from the test card, handed over at the signal's size.
pub fn still(name: &'static str, config: Config) -> Case {
    let card = config::test_card();
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
        source: card,
        steps: vec![(signal, frames)],
        limits,
        timing: Timing::Ticks,
    }
}

/// The cases every port draws.
pub fn cases() -> Vec<Case> {
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
        // The artifact pattern upside down, as the game's Linux build draws it.
        still(
            "alternating-flipped",
            Config {
                phase: Phase::Alternating,
                ntsc_blending: 0.35,
                flip_artifacts: true,
                ..base()
            },
        ),
        // The bezel reflecting the picture as the screen shows it, as the game draws it.
        still(
            "reflection-as-screen",
            Config {
                reflection_as_screen: true,
                overscan: 1.25,
                reflection: 2.,
                ..base()
            },
        ),
        // A backdrop past the bezel's edges, at the sides of the 16:9 frame.
        still(
            "backdrop",
            Config {
                backdrop_color: [0.0625, 0.25, 0.5],
                ..base()
            },
        ),
        // Tubes of other shapes, as the goldens hold them.
        still(
            "widescreen-tube",
            Config {
                tube_aspect: 16. / 9.,
                output: "240x180".into(),
                fit: config::Fit::Contain,
                reflection: 2.,
                ..base()
            },
        ),
        still(
            "portrait-tube",
            Config {
                tube_aspect: 0.75,
                reflection: 2.,
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
pub fn render(renderer: &Renderer, case: &Case) -> RgbaImage {
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

/// The size a case's picture is drawn at: the goldens', unless its look says another.
pub fn output(case: &Case) -> (u32, u32) {
    case.config
        .output_size(case.source.dimensions())
        .expect("a case's output size")
}

/// The picture's shape as a host would report it: its signal widened by its pixel aspect.
pub fn aspect(case: &Case) -> f32 {
    let (width, height) = case.steps[0].0.dimensions();
    width as f32 / height as f32 * case.config.pixel_aspect
}

/// The worst channel difference once the `SEAM_PIXELS` worst pixels are set aside, the mean
/// over every pixel, and how many pixels differ by more than `worst`, all in 8-bit steps.
pub fn compare(a: &RgbaImage, b: &RgbaImage, worst: u8) -> (u8, f64, usize) {
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

/// Where a port's test leaves its work, and the pictures of any case that drifted.
pub fn output_dir(port: &str) -> PathBuf {
    let dir = match std::env::var_os("CRTSIM_TEST_OUTPUT") {
        Some(dir) => PathBuf::from(dir).join(format!("{port}-parity")),
        None => std::env::temp_dir().join(format!("crtsim-{port}-parity-{}", std::process::id())),
    };
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("output folder");
    dir
}

/// Draws each case through the renderer and through `port`, which draws it in a folder of its
/// own under `dir`, and fails naming every case past its limits, saving both pictures of each.
pub fn check(cases: Vec<Case>, dir: &Path, mut port: impl FnMut(&Case, &Path) -> RgbaImage) {
    let renderer =
        pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).expect("Vulkan renderer");
    let mut failures = vec![];
    for case in cases {
        let expected = render(&renderer, &case);
        let got = port(&case, &dir.join(case.name));
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
