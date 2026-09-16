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
    Shutdown,
    Load(PathBuf),
    LoadVideo {
        path: PathBuf,
        time: f64,
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
    },
}
pub enum Event {
    Progress {
        progress: RenderProgress,
    },
    Loaded(Result<(PathBuf, RgbaImage, RgbaImage), String>),
    VideoLoaded(Result<(crtsim_media::Video, RgbaImage, RgbaImage, f64), String>),
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
                Job::LoadVideo { path, time, cancel } => Event::VideoLoaded(
                    (|| -> anyhow::Result<_> {
                        let video = crtsim_media::probe(&path, &cancel)?;
                        let image = crtsim_media::preview(&video, time, &cancel)?;
                        let thumb = image::DynamicImage::ImageRgba8(image.clone())
                            .thumbnail(2048, 2048)
                            .to_rgba8();
                        Ok((video, image, thumb, time))
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
                    let result = render(&mut renderer, backends, &input, &config, |_| {})
                        .map(|im| (im, started.elapsed().as_secs_f32()));
                    Event::Preview { revision, result }
                }
                Job::Export {
                    input,
                    config,
                    path,
                } => Event::Exported(
                    render(&mut renderer, backends, &input, &config, |mut progress| {
                        progress.fraction *= 0.9;
                        let _ = events.send(Event::Progress { progress });
                        ctx.request_repaint();
                    })
                    .and_then(|im| {
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
        let image = renderer.render_with_progress(input, c, &mut progress)?.crt;
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
