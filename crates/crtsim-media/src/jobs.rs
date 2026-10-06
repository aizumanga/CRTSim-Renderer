//! Video jobs: playing a video, and exporting it as a video or an animation. Each is one loop,
//! whatever the host: frames are read from a `FrameSource`, rendered, and handed on, each step
//! awaited, so a browser page keeps drawing on its only thread and the desktop drives the same
//! loop from a worker thread.
//!
//! Where frames come from is the source's: FFmpeg on the desktop (`decode`), the browser's
//! decoder on a page, and for animated GIF and WebP and the video test card, `animated`.
use crate::{
    animated::{self, Planned},
    animation::AnimationPlan,
    check_cancel,
    decode::{self, Request},
    export::{self, Encoding},
    made_here, mux, AnimationFormat, AnimationOptions, Container, Options, Progress, Rate, Source,
    Video,
};
use anyhow::{bail, ensure, Result};
use crtsim_core::{config::Config, Renderer, Sequence};
use image::RgbaImage;
use std::sync::{atomic::AtomicBool, Arc};
use web_time::Instant;

/// A video's frames in order, as a `Span` asks for them.
#[allow(async_fn_in_trait, reason = "a page's futures stay on its one thread")]
pub trait FrameSource {
    /// The next frame, or `None` after the last.
    async fn next(&mut self) -> Result<Option<RgbaImage>>;
}

/// Which frames of a video to read.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// Seconds into the video to start from.
    pub start: f64,
    /// A constant rate to take frames at, holding and dropping them as FFmpeg's `fps` filter
    /// does; or `None` for the one frame showing at `start`.
    pub rate: Option<Rate>,
    /// At most this many seconds.
    pub limit: Option<f64>,
}

/// Which of a video's frames, arriving in the order they show, a span at a constant rate
/// takes: for each tick, the last frame that starts less than half a tick after it, as the
/// animation schedule and FFmpeg's `fps` filter take them. Frames hold and drop but are
/// never blended.
#[derive(Clone, Debug)]
pub struct Ticks {
    start: f64,
    fps: f64,
    count: u64,
    next: u64,
}

impl Ticks {
    /// The ticks of `span`, which must have a rate, in a video lasting `duration` seconds.
    pub fn new(span: &Span, fps: f64, duration: f64) -> Self {
        let end = span
            .limit
            .map_or(duration, |limit| duration.min(span.start + limit));
        let count = ((end - span.start) * fps - 1e-6).ceil().max(1.) as u64;
        Self {
            start: span.start,
            fps,
            count,
            next: 0,
        }
    }

    /// How many of the ticks to come take the latest frame, now that the next one is known
    /// to start at `next_start`.
    pub fn before(&mut self, next_start: f64) -> u64 {
        let mut taken = 0;
        while self.next < self.count
            && self.start + (self.next as f64 + 0.5) / self.fps <= next_start + 1e-9
        {
            self.next += 1;
            taken += 1;
        }
        taken
    }

    /// How many ticks the last frame takes, which is all those left.
    pub fn rest(&mut self) -> u64 {
        let left = self.count - self.next;
        self.next = self.count;
        left
    }

    /// Whether every tick has its frame.
    pub fn done(&self) -> bool {
        self.next == self.count
    }
}

/// Frames of an animated GIF or WebP, decoded here, or of the video test card, drawn here.
pub struct Decoded(Planned);

impl FrameSource for Decoded {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        self.0.next().transpose()
    }
}

/// The frames of `video` that `span` asks for, when they can be made here: an animation's or
/// the video test card's.
pub fn decoded(video: &Video, span: &Span) -> Result<Decoded> {
    planned(video, &Request::of(span))
}

/// A video's frames wherever this crate decodes them: by FFmpeg or on a thread of their own on
/// the desktop, and on a page, an animation's here. A video file a browser was handed is
/// decoded by the browser, which the host does.
pub enum Frames {
    Piped(decode::Piped),
    Decoded(Decoded),
}

impl FrameSource for Frames {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        match self {
            Self::Piped(frames) => frames.next().await,
            Self::Decoded(frames) => frames.next().await,
        }
    }
}

