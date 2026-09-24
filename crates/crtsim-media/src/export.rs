//! Rendering a whole video or animation: decode, render and encode at once, then finish the
//! file, such as by muxing the source's tracks, and only then replace the output.
use crate::{
    animation::AnimationPlan,
    check_cancel, command,
    decode::{Decoder, Request},
    plan::ExportPlan,
    process::Process,
    AnimationFormat, AnimationOptions, Options, Timing, Video,
};
use anyhow::{ensure, Context, Result};
use crtsim_core::{
    config::{Config, Phase},
    RenderProgress, Renderer, Sequence,
};
use image::RgbaImage;
use std::{
    io::{Read, Write},
    path::Path,
    process::Command,
    sync::{atomic::AtomicBool, mpsc, Arc},
    time::Instant,
};

pub(crate) fn require_encoder(name: &str, cancel: &Arc<AtomicBool>) -> Result<()> {
    let mut cmd = command("ffmpeg");
    cmd.args(["-hide_banner", "-encoders"]);
    let bytes = Process::output(
        &mut cmd,
        cancel,
        2 * 1024 * 1024,
        "FFmpeg encoder list is too large",
    )?;
    let available = String::from_utf8_lossy(&bytes)
        .lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name));
    ensure!(
        available,
        "This FFmpeg installation does not provide the required {name} video encoder"
    );
    Ok(())
}

/// The settings frames are rendered with at `fps` frames per second under `timing`.
pub fn render_config(config: &Config, timing: Timing, fps: f64) -> Config {
    let mut c = config.clone();
    match timing {
        Timing::Stable => {
            c.phase = Phase::Stable;
            for weight in &mut c.persistence {
                *weight = weight.powf((60. / fps) as f32);
            }
        }
        Timing::Ntsc60 => c.phase = Phase::Alternating,
        Timing::Disabled => {
            c.phase = Phase::Stable;
            c.persistence = [0.; 3];
            c.warmup = 0;
        }
    }
    c
}

/// The rate frames are rendered and encoded at.
#[derive(Clone, Debug)]
pub(crate) struct Rate {
    /// As FFmpeg is told it: exact, such as 30000/1001.
    pub text: String,
    pub fps: f64,
}

impl Rate {
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
    /// The settings each frame is rendered with.
    pub render: &'p Config,
    /// The rendered size.
    pub size: (u32, u32),
    pub rate: &'p Rate,
    /// Where in the video to start, in seconds.
    pub start: f64,
    /// At most this many seconds.
    pub limit: Option<f64>,
}

impl Frames<'_> {
    /// How long `count` frames last.
    pub fn duration(&self, count: u64) -> f64 {
        count as f64 / self.rate.fps
    }

    /// How much of the video is rendered, in seconds.
    pub fn length(&self) -> f64 {
        let rest = self.video.duration - self.start;
        self.limit.map_or(rest, |limit| rest.min(limit))
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
    render: impl FnMut(&RgbaImage, &Config) -> Result<RgbaImage>,
    progress: impl FnMut(RenderProgress),
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
    render: impl FnMut(&RgbaImage, &Config) -> Result<RgbaImage>,
    progress: impl FnMut(RenderProgress),
) -> Result<()> {
    check_cancel(cancel)?;
    let format = AnimationFormat::of(output).context("Choose a GIF or WebP filename")?;
    let plan = AnimationPlan::new(video, format, config, options)?;
    run(&plan, output, cancel, render, progress)
}

