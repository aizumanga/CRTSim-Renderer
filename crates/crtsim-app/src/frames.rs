//! Where a video's frames come from: wherever `crtsim_media` decodes them, or, for a video
//! file handed over as bytes, the browser.
use anyhow::Result;
use crtsim_media::{
    jobs::{self, FrameSource, Span},
    Video,
};
use image::RgbaImage;
use std::sync::{atomic::AtomicBool, Arc};

pub enum Frames {
    Media(jobs::Frames),
    #[cfg(target_arch = "wasm32")]
    Browser(crate::web_video::Frames),
}

impl FrameSource for Frames {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        match self {
            Self::Media(frames) => frames.next().await,
            #[cfg(target_arch = "wasm32")]
            Self::Browser(frames) => frames.next().await,
        }
    }
}

/// The frames of `video` that `span` asks for.
pub async fn open(video: &Video, span: &Span, cancel: &Arc<AtomicBool>) -> Result<Frames> {
    #[cfg(target_arch = "wasm32")]
    if let crtsim_media::Source::Demuxed(_) = video.source {
        return Ok(Frames::Browser(
            crate::web_video::frames(video, span).await?,
        ));
    }
    Ok(Frames::Media(jobs::frames(video, span, cancel)?))
}

/// Frame `number` of `video`, counting from 0 in the order they show.
pub async fn frame(video: &Video, number: u64, cancel: &Arc<AtomicBool>) -> Result<RgbaImage> {
    #[cfg(target_arch = "wasm32")]
    if let crtsim_media::Source::Demuxed(_) = video.source {
        return crate::web_video::frame(video, number).await;
    }
    jobs::frame(video, number, cancel)
}
