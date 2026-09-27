use crate::{file_name, files};
use anyhow::{ensure, Result};
use crtsim_core::{config::Config, nes_luts, RenderProgress, Renderer, Sequence};
use crtsim_media::{Options, Video};
use eframe::egui;
use image::RgbaImage;
use std::{
    collections::VecDeque,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// Where the worker's renderer gets its device.
#[derive(Clone)]
pub enum Gpu {
    /// Make one. For callers with no window -- the tests, and any platform where the
    /// interface could not hand its own device over.
    Own(wgpu::Backends),
    /// Render on the device the interface draws with, so a finished frame can reach the
    /// screen without a round trip through system memory.
    Shared(eframe::egui_wgpu::RenderState),
}
/// Held while a renderer is built. Construction validates its pipelines inside an error scope,
/// and wgpu keeps one scope stack per device, not per thread: two workers building at once on
/// a shared device could each pop the other's scope and report the wrong error, or none.
static BUILDING: Mutex<()> = Mutex::new(());

impl Gpu {
    fn renderer(&self) -> Result<Renderer> {
        let _building = BUILDING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match self {
            Self::Own(backends) => pollster::block_on(Renderer::new(*backends)),
            Self::Shared(state) => pollster::block_on(Renderer::with_device(
                state.device.clone(),
                state.queue.clone(),
                state.adapter.get_info(),
            )),
        }
    }
    /// The interface's own device, where there is one. Only a frame rendered on it can be
    /// drawn without being copied through system memory first.
    pub fn render_state(&self) -> Option<&eframe::egui_wgpu::RenderState> {
        match self {
            Self::Own(_) => None,
            Self::Shared(state) => Some(state),
        }
    }
}

/// A thread's renderer: made when a job first needs one, and made again after a driver fails.
struct Graphics {
    gpu: Gpu,
    renderer: Option<Renderer>,
}

impl Graphics {
    fn new(gpu: Gpu) -> Self {
        Self {
            gpu,
            renderer: None,
        }
    }

    /// The renderer, made on first use. `starting` is told when it is being made, which takes
    /// a moment.
    fn renderer(&mut self, starting: impl FnOnce()) -> Result<&Renderer> {
        if self.renderer.is_none() {
            starting();
            self.renderer = Some(self.gpu.renderer()?);
        }
        Ok(self.renderer.as_ref().expect("made above"))
    }

    /// Runs `work`, turning a panic -- a graphics driver failing under it -- into the error
    /// `failure` and dropping the renderer it may have broken, so the next job makes a new one.
    /// The job is lost; the settings stay usable.
    fn guard<T>(&mut self, failure: &str, work: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let this = &mut *self;
        std::panic::catch_unwind(AssertUnwindSafe(|| work(this))).unwrap_or_else(|_| {
            self.renderer = None;
            Err(anyhow::anyhow!("{failure}"))
        })
    }
}

const STILL_FAILED: &str = "Graphics driver failed. Try a smaller resolution or restart with \
                            another --backend. Settings are still available to save.";

/// Where the interface sends work. Previews have a thread of their own, so they keep coming
/// while an export holds the other one for minutes: a preview queued behind an export used to
/// wait for all of it, and settings edited meanwhile could not be seen until it finished.
#[derive(Clone)]
pub struct Jobs {
    work: mpsc::Sender<Job>,
    preview: mpsc::Sender<PreviewJob>,
}
/// A job could not be sent because its worker has stopped.
#[derive(Debug)]
pub struct Stopped;
impl Jobs {
    pub fn send(&self, job: Job) -> Result<(), Stopped> {
        self.work.send(job).map_err(|_| Stopped)
    }
    pub fn preview(&self, job: PreviewJob) -> Result<(), Stopped> {
        self.preview.send(job).map_err(|_| Stopped)
    }
    /// Asks both threads to stop once their current job is done.
    pub fn shutdown(&self) {
        let _ = self.preview.send(PreviewJob::Shutdown);
        let _ = self.work.send(Job::Shutdown);
    }
    /// Channels in place of the threads, for a test to inspect what the interface sends.
    #[cfg(test)]
    pub fn capture() -> (Self, mpsc::Receiver<Job>, mpsc::Receiver<PreviewJob>) {
        let (work, jobs) = mpsc::channel();
        let (preview, previews) = mpsc::channel();
        (Self { work, preview }, jobs, previews)
    }
}

/// Work for the preview thread.
pub enum PreviewJob {
    Preview {
        revision: u64,
        input: Arc<RgbaImage>,
        config: Box<Config>,
    },
    /// A small picture for a gallery entry. Always waits for a pending preview, and is dropped
    /// unrendered once a thumbnail of a newer source is queued behind it.
    Thumbnail {
        generation: u64,
        key: ThumbnailKey,
        input: Arc<RgbaImage>,
        look: Look,
    },
    Shutdown,
}

/// Work for the other thread: loading, exports and playback, one at a time.
pub enum Job {
    Load(PathBuf),
    LoadVideo {
        path: PathBuf,
        frame: u64,
        /// The video and its frame count, when a frame of the one already open is wanted.
        cached: Option<(Video, u64)>,
        cancel: Arc<AtomicBool>,
    },
    ImportPreset {
        path: PathBuf,
        input: (u32, u32),
        cancel: Arc<AtomicBool>,
    },
    /// Rendered and saved to `path`, which is only replaced once the whole file is ready.
    Export {
        export: Export,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    Playback {
        video: Video,
        config: Config,
        options: Options,
        feed: Feed,
    },
    Shutdown,
}

/// What an export renders, each with the settings captured when it was asked for.
pub enum Export {
    /// A still, saved as a PNG that carries its settings.
    Image {
        input: Arc<RgbaImage>,
        config: Config,
    },
    /// A whole video, with its audio and other tracks.
    Video {
        video: Video,
        options: Options,
        config: Config,
    },
    /// An animated GIF or WebP, as the path's extension asks.
    Animation {
        video: Video,
        options: crtsim_media::AnimationOptions,
        config: Config,
    },
    /// A batch job, whose source is only read once it runs. The queue chose a video or a PNG
    /// when it named the output.
    Batch {
        source: PathBuf,
        options: Options,
        config: Config,
    },
}

impl Export {
    /// Reported when the graphics driver fails under the export.
    fn driver_failed(&self) -> &'static str {
        match self {
            Self::Image { .. } => STILL_FAILED,
            Self::Video { .. } => "Video graphics driver failed. Try a smaller resolution.",
            Self::Animation { .. } => "Animation graphics driver failed. Try a smaller size.",
            Self::Batch { .. } => "Batch graphics driver failed. Try a smaller resolution.",
        }
    }

    fn run(
        &self,
        graphics: &mut Graphics,
        path: &Path,
        cancel: &Arc<AtomicBool>,
        progress: &dyn Fn(RenderProgress),
    ) -> Result<()> {
        match self {
            Self::Image { input, config } => {
                export_image(graphics, input, config, path, cancel, progress)
            }
            Self::Video {
                video,
                options,
                config,
            } => export_video(graphics, video, config, options, path, cancel, progress),
            Self::Animation {
                video,
                options,
                config,
            } => {
                let renderer = graphics.renderer(|| {})?;
                crtsim_media::export_animation(
                    video, path, config, options, renderer, cancel, progress,
                )
            }
            Self::Batch {
                source,
                options,
                config,
            } => {
                if crtsim_media::Container::of(path).is_some() {
                    let video = crtsim_media::probe(source, cancel)?;
                    export_video(graphics, &video, config, options, path, cancel, progress)
                } else {
                    let input = crtsim_core::input::load_image(source)?;
                    export_image(graphics, &input, config, path, cancel, progress)
                }
            }
        }
    }
}

/// Which gallery entry a thumbnail belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThumbnailKey {
    Preset(String),
    Lut(usize),
}
/// What a thumbnail shows.
pub enum Look {
    /// The whole CRT picture with these settings, at thumbnail size.
    Crt(Box<Config>),
    /// Only an included LUT's color mapping, applied to an input already at thumbnail size:
    /// that is all a LUT changes, and it needs no GPU and no re-render when other settings move.
    Lut(usize),
}
pub struct PlaybackFrame {
    pub time: f64,
    pub source: RgbaImage,
    pub crt: RgbaImage,
}
/// The worker's end of a playback: where in the video to start, where to send the frames it
/// renders, and the flag that says to stop.
pub struct Feed {
    pub start: f64,
    pub frames: mpsc::SyncSender<Result<PlaybackFrame, String>>,
    pub cancel: Arc<AtomicBool>,
}
/// A finished preview. A renderer on its own device cannot hand its textures to the
/// interface, so it still sends pixels.
pub enum Preview {
    Pixels(RgbaImage),
    Frame(crtsim_core::PreviewFrame),
}
impl Preview {
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Pixels(image) => image.dimensions(),
            Self::Frame(frame) => (frame.width, frame.height),
        }
    }
}

