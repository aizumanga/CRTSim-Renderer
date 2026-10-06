//! Decoded frames of a video, one at a time: single previews and continuous playback.
//!
//! FFmpeg decodes videos. Animated GIF and WebP are decoded by `animated` instead, which also
//! draws the video test card's frames; all come out as the same stream of packed RGBA frames
//! at the video's size.
use crate::{
    animated,
    export::QUEUED_FRAMES,
    jobs::{self, FrameSource, Span},
    probe::Source,
    process::Process,
    Options, Rate, Tool, Video,
};
use anyhow::{ensure, Context, Result};
use crtsim_core::{config::Config, Renderer};
use image::RgbaImage;
use std::{
    io::Read,
    process::Command,
    sync::{atomic::AtomicBool, Arc},
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

fn decode_command(video: &Video, request: &Request) -> Command {
    let mut cmd = Tool::Ffmpeg.command();
    // Seeking is input-relative and preview-only; full exports always start at zero.
    if request.start > 0. {
        cmd.args(["-ss", &request.start.to_string()]);
    }
    cmd.arg("-i").arg(&video.path).args([
        "-map",
        &format!("0:{}", video.stream),
        "-an",
        "-sn",
        "-dn",
    ]);
    let mut filters = vec!["setpts=PTS-STARTPTS".to_string()];
    if let Some(frame) = request.frame {
        // Select by decoded frame ordinal, including VFR sources. Decode from the start
        // to avoid timestamp rounding and keyframe seeks skipping or repeating frames.
        filters.push(format!("select=eq(n\\,{frame})"));
    }
    if let Some(rate) = request.rate {
        filters.push(format!("fps={}:start_time=0:round=near", rate.text));
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
    if request.rate.is_none() {
        cmd.args(["-frames:v", "1"]);
    }
    if let Some(limit) = request.limit {
        cmd.args(["-t", &limit.to_string()]);
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
    decode_one(
        video,
        &request,
        cancel,
        "FFmpeg did not return the requested frame",
    )
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

/// A video's frames as FFmpeg decodes them, or an animation's on a thread of its own, read
/// from the decode one at a time as they come, while it decodes the next.
pub struct Piped {
    decoder: Decoder,
    frames: Box<dyn Read + Send>,
    size: (u32, u32),
    ended: bool,
}

impl Piped {
    pub fn open(video: &Video, span: &Span, cancel: &Arc<AtomicBool>) -> Result<Self> {
        let mut decoder = Decoder::open(video, &Request::of(span), cancel)?;
        Ok(Self {
            frames: decoder.frames(),
            decoder,
            size: video.size,
            ended: false,
        })
    }
}

impl FrameSource for Piped {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        if self.ended {
            return Ok(None);
        }
        let mut frame = RgbaImage::new(self.size.0, self.size.1);
        let bytes = frame.as_mut();
        if self.frames.read(&mut bytes[..1])? == 0 {
            // The end of the frames: whether the decode finished or failed.
            self.ended = true;
            self.decoder.wait()?;
            return Ok(None);
        }
        self.frames
            .read_exact(&mut bytes[1..])
            .context("Truncated video frame")?;
        Ok(Some(frame))
    }
}

/// Plays `video` on this thread, as `jobs::playback` does, from frames FFmpeg decodes.
/// `frame_ready` sets the pace by blocking until there is room for the frame; cancelling also
/// stops the decode.
pub fn playback(
    video: &Video,
    start: f64,
    config: impl Fn() -> Arc<Config>,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    mut frame_ready: impl FnMut(f64, RgbaImage, RgbaImage) -> Result<()>,
) -> Result<()> {
    pollster::block_on(jobs::playback(
        video,
        start,
        config,
        options,
        async |span| jobs::frames(video, span, cancel),
        async |sequence, frame, config| {
            jobs::render(renderer, sequence, frame, config, cancel).await
        },
        cancel,
        async |time, source, crt| frame_ready(time, source, crt),
    ))
}
