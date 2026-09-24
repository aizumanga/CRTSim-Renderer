//! Golden-image regression tests.
//!
//! The CI fixtures prove a render *finished*; these prove it still produces the same picture.
//! They are the safety net for changes to the shader, the prepare step or the pass order,
//! where a wrong constant silently shifts every frame instead of failing.
//!
//! Run them against a software Vulkan driver, which is repeatable enough to compare:
//!
//! ```text
//! cargo test -p crtsim-core --test golden -- --ignored --nocapture
//! ```
//!
//! `--nocapture` prints each case's measured margin, so it is visible when a change is
//! creeping towards a limit rather than only once it trips one.
//!
//! After an intended rendering change, refresh the goldens and review the image diff:
//!
//! ```text
//! CRTSIM_UPDATE_GOLDEN=1 cargo test -p crtsim-core --test golden -- --ignored
//! ```

use crtsim_core::config::{self, ColorMode, Config, Phase};
use crtsim_core::{nes_luts, Renderer, Sequence};
use image::RgbaImage;
use std::path::{Path, PathBuf};

/// Small enough to keep the committed goldens small, large enough that the mask, the
/// scanlines and the bezel are all still resolved.
const OUTPUT: &str = "320x180";

/// The limits are deliberately near zero. One driver renders a given frame bit-exactly --
/// `gpu_smoke` already asserts that two renders agree -- so any real difference here is a
/// change in behavior, not noise. Loose perceptual limits were measured against a subtle
/// regression (luma weights rounded to 0.30/0.59/0.11) and missed it completely: that edit
/// moves the mean by 0.015-0.039 steps and no pixel by more than 6, which any tolerance
/// wide enough to absorb a driver change would swallow. Hence bit-exactness against a
/// recorded driver instead, with only enough slack for a stray pixel.
///
/// Largest channel difference allowed on any single pixel, in 0-255 steps.
const WORST_STEP: u8 = 1;
/// Mean absolute channel difference allowed over the whole frame, in 0-255 steps. At 320x180
/// this is roughly 460 single-step pixels before it trips.
const MEAN_STEP: f64 = 0.002;
/// Reported, not enforced: the share of pixels past this many steps says whether a failure is
/// a localized break or a shift spread across the frame.
const OUTLIER_STEP: u8 = 8;

struct Difference {
    mean: f64,
    outlier_share: f64,
    worst: u8,
}

impl Difference {
    fn acceptable(&self) -> bool {
        self.worst <= WORST_STEP && self.mean <= MEAN_STEP
    }
    fn summary(&self) -> String {
        format!(
            "worst channel {}/{WORST_STEP}, mean {:.4}/{MEAN_STEP} steps, {:.4}% of pixels past {OUTLIER_STEP} steps",
            self.worst,
            self.mean,
            self.outlier_share * 100.
        )
    }
}

fn compare(golden: &RgbaImage, actual: &RgbaImage) -> Difference {
    let mut total = 0_u64;
    let mut outliers = 0_u64;
    let mut worst = 0_u8;
    for (a, b) in golden.pixels().zip(actual.pixels()) {
        let mut pixel_worst = 0_u8;
        for channel in 0..4 {
            let delta = a[channel].abs_diff(b[channel]);
            total += u64::from(delta);
            pixel_worst = pixel_worst.max(delta);
        }
        worst = worst.max(pixel_worst);
        if pixel_worst > OUTLIER_STEP {
            outliers += 1;
        }
    }
    let pixels = golden.pixels().len() as f64;
    Difference {
        mean: total as f64 / (pixels * 4.),
        outlier_share: outliers as f64 / pixels,
        worst,
    }
}

