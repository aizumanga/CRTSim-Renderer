//! Bounded-memory FFmpeg decode -> ordered GPU frames -> encode -> audio mux.
mod process;

use anyhow::{ensure, Context, Result};
use crtsim_core::{
    config::{self, Config, Phase},
    Renderer, Sequence,
};
use image::RgbaImage;
use process::Process;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

#[derive(Clone, Debug)]
pub struct Video {
    pub path: PathBuf,
    pub size: (u32, u32),
    pub fps: f64,
    pub rate: String,
    pub duration: f64,
    pub audio: bool,
    pub audio_offset: f64,
    pub hdr: bool,
    pub stream: u64,
    /// Container-provided decoded-frame count. Missing for many streaming/Matroska sources.
    pub frames: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Timing {
    #[default]
    Stable,
    Ntsc60,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Audio {
    #[default]
    Auto,
    Encode,
    Mute,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub timing: Timing,
    pub audio: Audio,
}

const PRESET_PREFIX: &str = "CRTSim-Renderer-Preset:";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub version: u32,
    pub config: Config,
    pub video_options: Options,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PresetWire {
    version: u32,
    config: Value,
    video_options: Options,
}

/// Container comments survive MP4, Matroska and WebM muxing, unlike arbitrary MP4 keys.
pub fn import_preset(path: &Path, input: (u32, u32), cancel: &Arc<AtomicBool>) -> Result<Preset> {
    let mut cmd = command("ffprobe");
    cmd.args(["-show_entries", "format_tags=comment", "-of", "json"])
        .arg(path.canonicalize().context("Cannot open video")?);
    let mut process = Process::spawn(&mut cmd, cancel)?;
    drop(process.stdin());
    let mut bytes = Vec::new();
    process
        .stdout()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "Video preset metadata exceeds 1 MB"
    );
    process.wait()?;
    parse_preset(&serde_json::from_slice(&bytes)?, input)
}

fn parse_preset(root: &Value, input: (u32, u32)) -> Result<Preset> {
    let comment = root["format"]["tags"]
        .as_object()
        .and_then(|tags| {
            tags.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("comment"))
        })
        .and_then(|(_, value)| value.as_str())
        .and_then(|text| text.strip_prefix(PRESET_PREFIX))
        .context("This video does not contain a CRTSim-Renderer preset")?;
    let wire: PresetWire =
        serde_json::from_str(comment).context("Invalid video preset metadata")?;
    ensure!(wire.version == 1, "Unsupported video preset version");
    let preset = Preset {
        version: wire.version,
        config: Config::from_json_slice(&serde_json::to_vec(&wire.config)?)?,
        video_options: wire.video_options,
    };
    preset.config.signal_size(input)?;
    preset.config.output_size(input)?;
    Ok(preset)
}

/// Count decoded frames only when opening a video; subsequent frame seeks reuse this count.
pub fn frame_count(video: &Video, cancel: &Arc<AtomicBool>) -> Result<u64> {
    check_cancel(cancel)?;
    if let Some(frames) = video.frames.filter(|frames| *frames > 0) {
        return Ok(frames);
    }
    let mut cmd = command("ffprobe");
    cmd.args([
        "-select_streams",
        &video.stream.to_string(),
        "-count_frames",
        "-show_entries",
        "stream=nb_read_frames",
        "-of",
        "json",
    ])
    .arg(&video.path);
    let mut process = Process::spawn(&mut cmd, cancel)?;
    drop(process.stdin());
    let mut bytes = Vec::new();
    process.stdout().take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "Frame count response is too large");
    process.wait()?;
    let root: Value = serde_json::from_slice(&bytes)?;
    let count = root["streams"][0]["nb_read_frames"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .context("Cannot count video frames")?;
    ensure!(count > 0, "Video contains no decoded frames");
    Ok(count)
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub fraction: f32,
    pub stage: String,
}

pub fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "Export cancelled");
    Ok(())
}

fn command(program: &str) -> Command {
    // Explicit executable overrides also support portable FFmpeg installations with spaces.
    let key = if program == "ffmpeg" {
        "CRTSIM_FFMPEG"
    } else {
        "CRTSIM_FFPROBE"
    };
    let mut command = Command::new(std::env::var_os(key).unwrap_or_else(|| program.into()));
    if std::env::var_os("CRTSIM_APPIMAGE").is_some() {
        if let Some(original) = std::env::var_os("CRTSIM_HOST_LD_LIBRARY_PATH") {
            command.env("LD_LIBRARY_PATH", original);
        }
    }
    command.args(["-v", "error"]);
    if program == "ffmpeg" {
        command.arg("-nostdin");
    }
    command
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|n| n.is_finite())
}

