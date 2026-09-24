//! Exporting to animated GIF and WebP: a small CRT canvas at a modest rate, since both formats
//! grow quickly with size, rate and length.
//!
//! A GIF holds 256 colors, chosen for the whole animation. The frames are first encoded
//! losslessly to a temporary file, the palette is chosen from all of them, and then they are
//! mapped to it. FFmpeg can do both in one pass only by holding every frame in memory.
//! WebP is encoded directly.
use crate::{
    command,
    export::{require_encoder, Encoding, Frames, Step, Work},
    render_config, AnimationFormat, Rate, Timing, Video,
};
use anyhow::{ensure, Result};
use crtsim_core::config::Config;
use std::{
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

/// How a GIF spreads the colors its palette lacks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dither {
    /// A fixed pattern. It changes little from frame to frame, so it compresses well.
    #[default]
    Bayer,
    /// Error diffusion: smoother gradients, but the noise moves and the file grows.
    Diffusion,
    /// No dithering: the smallest files, with banding in gradients.
    None,
}

/// How an animation is exported.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationOptions {
    /// The longest side of the CRT canvas, in pixels. The CRT is rendered at this size: scaling
    /// it down afterwards would turn the mask and scanlines into moiré.
    pub max_side: u32,
    /// Frames per second. GIF stores each frame's time in hundredths of a second: other rates,
    /// such as 24, alternate between two delays that average to the rate.
    pub fps: u32,
    /// Stable or Disabled; NTSC timing needs 60 frames per second.
    pub timing: Timing,
    /// Where in the source to start, in seconds.
    pub start: f64,
    /// At most this many seconds, from `start`.
    pub max_seconds: Option<f64>,
    /// GIF only.
    pub dither: Dither,
    /// WebP only: 0–100, higher keeps more detail. Ignored when lossless.
    pub quality: u8,
    /// WebP only: keeps every pixel exactly, including the mask's colors, which lossy WebP
    /// blurs by storing color at half resolution.
    pub lossless: bool,
}

impl Default for AnimationOptions {
    fn default() -> Self {
        Self {
            max_side: 640,
            fps: 24,
            timing: Timing::Stable,
            start: 0.,
            max_seconds: Some(10.),
            dither: Dither::default(),
            quality: 75,
            lossless: false,
        }
    }
}

impl AnimationFormat {
    /// The fastest rate the format plays reliably: browsers slow GIF frames shorter than
    /// 2 hundredths to 10, so GIF stays well clear of that.
    pub fn max_fps(self) -> u32 {
        match self {
            Self::Gif => 30,
            Self::Webp => 60,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Gif => "GIF",
            Self::Webp => "WebP",
        }
    }

    fn encoder(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webp => "libwebp_anim",
        }
    }
}

impl AnimationOptions {
    pub fn validate(&self, format: AnimationFormat) -> Result<()> {
        ensure!(
            (64..=4096).contains(&self.max_side),
            "The longest side must be 64–4096 pixels"
        );
        let top = format.max_fps();
        ensure!(
            (1..=top).contains(&self.fps),
            "{} frame rate must be 1–{top} per second",
            format.name()
        );
        ensure!(
            self.timing != Timing::Ntsc60,
            "Animations use Stable or Disabled timing: NTSC timing needs 60 frames per second"
        );
        ensure!(
            self.start.is_finite() && self.start >= 0.,
            "The start must be a time in the video"
        );
        ensure!(
            self.max_seconds.is_none_or(|s| s.is_finite() && s > 0.),
            "The length limit must be more than zero seconds"
        );
        ensure!(self.quality <= 100, "WebP quality must be 0–100");
        Ok(())
    }