/// Amplified per-pixel difference, so a failure's artifact shows *where* the picture moved
/// instead of only by how much. The differences worth seeing here are a handful of steps, so
/// the gain is steep enough to make one step visible and to saturate by four.
fn difference_image(golden: &RgbaImage, actual: &RgbaImage) -> RgbaImage {
    let mut out = RgbaImage::new(golden.width(), golden.height());
    for (out, (a, b)) in out.pixels_mut().zip(golden.pixels().zip(actual.pixels())) {
        let amplified = |channel: usize| a[channel].abs_diff(b[channel]).saturating_mul(64);
        *out = image::Rgba([amplified(0), amplified(1), amplified(2), 255]);
    }
    out
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// The goldens are only bit-comparable against the driver that produced them, so that driver
/// is recorded beside them. A mismatch is reported as itself rather than as a pile of pixel
/// differences, because the fix is to regenerate and review, not to widen a tolerance.
///
/// Only the driver version identifies it. `adapter.name` is deliberately left out: llvmpipe
/// puts its vector width in there ("llvmpipe (LLVM 20.1.2, 256 bits)"), which follows the host
/// CPU's SIMD support and would report a mismatch between two runners rendering identically.
/// The full adapter is printed by the test for diagnosis.
fn driver_id(adapter: &wgpu::AdapterInfo) -> String {
    adapter.driver_info.clone()
}

/// Where to drop the rendered and diff images for a failing case. CI points this at the
/// directory it uploads, so a red run carries the evidence with it.
fn report_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("CRTSIM_TEST_OUTPUT")?).join("golden-failures");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn base() -> Config {
    Config {
        output: OUTPUT.into(),
        ..Config::default()
    }
}

/// One rendered image per distinct path through the renderer. Anything that reaches the GPU by
/// its own route — a color mode, the composite phases, the LUT, the frame-to-frame feedback —
/// earns a case, because a change there is invisible in the others.
fn cases(renderer: &Renderer) -> Vec<(&'static str, RgbaImage)> {
    let source = config::test_card();
    let render = |c: &Config| renderer.render(&source, c).expect("render");

    let reference = render(&base());
    let mut cases = vec![
        // The CRT picture and, separately, the signal feeding it: a regression in the
        // prepare step or the composite pass shows up in the signal before the CRT passes
        // have a chance to hide it.
        ("reference", reference.crt),
        ("reference-signal", reference.signal),
    ];

    for (name, phase) in [("phase-a", Phase::A), ("phase-b", Phase::B)] {
        cases.push((name, render(&Config { phase, ..base() }).crt));
    }

    // The only non-default color path. `ColorMode::Reference` needs no case of its own: it is
    // what `reference` above already renders.
    cases.push((
        "linear-light",
        render(&Config {
            color_mode: ColorMode::LinearLight,
            ..base()
        })
        .crt,
    ));

    // Mask sampling and the bezel are each a branch of their own in the shader and the pass
    // list, and neither shows up in a frame rendered with the defaults.
    cases.push((
        "antialiased-mask",
        render(&Config {
            mask_antialias: true,
            ..base()
        })
        .crt,
    ));
    cases.push((
        "screen-only",
        render(&Config {
            screen_only: true,
            ..base()
        })
        .crt,
    ));

    // Interlaced scanning: a 480-row signal whose last tick scanned one field, the other left
    // to decay, as a still of an interlaced set shows it.
    cases.push((
        "interlaced",
        render(&Config {
            signal: "480p".into(),
            interlace: true,
            ..base()
        })
        .crt,
    ));

    let lut = nes_luts::load(0).expect("bundled LUT");
    cases.push((
        "nes-lut",
        render(&Config {
            lut: Some(std::sync::Arc::new(lut)),
            ..base()
        })
        .crt,
    ));

    // Frame-to-frame feedback: the second frame of a sequence keeps a trail of the first, so
    // this is the only case that can catch the history targets being reset or swapped wrongly.
    let trailing = Config {
        signal: "64x64".into(),
        warmup: 0,
        persistence: [0.9; 3],
        ..base()
    };
    let mut sequence = Sequence::default();
    let white = RgbaImage::from_pixel(64, 64, image::Rgba([255; 4]));
    let black = RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 255]));
    renderer
        .render_frame(&white, &trailing, &mut sequence, None, |_| {})
        .expect("first sequence frame");
    cases.push((
        "persistence-trail",
        renderer
            .render_frame(&black, &trailing, &mut sequence, None, |_| {})
            .expect("second sequence frame"),
    ));

    cases.extend(prepare_cases(renderer));
    cases
}

