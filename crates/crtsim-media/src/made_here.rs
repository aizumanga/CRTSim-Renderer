//! Outputs made without FFmpeg, as a browser page makes them, into a file in memory: animated
//! GIF and WebP written here, frame by frame, and MP4 and WebM encoded by the host and muxed
//! here.
use crate::{
    gif_writer::{GifWriter, Histogram},
    jobs::{Output, VideoEncoding},
    mux,
    webp_writer::{self, WebpWriter},
    AnimationFormat, AnimationOptions, Audio, Container, Options, Progress, Source,
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

/// An MP4 or WebM: frames encoded by the host, such as a browser's WebCodecs, with a keyframe
/// every two seconds for seeking, and muxed here with the source's sound and the preset the
/// export used, which an import reads back. Sound the container holds as it is, is copied;
/// otherwise, or when the options ask for it, `reencode` turns it into Opus, which both
/// containers hold.
pub struct Video<'v, E, R> {
    video: &'v crate::Video,
    container: Container,
    /// The preset, as the container's comment.
    preset: String,
    audio: Audio,
    fps: f64,
    /// Frames between keyframes.
    group: u64,
    encoder: E,
    reencode: R,
    count: u64,
}

impl<'v, E, R> Video<'v, E, R> {
    pub fn new(
        video: &'v crate::Video,
        container: Container,
        preset: String,
        options: &Options,
        fps: f64,
        encoder: E,
        reencode: R,
    ) -> Self {
        Self {
            video,
            container,
            preset,
            audio: options.audio,
            fps,
            group: (fps * 2.).round().max(1.) as u64,
            encoder,
            reencode,
            count: 0,
        }
    }

    /// The source's sound as it is stored, if it is kept and the browser was handed it.
    fn source_audio(&self) -> Option<mux::EncodedAudio> {
        let (Source::Demuxed(demuxed), Some(contents)) = (&self.video.source, &self.video.contents)
        else {
            return None;
        };
        if self.audio == Audio::Mute {
            return None;
        }
        demuxed.audio.as_ref().map(|track| mux::EncodedAudio {
            codec: track.codec.clone(),
            description: track.description.clone(),
            sample_rate: track.sample_rate,
            channels: track.channels,
            packets: track
                .samples
                .iter()
                .map(|sample| mux::Packet {
                    data: contents.as_ref()[sample.range.clone()].to_vec(),
                    time: sample.time,
                    duration: sample.duration,
                    key: true,
                })
                .collect(),
        })
    }
}

impl<E, R> Output for Video<'_, E, R>
where
    E: VideoEncoding,
    R: AsyncFnOnce(&mux::EncodedAudio) -> Result<mux::EncodedAudio>,
{
    type Made = Vec<u8>;

    async fn frame(&mut self, _pass: u32, frame: RgbaImage) -> Result<()> {
        let time = self.count as f64 / self.fps;
        let key = self.count.is_multiple_of(self.group);
        self.encoder
            .encode(&frame, time, 1. / self.fps, key)
            .await?;
        self.count += 1;
        Ok(())
    }

    async fn finish(self, _count: u64, progress: &dyn Fn(Progress)) -> Result<Vec<u8>> {
        progress(Progress {
            fraction: 0.92,
            stage: "Finishing encoding".into(),
        });
        let audio = self.source_audio();
        let encoded = self.encoder.finish().await?;
        let container = self.container;
        let audio = match audio {
            Some(audio)
                if self.audio == Audio::Encode || !mux::takes_audio(container, &audio.codec) =>
            {
                progress(Progress {
                    fraction: 0.95,
                    stage: "Converting the sound to Opus".into(),
                });
                Some((self.reencode)(&audio).await.map_err(|error| {
                    anyhow::anyhow!(
                        "Cannot convert the sound ({}) to Opus for {}: {error:#}. Choose No \
                         audio, or another format.",
                        audio.codec,
                        container.extension().to_uppercase()
                    )
                })?)
            }
            audio => audio,
        };
        progress(Progress {
            fraction: 0.98,
            stage: "Writing the file".into(),
        });
        mux::write(container, &encoded, audio.as_ref(), Some(&self.preset))
    }
}
