//! What a video file holds, as ffprobe reports it.
use crate::{animated, check_cancel, command, process::Process, AnimationFormat, MediaKind};
use anyhow::{ensure, Context, Result};
use crtsim_core::config;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc},
};

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
    pub source: Source,
}

/// What decodes a video's frames.
#[derive(Clone, Debug)]
pub enum Source {
    /// FFmpeg, which also reads the file's other tracks.
    Ffmpeg,
    /// An animated GIF or WebP, decoded here. It has no other tracks. `delays` is how long
    /// each frame shows, in milliseconds.
    Animated {
        format: AnimationFormat,
        delays: std::sync::Arc<[u32]>,
    },
}

impl Video {
    /// The tracks that hold `kind`, in the file's order.
    pub fn tracks_of(&self, kind: TrackKind) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(move |track| track.kind == kind)
    }

    /// When decoded frame `frame`, counting from 0, starts, in seconds: exactly for an
    /// animation, whose frame times are known, and at the average rate for a video.
    pub fn frame_time(&self, frame: u64) -> f64 {
        match &self.source {
            Source::Animated { delays, .. } => {
                let shown = delays.iter().take(frame as usize);
                shown.map(|&delay| f64::from(delay)).sum::<f64>() / 1000.
            }
            Source::Ffmpeg => frame as f64 / self.fps,
        }
        .min(self.duration)
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
    if let MediaKind::Animation(format) = MediaKind::of(&path) {
        return animated::probe(path, format, cancel);
    }
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
        source: Source::Ffmpeg,
    })
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
}
