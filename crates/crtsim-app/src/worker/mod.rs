mod export;
mod lanes;

pub use export::Export;
use lanes::Inbox;
pub use lanes::{Jobs, PreviewLane, Runtime, Stopped, WorkLane};

use crate::{file_name, preview::Kind, timeline::Timeline};
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
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    task::Poll,
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
        kind: Kind,
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

/// How the app reads and writes colours: what it does with the colour profile a picture
/// carries, and the space its PNG stills are written in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Colors {
    pub profiles: crtsim_core::input::Profiles,
    pub space: crtsim_core::color::Space,
}

/// Work for the other thread: loading, exports and playback, one at a time.
#[expect(
    clippy::large_enum_variant,
    reason = "a job is sent once for something the user asked for, so its size costs nothing"
)]
pub enum Job {
    /// An image, its colour profile used as asked.
    Load(PathBuf, crtsim_core::input::Profiles),
    /// An image handed over by name and contents, as a browser gives a file picked or dropped.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop opens files by path")
    )]
    LoadBytes {
        name: String,
        bytes: Vec<u8>,
        profiles: crtsim_core::input::Profiles,
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
    /// Rendered and saved to `path`, which is only replaced once the whole file is ready. A
    /// still is written in `colors`' space, and a batch's images read as they ask.
    Export {
        export: Export,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
        colors: Colors,
    },
    Playback {
        video: Video,
        options: Options,
        feed: Feed,
    },
    Shutdown,
}

impl Job {
    /// The flag that stops this job, if it can be stopped.
    pub fn cancel(&self) -> Option<&Arc<AtomicBool>> {
        match self {
            Self::LoadVideo { cancel, .. }
            | Self::ImportPreset { cancel, .. }
            | Self::Export { cancel, .. } => Some(cancel),
            Self::Playback { feed, .. } => Some(&feed.cancel),
            Self::Load(..) | Self::LoadBytes { .. } | Self::Shutdown => None,
        }
    }
}

/// Which gallery entry a thumbnail belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThumbnailKey {
    /// A preset by name, included or personal. Personal presets saved before included names
    /// were refused can share one with an included preset, and each keeps its own picture.
    Preset {
        name: String,
        personal: bool,
    },
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
    /// Where in the video the frame is, in seconds.
    pub time: f64,
    /// How long the playing has lasted when the frame shows, which a loop takes on past the
    /// video's end where `time` starts again.
    pub elapsed: f64,
    pub source: RgbaImage,
    pub crt: RgbaImage,
}
/// The worker's end of a playback: where in the video to start, the settings to render with,
/// where to send the frames it renders, and the flag that says to stop.
pub struct Feed {
    pub start: f64,
    pub settings: PlayingSettings,
    /// Whether to play again from the start when the video ends, which the interface sets as
    /// the video plays.
    pub looping: Arc<AtomicBool>,
    pub frames: mpsc::SyncSender<Result<PlaybackFrame, String>>,
    pub cancel: Arc<AtomicBool>,
}

/// The settings a video playing renders with, shared between the interface, which replaces
/// them as they are edited, and the worker, which takes them as it starts each frame.
#[derive(Clone)]
pub struct PlayingSettings(Arc<Mutex<Arc<Config>>>);

impl PlayingSettings {
    pub fn new(config: Config) -> Self {
        Self(Arc::new(Mutex::new(Arc::new(config))))
    }

