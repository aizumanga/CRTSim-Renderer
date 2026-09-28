use crate::{file_name, files, timeline::Timeline};
use anyhow::{ensure, Result};
use crtsim_core::{config::Config, nes_luts, RenderProgress, Renderer, Sequence, Stage};
use crtsim_media::{Options, Progress, Video};
use eframe::egui;
use image::RgbaImage;
use std::{
    collections::VecDeque,
    future::Future,
    panic::AssertUnwindSafe,
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    task::{Poll, Waker},
    time::Duration,
};
use web_time::Instant;

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
impl Gpu {
    /// A renderer on this device. Building one validates its pipelines in an error scope, which
    /// wgpu keeps per thread and which opens and closes with no await between, so renderers can
    /// be built at once on the lanes' threads, or one after another as tasks on one thread.
    async fn renderer(&self) -> Result<Renderer> {
        match self {
            Self::Own(backends) => Renderer::new(*backends).await,
            Self::Shared(state) => {
                Renderer::with_device(
                    Arc::new(state.device.clone()),
                    Arc::new(state.queue.clone()),
                    state.adapter.get_info(),
                )
                .await
            }
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
    async fn renderer(&mut self, starting: impl FnOnce()) -> Result<&Renderer> {
        if self.renderer.is_none() {
            starting();
            self.renderer = Some(self.gpu.renderer().await?);
        }
        Ok(self.renderer.as_ref().expect("made above"))
    }

    /// Runs `work`, turning a panic -- a graphics driver failing under it -- into the error
    /// `failure` and dropping the renderer it may have broken, so the next job makes a new one.
    /// The job is lost; the settings stay usable.
    async fn guard<T>(
        &mut self,
        failure: &str,
        work: impl AsyncFnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let caught = {
            let mut job = std::pin::pin!(work(self));
            std::future::poll_fn(|cx| {
                match std::panic::catch_unwind(AssertUnwindSafe(|| job.as_mut().poll(cx))) {
                    Ok(Poll::Pending) => Poll::Pending,
                    Ok(Poll::Ready(result)) => Poll::Ready(Some(result)),
                    Err(_) => Poll::Ready(None),
                }
            })
            .await
        };
        caught.unwrap_or_else(|| {
            self.renderer = None;
            Err(anyhow::anyhow!("{failure}"))
        })
    }
}

const STILL_FAILED: &str = "Graphics driver failed. Try a smaller resolution or restart with \
                            another --backend. Settings are still available to save.";

/// Where the interface sends work. The worker has two lanes: previews have one of their own,
/// so they keep coming while an export holds the other for minutes. A preview queued behind an
/// export used to wait for all of it, and settings edited meanwhile could not be seen until it
/// finished.
#[derive(Clone)]
pub struct Jobs {
    work: mpsc::Sender<Job>,
    preview: mpsc::Sender<PreviewJob>,
    /// Last, so it rings once the senders above are gone.
    bells: Bells,
}
/// A job could not be sent because its worker has stopped.
#[derive(Debug)]
pub struct Stopped;
impl Jobs {
    pub fn send(&self, job: Job) -> Result<(), Stopped> {
        self.work.send(job).map_err(|_| Stopped)?;
        self.bells.0[0].ring();
        Ok(())
    }
    pub fn preview(&self, job: PreviewJob) -> Result<(), Stopped> {
        self.preview.send(job).map_err(|_| Stopped)?;
        self.bells.0[1].ring();
        Ok(())
    }
    /// Asks both lanes to stop once their current job is done.
    pub fn shutdown(&self) {
        let _ = self.preview(PreviewJob::Shutdown);
        let _ = self.send(Job::Shutdown);
    }
    /// Channels in place of the lanes, for a test to inspect what the interface sends.
    #[cfg(test)]
    pub fn capture() -> (Self, mpsc::Receiver<Job>, mpsc::Receiver<PreviewJob>) {
        let (work, jobs) = mpsc::channel();
        let (preview, previews) = mpsc::channel();
        (
            Self {
                work,
                preview,
                bells: Bells::default(),
            },
            jobs,
            previews,
        )
    }
}

/// The work lane's bell and the preview lane's. Dropped with the interface's end of the
/// channels, after them, they wake the lanes to see the channels closed and stop: a lane waiting
/// for a job would otherwise wait forever.
#[derive(Clone, Default)]
struct Bells([Arc<Bell>; 2]);

impl Drop for Bells {
    fn drop(&mut self) {
        for bell in &self.0 {
            bell.ring();
        }
    }
}

/// Wakes a lane waiting for its next job. A lane on a thread of its own sleeps until then; one
/// sharing the host's thread as a task gives it back.
#[derive(Default)]
struct Bell(Mutex<Option<Waker>>);

impl Bell {
    fn ring(&self) {
        if let Some(waker) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            waker.wake();
        }
    }
}

/// A lane's end of its channel.
struct Inbox<T> {
    jobs: mpsc::Receiver<T>,
    bell: Arc<Bell>,
}

impl<T> Inbox<T> {
    /// The next job, or `None` once the interface has gone.
    async fn next(&self) -> Option<T> {
        std::future::poll_fn(|cx| {
            let take = || match self.jobs.try_recv() {
                Ok(job) => Some(Some(job)),
                Err(mpsc::TryRecvError::Disconnected) => Some(None),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(job) = take() {
                return Poll::Ready(job);
            }
            *self.bell.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
            // A job sent between the look and the waker going in has already rung.
            take().map_or(Poll::Pending, Poll::Ready)
        })
        .await
    }

    /// Every job already waiting.
    fn waiting(&self) -> impl Iterator<Item = T> + '_ {
        self.jobs.try_iter()
    }
}

/// A lane as a task, which runs on the thread it was made on.
pub type Task = Pin<Box<dyn Future<Output = ()>>>;

/// How the worker's two lanes run.
pub enum Runtime {
    /// Each lane on a thread of its own, sleeping between jobs: the desktop.
    Threads,
    /// Each lane a task on the host's own thread, handed to this to run. The browser has no
    /// threads; the lanes take turns there, at every await.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop runs its lanes on threads")
    )]
    Tasks(Rc<dyn Fn(Task)>),
}

