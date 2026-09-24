//! Animated GIF and WebP, decoded here rather than by FFmpeg, so they open without it.
//!
//! Each frame comes out composited onto the whole canvas, with how long it shows. Frames are
//! read in order and one at a time, since each one builds on the canvas the last one left.
use crate::{
    check_cancel,
    decode::Request,
    probe::{Source, Track, TrackKind, Video},
};
use anyhow::{ensure, Context, Result};
use crtsim_core::config;
use image::{codecs::gif::GifDecoder, AnimationDecoder, ImageDecoder, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
};

/// Most memory one decoder may allocate, the same budget as a still image.
const MEMORY_LIMIT: usize = 512 * 1024 * 1024;
/// More frames than any animation worth rendering; a bound on the delays kept per file.
const MAX_FRAMES: usize = 100_000;

/// The animated image formats: opened as animations, and written by the animation export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimationFormat {
    Gif,
    Webp,
}

impl AnimationFormat {
    pub const ALL: [Self; 2] = [Self::Gif, Self::Webp];

    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }

    /// The format a file name asks for, by its extension.
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Self::ALL
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }
}

/// The format of `path` if it is a GIF or WebP with more than one frame. Reads the header, and
/// for a GIF at most two frames.
pub(crate) fn detect(path: &Path) -> Option<AnimationFormat> {
    let format = AnimationFormat::of(path)?;
    let file = BufReader::new(File::open(path).ok()?);
    let animated = match format {
        AnimationFormat::Gif => {
            let frames = GifDecoder::new(file).ok()?.into_frames();
            frames.take(2).filter(Result::is_ok).count() == 2
        }
        AnimationFormat::Webp => image_webp::WebPDecoder::new(file)
            .is_ok_and(|decoder| decoder.is_animated() && decoder.num_frames() > 1),
    };
    animated.then_some(format)
}

/// How long a frame with this delay shows, in milliseconds. Browsers show frames that ask for
/// 10 ms or less for 100 ms instead, and animations are made to look right in them.
fn shown_ms(delay: u32) -> u32 {
    if delay <= 10 {
        100
    } else {
        delay
    }
}

/// The frames of one animation, in order, each with how long it shows.
enum Frames {
    Gif(image::Frames<'static>),
    Webp {
        decoder: Box<image_webp::WebPDecoder<BufReader<File>>>,
        /// One frame as the decoder writes it: RGB, or RGBA when the file has alpha.
        buffer: Vec<u8>,
    },
}

impl Frames {
    /// Opens `path`, returning its canvas size and its frames.
    fn open(path: &Path, format: AnimationFormat) -> Result<((u32, u32), Self)> {
        let file = BufReader::new(File::open(path).context("Cannot open animation")?);
        match format {
            AnimationFormat::Gif => {
                let mut decoder = GifDecoder::new(file).context("Cannot read GIF")?;
                let mut limits = image::io::Limits::default();
                limits.max_alloc = Some(MEMORY_LIMIT as u64);
                decoder.set_limits(limits)?;
                Ok((decoder.dimensions(), Self::Gif(decoder.into_frames())))
            }
            AnimationFormat::Webp => {
                let mut decoder = image_webp::WebPDecoder::new(file).context("Cannot read WebP")?;
                ensure!(decoder.is_animated(), "This WebP is not animated");
                decoder.set_memory_limit(MEMORY_LIMIT);
                // Browsers start from, and clear to, a transparent canvas, not the file's hint.
                decoder.set_background_color([0; 4])?;
                let size = decoder
                    .output_buffer_size()
                    .context("WebP frames are too large")?;
                Ok((
                    decoder.dimensions(),
                    Self::Webp {
                        decoder: Box::new(decoder),
                        buffer: vec![0; size],
                    },
                ))
            }
        }
    }
}

impl Iterator for Frames {
    /// A frame and how long it shows, in milliseconds.
    type Item = Result<(RgbaImage, u32)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Gif(frames) => Some(frames.next()?.map_err(Into::into).map(|frame| {
                let (numerator, denominator) = frame.delay().numer_denom_ms();
                let delay = numerator.checked_div(denominator).unwrap_or(0);
                (frame.into_buffer(), shown_ms(delay))
            })),
            Self::Webp { decoder, buffer } => {
                let delay = match decoder.read_frame(buffer) {
                    Err(image_webp::DecodingError::NoMoreFrames) => return None,
                    Err(error) => return Some(Err(error.into())),
                    Ok(delay) => delay,
                };
                let (width, height) = decoder.dimensions();
                let pixels = if decoder.has_alpha() {
                    buffer.clone()
                } else {
                    let (rgb, _) = buffer.as_chunks::<3>();
                    rgb.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect()
                };
                Some(
                    RgbaImage::from_raw(width, height, pixels)
                        .context("Invalid WebP frame")
                        .map(|image| (image, shown_ms(delay))),
                )
            }
        }
    }
}

