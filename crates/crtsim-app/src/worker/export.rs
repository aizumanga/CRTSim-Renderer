//! Exports: a still, a contact sheet, a video, an animation or a batch job, rendered on the
//! work lane's renderer and saved.

use super::{still, Colors, Graphics, STILL_FAILED};
use crate::files;
use anyhow::{ensure, Result};
use crtsim_core::config::Config;
use crtsim_media::{Options, Progress, Video};
use image::RgbaImage;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

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
    /// A contact sheet: the still rendered through each look, laid out in a grid.
    Sheet {
        input: Arc<RgbaImage>,
        looks: Vec<crtsim_core::sheet::Look>,
        tile: (u32, u32),
        columns: u32,
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
    pub(super) fn driver_failed(&self) -> &'static str {
        match self {
            Self::Image { .. } | Self::Sheet { .. } => STILL_FAILED,
            Self::Video { .. } => "Video graphics driver failed. Try a smaller resolution.",
            Self::Animation { .. } => "Animation graphics driver failed. Try a smaller size.",
            Self::Batch { .. } => "Batch graphics driver failed. Try a smaller resolution.",
        }
    }

    pub(super) async fn run(
        &self,
        graphics: &mut Graphics,
        path: &Path,
        cancel: &Arc<AtomicBool>,
        progress: &dyn Fn(Progress),
        colors: &Colors,
    ) -> Result<()> {
        let space = &colors.space;
        match self {
            Self::Image { input, config } => {
                export_image(graphics, input, config, path, cancel, progress, space).await
            }
            Self::Sheet {
                input,
                looks,
                tile,
                columns,
            } => {
                let renderer = graphics.renderer(|| {}).await?;
                let count = looks.len();
                let sheet = crtsim_core::sheet::render(
                    renderer,
                    input,
                    looks,
                    *tile,
                    *columns,
                    Some(cancel),
                    |done| {
                        progress(Progress {
                            fraction: done as f32 / count as f32 * 0.95,
                            stage: format!("Rendered {done} of {count} looks"),
                        })
                    },
                )
                .await?;
                ensure!(!cancel.load(Ordering::Relaxed), "Render cancelled");
                progress(Progress {
                    fraction: 0.97,
                    stage: "Encoding and saving PNG".into(),
                });
                files::save_png(path, sheet, None, space)
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
            } => export_animation(graphics, video, config, options, path, cancel, progress).await,
            Self::Batch {
                source,
                options,
                config,
            } => {
                if crtsim_media::Container::of(path).is_some() {
                    let video = crtsim_media::probe(source, cancel)?;
                    export_video(graphics, &video, config, options, path, cancel, progress).await
                } else {
                    let input = crtsim_core::input::read(source, colors.profiles)?.image;
                    export_image(graphics, &input, config, path, cancel, progress, space).await
                }
            }
        }
    }
}

/// A still at full resolution, saved as a PNG that carries its settings.
async fn export_image(
    graphics: &mut Graphics,
    input: &RgbaImage,
    config: &Config,
    path: &Path,
    cancel: &AtomicBool,
    progress: &dyn Fn(Progress),
    space: &crtsim_core::color::Space,
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
    files::save_png(path, image, Some(config), space)
}

/// An animated GIF or WebP of `video`: encoded by FFmpeg on the desktop, and in a browser
/// written here.
async fn export_animation(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &crtsim_media::AnimationOptions,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    if cfg!(target_arch = "wasm32") {
        return export_animation_here(graphics, video, config, options, path, cancel, progress)
            .await;
    }
    let renderer = graphics.renderer(|| {}).await?;
    crtsim_media::export_animation(video, path, config, options, renderer, cancel, progress)
}

/// An animation made here without FFmpeg, as a browser makes it, and saved, which in a
/// browser is a download.
pub(super) async fn export_animation_here(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &crtsim_media::AnimationOptions,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    use crtsim_media::jobs;
    let format = crtsim_media::AnimationFormat::of(path)
        .ok_or_else(|| anyhow::anyhow!("Choose a GIF, WebP or PNG filename"))?;
    let renderer = graphics.renderer(|| {}).await?;
    let bytes = jobs::export_animation(
        video,
        format,
        config,
        options,
        async |span| crate::frames::open(video, span, cancel).await,
        async |sequence, frame, config| {
            jobs::render(renderer, sequence, frame, config, cancel).await
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
        crtsim_media::AnimationFormat::Apng => "image/png",
    };
    files::deliver(path, &bytes, mime)
}

/// A whole video exported in a browser without FFmpeg, and downloaded. Its frames and sound
/// are encoded by the browser.
#[cfg(target_arch = "wasm32")]
async fn export_video(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &Options,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    let container = crtsim_media::Container::of(path)
        .ok_or_else(|| anyhow::anyhow!("Choose an MP4 or WebM filename"))?;
    let renderer = graphics.renderer(|| {}).await?;
    let bytes = crtsim_media::jobs::export_video(
        video,
        container,
        config,
        options,
        async |span| crate::frames::open(video, span, cancel).await,
        async |sequence, frame, config| {
            crtsim_media::jobs::render(renderer, sequence, frame, config, cancel).await
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
    files::deliver(path, &bytes, mime)
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

/// A whole video rendered and encoded by FFmpeg, with its audio and other tracks.
#[cfg(not(target_arch = "wasm32"))]
async fn export_video(
    graphics: &mut Graphics,
    video: &Video,
    config: &Config,
    options: &Options,
    path: &Path,
    cancel: &Arc<AtomicBool>,
    progress: &dyn Fn(Progress),
) -> Result<()> {
    let renderer = graphics.renderer(|| {}).await?;
    crtsim_media::export(video, path, config, options, renderer, cancel, progress)
}