fn rate(text: &str) -> Option<f64> {
    let (a, b) = text.split_once('/').or_else(|| text.split_once(':'))?;
    let result = a.parse::<f64>().ok()? / b.parse::<f64>().ok()?;
    (result.is_finite() && result > 0.).then_some(result)
}

fn positive_integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .filter(|value| *value > 0)
}

pub fn probe(path: &Path, cancel: &Arc<AtomicBool>) -> Result<Video> {
    let path = path.canonicalize().context("Cannot open video")?;
    ensure!(path.is_file(), "Choose a local video file");
    let mut cmd = command("ffprobe");
    cmd.args(["-show_streams", "-show_format", "-of", "json"])
        .arg(&path);
    let mut process = Process::spawn(&mut cmd, cancel)?;
    drop(process.stdin());
    let mut bytes = Vec::new();
    process
        .stdout()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "Video metadata exceeds 1 MB");
    process.wait()?;
    parse_probe(path, &serde_json::from_slice(&bytes)?)
}

fn parse_probe(path: PathBuf, root: &Value) -> Result<Video> {
    let streams = root["streams"].as_array().context("No streams in video")?;
    let v = streams
        .iter()
        .find(|v| v["codec_type"] == "video" && v["disposition"]["attached_pic"] != 1)
        .context("No video stream")?;
    let w = v["width"].as_u64().context("No video width")?;
    let h = v["height"].as_u64().context("No video height")?;
    ensure!(w <= 16384 && h <= 16384, "Video dimensions exceed 16384");
    let mut size = (w as u32, h as u32);
    config::validate_size(size)?;
    // Normalize non-square source pixels before the CRT's own pixel-aspect setting.
    let sar = v["sample_aspect_ratio"]
        .as_str()
        .and_then(rate)
        .unwrap_or(1.);
    ensure!(
        (0.1..=10.).contains(&sar),
        "Unsupported video pixel aspect ratio"
    );
    size.0 = (f64::from(size.0) * sar).round().max(1.) as u32;
    let rotation = v["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|d| number(&d["rotation"])))
        .or_else(|| number(&v["tags"]["rotate"]))
        .unwrap_or(0.);
    let rotation = (rotation.round() as i32).rem_euclid(360);
    ensure!(
        [0, 90, 180, 270].contains(&rotation),
        "Only right-angle video rotation is supported"
    );
    if rotation == 90 || rotation == 270 {
        size = (size.1, size.0);
    }
    config::validate_size(size)?;
    let rate_text = ["avg_frame_rate", "r_frame_rate"]
        .iter()
        .find_map(|key| {
            let text = v[*key].as_str()?;
            let fps = rate(text)?;
            ((1.0..=240.0).contains(&fps)).then_some(text.to_string())
        })
        .context("Cannot determine a supported frame rate (1–240 FPS)")?;
    let duration = number(&v["duration"])
        .or_else(|| number(&root["format"]["duration"]))
        .context("Cannot determine video duration")?;
    ensure!(
        duration > 0. && duration <= 7. * 24. * 3600.,
        "Unsupported video duration"
    );
    let a = streams.iter().find(|v| v["codec_type"] == "audio");
    let video_start = number(&v["start_time"]).unwrap_or(0.);
    let audio_start = a
        .and_then(|a| number(&a["start_time"]))
        .unwrap_or(video_start);
    Ok(Video {
        path,
        size,
        fps: rate(&rate_text).unwrap(),
        rate: rate_text,
        duration,
        audio: a.is_some(),
        audio_offset: audio_start - video_start,
        hdr: matches!(
            v["color_transfer"].as_str(),
            Some("smpte2084" | "arib-std-b67")
        ),
        stream: v["index"].as_u64().context("Missing video stream index")?,
        frames: positive_integer(&v["nb_frames"]),
    })
}

fn require_encoder(name: &str, cancel: &Arc<AtomicBool>) -> Result<()> {
    let mut cmd = command("ffmpeg");
    cmd.args(["-hide_banner", "-encoders"]);
    let mut process = Process::spawn(&mut cmd, cancel)?;
    drop(process.stdin());
    let mut bytes = Vec::new();
    process
        .stdout()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "FFmpeg encoder list is too large"
    );
    process.wait()?;
    let available = String::from_utf8_lossy(&bytes)
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name));
    ensure!(
        available,
        "This FFmpeg installation does not provide the required {name} video encoder"
    );
    Ok(())
}

