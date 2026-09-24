//! What an export tells FFmpeg, worked out before any process starts, so it can be checked
//! without FFmpeg installed.
use crate::{
    command,
    export::{require_encoder, Encoding, Frames, Step, Work},
    process::Process,
    render_config, Audio, Container, Encoder, EncodingSpeed, Options, Preset, Quality, Rate,
    Source, TrackKind, Video, PRESET_PREFIX,
};
use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use std::{
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

/// Largest metadata an export writes, in bytes.
const METADATA_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) struct ExportPlan<'a> {
    pub video: &'a Video,
    pub options: &'a Options,
    /// The settings as chosen, which the file records.
    pub config: &'a Config,
    /// The same settings adjusted for the timing, which frames are rendered with.
    pub render: Config,
    pub container: Container,
    pub codec: &'static str,
    pub size: (u32, u32),
    pub rate: Rate,
}

impl<'a> ExportPlan<'a> {
    /// Checks that `video` can be exported to `output` with these settings.
    pub fn new(
        video: &'a Video,
        output: &Path,
        config: &'a Config,
        options: &'a Options,
    ) -> Result<Self> {
        options.validate()?;
        config.validate()?;
        config.signal_size(video.size)?;
        let size = config.output_size(video.size)?;
        ensure!(
            size.0 % 2 == 0 && size.1 % 2 == 0,
            "Video output width and height must be even (for example 1920×1080)"
        );
        let container = Container::of(output).context("Choose an MP4, MKV or WebM filename")?;
        let codec = options.encoder.codec(container)?;
        let rate = Rate::of(video, options);
        Ok(Self {
            video,
            options,
            config,
            render: render_config(config, options.timing, rate.fps),
            container,
            codec,
            size,
            rate,
        })
    }

    /// How long `frames` frames last.
    pub fn duration(&self, frames: u64) -> f64 {
        frames as f64 / self.rate.fps
    }

    /// The file's metadata, in FFmpeg's format: the source's tags when other tracks are kept,
    /// and the settings as a comment an import can read back.
    pub fn metadata(&self) -> Result<String> {
        // Store the user's original controls plus timing, not the decay-adjusted config:
        // reimporting the latter would apply the timing correction a second time.
        let preset = format!(
            "{PRESET_PREFIX}{}",
            serde_json::to_string(&Preset {
                version: 1,
                config: self.config.clone(),
                video_options: self.options.clone(),
            })?
        );
        ensure!(
            preset.len() <= METADATA_LIMIT,
            "Video preset metadata exceeds 16 MB"
        );
        let mut tags = String::from(";FFMETADATA1\n");
        if self.options.preserve_streams {
            for (key, value) in &self.video.metadata {
                // Re-exporting our own output must not recursively embed old LUT presets.
                if key.eq_ignore_ascii_case("comment") && value.starts_with(PRESET_PREFIX) {
                    continue;
                }
                let key = if key.eq_ignore_ascii_case("comment") {
                    "source_comment"
                } else {
                    key
                };
                tags.push_str(&format!("{}={}\n", escape(key), escape(value)));
            }
        }
        tags.push_str(&format!("comment={}\n", escape(&preset)));
        ensure!(
            tags.len() <= METADATA_LIMIT,
            "Combined source and preset metadata exceeds 16 MB"
        );
        Ok(tags)
    }

    /// The encoder: rendered frames in as raw RGBA, one compressed video stream out to
    /// `silent`.
    pub fn encode(&self, silent: &Path) -> Command {
        let mut cmd = command("ffmpeg");
        cmd.args([
            "-y",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-s",
            &format!("{}x{}", self.size.0, self.size.1),
            "-r",
            &self.rate.text,
            "-i",
            "pipe:0",
            "-an",
            "-c:v",
            self.codec,
        ]);
        self.rate_control(&mut cmd);
        // Renderer bytes are full-range RGB with BT.709/sRGB primaries. Make the RGB-to-YUV
        // matrix and legal range explicit, then tag the encoded stream for consistent playback.
        cmd.args([
            "-vf",
            "scale=in_range=full:out_range=tv:in_color_matrix=bt709:out_color_matrix=bt709,\
             format=yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
            "-color_range",
            "tv",
        ])
        .arg(silent);
        cmd
    }

