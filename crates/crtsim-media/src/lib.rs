//! Bounded-memory FFmpeg decode -> ordered GPU frames -> encode -> audio mux.
mod plan;
mod process;

use anyhow::{ensure, Context, Result};
use crtsim_core::{
    config::{self, Config, Phase},
    RenderProgress, Renderer, Sequence,
};
use image::RgbaImage;
use plan::ExportPlan;
use process::Process;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Instant,
};

/// Files opened as video, by extension; FFmpeg decodes them.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "m4v"];

/// Whether a file is opened as video rather than as an image.
pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

#[derive(Clone, Debug)]
pub struct Video {
    pub metadata: std::collections::BTreeMap<String, String>,
    pub tracks: Vec<Track>,
    pub start: f64,
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

impl Video {
    /// The tracks that hold `kind`, in the file's order.
    pub fn tracks_of(&self, kind: TrackKind) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(move |track| track.kind == kind)
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub index: u64,
    pub kind: TrackKind,
    pub codec: String,
    pub offset: f64,
}

/// What a track holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
    Subtitle,
    Attachment,
    Data,
    Other,
}

impl TrackKind {
    /// The kind ffprobe reports as `codec_type`.
    fn parse(codec_type: &str) -> Self {
        match codec_type {
            "video" => Self::Video,
            "audio" => Self::Audio,
            "subtitle" => Self::Subtitle,
            "attachment" => Self::Attachment,
            "data" => Self::Data,
            _ => Self::Other,
        }
    }
}

/// The kinds of file a video is exported to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Container {
    /// H.264, which plays on nearly everything.
    #[default]
    Mp4,
    /// H.264 in a container that keeps more kinds of track.
    Mkv,
    /// VP9, for the web.
    Webm,
}

impl Container {
    pub const ALL: [Self; 3] = [Self::Mp4, Self::Mkv, Self::Webm];

