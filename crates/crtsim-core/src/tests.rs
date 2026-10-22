use super::*;
use config::Phase;

/// A still with the signals it was made from.
struct Rendered {
    clean: RgbaImage,
    signal: RgbaImage,
    crt: RgbaImage,
}

fn render(r: &Renderer, input: &RgbaImage, c: &Config) -> Result<Rendered> {
    pollster::block_on(async {
        let mut sequence = Sequence::still();
        r.frame(&mut sequence, input, c, None, |_| {}).await?;
        let crt = r.read(&mut sequence).await?;
        let Signals { clean, signal } = r.signals(&sequence).await?;
        Ok(Rendered { clean, signal, crt })
    })
}

/// `sequence`'s next frame of `input`, read back.
fn next(r: &Renderer, sequence: &mut Sequence, input: &RgbaImage, c: &Config) -> RgbaImage {
    pollster::block_on(async {
        r.frame(sequence, input, c, None, |_| {}).await?;
        r.read(sequence).await
    })
    .unwrap()
}
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
    let mut sequence = Sequence::still();
    next(&r, &mut sequence, &white, &c);
    let trailing = next(&r, &mut sequence, &black, &c);
    let reset = next(&r, &mut Sequence::still(), &black, &c);
    assert_ne!(trailing, reset, "history was reset between video frames");
    assert_eq!(reset, render(&r, &black, &c).unwrap().crt);
}
#[test]
#[ignore = "requires a Vulkan adapter"]
fn an_editing_sequence_follows_edits_on_its_history_and_restarts_for_a_new_signal() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let c = Config {
        output: "160x120".into(),
        signal: "32x32".into(),
        persistence: [0.9; 3],
        ..Config::default()
    };
    let card = config::test_card();
    let mut editing = Sequence::editing();
    assert_eq!(
        next(&r, &mut editing, &card, &c),
        render(&r, &card, &c).unwrap().crt,
        "the first frame is a still"
    );
    // Turning the colour down shows at once, over the glow of the colour before.
    let edited = Config {
        chroma: 0.,
        ..c.clone()
    };
    assert_ne!(
        next(&r, &mut editing, &card, &edited),
        render(&r, &card, &edited).unwrap().crt,
        "an edit runs on from the glow before it"
    );
    let resized = Config {
        output: "200x150".into(),
        ..edited.clone()
    };
    let at_new_size = next(&r, &mut editing, &card, &resized);
    assert_eq!(at_new_size.dimensions(), (200, 150));
    assert_ne!(
        at_new_size,
        render(&r, &card, &resized).unwrap().crt,
        "a new output size runs on from the glow before it"
    );
    let resampled = Config {
        signal: "64x64".into(),
        ..resized.clone()
    };
    assert_eq!(
        next(&r, &mut editing, &card, &resampled),
        render(&r, &card, &resampled).unwrap().crt,
        "a new signal size starts again from a still"
    );
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn a_video_keeps_its_glow_at_a_new_output_size_and_refuses_a_new_signal() {
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
    let resized = Config {
        output: "320x240".into(),
        ..c.clone()
    };
    let mut video = Sequence::video(Timing::Stable, 60.);
    next(&r, &mut video, &white, &c);
    let trailing = next(&r, &mut video, &black, &resized);
    assert_eq!(trailing.dimensions(), (320, 240));
    let mut fresh = Sequence::video(Timing::Stable, 60.);
    assert_ne!(
        trailing,
        next(&r, &mut fresh, &black, &resized),
        "the white still glows behind the black at the new size"
    );
    let resampled = Config {
        signal: "64x64".into(),
        ..resized
    };
    assert!(
        pollster::block_on(r.frame(&mut video, &black, &resampled, None, |_| {})).is_err(),
        "a video's sequence never restarts by itself"
    );
}
/// A host's texture of `format` and `size`, holding `image` when one is given.
fn host_texture(
    r: &Renderer,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    image: Option<&RgbaImage>,
) -> wgpu::Texture {
    let texture = r.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("host texture"),
        size: gpu::extent(width, height),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    if let Some(image) = image {
        let view = texture.create_view(&Default::default());
        Target {
            texture: texture.clone(),
            view,
            width,
            height,
        }
        .upload(&r.queue, image);
    }
    texture
}