impl Runtime {
    /// Threads on the desktop; tasks in the browser, which has only the page's thread.
    pub fn host() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Self::Threads;
        #[cfg(target_arch = "wasm32")]
        return Self::Tasks(Rc::new(wasm_bindgen_futures::spawn_local));
    }
}

/// The worker a running interface talks to.
pub struct Worker {
    pub jobs: Jobs,
    pub events: mpsc::Receiver<Event>,
    /// Joins the lanes' threads once they stop, where they have threads.
    pub threads: Option<std::thread::JoinHandle<()>>,
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
    /// An image handed over by name and contents, as a browser gives a file picked or dropped.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop opens files by path")
    )]
    LoadBytes {
        name: String,
        bytes: Vec<u8>,
    },
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

    async fn run(
        &self,
        graphics: &mut Graphics,
        path: &Path,
        cancel: &Arc<AtomicBool>,
        progress: &dyn Fn(Progress),
    ) -> Result<()> {
        match self {
            Self::Image { input, config } => {
                export_image(graphics, input, config, path, cancel, progress).await
            }
            Self::Video {
                video,
                options,
                config,
            } => export_video(graphics, video, config, options, path, cancel, progress).await,
            Self::Animation {
                video,
                options,
                config,
            } if video.contents.is_some() => {
                export_animation_here(graphics, video, config, options, path, cancel, progress)
                    .await
            }
            Self::Animation {
                video,
                options,
                config,
            } => {
                let renderer = graphics.renderer(|| {}).await?;
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
                    export_video(graphics, &video, config, options, path, cancel, progress).await
                } else {
                    let input = crtsim_core::input::load_image(source)?;
                    export_image(graphics, &input, config, path, cancel, progress).await
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
    Failed(anyhow::Error),
}
impl Failure {
    /// A job's error, which is a cancellation when the job had been asked to stop.
    fn of(error: anyhow::Error, cancel: &AtomicBool) -> Self {
        if cancel.load(Ordering::Relaxed) {
            Self::Cancelled
        } else {
            Self::Failed(error)
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
    pub timeline: Option<Timeline>,
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
    Progress(Progress),
    Loaded(Outcome<Loaded>),
    PresetImported(Outcome<ImportedPreset>),
    Preview {
        revision: u64,
        result: Result<Previewed>,
    },
    Exported(Outcome<PathBuf>),
    Thumbnail {
        generation: u64,
        key: ThumbnailKey,
        result: Result<RgbaImage>,
    },
}
/// Starts the worker's two lanes on `runtime`.
pub fn start(ctx: egui::Context, gpu: Gpu, runtime: Runtime) -> Worker {
    let (work, jobs) = mpsc::channel();
    let (preview, previews) = mpsc::channel();
    let (events, receive) = mpsc::channel();
    let bells = Bells::default();
    let jobs = Inbox {
        jobs,
        bell: bells.0[0].clone(),
    };
    let previews = Inbox {
        jobs: previews,
        bell: bells.0[1].clone(),
    };
    let (preview_ctx, preview_gpu, preview_events) = (ctx.clone(), gpu.clone(), events.clone());
    let threads = match runtime {
        // Each lane is made on its own thread: a lane holds what only one thread may use.
        Runtime::Threads => {
            let previews = std::thread::spawn(move || {
                pollster::block_on(preview_lane(
                    preview_ctx,
                    preview_gpu,
                    previews,
                    preview_events,
                ))
            });
            Some(std::thread::spawn(move || {
                pollster::block_on(work_lane(ctx, gpu, jobs, events));
                let _ = previews.join();
            }))
        }
        Runtime::Tasks(spawn) => {
            spawn(Box::pin(work_lane(ctx, gpu, jobs, events)));
            spawn(Box::pin(preview_lane(
                preview_ctx,
                preview_gpu,
                previews,
                preview_events,
            )));
            None
        }
    };
    Worker {
        jobs: Jobs {
            work,
            preview,
            bells,
        },
        events: receive,
        threads,
    }
}

/// Previews and thumbnails, with a renderer of its own. On a shared device the two lanes'
/// renderers draw on the same GPU, so a preview taken during an export slows that export
/// somewhat, and the export slows the preview; neither waits for the other to finish.
async fn preview_lane(
    ctx: egui::Context,
    gpu: Gpu,
    jobs: Inbox<PreviewJob>,
    events: mpsc::Sender<Event>,
) {
    let mut graphics = Graphics::new(gpu);
    let mut backlog = VecDeque::new();
    loop {
        // Collect everything waiting, waiting only when there is nothing to do.
        if backlog.is_empty() {
            match jobs.next().await {
                Some(job) => backlog.push_back(job),
                None => return,
            }
        }
        backlog.extend(jobs.waiting());
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
                    .await
                    .map(|image| Previewed {
                        image,
                        seconds: started.elapsed().as_secs_f32(),
                    });
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
                        graphics
                            .guard(STILL_FAILED, async |g| {
                                still(g, &input, &config, None, |_| {}).await
                            })
                            .await
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
                    result,
                }
            }
        };
        if events.send(event).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The preview lane's next job: shutdown before anything, then the preview someone is
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
async fn work_lane(ctx: egui::Context, gpu: Gpu, jobs: Inbox<Job>, events: mpsc::Sender<Event>) {
    let mut graphics = Graphics::new(gpu);
    let progress = |progress: Progress| {
        let _ = events.send(Event::Progress(progress));
        ctx.request_repaint();
    };
    while let Some(job) = jobs.next().await {
        let event = match job {
            Job::Shutdown => break,
            Job::Playback {
                video,
                config,
                options,
                feed,
            } => {
                play(&mut graphics, &ctx, &video, &config, &options, &feed).await;
                continue;
            }
            // Loading an image cannot be cancelled.
            Job::Load(path) => Event::Loaded(load_image(path).map_err(Failure::Failed)),
            Job::LoadBytes { name, bytes } => {
                Event::Loaded(load_bytes(name, bytes).await.map_err(Failure::Failed))
            }
            Job::LoadVideo {
                path,
                frame,
                cached,
                cancel,
            } => Event::Loaded(
                load_video(&path, frame, cached, &cancel)
                    .await
                    .map_err(|e| Failure::of(e, &cancel)),
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
                    .guard(export.driver_failed(), async |g| {
                        export.run(g, &path, &cancel, &progress).await
                    })
                    .await
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
    let name = file_name(&path);
    Ok(loaded(path.canonicalize().unwrap_or(path), name, image))
}

/// An image opened for editing, with the thumbnail that shows the original.
fn loaded(path: PathBuf, name: String, image: RgbaImage) -> Loaded {
    // Keep thumbnail processing off the interface's turn, including alpha before resizing.
    let mut opaque = image.clone();
    crtsim_core::config::flatten_alpha(&mut opaque, [0; 3]);
    let thumbnail = image::DynamicImage::ImageRgba8(opaque)
        .thumbnail(2048, 2048)
        .to_rgba8();
    Loaded {
        name,
        path,
        image,
        thumbnail,
        timeline: None,
    }
}

/// A file a browser handed over: a video or animation opens with its frames, anything else
/// as an image.
async fn load_bytes(name: String, bytes: Vec<u8>) -> Result<Loaded> {
    let path = PathBuf::from(&name);
    let contents = crtsim_media::Contents(bytes.into());
    let video = if crtsim_media::MediaKind::of(&path) == crtsim_media::MediaKind::Video {
        Some(crtsim_media::probe_demuxed(path.clone(), contents.clone())?)
    } else if let Some(format) = crtsim_media::detect_bytes(&path, &contents) {
        let cancel = Arc::new(AtomicBool::new(false));
        Some(crtsim_media::probe_bytes(
            path.clone(),
            contents.clone(),
            format,
            &cancel,
        )?)
    } else {
        None
    };
    if let Some(video) = video {
        let frames = video.frames.unwrap_or(1);
        let image = crate::frames::frame(&video, 0).await?;
        return Ok(opened(video, 0, frames, image));
    }
    let image = crtsim_core::input::decode_image(contents.as_ref())?;
    Ok(loaded(path, name, image))
}

async fn load_video(
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
    // A video handed over as bytes is decoded here; one found by its path, by FFmpeg.
    let image = if video.contents.is_some() {
        crate::frames::frame(&video, frame).await?
    } else {
        crtsim_media::preview_frame(&video, frame, cancel)?
    };
    Ok(opened(video, frame, frames, image))
}

/// Frame `frame` of `video`, of `frames`, opened for editing with its timeline.
fn opened(video: Video, frame: u64, frames: u64, image: RgbaImage) -> Loaded {
    let thumbnail = image::DynamicImage::ImageRgba8(image.clone())
        .thumbnail(2048, 2048)
        .to_rgba8();
    Loaded {
        path: video.path.clone(),
        name: file_name(&video.path),
        image,
        thumbnail,
        timeline: Some(Timeline::new(video, frame, frames)),
    }
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

/// A render's progress in the words the status bar shows.
fn worded(progress: RenderProgress) -> Progress {
    let stage = match progress.stage {
        Stage::Preparing => "Preparing image".into(),
        Stage::Simulating { done, of } => format!("Simulation {done}/{of}"),
        Stage::Reading => "Glass, lighting and bloom complete; reading pixels".into(),
        Stage::Done => "Render complete".into(),
    };
    Progress {
        fraction: progress.fraction,
        stage,
    }
}

/// A still from fresh history on this lane's renderer.
async fn still(
    graphics: &mut Graphics,
    input: &RgbaImage,
    c: &Config,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(Progress),
) -> Result<RgbaImage> {
    let renderer = graphics
        .renderer(|| {
            progress(Progress {
                fraction: 0.,
                stage: "Initializing graphics device".into(),
            })
        })
        .await?;
    renderer
        .still(input, c, cancel, |p| progress(worded(p)))
        .await
}

/// A frame for the screen. On the interface's own device it is left there; otherwise it is
/// read back, which is what the interface then has to upload again.
async fn preview(graphics: &mut Graphics, input: &RgbaImage, c: &Config) -> Result<Preview> {
    graphics
        .guard(
            "Graphics device failed while rendering the preview",
            async |g| {
                if g.gpu.render_state().is_none() {
                    return still(g, input, c, None, |_| {}).await.map(Preview::Pixels);
                }
                let renderer = g.renderer(|| {}).await?;
                let mut sequence = Sequence::still();
                renderer
                    .frame(&mut sequence, input, c, None, |_| {})
                    .await?;
                renderer.show(&sequence).map(Preview::Frame)
            },
        )
        .await
}

/// A still at full resolution, saved as a PNG that carries its settings.
async fn export_image(
    graphics: &mut Graphics,
    input: &RgbaImage,
    config: &Config,
    path: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    let image = still(graphics, input, config, Some(cancel), |mut stage| {
        stage.fraction *= 0.9;
        progress(stage);
    })
    .await?;
    ensure!(!cancel.load(Ordering::Relaxed), "Render cancelled");
    progress(Progress {
        fraction: 0.95,
        stage: "Encoding and saving PNG".into(),
    });
    #[cfg(not(target_arch = "wasm32"))]
    return files::save_png(path, image, Some(config));
    // A browser saves by downloading, under the name the path gives.
    #[cfg(target_arch = "wasm32")]
    return crate::web::download(
        &file_name(path),
        &files::png_bytes(image, Some(config))?,
        "image/png",
    );
}

/// An animation of a video handed over as bytes, made here without FFmpeg and saved, which in
/// a browser is a download.
async fn export_animation_here(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &crtsim_media::AnimationOptions,
    path: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    use crtsim_media::page;
    let format = crtsim_media::AnimationFormat::of(path)
        .ok_or_else(|| anyhow::anyhow!("Choose a GIF or WebP filename"))?;
    let renderer = graphics.renderer(|| {}).await?;
    let bytes = page::export_animation(
        video,
        format,
        config,
        options,
        async |span| crate::frames::open(video, span).await,
        async |sequence, frame, config| {
            renderer
                .frame(sequence, frame, config, Some(cancel), |_| {})
                .await?;
            renderer.read(sequence).await
        },
        async |frame, quality| lossy_webp(frame, quality).await,
        cancel,
        progress,
    )
    .await?;
    ensure!(!cancel.load(Ordering::Relaxed), "Render cancelled");
    let mime = match format {
        crtsim_media::AnimationFormat::Gif => "image/gif",
        crtsim_media::AnimationFormat::Webp => "image/webp",
    };
    save(path, &bytes, mime)
}

/// A video handed over as bytes, exported here without FFmpeg and saved, which in a browser
/// is a download. Its frames and sound are encoded by the browser.
async fn export_video_here(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &Options,
    path: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (graphics, video, config, options, path, cancel, progress);
        anyhow::bail!("Videos handed over as bytes are encoded only in a browser")
    }
    #[cfg(target_arch = "wasm32")]
    {
        let container = crtsim_media::Container::of(path)
            .ok_or_else(|| anyhow::anyhow!("Choose an MP4 or WebM filename"))?;
        let renderer = graphics.renderer(|| {}).await?;
        let bytes = crtsim_media::page::export_video(
            video,
            container,
            config,
            options,
            async |span| crate::frames::open(video, span).await,
            async |sequence, frame, config| {
                renderer
                    .frame(sequence, frame, config, Some(cancel), |_| {})
                    .await?;
                renderer.read(sequence).await
            },
            async |settings| crate::web_encode::Encoder::open(settings).await,
            async |audio| crate::web_encode::opus(audio).await,
            cancel,
            progress,
        )
        .await?;
        ensure!(!cancel.load(Ordering::Relaxed), "Render cancelled");
        let mime = match container {
            crtsim_media::Container::Webm => "video/webm",
            _ => "video/mp4",
        };
        save(path, &bytes, mime)
    }
}

/// A frame as a lossy still WebP, which only a browser's own encoder writes here.
async fn lossy_webp(frame: &RgbaImage, quality: u8) -> Result<Vec<u8>> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::lossy_webp(frame, quality).await;
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (frame, quality);
        anyhow::bail!("Lossy WebP is written by FFmpeg on the desktop")
    }
}

/// Saves a finished file: written to `path`, or in a browser downloaded under its name.
fn save(path: &Path, bytes: &[u8], mime: &str) -> Result<()> {
    #[cfg(target_arch = "wasm32")]
    return crate::web::download(&file_name(path), bytes, mime);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = mime;
        let partial = path.with_extension("partial");
        std::fs::write(&partial, bytes)?;
        Ok(std::fs::rename(partial, path)?)
    }
}

/// A whole video rendered and encoded, with its audio and other tracks.
async fn export_video(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &Options,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    if video.contents.is_some() {
        return export_video_here(graphics, video, config, options, path, cancel, progress).await;
    }
    let renderer = graphics.renderer(|| {}).await?;
    crtsim_media::export(video, path, config, options, renderer, cancel, progress)
}

/// Streams rendered frames of `video` into the feed, which the interface plays from. An error
/// is queued behind the frames already there, unless playback has been stopped meanwhile.
async fn play(
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
    let played = graphics
        .guard("Playback graphics driver failed", async |g| {
            let renderer = g.renderer(|| {}).await?;
            // A video handed over as bytes is decoded and paced here, awaiting room in the
            // feed so a browser page keeps drawing; one found by its path, by FFmpeg.
            if video.contents.is_some() {
                use crtsim_media::page;
                return page::playback(
                    video,
                    *start,
                    config,
                    options,
                    renderer,
                    async |span| crate::frames::open(video, span).await,
                    cancel,
                    async |time, source, crt| {
                        let mut item = Ok(PlaybackFrame { time, source, crt });
                        loop {
                            match offer(frames, item, cancel, ctx)? {
                                None => return Ok(()),
                                Some(back) => item = back,
                            }
                            pause().await;
                        }
                    },
                )
                .await;
            }
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
                        match offer(frames, item, cancel, ctx)? {
                            None => return Ok(()),
                            Some(back) => item = back,
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                },
            )
        })
        .await;
    if let Err(error) = played {
        let error = format!("{error:#}");
        // Keep the terminal error behind already buffered frames without blocking shutdown.
        let mut item = Err(error);
        while !cancel.load(Ordering::Relaxed) {
            match frames.try_send(item) {
                Err(mpsc::TrySendError::Full(back)) => {
                    item = back;
                    pause().await;
                }
                _ => break,
            }
        }
    }
    ctx.request_repaint();
}

/// Offers a frame to the feed: `None` once it is taken, or the frame back when the feed is full.
fn offer(
    frames: &mpsc::SyncSender<Result<PlaybackFrame, String>>,
    item: Result<PlaybackFrame, String>,
    cancel: &AtomicBool,
    ctx: &egui::Context,
) -> Result<Option<Result<PlaybackFrame, String>>> {
    ensure!(!cancel.load(Ordering::Relaxed), "Playback cancelled");
    match frames.try_send(item) {
        Ok(()) => {
            ctx.request_repaint();
            Ok(None)
        }
        Err(mpsc::TrySendError::Full(back)) => Ok(Some(back)),
        Err(mpsc::TrySendError::Disconnected(_)) => anyhow::bail!("Playback stopped"),
    }
}

/// A short wait before trying the feed again: a sleep on a thread, a timer in a browser.
async fn pause() {
    #[cfg(target_arch = "wasm32")]
    crate::web::sleep(5).await;
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::sleep(Duration::from_millis(5));
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

    /// Lanes run as tasks on this thread, the way a browser runs them: `drive` polls every
    /// task in turn, as the page's event loop would between frames.
    #[derive(Default)]
    struct Tasks(Rc<std::cell::RefCell<Vec<Task>>>);

    impl Tasks {
        fn runtime(&self) -> Runtime {
            let tasks = self.0.clone();
            Runtime::Tasks(Rc::new(move |task| tasks.borrow_mut().push(task)))
        }

        /// Polls the tasks until `event` has an event to give, or the time is up.
        fn next<T>(&self, events: &mpsc::Receiver<T>) -> T {
            let started = Instant::now();
            let mut cx = std::task::Context::from_waker(Waker::noop());
            loop {
                if let Ok(event) = events.try_recv() {
                    return event;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(120),
                    "worker stalled"
                );
                self.0
                    .borrow_mut()
                    .retain_mut(|task| task.as_mut().poll(&mut cx).is_pending());
            }
        }
    }

    /// Waits for a worker's next event.
    type Next = Box<dyn Fn(&Worker) -> Event>;

    /// The same worker on either runtime, and a way to wait for its next event.
    fn workers() -> Vec<(&'static str, Worker, Next)> {
        let gpu = Gpu::Own(wgpu::Backends::VULKAN);
        let threaded = start(egui::Context::default(), gpu.clone(), Runtime::Threads);
        let tasks = Tasks::default();
        let tasked = start(egui::Context::default(), gpu, tasks.runtime());
        vec![
            (
                "threads",
                threaded,
                Box::new(|worker: &Worker| {
                    worker
                        .events
                        .recv_timeout(Duration::from_secs(120))
                        .expect("worker stalled")
                }),
            ),
            (
                "tasks",
                tasked,
                Box::new(move |worker: &Worker| tasks.next(&worker.events)),
            ),
        ]
    }

    /// Shuts `worker` down, waiting for its lanes where they have threads.
    fn stop(worker: Worker) {
        worker.jobs.shutdown();
        if let Some(threads) = worker.threads {
            threads.join().unwrap();
        }
    }

    /// A LUT thumbnail needs no GPU, so either runtime can be checked end to end anywhere.
    #[test]
    fn a_thumbnail_comes_back_from_either_runtime() {
        for (runtime, worker, next) in workers() {
            worker
                .jobs
                .preview(PreviewJob::Thumbnail {
                    generation: 4,
                    key: ThumbnailKey::Lut(2),
                    input: Arc::new(RgbaImage::from_pixel(8, 8, image::Rgba([90, 60, 30, 255]))),
                    look: Look::Lut(2),
                })
                .unwrap();
            match next(&worker) {
                Event::Thumbnail {
                    generation: 4,
                    key: ThumbnailKey::Lut(2),
                    result: Ok(image),
                } => assert_eq!(image.dimensions(), (8, 8), "{runtime}"),
                _ => panic!("{runtime}: expected the thumbnail"),
            }
            stop(worker);
        }
    }

    /// A browser hands files over as bytes; either runtime decodes them into a source.
    #[test]
    fn an_image_handed_over_as_bytes_opens_on_either_runtime() {
        let png = crate::files::png_bytes(RgbaImage::new(6, 4), None).unwrap();
        for (runtime, worker, next) in workers() {
            worker
                .jobs
                .send(Job::LoadBytes {
                    name: "picked.png".into(),
                    bytes: png.clone(),
                })
                .unwrap();
            match next(&worker) {
                Event::Loaded(Ok(loaded)) => {
                    assert_eq!(loaded.name, "picked.png", "{runtime}");
                    assert_eq!(loaded.image.dimensions(), (6, 4), "{runtime}");
                }
                _ => panic!("{runtime}: expected the image"),
            }
            worker
                .jobs
                .send(Job::LoadBytes {
                    name: "broken.png".into(),
                    bytes: b"not a picture".to_vec(),
                })
                .unwrap();
            assert!(
                matches!(next(&worker), Event::Loaded(Err(Failure::Failed(_)))),
                "{runtime}: a broken file is refused"
            );
            stop(worker);
        }
    }

    /// An animated GIF of `frames` 8x6 frames of 100 ms, frame `i` gray level `i * 40`.
    fn animated_gif(frames: u8) -> Vec<u8> {
        use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba};
        let mut bytes = vec![];
        let mut encoder = GifEncoder::new(&mut bytes);
        for i in 0..frames {
            let image = RgbaImage::from_pixel(8, 6, Rgba([i * 40, i * 40, i * 40, 255]));
            let delay = Delay::from_numer_denom_ms(100, 1);
            encoder
                .encode_frame(Frame::from_parts(image, 0, 0, delay))
                .unwrap();
        }
        drop(encoder);
        bytes
    }

    #[test]
    fn an_animation_handed_over_as_bytes_opens_with_its_frames_on_either_runtime() {
        for (runtime, worker, next) in workers() {
            let job = Job::LoadBytes {
                name: "clip.gif".into(),
                bytes: animated_gif(4),
            };
            worker.jobs.send(job).unwrap();
            let Event::Loaded(Ok(loaded)) = next(&worker) else {
                panic!("{runtime}: expected the animation");
            };
            let timeline = loaded.timeline.expect("an animation has a timeline");
            assert_eq!((timeline.frames, loaded.image.dimensions()), (4, (8, 6)));
            // Another frame of the one already open, decoded here from its bytes.
            worker
                .jobs
                .send(Job::LoadVideo {
                    path: "clip.gif".into(),
                    frame: 2,
                    cached: Some((timeline.video.clone(), timeline.frames)),
                    cancel: Arc::new(AtomicBool::new(false)),
                })
                .unwrap();
            let Event::Loaded(Ok(loaded)) = next(&worker) else {
                panic!("{runtime}: expected frame 3");
            };
            assert_eq!(loaded.image.get_pixel(0, 0)[0], 80, "{runtime}");
            stop(worker);
        }
    }

    /// Animations of a video handed over as bytes are written here, as a browser writes them.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn an_animation_handed_over_as_bytes_exports_without_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let (_, worker, next) = workers().remove(1);
        worker
            .jobs
            .send(Job::LoadBytes {
                name: "clip.gif".into(),
                bytes: animated_gif(3),
            })
            .unwrap();
        let Event::Loaded(Ok(loaded)) = next(&worker) else {
            panic!("expected the animation");
        };
        let video = loaded.timeline.unwrap().video;
        for (name, lossless) in [("crt.gif", false), ("crt.webp", true)] {
            let options = crtsim_media::AnimationOptions {
                max_side: 96,
                fps: 10,
                lossless,
                ..Default::default()
            };
            let path = dir.path().join(name);
            worker
                .jobs
                .send(Job::Export {
                    export: Export::Animation {
                        video: video.clone(),
                        options,
                        config: Config::default(),
                    },
                    path: path.clone(),
                    cancel: Arc::new(AtomicBool::new(false)),
                })
                .unwrap();
            loop {
                match next(&worker) {
                    Event::Progress(_) => continue,
                    Event::Exported(result) => {
                        assert!(result.is_ok(), "{name}: {:?}", result.err());
                        break;
                    }
                    _ => panic!("{name}: unexpected event"),
                }
            }
            let bytes = std::fs::read(&path).unwrap();
            let size = image::load_from_memory(&bytes)
                .unwrap()
                .to_rgba8()
                .dimensions();
            assert_eq!(size.0.max(size.1), 96, "{name}");
        }
        stop(worker);
    }

    /// The point of the second lane: a preview finishes while an export is still running,
    /// where it used to wait in the queue behind all of it. On threads the lanes run at once;
    /// as tasks on one thread they take turns between the export's batches.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn a_preview_finishes_while_an_export_is_still_running() {
        for (runtime, worker, next) in workers() {
            let dir = tempfile::tempdir().unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            let input = Arc::new(crtsim_core::config::test_card());
            // Long enough to still be running when the preview is done: the maximum warm-up,
            // at 4K.
            worker
                .jobs
                .send(Job::Export {
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
            worker
                .jobs
                .preview(PreviewJob::Preview {
                    revision: 7,
                    input,
                    config: Box::new(Config {
                        output: "320x180".into(),
                        warmup: 0,
                        ..Config::default()
                    }),
                })
                .unwrap();
            loop {
                match next(&worker) {
                    Event::Progress(_) => continue,
                    Event::Preview { revision, result } => {
                        assert_eq!(revision, 7, "{runtime}");
                        assert!(result.is_ok(), "{runtime}: preview failed");
                        break;
                    }
                    Event::Exported(_) => panic!("{runtime}: the export finished first"),
                    _ => panic!("{runtime}: unexpected event"),
                }
            }
            cancel.store(true, Ordering::Relaxed);
            loop {
                if let Event::Exported(result) = next(&worker) {
                    assert!(
                        matches!(result, Err(Failure::Cancelled)),
                        "{runtime}: export should have been cancelled"
                    );
                    break;
                }
            }
            stop(worker);
        }
    }
}