/// Reads an animation's size and frame timing, decoding each frame once.
pub(crate) fn probe(
    path: PathBuf,
    format: AnimationFormat,
    cancel: &Arc<AtomicBool>,
) -> Result<Video> {
    let (size, frames) = Frames::open(&path, format)?;
    config::validate_size(size)?;
    let mut delays = vec![];
    for frame in frames {
        check_cancel(cancel)?;
        delays.push(frame?.1);
        ensure!(delays.len() <= MAX_FRAMES, "Animation has too many frames");
    }
    ensure!(!delays.is_empty(), "Animation contains no frames");
    let total: u64 = delays.iter().map(|&delay| u64::from(delay)).sum();
    let duration = total as f64 / 1000.;
    ensure!(
        duration <= 7. * 24. * 3600.,
        "Unsupported animation duration"
    );
    let (fps, rate) = average_rate(delays.len() as u64, total);
    Ok(Video {
        metadata: Default::default(),
        tracks: vec![Track {
            index: 0,
            kind: TrackKind::Video,
            codec: format.extension().into(),
            offset: 0.,
        }],
        start: 0.,
        path,
        size,
        fps,
        rate,
        duration,
        audio: false,
        audio_offset: 0.,
        hdr: false,
        stream: 0,
        frames: Some(delays.len() as u64),
        source: Source::Animated {
            format,
            delays: delays.into(),
        },
    })
}