fn decode_command(video: &Video, fps: Option<&str>, time: f64, frame: Option<u64>) -> Command {
    let mut cmd = command("ffmpeg");
    // Seeking is input-relative and preview-only; full exports always start at zero.
    if time > 0. {
        cmd.args(["-ss", &time.to_string()]);
    }
    cmd.arg("-i").arg(&video.path).args([
        "-map",
        &format!("0:{}", video.stream),
        "-an",
        "-sn",
        "-dn",
    ]);
    let mut filters = vec!["setpts=PTS-STARTPTS".to_string()];
    if let Some(frame) = frame {
        // Select by decoded frame ordinal, including VFR sources. Decode from the start
        // to avoid timestamp rounding and keyframe seeks skipping or repeating frames.
        filters.push(format!("select=eq(n\\,{frame})"));
    }
    if let Some(fps) = fps {
        filters.push(format!("fps={fps}:start_time=0:round=near"));
    }
    if video.hdr {
        filters.push("zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=mobius:desat=0,zscale=t=bt709:m=bt709:r=full".into());
    }
    filters.push(format!(
        "scale={}:{}:flags=lanczos,setsar=1",
        video.size.0, video.size.1
    ));
    cmd.args(["-vf", &filters.join(",")]);
    if fps.is_none() {
        cmd.args(["-frames:v", "1"]);
    }
    cmd.args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    cmd
}

pub fn preview(video: &Video, time: f64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    ensure!(
        time.is_finite() && time >= 0. && time < video.duration,
        "Preview time is outside the video"
    );
    let mut child = Process::spawn(&mut decode_command(video, None, time, None), cancel)?;
    drop(child.stdin());
    let mut bytes = vec![0; video.size.0 as usize * video.size.1 as usize * 4];
    let read = child.stdout().read_exact(&mut bytes);
    child.wait()?;
    read.context("FFmpeg did not return a complete preview frame")?;
    RgbaImage::from_raw(video.size.0, video.size.1, bytes).context("Invalid preview pixels")
}

pub fn preview_frame(video: &Video, frame: u64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    let mut child = Process::spawn(&mut decode_command(video, None, 0., Some(frame)), cancel)?;
    drop(child.stdin());
    let mut bytes = vec![0; video.size.0 as usize * video.size.1 as usize * 4];
    let read = child.stdout().read_exact(&mut bytes);
    child.wait()?;
    read.context("FFmpeg did not return the requested frame")?;
    RgbaImage::from_raw(video.size.0, video.size.1, bytes).context("Invalid preview pixels")
}

pub fn render_config(config: &Config, options: &Options, fps: f64) -> Config {
    let mut c = config.clone();
    match options.timing {
        Timing::Stable => {
            c.phase = Phase::Stable;
            for weight in &mut c.persistence {
                *weight = weight.powf((60. / fps) as f32);
            }
        }
        Timing::Ntsc60 => c.phase = Phase::Alternating,
        Timing::Disabled => {
            c.phase = Phase::Stable;
            c.persistence = [0.; 3];
            c.warmup = 0;
        }
    }
    c
}