    /// The settings the next frame starts with.
    pub fn get(&self) -> Arc<Config> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set(&self, config: Config) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(config);
    }
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
    /// What became of an image's colour profile, when it carries one other than sRGB.
    pub note: Option<String>,
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
    let (jobs, work, previews) = Jobs::open();
    let (events, receive) = mpsc::channel();
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
                pollster::block_on(work_lane(ctx, gpu, work, events));
                let _ = previews.join();
            }))
        }
        Runtime::Tasks(spawn) => {
            spawn(Box::pin(work_lane(ctx, gpu, work, events)));
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
        jobs,
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
    // The CRT on screen and the source it shows, which interactive previews of that source run
    // on. A settled preview, or another source, starts it afresh.
    let mut editing: Option<(Arc<RgbaImage>, Sequence)> = None;
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
                kind,
                input,
                config,
            } => {
                let started = Instant::now();
                let runs_on = kind == Kind::Interactive
                    && editing
                        .as_ref()
                        .is_some_and(|(shown, _)| Arc::ptr_eq(shown, &input));
                if !runs_on {
                    editing = Some((input.clone(), Sequence::editing()));
                }
                let (_, sequence) = editing.as_mut().expect("made above");
                let result = preview(&mut graphics, sequence, &input, &config)
                    .await
                    .map(|image| Previewed {
                        image,
                        seconds: started.elapsed().as_secs_f32(),
                    });
                // A failed render may have lost its renderer, which the sequence belongs to.
                if result.is_err() {
                    editing = None;
                }
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
                options,
                feed,
            } => {
                play(&mut graphics, &ctx, &video, &options, &feed).await;
                continue;
            }
            // Loading an image cannot be cancelled.
            Job::Load(path, profiles) => {
                Event::Loaded(load_image(path, profiles).map_err(Failure::Failed))
            }
            Job::LoadBytes {
                name,
                bytes,
                profiles,
            } => Event::Loaded(
                load_bytes(name, bytes, profiles)
                    .await
                    .map_err(Failure::Failed),
            ),
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
                colors,
            } => Event::Exported(
                graphics
                    .guard(export.driver_failed(), async |g| {
                        export.run(g, &path, &cancel, &progress, &colors).await
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

fn load_image(path: PathBuf, profiles: crtsim_core::input::Profiles) -> Result<Loaded> {
    let decoded = crtsim_core::input::read(&path, profiles)?;
    let name = file_name(&path);
    Ok(loaded(
        path.canonicalize().unwrap_or(path),
        name,
        decoded.image,
        decoded.note,
    ))
}

/// An image opened for editing, with the thumbnail that shows the original and what became of
/// its colour profile.
fn loaded(path: PathBuf, name: String, image: RgbaImage, note: Option<String>) -> Loaded {
    Loaded {
        name,
        path,
        thumbnail: original(&image),
        image,
        timeline: None,
        note,
    }
}

/// `image` made opaque and at most 2048 pixels on a side, for showing the original. It is made
/// here to keep it off the interface's turn.
fn original(image: &RgbaImage) -> RgbaImage {
    let mut opaque = image.clone();
    crtsim_core::config::flatten_alpha(&mut opaque, [0; 3]);
    image::DynamicImage::ImageRgba8(opaque)
        .thumbnail(2048, 2048)
        .to_rgba8()
}

/// A file a browser handed over: a video or animation opens with its frames, anything else
/// as an image.
async fn load_bytes(
    name: String,
    bytes: Vec<u8>,
    profiles: crtsim_core::input::Profiles,
) -> Result<Loaded> {
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
        let cancel = Arc::new(AtomicBool::new(false));
        let image = crate::frames::frame(&video, 0, &cancel).await?;
        return Ok(opened(video, 0, frames, image));
    }
    let decoded = crtsim_core::input::read_bytes(contents.as_ref(), profiles)?;
    Ok(loaded(path, name, decoded.image, decoded.note))
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
    let image = crate::frames::frame(&video, frame, cancel).await?;
    Ok(opened(video, frame, frames, image))
}

/// Frame `frame` of `video`, of `frames`, opened for editing with its timeline.
fn opened(video: Video, frame: u64, frames: u64, image: RgbaImage) -> Loaded {
    Loaded {
        path: video.path.clone(),
        name: file_name(&video.path),
        thumbnail: original(&image),
        image,
        timeline: Some(Timeline::new(video, frame, frames)),
        note: None,
    }
}

fn import_preset(
    path: PathBuf,
    input: (u32, u32),
    cancel: &Arc<AtomicBool>,
) -> Result<ImportedPreset> {
    let file = crate::incoming::File::Path(path.clone());
    let (config, options) = crate::incoming::imported(&file, input, cancel)?;
    Ok(ImportedPreset {
        path,
        config,
        options,
    })
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

/// `sequence`'s next frame, for the screen. On the interface's own device it is left there;
/// otherwise it is read back, which is what the interface then has to upload again.
async fn preview(
    graphics: &mut Graphics,
    sequence: &mut Sequence,
    input: &RgbaImage,
    c: &Config,
) -> Result<Preview> {
    graphics
        .guard(
            "Graphics device failed while rendering the preview",
            async |g| {
                let shared = g.gpu.render_state().is_some();
                let renderer = g.renderer(|| {}).await?;
                renderer.frame(sequence, input, c, None, |_| {}).await?;
                if shared {
                    renderer.show(sequence).map(Preview::Frame)
                } else {
                    renderer.read(sequence).await.map(Preview::Pixels)
                }
            },
        )
        .await
}

/// Streams rendered frames of `video` into the feed, which the interface plays from. An error
/// is queued behind the frames already there, unless playback has been stopped meanwhile.
async fn play(
    graphics: &mut Graphics,
    ctx: &egui::Context,
    video: &Video,
    options: &Options,
    feed: &Feed,
) {
    let Feed {
        start,
        settings,
        looping,
        frames,
        cancel,
    } = feed;
    let config = || settings.get();
    let played = graphics
        .guard("Playback graphics driver failed", async |g| {
            let renderer = g.renderer(|| {}).await?;
            // Paced here, awaiting room in the feed so a browser page keeps drawing.
            crtsim_media::jobs::playback(
                video,
                *start,
                config,
                options,
                || looping.load(Ordering::Relaxed),
                async |span| crate::frames::open(video, span, cancel).await,
                async |sequence, frame, config| {
                    crtsim_media::jobs::render(renderer, sequence, frame, config, cancel).await
                },
                cancel,
                async |time, elapsed, source, crt| {
                    let mut item = Ok(PlaybackFrame {
                        time,
                        elapsed,
                        source,
                        crt,
                    });
                    loop {
                        match offer(frames, item, cancel, ctx)? {
                            None => return Ok(()),
                            Some(back) => item = back,
                        }
                        pause().await;
                    }
                },
            )
            .await
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
    std::thread::sleep(std::time::Duration::from_millis(5));
}

#[cfg(test)]
mod tests {
    use super::{export::export_animation_here, lanes::Task, *};
    use std::{rc::Rc, task::Waker};

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
                kind: Kind::Settled,
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
                    started.elapsed() < std::time::Duration::from_secs(120),
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
                        .recv_timeout(std::time::Duration::from_secs(120))
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
        let png = crate::files::png_bytes(RgbaImage::new(6, 4), None, &Default::default()).unwrap();
        for (runtime, worker, next) in workers() {
            worker
                .jobs
                .send(Job::LoadBytes {
                    name: "picked.png".into(),
                    bytes: png.clone(),
                    profiles: Default::default(),
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
                    profiles: Default::default(),
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
                profiles: Default::default(),
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

    /// The video test card has no file to read and needs no FFmpeg: its frames are drawn.
    #[test]
    fn the_video_test_card_opens_at_any_frame_on_either_runtime() {
        for (runtime, worker, next) in workers() {
            let video = crtsim_media::test_clip();
            for frame in [0, 450] {
                worker
                    .jobs
                    .send(Job::LoadVideo {
                        path: video.path.clone(),
                        frame,
                        cached: Some((video.clone(), 600)),
                        cancel: Arc::new(AtomicBool::new(false)),
                    })
                    .unwrap();
                let Event::Loaded(Ok(loaded)) = next(&worker) else {
                    panic!("{runtime}: expected frame {frame}");
                };
                assert_eq!(loaded.name, "Video test card", "{runtime}");
                assert_eq!(loaded.image, crtsim_core::test_clip::frame(frame));
                let timeline = loaded.timeline.expect("the clip has a timeline");
                assert!(timeline.is_video_test_card());
                assert_eq!((timeline.shown, timeline.frames), (frame, 600));
            }
            stop(worker);
        }
    }

    /// Exports of the video test card: made here as a browser makes them, and on the desktop
    /// by FFmpeg, as any animation's are.
    #[test]
    #[ignore = "requires a Vulkan adapter and FFmpeg"]
    fn the_video_test_card_exports_as_a_browser_and_the_desktop_export_it() {
        let dir = tempfile::tempdir().unwrap();
        let video = crtsim_media::test_clip();
        let options = crtsim_media::AnimationOptions {
            max_side: 128,
            fps: 20,
            start: 2.,
            max_seconds: Some(1.),
            lossless: true,
            ..Default::default()
        };
        let small = Config {
            output: "160x120".into(),
            ..Config::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        // As a browser does: one frame at a time, the file made in memory.
        let mut graphics = Graphics::new(Gpu::Own(wgpu::Backends::VULKAN));
        let config = Config::default();
        for name in ["here.gif", "here.webp"] {
            let path = dir.path().join(name);
            let export = export_animation_here(
                &mut graphics,
                &video,
                &config,
                &options,
                &path,
                &cancel,
                &|_| {},
            );
            pollster::block_on(export).unwrap();
        }
        // As the desktop does, through the worker.
        let (_, worker, next) = workers().remove(0);
        let exports = [
            (
                "desktop.gif",
                Export::Animation {
                    video: video.clone(),
                    options: options.clone(),
                    config: Config::default(),
                },
            ),
            (
                "desktop.mp4",
                Export::Video {
                    video: video.clone(),
                    options: Options::default(),
                    config: small,
                },
            ),
        ];
        for (name, export) in exports {
            worker
                .jobs
                .send(Job::Export {
                    export,
                    path: dir.path().join(name),
                    cancel: cancel.clone(),
                    colors: Default::default(),
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
        }
        stop(worker);
        for (name, frames) in [
            // A second at 20 per second.
            ("here.gif", 20),
            ("here.webp", 20),
            ("desktop.gif", 20),
            // The whole clip, at its own rate.
            ("desktop.mp4", 600),
        ] {
            let written = crtsim_media::probe(&dir.path().join(name), &cancel).unwrap();
            let count = crtsim_media::frame_count(&written, &cancel).unwrap();
            assert_eq!(count, frames, "{name}");
        }
    }

    /// An animation handed over as bytes, as a browser hands it over, exports on the desktop as
    /// any other: as animations and as a video, all encoded by FFmpeg.
    #[test]
    #[ignore = "requires a Vulkan adapter and FFmpeg"]
    fn an_animation_handed_over_as_bytes_exports_as_animations_and_as_a_video() {
        let dir = tempfile::tempdir().unwrap();
        let (_, worker, next) = workers().remove(1);
        worker
            .jobs
            .send(Job::LoadBytes {
                name: "clip.gif".into(),
                bytes: animated_gif(3),
                profiles: Default::default(),
            })
            .unwrap();
        let Event::Loaded(Ok(loaded)) = next(&worker) else {
            panic!("expected the animation");
        };
        let video = loaded.timeline.unwrap().video;
        let animation = |lossless| Export::Animation {
            video: video.clone(),
            options: crtsim_media::AnimationOptions {
                max_side: 96,
                fps: 10,
                lossless,
                ..Default::default()
            },
            config: Config::default(),
        };
        let exports = [
            ("crt.gif", animation(false)),
            ("crt.webp", animation(true)),
            (
                "crt.mp4",
                Export::Video {
                    video: video.clone(),
                    options: Options::default(),
                    config: Config {
                        output: "640x480".into(),
                        ..Config::default()
                    },
                },
            ),
        ];
        for (name, export) in exports {
            let path = dir.path().join(name);
            worker
                .jobs
                .send(Job::Export {
                    export,
                    path: path.clone(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    colors: Default::default(),
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
            let size = if name == "crt.mp4" {
                let cancel = Arc::new(AtomicBool::new(false));
                crtsim_media::probe(&path, &cancel).unwrap().size
            } else {
                let bytes = std::fs::read(&path).unwrap();
                image::load_from_memory(&bytes)
                    .unwrap()
                    .to_rgba8()
                    .dimensions()
            };
            let longest = size.0.max(size.1);
            assert_eq!(longest, if name == "crt.mp4" { 640 } else { 96 }, "{name}");
        }
        stop(worker);
    }

    /// Interactive previews run the CRT on screen on, until another source or output size; a
    /// settled preview is the still an export of the same settings renders.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn interactive_previews_run_on_and_a_settled_one_is_a_still() {
        let renderer = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
        let input = Arc::new(crtsim_core::config::test_card());
        let config = Config {
            output: "320x240".into(),
            ..Config::default()
        };
        let grey = Config {
            chroma: 0.,
            ..config.clone()
        };
        let wide = Config {
            output: "400x240".into(),
            ..grey.clone()
        };
        let still =
            |c: &Config| pollster::block_on(renderer.still(&input, c, None, |_| {})).unwrap();
        for (runtime, worker, next) in workers() {
            let preview_of = |input: &Arc<RgbaImage>, revision, kind, config: &Config| {
                worker
                    .jobs
                    .preview(PreviewJob::Preview {
                        revision,
                        kind,
                        input: input.clone(),
                        config: Box::new(config.clone()),
                    })
                    .unwrap();
                match next(&worker) {
                    Event::Preview {
                        revision: back,
                        result:
                            Ok(Previewed {
                                image: Preview::Pixels(image),
                                ..
                            }),
                    } if back == revision => image,
                    _ => panic!("{runtime}: expected preview {revision}"),
                }
            };
            let preview =
                |revision, kind, config: &Config| preview_of(&input, revision, kind, config);
            assert_eq!(
                preview(1, Kind::Interactive, &config),
                still(&config),
                "{runtime}: the first is a still"
            );
            assert_ne!(
                preview(2, Kind::Interactive, &grey),
                still(&grey),
                "{runtime}: an edit runs on over the glow before it"
            );
            let reopened = Arc::new((*input).clone());
            assert_eq!(
                preview_of(&reopened, 3, Kind::Interactive, &grey),
                still(&grey),
                "{runtime}: another source starts afresh, without the glow of the last"
            );
            assert_eq!(
                preview(3, Kind::Interactive, &wide).dimensions(),
                (400, 240),
                "{runtime}: a new size starts again"
            );
            assert_eq!(
                preview(4, Kind::Settled, &wide),
                still(&wide),
                "{runtime}: settled"
            );
            stop(worker);
        }
    }

    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn a_contact_sheet_is_saved_with_a_tile_for_each_look() {
        for (runtime, worker, next) in workers() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("sheet.png");
            let looks = ["General image", "Clean RGB", "NTSC 240p"]
                .map(|name| crtsim_core::sheet::Look {
                    name: name.into(),
                    config: crtsim_core::presets::named(name).unwrap().config,
                })
                .into();
            worker
                .jobs
                .send(Job::Export {
                    export: Export::Sheet {
                        input: Arc::new(crtsim_core::config::test_card()),
                        looks,
                        tile: (160, 120),
                        columns: 2,
                    },
                    path: path.clone(),
                    cancel: Arc::new(AtomicBool::new(false)),
                    colors: Default::default(),
                })
                .unwrap();
            loop {
                if let Event::Exported(result) = next(&worker) {
                    assert!(result.is_ok(), "{runtime}: the sheet failed");
                    break;
                }
            }
            // Two columns of 160 and two rows of 120 with their labels, between 4-pixel gaps.
            let sheet = image::open(&path).unwrap();
            assert_eq!((sheet.width(), sheet.height()), (2 * 164 + 4, 2 * 136 + 4));
            stop(worker);
        }
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
                    colors: Default::default(),
                })
                .unwrap();
            worker
                .jobs
                .preview(PreviewJob::Preview {
                    revision: 7,
                    kind: Kind::Settled,
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