/// The frames of `video` that `span` asks for.
pub fn frames(video: &Video, span: &Span, cancel: &Arc<AtomicBool>) -> Result<Frames> {
    match video.source {
        Source::Demuxed(_) => bail!("This video is decoded by the browser"),
        // A page has one thread, so an animation is decoded as its frames are asked for.
        _ if cfg!(target_arch = "wasm32") => Ok(Frames::Decoded(decoded(video, span)?)),
        _ => Ok(Frames::Piped(decode::Piped::open(video, span, cancel)?)),
    }
}

/// Frame `number` of `video`, counting from 0, wherever this crate decodes it, as `frames`
/// reads them: by FFmpeg or on a thread of its own on the desktop, and on a page here.
pub fn frame(video: &Video, number: u64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    match video.source {
        Source::Demuxed(_) => bail!("This video is decoded by the browser"),
        _ if cfg!(target_arch = "wasm32") => decoded_frame(video, number),
        _ => decode::preview_frame(video, number, cancel),
    }
}

/// Frame `number` of `video`, counting from 0, decoded here.
pub(crate) fn decoded_frame(video: &Video, number: u64) -> Result<RgbaImage> {
    let request = Request {
        rate: None,
        start: 0.,
        frame: Some(number),
        limit: None,
    };
    match planned(video, &request)?.0.next() {
        Some(frame) => frame,
        None => bail!("The animation has no frame {}", number + 1),
    }
}

fn planned(video: &Video, request: &Request) -> Result<Decoded> {
    Ok(Decoded(animated::Plan::new(video, request)?.open()?))
}

/// A frame rendered on `renderer`, as a job's `render` renders it on a host's GPU.
pub async fn render(
    renderer: &Renderer,
    sequence: &mut Sequence,
    frame: &RgbaImage,
    config: &Config,
    cancel: &AtomicBool,
) -> Result<RgbaImage> {
    renderer
        .frame(sequence, frame, config, Some(cancel), |_| {})
        .await?;
    renderer.read(sequence).await
}

/// Plays `video` from `start` seconds: each frame rendered by `render` at the playback rate,
/// with a short warm-up before `start` so the CRT's history is already running, and handed to
/// `shown` with its time and source. `shown` sets the pace by awaiting room for the frame. Each
/// frame renders with the settings `config` gives as it starts, so edits made while playing
/// show on the frames rendered after them.
#[expect(
    clippy::too_many_arguments,
    reason = "each is one part the host supplies"
)]
pub async fn playback<S: FrameSource>(
    video: &Video,
    start: f64,
    config: impl Fn() -> std::sync::Arc<Config>,
    options: &Options,
    mut open: impl AsyncFnMut(&Span) -> Result<S>,
    mut render: impl AsyncFnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    cancel: &AtomicBool,
    mut shown: impl AsyncFnMut(f64, RgbaImage, RgbaImage) -> Result<()>,
) -> Result<()> {
    let rate = Rate::of(video, options);
    let preroll = (start - 0.2).max(0.);
    let span = Span {
        start: preroll,
        rate: Some(rate.clone()),
        limit: None,
    };
    let mut source = open(&span).await?;
    let mut sequence = Sequence::video(options.timing, rate.fps);
    let mut index = 0u64;
    while let Some(input) = source.next().await? {
        check_cancel(cancel)?;
        let crt = render(&mut sequence, &input, &config()).await?;
        check_cancel(cancel)?;
        let time = preroll + index as f64 / rate.fps;
        index += 1;
        if time + 0.00001 >= start {
            shown(time, input, crt).await?;
        }
    }
    Ok(())
}

/// What a host encodes a video's frames with, such as a browser's WebCodecs.
#[allow(async_fn_in_trait, reason = "a page's futures stay on its one thread")]
pub trait VideoEncoding {
    /// Encodes the next frame, which shows at `time` seconds for `duration`.
    async fn encode(
        &mut self,
        frame: &RgbaImage,
        time: f64,
        duration: f64,
        key: bool,
    ) -> Result<()>;
    /// The encoded frames, once every one is done.
    async fn finish(self) -> Result<mux::EncodedVideo>;
}

