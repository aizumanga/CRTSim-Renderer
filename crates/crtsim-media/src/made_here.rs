//! Outputs made without FFmpeg, as a browser page makes them: animated GIF and WebP written
//! here, frame by frame, into a file in memory.
use crate::{
    gif_writer::{GifWriter, Histogram},
    jobs::Output,
    webp_writer::{self, WebpWriter},
    AnimationFormat, AnimationOptions, Progress,
};
use anyhow::{ensure, Result};
use image::RgbaImage;

/// An animated GIF or WebP written here. A GIF's first pass only counts colors; its second,
/// and a WebP's only pass, write. A WebP's changed areas are written lossless here, or lossy
/// by `lossy`, which the host supplies.
pub struct Animation<L> {
    format: AnimationFormat,
    size: (u32, u32),
    fps: u32,
    dither: crate::Dither,
    lossless: bool,
    quality: u8,
    histogram: Histogram,
    gif: Option<GifWriter>,
    webp: Option<WebpWriter>,
    lossy: L,
}

impl<L: AsyncFnMut(&RgbaImage, u8) -> Result<Vec<u8>>> Animation<L> {
    pub fn new(
        format: AnimationFormat,
        size: (u32, u32),
        options: &AnimationOptions,
        lossy: L,
    ) -> Result<Self> {
        let webp = match format {
            AnimationFormat::Webp => Some(WebpWriter::new(size, options.fps)?),
            AnimationFormat::Gif => None,
        };
        Ok(Self {
            format,
            size,
            fps: options.fps,
            dither: options.dither,
            lossless: options.lossless,
            quality: options.quality,
            histogram: Histogram::default(),
            gif: None,
            webp,
            lossy,
        })
    }
}

impl<L: AsyncFnMut(&RgbaImage, u8) -> Result<Vec<u8>>> Output for Animation<L> {
    type Made = Vec<u8>;

    fn passes(&self) -> u32 {
        match self.format {
            AnimationFormat::Gif => 2,
            AnimationFormat::Webp => 1,
        }
    }

    fn pass_name(&self, pass: u32) -> &'static str {
        match (self.format, pass) {
            (AnimationFormat::Gif, 0) => " · choosing colors",
            (AnimationFormat::Gif, _) => " · writing",
            (AnimationFormat::Webp, _) => "",
        }
    }

    async fn frame(&mut self, pass: u32, frame: RgbaImage) -> Result<()> {
        if let Some(writer) = &mut self.webp {
            let change = writer.changed(&frame)?;
            let picture = match change {
                None => None,
                Some(rect) if self.lossless => Some(webp_writer::lossless(&rect.crop(&frame))?),
                Some(rect) => {
                    let picture = (self.lossy)(&rect.crop(&frame), self.quality).await?;
                    ensure!(
                        picture.starts_with(b"RIFF"),
                        "This browser cannot write lossy WebP. Choose Lossless, or GIF."
                    );
                    Some(picture)
                }
            };
            return writer.add(&frame, change.zip(picture.as_deref()));
        }
        if pass == 0 {
            self.histogram.add(&frame);
            return Ok(());
        }
        let writer = match &mut self.gif {
            Some(writer) => writer,
            None => self.gif.insert(GifWriter::new(
                self.size,
                self.fps,
                self.histogram.palette(),
                self.dither,
            )?),
        };
        writer.frame(&frame)
    }

    async fn finish(self, _count: u64, _progress: &dyn Fn(Progress)) -> Result<Vec<u8>> {
        match (self.gif, self.webp) {
            (_, Some(writer)) => writer.finish(),
            (Some(writer), _) => writer.finish(),
            (None, None) => anyhow::bail!("No frames were written"),
        }
    }
}