/// The rate `frames` frames lasting `total_ms` average, exactly and as a number, kept within
/// the 1–240 per second videos are held to. Mirrors ffprobe's average frame rate.
fn average_rate(frames: u64, total_ms: u64) -> (f64, String) {
    let (mut numerator, mut denominator) = (frames * 1000, total_ms.max(1));
    let divisor = gcd(numerator, denominator);
    (numerator, denominator) = (numerator / divisor, denominator / divisor);
    let fps = numerator as f64 / denominator as f64;
    if fps < 1. {
        (1., "1/1".into())
    } else if fps > 240. {
        (240., "240/1".into())
    } else {
        (fps, format!("{numerator}/{denominator}"))
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Which source frame each decoded frame is, in order: one frame by its number, the frame
/// showing at `request.start`, or a constant-rate sequence from there. A constant rate takes,
/// for each of its ticks, the last frame that starts less than half a tick after it: FFmpeg's
/// `fps=round=near` rounds each start to the nearest tick, halves up. Frames hold or drop but
/// are never blended.
pub(crate) fn schedule(delays: &[u32], request: &Request) -> Result<Vec<usize>> {
    if let Some(frame) = request.frame {
        ensure!(
            (frame as usize) < delays.len(),
            "The animation has no frame {}",
            frame + 1
        );
        return Ok(vec![frame as usize]);
    }
    let mut starts = Vec::with_capacity(delays.len());
    let mut end = 0.;
    for &delay in delays {
        starts.push(end);
        end += f64::from(delay) / 1000.;
    }
    // The last frame starting before `time`, or also at it when `at` holds.
    let last = |time: f64, at: bool| {
        let starts_by = |&start: &f64| start < time - 1e-9 || (at && start <= time + 1e-9);
        starts.partition_point(starts_by).max(1) - 1
    };
    ensure!(
        request.start.is_finite() && request.start >= 0. && request.start < end,
        "Preview time is outside the animation"
    );
    let Some(rate) = request.rate else {
        return Ok(vec![last(request.start, true)]);
    };
    let end = request
        .limit
        .map_or(end, |limit| end.min(request.start + limit));
    let ticks = ((end - request.start) * rate.fps - 1e-6).ceil().max(1.) as usize;
    Ok((0..ticks)
        .map(|tick| last(request.start + (tick as f64 + 0.5) / rate.fps, false))
        .collect())
}

/// A decode running on its own thread, where the frames are decoded: GIF frames cannot move
/// between threads. It stops when asked, when cancelled or when its reader is dropped.
pub(crate) struct Decoding {
    thread: Option<JoinHandle<Result<()>>>,
    stop: Arc<AtomicBool>,
}

impl Decoding {
    /// Starts decoding the frames `plan` lists and returns the decode with a reader of them,
    /// as packed RGBA at the canvas size. At most `queued` frames wait to be read.
    pub(crate) fn start(
        path: &Path,
        format: AnimationFormat,
        plan: Vec<usize>,
        queued: usize,
        cancel: &Arc<AtomicBool>,
    ) -> (Self, impl Read + Send) {
        let (send, frames) = mpsc::sync_channel::<Vec<u8>>(queued);
        let stop = Arc::new(AtomicBool::new(false));
        let (path, cancel, stopped) = (path.to_owned(), cancel.clone(), stop.clone());
        let thread = std::thread::spawn(move || -> Result<()> {
            let (_, frames) = Frames::open(&path, format)?;
            let mut next = 0;
            for (index, frame) in frames.enumerate() {
                if next == plan.len() || stopped.load(Ordering::Relaxed) {
                    return Ok(());
                }
                check_cancel(&cancel)?;
                let (frame, _) = frame?;
                while plan.get(next) == Some(&index) {
                    if send.send(frame.as_raw().clone()).is_err() {
                        // The reader is gone: nothing wants the rest.
                        return Ok(());
                    }
                    next += 1;
                }
            }
            ensure!(next == plan.len(), "The animation ended early");
            Ok(())
        });
        let reader = Received {
            frames,
            current: vec![],
            read: 0,
        };
        (
            Self {
                thread: Some(thread),
                stop,
            },
            reader,
        )
    }

    pub(crate) fn kill(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Waits for the decode to end, with its error if it failed.
    pub(crate) fn wait(&mut self) -> Result<()> {
        match self.thread.take() {
            Some(thread) => thread.join().expect("animation decoder panicked"),
            None => Ok(()),
        }
    }
}

impl Drop for Decoding {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The decoded frames as one byte stream, like the raw video FFmpeg writes. It ends when the
/// decode does.
struct Received {
    frames: mpsc::Receiver<Vec<u8>>,
    current: Vec<u8>,
    read: usize,
}

impl Read for Received {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.read == self.current.len() {
            match self.frames.recv() {
                Ok(frame) => (self.current, self.read) = (frame, 0),
                Err(_) => return Ok(0),
            }
        }
        let count = buffer.len().min(self.current.len() - self.read);
        buffer[..count].copy_from_slice(&self.current[self.read..self.read + count]);
        self.read += count;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rate;
    use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba};

    fn rate(fps: u32) -> Rate {
        Rate {
            text: format!("{fps}/1"),
            fps: f64::from(fps),
        }
    }

    fn request(rate: Option<&Rate>, start: f64, frame: Option<u64>) -> Request<'_> {
        Request {
            rate,
            start,
            frame,
            limit: None,
        }
    }

    /// A GIF of `delays.len()` 3x2 frames, frame `i` filled with gray level `i * 10`.
    fn write_gif(path: &Path, delays_ms: &[u32]) {
        let mut encoder = GifEncoder::new(File::create(path).unwrap());
        for (i, &delay) in delays_ms.iter().enumerate() {
            let level = (i * 10 % 256) as u8;
            let image = RgbaImage::from_pixel(3, 2, Rgba([level, level, level, 255]));
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                ))
                .unwrap();
        }
    }

    #[test]
    fn short_delays_show_as_browsers_show_them() {
        assert_eq!(shown_ms(0), 100);
        assert_eq!(shown_ms(10), 100);
        assert_eq!(shown_ms(20), 20);
        assert_eq!(average_rate(10, 1000), (10., "10/1".into()));
        assert_eq!(average_rate(24, 1000), (24., "24/1".into()));
        assert_eq!(average_rate(3, 100), (30., "30/1".into()));
        assert_eq!(average_rate(1, 5000), (1., "1/1".into()));
    }

    #[test]
    fn a_constant_rate_holds_and_drops_frames_by_their_start() {
        // Frames of 100, 200 and 100 ms at 10 per second: the long one shows twice.
        let delays = [100, 200, 100];
        let ten = rate(10);
        let plan = schedule(&delays, &request(Some(&ten), 0., None)).unwrap();
        assert_eq!(plan, vec![0, 1, 1, 2]);
        // At 5 per second, the second frame starts exactly between two ticks and rounds up
        // to the later one, as FFmpeg rounds it; the third is dropped.
        let five = rate(5);
        assert_eq!(
            schedule(&delays, &request(Some(&five), 0., None)).unwrap(),
            vec![0, 1]
        );
        // From a later start, and cut short by a limit.
        assert_eq!(
            schedule(&delays, &request(Some(&ten), 0.1, None)).unwrap(),
            vec![1, 1, 2]
        );
        let limited = Request {
            limit: Some(0.2),
            ..request(Some(&ten), 0.1, None)
        };
        assert_eq!(schedule(&delays, &limited).unwrap(), vec![1, 1]);
        // One frame by time, or by number.
        assert_eq!(
            schedule(&delays, &request(None, 0.25, None)).unwrap(),
            vec![1]
        );
        assert_eq!(
            schedule(&delays, &request(None, 0., Some(2))).unwrap(),
            vec![2]
        );
        assert!(schedule(&delays, &request(None, 0., Some(3))).is_err());
        assert!(schedule(&delays, &request(None, 0.4, None)).is_err());
    }

    #[test]
    fn gif_frames_probe_and_decode_without_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.gif");
        write_gif(&path, &[100, 100, 200, 0]);
        assert_eq!(detect(&path), Some(AnimationFormat::Gif));
        let cancel = Arc::new(AtomicBool::new(false));
        let video = probe(path.clone(), AnimationFormat::Gif, &cancel).unwrap();
        assert_eq!(video.size, (3, 2));
        assert_eq!(video.frames, Some(4));
        // The last frame asks for no delay and shows for 100 ms.
        assert!((video.duration - 0.5).abs() < 1e-9);
        assert_eq!(video.rate, "8/1");
        let Source::Animated { delays, .. } = &video.source else {
            panic!("a GIF is decoded here");
        };
        let ten = rate(10);
        let plan = schedule(delays, &request(Some(&ten), 0., None)).unwrap();
        assert_eq!(plan, vec![0, 1, 2, 2, 3]);
        let (mut decoding, mut frames) =
            Decoding::start(&path, AnimationFormat::Gif, plan, 2, &cancel);
        let mut bytes = vec![];
        frames.read_to_end(&mut bytes).unwrap();
        decoding.wait().unwrap();
        let (frames, _) = bytes.as_chunks::<{ 3 * 2 * 4 }>();
        let levels: Vec<u8> = frames.iter().map(|frame| frame[0]).collect();
        assert_eq!(levels, vec![0, 10, 20, 20, 30]);

        // A one-frame GIF is a still image.
        let still = dir.path().join("still.gif");
        write_gif(&still, &[100]);
        assert_eq!(detect(&still), None);
    }

    #[test]
    fn webp_frames_probe_and_decode_without_ffmpeg() {
        // Four lossless 4x2 frames of 100 ms, gray levels 0, 40, 80 and 120, from libwebp.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/four-frames.webp");
        assert_eq!(detect(&path), Some(AnimationFormat::Webp));
        let cancel = Arc::new(AtomicBool::new(false));
        let video = probe(path.clone(), AnimationFormat::Webp, &cancel).unwrap();
        assert_eq!((video.size, video.frames), ((4, 2), Some(4)));
        assert!((video.duration - 0.4).abs() < 1e-9);
        assert_eq!(video.rate, "10/1");
        let plan = schedule(&[100; 4], &request(None, 0., Some(2))).unwrap();
        let (mut decoding, mut frames) =
            Decoding::start(&path, AnimationFormat::Webp, plan, 2, &cancel);
        let mut bytes = vec![];
        frames.read_to_end(&mut bytes).unwrap();
        decoding.wait().unwrap();
        assert_eq!(bytes.len(), 4 * 2 * 4);
        // image-webp 0.2.4 alpha-blends opaque pixels too, and its integer blend truncates
        // them one level darker; libwebp skips them and reads 80. One level in 255 is not
        // visible, so this is accepted rather than worked around.
        assert!(matches!(bytes[0], 79 | 80), "{:?}", &bytes[..4]);
        assert_eq!(bytes[3], 255);
    }

    #[test]
    fn files_open_as_before_except_animations_and_batches_keep_webp_a_still() {
        use crate::MediaKind;
        let dir = tempfile::tempdir().unwrap();
        let (clip, still) = (dir.path().join("clip.GIF"), dir.path().join("still.gif"));
        write_gif(&clip, &[100, 100]);
        write_gif(&still, &[100]);
        let webp = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/four-frames.webp");
        let png = dir.path().join("still.png");
        RgbaImage::new(2, 2).save(&png).unwrap();
        let mkv = dir.path().join("any.MKV");
        let kinds = [&mkv, &clip, &webp, &still, &png].map(|path| MediaKind::of(path));
        assert_eq!(
            kinds,
            [
                MediaKind::Video,
                MediaKind::Animation(AnimationFormat::Gif),
                MediaKind::Animation(AnimationFormat::Webp),
                MediaKind::Still,
                MediaKind::Still,
            ]
        );
        assert_eq!(
            kinds.map(MediaKind::is_moving),
            [true, true, true, false, false]
        );
        // A batch made a PNG of an animated WebP before, and still does.
        assert_eq!(
            kinds.map(MediaKind::batch_as_video),
            [true, true, false, false, false]
        );
    }

    #[test]
    fn a_dropped_reader_stops_the_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.gif");
        write_gif(&path, &[20; 50]);
        let cancel = Arc::new(AtomicBool::new(false));
        let (mut decoding, frames) =
            Decoding::start(&path, AnimationFormat::Gif, (0..50).collect(), 1, &cancel);
        drop(frames);
        assert!(decoding.wait().is_ok());
        cancel.store(true, Ordering::Relaxed);
        let (mut decoding, mut frames) =
            Decoding::start(&path, AnimationFormat::Gif, (0..50).collect(), 1, &cancel);
        let _ = frames.read_to_end(&mut vec![]);
        assert!(decoding.wait().unwrap_err().to_string().contains("cancel"));
    }
}
