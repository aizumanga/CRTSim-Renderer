use crate::files;
use crtsim_core::{config::Config, RenderProgress, Renderer};
use eframe::egui;
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, mpsc, Arc},
    time::Instant,
};

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
    Export {
        input: Arc<RgbaImage>,
        config: Config,
        path: PathBuf,
        cancel: Arc<AtomicBool>,
    },
}
pub struct PlaybackFrame {
    pub time: f64,
    pub source: RgbaImage,
    pub crt: RgbaImage,
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
        result: Result<(RgbaImage, f32), String>,
    },
    Exported(Result<PathBuf, String>),
}
pub fn start(
    ctx: egui::Context,
    backends: wgpu::Backends,
) -> (
    mpsc::Sender<Job>,
    mpsc::Receiver<Event>,
    std::thread::JoinHandle<()>,
) {
    let (send, jobs) = mpsc::channel();
    let (events, receive) = mpsc::channel();
    let thread = std::thread::spawn(move || {
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
                                renderer = Some(pollster::block_on(Renderer::new(backends))?);
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
                                                std::thread::sleep(
                                                    std::time::Duration::from_millis(5),
                                                );
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
                                    renderer = Some(
                                        pollster::block_on(Renderer::new(backends))
                                            .map_err(|e| format!("{e:#}"))?,
                                    );
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
                                let input =
                                    files::load_image(&source).map_err(|e| format!("{e:#}"))?;
                                let im = render(
                                    &mut renderer,
                                    backends,
                                    &input,
                                    &config,
                                    Some(&cancel),
                                    |p| {
                                        let _ = events.send(Event::Progress { progress: p });
                                        ctx.request_repaint();
                                    },
                                )?;
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
                                renderer = Some(pollster::block_on(Renderer::new(backends))?);
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
                Job::Preview {
                    revision,
                    input,
                    config,
                } => {
                    let started = Instant::now();
                    let result = render(&mut renderer, backends, &input, &config, None, |_| {})
                        .map(|im| (im, started.elapsed().as_secs_f32()));
                    Event::Preview { revision, result }
                }
                Job::Export {
                    input,
                    config,
                    path,
                    cancel,
                } => Event::Exported(
                    render(
                        &mut renderer,
                        backends,
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
    });
    (send, receive, thread)
}
fn render(
    renderer: &mut Option<Renderer>,
    backends: wgpu::Backends,
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
            *renderer = Some(pollster::block_on(Renderer::new(backends))?);
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