/// A host's texture's bytes, read back as they are stored.
fn host_bytes(r: &Renderer, texture: &wgpu::Texture) -> RgbaImage {
    let target = Target {
        texture: texture.clone(),
        view: texture.create_view(&Default::default()),
        width: texture.width(),
        height: texture.height(),
    };
    pollster::block_on(r.read_target(&target)).unwrap()
}

/// `sequence`'s next frame recorded for a host, from `source` into `target`, and submitted.
/// The colours a float texture of linear light holds, encoded as sRGB bytes.
fn float_bytes(r: &Renderer, texture: &wgpu::Texture) -> RgbaImage {
    let (width, height) = (texture.width(), texture.height());
    let pitch = (width * 8).div_ceil(256) * 256;
    let buffer = r.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("float readback"),
        size: u64::from(pitch * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = r.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pitch),
                rows_per_image: None,
            },
        },
        gpu::extent(width, height),
    );
    r.queue.submit(Some(encoder.finish()));
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    r.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap();
    let encode = |v: f32| {
        let v = v.clamp(0., 1.);
        let v = if v <= 0.0031308 {
            v * 12.92
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round() as u8
    };
    RgbaImage::from_fn(width, height, |x, y| {
        let at = (y * pitch + x * 8) as usize;
        let texel: &[half::f16] = bytemuck::cast_slice(&bytes[at..at + 8]);
        image::Rgba([
            encode(texel[0].to_f32()),
            encode(texel[1].to_f32()),
            encode(texel[2].to_f32()),
            255,
        ])
    })
}
fn hosted(
    r: &Renderer,
    sequence: &mut Sequence,
    source: &wgpu::Texture,
    target: &wgpu::Texture,
    c: &Config,
) {
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let frame = HostFrame {
        source,
        target,
        seconds: 1. / 60.,
    };
    r.encode(&mut encoder, sequence, &frame, c).unwrap();
    r.queue.submit(Some(encoder.finish()));
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn a_host_frame_draws_what_a_video_frame_draws_and_keeps_its_glow_at_a_new_size() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    // A video's sequence with no warm-up, at 60 frames a second, runs as a host's does.
    let c = Config {
        output: "320x240".into(),
        warmup: 0,
        persistence: [0.9; 3],
        ..Config::general()
    };
    let card = config::test_card();
    let black = RgbaImage::from_pixel(256, 224, image::Rgba([0, 0, 0, 255]));
    let mut video = Sequence::video(Timing::Stable, 60.);
    next(&r, &mut video, &card, &c);
    let expected = next(&r, &mut video, &black, &c);

    use wgpu::TextureFormat::{Bgra8UnormSrgb, Rgba16Float, Rgba8Unorm, Rgba8UnormSrgb};
    let first = host_texture(&r, Rgba8Unorm, (256, 224), Some(&card));
    let second = host_texture(&r, Rgba8Unorm, (256, 224), Some(&black));
    let target = host_texture(&r, Rgba8Unorm, (320, 240), None);
    let mut host = Sequence::host();
    hosted(&r, &mut host, &first, &target, &c);
    hosted(&r, &mut host, &second, &target, &c);
    assert_eq!(host_bytes(&r, &target), expected);

    // sRGB textures hold the same bytes, which read and write as the colours they encode.
    let first = host_texture(&r, Rgba8UnormSrgb, (256, 224), Some(&card));
    let second = host_texture(&r, Rgba8UnormSrgb, (256, 224), Some(&black));
    let target = host_texture(&r, Bgra8UnormSrgb, (320, 240), None);
    let mut host = Sequence::host();
    hosted(&r, &mut host, &first, &target, &c);
    hosted(&r, &mut host, &second, &target, &c);
    let mut bgra = host_bytes(&r, &target);
    for pixel in bgra.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let most = bgra
        .pixels()
        .zip(expected.pixels())
        .flat_map(|(a, b)| (0..3).map(move |i| a[i].abs_diff(b[i])))
        .max();
    assert!(most <= Some(1), "sRGB textures differ by {most:?} steps");

    // Float textures hold linear light, as an engine's HDR target does: read and drawn as the
    // colours they encode, they give what sRGB textures give.
    let linear = |image: &RgbaImage| {
        let texture = host_texture(&r, Rgba16Float, image.dimensions(), None);
        let texels: Vec<half::f16> = image
            .pixels()
            .flat_map(|pixel| {
                let decode = |v: u8| {
                    let v = f32::from(v) / 255.;
                    if v <= 0.04045 {
                        v / 12.92
                    } else {
                        ((v + 0.055) / 1.055).powf(2.4)
                    }
                };
                [decode(pixel[0]), decode(pixel[1]), decode(pixel[2]), 1.]
            })
            .map(half::f16::from_f32)
            .collect();
        r.queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(8 * image.width()),
                rows_per_image: None,
            },
            gpu::extent(image.width(), image.height()),
        );
        texture
    };
    let first = linear(&card);
    let second = linear(&black);
    let target = host_texture(&r, Rgba16Float, (320, 240), None);
    let mut host = Sequence::host();
    hosted(&r, &mut host, &first, &target, &c);
    hosted(&r, &mut host, &second, &target, &c);
    let most = float_bytes(&r, &target)
        .pixels()
        .zip(expected.pixels())
        .flat_map(|(a, b)| (0..3).map(move |i| a[i].abs_diff(b[i])))
        .max();
    assert!(most <= Some(1), "float textures differ by {most:?} steps");

    // A new output size keeps the glow: the card still shows behind black at twice the size.
    let card = host_texture(&r, Rgba8Unorm, (256, 224), Some(&card));
    let black = host_texture(&r, Rgba8Unorm, (256, 224), Some(&black));
    let small = host_texture(&r, Rgba8Unorm, (320, 240), None);
    let large = host_texture(&r, Rgba8Unorm, (640, 480), None);
    let mut host = Sequence::host();
    hosted(&r, &mut host, &card, &small, &c);
    hosted(&r, &mut host, &black, &large, &c);
    assert_eq!(host.tick, 2, "the sequence went on");
    let alone = host_texture(&r, Rgba8Unorm, (640, 480), None);
    hosted(&r, &mut Sequence::host(), &black, &alone, &c);
    assert_ne!(host_bytes(&r, &large), host_bytes(&r, &alone));
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn a_host_frame_needs_a_host_sequence_and_usable_textures() {
    let r = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let texture = host_texture(&r, wgpu::TextureFormat::Rgba8Unorm, (16, 16), None);
    let mut encoder = r.device.create_command_encoder(&Default::default());
    let frame = HostFrame {
        source: &texture,
        target: &texture,
        seconds: 1. / 60.,
    };
    let c = Config::default();
    let error = r
        .encode(&mut encoder, &mut Sequence::still(), &frame, &c)
        .unwrap_err();
    assert!(error.to_string().contains("Sequence::host"), "{error}");
    let depth = r.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: gpu::extent(16, 16),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    for (source, target) in [(&depth, &texture), (&texture, &depth)] {
        let frame = HostFrame {
            source,
            target,
            seconds: 1. / 60.,
        };
        assert!(r
            .encode(&mut encoder, &mut Sequence::host(), &frame, &c)
            .is_err());
    }
    // A frame of no time, as an engine's first can be, holds the glow as it is.
    let source = host_texture(&r, wgpu::TextureFormat::Rgba8Unorm, (16, 16), None);
    let frame = HostFrame {
        source: &source,
        target: &texture,
        seconds: 0.,
    };
    r.encode(&mut encoder, &mut Sequence::host(), &frame, &c)
        .unwrap();
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
    let mut sequence = Sequence::still();
    pollster::block_on(r.frame(&mut sequence, &source, &c, None, |_| {})).unwrap();
    let preview = r.show(&sequence).unwrap();
    assert_eq!((preview.width, preview.height), (320, 180));
    // Read the frame back the same way an export would, to compare like with like. The
    // copy into the preview texture keeps the bytes as they are.
    let view = preview.texture.create_view(&Default::default());
    let target = Target {
        texture: preview.texture,
        view,
        width: preview.width,
        height: preview.height,
    };
    assert_eq!(
        pollster::block_on(r.read_target(&target)).unwrap(),
        render(&r, &source, &c).unwrap().crt
    );
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
        } else if case % 4 == 2 {
            // Both models: the game's 32-step table and the signal's 64-step one.
            c.palette = Some(crate::palette::NesPalette {
                tint: 5.18 + next(1.),
                model: if case % 8 == 2 {
                    crate::palette::Model::Game
                } else {
                    crate::palette::Model::Signal
                },
                ..Default::default()
            });
        }
        if case % 5 < 2 {
            c.hue = next(180.);
            c.chroma = 1. + next(1.);
        }
        r.prepare = PrepareOn::Cpu;
        let cpu = render(&r, &source, &c).unwrap().clean;
        r.prepare = PrepareOn::Gpu;
        let gpu = render(&r, &source, &c).unwrap().clean;
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
        let signal = render(&r, &white, c).unwrap().signal;
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
    assert!(
        pollster::block_on(r.still(&source, &c, Some(&cancel), |_| {}))
            .err()
            .unwrap()
            .to_string()
            .contains("cancelled")
    );
    let a = render(&r, &source, &c).unwrap();
    let b = render(&r, &source, &c).unwrap();
    assert_eq!(a.crt.dimensions(), (641, 361));
    assert_eq!(a.crt, b.crt);
    assert!(a.crt.pixels().any(|p| p[0] > 180));
    assert!(a.crt.pixels().any(|p| p[0] < 20));
    c.phase = Phase::A;
    let a = render(&r, &source, &c).unwrap();
    c.phase = Phase::B;
    let b = render(&r, &source, &c).unwrap();
    assert_ne!(a.signal, b.signal);
    c.phase = Phase::Stable;
    c.artifacts = 0.;
    c.sharpness = 0.;
    c.persistence = [0.; 3];
    let a = render(&r, &source, &c).unwrap();
    assert_eq!(a.clean, a.signal);
    c.color_mode = ColorMode::LinearLight;
    let mut progress = vec![];
    let linear = pollster::block_on(r.still(&source, &c, None, |p| progress.push(p))).unwrap();
    assert_eq!(linear.dimensions(), (641, 361));
    assert_ne!(linear, a.crt);
    assert_eq!(
        progress.first().map(|p| (p.fraction, p.stage)),
        Some((0., Stage::Preparing))
    );
    assert_eq!(
        progress.last().map(|p| (p.fraction, p.stage)),
        Some((1., Stage::Done))
    );
    assert!(progress.windows(2).all(|w| w[0].fraction <= w[1].fraction));
    let ticks = c.warmup + 1;
    assert!(progress.iter().any(|p| p.stage
        == Stage::Simulating {
            done: ticks,
            of: ticks
        }));
    c.color_mode = ColorMode::Reference;
    let filtered = render(&r, &source, &c).unwrap();
    let unfiltered = Config {
        mask_antialias: false,
        ..c.clone()
    };
    assert_ne!(filtered.crt, render(&r, &source, &unfiltered).unwrap().crt);
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