/// A source that makes a resize visible: smooth gradients, hard edges at odd angles and
/// detail finer than any signal it is scaled to, where the test card is already signal-sized
/// and only ever reaches prepare as an identity.
fn detailed_source() -> RgbaImage {
    RgbaImage::from_fn(640, 480, |x, y| {
        let ring = ((x as i32 - 320).pow(2) + (y as i32 - 240).pow(2)) / 96;
        let stripes = if (x + 2 * y) % 7 < 3 { 230 } else { 20 };
        let (r, g, b) = if y < 160 {
            ((x * 255 / 639) as u8, (y * 255 / 159) as u8, 128)
        } else if y < 320 {
            (stripes, (ring % 256) as u8, 255 - stripes)
        } else {
            let check = if (x / 3 + y / 3) % 2 == 0 { 250 } else { 5 };
            (check, 255 - (x * 200 / 639) as u8, check / 2)
        };
        image::Rgba([r, g, b, 255])
    })
}

/// The same source with a transparent hole and a translucent band, for the alpha composite.
fn translucent_source() -> RgbaImage {
    let mut image = detailed_source();
    for (x, y, p) in image.enumerate_pixels_mut() {
        p[3] = if (200..440).contains(&x) && (140..340).contains(&y) {
            0
        } else if (40..120).contains(&y) {
            (x * 255 / 639) as u8
        } else {
            255
        };
    }
    image
}