    /// How the encoder trades size for quality: a target bitrate for hardware, a constant
    /// quality for software.
    fn rate_control(&self, cmd: &mut Command) {
        let options = self.options;
        if options.encoder != Encoder::Software {
            let kbps = (self.bitrate_mbps() * 1000.).round() as u32;
            cmd.args(["-b:v", &format!("{kbps}k")]);
            return;
        }
        let crf = options.effective_crf(self.container).to_string();
        if self.container == Container::Webm {
            let cpu_used = match options.speed {
                Some(EncodingSpeed::Fast) => "8",
                Some(EncodingSpeed::Slow) => "1",
                _ => "4",
            };
            cmd.args([
                "-crf",
                &crf,
                "-b:v",
                "0",
                "-deadline",
                "good",
                "-cpu-used",
                cpu_used,
            ]);
        } else {
            let preset = match options.speed {
                Some(EncodingSpeed::Fast) => "veryfast",
                Some(EncodingSpeed::Balanced) => "medium",
                Some(EncodingSpeed::Slow) => "slow",
                None if options.quality == Quality::Draft => "veryfast",
                None => "medium",
            };
            cmd.args(["-crf", &crf, "-preset", preset]);
        }
    }

    /// A hardware encoder's target: the one asked for, or else the quality's rate for 1080p at
    /// 30 frames per second, scaled to this size and rate.
    fn bitrate_mbps(&self) -> f64 {
        let mbps = match self.options.quality {
            Quality::Draft => 6,
            Quality::Balanced => 12,
            Quality::High => 24,
            Quality::Archival => 40,
        };
        let (width, height) = self.size;
        let automatic = (mbps as f64
            * (width as f64 * height as f64 / (1920. * 1080.))
            * (self.rate.fps / 30.))
            .clamp(2., 200.);
        self.options
            .bitrate_mbps
            .map(f64::from)
            .unwrap_or(automatic)
    }

    /// Whether to try copying the audio as it is before re-encoding it: only when asked to keep
    /// it where possible, and when no track has to be moved to line up with the video.
    pub fn copies_audio(&self) -> bool {
        self.options.audio == Audio::Auto
            && self
                .video
                .tracks_of(TrackKind::Audio)
                .all(|track| track.offset.abs() < 0.002)
    }

    /// The muxer: the encoded video from `silent`, the source's audio and other tracks as the
    /// options keep them and the metadata file, cut to the `frames` encoded and written to
    /// `destination`. `copy_audio` keeps the audio as it is, which not every container
    /// accepts; otherwise it is re-encoded.
    pub fn mux(
        &self,
        silent: &Path,
        metadata: &Path,
        destination: &Path,
        frames: u64,
        copy_audio: bool,
    ) -> Command {
        let (video, options, container) = (self.video, self.options, self.container);
        // An animation decoded here has no other tracks, and FFmpeg may not read it at all.
        let source = matches!(video.source, Source::Ffmpeg);
        let mut cmd = command("ffmpeg");
        cmd.args(["-y", "-copyts", "-i"]).arg(silent);
        if source {
            cmd.args(["-itsoffset", &(-video.start).to_string(), "-i"])
                .arg(&video.path);
        }
        cmd.args(["-f", "ffmetadata", "-i"]).arg(metadata);
        cmd.args(["-map", "0:v:0", "-c:v", "copy"]);
        if video.audio && options.audio != Audio::Mute {
            let all = options.preserve_streams;
            cmd.args(["-map", if all { "1:a?" } else { "1:a:0" }]);
            if copy_audio {
                cmd.args(["-c:a", "copy"]);
            } else {
                cmd.args(["-c:a", container.audio_codec(), "-b:a", "192k"]);
                let kept = if all { usize::MAX } else { 1 };
                for (index, track) in video.tracks_of(TrackKind::Audio).take(kept).enumerate() {
                    let filter = if track.offset >= 0. {
                        format!(
                            "asetpts=PTS-STARTPTS,adelay={}:all=1",
                            (track.offset * 1000.).round()
                        )
                    } else {
                        format!("atrim=start={},asetpts=PTS-STARTPTS", -track.offset)
                    };
                    cmd.args([&format!("-filter:a:{index}"), &filter]);
                }
            }
        }
        if options.preserve_streams && source {
            let subtitles = video
                .tracks_of(TrackKind::Subtitle)
                .filter_map(|track| Some((track, container.subtitle_codec(&track.codec)?)));
            for (index, (track, codec)) in subtitles.enumerate() {
                cmd.args([
                    "-map",
                    &format!("1:{}", track.index),
                    &format!("-c:s:{index}"),
                    codec,
                ]);
            }
            if container == Container::Mkv {
                cmd.args(["-map", "1:t?", "-c:t", "copy"]);
            }
            cmd.args(["-map_chapters", "1"]);
        } else {
            cmd.args(["-map_chapters", "-1"]);
        }
        cmd.args([
            "-t",
            &self.duration(frames).to_string(),
            "-map_metadata",
            if source { "2" } else { "1" },
        ]);
        if container == Container::Mp4 {
            cmd.args(["-movflags", "+faststart+use_metadata_tags"]);
        }
        cmd.args(["-f", container.format()]).arg(destination);
        cmd
    }
}

