use crate::files;
use crtsim_core::{config::Config, RenderProgress, Renderer};
use eframe::egui;
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
    time::Instant,
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
    fn renderer(&self) -> anyhow::Result<Renderer> {
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

/// Where the interface sends work. Previews have a thread of their own, so they keep coming
/// while an export holds the other one for minutes: a preview queued behind an export used to
/// wait for all of it, and settings edited meanwhile could not be seen until it finished.
#[derive(Clone)]
pub struct Jobs {
    work: mpsc::Sender<Job>,
    preview: mpsc::Sender<Job>,
}
/// A job could not be sent because its worker has stopped.
#[derive(Debug)]
pub struct Stopped;
impl Jobs {
    pub fn send(&self, job: Job) -> Result<(), Stopped> {
        match job {
            Job::Preview { .. } | Job::Thumbnail { .. } => self.preview.send(job),
            Job::Shutdown => {
                let _ = self.preview.send(Job::Shutdown);
                self.work.send(job)
            }
            job => self.work.send(job),
        }
        .map_err(|_| Stopped)
    }
    /// Both threads' jobs on one channel, for a test to inspect what the interface sends.
    #[cfg(test)]
    pub fn capture(send: mpsc::Sender<Job>) -> Self {
        Self {
            work: send.clone(),
            preview: send,
        }
    }
}

pub enum Job {
    Playback {
        video: crtsim_media::Video,
        start: f64,
        config: Config,
        options: crtsim_media::Options,
        cancel: Arc<AtomicBool>,
        frames: mpsc::SyncSender<Result<PlaybackFrame, String>>,
    },
    Batch {
        source: PathBuf,
        config: Config,
        options: crtsim_media::Options,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    Shutdown,
    Load(PathBuf),
    ImportPreset {
        path: PathBuf,
        input: (u32, u32),
        cancel: Arc<AtomicBool>,
    },
    LoadVideo {
        path: PathBuf,
        frame: u64,
        cached: Option<(crtsim_media::Video, u64)>,
        cancel: Arc<AtomicBool>,
    },
    ExportVideo {
        video: crtsim_media::Video,
        options: crtsim_media::Options,
        config: Config,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
    Preview {
        revision: u64,
        input: Arc<RgbaImage>,
        config: Config,
    },
    /// A small picture for a gallery entry. Always waits for a pending preview, and is dropped
    /// unrendered once a thumbnail of a newer source is queued behind it.
    Thumbnail {
        generation: u64,
        key: ThumbnailKey,
        input: Arc<RgbaImage>,
        look: Look,
    },
    Export {
        input: Arc<RgbaImage>,
        config: Config,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
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

pub enum Event {
    Progress {
        progress: RenderProgress,
    },
    Loaded(Result<(PathBuf, RgbaImage, RgbaImage), String>),
    VideoLoaded(Result<(crtsim_media::Video, RgbaImage, RgbaImage, u64, u64), String>),
    PresetImported(Result<(PathBuf, Config, Option<crtsim_media::Options>), String>),
    Preview {
        revision: u64,
        result: Result<(Preview, f32), String>,
    },
    Exported(Result<PathBuf, String>),
    Thumbnail {
        generation: u64,
        key: ThumbnailKey,
        result: Result<RgbaImage, String>,
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
    jobs: mpsc::Receiver<Job>,
    events: mpsc::Sender<Event>,
) {
    let mut renderer: Option<Renderer> = None;
    let mut backlog = std::collections::VecDeque::new();
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
            return;
        };
        let event = match job {
            Job::Preview {
                revision,
                input,
                config,
            } => {
                let started = Instant::now();
                let result = preview(&mut renderer, &gpu, &input, &config)
                    .map(|p| (p, started.elapsed().as_secs_f32()));
                Event::Preview { revision, result }
            }
            Job::Thumbnail {
                generation,
                key,
                input,
                look,
            } => {
                let result = match look {
                    Look::Crt(config) => render(&mut renderer, &gpu, &input, &config, None, |_| {}),
                    Look::Lut(index) => crtsim_core::nes_luts::load(index)
                        .map(|lut| {
                            let mut image = (*input).clone();
                            lut.apply(&mut image);
                            image
                        })
                        .map_err(|e| format!("{e:#}")),
                };
                Event::Thumbnail {
                    generation,
                    key,
                    result,
                }
            }
            _ => return,
        };
        if events.send(event).is_err() {
            return;
        }
        ctx.request_repaint();
    }
}

/// The preview thread's next job: shutdown first, then the preview someone is waiting to see,
/// then thumbnails in the order asked for, skipping any whose source has since been replaced.
fn next_job(backlog: &mut std::collections::VecDeque<Job>) -> Option<Job> {
    if backlog.iter().any(|job| matches!(job, Job::Shutdown)) {
        return None;
    }
    let newest = backlog
        .iter()
        .filter_map(|job| match job {
            Job::Thumbnail { generation, .. } => Some(*generation),
            _ => None,
        })
        .max();
    backlog.retain(
        |job| !matches!(job, Job::Thumbnail { generation, .. } if Some(*generation) < newest),
    );
    match backlog
        .iter()
        .position(|job| matches!(job, Job::Preview { .. }))
    {
        Some(index) => backlog.remove(index),
        None => backlog.pop_front(),
    }
}

/// Everything but previews: loading, exports, batches and playback, one at a time.
fn work(ctx: egui::Context, gpu: Gpu, jobs: mpsc::Receiver<Job>, events: mpsc::Sender<Event>) {
    let mut renderer: Option<Renderer> = None;
    while let Ok(job) = jobs.recv() {
        let event = match job {
            Job::Shutdown => break,
            Job::Playback {
                video,
                start,
                config,
                options,
                cancel,
                frames,
            } => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> anyhow::Result<()> {
                        if renderer.is_none() {
                            renderer = Some(gpu.renderer()?);
                        }
                        crtsim_media::playback(
                            &video,
                            start,
                            &config,
                            &options,
                            renderer.as_ref().unwrap(),
                            &cancel,
                            |time, source, crt| {
                                let mut item = Ok(PlaybackFrame { time, source, crt });
                                loop {
                                    anyhow::ensure!(
                                        !cancel.load(std::sync::atomic::Ordering::Relaxed),
                                        "Playback cancelled"
                                    );
                                    match frames.try_send(item) {
                                        Ok(()) => {
                                            ctx.request_repaint();
                                            return Ok(());
                                        }
                                        Err(mpsc::TrySendError::Full(back)) => {
                                            item = back;
                                            std::thread::sleep(std::time::Duration::from_millis(5));
                                        }
                                        Err(mpsc::TrySendError::Disconnected(_)) => {
                                            anyhow::bail!("Playback stopped")
                                        }
                                    }
                                }
                            },
                        )
                    },
                ));
                let error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(format!("{e:#}")),
                    Err(_) => {
                        renderer = None;
                        Some("Playback graphics driver failed".into())
                    }
                };
                if let Some(error) = error {
                    // Keep the terminal error behind already buffered frames without blocking shutdown.
                    while !cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        match frames.try_send(Err(error.clone())) {
                            Err(mpsc::TrySendError::Full(_)) => {
                                std::thread::sleep(std::time::Duration::from_millis(5))
                            }
                            _ => break,
                        }
                    }
                }
                ctx.request_repaint();
                continue;
            }
            Job::Batch {
                source,
                config,
                options,
                path,
                cancel,
            } => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> Result<PathBuf, String> {
                        if crate::workflow::is_video(&source) {
                            let video = crtsim_media::probe(&source, &cancel)
                                .map_err(|e| format!("{e:#}"))?;
                            if renderer.is_none() {
                                renderer = Some(gpu.renderer().map_err(|e| format!("{e:#}"))?);
                            }
                            crtsim_media::export(
                                &video,
                                &path,
                                &config,
                                &options,
                                renderer.as_ref().unwrap(),
                                &cancel,
                                |p| {
                                    let _ = events.send(Event::Progress {
                                        progress: RenderProgress {
                                            fraction: p.fraction,
                                            stage: p.stage,
                                        },
                                    });
                                    ctx.request_repaint();
                                },
                            )
                            .map_err(|e| format!("{e:#}"))?;
                        } else {
                            let input = files::load_image(&source).map_err(|e| format!("{e:#}"))?;
                            let im =
                                render(&mut renderer, &gpu, &input, &config, Some(&cancel), |p| {
                                    let _ = events.send(Event::Progress { progress: p });
                                    ctx.request_repaint();
                                })?;
                            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                                return Err("Export cancelled".into());
                            }
                            files::save_png(&path, im, Some(&config))
                                .map_err(|e| format!("{e:#}"))?;
                        }
                        Ok(path)
                    },
                ));
                Event::Exported(match result {
                    Ok(result) => result,
                    Err(_) => {
                        renderer = None;
                        Err("Batch graphics driver failed. Try a smaller resolution.".into())
                    }
                })
            }
            Job::ImportPreset {
                path,
                input,
                cancel,
            } => Event::PresetImported(
                (|| -> anyhow::Result<_> {
                    if path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
                    {
                        let config = files::load_preset_from_image(&path, input)?;
                        Ok((path, config, None))
                    } else {
                        let preset = crtsim_media::import_preset(&path, input, &cancel)?;
                        Ok((path, preset.config, Some(preset.video_options)))
                    }
                })()
                .map_err(|e| format!("{e:#}")),
            ),
            Job::LoadVideo {
                path,
                frame,
                cached,
                cancel,
            } => Event::VideoLoaded(
                (|| -> anyhow::Result<_> {
                    let (video, count) = match cached {
                        Some(cached) => cached,
                        None => {
                            let video = crtsim_media::probe(&path, &cancel)?;
                            let count = crtsim_media::frame_count(&video, &cancel)?;
                            (video, count)
                        }
                    };
                    anyhow::ensure!(frame < count, "Frame is outside the video");
                    let image = crtsim_media::preview_frame(&video, frame, &cancel)?;
                    let thumb = image::DynamicImage::ImageRgba8(image.clone())
                        .thumbnail(2048, 2048)
                        .to_rgba8();
                    Ok((video, image, thumb, frame, count))
                })()
                .map_err(|e| format!("{e:#}")),
            ),
            Job::ExportVideo {
                video,
                options,
                config,
                path,
                cancel,
            } => {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                    || -> anyhow::Result<_> {
                        if renderer.is_none() {
                            renderer = Some(gpu.renderer()?);
                        }
                        crtsim_media::export(
                            &video,
                            &path,
                            &config,
                            &options,
                            renderer.as_ref().unwrap(),
                            &cancel,
                            |p| {
                                let _ = events.send(Event::Progress {
                                    progress: RenderProgress {
                                        fraction: p.fraction,
                                        stage: p.stage,
                                    },
                                });
                                ctx.request_repaint();
                            },
                        )?;
                        Ok(path)
                    },
                ));
                Event::Exported(match result {
                    Ok(result) => result.map_err(|e| format!("{e:#}")),
                    Err(_) => {
                        renderer = None;
                        Err("Video graphics driver failed. Try a smaller resolution.".into())
                    }
                })
            }
            Job::Load(path) => Event::Loaded(
                files::load_image(&path)
                    .map(|im| {
                        // Keep thumbnail processing off the UI thread, including alpha before resizing.
                        let mut opaque = im.clone();
                        for p in opaque.pixels_mut() {
                            for i in 0..3 {
                                p[i] = ((u16::from(p[i]) * u16::from(p[3]) + 127) / 255) as u8;
                            }
                            p[3] = 255;
                        }
                        let thumb = image::DynamicImage::ImageRgba8(opaque)
                            .thumbnail(2048, 2048)
                            .to_rgba8();
                        (path, im, thumb)
                    })
                    .map_err(|e| format!("{e:#}")),
            ),
            Job::Preview { .. } | Job::Thumbnail { .. } => {
                unreachable!("previews and thumbnails are routed to their own thread")
            }
            Job::Export {
                input,
                config,
                path,
                cancel,
            } => Event::Exported(
                render(
                    &mut renderer,
                    &gpu,
                    &input,
                    &config,
                    Some(&cancel),
                    |mut progress| {
                        progress.fraction *= 0.9;
                        let _ = events.send(Event::Progress { progress });
                        ctx.request_repaint();
                    },
                )
                .and_then(|im| {
                    if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                        return Err("Render cancelled".into());
                    }
                    let _ = events.send(Event::Progress {
                        progress: RenderProgress {
                            fraction: 0.95,
                            stage: "Encoding and saving PNG".into(),
                        },
                    });
                    ctx.request_repaint();
                    files::save_png(&path, im, Some(&config)).map_err(|e| format!("{e:#}"))
                })
                .map(|_| path),
            ),
        };
        if events.send(event).is_err() {
            break;
        }
        ctx.request_repaint();
    }
}
/// A frame for the screen. On the interface's own device it is left there; otherwise it is
/// read back, which is what the interface then has to upload again.
fn preview(
    renderer: &mut Option<Renderer>,
    gpu: &Gpu,
    input: &RgbaImage,
    c: &Config,
) -> Result<Preview, String> {
    if gpu.render_state().is_none() {
        return render(renderer, gpu, input, c, None, |_| {}).map(Preview::Pixels);
    }
    // Surface backend validation/device errors in the window, leaving settings usable.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
        if renderer.is_none() {
            *renderer = Some(gpu.renderer()?);
        }
        renderer.as_ref().unwrap().render_preview(input, c)
    }));
    match result {
        Ok(Ok(frame)) => Ok(Preview::Frame(frame)),
        Ok(Err(e)) => Err(format!("{e:#}")),
        Err(_) => {
            *renderer = None;
            Err("Graphics device failed while rendering the preview".into())
        }
    }
}