/// The prepare step feeds every frame, but the cases above reach it only as an identity: the
/// test card is already 256x224, opaque and ungraded. These capture its output directly -- the
/// `clean` image, before the simulation can soften a difference -- once per route through it:
/// each resize filter both ways, the alpha composite, the source edits with each sampler, and
/// the grade with a LUT so the order of the two is pinned too.
fn prepare_cases(renderer: &Renderer) -> Vec<(&'static str, RgbaImage)> {
    let detailed = detailed_source();
    let translucent = translucent_source();
    let card = config::test_card();
    let edit = crtsim_core::workflow::SourceEdit {
        crop: [0.05, 0.1, 0.15, 0.02],
        rotation: 7.5,
        zoom: 1.3,
        position: [0.06, -0.04],
        background: [30, 60, 90],
        checkerboard: false,
    };
    let lanczos = |signal: &str| Config {
        signal: signal.into(),
        filter: config::Filter::Lanczos,
        ..base()
    };
    let nearest = |signal: &str| Config {
        signal: signal.into(),
        filter: config::Filter::Nearest,
        ..base()
    };
    let lut = std::sync::Arc::new(nes_luts::load(3).expect("bundled LUT"));
    let routes: Vec<(&'static str, &RgbaImage, Config)> = vec![
        ("prepare-lanczos-down", &detailed, lanczos("200x150")),
        ("prepare-lanczos-up", &card, lanczos("360p")),
        ("prepare-lanczos-one-axis", &detailed, lanczos("640x224")),
        ("prepare-nearest-down", &detailed, nearest("original")),
        ("prepare-nearest-up", &card, nearest("480x400")),
        (
            "prepare-alpha",
            &translucent,
            Config {
                source: crtsim_core::workflow::SourceEdit {
                    background: [200, 40, 120],
                    ..Default::default()
                },
                ..lanczos("200x150")
            },
        ),
        (
            "prepare-edit-bilinear",
            &translucent,
            Config {
                source: edit.clone(),
                ..lanczos("240x180")
            },
        ),
        (
            "prepare-edit-nearest",
            &translucent,
            Config {
                source: crtsim_core::workflow::SourceEdit {
                    checkerboard: true,
                    ..edit.clone()
                },
                ..nearest("240x180")
            },
        ),
        (
            "prepare-grade",
            &detailed,
            Config {
                hue: 33.,
                chroma: 1.6,
                ..lanczos("200x150")
            },
        ),
        (
            "prepare-lut-grade",
            &detailed,
            Config {
                lut: Some(lut),
                hue: -20.,
                chroma: 0.7,
                ..lanczos("200x150")
            },
        ),
    ];
    routes
        .into_iter()
        .map(|(name, source, c)| (name, renderer.render(source, &c).expect("render").clean))
        .collect()
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn rendering_matches_the_committed_golden_images() {
    let renderer =
        pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).expect("Vulkan renderer");
    println!("golden images rendered by {:?}", renderer.adapter);
    let directory = golden_dir();
    let driver_path = directory.join("driver.txt");
    let driver = driver_id(&renderer.adapter);
    let updating = std::env::var_os("CRTSIM_UPDATE_GOLDEN").is_some();
    if updating {
        std::fs::create_dir_all(&directory).expect("golden directory");
        std::fs::write(&driver_path, format!("{driver}\n")).expect("write driver record");
        if let Some(report) = report_dir() {
            std::fs::write(report.join("driver.txt"), format!("{driver}\n"))
                .expect("write driver record copy");
        }
    } else {
        let recorded = std::fs::read_to_string(&driver_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", driver_path.display()));
        assert_eq!(
            recorded.trim(),
            driver,
            "the goldens were rendered by a different driver, so they are not comparable \
             bit-for-bit. Either run with the recorded driver (CI installs Mesa's lavapipe \
             via mesa-vulkan-drivers), or, if this driver is the new baseline, regenerate \
             with CRTSIM_UPDATE_GOLDEN=1 and review the image diff in the commit."
        );
    }

    let mut failures = vec![];
    for (name, actual) in cases(&renderer) {
        let path = directory.join(format!("{name}.png"));
        if updating {
            actual.save(&path).expect("write golden");
            println!("{name}: updated {}", path.display());
            // Also leave a copy where CI uploads from. If the goldens have to be regenerated
            // against a driver only CI has, they cannot be produced locally at all, so the
            // update run has to hand them back as a downloadable artifact to commit.
            if let Some(report) = report_dir() {
                actual
                    .save(report.join(format!("{name}.png")))
                    .expect("write golden copy");
            }
            continue;
        }
        let golden = match image::open(&path) {
            Ok(image) => image.to_rgba8(),
            Err(e) => {
                failures.push(format!(
                    "{name}: cannot read {}: {e}. Generate it with CRTSIM_UPDATE_GOLDEN=1.",
                    path.display()
                ));
                continue;
            }
        };
        if golden.dimensions() != actual.dimensions() {
            failures.push(format!(
                "{name}: {:?} does not match the golden's {:?}",
                actual.dimensions(),
                golden.dimensions()
            ));
            continue;
        }
        let difference = compare(&golden, &actual);
        println!("{name}: {}", difference.summary());
        if difference.acceptable() {
            continue;
        }
        let mut message = format!("{name}: {}", difference.summary());
        if let Some(report) = report_dir() {
            let actual_path = report.join(format!("{name}-actual.png"));
            let diff_path = report.join(format!("{name}-diff.png"));
            actual.save(&actual_path).expect("write actual");
            difference_image(&golden, &actual)
                .save(&diff_path)
                .expect("write diff");
            message += &format!(
                "; wrote {} and {}",
                actual_path.display(),
                diff_path.display()
            );
        }
        failures.push(message);
    }

    assert!(
        failures.is_empty(),
        "rendering changed in {} case(s):\n  {}\n\nIf the change was intended, refresh the \
         goldens with CRTSIM_UPDATE_GOLDEN=1 and review the image diff in the commit.",
        failures.len(),
        failures.join("\n  ")
    );
}