impl Encoding for ExportPlan<'_> {
    fn frames(&self) -> Frames<'_> {
        Frames {
            video: self.video,
            render: &self.render,
            size: self.size,
            rate: &self.rate,
            start: 0.,
            limit: None,
        }
    }

    fn check(&self, cancel: &Arc<AtomicBool>) -> Result<()> {
        require_encoder(self.codec, cancel)?;
        if self.options.encoder != Encoder::Software {
            let mut check = command("ffmpeg");
            check.args([
                "-f",
                "lavfi",
                "-i",
                "color=size=128x128:rate=30",
                "-frames:v",
                "2",
                "-c:v",
                self.codec,
                "-f",
                "null",
                "-",
            ]);
            Process::run(&mut check, cancel).context(
                "Selected hardware encoder is unavailable on this machine; choose Software",
            )?;
        }
        Ok(())
    }

    fn intermediate(&self) -> Option<&'static str> {
        Some(self.container.extension())
    }

    fn encoder(&self, encoded: &Path) -> Command {
        self.encode(encoded)
    }

    fn metadata_file(&self) -> Result<Option<String>> {
        self.metadata().map(Some)
    }

    /// Muxes the source's audio and other tracks back in, copying the audio as it is when it
    /// can and re-encoding it when copying fails.
    fn finishing(&self, work: &Work, frames: u64) -> Vec<Step> {
        let mux = |copy_audio| {
            self.mux(
                work.encoded,
                work.metadata,
                work.destination,
                frames,
                copy_audio,
            )
        };
        let commands = if self.copies_audio() {
            vec![mux(true), mux(false)]
        } else {
            vec![mux(false)]
        };
        vec![Step {
            stage: "Preserving audio and finalizing container",
            commands,
            failed: "Audio remux and re-encoding both failed",
        }]
    }
}

