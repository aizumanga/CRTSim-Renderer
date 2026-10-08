//! Decoded frames of a video, one at a time: single previews and continuous playback.
//!
//! FFmpeg decodes videos. Animated GIF and WebP are decoded by `animated` instead, which also
//! draws the video test card's frames; all come out as the same stream of packed RGBA frames
//! at the video's size.
use crate::{
    animated,
    export::QUEUED_FRAMES,
    jobs::{self, FrameSource, Span},
    probe::{Drawing, Source},
    process::Process,
    Options, Rate, Tool, Video,
};
use anyhow::{ensure, Context, Result};
use crtsim_core::{config::Config, Renderer};
use image::RgbaImage;
use std::{
    io::Read,
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, mpsc, Arc},
};

/// Which frames to decode.
pub(crate) struct Request<'a> {
    /// A constant rate to take frames at, or `None` for a single frame.
    pub rate: Option<&'a Rate>,
    /// Seconds into the video to start from.
    pub start: f64,
    /// Instead, one frame by its number in decoding order, counting from 0.
    pub frame: Option<u64>,
    /// At most this many seconds of frames.
    pub limit: Option<f64>,
}

impl<'a> Request<'a> {
    /// The frames `span` asks for.
    pub fn of(span: &'a Span) -> Self {
        Self {
            rate: span.rate.as_ref(),
            start: span.start,
            frame: None,
            limit: span.limit,
        }
    }
}

/// A decode in progress. Its frames are read from `frames`; `wait` then reports how it ended.
pub(crate) struct Decoder {
    frames: Option<Box<dyn Read + Send>>,
    work: Work,
}

enum Work {
    Ffmpeg(Process),
    Animated(animated::Decoding),
}

impl Decoder {
    pub fn open(video: &Video, request: &Request, cancel: &Arc<AtomicBool>) -> Result<Self> {
        Ok(match &video.source {
            Source::Ffmpeg => {
                let mut process = Process::spawn(&mut decode_command(video, request), cancel)?;
                drop(process.stdin());
                Self {
                    frames: Some(Box::new(process.stdout())),
                    work: Work::Ffmpeg(process),
                }
            }
            Source::Demuxed(_) => anyhow::bail!("This video is decoded by the browser"),
            Source::Animated { .. } | Source::TestClip => {
                let plan = animated::Plan::new(video, request)?;
                let (decoding, frames) = animated::Decoding::start(plan, QUEUED_FRAMES, cancel);
                Self {
                    frames: Some(Box::new(frames)),
                    work: Work::Animated(decoding),
                }
            }
        })
    }

    /// The decoded frames, as packed RGBA. Taken once.
    pub fn frames(&mut self) -> Box<dyn Read + Send> {
        self.frames
            .take()
            .expect("a decode's frames are taken once")
    }

    /// Stops the decode now, so a reader blocked on it gets the end of its frames.
    pub fn kill(&self) {
        match &self.work {
            Work::Ffmpeg(process) => process.kill(),
            Work::Animated(decoding) => decoding.kill(),
        }
    }

    /// Waits for the decode to end, with its error if it failed.
    pub fn wait(&mut self) -> Result<()> {
        match &mut self.work {
            Work::Ffmpeg(process) => process.wait(),
            Work::Animated(decoding) => decoding.wait(),
        }
    }
}

/// How much footage before a frame is decoded and dropped when a bitmap subtitle is laid over
/// it, in seconds. A bitmap subtitle is one packet at the time it appears, and FFmpeg's seek
/// skips the packets before the frame it seeks to, so a subtitle already showing there would be
/// missing; the longest one on screen at once is rarely longer than this.
const BITMAP_LEAD: f64 = 10.;

