//! Exporting a whole video or animation with FFmpeg on the desktop: `jobs::export` renders
//! the frames, which FFmpeg encodes as they come on a thread of their own, and FFmpeg then
//! finishes the file, such as by muxing the source's tracks; only then is the output replaced.
use crate::{
    animation::AnimationPlan,
    check_cancel,
    jobs::{self, Output, Span},
    plan::ExportPlan,
    process::Process,
    AnimationFormat, AnimationOptions, Options, Timing, Video,
};
use anyhow::{ensure, Context, Result};
use crtsim_core::{config::Config, Renderer, Sequence};
use image::RgbaImage;
use std::{
    cell::RefCell,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicBool, mpsc, Arc},
    thread::JoinHandle,
};

/// How far an export has got, in words for the person waiting on it.
#[derive(Clone, Debug)]
pub struct Progress {
    /// Completed, weighted stages; not an estimate of elapsed time.
    pub fraction: f32,
    pub stage: String,
}

/// The rate frames are rendered and encoded at.
#[derive(Clone, Debug, PartialEq)]
pub struct Rate {
    /// As FFmpeg is told it: exact, such as 30000/1001.
    pub text: String,
    pub fps: f64,
}

impl Rate {
    /// `fps` frames a second, as FFmpeg is told a decimal rate.
    pub fn per_second(fps: f64) -> Self {
        Self {
            text: format!("{fps}"),
            fps,
        }
    }

    /// The source's own rate, or 60 per second for NTSC timing.
    pub fn of(video: &Video, options: &Options) -> Self {
        if options.timing == Timing::Ntsc60 {
            Self {
                text: "60/1".into(),
                fps: 60.,
            }
        } else {
            Self {
                text: video.rate.clone(),
                fps: video.fps,
            }
        }
    }
}

/// What exporting needs from a plan: the frames to render, and how to encode and finish them.
/// A video and an animation differ only here.
pub(crate) trait Encoding {
    fn frames(&self) -> Frames<'_>;
    /// Checks that FFmpeg can encode this, before anything is rendered.
    fn check(&self, cancel: &Arc<AtomicBool>) -> Result<()>;
    /// The extension of a temporary file the frames are encoded to first, or `None` to
    /// encode them straight to the output.
    fn intermediate(&self) -> Option<&'static str>;
    /// The encoder: rendered frames in as raw RGBA, written to `encoded`.
    fn encoder(&self, encoded: &Path) -> Command;
    /// What the finishing steps read from `Work::metadata`, if they need anything.
    fn metadata_file(&self) -> Result<Option<String>>;
    /// What turns the `frames` encoded frames into the output, in order.
    fn finishing(&self, work: &Work, frames: u64) -> Vec<Step>;
}

/// The frames an export renders.
pub(crate) struct Frames<'p> {
    pub video: &'p Video,
    /// The settings each frame is rendered with, before the timing adjusts them.
    pub render: &'p Config,
    pub timing: Timing,
    /// The rendered size.
    pub size: (u32, u32),
    pub rate: &'p Rate,
    /// Where in the video to start, in seconds.
    pub start: f64,
    /// At most this many seconds.
    pub limit: Option<f64>,
}

impl Frames<'_> {
    /// The sequence the frames are rendered in, following the export's timing at its rate.
    pub fn sequence(&self) -> Sequence {
        Sequence::video(self.timing, self.rate.fps)
    }

    /// How long `count` frames last.
    pub fn duration(&self, count: u64) -> f64 {
        count as f64 / self.rate.fps
    }

    /// How much of the video is rendered, in seconds.
    pub fn length(&self) -> f64 {
        let rest = self.video.duration - self.start;
        self.limit.map_or(rest, |limit| rest.min(limit))
    }

    /// How many frames are rendered: one for each tick of the rate in `length`.
    pub fn count(&self) -> u64 {
        (self.length() * self.rate.fps - 1e-6).ceil().max(1.) as u64
    }

    /// The frames to read.
    pub fn span(&self) -> Span {
        Span {
            start: self.start,
            rate: Some(self.rate.clone()),
            limit: self.limit,
        }
    }
}

/// Where an export's files are while it runs.
pub(crate) struct Work<'a> {
    /// The encoded frames.
    pub encoded: &'a Path,
    /// What `Encoding::metadata_file` gave, written out.
    pub metadata: &'a Path,
    /// A temporary folder for anything else.
    pub folder: &'a Path,
    /// The finished file, before it replaces the output.
    pub destination: &'a Path,
}

/// One command that finishes an export, with alternatives tried in turn when it fails.
pub(crate) struct Step {
    pub stage: &'static str,
    pub commands: Vec<Command>,
    /// The error when every alternative failed.
    pub failed: &'static str,
}

impl Step {
    pub fn new(stage: &'static str, command: Command) -> Self {
        Self {
            stage,
            commands: vec![command],
            failed: "",
        }
    }

