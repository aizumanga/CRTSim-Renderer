//! Where a video's frames come from when FFmpeg does not decode them: an animation's are
//! decoded here, and a video file's by the browser.
use anyhow::Result;
use crtsim_media::{
    page::{self, FrameSource, Span},
    Video,
};
use image::RgbaImage;

pub enum Frames {
    Decoded(page::Decoded),
    #[cfg(target_arch = "wasm32")]
    Browser(crate::web_video::Frames),
}

impl FrameSource for Frames {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        match self {
            Self::Decoded(frames) => frames.next().await,
            #[cfg(target_arch = "wasm32")]
            Self::Browser(frames) => frames.next().await,
        }
    }
}

/// The frames of `video` that `span` asks for.
pub async fn open(video: &Video, span: &Span) -> Result<Frames> {
    if let crtsim_media::Source::Demuxed(_) = video.source {
        #[cfg(target_arch = "wasm32")]
        return Ok(Frames::Browser(
            crate::web_video::frames(video, span).await?,
        ));
        #[cfg(not(target_arch = "wasm32"))]
        anyhow::bail!("Video files handed over as bytes are decoded only in a browser");
    }
    Ok(Frames::Decoded(page::decoded(video, span)?))
}

/// Frame `number` of `video`, counting from 0 in the order they show.
pub async fn frame(video: &Video, number: u64) -> Result<RgbaImage> {
    if let crtsim_media::Source::Demuxed(_) = video.source {
        #[cfg(target_arch = "wasm32")]
        return crate::web_video::frame(video, number).await;
        #[cfg(not(target_arch = "wasm32"))]
        anyhow::bail!("Video files handed over as bytes are decoded only in a browser");
    }
    page::frame(video, number)
}