    /// The container a file name asks for, by its extension.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Self::ALL
            .into_iter()
            .find(|container| extension.eq_ignore_ascii_case(container.extension()))
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }

    /// FFmpeg's name for it.
    fn format(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "matroska",
            Self::Webm => "webm",
        }
    }

    /// The codec audio is re-encoded with when it cannot be copied as it is.
    fn audio_codec(self) -> &'static str {
        match self {
            Self::Webm => "libopus",
            Self::Mp4 | Self::Mkv => "aac",
        }
    }

    /// How a subtitle track in `codec` is stored, if it can be: Matroska copies any, the
    /// others convert text subtitles to their own format and cannot hold bitmap ones.
    fn subtitle_codec(self, codec: &str) -> Option<&'static str> {
        let text = ["subrip", "ass", "ssa", "webvtt", "mov_text", "text"].contains(&codec);
        match self {
            Self::Mkv => Some("copy"),
            Self::Mp4 if text => Some("mov_text"),
            Self::Webm if text => Some("webvtt"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Quality {
    Draft,
    #[default]
    Balanced,
    High,
    Archival,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoder {
    #[default]
    Software,
    Nvenc,
    Qsv,
    Amf,
    VideoToolbox,
}
impl Encoder {
    pub fn codec(self, container: Container) -> Result<&'static str> {
        let webm = container == Container::Webm;
        ensure!(
            !webm || self == Self::Software,
            "WebM requires the software VP9 encoder; choose MP4/MKV for hardware H.264"
        );
        Ok(match self {
            Self::Software if webm => "libvpx-vp9",
            Self::Software => "libx264",
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::VideoToolbox => "h264_videotoolbox",
        })
    }
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub timing: Timing,
    pub audio: Audio,
    pub quality: Quality,
    pub encoder: Encoder,
    pub preserve_streams: bool,
    /// Optional constant-quality override for software H.264/VP9 (lower is better).
    pub crf: Option<u8>,
    /// Optional fixed target bitrate for hardware H.264, in megabits per second.
    pub bitrate_mbps: Option<u32>,
    pub speed: Option<EncodingSpeed>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EncodingSpeed {
    Fast,
    Balanced,
    Slow,
}
impl Options {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.crf.is_none_or(|v| v <= 51),
            "Quality value must be 0–51"
        );
        ensure!(
            self.bitrate_mbps.is_none_or(|v| (1..=200).contains(&v)),
            "Video bitrate must be 1–200 Mbps"
        );
        Ok(())
    }
    /// The constant quality a software encoder is given: the one asked for, or the quality
    /// profile's, which is higher for VP9 to give files of a similar size.
    pub fn effective_crf(&self, container: Container) -> u8 {
        self.crf.unwrap_or(
            match self.quality {
                Quality::Draft => 26,
                Quality::Balanced => 18,
                Quality::High => 14,
                Quality::Archival => 10,
            } + if container == Container::Webm { 6 } else { 0 },
        )
    }
}
impl Default for Options {
    fn default() -> Self {
        Self {
            timing: Timing::default(),
            audio: Audio::default(),
            quality: Quality::default(),
            encoder: Encoder::default(),
            preserve_streams: true,
            crf: None,
            bitrate_mbps: None,
            speed: None,
        }
    }
}

/// The tracks of `video` an export to `container` leaves out, when it keeps the others.
pub fn preservation_notes(video: &Video, container: Container) -> Vec<String> {
    let mut notes = vec![];
    for t in &video.tracks {
        match t.kind {
            TrackKind::Subtitle if container.subtitle_codec(&t.codec).is_none() => {
                notes.push(format!(
                    "Subtitle {} ({}) cannot be stored in {}; use MKV to preserve it.",
                    t.index,
                    t.codec,
                    container.extension()
                ))
            }
            TrackKind::Attachment if container != Container::Mkv => {
                notes.push(format!("Attachment {} is preserved only in MKV.", t.index))
            }
            TrackKind::Data => notes.push(format!("Data track {} is not copied.", t.index)),
            _ => {}
        }
    }
    notes
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
    let bytes = Process::output(
        &mut cmd,
        cancel,
        32 * 1024 * 1024,
        "Video preset metadata exceeds 32 MB",
    )?;
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
    preset.video_options.validate()?;
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
    let bytes = Process::output(&mut cmd, cancel, 65536, "Frame count response is too large")?;
    let root: Value = serde_json::from_slice(&bytes)?;
    let count = root["streams"][0]["nb_read_frames"]
        .as_str()
        .and_then(|s| s.parse::<u64>().ok())
        .context("Cannot count video frames")?;
    ensure!(count > 0, "Video contains no decoded frames");
    Ok(count)
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
    let bytes = Process::output(
        &mut cmd,
        cancel,
        32 * 1024 * 1024,
        "Video metadata exceeds 32 MB",
    )?;
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
        metadata: root["format"]["tags"]
            .as_object()
            .map(|tags| {
                tags.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
        tracks: streams
            .iter()
            .filter_map(|s| {
                Some(Track {
                    index: s["index"].as_u64()?,
                    kind: TrackKind::parse(s["codec_type"].as_str()?),
                    codec: s["codec_name"].as_str().unwrap_or("").into(),
                    offset: number(&s["start_time"]).unwrap_or(video_start) - video_start,
                })
            })
            .collect(),
        start: video_start,
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
    let bytes = Process::output(
        &mut cmd,
        cancel,
        2 * 1024 * 1024,
        "FFmpeg encoder list is too large",
    )?;
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
        filters.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
             tonemap=tonemap=mobius:desat=0,zscale=t=bt709:m=bt709:r=full"
                .into(),
        );
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

/// The frame shown at `time` seconds.
pub fn preview(video: &Video, time: f64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    ensure!(
        time.is_finite() && time >= 0. && time < video.duration,
        "Preview time is outside the video"
    );
    let mut decode = decode_command(video, None, time, None);
    decode_one(
        video,
        &mut decode,
        cancel,
        "FFmpeg did not return a complete preview frame",
    )
}

/// The frame numbered `frame`, counting from 0 in decoding order.
pub fn preview_frame(video: &Video, frame: u64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    let mut decode = decode_command(video, None, 0., Some(frame));
    decode_one(
        video,
        &mut decode,
        cancel,
        "FFmpeg did not return the requested frame",
    )
}

/// Runs a decode that stops after one frame and returns it. `missing` is the error when it
/// ends before a whole frame.
fn decode_one(
    video: &Video,
    decode: &mut Command,
    cancel: &Arc<AtomicBool>,
    missing: &str,
) -> Result<RgbaImage> {
    let mut child = Process::spawn(decode, cancel)?;
    drop(child.stdin());
    let mut bytes = vec![0; video.size.0 as usize * video.size.1 as usize * 4];
    let read = child.stdout().read_exact(&mut bytes);
    child.wait()?;
    read.context(missing.to_owned())?;
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

/// The rate frames are rendered and encoded at.
#[derive(Clone, Debug)]
pub(crate) struct Rate {
    /// As FFmpeg is told it: exact, such as 30000/1001.
    pub text: String,
    pub fps: f64,
}

impl Rate {
    /// The source's own rate, or 60 per second for NTSC timing.
    pub fn of(video: &Video, options: &Options) -> Self {
        if options.timing == Timing::Ntsc60 {
            Self {
                text: "60/1".into(),
                fps: 60.,
            }
        } else {
            Self {
                text: video.rate.clone(),
                fps: video.fps,
            }
        }
    }
}

/// The caller supplies the renderer, so the same media pipeline can be tested without a GPU.
pub fn export_with(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    cancel: &Arc<AtomicBool>,
    mut render: impl FnMut(&RgbaImage, &Config) -> Result<RgbaImage>,
    mut progress: impl FnMut(RenderProgress),
) -> Result<()> {
    check_cancel(cancel)?;
    let plan = ExportPlan::new(video, output, config, options)?;
    require_encoder(plan.codec, cancel)?;
    if options.encoder != Encoder::Software {
        let mut check = command("ffmpeg");
        check.args([
            "-f",
            "lavfi",
            "-i",
            "color=size=128x128:rate=30",
            "-frames:v",
            "2",
            "-c:v",
            plan.codec,
            "-f",
            "null",
            "-",
        ]);
        Process::run(&mut check, cancel)
            .context("Selected hardware encoder is unavailable on this machine; choose Software")?;
    }
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
    let silent = folder
        .path()
        .join(format!("video.{}", plan.container.extension()));
    let final_file = tempfile::NamedTempFile::new_in(parent)?;
    // Use a file: embedded LUTs are too large for OS command-line limits.
    let metadata = folder.path().join("preset.ffmeta");
    std::fs::write(&metadata, plan.metadata()?)?;
    let mut encoder = Process::spawn(&mut plan.encode(&silent), cancel)?;
    let input = encoder.stdin();
    let mut decoder = Process::spawn(
        &mut decode_command(video, Some(&plan.rate.text), 0., None),
        cancel,
    )?;
    drop(decoder.stdin());
    let decoded = decoder.stdout();
    progress(RenderProgress {
        fraction: 0.,
        stage: "Decoding and rendering video".into(),
    });
    let frames = match pipeline(
        decoded,
        input,
        video.size,
        cancel,
        |frame, count, started| {
            let result = render(frame, &plan.render)?;
            ensure!(
                result.dimensions() == plan.size,
                "Renderer returned the wrong video dimensions"
            );
            let fraction = (plan.duration(count) / video.duration).min(1.);
            let remaining = if fraction > 0. {
                started.elapsed().as_secs_f64() * (1. - fraction) / fraction
            } else {
                0.
            };
            progress(RenderProgress {
                fraction: (fraction * 0.9) as f32,
                stage: format!(
                    "Frame {count} · {:.1} FPS · approximately {:.0}s remaining",
                    count as f64 / started.elapsed().as_secs_f64().max(0.001),
                    remaining
                ),
            });
            Ok(result)
        },
    ) {
        Ok(frames) => frames,
        Err(Failure::Write(error)) => {
            // The encoder's own log says more than a broken pipe does.
            encoder.wait()?;
            return Err(error.into());
        }
        Err(Failure::Other(error)) => {
            decoder.kill();
            encoder.kill();
            return Err(error);
        }
    };
    decoder.wait()?;
    ensure!(frames > 0, "No frames decoded");
    progress(RenderProgress {
        fraction: 0.91,
        stage: "Finishing video encoding".into(),
    });
    encoder.wait()?;
    check_cancel(cancel)?;
    progress(RenderProgress {
        fraction: 0.95,
        stage: "Preserving audio and finalizing container".into(),
    });
    let mux = |copy_audio| {
        let mut mux = plan.mux(&silent, &metadata, final_file.path(), frames, copy_audio);
        Process::run(&mut mux, cancel)
    };
    let copy = plan.copies_audio();
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
    progress(RenderProgress {
        fraction: 1.,
        stage: format!(
            "Saved {frames} frames with {:.3}s duration",
            plan.duration(frames)
        ),
    });
    Ok(())
}

/// How a pipelined export stopped early.
enum Failure {
    /// Writing to the encoder failed, usually because it exited; its log has the reason.
    Write(std::io::Error),
    Other(anyhow::Error),
}

/// Frames in flight between two stages. Enough to absorb one stage's jitter; more would only
/// hold another full frame of memory each, which at 4K is 33 MB.
const QUEUED_FRAMES: usize = 2;

/// Decodes, renders and encodes at the same time instead of in turn: the decoder is read on
/// one thread and the encoder written on another, so the render loop only waits on them when
/// a queue between them runs empty or full. Order is kept -- each queue is first in, first
/// out -- so frames reach the encoder exactly as they left the decoder.
///
/// `render` gets each frame with its 1-based number and the time the pipeline started.
/// Returns the number of frames encoded. On `Failure::Other` the caller must kill both
/// processes, so a stage blocked on its pipe returns and the scope can end.
fn pipeline(
    mut decoded: impl Read + Send,
    mut encoded: impl Write + Send,
    (width, height): (u32, u32),
    cancel: &Arc<AtomicBool>,
    mut render: impl FnMut(&RgbaImage, u64, Instant) -> Result<RgbaImage>,
) -> std::result::Result<u64, Failure> {
    std::thread::scope(|scope| {
        let (frames_in, frames) = mpsc::sync_channel::<Result<RgbaImage>>(QUEUED_FRAMES);
        // Buffers go back to the decoder once rendered, so steady state allocates nothing.
        let (recycle, spare) = mpsc::channel::<RgbaImage>();
        scope.spawn(move || loop {
            let mut frame = spare
                .try_recv()
                .unwrap_or_else(|_| RgbaImage::new(width, height));
            let bytes = frame.as_mut();
            let read = match decoded.read(&mut bytes[..1]) {
                Ok(0) => break,
                Ok(_) => decoded
                    .read_exact(&mut bytes[1..])
                    .context("Truncated decoded video frame")
                    .map(|()| frame),
                Err(error) => Err(error.into()),
            };
            let failed = read.is_err();
            if frames_in.send(read).is_err() || failed {
                break;
            }
        });
        let (results_in, results) = mpsc::sync_channel::<RgbaImage>(QUEUED_FRAMES);
        let writer = scope.spawn(move || -> std::io::Result<()> {
            for frame in results {
                encoded.write_all(frame.as_raw())?;
            }
            // Dropping the pipe here is what tells the encoder the video has ended.
            Ok(())
        });
        let started = Instant::now();
        let mut count = 0u64;
        let rendered = (|| -> Result<bool> {
            for frame in frames.iter() {
                check_cancel(cancel)?;
                let frame = frame?;
                count += 1;
                let result = render(&frame, count, started)?;
                let _ = recycle.send(frame);
                check_cancel(cancel)?;
                if results_in.send(result).is_err() {
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        // Let the writer finish what is queued and close the pipe, then collect it.
        drop(results_in);
        drop(frames);
        let written = writer.join().expect("encoder writer panicked");
        match (rendered, written) {
            (Err(error), _) => Err(Failure::Other(error)),
            (Ok(_), Err(error)) => Err(Failure::Write(error)),
            (Ok(true), Ok(())) => Ok(count),
            (Ok(false), Ok(())) => unreachable!("the writer only stops early on an error"),
        }
    })
}

pub fn export(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(RenderProgress),
) -> Result<()> {
    let mut sequence = Sequence::default();
    export_with(
        video,
        output,
        config,
        options,
        cancel,
        |frame, config| renderer.render_frame(frame, config, &mut sequence, Some(cancel), |_| {}),
        progress,
    )
}

/// Stream a CFR preview, retaining a short history before the requested media time.
/// The callback provides backpressure; cancellation also interrupts decoder reads.
pub fn playback(
    video: &Video,
    start: f64,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    mut frame_ready: impl FnMut(f64, RgbaImage, RgbaImage) -> Result<()>,
) -> Result<()> {
    let rate = Rate::of(video, options);
    let preroll = (start - 0.2).max(0.);
    let mut decoder = Process::spawn(
        &mut decode_command(video, Some(&rate.text), preroll, None),
        cancel,
    )?;
    drop(decoder.stdin());
    let mut output = decoder.stdout();
    let mut input = RgbaImage::new(video.size.0, video.size.1);
    let c = render_config(config, options, rate.fps);
    let mut sequence = Sequence::default();
    let mut index = 0u64;
    loop {
        check_cancel(cancel)?;
        let bytes = input.as_mut();
        if output.read(&mut bytes[..1])? == 0 {
            break;
        }
        output
            .read_exact(&mut bytes[1..])
            .context("Truncated playback frame")?;
        let rendered = renderer.render_frame(&input, &c, &mut sequence, Some(cancel), |_| {})?;
        let time = preroll + index as f64 / rate.fps;
        index += 1;
        if time + 0.00001 >= start {
            frame_ready(time, input.clone(), rendered)?;
        }
    }
    decoder.wait()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `count` 2x2 frames, each filled with its own index.
    fn numbered_frames(count: u8) -> Vec<u8> {
        (0..count).flat_map(|i| [i; 16]).collect()
    }

    #[test]
    fn pipeline_keeps_frame_order_and_counts_what_it_encodes() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut encoded = Vec::new();
        let mut seen = vec![];
        let frames = pipeline(
            std::io::Cursor::new(numbered_frames(40)),
            &mut encoded,
            (2, 2),
            &cancel,
            |frame, count, _| {
                seen.push(count);
                // The stages overlap, so a slow render must not let frames overtake it.
                if count % 7 == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(frame.clone())
            },
        );
        assert!(matches!(frames, Ok(40)));
        assert_eq!(seen, (1..=40).collect::<Vec<_>>());
        assert_eq!(encoded, numbered_frames(40));
    }

    #[test]
    fn pipeline_reports_each_way_it_can_stop() {
        let cancel = Arc::new(AtomicBool::new(false));
        let identity = |frame: &RgbaImage, _, _| Ok(frame.clone());
        let mut truncated = numbered_frames(3);
        truncated.pop();
        match pipeline(
            std::io::Cursor::new(truncated),
            std::io::sink(),
            (2, 2),
            &cancel,
            identity,
        ) {
            Err(Failure::Other(error)) => assert!(error.to_string().contains("Truncated")),
            _ => panic!("a truncated frame must fail"),
        }
        match pipeline(
            std::io::Cursor::new(numbered_frames(5)),
            std::io::sink(),
            (2, 2),
            &cancel,
            |_, count, _| {
                ensure!(count < 3, "render failed");
                Ok(RgbaImage::new(2, 2))
            },
        ) {
            Err(Failure::Other(error)) => assert_eq!(error.to_string(), "render failed"),
            _ => panic!("a render error must stop the pipeline"),
        }
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        match pipeline(
            std::io::Cursor::new(numbered_frames(50)),
            Closed,
            (2, 2),
            &cancel,
            identity,
        ) {
            Err(Failure::Write(error)) => {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe)
            }
            _ => panic!("an encoder that stops reading must fail the export"),
        }
        cancel.store(true, Ordering::Relaxed);
        match pipeline(
            std::io::Cursor::new(numbered_frames(5)),
            std::io::sink(),
            (2, 2),
            &cancel,
            identity,
        ) {
            Err(Failure::Other(error)) => assert!(error.to_string().contains("cancel")),
            _ => panic!("cancellation must stop the pipeline"),
        }
    }

    #[test]
    fn export_overrides_validate_and_old_options_keep_profile_defaults() {
        let legacy: Options =
            serde_json::from_str(r#"{"timing":"stable","audio":"auto"}"#).unwrap();
        assert_eq!(legacy.effective_crf(Container::Mp4), 18);
        assert_eq!(legacy.effective_crf(Container::Webm), 24);
        assert!(legacy.validate().is_ok());
        let custom = Options {
            crf: Some(21),
            bitrate_mbps: Some(16),
            speed: Some(EncodingSpeed::Slow),
            ..Options::default()
        };
        assert_eq!(custom.effective_crf(Container::Webm), 21);
        assert_eq!(
            serde_json::from_str::<Options>(&serde_json::to_string(&custom).unwrap()).unwrap(),
            custom
        );
        assert!(Options {
            crf: Some(52),
            ..custom.clone()
        }
        .validate()
        .is_err());
        assert!(Options {
            bitrate_mbps: Some(0),
            ..custom
        }
        .validate()
        .is_err());
    }

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