    /// What exporting `video` as `format` with these options makes, worked out without
    /// rendering anything.
    pub fn summary(
        &self,
        video: &Video,
        config: &Config,
        format: AnimationFormat,
    ) -> Result<AnimationSummary> {
        let plan = AnimationPlan::new(video, format, config, self)?;
        let frames = plan.frame_count();
        let pixels = frames as f64 * f64::from(plan.size.0) * f64::from(plan.size.1);
        let (low, high) = bytes_per_pixel(format, self);
        Ok(AnimationSummary {
            size: plan.size,
            frames,
            bytes: ((pixels * low) as u64, (pixels * high) as u64),
            // The GIF's lossless FFV1 intermediate; WebP is written directly.
            temporary: match format {
                AnimationFormat::Gif => (pixels * 2.) as u64,
                AnimationFormat::Webp => 0,
            },
        })
    }
}

/// How large an animation export will be.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationSummary {
    /// The CRT canvas, as rendered.
    pub size: (u32, u32),
    pub frames: u64,
    /// The file's likely size, lowest and highest, in bytes.
    pub bytes: (u64, u64),
    /// Space the export needs next to the destination while it runs, in bytes.
    pub temporary: u64,
}

/// Compressed bytes per rendered pixel of each frame, from lowest to highest. Measured on the
/// default CRT at 640x360 (the `animation_exports` integration test prints them): a still test
/// card gives the low end and busy moving test footage a little under half the high end, which
/// leaves room for noisier sources such as film grain.
fn bytes_per_pixel(format: AnimationFormat, options: &AnimationOptions) -> (f64, f64) {
    match format {
        AnimationFormat::Gif => match options.dither {
            Dither::None => (0.006, 0.2),
            Dither::Bayer => (0.008, 0.25),
            Dither::Diffusion => (0.008, 0.35),
        },
        AnimationFormat::Webp if options.lossless => (0.02, 0.5),
        AnimationFormat::Webp => (0.001, 0.07),
    }
}

/// An animation export, checked before anything starts.
pub(crate) struct AnimationPlan<'a> {
    video: &'a Video,
    options: &'a AnimationOptions,
    format: AnimationFormat,
    render: Config,
    size: (u32, u32),
    rate: Rate,
}

impl<'a> AnimationPlan<'a> {
    pub fn new(
        video: &'a Video,
        format: AnimationFormat,
        config: &Config,
        options: &'a AnimationOptions,
    ) -> Result<Self> {
        options.validate(format)?;
        ensure!(
            options.start < video.duration,
            "The start is after the end of the video"
        );
        let canvas = config.with_max_output_side(video.size, Some(options.max_side))?;
        let size = canvas.output_size(video.size)?;
        let rate = Rate {
            text: format!("{}/1", options.fps),
            fps: f64::from(options.fps),
        };
        Ok(Self {
            video,
            options,
            format,
            render: render_config(&canvas, options.timing, rate.fps),
            size,
            rate,
        })
    }

    fn frame_count(&self) -> u64 {
        (self.frames().length() * self.rate.fps - 1e-6)
            .ceil()
            .max(1.) as u64
    }

    /// The rendered frames in, as raw RGBA at the plan's rate.
    fn raw_input(&self, cmd: &mut Command) {
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
        ]);
    }
}