/// The caller supplies the renderer, so the same media pipeline can be tested without a GPU.
pub fn export_with(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    cancel: &Arc<AtomicBool>,
    mut render: impl FnMut(&RgbaImage, &Config) -> Result<RgbaImage>,
    mut progress: impl FnMut(Progress),
) -> Result<()> {
    check_cancel(cancel)?;
    config.validate()?;
    config.signal_size(video.size)?;
    let size = config.output_size(video.size)?;
    ensure!(
        size.0 % 2 == 0 && size.1 % 2 == 0,
        "Video output width and height must be even (for example 1920×1080)"
    );
    let extension = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        ["mp4", "mkv", "webm"].contains(&extension.as_str()),
        "Choose an MP4, MKV or WebM filename"
    );
    require_encoder(
        if extension == "webm" {
            "libvpx-vp9"
        } else {
            "libx264"
        },
        cancel,
    )?;
    if output.exists() {
        ensure!(
            output.canonicalize()? != video.path,
            "Choose an output other than the source video"
        );
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let folder = tempfile::tempdir_in(parent)?;
    let silent = folder.path().join(format!("video.{extension}"));
    let final_file = tempfile::NamedTempFile::new_in(parent)?;
    let rate = if options.timing == Timing::Ntsc60 {
        "60/1"
    } else {
        &video.rate
    };
    let fps = if options.timing == Timing::Ntsc60 {
        60.
    } else {
        video.fps
    };
    let c = render_config(config, options, fps);
    // Store the user's original controls plus timing, not the decay-adjusted config:
    // reimporting the latter would apply the timing correction a second time.
    let metadata = format!(
        "{PRESET_PREFIX}{}",
        serde_json::to_string(&Preset {
            version: 1,
            config: config.clone(),
            video_options: options.clone(),
        })?
    );
    ensure!(
        metadata.len() <= 16384,
        "Video preset metadata exceeds 16 KB"
    );
    let mut encode_cmd = command("ffmpeg");
    encode_cmd.args([
        "-y",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &format!("{}x{}", size.0, size.1),
        "-r",
        rate,
        "-i",
        "pipe:0",
        "-an",
        "-c:v",
    ]);
    if extension == "webm" {
        encode_cmd.args([
            "libvpx-vp9",
            "-crf",
            "24",
            "-b:v",
            "0",
            "-deadline",
            "good",
            "-cpu-used",
            "4",
        ]);
    } else {
        encode_cmd.args(["libx264", "-crf", "18", "-preset", "medium"]);
    }
    // Renderer bytes are full-range RGB with BT.709/sRGB primaries. Make the RGB-to-YUV
    // matrix and legal range explicit, then tag the encoded stream for consistent playback.
    encode_cmd
        .args([
            "-vf",
            "scale=in_range=full:out_range=tv:in_color_matrix=bt709:out_color_matrix=bt709,format=yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ])
        .arg(&silent);
    let mut encoder = Process::spawn(&mut encode_cmd, cancel)?;
    let mut input = encoder.stdin();
    let mut decoder = Process::spawn(&mut decode_command(video, Some(rate), 0., None), cancel)?;
    drop(decoder.stdin());
    let mut decoded = decoder.stdout();
    let mut frame = RgbaImage::new(video.size.0, video.size.1);
    let started = Instant::now();
    let mut frames = 0u64;
    progress(Progress {
        fraction: 0.,
        stage: "Decoding and rendering video".into(),
    });
    loop {
        check_cancel(cancel)?;
        let bytes = frame.as_mut();
        let count = decoded.read(&mut bytes[..1])?;
        if count == 0 {
            break;
        }
        decoded
            .read_exact(&mut bytes[1..])
            .context("Truncated decoded video frame")?;
        let result = render(&frame, &c)?;
        ensure!(
            result.dimensions() == size,
            "Renderer returned the wrong video dimensions"
        );
        check_cancel(cancel)?;
        if let Err(error) = input.write_all(result.as_raw()) {
            drop(input);
            encoder.wait()?;
            return Err(error.into());
        }
        frames += 1;
        let fraction = ((frames as f64 / fps) / video.duration).min(1.);
        let remaining = if fraction > 0. {
            started.elapsed().as_secs_f64() * (1. - fraction) / fraction
        } else {
            0.
        };
        progress(Progress {
            fraction: (fraction * 0.9) as f32,
            stage: format!(
                "Frame {frames} · {:.1} FPS · approximately {:.0}s remaining",
                frames as f64 / started.elapsed().as_secs_f64().max(0.001),
                remaining
            ),
        });
    }
    decoder.wait()?;
    ensure!(frames > 0, "No frames decoded");
    drop(input);
    progress(Progress {
        fraction: 0.91,
        stage: "Finishing video encoding".into(),
    });
    encoder.wait()?;
    check_cancel(cancel)?;
    let duration = frames as f64 / fps;
    let mux = |copy_audio: bool| -> Result<()> {
        let mut cmd = command("ffmpeg");
        cmd.args(["-y", "-i"]).arg(&silent);
        let audio = video.audio && options.audio != Audio::Mute;
        if audio {
            cmd.arg("-i").arg(&video.path);
        }
        cmd.args(["-map", "0:v:0", "-c:v", "copy"]);
        if audio {
            cmd.args(["-map", "1:a:0"]);
            if copy_audio {
                cmd.args(["-c:a", "copy"]);
            } else {
                cmd.args([
                    "-c:a",
                    if extension == "webm" {
                        "libopus"
                    } else {
                        "aac"
                    },
                    "-b:a",
                    "192k",
                ]);
                let filter = if video.audio_offset >= 0. {
                    format!(
                        "asetpts=PTS-STARTPTS,adelay={}:all=1",
                        (video.audio_offset * 1000.).round()
                    )
                } else {
                    format!("atrim=start={},asetpts=PTS-STARTPTS", -video.audio_offset)
                };
                cmd.args(["-af", &filter]);
            }
        }
        cmd.args([
            "-t",
            &duration.to_string(),
            "-map_metadata",
            "-1",
            "-metadata",
            &format!("comment={metadata}"),
        ]);
        if extension == "mp4" {
            cmd.args(["-movflags", "+faststart"]);
        }
        cmd.args([
            "-f",
            if extension == "mkv" {
                "matroska"
            } else {
                &extension
            },
        ])
        .arg(final_file.path());
        let mut process = Process::spawn(&mut cmd, cancel)?;
        drop(process.stdin());
        process.wait()
    };
    progress(Progress {
        fraction: 0.95,
        stage: "Preserving audio and finalizing container".into(),
    });
    let copy = options.audio == Audio::Auto && video.audio_offset.abs() < 0.002;
    if let Err(error) = mux(copy) {
        check_cancel(cancel)?;
        if copy {
            mux(false).context("Audio remux and re-encoding both failed")?;
        } else {
            return Err(error);
        }
    }
    check_cancel(cancel)?;
    final_file.as_file().sync_all()?;
    final_file
        .persist(output)
        .map_err(|e| anyhow::anyhow!("Cannot publish output: {}", e.error))?;
    progress(Progress {
        fraction: 1.,
        stage: format!("Saved {frames} frames with {:.3}s duration", duration),
    });
    Ok(())
}

pub fn export(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(Progress),
) -> Result<()> {
    let mut sequence = Sequence::default();
    export_with(
        video,
        output,
        config,
        options,
        cancel,
        |frame, config| {
            renderer.render_video_frame_with_cancel(frame, config, &mut sequence, cancel)
        },
        progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_and_duration_validation() {
        assert_eq!(rate("60000/1001"), Some(60000. / 1001.));
        assert_eq!(rate("0/0"), None);
        assert_eq!(rate("30/0"), None);
        let mut metadata = serde_json::json!({"streams": [{"index":0,"codec_type":"video","width":720,"height":480,"sample_aspect_ratio":"8:9","avg_frame_rate":"30000/1001","duration":"1.0","side_data_list":[{"rotation":90}]}]});
        let info = parse_probe("x.mkv".into(), &metadata).unwrap();
        assert_eq!(info.size, (480, 640));
        assert_eq!(info.frames, None);
        metadata["streams"][0]["nb_frames"] = "42".into();
        assert_eq!(
            parse_probe("x.mkv".into(), &metadata).unwrap().frames,
            Some(42)
        );
        metadata["streams"][0]["duration"] = "NaN".into();
        assert!(parse_probe("x.mkv".into(), &metadata).is_err());
    }

    #[test]
    fn metadata_rejects_unknown_versions_invalid_settings_and_unrelated_comments() {
        let mut data = serde_json::json!({"version": 1, "config": Config::default(), "video_options": Options::default()});
        let tagged = |data: &Value| serde_json::json!({"format": {"tags": {"COMMENT": format!("{PRESET_PREFIX}{data}")}}});
        assert!(parse_preset(&tagged(&data), (64, 48)).is_ok());
        data["version"] = 2.into();
        assert!(parse_preset(&tagged(&data), (64, 48)).is_err());
        data["version"] = 1.into();
        data["config"]["output"] = "0x0".into();
        assert!(parse_preset(&tagged(&data), (64, 48)).is_err());
        assert!(parse_preset(
            &serde_json::json!({"format":{"tags":{"comment":"ordinary comment"}}}),
            (64, 48)
        )
        .is_err());
    }

    #[test]
    fn persistence_uses_media_time() {
        let config = Config::default();
        let at30 = render_config(&config, &Options::default(), 30.);
        let at60 = render_config(&config, &Options::default(), 60.);
        assert!((at30.persistence[0] - at60.persistence[0].powi(2)).abs() < 0.00001);
        let disabled = render_config(
            &config,
            &Options {
                timing: Timing::Disabled,
                ..Options::default()
            },
            30.,
        );
        assert_eq!(disabled.persistence, [0.; 3]);
    }
}
