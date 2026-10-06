//! Animated GIF and WebP, decoded here rather than by FFmpeg, so they open without it; and the
//! video test card, whose frames are drawn here.
//!
//! Each frame comes out composited onto the whole canvas, with how long it shows. Frames are
//! read in order and one at a time, since each one builds on the canvas the last one left.
use crate::{
    check_cancel,
    decode::Request,
    probe::{Contents, Source, Track, TrackKind, Video},
};
use anyhow::{bail, ensure, Context, Result};
use crtsim_core::{config, test_clip};
use image::{codecs::gif::GifDecoder, AnimationDecoder, ImageDecoder, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Read, Seek},
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

/// Where an animation's bytes are read from: its file, or the bytes a browser handed over.
#[derive(Clone, Debug)]
pub(crate) enum Origin {
    Path(PathBuf),
    Bytes(Contents),
}

/// What the decoders read from.
trait Reader: BufRead + Seek + Send {}
impl<T: BufRead + Seek + Send> Reader for T {}

impl Origin {
    /// Where `video`'s frames are read from.
    pub(crate) fn of(video: &Video) -> Self {
        match &video.contents {
            Some(contents) => Self::Bytes(contents.clone()),
            None => Self::Path(video.path.clone()),
        }
    }

    fn reader(&self) -> Result<Box<dyn Reader>> {
        Ok(match self {
            Self::Path(path) => Box::new(BufReader::new(
                File::open(path).context("Cannot open animation")?,
            )),
            Self::Bytes(contents) => Box::new(Cursor::new(contents.clone())),
        })
    }
}

/// The format of `path` if it is a GIF or WebP with more than one frame. Reads the header, and
/// for a GIF at most two frames.
pub(crate) fn detect(path: &Path) -> Option<AnimationFormat> {
    detect_in(&Origin::Path(path.to_owned()), AnimationFormat::of(path)?)
}

/// The format of the file called `name` with these `contents`, if it is an animation.
pub fn detect_bytes(name: &Path, contents: &Contents) -> Option<AnimationFormat> {
    detect_in(&Origin::Bytes(contents.clone()), AnimationFormat::of(name)?)
}

fn detect_in(origin: &Origin, format: AnimationFormat) -> Option<AnimationFormat> {
    let file = origin.reader().ok()?;
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
        decoder: Box<image_webp::WebPDecoder<Box<dyn Reader>>>,
        /// One frame as the decoder writes it: RGB, or RGBA when the file has alpha.
        buffer: Vec<u8>,
    },
}

impl Frames {
    /// Opens the animation, returning its canvas size and its frames.
    fn open(origin: &Origin, format: AnimationFormat) -> Result<((u32, u32), Self)> {
        let file = origin.reader()?;
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
    probe_in(Origin::Path(path.clone()), path, format, cancel)
}

/// Reads the size and frame timing of the animation called `name`, from the `contents` a
/// browser handed over. The video keeps them, to decode its frames from.
pub fn probe_bytes(
    name: PathBuf,
    contents: Contents,
    format: AnimationFormat,
    cancel: &Arc<AtomicBool>,
) -> Result<Video> {
    probe_in(Origin::Bytes(contents), name, format, cancel)
}

fn probe_in(
    origin: Origin,
    path: PathBuf,
    format: AnimationFormat,
    cancel: &Arc<AtomicBool>,
) -> Result<Video> {
    let (size, frames) = Frames::open(&origin, format)?;
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
        contents: match origin {
            Origin::Bytes(contents) => Some(contents),
            Origin::Path(_) => None,
        },
    })
}