/// A key or value as FFmpeg's metadata format needs it written.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('=', "\\=")
        .replace(';', "\\;")
        .replace('#', "\\#")
        .replace('\n', "\\\n")
        .replace('\r', "")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Timing, Track};

    /// A 64×48 source at NTSC's rate holding these tracks, each a kind, a codec and how far it
    /// starts after the video, numbered from 1 after the video's own.
    fn video(tracks: &[(TrackKind, &str, f64)]) -> Video {
        Video {
            metadata: Default::default(),
            tracks: tracks
                .iter()
                .enumerate()
                .map(|(index, &(kind, codec, offset))| Track {
                    index: index as u64 + 1,
                    kind,
                    codec: codec.into(),
                    offset,
                })
                .collect(),
            start: 0.,
            path: "source.mkv".into(),
            size: (64, 48),
            fps: 30000. / 1001.,
            rate: "30000/1001".into(),
            duration: 1.,
            audio: tracks.iter().any(|track| track.0 == TrackKind::Audio),
            audio_offset: 0.,
            hdr: false,
            stream: 0,
            frames: None,
            source: Source::Ffmpeg,
        }
    }

    fn config() -> Config {
        Config {
            output: "64x48".into(),
            ..Config::default()
        }
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// Every value given after `flag`.
    fn values(args: &[String], flag: &str) -> Vec<String> {
        args.windows(2)
            .filter(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
            .collect()
    }

    fn mux(plan: &ExportPlan, copy_audio: bool) -> Vec<String> {
        args(&plan.mux(
            Path::new("video.tmp"),
            Path::new("preset.ffmeta"),
            Path::new("out.tmp"),
            30,
            copy_audio,
        ))
    }

    const TRACKS: &[(TrackKind, &str, f64)] = &[
        (TrackKind::Audio, "pcm_s16le", 0.),
        (TrackKind::Audio, "aac", 0.12),
        (TrackKind::Subtitle, "subrip", 0.),
        (TrackKind::Subtitle, "hdmv_pgs_subtitle", 0.),
        (TrackKind::Attachment, "ttf", 0.),
        (TrackKind::Data, "bin_data", 0.),
    ];

    #[test]
    fn each_container_keeps_the_tracks_it_can_hold() {
        let source = video(TRACKS);
        let options = Options::default();
        let config = config();
        let plan = |name: &str| ExportPlan::new(&source, Path::new(name), &config, &options);

        let mkv = mux(&plan("out.MKV").unwrap(), false);
        assert_eq!(values(&mkv, "-f").last().unwrap(), "matroska");
        assert_eq!(values(&mkv, "-c:a"), ["aac"]);
        // Every subtitle is copied, and attachments come along.
        assert_eq!(
            values(&mkv, "-map"),
            ["0:v:0", "1:a?", "1:3", "1:4", "1:t?"]
        );
        assert_eq!(values(&mkv, "-c:s:0"), ["copy"]);
        assert_eq!(values(&mkv, "-c:s:1"), ["copy"]);
        assert!(!mkv.contains(&"-movflags".to_string()));

        let mp4 = mux(&plan("out.mp4").unwrap(), false);
        assert_eq!(values(&mp4, "-f").last().unwrap(), "mp4");
        assert_eq!(values(&mp4, "-movflags"), ["+faststart+use_metadata_tags"]);
        // Only the text subtitle fits, converted; no attachments.
        assert_eq!(values(&mp4, "-map"), ["0:v:0", "1:a?", "1:3"]);
        assert_eq!(values(&mp4, "-c:s:0"), ["mov_text"]);

        let webm = mux(&plan("out.webm").unwrap(), false);
        assert_eq!(values(&webm, "-f").last().unwrap(), "webm");
        assert_eq!(values(&webm, "-c:a"), ["libopus"]);
        assert_eq!(values(&webm, "-c:s:0"), ["webvtt"]);

        let notes = |container| crate::preservation_notes(&source, container);
        assert_eq!(notes(Container::Mkv), ["Data track 6 is not copied."]);
        assert_eq!(
            notes(Container::Mp4),
            [
                "Subtitle 4 (hdmv_pgs_subtitle) cannot be stored in mp4; use MKV to preserve it.",
                "Attachment 5 is preserved only in MKV.",
                "Data track 6 is not copied.",
            ]
        );
    }

    #[test]
    fn an_animation_is_muxed_without_reading_its_file_again() {
        let mut source = video(&[]);
        source.source = Source::Animated {
            format: crate::AnimationFormat::Gif,
            delays: [100, 100].into(),
        };
        let config = config();
        let options = Options::default();
        let plan = ExportPlan::new(&source, Path::new("out.mkv"), &config, &options).unwrap();
        let args = mux(&plan, true);
        assert_eq!(values(&args, "-i"), ["video.tmp", "preset.ffmeta"]);
        assert_eq!(values(&args, "-map"), ["0:v:0"]);
        assert_eq!(values(&args, "-map_metadata"), ["1"]);
        assert_eq!(values(&args, "-map_chapters"), ["-1"]);
    }

    #[test]
    fn audio_is_moved_to_line_up_and_copied_only_when_it_already_does() {
        let config = config();
        let options = Options::default();
        let late = video(&[
            (TrackKind::Audio, "aac", 0.),
            (TrackKind::Audio, "aac", 0.12),
            (TrackKind::Audio, "aac", -0.05),
        ]);
        let plan = ExportPlan::new(&late, Path::new("out.mp4"), &config, &options).unwrap();
        assert!(!plan.copies_audio());
        let args = mux(&plan, false);
        assert_eq!(
            values(&args, "-filter:a:0"),
            ["asetpts=PTS-STARTPTS,adelay=0:all=1"]
        );
        assert_eq!(
            values(&args, "-filter:a:1"),
            ["asetpts=PTS-STARTPTS,adelay=120:all=1"]
        );
        assert_eq!(
            values(&args, "-filter:a:2"),
            ["atrim=start=0.05,asetpts=PTS-STARTPTS"]
        );

        let aligned = video(&[(TrackKind::Audio, "aac", 0.001)]);
        let plan = ExportPlan::new(&aligned, Path::new("out.mp4"), &config, &options).unwrap();
        assert!(plan.copies_audio());
        let args = mux(&plan, true);
        assert_eq!(values(&args, "-c:a"), ["copy"]);
        assert!(values(&args, "-filter:a:0").is_empty());
        let encode = Options {
            audio: Audio::Encode,
            ..Options::default()
        };
        let plan = ExportPlan::new(&aligned, Path::new("out.mp4"), &config, &encode).unwrap();
        assert!(!plan.copies_audio());
    }

    #[test]
    fn muted_or_trimmed_exports_map_only_what_they_keep() {
        let source = video(TRACKS);
        let config = config();
        let mute = Options {
            audio: Audio::Mute,
            ..Options::default()
        };
        let plan = ExportPlan::new(&source, Path::new("out.mkv"), &config, &mute).unwrap();
        assert!(!mux(&plan, false).iter().any(|arg| arg.starts_with("1:a")));

        let trimmed = Options {
            preserve_streams: false,
            ..Options::default()
        };
        let plan = ExportPlan::new(&source, Path::new("out.mkv"), &config, &trimmed).unwrap();
        let args = mux(&plan, false);
        assert_eq!(values(&args, "-map"), ["0:v:0", "1:a:0"]);
        assert_eq!(values(&args, "-map_chapters"), ["-1"]);
        assert_eq!(values(&args, "-filter:a:0").len(), 1);
        assert!(values(&args, "-filter:a:1").is_empty());
        // Cut to the 30 frames encoded, at 30000/1001.
        let duration: f64 = values(&args, "-t")[0].parse().unwrap();
        assert!((duration - 1.001).abs() < 1e-9);
    }

    #[test]
    fn rate_control_follows_quality_speed_and_encoder() {
        let source = video(&[]);
        let rate_control = |output: &str, size: &str, options: Options| {
            let config = Config {
                output: size.into(),
                ..Config::default()
            };
            let plan = ExportPlan::new(&source, Path::new(output), &config, &options).unwrap();
            let args = args(&plan.encode(Path::new("video.tmp")));
            ["-crf", "-preset", "-cpu-used", "-b:v"]
                .map(|flag| values(&args, flag).join(" "))
                .to_vec()
        };
        let draft = Options {
            quality: Quality::Draft,
            ..Options::default()
        };
        assert_eq!(
            rate_control("a.mp4", "64x48", draft),
            ["26", "veryfast", "", ""]
        );
        let slow = Options {
            speed: Some(EncodingSpeed::Slow),
            ..Options::default()
        };
        assert_eq!(rate_control("a.webm", "64x48", slow), ["24", "", "1", "0"]);
        // Hardware: the profile's 1080p30 rate, scaled to the size and rate and kept in range.
        let hardware = |quality, bitrate_mbps| Options {
            encoder: Encoder::Nvenc,
            quality,
            bitrate_mbps,
            ..Options::default()
        };
        let b_v = |size, options| rate_control("a.mp4", size, options)[3].clone();
        assert_eq!(
            b_v("1920x1080", hardware(Quality::Balanced, None)),
            "11988k"
        );
        assert_eq!(
            b_v("3840x2160", hardware(Quality::Archival, None)),
            "159840k"
        );
        assert_eq!(b_v("64x48", hardware(Quality::High, None)), "2000k");
        assert_eq!(b_v("64x48", hardware(Quality::High, Some(16))), "16000k");
    }

    #[test]
    fn exports_that_cannot_be_encoded_are_refused_before_starting() {
        let source = video(&[]);
        let options = Options::default();
        let refused = |output: &str, size: &str, options: &Options| {
            let config = Config {
                output: size.into(),
                ..Config::default()
            };
            ExportPlan::new(&source, Path::new(output), &config, options)
                .err()
                .map(|error| error.to_string())
        };
        assert!(refused("a.mp4", "64x48", &options).is_none());
        assert!(refused("a.mp4", "65x48", &options)
            .unwrap()
            .contains("even"));
        assert!(refused("a.mov", "64x48", &options)
            .unwrap()
            .contains("MP4, MKV or WebM"));
        let hardware = Options {
            encoder: Encoder::Qsv,
            ..Options::default()
        };
        assert!(refused("a.webm", "64x48", &hardware)
            .unwrap()
            .contains("VP9"));
    }

    #[test]
    fn ntsc_timing_encodes_at_60_and_the_file_keeps_the_settings_as_chosen() {
        let source = video(&[]);
        let config = config();
        let options = Options {
            timing: Timing::Ntsc60,
            ..Options::default()
        };
        let plan = ExportPlan::new(&source, Path::new("a.mkv"), &config, &options).unwrap();
        assert_eq!(values(&args(&plan.encode(Path::new("v"))), "-r"), ["60/1"]);
        assert_eq!(plan.duration(30), 0.5);
        let stable = Options::default();
        let plan = ExportPlan::new(&source, Path::new("a.mkv"), &config, &stable).unwrap();
        assert_eq!(
            values(&args(&plan.encode(Path::new("v"))), "-r"),
            ["30000/1001"]
        );
        // Rendering uses decay adjusted to the rate; the file records the settings chosen.
        assert_ne!(plan.render.persistence, config.persistence);
        let metadata = plan.metadata().unwrap();
        let comment = metadata.lines().last().unwrap().strip_prefix("comment=");
        let tags = serde_json::json!({"format": {"tags": {"comment": unescape(comment.unwrap())}}});
        let preset = crate::parse_preset(&tags, source.size).unwrap();
        assert_eq!(preset.config, config);
        assert_eq!(preset.video_options, stable);
    }

    /// Metadata as FFmpeg reads it back: a backslash keeps the character after it as it is.
    fn unescape(text: &str) -> String {
        let mut chars = text.chars();
        let mut plain = String::new();
        while let Some(c) = chars.next() {
            plain.push(if c == '\\' {
                chars.next().unwrap_or(c)
            } else {
                c
            });
        }
        plain
    }

    #[test]
    fn metadata_keeps_the_source_tags_escaped_but_not_an_older_preset() {
        let mut source = video(&[]);
        source.metadata = [
            ("title", "A=B; C#D\\E\r\nF"),
            ("COMMENT", "Keep this comment"),
            ("comment", "CRTSim-Renderer-Preset:{\"old\":true}"),
        ]
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .into();
        let config = config();
        let options = Options::default();
        let plan = ExportPlan::new(&source, Path::new("a.mkv"), &config, &options).unwrap();
        let metadata = plan.metadata().unwrap();
        let lines: Vec<&str> = metadata.lines().collect();
        assert_eq!(
            lines[..3],
            [
                ";FFMETADATA1",
                "source_comment=Keep this comment",
                "title=A\\=B\\; C\\#D\\\\E\\"
            ]
        );
        assert_eq!(lines[3], "F");
        assert!(lines[4].starts_with("comment=CRTSim-Renderer-Preset:"));
        assert_eq!(lines.len(), 5);

        let trimmed = Options {
            preserve_streams: false,
            ..Options::default()
        };
        let plan = ExportPlan::new(&source, Path::new("a.mkv"), &config, &trimmed).unwrap();
        assert_eq!(plan.metadata().unwrap().lines().count(), 2);
    }
}