/// How a host is asked to encode a video's frames.
#[derive(Clone, Debug, PartialEq)]
pub struct EncoderSettings {
    /// As WebCodecs names it.
    pub codec: &'static str,
    pub size: (u32, u32),
    pub fps: f64,
    /// Bits per second the encoder aims for.
    pub bitrate: u64,
}

impl EncoderSettings {
    /// The codec `container` is written with here, and a bit rate for `quality` that leaves
    /// the mask's fine pattern visible.
    pub fn new(
        container: Container,
        size: (u32, u32),
        fps: f64,
        quality: crate::Quality,
    ) -> Result<Self> {
        let codec = match container {
            // High profile, level 5.1: up to 4K.
            Container::Mp4 => "avc1.640033",
            Container::Webm => "vp09.00.51.08",
            Container::Mkv => bail!("MKV is written by FFmpeg"),
        };
        let bits_per_pixel = match quality {
            crate::Quality::Draft => 0.05,
            crate::Quality::Balanced => 0.1,
            crate::Quality::High => 0.16,
            crate::Quality::Archival => 0.3,
        } * if container == Container::Webm {
            0.75
        } else {
            1.
        };
        let pixels = f64::from(size.0) * f64::from(size.1);
        let bitrate = (pixels * fps * bits_per_pixel).max(500_000.) as u64;
        Ok(Self {
            codec,
            size,
            fps,
            bitrate,
        })
    }
}

/// Exports `video` as an MP4 or WebM, returning the file.
///
/// `open` and `render` are as for an animation. `encoder` makes the host's encoder for the
/// settings it is given. Sound the container holds as it is, is copied; otherwise, or when
/// the options ask for it, `reencode` turns it into Opus, which both containers hold.
#[expect(
    clippy::too_many_arguments,
    reason = "each is one part the host supplies"
)]
pub async fn export_video<S: FrameSource, E: VideoEncoding>(
    video: &Video,
    container: Container,
    config: &Config,
    options: &Options,
    open: impl AsyncFnMut(&Span) -> Result<S>,
    render: impl AsyncFnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    encoder: impl AsyncFnOnce(&EncoderSettings) -> Result<E>,
    reencode: impl AsyncFnOnce(&mux::EncodedAudio) -> Result<mux::EncodedAudio>,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<Vec<u8>> {
    let size = crate::plan::video_size(video, config, options)?;
    let rate = Rate::of(video, options);
    let settings = EncoderSettings::new(container, size, rate.fps, options.quality)?;
    let frames = export::Frames {
        video,
        render: config,
        timing: options.timing,
        size,
        rate: &rate,
        start: 0.,
        limit: None,
    };
    let encoder = encoder(&settings).await?;
    let output = made_here::Video::new(video, container, options, rate.fps, encoder, reencode);
    export(&frames, open, render, output, cancel, progress).await
}

/// Where an export's rendered frames go: an encoder, and what makes the file from them.
#[allow(async_fn_in_trait, reason = "a page's futures stay on its one thread")]
pub trait Output {
    /// What the export makes: a file saved, or a file's bytes for a page to download.
    type Made;

    /// How many times the frames are rendered and handed over. A GIF written here chooses its
    /// colors from every frame before it writes any, without holding them all.
    fn passes(&self) -> u32 {
        1
    }

    /// What pass `pass` does, for the progress shown: empty when there is one.
    fn pass_name(&self, _pass: u32) -> &'static str {
        ""
    }

    /// Takes the next rendered frame of pass `pass`, counting from 0.
    async fn frame(&mut self, pass: u32, frame: RgbaImage) -> Result<()>;

    /// Makes the file from the `count` frames handed over, reporting its own steps.
    async fn finish(self, count: u64, progress: &dyn Fn(Progress)) -> Result<Self::Made>;
}

