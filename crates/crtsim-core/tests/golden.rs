//! Golden-image regression tests.
//!
//! The CI fixtures prove a render *finished*; these prove it still produces the same picture.
//! They are the safety net for changes to the shader, the CPU prepare step or the pass order,
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
        // The CRT picture and, separately, the signal feeding it: a regression in the CPU
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
        .render_video_frame(&white, &trailing, &mut sequence)
        .expect("first sequence frame");
    cases.push((
        "persistence-trail",
        renderer
            .render_video_frame(&black, &trailing, &mut sequence)
            .expect("second sequence frame"),
    ));

    cases
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