fn decode_command(video: &Video, request: &Request) -> Command {
    let drawing = video
        .subtitle
        .and_then(|n| video.subtitles().nth(n).map(|track| (n, track)))
        .and_then(|(n, track)| Some((n, track.drawing()?)));
    let lead = match drawing {
        Some((_, Drawing::Bitmap)) => request.start.min(BITMAP_LEAD),
        _ => 0.,
    };
    let mut cmd = Tool::Ffmpeg.command();
    // Seeking is input-relative and preview-only; full exports always start at zero.
    if request.start - lead > 0. {
        cmd.args(["-ss", &(request.start - lead).to_string()]);
    }
    cmd.arg("-i").arg(&video.path);
    // What the frames pass through before a bitmap subtitle is laid over them, and after.
    let mut filters = vec!["setpts=PTS-STARTPTS".to_string()];
    let mut after = vec![];
    if let Some(frame) = request.frame {
        // Select by decoded frame ordinal, including VFR sources. Decode from the start
        // to avoid timestamp rounding and keyframe seeks skipping or repeating frames.
        filters.push(format!("select=eq(n\\,{frame})"));
    }
    if lead > 0. {
        after.push(format!("trim=start={lead}"));
        after.push("setpts=PTS-STARTPTS".to_string());
    }
    if let Some(rate) = request.rate {
        after.push(format!("fps={}:start_time=0:round=near", rate.text));
    }
    if video.hdr {
        after.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,\
             tonemap=tonemap=mobius:desat=0,zscale=t=bt709:m=bt709:r=full"
                .into(),
        );
    }
    if let Some((n, Drawing::Text)) = drawing {
        // The filter reads the file's own times, which the frames no longer have: they start
        // at 0 where decoding did, and the file may start later.
        after.push(format!("setpts=PTS+{}/TB", request.start + video.start));
        after.push(format!(
            "subtitles=filename={}:si={n}",
            filter_path(&video.path)
        ));
        after.push("setpts=PTS-STARTPTS".to_string());
    }
    after.push(format!(
        "scale={}:{}:flags=lanczos,setsar=1",
        video.size.0, video.size.1
    ));
    if let Some((n, Drawing::Bitmap)) = drawing {
        // The video and the subtitle are two outputs of one input, so they are joined in one
        // graph. Once the subtitle ends, the video goes on without it.
        let graph = format!(
            "[0:{}]{}[video];[video][0:s:{n}]overlay=eof_action=pass[shown];[shown]{}[out]",
            video.stream,
            filters.join(","),
            after.join(",")
        );
        cmd.args(["-filter_complex", &graph, "-map", "[out]"]);
    } else {
        filters.extend(after);
        cmd.args(["-map", &format!("0:{}", video.stream)]);
        cmd.args(["-vf", &filters.join(",")]);
    }
    cmd.args(["-an", "-sn", "-dn"]);
    if request.rate.is_none() {
        cmd.args(["-frames:v", "1"]);
    }
    if let Some(limit) = request.limit {
        cmd.args(["-t", &limit.to_string()]);
    }
    cmd.args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"]);
    cmd
}

/// A file's path as a filter's option takes it: FFmpeg reads the argument as a filter graph, in
/// which `[],;'\` mean something, and then each filter's options, in which `:'\` do, so it
/// escapes twice.
fn filter_path(path: &Path) -> String {
    let option = |text: &str, special: &[char]| -> String {
        text.chars().fold(String::new(), |mut escaped, c| {
            if special.contains(&c) {
                escaped.push('\\');
            }
            escaped.push(c);
            escaped
        })
    };
    let once = option(&path.to_string_lossy(), &['\\', '\'', ':']);
    option(&once, &['\\', '\'', '[', ']', ',', ';'])
}

/// The frame shown at `time` seconds.
pub fn preview(video: &Video, time: f64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    ensure!(
        time.is_finite() && time >= 0. && time < video.duration,
        "Preview time is outside the video"
    );
    let request = Request {
        rate: None,
        start: time,
        frame: None,
        limit: None,
    };
    decode_one(
        video,
        &request,
        cancel,
        "FFmpeg did not return a complete preview frame",
    )
}

/// The frame numbered `frame`, counting from 0 in decoding order.
pub fn preview_frame(video: &Video, frame: u64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    let request = Request {
        rate: None,
        start: 0.,
        frame: Some(frame),
        limit: None,
    };
    let missing = match video.source {
        Source::Ffmpeg => "FFmpeg did not return the requested frame".to_owned(),
        _ => format!("The animation has no frame {}", frame + 1),
    };
    decode_one(video, &request, cancel, &missing)
}

/// Runs a decode that stops after one frame and returns it. `missing` is the error when it
/// ends before a whole frame.
fn decode_one(
    video: &Video,
    request: &Request,
    cancel: &Arc<AtomicBool>,
    missing: &str,
) -> Result<RgbaImage> {
    let mut decoder = Decoder::open(video, request, cancel)?;
    let mut bytes = vec![0; video.size.0 as usize * video.size.1 as usize * 4];
    let read = decoder.frames().read_exact(&mut bytes);
    decoder.wait()?;
    read.context(missing.to_owned())?;
    RgbaImage::from_raw(video.size.0, video.size.1, bytes).context("Invalid preview pixels")
}