/// Exports `frames` of a video: each read with `open`, rendered with `render` as the next
/// frame of the CRT's sequence, and handed to `output`, which then makes the file.
pub(crate) async fn export<S: FrameSource, O: Output>(
    frames: &export::Frames<'_>,
    mut open: impl AsyncFnMut(&Span) -> Result<S>,
    mut render: impl AsyncFnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    mut output: O,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<O::Made> {
    check_cancel(cancel)?;
    let span = frames.span();
    let expected = frames.count();
    let passes = output.passes();
    progress(Progress {
        fraction: 0.,
        stage: "Decoding and rendering video".into(),
    });
    let started = Instant::now();
    let mut count = 0;
    for pass in 0..passes {
        let mut source = open(&span).await?;
        let mut sequence = frames.sequence();
        count = 0;
        while let Some(frame) = source.next().await? {
            check_cancel(cancel)?;
            let rendered = render(&mut sequence, &frame, frames.render).await?;
            check_cancel(cancel)?;
            ensure!(
                rendered.dimensions() == frames.size,
                "Renderer returned the wrong dimensions"
            );
            output.frame(pass, rendered).await?;
            count += 1;
            let done =
                ((f64::from(pass) + count as f64 / expected as f64) / f64::from(passes)).min(1.);
            let elapsed = started.elapsed().as_secs_f64();
            progress(Progress {
                fraction: (done * 0.9) as f32,
                stage: format!(
                    "Frame {count} of {expected}{} · {:.1} FPS · approximately {:.0}s remaining",
                    output.pass_name(pass),
                    count as f64 / elapsed.max(0.001),
                    elapsed * (1. - done) / done
                ),
            });
        }
        ensure!(count > 0, "No frames decoded");
    }
    let made = output.finish(count, progress).await?;
    progress(Progress {
        fraction: 1.,
        stage: format!(
            "Saved {count} frames with {:.3}s duration",
            frames.duration(count)
        ),
    });
    Ok(made)
}

/// Exports `video` as an animated GIF or WebP without FFmpeg, returning the file.
///
/// `open` reads the frames a span asks for; `render` turns each into the next frame of the
/// CRT's sequence; `lossy` encodes a frame as a lossy still WebP at a quality from 0 to 100,
/// which the host does, since no encoder for it is written in Rust. A GIF's colors are chosen
/// from every frame before any is written, so its frames are read and rendered twice.
#[allow(
    clippy::too_many_arguments,
    reason = "each is one part the host supplies"
)]
pub async fn export_animation<S: FrameSource>(
    video: &Video,
    format: AnimationFormat,
    config: &Config,
    options: &AnimationOptions,
    open: impl AsyncFnMut(&Span) -> Result<S>,
    render: impl AsyncFnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    lossy: impl AsyncFnMut(&RgbaImage, u8) -> Result<Vec<u8>>,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<Vec<u8>> {
    let plan = AnimationPlan::new(video, format, config, options)?;
    let frames = plan.frames();
    let output = made_here::Animation::new(format, frames.size, options, lossy)?;
    export(&frames, open, render, output, cancel, progress).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{webp_writer, Audio};
    use crate::{Contents, Dither};
    use image::{codecs::gif::GifEncoder, AnimationDecoder, Delay, Frame, Rgba};
    use std::sync::Arc;

    /// An animated GIF of 4x4 frames lasting 100 ms each, frame `i` gray level `i * 60`,
    /// opened as a browser hands it over.
    fn clip(frames: u8) -> Video {
        let mut bytes = vec![];
        {
            let mut encoder = GifEncoder::new(&mut bytes);
            for i in 0..frames {
                let image = RgbaImage::from_pixel(4, 4, Rgba([i * 60, i * 60, i * 60, 255]));
                let delay = Delay::from_numer_denom_ms(100, 1);
                encoder
                    .encode_frame(Frame::from_parts(image, 0, 0, delay))
                    .unwrap();
            }
        }
        let contents = Contents(bytes.into());
        let name = std::path::PathBuf::from("clip.gif");
        assert_eq!(
            crate::detect_bytes(&name, &contents),
            Some(AnimationFormat::Gif)
        );
        let cancel = Arc::new(AtomicBool::new(false));
        crate::probe_bytes(name, contents, AnimationFormat::Gif, &cancel).unwrap()
    }

    /// Stands in for the CRT: the frame, filling the output size.
    async fn render(_: &mut Sequence, frame: &RgbaImage, config: &Config) -> Result<RgbaImage> {
        let (width, height) = config.output_size(frame.dimensions())?;
        Ok(RgbaImage::from_pixel(width, height, *frame.get_pixel(0, 0)))
    }

    fn export(
        video: &Video,
        format: AnimationFormat,
        options: &AnimationOptions,
    ) -> Result<Vec<u8>> {
        let config = Config {
            output: "64x64".into(),
            ..Config::default()
        };
        let cancel = AtomicBool::new(false);
        pollster::block_on(export_animation(
            video,
            format,
            &config,
            options,
            async |span| decoded(video, span),
            async |sequence, frame, config| render(sequence, frame, config).await,
            // A host whose lossy WebP is, here, the lossless one.
            async |frame, _| webp_writer::lossless(frame),
            &cancel,
            &|_| {},
        ))
    }

    #[test]
    fn frames_decode_from_bytes_one_at_a_time() {
        let video = clip(3);
        assert_eq!((video.size, video.frames), ((4, 4), Some(3)));
        assert_eq!(
            decoded_frame(&video, 2).unwrap().get_pixel(0, 0).0,
            [120, 120, 120, 255]
        );
        let span = Span {
            start: 0.05,
            rate: Some(Rate::per_second(20.)),
            limit: None,
        };
        let mut source = decoded(&video, &span).unwrap();
        let mut levels = vec![];
        while let Some(frame) = pollster::block_on(source.next()).unwrap() {
            levels.push(frame.get_pixel(0, 0)[0]);
        }
        // From 50 ms at 20 per second: each 100 ms frame shows twice, the first from its middle.
        assert_eq!(levels, vec![0, 60, 60, 120, 120]);
    }

    /// Frames of one gray level after another, as a decoder would hand them over.
    #[test]
    fn playback_warms_up_before_its_start_and_shows_the_frames_from_there() {
        let video = clip(3);
        let cancel = AtomicBool::new(false);
        let mut shown = vec![];
        let start = 0.35;
        pollster::block_on(playback(
            &video,
            start,
            || std::sync::Arc::new(Config::general()),
            &Options::default(),
            async |span| {
                assert_eq!(span.start, start - 0.2, "a warm-up before the start");
                Ok(Grays(0, 4))
            },
            async |_, frame, _| Ok(frame.clone()),
            &cancel,
            async |time, source, crt| {
                assert_eq!(source, crt);
                shown.push((time, source.get_pixel(0, 0).0[0]));
                Ok(())
            },
        ))
        .unwrap();
        let fps = video.fps;
        let expected: Vec<_> = (0..4u8)
            .map(|i| (start - 0.2 + f64::from(i) / fps, i + 1))
            .filter(|(time, _)| time + 1e-5 >= start)
            .collect();
        assert!(!expected.is_empty() && expected.len() < 4, "{expected:?}");
        assert_eq!(shown, expected);
        // Cancelled while a frame renders: it is not shown.
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let played = pollster::block_on(playback(
            &video,
            0.,
            || std::sync::Arc::new(Config::general()),
            &Options::default(),
            async |_| Ok(Grays(0, 4)),
            async |_, frame, _| Ok(frame.clone()),
            &cancel,
            async |_, _, _| panic!("nothing is shown once cancelled"),
        ));
        assert!(played.is_err());
    }

    /// An output that keeps what it is handed: each frame's gray level, by pass.
    struct Kept(u32, Vec<(u32, u8)>);

    impl Output for Kept {
        type Made = Vec<(u32, u8)>;
        fn passes(&self) -> u32 {
            self.0
        }
        async fn frame(&mut self, pass: u32, frame: RgbaImage) -> Result<()> {
            self.1.push((pass, frame.get_pixel(0, 0).0[0]));
            Ok(())
        }
        async fn finish(self, count: u64, _: &dyn Fn(Progress)) -> Result<Self::Made> {
            assert_eq!(count, 3);
            Ok(self.1)
        }
    }

    #[test]
    fn an_export_renders_each_pass_in_order_and_stops_on_a_failure_or_cancel() {
        let video = clip(3);
        let config = Config::general();
        let rate = Rate::per_second(10.);
        let frames = export::Frames {
            video: &video,
            render: &config,
            timing: crate::Timing::Stable,
            size: (8, 6),
            rate: &rate,
            start: 0.,
            limit: None,
        };
        let cancel = AtomicBool::new(false);
        let last = std::cell::Cell::new(0.);
        let made = pollster::block_on(super::export(
            &frames,
            async |_| Ok(Grays(0, 3)),
            async |_, frame, _| Ok(frame.clone()),
            Kept(2, vec![]),
            &cancel,
            &|progress| last.set(progress.fraction),
        ))
        .unwrap();
        assert_eq!(made, [(0, 1), (0, 2), (0, 3), (1, 1), (1, 2), (1, 3)]);
        assert_eq!(last.get(), 1.);
        let failed = pollster::block_on(super::export(
            &frames,
            async |_| Ok(Grays(0, 3)),
            async |_, _, _| anyhow::bail!("render failed"),
            Kept(1, vec![]),
            &cancel,
            &|_| {},
        ));
        assert_eq!(failed.unwrap_err().to_string(), "render failed");
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let cancelled = pollster::block_on(super::export(
            &frames,
            async |_| Ok(Grays(0, 3)),
            async |_, frame, _| Ok(frame.clone()),
            Kept(1, vec![]),
            &cancel,
            &|_| {},
        ));
        assert!(cancelled.unwrap_err().to_string().contains("cancel"));
    }

    struct Grays(u8, u8);

    impl FrameSource for Grays {
        async fn next(&mut self) -> Result<Option<RgbaImage>> {
            if self.0 == self.1 {
                return Ok(None);
            }
            self.0 += 1;
            Ok(Some(RgbaImage::from_pixel(
                8,
                6,
                Rgba([self.0, self.0, self.0, 255]),
            )))
        }
    }

    /// Stands in for WebCodecs: each frame becomes a packet holding its gray level.
    #[derive(Default)]
    struct Encoder(Vec<mux::Packet>, (u32, u32));

    impl VideoEncoding for Encoder {
        async fn encode(
            &mut self,
            frame: &RgbaImage,
            time: f64,
            duration: f64,
            key: bool,
        ) -> Result<()> {
            let data = vec![frame.get_pixel(0, 0)[0]; 4];
            self.0.push(mux::Packet {
                data,
                time,
                duration,
                key,
            });
            Ok(())
        }

        async fn finish(self) -> Result<mux::EncodedVideo> {
            Ok(mux::EncodedVideo {
                codec: "vp09.00.51.08".into(),
                description: None,
                size: self.1,
                packets: self.0,
            })
        }
    }

    fn fixture_video(name: &str) -> Video {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        crate::probe_demuxed(name.into(), Contents(std::fs::read(path).unwrap().into())).unwrap()
    }

    /// Exports `video` through the stand-ins, returning the file read back and whether the
    /// sound was handed over to be converted.
    fn export_video_as(
        video: &Video,
        container: Container,
        audio: Audio,
    ) -> (crate::demux::Demuxed, bool) {
        let config = Config {
            output: "64x48".into(),
            ..Config::default()
        };
        let options = Options {
            audio,
            ..Options::default()
        };
        let frames = video.frames.unwrap() as u8;
        let converted = std::cell::Cell::new(false);
        let file = pollster::block_on(export_video(
            video,
            container,
            &config,
            &options,
            async |_| Ok(Grays(0, frames)),
            async |sequence, frame, config| render(sequence, frame, config).await,
            async |settings| {
                assert_eq!(settings.size, (64, 48));
                Ok(Encoder(vec![], settings.size))
            },
            async |audio| {
                converted.set(true);
                // Stands in for Opus: the packets as they are, under Opus's name.
                Ok(mux::EncodedAudio {
                    codec: "opus".into(),
                    description: Some(b"OpusHead\x01\x01\x38\x01\x80\xbb\0\0\0\0\0".to_vec()),
                    ..audio.clone()
                })
            },
            &AtomicBool::new(false),
            &|_| {},
        ))
        .unwrap();
        let name = format!("out.{}", container.extension());
        (
            crate::demux::demux(std::path::Path::new(&name), &file).unwrap(),
            converted.get(),
        )
    }

    #[test]
    fn videos_export_through_the_hosts_encoder_with_their_sound() {
        // VP9 and Opus into WebM: the sound is copied.
        let video = fixture_video("vp9-opus.webm");
        let (webm, converted) = export_video_as(&video, Container::Webm, Audio::Auto);
        assert!(!converted);
        let samples = &webm.video.samples;
        assert_eq!(samples.len(), 10);
        let times: Vec<u32> = samples
            .iter()
            .map(|s| (s.time * 1000.).round() as u32)
            .collect();
        assert_eq!(times, (0..10).map(|i| i * 100).collect::<Vec<_>>());
        // A keyframe every two seconds: only the first, in one second.
        assert_eq!(samples.iter().filter(|s| s.key).count(), 1);
        let source = video.source.clone();
        let Source::Demuxed(demuxed) = source else {
            unreachable!()
        };
        assert_eq!(
            webm.audio.unwrap().samples.len(),
            demuxed.audio.as_ref().unwrap().samples.len()
        );
        // Asked for, the sound is converted even where it could be copied.
        assert!(export_video_as(&video, Container::Mp4, Audio::Encode).1);
        // AAC cannot go into WebM, so it is converted; and without sound, nothing is.
        let aac = fixture_video("h264-aac.mp4");
        let (webm, converted) = export_video_as(&aac, Container::Webm, Audio::Auto);
        assert!(converted);
        assert_eq!(webm.audio.unwrap().codec, "opus");
        let (silent, converted) = export_video_as(&aac, Container::Webm, Audio::Mute);
        assert!(!converted && silent.audio.is_none());
    }

    #[test]
    fn ticks_take_the_frames_the_animation_schedule_takes() {
        // Frames of 100, 200 and 100 ms, as the schedule's own test has them.
        let starts = [0., 0.1, 0.3];
        let taken = |span: Span, fps: f64| {
            let mut ticks = Ticks::new(&span, fps, 0.4);
            let mut shown = vec![];
            for (frame, next) in starts.iter().skip(1).enumerate() {
                shown.extend(std::iter::repeat_n(frame, ticks.before(*next) as usize));
            }
            shown.extend(std::iter::repeat_n(starts.len() - 1, ticks.rest() as usize));
            assert!(ticks.done());
            shown
        };
        let from = |start| Span {
            start,
            rate: None,
            limit: None,
        };
        assert_eq!(taken(from(0.), 10.), vec![0, 1, 1, 2]);
        assert_eq!(taken(from(0.), 5.), vec![0, 1]);
        assert_eq!(taken(from(0.1), 10.), vec![1, 1, 2]);
        let limited = Span {
            limit: Some(0.2),
            ..from(0.1)
        };
        assert_eq!(taken(limited, 10.), vec![1, 1]);
    }

    #[test]
    fn animations_export_without_ffmpeg() {
        let video = clip(3);
        let options = AnimationOptions {
            fps: 10,
            dither: Dither::Bayer,
            ..AnimationOptions::default()
        };
        let gif = export(&video, AnimationFormat::Gif, &options).unwrap();
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&gif)).unwrap();
        let frames: Vec<_> = decoder.into_frames().map(Result::unwrap).collect();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].buffer().dimensions(), (64, 64));
        let levels: Vec<u8> = frames
            .iter()
            .map(|f| f.buffer().get_pixel(9, 9)[0])
            .collect();
        assert!(
            levels
                .iter()
                .zip([0, 60, 120])
                .all(|(&got, want)| got.abs_diff(want) <= 4),
            "{levels:?}"
        );
        for lossless in [true, false] {
            let options = AnimationOptions {
                lossless,
                ..options.clone()
            };
            let webp = export(&video, AnimationFormat::Webp, &options).unwrap();
            let decoder = image_webp::WebPDecoder::new(std::io::Cursor::new(&webp)).unwrap();
            assert_eq!((decoder.dimensions(), decoder.num_frames()), ((64, 64), 3));
        }
    }

    #[test]
    fn a_host_without_lossy_webp_is_told_what_to_choose() {
        let video = clip(2);
        let config = Config::default();
        let cancel = AtomicBool::new(false);
        let error = pollster::block_on(export_animation(
            &video,
            AnimationFormat::Webp,
            &config,
            &AnimationOptions::default(),
            async |span| decoded(&video, span),
            async |sequence, frame, config| render(sequence, frame, config).await,
            async |_, _| Ok(b"\x89PNG".to_vec()),
            &cancel,
            &|_| {},
        ))
        .unwrap_err();
        assert!(error.to_string().contains("Lossless"), "{error}");
    }
}
