use crtsim_core::config::Config;
use crtsim_media::{Audio, Options, Timing};
use std::{
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

fn fixture(path: &Path, rate: &str) {
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("testsrc2=size=64x48:rate={rate}"),
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "0.4",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn inspect(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-count_frames",
            "-show_streams",
            "-show_format",
            "-show_chapters",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
#[ignore = "requires FFmpeg and ffprobe"]
fn multiple_tracks_subtitles_chapters_metadata_and_lut_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let subs = dir.path().join("captions.srt");
    std::fs::write(&subs, "1\n00:00:00,000 --> 00:00:00,300\nHello CRT\n").unwrap();
    let meta = dir.path().join("source.ffmeta");
    std::fs::write(&meta,";FFMETADATA1\ntitle=My source\ncomment=Keep this comment\nartist=Example\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=300\ntitle=Opening\n").unwrap();
    let source = dir.path().join("multitrack.mkv");
    let output = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-i",
        ])
        .arg(&subs)
        .args(["-f", "ffmetadata", "-i"])
        .arg(&meta)
        .args([
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-map",
            "3:s",
            "-map_metadata",
            "4",
            "-map_chapters",
            "4",
            "-metadata:s:a:0",
            "language=eng",
            "-metadata:s:a:1",
            "language=jpn",
            "-metadata:s:s:0",
            "language=eng",
            "-t",
            "0.4",
            "-c:v",
            "libx264",
            "-c:a",
            "pcm_s16le",
            "-c:s",
            "srt",
            "-attach",
        ])
        .arg(&subs)
        .args(["-metadata:s:t:0", "mimetype=text/plain"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cancel = Arc::new(AtomicBool::new(false));
    let video = crtsim_media::probe(&source, &cancel).unwrap();
    let lut = crtsim_core::workflow::Lut::parse_cube(
        "Identity # = ; \\".into(),
        "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1",
    )
    .unwrap();
    let config = Config {
        output: "64x48".into(),
        lut: Some(Arc::new(lut)),
        ..Config::default()
    };
    let options = Options {
        crf: Some(21),
        speed: Some(crtsim_media::EncodingSpeed::Fast),
        ..Options::default()
    };
    for ext in ["mkv", "mp4", "webm"] {
        let destination = dir.path().join(format!("result.{ext}"));
        crtsim_media::export_with(
            &video,
            &destination,
            &config,
            &options,
            &cancel,
            |im, _| Ok(im.clone()),
            |_| {},
        )
        .unwrap();
        let data = inspect(&destination);
        let streams = data["streams"].as_array().unwrap();
        assert_eq!(
            streams
                .iter()
                .filter(|s| s["codec_type"] == "audio")
                .count(),
            2
        );
        assert_eq!(
            streams
                .iter()
                .filter(|s| s["codec_type"] == "subtitle")
                .count(),
            1
        );
        assert_eq!(
            streams
                .iter()
                .filter(|s| s["codec_type"] == "attachment")
                .count(),
            usize::from(ext == "mkv")
        );
        assert_eq!(data["chapters"].as_array().unwrap().len(), 1);
        let tags = data["format"]["tags"].as_object().unwrap();
        let tag = |name: &str| {
            tags.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str().unwrap())
                .unwrap_or("")
        };
        assert_eq!(tag("title"), "My source");
        assert_eq!(tag("source_comment"), "Keep this comment");
        let audio: Vec<_> = streams
            .iter()
            .filter(|s| s["codec_type"] == "audio")
            .collect();
        assert_eq!(audio[0]["tags"]["language"], "eng");
        assert_eq!(audio[1]["tags"]["language"], "jpn");
        assert_eq!(
            crtsim_media::import_preset(&destination, (64, 48), &cancel)
                .unwrap()
                .video_options,
            options
        );
        assert_eq!(
            crtsim_media::import_preset(&destination, (64, 48), &cancel)
                .unwrap()
                .config,
            config
        );
    }
}

#[test]
#[ignore = "requires FFmpeg and ffprobe with libx264, libvpx-vp9, AAC and Opus"]
fn ffmpeg_streaming_audio_timing_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source with spaces.mkv");
    fixture(&source, "30");
    let cancel = Arc::new(AtomicBool::new(false));
    let info = crtsim_media::probe(&source, &cancel).unwrap();
    assert_eq!(info.size, (64, 48));
    assert!(info.audio);
    assert!(crtsim_media::import_preset(&source, info.size, &cancel).is_err());
    assert_eq!(
        crtsim_media::preview(&info, 0.1, &cancel)
            .unwrap()
            .dimensions(),
        (64, 48)
    );
    let config = Config {
        output: "64x48".into(),
        ..Config::default()
    };
    for (ext, timing, expected_frames, audio) in [
        ("mp4", Timing::Stable, 12, Audio::Encode),
        ("mkv", Timing::Ntsc60, 24, Audio::Auto),
        ("webm", Timing::Disabled, 12, Audio::Auto), // PCM requires Opus fallback in WebM.
    ] {
        let output = dir.path().join(format!("render.{ext}"));
        let options = Options {
            timing,
            audio,
            ..Options::default()
        };
        let mut stages = Vec::new();
        crtsim_media::export_with(
            &info,
            &output,
            &config,
            &options,
            &cancel,
            |image, _| Ok(image.clone()),
            |p| stages.push(p.fraction),
        )
        .unwrap();
        let preset = crtsim_media::import_preset(&output, info.size, &cancel).unwrap();
        assert_eq!(preset.config, config);
        assert_eq!(preset.video_options, options);
        assert_eq!(stages.last(), Some(&1.));
        assert!(stages.windows(2).all(|w| w[0] <= w[1]));
        let result = inspect(&output);
        let streams = result["streams"].as_array().unwrap();
        let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
        assert_eq!(video["color_space"], "bt709");
        assert_eq!(video["color_transfer"], "bt709");
        assert_eq!(video["color_primaries"], "bt709");
        assert_eq!(video["color_range"], "tv");
        assert_eq!(
            video["nb_read_frames"]
                .as_str()
                .unwrap()
                .parse::<u32>()
                .unwrap(),
            expected_frames
        );
        let sound = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
        assert_eq!(
            sound["codec_name"],
            match ext {
                "mp4" => "aac",
                "mkv" => "pcm_s16le",
                _ => "opus",
            }
        );
        let duration: f64 = result["format"]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((duration - 0.4).abs() < 0.05, "{ext} duration {duration}");
    }
    let output = dir.path().join("kept.mp4");
    std::fs::write(&output, b"keep existing destination").unwrap();
    let error = crtsim_media::export_with(
        &info,
        &output,
        &config,
        &Options::default(),
        &cancel,
        |image, _| {
            cancel.store(true, Ordering::Relaxed);
            Ok(image.clone())
        },
        |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(
        std::fs::read(&output).unwrap(),
        b"keep existing destination"
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        5,
        "temporary files leaked"
    );
    cancel.store(false, Ordering::Relaxed);
    let silent = dir.path().join("silent.mp4");
    crtsim_media::export_with(
        &info,
        &silent,
        &config,
        &Options {
            audio: Audio::Mute,
            ..Options::default()
        },
        &cancel,
        |image, _| Ok(image.clone()),
        |_| {},
    )
    .unwrap();
    assert_eq!(inspect(&silent)["streams"].as_array().unwrap().len(), 1);
}

#[test]
#[ignore = "requires FFmpeg, ffprobe and a Vulkan adapter"]
fn video_gpu_sequence_export() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("input.mkv");
    fixture(&path, "30000/1001");
    let cancel = Arc::new(AtomicBool::new(false));
    let info = crtsim_media::probe(&path, &cancel).unwrap();
    let renderer = pollster::block_on(crtsim_core::Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let config = Config {
        output: "160x120".into(),
        signal: "64x48".into(),
        warmup: 2,
        ..Config::default()
    };
    let output = dir.path().join("gpu.mp4");
    crtsim_media::export(
        &info,
        &output,
        &config,
        &Options::default(),
        &renderer,
        &cancel,
        |_| {},
    )
    .unwrap();
    let result = crtsim_media::probe(&output, &cancel).unwrap();
    assert_eq!(result.size, (160, 120));
    assert!(result.audio);
    assert!((result.fps - 30000. / 1001.).abs() < 0.001);
    let frame = crtsim_media::preview_frame(&info, 0, &cancel).unwrap();
    let mut screen_config = config.clone();
    screen_config.screen_only = true;
    assert_ne!(
        renderer.render(&frame, &config).unwrap().crt,
        renderer.render(&frame, &screen_config).unwrap().crt
    );
    let mut times = vec![];
    crtsim_media::playback(
        &info,
        0.15,
        &screen_config,
        &Options::default(),
        &renderer,
        &cancel,
        |time, source, crt| {
            assert_eq!(source.dimensions(), info.size);
            assert_eq!(crt.dimensions(), (160, 120));
            times.push(time);
            Ok(())
        },
    )
    .unwrap();
    assert!(!times.is_empty());
    assert!(times[0] >= 0.15);
    assert!(times.windows(2).all(|p| p[0] < p[1]));
    assert!(crtsim_media::playback(
        &info,
        0.,
        &screen_config,
        &Options::default(),
        &renderer,
        &cancel,
        |_, _, _| {
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        }
    )
    .is_err());
    cancel.store(false, Ordering::Relaxed);
    if let Some(folder) = std::env::var_os("CRTSIM_TEST_OUTPUT") {
        let folder = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::copy(output, folder.join("video-crt.mp4")).unwrap();
        std::fs::copy(path, folder.join("video-input.mkv")).unwrap();
    }
}

#[test]
#[ignore = "requires FFmpeg and ffprobe"]
fn frame_rates_and_audio_offset() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let config = Config {
        output: "64x48".into(),
        ..Config::default()
    };
    let source = dir.path().join("source.mkv");
    let output = dir.path().join("output.mp4");
    for rate in ["24", "25", "30", "50", "60000/1001", "60"] {
        fixture(&source, rate);
        let info = crtsim_media::probe(&source, &cancel).unwrap();
        crtsim_media::export_with(
            &info,
            &output,
            &config,
            &Options {
                audio: Audio::Mute,
                ..Options::default()
            },
            &cancel,
            |image, _| Ok(image.clone()),
            |_| {},
        )
        .unwrap();
        let result = crtsim_media::probe(&output, &cancel).unwrap();
        assert!((result.fps - info.fps).abs() < 0.001, "{rate}");
        assert!(
            (result.duration - info.duration).abs() <= 1. / info.fps + 0.005,
            "{rate}: {:?}",
            result
        );
    }
    // Remove unevenly spaced frames without retiming them, then normalize using timestamps.
    let vfr = dir.path().join("variable-rate.mkv");
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&source)
        .args([
            "-vf",
            "select='not(eq(mod(n,3),1))'",
            "-fps_mode",
            "vfr",
            "-an",
            "-c:v",
            "libx264",
        ])
        .arg(&vfr)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let info = crtsim_media::probe(&vfr, &cancel).unwrap();
    crtsim_media::export_with(
        &info,
        &output,
        &config,
        &Options::default(),
        &cancel,
        |image, _| Ok(image.clone()),
        |_| {},
    )
    .unwrap();
    let normalized = crtsim_media::probe(&output, &cancel).unwrap();
    assert!((normalized.duration - 0.4).abs() <= 1. / info.fps + 0.005);
    let timestamps = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "frame=best_effort_timestamp_time",
            "-of",
            "json",
        ])
        .arg(&output)
        .output()
        .unwrap();
    assert!(timestamps.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&timestamps.stdout).unwrap();
    let times: Vec<f64> = parsed["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            f["best_effort_timestamp_time"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert!(times
        .windows(2)
        .all(|w| ((w[1] - w[0]) - 1. / normalized.fps).abs() < 0.00001));
    let delayed = dir.path().join("delayed.mkv");
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&source)
        .args(["-itsoffset", "0.12", "-i"])
        .arg(&source)
        .args(["-map", "0:v:0", "-map", "1:a:0", "-c", "copy"])
        .arg(&delayed)
        .output()
        .unwrap();
    assert!(result.status.success());
    let info = crtsim_media::probe(&delayed, &cancel).unwrap();
    assert!((info.audio_offset - 0.12).abs() < 0.002);
    crtsim_media::export_with(
        &info,
        &output,
        &config,
        &Options::default(),
        &cancel,
        |image, _| Ok(image.clone()),
        |_| {},
    )
    .unwrap();
    let samples = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args([
            "-map", "0:a:0", "-f", "f32le", "-ac", "1", "-ar", "48000", "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(samples.status.success());
    let samples: Vec<f32> = samples
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let energy = |slice: &[f32]| slice.iter().map(|s| s.abs()).sum::<f32>() / slice.len() as f32;
    assert!(
        energy(&samples[..4000]) < 0.001,
        "audio started before its timestamp"
    );
    assert!(
        energy(&samples[8000..12000]) > 0.01,
        "delayed audio was lost"
    );
}

#[test]
#[ignore = "requires FFmpeg and ffprobe"]
fn exact_frame_navigation_including_variable_rate() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("frames.mkv");
    fixture(&source, "30");
    let vfr = dir.path().join("vfr.mkv");
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&source)
        .args([
            "-vf",
            "select='not(eq(mod(n,3),1))'",
            "-fps_mode",
            "vfr",
            "-an",
            "-c:v",
            "ffv1",
        ])
        .arg(&vfr)
        .output()
        .unwrap();
    assert!(result.status.success());
    let cancel = Arc::new(AtomicBool::new(false));
    for path in [&source, &vfr] {
        let info = crtsim_media::probe(path, &cancel).unwrap();
        let all = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-an",
                "-fps_mode",
                "passthrough",
                "-pix_fmt",
                "rgba",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(all.status.success());
        let frame_bytes = 64 * 48 * 4;
        let count = crtsim_media::frame_count(&info, &cancel).unwrap();
        assert_eq!(count as usize, all.stdout.len() / frame_bytes);
        for index in [0, 1, count - 1, 2, 0] {
            let selected = crtsim_media::preview_frame(&info, index, &cancel).unwrap();
            let start = index as usize * frame_bytes;
            assert_eq!(selected.as_raw(), &all.stdout[start..start + frame_bytes]);
        }
        cancel.store(true, Ordering::Relaxed);
        assert!(crtsim_media::preview_frame(&info, 0, &cancel).is_err());
        cancel.store(false, Ordering::Relaxed);
    }
}