/// How a cancellable job ended without its result.
#[derive(Debug)]
pub enum Failure {
    /// Stopped on request; nothing went wrong.
    Cancelled,
    Failed(String),
}
impl Failure {
    /// A job's error, which is a cancellation when the job had been asked to stop.
    fn of(error: anyhow::Error, cancel: &AtomicBool) -> Self {
        if cancel.load(Ordering::Relaxed) {
            Self::Cancelled
        } else {
            Self::Failed(format!("{error:#}"))
        }
    }
}
pub type Outcome<T> = std::result::Result<T, Failure>;

/// A source opened for editing: an image, or one frame of a video.
pub struct Loaded {
    /// Where it is, with links resolved.
    pub path: PathBuf,
    /// What to call it.
    pub name: String,
    pub image: RgbaImage,
    /// The image made opaque and at most 2048 pixels on a side, for showing the original.
    pub thumbnail: RgbaImage,
    /// For a frame of a video, the video and where in it the frame is.
    pub video: Option<VideoFrame>,
}
pub struct VideoFrame {
    pub video: Video,
    pub frame: u64,
    /// How many frames the video decodes to.
    pub frames: u64,
}
pub struct ImportedPreset {
    pub path: PathBuf,
    pub config: Config,
    /// Present for a preset read from a video, which also records how it was encoded.
    pub options: Option<Options>,
}
pub struct Previewed {
    pub image: Preview,
    pub seconds: f32,
}