fn render(
    renderer: &mut Option<Renderer>,
    gpu: &Gpu,
    input: &RgbaImage,
    c: &Config,
    cancel: Option<&AtomicBool>,
    mut progress: impl FnMut(RenderProgress),
) -> Result<RgbaImage, String> {
    // Surface backend validation/device errors in the window, leaving settings usable.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> anyhow::Result<_> {
        if renderer.is_none() {
            progress(RenderProgress {
                fraction: 0.,
                stage: "Initializing graphics device".into(),
            });
            *renderer = Some(gpu.renderer()?);
        }
        let renderer = renderer.as_ref().unwrap();
        let image = match cancel {
            Some(cancel) => {
                renderer
                    .render_with_progress_and_cancel(input, c, cancel, &mut progress)?
                    .crt
            }
            None => renderer.render_with_progress(input, c, &mut progress)?.crt,
        };
        Ok(image)
    }));
    match result {
        Ok(v) => v.map_err(|e| format!("{e:#}")),
        Err(_) => {
            *renderer = None;
            Err("Graphics driver failed. Try a smaller resolution or restart with another --backend. Settings are still available to save.".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thumbnail(generation: u64, index: usize) -> Job {
        Job::Thumbnail {
            generation,
            key: ThumbnailKey::Lut(index),
            input: Arc::new(RgbaImage::new(1, 1)),
            look: Look::Lut(index),
        }
    }

    #[test]
    fn a_preview_goes_before_thumbnails_and_stale_thumbnails_are_dropped() {
        let mut backlog: std::collections::VecDeque<Job> = [
            thumbnail(1, 0),
            thumbnail(1, 1),
            Job::Preview {
                revision: 3,
                input: Arc::new(RgbaImage::new(1, 1)),
                config: Config::default(),
            },
            thumbnail(2, 0),
        ]
        .into();
        assert!(matches!(
            next_job(&mut backlog),
            Some(Job::Preview { revision: 3, .. })
        ));
        // The source changed after the first two were asked for: only the newer one is left.
        assert!(matches!(
            next_job(&mut backlog),
            Some(Job::Thumbnail { generation: 2, .. })
        ));
        assert!(next_job(&mut backlog).is_none());
        backlog.extend([thumbnail(3, 0), Job::Shutdown]);
        assert!(
            next_job(&mut backlog).is_none(),
            "shutdown is never kept waiting"
        );
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
            input: input.clone(),
            config: Config {
                output: "4k".into(),
                warmup: 240,
                ..Config::default()
            },
            path: dir.path().join("export.png"),
            cancel: cancel.clone(),
        })
        .unwrap();
        jobs.send(Job::Preview {
            revision: 7,
            input,
            config: Config {
                output: "320x180".into(),
                warmup: 0,
                ..Config::default()
            },
        })
        .unwrap();
        let timeout = std::time::Duration::from_secs(120);
        loop {
            match events.recv_timeout(timeout).expect("worker stalled") {
                Event::Progress { .. } => continue,
                Event::Preview { revision, result } => {
                    assert_eq!(revision, 7);
                    result.expect("preview");
                    break;
                }
                Event::Exported(_) => panic!("the export finished before the preview"),
                _ => panic!("unexpected event"),
            }
        }
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        loop {
            match events.recv_timeout(timeout).expect("worker stalled") {
                Event::Exported(result) => {
                    assert!(result.is_err(), "export should have been cancelled");
                    break;
                }
                _ => continue,
            }
        }
        jobs.send(Job::Shutdown).unwrap();
        thread.join().unwrap();
    }
}