    fn run(&mut self, cancel: &Arc<AtomicBool>) -> Result<()> {
        let count = self.commands.len();
        for (index, command) in self.commands.iter_mut().enumerate() {
            match Process::run(command, cancel) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    check_cancel(cancel)?;
                    if index + 1 == count {
                        return Err(if count > 1 {
                            error.context(self.failed)
                        } else {
                            error
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

/// The caller supplies the renderer, so the same media pipeline can be tested without a GPU.
pub fn export_with(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    cancel: &Arc<AtomicBool>,
    render: impl FnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    progress: impl FnMut(Progress),
) -> Result<()> {
    check_cancel(cancel)?;
    let plan = ExportPlan::new(video, output, config, options)?;
    run(&plan, output, cancel, render, progress)
}

/// An animated GIF or WebP, as `output`'s extension asks. The caller supplies the renderer.
pub fn export_animation_with(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &AnimationOptions,
    cancel: &Arc<AtomicBool>,
    render: impl FnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    progress: impl FnMut(Progress),
) -> Result<()> {
    check_cancel(cancel)?;
    let format = AnimationFormat::of(output).context("Choose a GIF or WebP filename")?;
    let plan = AnimationPlan::new(video, format, config, options)?;
    run(&plan, output, cancel, render, progress)
}

/// Renders the plan's frames on this thread with `jobs::export`, from frames FFmpeg decodes
/// into frames FFmpeg encodes, and only then replaces `output`.
fn run(
    plan: &impl Encoding,
    output: &Path,
    cancel: &Arc<AtomicBool>,
    mut render: impl FnMut(&mut Sequence, &RgbaImage, &Config) -> Result<RgbaImage>,
    progress: impl FnMut(Progress),
) -> Result<()> {
    let frames = plan.frames();
    let encoder = Ffmpeg::start(plan, output, cancel)?;
    let progress = RefCell::new(progress);
    pollster::block_on(jobs::export(
        &frames,
        async |span| jobs::frames(frames.video, span, cancel),
        async |sequence, frame, config| render(sequence, frame, config),
        encoder,
        cancel,
        &|step| (progress.borrow_mut())(step),
    ))
}

/// A plan's encoder and what finishes its file: FFmpeg, fed the rendered frames as raw RGBA
/// through `Feed`, while they are rendered.
struct Ffmpeg<'p, P: Encoding> {
    /// First, so an export that stops early stops its encoder before the temporary files
    /// below are deleted: Windows cannot delete a file a running FFmpeg holds open.
    encoder: Process,
    feed: Feed,
    plan: &'p P,
    output: PathBuf,
    cancel: Arc<AtomicBool>,
    folder: tempfile::TempDir,
    destination: tempfile::NamedTempFile,
    encoded: PathBuf,
    metadata: PathBuf,
}

impl<'p, P: Encoding> Ffmpeg<'p, P> {
    /// Checks that FFmpeg can encode `plan` to `output`, and starts it.
    fn start(plan: &'p P, output: &Path, cancel: &Arc<AtomicBool>) -> Result<Self> {
        check_cancel(cancel)?;
        plan.check(cancel)?;
        let video = plan.frames().video;
        if output.exists() {
            ensure!(
                output.canonicalize()? != video.path,
                "Choose an output other than the source video"
            );
        }
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let folder = tempfile::tempdir_in(parent)?;
        let destination = tempfile::NamedTempFile::new_in(parent)?;
        // Use a file: embedded LUTs are too large for OS command-line limits.
        let metadata = folder.path().join("preset.ffmeta");
        if let Some(text) = plan.metadata_file()? {
            std::fs::write(&metadata, text)?;
        }
        let encoded = match plan.intermediate() {
            Some(extension) => folder.path().join(format!("video.{extension}")),
            None => destination.path().to_owned(),
        };
        let mut encoder = Process::spawn(&mut plan.encoder(&encoded), cancel)?;
        let feed = Feed::start(encoder.stdin());
        Ok(Self {
            encoder,
            feed,
            plan,
            output: output.to_owned(),
            cancel: cancel.clone(),
            folder,
            destination,
            encoded,
            metadata,
        })
    }

    /// Why feeding the encoder failed: its own log, which says more than a broken pipe does.
    fn failed(&mut self, error: std::io::Error) -> anyhow::Error {
        match self.encoder.wait() {
            Err(logged) => logged,
            Ok(()) => error.into(),
        }
    }
}

impl<P: Encoding> Output for Ffmpeg<'_, P> {
    type Made = ();

    async fn frame(&mut self, _pass: u32, frame: RgbaImage) -> Result<()> {
        match self.feed.push(frame) {
            Ok(()) => Ok(()),
            Err(error) => Err(self.failed(error)),
        }
    }

    async fn finish(mut self, count: u64, progress: &dyn Fn(Progress)) -> Result<()> {
        if let Err(error) = self.feed.close() {
            return Err(self.failed(error));
        }
        progress(Progress {
            fraction: 0.91,
            stage: "Finishing encoding".into(),
        });
        self.encoder.wait()?;
        check_cancel(&self.cancel)?;
        let work = Work {
            encoded: &self.encoded,
            metadata: &self.metadata,
            folder: self.folder.path(),
            destination: self.destination.path(),
        };
        let mut steps = self.plan.finishing(&work, count);
        let total = steps.len();
        for (index, step) in steps.iter_mut().enumerate() {
            progress(Progress {
                fraction: 0.95 + 0.05 * index as f32 / total as f32,
                stage: step.stage.into(),
            });
            step.run(&self.cancel)?;
            check_cancel(&self.cancel)?;
        }
        self.destination.as_file().sync_all()?;
        self.destination
            .persist(&self.output)
            .map_err(|e| anyhow::anyhow!("Cannot publish output: {}", e.error))?;
        Ok(())
    }
}

/// Frames in flight between two stages. Enough to absorb one stage's jitter; more would only
/// hold another full frame of memory each, which at 4K is 33 MB.
pub(crate) const QUEUED_FRAMES: usize = 2;

/// Rendered frames written to an encoder's input on a thread of their own, so rendering only
/// waits on the encoder when `QUEUED_FRAMES` are already waiting for it. They are written in
/// the order they are pushed.
struct Feed {
    frames: Option<mpsc::SyncSender<RgbaImage>>,
    writer: Option<JoinHandle<std::io::Result<()>>>,
}

impl Feed {
    fn start(mut input: impl Write + Send + 'static) -> Self {
        let (frames, queued) = mpsc::sync_channel::<RgbaImage>(QUEUED_FRAMES);
        let writer = std::thread::spawn(move || {
            for frame in queued {
                input.write_all(frame.as_raw())?;
            }
            // Dropping the input here is what tells the encoder the frames have ended.
            Ok(())
        });
        Self {
            frames: Some(frames),
            writer: Some(writer),
        }
    }

    /// Queues `frame` to be written; the error the writer stopped with, if it has.
    fn push(&mut self, frame: RgbaImage) -> std::io::Result<()> {
        let sent = self.frames.as_ref().map(|frames| frames.send(frame));
        match sent {
            Some(Ok(())) => Ok(()),
            _ => self.close().and(Err(std::io::ErrorKind::BrokenPipe.into())),
        }
    }

    /// Writes what is queued and closes the encoder's input.
    fn close(&mut self) -> std::io::Result<()> {
        self.frames = None;
        match self.writer.take() {
            Some(writer) => writer.join().expect("encoder writer panicked"),
            None => Ok(()),
        }
    }
}

pub fn export(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(Progress),
) -> Result<()> {
    export_with(
        video,
        output,
        config,
        options,
        cancel,
        |sequence, frame, config| render_frame(renderer, sequence, frame, config, cancel),
        progress,
    )
}

/// `sequence`'s next frame of `input`, read back.
fn render_frame(
    renderer: &Renderer,
    sequence: &mut Sequence,
    input: &RgbaImage,
    config: &Config,
    cancel: &AtomicBool,
) -> Result<RgbaImage> {
    pollster::block_on(jobs::render(renderer, sequence, input, config, cancel))
}

/// An animated GIF or WebP, as `output`'s extension asks, rendered on `renderer`.
pub fn export_animation(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &AnimationOptions,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(Progress),
) -> Result<()> {
    export_animation_with(
        video,
        output,
        config,
        options,
        cancel,
        |sequence, frame, config| render_frame(renderer, sequence, frame, config, cancel),
        progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// What an encoder was written, kept for the test to read.
    #[derive(Clone, Default)]
    struct Written(Arc<Mutex<Vec<u8>>>);

    impl Write for Written {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_feed_writes_frames_in_the_order_they_are_pushed() {
        let written = Written::default();
        let mut feed = Feed::start(written.clone());
        for i in 0..40u8 {
            // The writer runs alongside, so a slow push must not let frames overtake it.
            if i % 7 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            feed.push(RgbaImage::from_pixel(2, 2, image::Rgba([i; 4])))
                .unwrap();
        }
        feed.close().unwrap();
        let expected: Vec<u8> = (0..40u8).flat_map(|i| [i; 16]).collect();
        assert_eq!(*written.0.lock().unwrap(), expected);
    }

    #[test]
    fn the_feed_reports_an_encoder_that_stops_reading() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut feed = Feed::start(Closed);
        let pushed = (0..50).try_for_each(|_| feed.push(RgbaImage::new(2, 2)));
        let error = pushed.and_then(|()| feed.close()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
}