pub enum Event {
    Progress(RenderProgress),
    Loaded(Outcome<Loaded>),
    PresetImported(Outcome<ImportedPreset>),
    Preview {
        revision: u64,
        result: std::result::Result<Previewed, String>,
    },
    Exported(Outcome<PathBuf>),
    Thumbnail {
        generation: u64,
        key: ThumbnailKey,
        result: std::result::Result<RgbaImage, String>,
    },
}
/// Starts the two worker threads. The handle returned joins both.
pub fn start(
    ctx: egui::Context,
    gpu: Gpu,
) -> (Jobs, mpsc::Receiver<Event>, std::thread::JoinHandle<()>) {
    let (send, jobs) = mpsc::channel();
    let (preview_send, previews) = mpsc::channel();
    let (events, receive) = mpsc::channel();
    let preview_thread = {
        let (ctx, gpu, events) = (ctx.clone(), gpu.clone(), events.clone());
        std::thread::spawn(move || preview_worker(ctx, gpu, previews, events))
    };
    let thread = std::thread::spawn(move || {
        work(ctx, gpu, jobs, events);
        let _ = preview_thread.join();
    });
    (
        Jobs {
            work: send,
            preview: preview_send,
        },
        receive,
        thread,
    )
}

/// Previews only, with a renderer of its own. On a shared device the two renderers draw on the
/// same GPU, so a preview taken during an export slows that export somewhat, and the export
/// slows the preview; neither waits for the other to finish.
fn preview_worker(
    ctx: egui::Context,
    gpu: Gpu,
    jobs: mpsc::Receiver<PreviewJob>,
    events: mpsc::Sender<Event>,
) {
    let mut graphics = Graphics::new(gpu);
    let mut backlog = VecDeque::new();
    loop {
        // Collect everything waiting, blocking only when there is nothing to do.
        if backlog.is_empty() {
            match jobs.recv() {
                Ok(job) => backlog.push_back(job),
                Err(_) => return,
            }
        }
        backlog.extend(jobs.try_iter());
        let Some(job) = next_job(&mut backlog) else {
            continue;
        };
        let event = match job {
            PreviewJob::Shutdown => return,
            PreviewJob::Preview {
                revision,
                input,
                config,
            } => {
                let started = Instant::now();
                let result = preview(&mut graphics, &input, &config)
                    .map(|image| Previewed {
                        image,
                        seconds: started.elapsed().as_secs_f32(),
                    })
                    .map_err(|e| format!("{e:#}"));
                Event::Preview { revision, result }
            }
            PreviewJob::Thumbnail {
                generation,
                key,
                input,
                look,
            } => {
                let result = match look {
                    Look::Crt(config) => {
                        graphics.guard(STILL_FAILED, |g| still(g, &input, &config, None, |_| {}))
                    }
                    Look::Lut(index) => nes_luts::load(index).map(|lut| {
                        let mut image = (*input).clone();
                        lut.apply(&mut image);
                        image
                    }),
                };
                Event::Thumbnail {
                    generation,
                    key,
                    result: result.map_err(|e| format!("{e:#}")),
                }
            }
        };
        if events.send(event).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The preview thread's next job: shutdown before anything, then the preview someone is
/// waiting to see, then thumbnails in the order asked for, skipping any whose source has since
/// been replaced.
fn next_job(backlog: &mut VecDeque<PreviewJob>) -> Option<PreviewJob> {
    if backlog
        .iter()
        .any(|job| matches!(job, PreviewJob::Shutdown))
    {
        backlog.clear();
        return Some(PreviewJob::Shutdown);
    }
    let newest = backlog
        .iter()
        .filter_map(|job| match job {
            PreviewJob::Thumbnail { generation, .. } => Some(*generation),
            _ => None,
        })
        .max();
    backlog.retain(|job| {
        !matches!(job, PreviewJob::Thumbnail { generation, .. } if Some(*generation) < newest)
    });
    match backlog
        .iter()
        .position(|job| matches!(job, PreviewJob::Preview { .. }))
    {
        Some(index) => backlog.remove(index),
        None => backlog.pop_front(),
    }
}

/// Everything but previews: loading, exports, batches and playback, one at a time.
fn work(ctx: egui::Context, gpu: Gpu, jobs: mpsc::Receiver<Job>, events: mpsc::Sender<Event>) {
    let mut graphics = Graphics::new(gpu);
    let progress = |progress: RenderProgress| {
        let _ = events.send(Event::Progress(progress));
        ctx.request_repaint();
    };
    while let Ok(job) = jobs.recv() {
        let event = match job {
            Job::Shutdown => break,
            Job::Playback {
                video,
                config,
                options,
                feed,
            } => {
                play(&mut graphics, &ctx, &video, &config, &options, &feed);
                continue;
            }
            // Loading an image cannot be cancelled.
            Job::Load(path) => {
                Event::Loaded(load_image(path).map_err(|e| Failure::Failed(format!("{e:#}"))))
            }
            Job::LoadVideo {
                path,
                frame,
                cached,
                cancel,
            } => Event::Loaded(
                load_video(&path, frame, cached, &cancel).map_err(|e| Failure::of(e, &cancel)),
            ),
            Job::ImportPreset {
                path,
                input,
                cancel,
            } => Event::PresetImported(
                import_preset(path, input, &cancel).map_err(|e| Failure::of(e, &cancel)),
            ),
            Job::Export {
                export,
                path,
                cancel,
            } => Event::Exported(
                graphics
                    .guard(export.driver_failed(), |g| {
                        export.run(g, &path, &cancel, &progress)
                    })
                    .map(|()| path)
                    .map_err(|e| Failure::of(e, &cancel)),
            ),
        };
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}

fn load_image(path: PathBuf) -> Result<Loaded> {
    let image = crtsim_core::input::load_image(&path)?;
    // Keep thumbnail processing off the UI thread, including alpha before resizing.
    let mut opaque = image.clone();
    crtsim_core::config::flatten_alpha(&mut opaque, [0; 3]);
    let thumbnail = image::DynamicImage::ImageRgba8(opaque)
        .thumbnail(2048, 2048)
        .to_rgba8();
    Ok(Loaded {
        name: file_name(&path),
        path: path.canonicalize().unwrap_or(path),
        image,
        thumbnail,
        video: None,
    })
}

fn load_video(
    path: &Path,
    frame: u64,
    cached: Option<(Video, u64)>,
    cancel: &Arc<AtomicBool>,
) -> Result<Loaded> {
    let (video, frames) = match cached {
        Some(cached) => cached,
        None => {
            let video = crtsim_media::probe(path, cancel)?;
            let frames = crtsim_media::frame_count(&video, cancel)?;
            (video, frames)
        }
    };
    ensure!(frame < frames, "Frame is outside the video");
    let image = crtsim_media::preview_frame(&video, frame, cancel)?;
    let thumbnail = image::DynamicImage::ImageRgba8(image.clone())
        .thumbnail(2048, 2048)
        .to_rgba8();
    Ok(Loaded {
        path: video.path.clone(),
        name: file_name(&video.path),
        image,
        thumbnail,
        video: Some(VideoFrame {
            video,
            frame,
            frames,
        }),
    })
}

fn import_preset(
    path: PathBuf,
    input: (u32, u32),
    cancel: &Arc<AtomicBool>,
) -> Result<ImportedPreset> {
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        let config = files::load_preset_from_image(&path, input)?;
        Ok(ImportedPreset {
            path,
            config,
            options: None,
        })
    } else {
        let preset = crtsim_media::import_preset(&path, input, cancel)?;
        Ok(ImportedPreset {
            path,
            config: preset.config,
            options: Some(preset.video_options),
        })
    }
}

/// A still from fresh history on this thread's renderer.
fn still(
    graphics: &mut Graphics,
    input: &RgbaImage,
    c: &Config,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(RenderProgress),
) -> Result<RgbaImage> {
    let renderer = graphics.renderer(|| {
        progress(RenderProgress {
            fraction: 0.,
            stage: "Initializing graphics device".into(),
        })
    })?;
    renderer.render_frame(input, c, &mut Sequence::default(), cancel, progress)
}

/// A frame for the screen. On the interface's own device it is left there; otherwise it is
/// read back, which is what the interface then has to upload again.
fn preview(graphics: &mut Graphics, input: &RgbaImage, c: &Config) -> Result<Preview> {
    graphics.guard("Graphics device failed while rendering the preview", |g| {
        if g.gpu.render_state().is_none() {
            return still(g, input, c, None, |_| {}).map(Preview::Pixels);
        }
        g.renderer(|| {})?
            .render_preview(input, c)
            .map(Preview::Frame)
    })
}

/// A still at full resolution, saved as a PNG that carries its settings.
fn export_image(
    graphics: &mut Graphics,
    input: &RgbaImage,
    config: &Config,
    path: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(RenderProgress),
) -> Result<()> {
    let image = still(graphics, input, config, Some(cancel), |mut stage| {
        stage.fraction *= 0.9;
        progress(stage);
    })?;
    ensure!(!cancel.load(Ordering::Relaxed), "Render cancelled");
    progress(RenderProgress {
        fraction: 0.95,
        stage: "Encoding and saving PNG".into(),
    });
    files::save_png(path, image, Some(config))
}

/// A whole video rendered and encoded, with its audio and other tracks.
fn export_video(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &Options,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(RenderProgress),
) -> Result<()> {
    let renderer = graphics.renderer(|| {})?;
    crtsim_media::export(video, path, config, options, renderer, cancel, progress)
}

/// Streams rendered frames of `video` into the feed, which the interface plays from. An error
/// is queued behind the frames already there, unless playback has been stopped meanwhile.
fn play(
    graphics: &mut Graphics,
    ctx: &egui::Context,
    video: &Video,
    config: &Config,
    options: &Options,
    feed: &Feed,
) {
    let Feed {
        start,
        frames,
        cancel,
    } = feed;
    let played = graphics.guard("Playback graphics driver failed", |g| {
        let renderer = g.renderer(|| {})?;
        crtsim_media::playback(
            video,
            *start,
            config,
            options,
            renderer,
            cancel,
            |time, source, crt| {
                let mut item = Ok(PlaybackFrame { time, source, crt });
                loop {
                    ensure!(!cancel.load(Ordering::Relaxed), "Playback cancelled");
                    match frames.try_send(item) {
                        Ok(()) => {
                            ctx.request_repaint();
                            return Ok(());
                        }
                        Err(mpsc::TrySendError::Full(back)) => {
                            item = back;
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(mpsc::TrySendError::Disconnected(_)) => {
                            anyhow::bail!("Playback stopped")
                        }
                    }
                }
            },
        )
    });
    if let Err(error) = played {
        let error = format!("{error:#}");
        // Keep the terminal error behind already buffered frames without blocking shutdown.
        while !cancel.load(Ordering::Relaxed) {
            match frames.try_send(Err(error.clone())) {
                Err(mpsc::TrySendError::Full(_)) => std::thread::sleep(Duration::from_millis(5)),
                _ => break,
            }
        }
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumbnail(generation: u64, index: usize) -> PreviewJob {
        PreviewJob::Thumbnail {
            generation,
            key: ThumbnailKey::Lut(index),
            input: Arc::new(RgbaImage::new(1, 1)),
            look: Look::Lut(index),
        }
    }

    #[test]
    fn a_preview_goes_before_thumbnails_and_stale_thumbnails_are_dropped() {
        let mut backlog: VecDeque<PreviewJob> = [
            thumbnail(1, 0),
            thumbnail(1, 1),
            PreviewJob::Preview {
                revision: 3,
                input: Arc::new(RgbaImage::new(1, 1)),
                config: Box::default(),
            },
            thumbnail(2, 0),
        ]
        .into();
        assert!(matches!(
            next_job(&mut backlog),
            Some(PreviewJob::Preview { revision: 3, .. })
        ));
        // The source changed after the first two were asked for: only the newer one is left.
        assert!(matches!(
            next_job(&mut backlog),
            Some(PreviewJob::Thumbnail { generation: 2, .. })
        ));
        assert!(next_job(&mut backlog).is_none());
        backlog.extend([thumbnail(3, 0), PreviewJob::Shutdown]);
        assert!(
            matches!(next_job(&mut backlog), Some(PreviewJob::Shutdown)),
            "shutdown is never kept waiting"
        );
        assert!(backlog.is_empty());
    }

    /// The point of the second thread: a preview finishes while an export is still running,
    /// where it used to wait in the queue behind all of it.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn a_preview_finishes_while_an_export_is_still_running() {
        let (jobs, events, thread) =
            start(egui::Context::default(), Gpu::Own(wgpu::Backends::VULKAN));
        let dir = tempfile::tempdir().unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let input = Arc::new(crtsim_core::config::test_card());
        // Long enough to still be running when the preview is done: the maximum warm-up, at 4K.
        jobs.send(Job::Export {
            export: Export::Image {
                input: input.clone(),
                config: Config {
                    output: "4k".into(),
                    warmup: 240,
                    ..Config::default()
                },
            },
            path: dir.path().join("export.png"),
            cancel: cancel.clone(),
        })
        .unwrap();
        jobs.preview(PreviewJob::Preview {
            revision: 7,
            input,
            config: Box::new(Config {
                output: "320x180".into(),
                warmup: 0,
                ..Config::default()
            }),
        })
        .unwrap();
        let timeout = Duration::from_secs(120);
        loop {
            match events.recv_timeout(timeout).expect("worker stalled") {
                Event::Progress(_) => continue,
                Event::Preview { revision, result } => {
                    assert_eq!(revision, 7);
                    assert!(result.is_ok(), "preview failed");
                    break;
                }
                Event::Exported(_) => panic!("the export finished before the preview"),
                _ => panic!("unexpected event"),
            }
        }
        cancel.store(true, Ordering::Relaxed);
        loop {
            match events.recv_timeout(timeout).expect("worker stalled") {
                Event::Exported(result) => {
                    assert!(
                        matches!(result, Err(Failure::Cancelled)),
                        "export should have been cancelled"
                    );
                    break;
                }
                _ => continue,
            }
        }
        jobs.shutdown();
        thread.join().unwrap();
    }
}
