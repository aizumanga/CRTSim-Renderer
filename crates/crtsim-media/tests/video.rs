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
#[ignore = "requires FFmpeg and ffprobe with libx264, libvpx-vp9, AAC and Opus"]
fn ffmpeg_streaming_audio_timing_and_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source with spaces.mkv");
    fixture(&source, "30");
    let cancel = Arc::new(AtomicBool::new(false));
    let info = crtsim_media::probe(&source, &cancel).unwrap();
    assert_eq!(info.size, (64, 48));
    assert!(info.audio);
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
        let options = Options { timing, audio };
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
        assert_eq!(stages.last(), Some(&1.));
        assert!(stages.windows(2).all(|w| w[0] <= w[1]));
        let result = inspect(&output);
        let streams = result["streams"].as_array().unwrap();
        let video = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
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
