use super::*;
#[test]
fn wgsl_validates_without_a_gpu() {
    let module = naga::front::wgsl::parse_str(SHADER).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn video_history_survives_between_frames_and_resets_for_new_sequence() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let c = Config {
        output: "160x120".into(),
        signal: "32x32".into(),
        warmup: 0,
        persistence: [0.9; 3],
        ..Config::default()
    };
    let white = RgbaImage::from_pixel(32, 32, image::Rgba([255; 4]));
    let black = RgbaImage::from_pixel(32, 32, image::Rgba([0, 0, 0, 255]));
    let mut sequence = Sequence::default();
    r.render_frame(&white, &c, &mut sequence, None, |_| {})
        .unwrap();
    let trailing = r
        .render_frame(&black, &c, &mut sequence, None, |_| {})
        .unwrap();
    let reset = r
        .render_frame(&black, &c, &mut Sequence::default(), None, |_| {})
        .unwrap();
    assert_ne!(trailing, reset, "history was reset between video frames");
    assert_eq!(reset, r.render(&black, &c).unwrap().crt);
}
#[test]
#[ignore = "requires a Vulkan adapter"]
fn preview_frame_matches_the_pixels_a_read_back_render_produces() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let c = Config {
        output: "320x180".into(),
        ..Config::default()
    };
    let source = config::test_card();
    let preview = r.render_preview(&source, &c).unwrap();
    assert_eq!((preview.width, preview.height), (320, 180));
    // Read the frame back the same way an export would, to compare like with like. The
    // copy into the preview texture keeps the bytes and only relabels them as sRGB, which
    // is the conversion egui would otherwise apply when it uploads the pixels itself.
    let view = preview.texture.create_view(&Default::default());
    let target = Target {
        texture: preview.texture,
        view,
        width: preview.width,
        height: preview.height,
    };
    assert_eq!(
        r.readback(&target).unwrap(),
        r.render(&source, &c).unwrap().crt
    );
}

#[test]
fn ntsc_blending_moves_the_two_phases_towards_each_other() {
    let artifact_mix = |phase, ntsc_blending, tick| {
        let c = Config {
            phase,
            ntsc_blending,
            ..Config::default()
        };
        let mut params = Params::new(&c, (256, 224), (320, 240));
        params.tick(c.phase, c.ntsc_blending, tick);
        params.signal[3]
    };
    // Unblended, the public source's switch between the two patterns.
    assert_eq!(artifact_mix(Phase::A, 0., 0), 0.);
    assert_eq!(artifact_mix(Phase::B, 0., 0), 1.);
    assert_eq!(artifact_mix(Phase::Alternating, 0., 7), 1.);
    // Super Win the Game's 0.35: each tick mixes in some of the other pattern.
    assert_eq!(artifact_mix(Phase::A, 0.35, 0), 0.35);
    assert_eq!(artifact_mix(Phase::B, 0.35, 0), 0.65);
    assert_eq!(artifact_mix(Phase::Alternating, 0.35, 4), 0.35);
    assert_eq!(artifact_mix(Phase::Alternating, 0.35, 5), 0.65);
    // Half-way, every tick shows the average, as stable phase always does.
    assert_eq!(artifact_mix(Phase::Alternating, 0.5, 1), 0.5);
    assert_eq!(artifact_mix(Phase::Stable, 0.35, 0), 0.5);
}

/// The GPU prepare step against the CPU one it replaced, across a spread of routes and
/// settings wider than the goldens pin. Not tied to a recorded driver like the goldens: the
/// shader repeats the CPU's arithmetic, so on any driver the two differ only where float
/// rounding lands a value on the other side of a half step.
#[test]
#[ignore = "requires a Vulkan adapter"]
fn gpu_prepare_matches_cpu_prepare() {
    let mut r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let source = RgbaImage::from_fn(301, 187, |x, y| {
        let v = (x * 37 + y * 101 + x * y * 13) % 256;
        let a = if (x / 40 + y / 30) % 3 == 0 {
            (x + y) % 256
        } else {
            255
        };
        image::Rgba([v as u8, (v * 7 % 256) as u8, (255 - v) as u8, a as u8])
    });
    let lut = Arc::new(nes_luts::load(7).unwrap());
    let mut state = 0x2545_f491_u32;
    let mut next = |range: f32| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state as f32 / u32::MAX as f32 * 2. - 1.) * range
    };
    let mut worst = 0;
    let mut differing = 0;
    let mut total = 0;
    for case in 0..48 {
        let signal = ["original", "native", "240p", "480p", "97x301", "600x100"][case % 6];
        let mut c = Config {
            signal: signal.into(),
            output: "64x48".into(),
            warmup: 0,
            filter: if case % 2 == 0 {
                config::Filter::Lanczos
            } else {
                config::Filter::Nearest
            },
            ..Config::default()
        };
        c.source.background = [(case * 40 % 256) as u8, 90, 200];
        if case % 3 == 0 {
            c.source.rotation = next(180.);
            c.source.zoom = 1. + next(0.6);
            c.source.position = [next(0.3), next(0.3)];
            c.source.crop = [next(0.2).abs(), next(0.2).abs(), 0.1, 0.];
            c.source.checkerboard = case % 4 == 0;
        }
        if case % 4 == 1 {
            c.lut = Some(lut.clone());
            if case % 8 == 5 {
                c.lut_strength = next(1.).abs();
            }
        }
        if case % 5 < 2 {
            c.hue = next(180.);
            c.chroma = 1. + next(1.);
        }
        r.prepare = PrepareOn::Cpu;
        let cpu = r.render(&source, &c).unwrap().clean;
        r.prepare = PrepareOn::Gpu;
        let gpu = r.render(&source, &c).unwrap().clean;
        assert_eq!(cpu.dimensions(), gpu.dimensions(), "case {case}");
        for (a, b) in cpu.pixels().zip(gpu.pixels()) {
            for channel in 0..4 {
                let delta = a[channel].abs_diff(b[channel]);
                worst = worst.max(delta);
                differing += usize::from(delta > 0);
                total += 1;
            }
        }
    }
    println!("{differing} of {total} channels differ, by at most {worst} step(s)");
    assert!(worst <= 1, "GPU prepare differs by {worst} steps");
}