/// A 64x48 animation of `seconds` at 10 frames per second, made by FFmpeg's `encoder`.
fn animation(path: &Path, encoder: &str, seconds: &str) {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=64x48:rate=10")
        .args(["-t", seconds, "-c:v", encoder, "-loop", "0"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires FFmpeg and ffprobe with libx264 and libwebp"]
fn animated_gif_and_webp_open_and_export_to_video() {
    let dir = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let config = Config {
        output: "64x48".into(),
        ..Config::default()
    };
    for (name, encoder) in [("clip.gif", "gif"), ("clip.webp", "libwebp_anim")] {
        let source = dir.path().join(name);
        animation(&source, encoder, "0.5");
        assert!(crtsim_media::MediaKind::of(&source).is_moving());
        let info = crtsim_media::probe(&source, &cancel).unwrap();
        assert_eq!(info.size, (64, 48), "{name}");
        assert_eq!(crtsim_media::frame_count(&info, &cancel).unwrap(), 5);
        assert!(
            (info.duration - 0.5).abs() < 1e-6,
            "{name}: {}",
            info.duration
        );
        let third = crtsim_media::preview_frame(&info, 2, &cancel).unwrap();
        assert_eq!(third.dimensions(), (64, 48));
        let output = dir.path().join(format!("{name}.mp4"));
        crtsim_media::export_with(
            &info,
            &output,
            &config,
            &Options::default(),
            &cancel,
            |image, _| Ok(image.clone()),
            |_| {},
        )
        .unwrap();
        let data = inspect(&output);
        let streams = data["streams"].as_array().unwrap();
        assert_eq!(streams.len(), 1, "{name}: only the picture");
        assert_eq!(streams[0]["nb_read_frames"], "5", "{name}");
        let preset = crtsim_media::import_preset(&output, info.size, &cancel).unwrap();
        assert_eq!(preset.config, config);
    }
}

/// Each frame's display time in seconds, as ffprobe reads a GIF.
fn gif_delays(path: &Path) -> Vec<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "frame=duration_time",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|line| line.trim().parse().unwrap())
        .collect()
}

#[test]
#[ignore = "requires FFmpeg, ffprobe with libx264 and libwebp, and a Vulkan adapter"]
fn animation_exports_are_small_timed_and_replace_the_output_only_when_done() {
    use crtsim_media::{AnimationFormat, AnimationOptions, Dither};
    let dir = tempfile::tempdir().unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let renderer = pollster::block_on(crtsim_core::Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    // Busy, moving test footage and a still test card: the extremes of what compresses.
    let mut sources = vec![];
    for (name, input) in [
        ("busy.mkv", "testsrc2=size=640x480:rate=30"),
        ("still.mkv", "smptebars=size=640x480:rate=30"),
    ] {
        let path = dir.path().join(name);
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", input])
            .args(["-t", "3", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(output.status.success());
        sources.push((name, crtsim_media::probe(&path, &cancel).unwrap()));
    }
    let config = Config {
        output: "1920x1080".into(),
        ..Config::default()
    };
    let two_seconds = AnimationOptions {
        max_seconds: Some(2.),
        ..AnimationOptions::default()
    };
    let exports = [
        ("bayer.gif", two_seconds.clone()),
        (
            "diffusion.gif",
            AnimationOptions {
                dither: Dither::Diffusion,
                ..two_seconds.clone()
            },
        ),
        (
            "plain.gif",
            AnimationOptions {
                dither: Dither::None,
                ..two_seconds.clone()
            },
        ),
        ("lossy.webp", two_seconds.clone()),
        (
            "lossless.webp",
            AnimationOptions {
                lossless: true,
                ..two_seconds.clone()
            },
        ),
    ];
    for (source_name, source) in &sources {
        for (name, options) in &exports {
            let output = dir.path().join(format!("{source_name}-{name}"));
            let format = AnimationFormat::of(&output).unwrap();
            let summary = options.summary(source, &config, format).unwrap();
            assert_eq!((summary.size, summary.frames), ((640, 360), 48));
            crtsim_media::export_animation(
                source,
                &output,
                &config,
                options,
                &renderer,
                &cancel,
                |_| {},
            )
            .unwrap();
            let bytes = std::fs::metadata(&output).unwrap().len();
            let per_pixel = bytes as f64 / (640. * 360. * 48.);
            println!(
                "{source_name} {name}: {bytes} bytes, {per_pixel:.3} per pixel; estimated {:?}",
                summary.bytes
            );
            if *source_name == "still.mkv" {
                // libwebp stores a run of identical frames as one longer frame.
                continue;
            }
            // Opened again as the animation it is.
            let back = crtsim_media::probe(&output, &cancel).unwrap();
            assert_eq!((back.size, back.frames), ((640, 360), Some(48)), "{name}");
            assert!(
                (back.duration - 2.).abs() < 0.03,
                "{name}: {}",
                back.duration
            );
        }
    }
    // 24 per second in hundredths: each frame shows for 4 or 5, averaging exactly 24.
    let delays = gif_delays(&dir.path().join("busy.mkv-bayer.gif"));
    assert_eq!(delays.len(), 48);
    assert!(delays.iter().all(|&d| d == 0.04 || d == 0.05), "{delays:?}");
    assert!((delays.iter().sum::<f64>() - 2.).abs() < 0.011);
    let gif = std::fs::read(dir.path().join("busy.mkv-bayer.gif")).unwrap();
    assert!(
        gif.windows(11).any(|w| w == b"NETSCAPE2.0"),
        "loops forever"
    );

    // A later start and a shorter limit, from an animated source.
    let (_, busy) = &sources[0];
    let clip = dir.path().join("clip.gif");
    let options = AnimationOptions {
        start: 1.,
        max_seconds: Some(0.5),
        fps: 10,
        ..AnimationOptions::default()
    };
    crtsim_media::export_animation(busy, &clip, &config, &options, &renderer, &cancel, |_| {})
        .unwrap();
    let clip_info = crtsim_media::probe(&clip, &cancel).unwrap();
    assert_eq!(clip_info.frames, Some(5));
    let again = dir.path().join("again.webp");
    crtsim_media::export_animation(
        &clip_info,
        &again,
        &config,
        &AnimationOptions::default(),
        &renderer,
        &cancel,
        |_| {},
    )
    .unwrap();
    // Half a second at 24 per second; libwebp may merge frames that came out identical.
    let again = crtsim_media::probe(&again, &cancel).unwrap();
    assert!((again.duration - 0.5).abs() < 0.03, "{}", again.duration);

    // A cancelled GIF leaves the old file and no temporary files behind.
    let kept = dir.path().join("kept.gif");
    std::fs::write(&kept, b"keep existing destination").unwrap();
    let files = std::fs::read_dir(dir.path()).unwrap().count();
    let error = crtsim_media::export_animation_with(
        busy,
        &kept,
        &config,
        &two_seconds,
        &cancel,
        |image, _| {
            cancel.store(true, Ordering::Relaxed);
            Ok(image::imageops::resize(
                image,
                640,
                360,
                image::imageops::FilterType::Nearest,
            ))
        },
        |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert_eq!(std::fs::read(&kept).unwrap(), b"keep existing destination");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), files);
}