impl Encoding for AnimationPlan<'_> {
    fn frames(&self) -> Frames<'_> {
        Frames {
            video: self.video,
            render: &self.render,
            size: self.size,
            rate: &self.rate,
            start: self.options.start,
            limit: self.options.max_seconds,
        }
    }

    fn check(&self, cancel: &Arc<AtomicBool>) -> Result<()> {
        require_encoder(self.format.encoder(), cancel)
    }

    fn intermediate(&self) -> Option<&'static str> {
        match self.format {
            AnimationFormat::Gif => Some("mkv"),
            AnimationFormat::Webp => None,
        }
    }

    fn encoder(&self, encoded: &Path) -> Command {
        let mut cmd = command("ffmpeg");
        self.raw_input(&mut cmd);
        match self.format {
            // Lossless, so the palette is chosen from the frames exactly as rendered.
            AnimationFormat::Gif => {
                cmd.args(["-c:v", "ffv1", "-pix_fmt", "bgr0"]);
            }
            AnimationFormat::Webp => {
                let options = self.options;
                // Lossy WebP is YUV 4:2:0 in any case; lossless keeps RGB.
                let (lossless, pixels) = if options.lossless {
                    ("1", "bgra")
                } else {
                    ("0", "yuv420p")
                };
                cmd.args([
                    "-c:v",
                    "libwebp_anim",
                    "-lossless",
                    lossless,
                    "-quality",
                    &options.quality.to_string(),
                    "-pix_fmt",
                    pixels,
                    "-loop",
                    "0",
                    "-f",
                    "webp",
                ]);
            }
        }
        cmd.arg(encoded);
        cmd
    }

    fn metadata_file(&self) -> Result<Option<String>> {
        // Neither FFmpeg's GIF nor its WebP writer stores tags, so the preset is not kept.
        Ok(None)
    }

    fn finishing(&self, work: &Work, _frames: u64) -> Vec<Step> {
        if self.format != AnimationFormat::Gif {
            return vec![];
        }
        let palette = work.folder.join("palette.png");
        let mut choose = command("ffmpeg");
        choose
            .args(["-y", "-i"])
            .arg(work.encoded)
            .args(["-vf", "palettegen", "-frames:v", "1", "-update", "1"])
            .arg(&palette);
        let dither = match self.options.dither {
            Dither::Bayer => "bayer:bayer_scale=3",
            Dither::Diffusion => "sierra2_4a",
            Dither::None => "none",
        };
        let mut write = command("ffmpeg");
        write
            .args(["-y", "-i"])
            .arg(work.encoded)
            .arg("-i")
            .arg(&palette)
            .args([
                "-lavfi",
                // Only what changed from the previous frame is dithered again, so still areas
                // stay the same and the GIF encoder can skip them.
                &format!("[0:v][1:v]paletteuse=dither={dither}:diff_mode=rectangle"),
                "-loop",
                "0",
                "-f",
                "gif",
            ])
            .arg(work.destination);
        vec![
            Step::new("Choosing the GIF's 256 colors", choose),
            Step::new("Writing the GIF", write),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Source, Track, TrackKind};

    /// Every value given after `flag`.
    fn values(cmd: &Command, flag: &str) -> Vec<String> {
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        args.windows(2)
            .filter(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
            .collect()
    }

    fn video(duration: f64) -> Video {
        Video {
            metadata: Default::default(),
            tracks: vec![Track {
                index: 0,
                kind: TrackKind::Video,
                codec: "h264".into(),
                offset: 0.,
            }],
            start: 0.,
            path: "source.mkv".into(),
            size: (1920, 1080),
            fps: 30.,
            rate: "30/1".into(),
            duration,
            audio: false,
            audio_offset: 0.,
            hdr: false,
            stream: 0,
            frames: None,
            source: Source::Ffmpeg,
        }
    }

    fn config() -> Config {
        Config {
            output: "1920x1080".into(),
            ..Config::default()
        }
    }

    #[test]
    fn options_are_checked_for_the_format() {
        let options = AnimationOptions::default();
        assert!(options.validate(AnimationFormat::Gif).is_ok());
        let fast = AnimationOptions {
            fps: 50,
            ..options.clone()
        };
        assert!(fast.validate(AnimationFormat::Gif).is_err());
        assert!(fast.validate(AnimationFormat::Webp).is_ok());
        let ntsc = AnimationOptions {
            timing: Timing::Ntsc60,
            ..options.clone()
        };
        assert!(ntsc.validate(AnimationFormat::Webp).is_err());
        for bad in [
            AnimationOptions {
                max_side: 32,
                ..options.clone()
            },
            AnimationOptions {
                max_seconds: Some(0.),
                ..options.clone()
            },
            AnimationOptions {
                start: -1.,
                ..options.clone()
            },
        ] {
            assert!(bad.validate(AnimationFormat::Gif).is_err());
        }
    }

    #[test]
    fn the_crt_is_rendered_small_at_the_chosen_rate_for_the_chosen_span() {
        let source = video(60.);
        let options = AnimationOptions {
            start: 5.,
            ..AnimationOptions::default()
        };
        let summary = options
            .summary(&source, &config(), AnimationFormat::Gif)
            .unwrap();
        // 1920x1080 capped to 640 on the long side; odd sizes are fine for animations.
        assert_eq!(summary.size, (640, 360));
        // Ten seconds at 24 per second.
        assert_eq!(summary.frames, 240);
        assert!(summary.bytes.0 < summary.bytes.1);
        assert!(summary.temporary > summary.bytes.1);
        let odd = AnimationOptions {
            max_side: 333,
            max_seconds: None,
            ..options.clone()
        };
        let summary = odd
            .summary(&source, &config(), AnimationFormat::Webp)
            .unwrap();
        assert_eq!(summary.size, (333, 187));
        // The rest of the video after the start.
        assert_eq!(summary.frames, 55 * 24);
        assert_eq!(summary.temporary, 0);
        // Larger, longer or faster animations are estimated larger.
        let bigger = AnimationOptions {
            max_side: 1280,
            fps: 30,
            ..AnimationOptions::default()
        };
        let small = AnimationOptions::default()
            .summary(&source, &config(), AnimationFormat::Gif)
            .unwrap();
        let large = bigger
            .summary(&source, &config(), AnimationFormat::Gif)
            .unwrap();
        assert!(large.bytes.0 > small.bytes.0 * 4);
        let late = AnimationOptions {
            start: 60.,
            ..AnimationOptions::default()
        };
        assert!(late
            .summary(&source, &config(), AnimationFormat::Gif)
            .is_err());
    }

    #[test]
    fn a_gif_goes_through_a_lossless_file_and_a_palette_and_webp_is_written_directly() {
        let source = video(2.);
        let config = config();
        let options = AnimationOptions {
            dither: Dither::None,
            ..AnimationOptions::default()
        };
        let gif = AnimationPlan::new(&source, AnimationFormat::Gif, &config, &options).unwrap();
        assert_eq!(gif.intermediate(), Some("mkv"));
        let encoder = gif.encoder(Path::new("video.mkv"));
        assert_eq!(values(&encoder, "-c:v"), ["ffv1"]);
        assert_eq!(values(&encoder, "-r"), ["24/1"]);
        assert_eq!(values(&encoder, "-s"), ["640x360"]);
        let work = Work {
            encoded: Path::new("video.mkv"),
            metadata: Path::new("preset.ffmeta"),
            folder: Path::new("work"),
            destination: Path::new("out.tmp"),
        };
        let steps = gif.finishing(&work, 48);
        assert_eq!(steps.len(), 2);
        assert_eq!(values(&steps[0].commands[0], "-vf"), ["palettegen"]);
        assert_eq!(
            values(&steps[1].commands[0], "-lavfi"),
            ["[0:v][1:v]paletteuse=dither=none:diff_mode=rectangle"]
        );
        assert_eq!(values(&steps[1].commands[0], "-loop"), ["0"]);
        assert!(gif.metadata_file().unwrap().is_none());

        let lossless = AnimationOptions {
            lossless: true,
            fps: 60,
            ..AnimationOptions::default()
        };
        let webp = AnimationPlan::new(&source, AnimationFormat::Webp, &config, &lossless).unwrap();
        assert_eq!(webp.intermediate(), None);
        let encoder = webp.encoder(Path::new("out.tmp"));
        assert_eq!(values(&encoder, "-c:v"), ["libwebp_anim"]);
        assert_eq!(values(&encoder, "-lossless"), ["1"]);
        assert_eq!(values(&encoder, "-r"), ["60/1"]);
        assert!(webp.finishing(&work, 120).is_empty());
    }
}