/// Renders the plan's frames, encodes and finishes them, and only then replaces `output`.
fn run(
    plan: &impl Encoding,
    output: &Path,
    cancel: &Arc<AtomicBool>,
    mut render: impl FnMut(&RgbaImage, &Config) -> Result<RgbaImage>,
    mut progress: impl FnMut(RenderProgress),
) -> Result<()> {
    plan.check(cancel)?;
    let frames = plan.frames();
    let video = frames.video;
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
    let final_file = tempfile::NamedTempFile::new_in(parent)?;
    // Use a file: embedded LUTs are too large for OS command-line limits.
    let metadata = folder.path().join("preset.ffmeta");
    if let Some(text) = plan.metadata_file()? {
        std::fs::write(&metadata, text)?;
    }
    let encoded = match plan.intermediate() {
        Some(extension) => folder.path().join(format!("video.{extension}")),
        None => final_file.path().to_owned(),
    };
    let mut encoder = Process::spawn(&mut plan.encoder(&encoded), cancel)?;
    let input = encoder.stdin();
    let request = Request {
        rate: Some(frames.rate),
        start: frames.start,
        frame: None,
        limit: frames.limit,
    };
    let mut decoder = Decoder::open(video, &request, cancel)?;
    let decoded = decoder.frames();
    progress(RenderProgress {
        fraction: 0.,
        stage: "Decoding and rendering video".into(),
    });
    let count = match pipeline(
        decoded,
        input,
        video.size,
        cancel,
        |frame, count, started| {
            let result = render(frame, frames.render)?;
            ensure!(
                result.dimensions() == frames.size,
                "Renderer returned the wrong video dimensions"
            );
            let fraction = (frames.duration(count) / frames.length()).min(1.);
            let remaining = if fraction > 0. {
                started.elapsed().as_secs_f64() * (1. - fraction) / fraction
            } else {
                0.
            };
            progress(RenderProgress {
                fraction: (fraction * 0.9) as f32,
                stage: format!(
                    "Frame {count} · {:.1} FPS · approximately {:.0}s remaining",
                    count as f64 / started.elapsed().as_secs_f64().max(0.001),
                    remaining
                ),
            });
            Ok(result)
        },
    ) {
        Ok(count) => count,
        Err(Failure::Write(error)) => {
            // The encoder's own log says more than a broken pipe does.
            encoder.wait()?;
            return Err(error.into());
        }
        Err(Failure::Other(error)) => {
            decoder.kill();
            encoder.kill();
            return Err(error);
        }
    };
    decoder.wait()?;
    ensure!(count > 0, "No frames decoded");
    progress(RenderProgress {
        fraction: 0.91,
        stage: "Finishing encoding".into(),
    });
    encoder.wait()?;
    check_cancel(cancel)?;
    let work = Work {
        encoded: &encoded,
        metadata: &metadata,
        folder: folder.path(),
        destination: final_file.path(),
    };
    let mut steps = plan.finishing(&work, count);
    let total = steps.len();
    for (index, step) in steps.iter_mut().enumerate() {
        progress(RenderProgress {
            fraction: 0.95 + 0.05 * index as f32 / total as f32,
            stage: step.stage.into(),
        });
        step.run(cancel)?;
        check_cancel(cancel)?;
    }
    final_file.as_file().sync_all()?;
    final_file
        .persist(output)
        .map_err(|e| anyhow::anyhow!("Cannot publish output: {}", e.error))?;
    progress(RenderProgress {
        fraction: 1.,
        stage: format!(
            "Saved {count} frames with {:.3}s duration",
            frames.duration(count)
        ),
    });
    Ok(())
}

/// How a pipelined export stopped early.
enum Failure {
    /// Writing to the encoder failed, usually because it exited; its log has the reason.
    Write(std::io::Error),
    Other(anyhow::Error),
}

/// Frames in flight between two stages. Enough to absorb one stage's jitter; more would only
/// hold another full frame of memory each, which at 4K is 33 MB.
pub(crate) const QUEUED_FRAMES: usize = 2;