/// A video's frames as FFmpeg decodes them, or an animation's on a thread of its own, read on
/// a thread of their own `QUEUED_FRAMES` ahead, so decoding goes on while a frame renders.
pub struct Piped {
    decoder: Decoder,
    frames: Prefetched,
    ended: bool,
}

impl Piped {
    pub fn open(video: &Video, span: &Span, cancel: &Arc<AtomicBool>) -> Result<Self> {
        let mut decoder = Decoder::open(video, &Request::of(span), cancel)?;
        Ok(Self {
            frames: Prefetched::start(decoder.frames(), video.size),
            decoder,
            ended: false,
        })
    }
}

impl FrameSource for Piped {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        if self.ended {
            return Ok(None);
        }
        let frame = self.frames.next();
        if !matches!(frame, Ok(Some(_))) {
            self.ended = true;
        }
        if let Ok(None) = frame {
            // The end of the frames: whether the decode finished or failed.
            self.decoder.wait()?;
        }
        frame
    }
}

impl Drop for Piped {
    fn drop(&mut self) {
        // A reader blocked on the decode gets the end of its frames, and its thread ends.
        self.decoder.kill();
    }
}

/// Frames of packed RGBA read from a stream on a thread of their own, in order, a few ahead.
pub(crate) struct Prefetched(mpsc::Receiver<Result<RgbaImage>>);

impl Prefetched {
    pub fn start(mut stream: impl Read + Send + 'static, (width, height): (u32, u32)) -> Self {
        let (read, frames) = mpsc::sync_channel(QUEUED_FRAMES);
        std::thread::spawn(move || loop {
            let mut frame = RgbaImage::new(width, height);
            let bytes = frame.as_mut();
            let next = match stream.read(&mut bytes[..1]) {
                Ok(0) => break,
                Ok(_) => stream
                    .read_exact(&mut bytes[1..])
                    .context("Truncated video frame")
                    .map(|()| frame),
                Err(error) => Err(error.into()),
            };
            let failed = next.is_err();
            if read.send(next).is_err() || failed {
                break;
            }
        });
        Self(frames)
    }

    /// The next frame, or `None` after the last.
    pub fn next(&self) -> Result<Option<RgbaImage>> {
        self.0.recv().ok().transpose()
    }
}

