//! Video as a browser page handles it, without FFmpeg or threads: frames are read, rendered
//! and encoded one at a time on the page's only thread, each step awaited so the page keeps
//! drawing, and a file is made in memory for the page to download.
//!
//! Animated GIF and WebP are decoded here. Other videos are decoded by the host, which hands
//! their frames over through `FrameSource`.
use crate::{
    animated::{self, Planned},
    animation::AnimationPlan,
    check_cancel,
    decode::Request,
    export::Encoding,
    gif_writer::{GifWriter, Histogram},
    webp_writer::{self, WebpWriter},
    AnimationFormat, AnimationOptions, Options, Progress, Rate, Source, Video,
};
use anyhow::{bail, ensure, Result};
use crtsim_core::{config::Config, Renderer, Sequence};
use image::RgbaImage;
use std::sync::atomic::AtomicBool;
use web_time::Instant;

/// A video's frames in order, as a `Span` asks for them.
#[allow(async_fn_in_trait, reason = "a page's futures stay on its one thread")]
pub trait FrameSource {
    /// The next frame, or `None` after the last.
    async fn next(&mut self) -> Result<Option<RgbaImage>>;
}

/// Which frames of a video to read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Seconds into the video to start from.
    pub start: f64,
    /// A constant rate to take frames at, holding and dropping them as FFmpeg's `fps` filter
    /// does; or `None` for the one frame showing at `start`.
    pub fps: Option<f64>,
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

/// Frames of an animated GIF or WebP, decoded here.
pub struct Decoded(Planned);

impl FrameSource for Decoded {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        self.0.next().transpose()
    }
}

/// The frames of `video` that `span` asks for, when they can be decoded here: an animation's.
pub fn decoded(video: &Video, span: &Span) -> Result<Decoded> {
    let rate = span.fps.map(|fps| Rate {
        text: format!("{fps}"),
        fps,
    });
    let request = Request {
        rate: rate.as_ref(),
        start: span.start,
        frame: None,
        limit: span.limit,
    };
    planned(video, &request)
}

/// Frame `number` of `video`, counting from 0, when it can be decoded here.
pub fn frame(video: &Video, number: u64) -> Result<RgbaImage> {
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
    let Source::Animated { format, delays } = &video.source else {
        bail!("Only animations are decoded here");
    };
    let plan = animated::schedule(delays, request)?;
    Ok(Decoded(Planned::open(
        &animated::Origin::of(video),
        *format,
        plan,
    )?))
}

/// Plays `video` from `start` seconds: each frame rendered on `renderer` at the playback rate,
/// with a short warm-up before `start` so the CRT's history is already running, and handed to
/// `shown` with its time and source. `shown` sets the pace by awaiting room for the frame.
#[expect(
    clippy::too_many_arguments,
    reason = "each is one part the host supplies"
)]
pub async fn playback<S: FrameSource>(
    video: &Video,
    start: f64,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    mut open: impl AsyncFnMut(&Span) -> Result<S>,
    cancel: &AtomicBool,
    mut shown: impl AsyncFnMut(f64, RgbaImage, RgbaImage) -> Result<()>,
) -> Result<()> {
    let rate = Rate::of(video, options);
    let preroll = (start - 0.2).max(0.);
    let span = Span {
        start: preroll,
        fps: Some(rate.fps),
        limit: None,
    };
    let mut source = open(&span).await?;
    let mut sequence = Sequence::video(options.timing, rate.fps);
    let mut index = 0u64;
    while let Some(input) = source.next().await? {
        check_cancel(cancel)?;
        renderer
            .frame(&mut sequence, &input, config, Some(cancel), |_| {})
            .await?;
        let crt = renderer.read(&mut sequence).await?;
        let time = preroll + index as f64 / rate.fps;
        index += 1;
        if time + 0.00001 >= start {
            shown(time, input, crt).await?;
        }
    }
    Ok(())
}

/// Exports `video` as an animated GIF or WebP, returning the file.
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
    mut open: impl AsyncFnMut(&Span) -> Result<S>,
    mut render: impl AsyncFnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    mut lossy: impl AsyncFnMut(&RgbaImage, u8) -> Result<Vec<u8>>,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<Vec<u8>> {
    let plan = AnimationPlan::new(video, format, config, options)?;
    let frames = plan.frames();
    let span = Span {
        start: frames.start,
        fps: Some(frames.rate.fps),
        limit: frames.limit,
    };
    let expected = plan.frame_count();
    let (size, fps) = (frames.size, options.fps);
    // A GIF's first pass only counts colors; its second, and a WebP's only pass, write.
    let passes = match format {
        AnimationFormat::Gif => 2,
        AnimationFormat::Webp => 1,
    };
    let mut histogram = Histogram::default();
    let mut gif = None;
    let mut webp = None;
    let started = Instant::now();
    for pass in 0..passes {
        match format {
            AnimationFormat::Gif if pass == 1 => {
                gif = Some(GifWriter::new(
                    size,
                    fps,
                    histogram.palette(),
                    options.dither,
                )?);
            }
            AnimationFormat::Webp => webp = Some(WebpWriter::new(size, fps)?),
            AnimationFormat::Gif => {}
        }
        let mut source = open(&span).await?;
        let mut sequence = frames.sequence();
        let mut count = 0u64;
        while let Some(frame) = source.next().await? {
            check_cancel(cancel)?;
            let output = render(&mut sequence, &frame, frames.render).await?;
            ensure!(
                output.dimensions() == size,
                "Renderer returned the wrong animation dimensions"
            );
            match (&mut gif, &mut webp) {
                (Some(writer), _) => writer.frame(&output)?,
                (_, Some(writer)) => {
                    let change = writer.changed(&output)?;
                    let picture = match change {
                        None => None,
                        Some(rect) if options.lossless => {
                            Some(webp_writer::lossless(&rect.crop(&output))?)
                        }
                        Some(rect) => {
                            let picture = lossy(&rect.crop(&output), options.quality).await?;
                            ensure!(
                                picture.starts_with(b"RIFF"),
                                "This browser cannot write lossy WebP. Choose Lossless, or GIF."
                            );
                            Some(picture)
                        }
                    };
                    writer.add(&output, change.zip(picture.as_deref()))?;
                }
                (None, None) => histogram.add(&output),
            }
            count += 1;
            let done = (pass as f64 + count as f64 / expected as f64) / passes as f64;
            let remaining = started.elapsed().as_secs_f64() * (1. - done).max(0.) / done;
            let doing = match (format, pass) {
                (AnimationFormat::Gif, 0) => " · choosing colors",
                (AnimationFormat::Gif, _) => " · writing",
                _ => "",
            };
            progress(Progress {
                fraction: (done.min(1.) * 0.97) as f32,
                stage: format!(
                    "Frame {count} of {expected}{doing} · approximately {remaining:.0}s remaining"
                ),
            });
        }
        ensure!(count > 0, "No frames decoded");
    }
    match (gif, webp) {
        (Some(writer), _) => writer.finish(),
        (_, Some(writer)) => writer.finish(),
        (None, None) => unreachable!("every format has a writer"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            frame(&video, 2).unwrap().get_pixel(0, 0).0,
            [120, 120, 120, 255]
        );
        let span = Span {
            start: 0.05,
            fps: Some(20.),
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
            fps: None,
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
