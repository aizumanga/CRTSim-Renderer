//! Bounded-memory FFmpeg decode -> ordered GPU frames -> encode -> audio mux.
mod animated;
mod animation;
mod decode;
mod export;
mod plan;
mod probe;
mod process;

pub use animated::AnimationFormat;
pub use animation::{AnimationOptions, AnimationSummary, Dither};
pub use decode::{playback, preview, preview_frame};
pub(crate) use export::Rate;
pub use export::{export, export_animation, export_animation_with, export_with, render_config};
pub use probe::{frame_count, probe, Source, Track, TrackKind, Video};

use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use process::Process;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

/// Files opened as video, by extension; FFmpeg decodes them.
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "m4v"];

/// How a file opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    /// A still image.
    Still,
    /// A video, by its extension. FFmpeg decodes it.
    Video,
    /// A GIF or WebP with more than one frame, decoded here.
    Animation(AnimationFormat),
}

impl MediaKind {
    /// Reads the file's header, and for a GIF up to two frames, to tell an animation from a
    /// still image of the same format.
    pub fn of(path: &Path) -> Self {
        let video = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        if video {
            Self::Video
        } else if let Some(format) = animated::detect(path) {
            Self::Animation(format)
        } else {
            Self::Still
        }
    }

    /// Whether it opens with frame navigation and playback, rather than as a still.
    pub fn is_moving(self) -> bool {
        self != Self::Still
    }

    /// Whether a batch renders it to a video rather than to a PNG. An animated WebP stays a
    /// PNG of its first frame, as batches made it before animations could be opened.
    pub fn batch_as_video(self) -> bool {
        matches!(self, Self::Video | Self::Animation(AnimationFormat::Gif))
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
    preset.config.validate_for(input)?;
    preset.video_options.validate()?;
    Ok(preset)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