/// Decodes, renders and encodes at the same time instead of in turn: the decoder is read on
/// one thread and the encoder written on another, so the render loop only waits on them when
/// a queue between them runs empty or full. Order is kept -- each queue is first in, first
/// out -- so frames reach the encoder exactly as they left the decoder.
///
/// `render` gets each frame with its 1-based number and the time the pipeline started.
/// Returns the number of frames encoded. On `Failure::Other` the caller must kill both
/// processes, so a stage blocked on its pipe returns and the scope can end.
fn pipeline(
    mut decoded: impl Read + Send,
    mut encoded: impl Write + Send,
    (width, height): (u32, u32),
    cancel: &Arc<AtomicBool>,
    mut render: impl FnMut(&RgbaImage, u64, Instant) -> Result<RgbaImage>,
) -> std::result::Result<u64, Failure> {
    std::thread::scope(|scope| {
        let (frames_in, frames) = mpsc::sync_channel::<Result<RgbaImage>>(QUEUED_FRAMES);
        // Buffers go back to the decoder once rendered, so steady state allocates nothing.
        let (recycle, spare) = mpsc::channel::<RgbaImage>();
        scope.spawn(move || loop {
            let mut frame = spare
                .try_recv()
                .unwrap_or_else(|_| RgbaImage::new(width, height));
            let bytes = frame.as_mut();
            let read = match decoded.read(&mut bytes[..1]) {
                Ok(0) => break,
                Ok(_) => decoded
                    .read_exact(&mut bytes[1..])
                    .context("Truncated decoded video frame")
                    .map(|()| frame),
                Err(error) => Err(error.into()),
            };
            let failed = read.is_err();
            if frames_in.send(read).is_err() || failed {
                break;
            }
        });
        let (results_in, results) = mpsc::sync_channel::<RgbaImage>(QUEUED_FRAMES);
        let writer = scope.spawn(move || -> std::io::Result<()> {
            for frame in results {
                encoded.write_all(frame.as_raw())?;
            }
            // Dropping the pipe here is what tells the encoder the video has ended.
            Ok(())
        });
        let started = Instant::now();
        let mut count = 0u64;
        let rendered = (|| -> Result<bool> {
            for frame in frames.iter() {
                check_cancel(cancel)?;
                let frame = frame?;
                count += 1;
                let result = render(&frame, count, started)?;
                let _ = recycle.send(frame);
                check_cancel(cancel)?;
                if results_in.send(result).is_err() {
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        // Let the writer finish what is queued and close the pipe, then collect it.
        drop(results_in);
        drop(frames);
        let written = writer.join().expect("encoder writer panicked");
        match (rendered, written) {
            (Err(error), _) => Err(Failure::Other(error)),
            (Ok(_), Err(error)) => Err(Failure::Write(error)),
            (Ok(true), Ok(())) => Ok(count),
            (Ok(false), Ok(())) => unreachable!("the writer only stops early on an error"),
        }
    })
}

pub fn export(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &Options,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(RenderProgress),
) -> Result<()> {
    let mut sequence = Sequence::default();
    export_with(
        video,
        output,
        config,
        options,
        cancel,
        |frame, config| renderer.render_frame(frame, config, &mut sequence, Some(cancel), |_| {}),
        progress,
    )
}

/// An animated GIF or WebP, as `output`'s extension asks, rendered on `renderer`.
pub fn export_animation(
    video: &Video,
    output: &Path,
    config: &Config,
    options: &AnimationOptions,
    renderer: &Renderer,
    cancel: &Arc<AtomicBool>,
    progress: impl FnMut(RenderProgress),
) -> Result<()> {
    let mut sequence = Sequence::default();
    export_animation_with(
        video,
        output,
        config,
        options,
        cancel,
        |frame, config| renderer.render_frame(frame, config, &mut sequence, Some(cancel), |_| {}),
        progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// `count` 2x2 frames, each filled with its own index.
    fn numbered_frames(count: u8) -> Vec<u8> {
        (0..count).flat_map(|i| [i; 16]).collect()
    }

    #[test]
    fn pipeline_keeps_frame_order_and_counts_what_it_encodes() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut encoded = Vec::new();
        let mut seen = vec![];
        let frames = pipeline(
            std::io::Cursor::new(numbered_frames(40)),
            &mut encoded,
            (2, 2),
            &cancel,
            |frame, count, _| {
                seen.push(count);
                // The stages overlap, so a slow render must not let frames overtake it.
                if count % 7 == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Ok(frame.clone())
            },
        );
        assert!(matches!(frames, Ok(40)));
        assert_eq!(seen, (1..=40).collect::<Vec<_>>());
        assert_eq!(encoded, numbered_frames(40));
    }

    #[test]
    fn pipeline_reports_each_way_it_can_stop() {
        let cancel = Arc::new(AtomicBool::new(false));
        let identity = |frame: &RgbaImage, _, _| Ok(frame.clone());
        let mut truncated = numbered_frames(3);
        truncated.pop();
        match pipeline(
            std::io::Cursor::new(truncated),
            std::io::sink(),
            (2, 2),
            &cancel,
            identity,
        ) {
            Err(Failure::Other(error)) => assert!(error.to_string().contains("Truncated")),
            _ => panic!("a truncated frame must fail"),
        }
        match pipeline(
            std::io::Cursor::new(numbered_frames(5)),
            std::io::sink(),
            (2, 2),
            &cancel,
            |_, count, _| {
                ensure!(count < 3, "render failed");
                Ok(RgbaImage::new(2, 2))
            },
        ) {
            Err(Failure::Other(error)) => assert_eq!(error.to_string(), "render failed"),
            _ => panic!("a render error must stop the pipeline"),
        }
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        match pipeline(
            std::io::Cursor::new(numbered_frames(50)),
            Closed,
            (2, 2),
            &cancel,
            identity,
        ) {
            Err(Failure::Write(error)) => {
                assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe)
            }
            _ => panic!("an encoder that stops reading must fail the export"),
        }
        cancel.store(true, Ordering::Relaxed);
        match pipeline(
            std::io::Cursor::new(numbered_frames(5)),
            std::io::sink(),
            (2, 2),
            &cancel,
            identity,
        ) {
            Err(Failure::Other(error)) => assert!(error.to_string().contains("cancel")),
            _ => panic!("cancellation must stop the pipeline"),
        }
    }

    #[test]
    fn persistence_uses_media_time() {
        let config = Config::default();
        let at30 = render_config(&config, Timing::Stable, 30.);
        let at60 = render_config(&config, Timing::Stable, 60.);
        assert!((at30.persistence[0] - at60.persistence[0].powi(2)).abs() < 0.00001);
        let disabled = render_config(&config, Timing::Disabled, 30.);
        assert_eq!(disabled.persistence, [0.; 3]);
    }
}