/// The video test card: ten seconds of pixel-art gameplay at 60 frames per second, drawn here,
/// so it opens, plays and exports as an animation does, without a file or FFmpeg to read it.
pub fn test_clip() -> Video {
    let fps = test_clip::FPS;
    Video {
        metadata: Default::default(),
        tracks: vec![Track {
            index: 0,
            kind: TrackKind::Video,
            codec: "drawn".into(),
            offset: 0.,
        }],
        start: 0.,
        // Not a file: what the app calls it.
        path: "Video test card".into(),
        size: test_clip::SIZE,
        fps: f64::from(fps),
        rate: format!("{fps}/1"),
        duration: test_clip::FRAMES as f64 / f64::from(fps),
        audio: false,
        audio_offset: 0.,
        hdr: false,
        stream: 0,
        frames: Some(test_clip::FRAMES),
        source: Source::TestClip,
        contents: None,
    }
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
    let mut starts = Vec::with_capacity(delays.len());
    let mut end = 0.;
    for &delay in delays {
        starts.push(end);
        end += f64::from(delay) / 1000.;
    }
    schedule_starts(&starts, end, request)
}

/// As `schedule`, for frames starting at `starts` and ending at `end`, in seconds.
fn schedule_starts(starts: &[f64], end: f64, request: &Request) -> Result<Vec<usize>> {
    if let Some(frame) = request.frame {
        ensure!(
            (frame as usize) < starts.len(),
            "The animation has no frame {}",
            frame + 1
        );
        return Ok(vec![frame as usize]);
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

/// Where a plan's frames come from.
enum Maker {
    /// Decoded from an animated GIF or WebP.
    File(Origin, AnimationFormat),
    /// Drawn by the video test card.
    TestClip,
}

/// Which frames of a video made here to read, in order, and where they come from. It can move
/// to the thread that reads them; the decoders it opens there cannot.
pub(crate) struct Plan {
    maker: Maker,
    frames: Vec<usize>,
}

impl Plan {
    /// The frames of `video` that `request` asks for: an animation's, or the video test card's.
    pub(crate) fn new(video: &Video, request: &Request) -> Result<Self> {
        match &video.source {
            Source::Animated { format, delays } => Ok(Self {
                maker: Maker::File(Origin::of(video), *format),
                frames: schedule(delays, request)?,
            }),
            Source::TestClip => {
                let fps = f64::from(test_clip::FPS);
                let starts: Vec<f64> = (0..test_clip::FRAMES)
                    .map(|frame| frame as f64 / fps)
                    .collect();
                Ok(Self {
                    maker: Maker::TestClip,
                    frames: schedule_starts(&starts, video.duration, request)?,
                })
            }
            Source::Ffmpeg | Source::Demuxed(_) => {
                bail!("Only animations and the video test card are made here")
            }
        }
    }

    /// Starts reading the frames, on the calling thread.
    pub(crate) fn open(self) -> Result<Planned> {
        Ok(match self.maker {
            Maker::File(origin, format) => Planned(Making::Decoded {
                frames: Frames::open(&origin, format)?.1,
                plan: self.frames,
                next: 0,
                decoded: 0,
                current: RgbaImage::new(0, 0),
            }),
            Maker::TestClip => Planned(Making::Drawn(self.frames.into_iter())),
        })
    }
}

/// The frames a plan lists, made in order on the calling thread: how a browser page, which
/// has no other thread, reads an animation.
pub(crate) struct Planned(Making);

/// How the frames are being made.
enum Making {
    /// Decoded one after another, as each builds on the last.
    Decoded {
        frames: Frames,
        plan: Vec<usize>,
        /// The plan's next entry.
        next: usize,
        /// How many frames have been decoded; the last of them is `current`.
        decoded: usize,
        current: RgbaImage,
    },
    /// Drawn each on its own, so a frame far into the clip is as quick as the first.
    Drawn(std::vec::IntoIter<usize>),
}

impl Iterator for Planned {
    type Item = Result<RgbaImage>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.0 {
            Making::Drawn(plan) => plan.next().map(|frame| Ok(test_clip::frame(frame as u64))),
            Making::Decoded {
                frames,
                plan,
                next,
                decoded,
                current,
            } => {
                let wanted = *plan.get(*next)?;
                while *decoded <= wanted {
                    match frames.next() {
                        Some(Ok((frame, _))) => *current = frame,
                        Some(Err(error)) => return Some(Err(error)),
                        None => return Some(Err(anyhow::anyhow!("The animation ended early"))),
                    }
                    *decoded += 1;
                }
                *next += 1;
                Some(Ok(current.clone()))
            }
        }
    }
}

/// A decode running on its own thread, where the frames are decoded: GIF frames cannot move
/// between threads. It stops when asked, when cancelled or when its reader is dropped.
pub(crate) struct Decoding {
    thread: Option<JoinHandle<Result<()>>>,
    stop: Arc<AtomicBool>,
}

impl Decoding {
    /// Starts making the frames `plan` lists and returns the decode with a reader of them,
    /// as packed RGBA at the canvas size. At most `queued` frames wait to be read.
    pub(crate) fn start(
        plan: Plan,
        queued: usize,
        cancel: &Arc<AtomicBool>,
    ) -> (Self, impl Read + Send) {
        let (send, frames) = mpsc::sync_channel::<Vec<u8>>(queued);
        let stop = Arc::new(AtomicBool::new(false));
        let (cancel, stopped) = (cancel.clone(), stop.clone());
        let thread = std::thread::spawn(move || -> Result<()> {
            for frame in plan.open()? {
                if stopped.load(Ordering::Relaxed) {
                    return Ok(());
                }
                check_cancel(&cancel)?;
                if send.send(frame?.into_raw()).is_err() {
                    // The reader is gone: nothing wants the rest.
                    return Ok(());
                }
            }
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

    /// A plan to decode `frames` of the animation at `path`.
    fn file(path: &Path, format: AnimationFormat, frames: Vec<usize>) -> Plan {
        Plan {
            maker: Maker::File(Origin::Path(path.to_owned()), format),
            frames,
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
            Decoding::start(file(&path, AnimationFormat::Gif, plan), 2, &cancel);
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
            Decoding::start(file(&path, AnimationFormat::Webp, plan), 2, &cancel);
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
    fn the_video_test_card_is_drawn_without_a_file_or_ffmpeg() {
        use crate::jobs::{self, FrameSource};
        let video = test_clip();
        assert_eq!(
            (video.size, video.frames, video.rate.as_str()),
            ((256, 224), Some(600), "60/1")
        );
        assert!((video.duration - 10.).abs() < 1e-9);
        assert!((video.frame_time(150) - 2.5).abs() < 1e-9);
        // A frame by its number, as the desktop reads it, on a thread of its own.
        let cancel = Arc::new(AtomicBool::new(false));
        let frame = crate::preview_frame(&video, 400, &cancel).unwrap();
        assert_eq!(frame, test_clip::frame(400));
        // Frames at half the rate from a second in, as a page reads them.
        let span = jobs::Span {
            start: 1.,
            rate: Some(crate::Rate::per_second(30.)),
            limit: Some(0.1),
        };
        let mut frames = jobs::decoded(&video, &span).unwrap();
        let mut read = vec![];
        while let Some(frame) = pollster::block_on(frames.next()).unwrap() {
            read.push(frame);
        }
        assert_eq!(read, [60, 62, 64].map(test_clip::frame));
        assert!(jobs::frame(&video, 600).is_err(), "there are 600 frames");
    }

    #[test]
    fn a_dropped_reader_stops_the_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.gif");
        write_gif(&path, &[20; 50]);
        let cancel = Arc::new(AtomicBool::new(false));
        let (mut decoding, frames) = Decoding::start(
            file(&path, AnimationFormat::Gif, (0..50).collect()),
            1,
            &cancel,
        );
        drop(frames);
        assert!(decoding.wait().is_ok());
        cancel.store(true, Ordering::Relaxed);
        let (mut decoding, mut frames) = Decoding::start(
            file(&path, AnimationFormat::Gif, (0..50).collect()),
            1,
            &cancel,
        );
        let _ = frames.read_to_end(&mut vec![]);
        assert!(decoding.wait().unwrap_err().to_string().contains("cancel"));
    }
}