/// Read in the signal, where rows are rows: a tick scans one field, and the other keeps only
/// what persistence leaves of its last scan.
#[test]
#[ignore = "requires a Vulkan adapter"]
fn interlaced_ticks_scan_alternate_fields() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let c = Config {
        output: "64x48".into(),
        signal: "32x16".into(),
        warmup: 0,
        persistence: [0.; 3],
        artifacts: 0.,
        sharpness: 0.,
        interlace: true,
        ..Config::default()
    };
    let white = RgbaImage::from_pixel(32, 16, image::Rgba([255; 4]));
    let rows = |c: &Config| -> Vec<u8> {
        let signal = r.render(&white, c).unwrap().signal;
        (0..16).map(|y| signal.get_pixel(16, y)[0]).collect()
    };
    // One tick scans the first field only.
    let first = rows(&c);
    assert!(first.iter().step_by(2).all(|&v| v == 255), "{first:?}");
    assert!(
        first.iter().skip(1).step_by(2).all(|&v| v == 0),
        "{first:?}"
    );
    // The second tick scans the other field; with no persistence the first goes dark.
    let second = rows(&Config {
        warmup: 1,
        ..c.clone()
    });
    assert!(second.iter().step_by(2).all(|&v| v == 0), "{second:?}");
    assert!(
        second.iter().skip(1).step_by(2).all(|&v| v == 255),
        "{second:?}"
    );
    // With persistence the unscanned field keeps a decayed copy instead.
    let decayed = rows(&Config {
        warmup: 1,
        persistence: [0.5; 3],
        ..c.clone()
    });
    assert!(
        decayed.iter().step_by(2).all(|&v| (120..=135).contains(&v)),
        "{decayed:?}"
    );
    assert!(
        decayed.iter().skip(1).step_by(2).all(|&v| v == 255),
        "{decayed:?}"
    );
    // Off, every tick scans every row.
    let progressive = rows(&Config {
        interlace: false,
        ..c
    });
    assert!(progressive.iter().all(|&v| v == 255), "{progressive:?}");
}

#[test]
#[ignore = "requires an explicitly provisioned GPU or software Vulkan driver"]
fn gpu_smoke() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::all())).unwrap();
    let mut c = Config {
        output: "641x361".into(),
        ..Config::default()
    };
    let source = config::test_card();
    let cancel = AtomicBool::new(true);
    assert!(r
        .render_frame(&source, &c, &mut Sequence::default(), Some(&cancel), |_| {})
        .err()
        .unwrap()
        .to_string()
        .contains("cancelled"));
    let a = r.render(&source, &c).unwrap();
    let b = r.render(&source, &c).unwrap();
    assert_eq!(a.crt.dimensions(), (641, 361));
    assert_eq!(a.crt, b.crt);
    assert!(a.crt.pixels().any(|p| p[0] > 180));
    assert!(a.crt.pixels().any(|p| p[0] < 20));
    c.phase = Phase::A;
    let a = r.render(&source, &c).unwrap();
    c.phase = Phase::B;
    let b = r.render(&source, &c).unwrap();
    assert_ne!(a.signal, b.signal);
    c.phase = Phase::Stable;
    c.artifacts = 0.;
    c.sharpness = 0.;
    c.persistence = [0.; 3];
    let a = r.render(&source, &c).unwrap();
    assert_eq!(a.clean, a.signal);
    c.color_mode = ColorMode::LinearLight;
    let mut progress = vec![];
    let linear = r
        .render_frame(&source, &c, &mut Sequence::default(), None, |p| {
            progress.push(p.fraction)
        })
        .unwrap();
    assert_eq!(linear.dimensions(), (641, 361));
    assert_ne!(linear, a.crt);
    assert_eq!(progress.first(), Some(&0.));
    assert_eq!(progress.last(), Some(&1.));
    assert!(progress.windows(2).all(|w| w[0] <= w[1]));
    c.color_mode = ColorMode::Reference;
    let filtered = r.render(&source, &c).unwrap();
    let unfiltered = Config {
        mask_antialias: false,
        ..c.clone()
    };
    assert_ne!(filtered.crt, r.render(&source, &unfiltered).unwrap().crt);
    if let Ok(dir) = std::env::var("CRTSIM_TEST_OUTPUT") {
        std::fs::create_dir_all(&dir).unwrap();
        linear
            .save(std::path::Path::new(&dir).join("linear-light.png"))
            .unwrap();
        filtered
            .crt
            .save(std::path::Path::new(&dir).join("filtered-mask.png"))
            .unwrap();
    }
}