/// Plays `video` on this thread, as `jobs::playback` does, from frames FFmpeg decodes.
/// `frame_ready` sets the pace by blocking until there is room for the frame; cancelling also
/// stops the decode.
#[expect(
    clippy::too_many_arguments,
    reason = "each is one part the host supplies"
)]
pub fn playback(
    video: &Video,
    start: f64,
    config: impl Fn() -> Arc<Config>,
    options: &Options,
    looping: impl Fn() -> bool,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    mut frame_ready: impl FnMut(f64, f64, RgbaImage, RgbaImage) -> Result<()>,
) -> Result<()> {
    pollster::block_on(jobs::playback(
        video,
        start,
        config,
        options,
        looping,
        async |span| jobs::frames(video, span, cancel),
        async |sequence, frame, config| {
            jobs::render(renderer, sequence, frame, config, cancel).await
        },
        cancel,
        async |time, elapsed, source, crt| frame_ready(time, elapsed, source, crt),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A video of 64 by 48 starting at `start`, with a text subtitle track, a bitmap one and
    /// one that cannot be drawn.
    fn subtitled(subtitle: Option<usize>) -> Video {
        let track = |index, kind, codec: &str| crate::Track {
            index,
            kind,
            codec: codec.into(),
            offset: 0.,
            language: None,
            title: None,
        };
        Video {
            metadata: Default::default(),
            tracks: vec![
                track(0, crate::TrackKind::Video, "h264"),
                track(1, crate::TrackKind::Subtitle, "subrip"),
                track(2, crate::TrackKind::Subtitle, "hdmv_pgs_subtitle"),
                track(3, crate::TrackKind::Subtitle, "eia_608"),
            ],
            start: 1.5,
            path: "clip [1].mkv".into(),
            size: (64, 48),
            fps: 25.,
            rate: "25/1".into(),
            duration: 60.,
            audio: false,
            audio_offset: 0.,
            hdr: false,
            stream: 0,
            frames: None,
            source: Source::Ffmpeg,
            contents: None,
            subtitle,
        }
    }

    fn args(video: &Video, request: &Request) -> Vec<String> {
        decode_command(video, request)
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// The value given after `flag`.
    fn value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|arg| arg == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    #[test]
    fn a_text_subtitle_is_drawn_at_the_files_times_and_a_bitmap_one_laid_over_with_a_lead_in() {
        let rate = Rate::per_second(25.);
        let request = Request {
            rate: Some(&rate),
            start: 30.,
            frame: None,
            limit: Some(2.),
        };
        let plain = args(&subtitled(None), &request);
        assert_eq!(value(&plain, "-ss"), Some("30"));
        assert_eq!(value(&plain, "-map"), Some("0:0"));
        assert!(!value(&plain, "-vf").unwrap().contains("subtitles"));

        // Text: its times are the file's, which start 1.5 s late and 30 s into the video.
        let text = args(&subtitled(Some(0)), &request);
        assert_eq!(value(&text, "-ss"), Some("30"));
        let filters = value(&text, "-vf").unwrap();
        assert!(
            filters
                .contains("fps=25:start_time=0:round=near,setpts=PTS+31.5/TB,subtitles=filename="),
            "{filters}"
        );
        assert!(
            filters.contains(r"clip \[1\].mkv:si=0,setpts=PTS-STARTPTS,scale="),
            "{filters}"
        );
        assert!(filters.find("subtitles").unwrap() < filters.find("scale=").unwrap());

        // Bitmap: laid over before anything else is done to the frames, after seeking
        // early enough that one already showing at the start is read.
        let bitmap = args(&subtitled(Some(1)), &request);
        assert_eq!(value(&bitmap, "-ss"), Some("20"));
        assert_eq!(value(&bitmap, "-map"), Some("[out]"));
        assert!(value(&bitmap, "-vf").is_none());
        let graph = value(&bitmap, "-filter_complex").unwrap();
        assert!(graph.starts_with("[0:0]setpts=PTS-STARTPTS[video];[video][0:s:1]overlay"));
        assert!(
            graph.contains("[shown]trim=start=10,setpts=PTS-STARTPTS,fps=25"),
            "{graph}"
        );
        assert!(graph.ends_with("setsar=1[out]"));
        // Near the start there is less to look back over.
        let early = Request {
            start: 4.,
            ..request
        };
        let bitmap = args(&subtitled(Some(1)), &early);
        assert_eq!(value(&bitmap, "-ss"), None);
        assert!(value(&bitmap, "-filter_complex")
            .unwrap()
            .contains("trim=start=4,"));

        // One that cannot be drawn, or is not there, leaves the picture as it was.
        assert_eq!(args(&subtitled(Some(2)), &request), plain);
        assert_eq!(args(&subtitled(Some(9)), &request), plain);
    }

    #[test]
    fn a_path_is_escaped_for_the_graph_and_then_the_filter() {
        let escaped = |path: &str| filter_path(Path::new(path));
        assert_eq!(escaped("movie.mkv"), "movie.mkv");
        assert_eq!(escaped("a b, [c]; d.mkv"), r"a b\, \[c\]\; d.mkv");
        // A colon and a backslash are the filter options' own, so C:\Videos is C\:\\Videos
        // there, and the graph then escapes each backslash it was given again.
        assert_eq!(escaped(r"C:\Videos"), r"C\\:\\\\Videos");
        assert_eq!(escaped("it's"), r"it\\\'s");
    }

    #[test]
    fn prefetched_frames_come_in_order_and_a_truncated_one_fails() {
        let bytes: Vec<u8> = (0..30u8).flat_map(|i| [i; 16]).collect();
        let frames = Prefetched::start(std::io::Cursor::new(bytes), (2, 2));
        for i in 0..30u8 {
            let frame = frames.next().unwrap().unwrap();
            assert_eq!(frame.as_raw(), &[i; 16]);
        }
        assert!(frames.next().unwrap().is_none());
        let truncated = Prefetched::start(std::io::Cursor::new(vec![0; 20]), (2, 2));
        assert!(truncated.next().unwrap().is_some());
        let error = truncated.next().unwrap_err();
        assert!(error.to_string().contains("Truncated"), "{error}");
    }
}
