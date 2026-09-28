//! Videos decoded by the browser: the container read in Rust (`crtsim_media::demux`), its
//! frames decoded by WebCodecs, and each frame drawn upright into a canvas to read it as RGBA.
//!
//! WebCodecs is bound here rather than through web-sys, which offers it only behind an
//! unstable build flag.
use anyhow::{anyhow, bail, ensure, Context, Result};
use crtsim_media::{
    demux::Demuxed,
    page::{FrameSource, Span, Ticks},
    Contents, Source, Video,
};
use image::RgbaImage;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    sync::Arc,
};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    type VideoDecoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &js_sys::Object) -> Result<VideoDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &VideoDecoder, config: &js_sys::Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn decode(this: &VideoDecoder, chunk: &EncodedVideoChunk) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn flush(this: &VideoDecoder) -> js_sys::Promise;
    #[wasm_bindgen(method, catch)]
    fn close(this: &VideoDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    fn decode_queue_size(this: &VideoDecoder) -> u32;
    #[wasm_bindgen(method, setter, js_name = ondequeue)]
    fn set_ondequeue(this: &VideoDecoder, handler: &JsValue);
    #[wasm_bindgen(static_method_of = VideoDecoder, js_name = isConfigSupported)]
    fn is_config_supported(config: &js_sys::Object) -> js_sys::Promise;

    type EncodedVideoChunk;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &js_sys::Object) -> Result<EncodedVideoChunk, JsValue>;

    type VideoFrame;
    #[wasm_bindgen(method, getter)]
    fn timestamp(this: &VideoFrame) -> f64;
    #[wasm_bindgen(method)]
    fn close(this: &VideoFrame);

    /// A 2D canvas context, for its `drawImage` that takes a video frame.
    type Canvas2d;
    #[wasm_bindgen(method, catch, js_name = drawImage)]
    fn draw_frame(
        this: &Canvas2d,
        frame: &VideoFrame,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), JsValue>;
}

fn failed(error: JsValue) -> anyhow::Error {
    anyhow!(
        "{}",
        error.as_string().unwrap_or_else(|| format!("{error:?}"))
    )
}

fn object(fields: &[(&str, JsValue)]) -> js_sys::Object {
    let object = js_sys::Object::new();
    for (name, value) in fields {
        let _ = js_sys::Reflect::set(&object, &JsValue::from_str(name), value);
    }
    object
}

/// A sample's time as WebCodecs counts it, in whole microseconds.
fn micros(seconds: f64) -> i64 {
    (seconds * 1e6).round() as i64
}

/// What the decoder's callbacks leave for the reader, and how they wake it.
#[derive(Default)]
struct Shared {
    frames: RefCell<VecDeque<VideoFrame>>,
    error: RefCell<Option<String>>,
    flushed: Cell<bool>,
    wake: RefCell<Option<js_sys::Function>>,
}

impl Shared {
    fn wake(&self) {
        if let Some(resolve) = self.wake.borrow_mut().take() {
            let _ = resolve.call0(&JsValue::NULL);
        }
    }

    /// Waits until a callback has something new.
    async fn changed(&self) {
        let changed = js_sys::Promise::new(&mut |resolve, _| {
            *self.wake.borrow_mut() = Some(resolve);
        });
        let _ = JsFuture::from(changed).await;
    }
}

/// A WebCodecs decoder fed a video's samples in order from one of them.
struct Decoder {
    decoder: VideoDecoder,
    shared: Rc<Shared>,
    _callbacks: [Closure<dyn FnMut(JsValue)>; 3],
    demuxed: Arc<Demuxed>,
    contents: Contents,
    next: usize,
    flushing: bool,
}

impl Decoder {
    /// A decoder for `video`, to be fed from sample `from` in decoding order, a keyframe.
    async fn open(demuxed: &Arc<Demuxed>, contents: &Contents, from: usize) -> Result<Self> {
        let track = &demuxed.video;
        let mut fields = vec![
            ("codec", JsValue::from_str(&track.codec)),
            ("codedWidth", track.coded.0.into()),
            ("codedHeight", track.coded.1.into()),
        ];
        if let Some(description) = &track.description {
            fields.push((
                "description",
                js_sys::Uint8Array::from(&description[..]).into(),
            ));
        }
        let config = object(&fields);
        let support = JsFuture::from(VideoDecoder::is_config_supported(&config))
            .await
            .map_err(failed)?;
        let supported = js_sys::Reflect::get(&support, &"supported".into())
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        ensure!(
            supported,
            "This browser cannot decode this video ({}). The desktop app can open it.",
            track.codec
        );
        let shared = Rc::new(Shared::default());
        let output = {
            let shared = shared.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |frame: JsValue| {
                shared.frames.borrow_mut().push_back(frame.unchecked_into());
                shared.wake();
            })
        };
        let error = {
            let shared = shared.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
                *shared.error.borrow_mut() = Some(failed(error).to_string());
                shared.wake();
            })
        };
        let dequeue = {
            let shared = shared.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| shared.wake())
        };
        let decoder = VideoDecoder::new(&object(&[
            ("output", output.as_ref().clone()),
            ("error", error.as_ref().clone()),
        ]))
        .map_err(failed)?;
        decoder.set_ondequeue(dequeue.as_ref());
        decoder.configure(&config).map_err(failed)?;
        Ok(Self {
            decoder,
            shared,
            _callbacks: [output, error, dequeue],
            demuxed: demuxed.clone(),
            contents: contents.clone(),
            next: from,
            flushing: false,
        })
    }

    fn feed(&mut self) -> Result<()> {
        let sample = &self.demuxed.video.samples[self.next];
        self.next += 1;
        let data = js_sys::Uint8Array::from(&self.contents.as_ref()[sample.range.clone()]);
        let chunk = EncodedVideoChunk::new(&object(&[
            (
                "type",
                JsValue::from_str(if sample.key { "key" } else { "delta" }),
            ),
            ("timestamp", (micros(sample.time) as f64).into()),
            ("duration", (micros(sample.duration) as f64).into()),
            ("data", data.into()),
        ]))
        .map_err(failed)?;
        self.decoder.decode(&chunk).map_err(failed)
    }

    /// The next decoded frame, in the order they show, or `None` after the last.
    async fn next(&mut self) -> Result<Option<VideoFrame>> {
        let samples = self.demuxed.video.samples.len();
        loop {
            if let Some(error) = self.shared.error.borrow_mut().take() {
                bail!("The browser could not decode the video: {error}");
            }
            if let Some(frame) = self.shared.frames.borrow_mut().pop_front() {
                return Ok(Some(frame));
            }
            if self.shared.flushed.get() {
                return Ok(None);
            }
            // A few samples ahead keep the decoder busy; more would only hold frames.
            if self.next < samples && self.decoder.decode_queue_size() < 3 {
                self.feed()?;
                continue;
            }
            if self.next == samples && !self.flushing {
                self.flushing = true;
                let (flushed, shared) = (self.decoder.flush(), self.shared.clone());
                wasm_bindgen_futures::spawn_local(async move {
                    if let Err(error) = JsFuture::from(flushed).await {
                        shared
                            .error
                            .borrow_mut()
                            .get_or_insert(failed(error).to_string());
                    }
                    shared.flushed.set(true);
                    shared.wake();
                });
            }
            self.shared.changed().await;
        }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        // Closing empties the queue, which the browser announces after the callback is gone.
        self.decoder.set_ondequeue(&JsValue::NULL);
        let _ = self.decoder.close();
        for frame in self.shared.frames.borrow_mut().drain(..) {
            frame.close();
        }
    }
}

/// Draws frames upright at the video's size and reads them back.
struct Painter {
    context: web_sys::OffscreenCanvasRenderingContext2d,
    size: (u32, u32),
    display: (u32, u32),
    rotation: u32,
}

impl Painter {
    fn new(video: &Video, demuxed: &Demuxed) -> Result<Self> {
        let canvas = web_sys::OffscreenCanvas::new(video.size.0, video.size.1).map_err(failed)?;
        // Every frame is read back, which a canvas kept in memory does faster.
        let options = object(&[("willReadFrequently", JsValue::TRUE)]);
        let context = canvas
            .get_context_with_context_options("2d", &options)
            .map_err(failed)?
            .context("The browser gave no canvas to draw frames on")?
            .unchecked_into();
        Ok(Self {
            context,
            size: video.size,
            display: demuxed.video.display,
            rotation: demuxed.video.rotation,
        })
    }

    fn paint(&self, frame: &VideoFrame) -> Result<RgbaImage> {
        let (width, height) = (f64::from(self.size.0), f64::from(self.size.1));
        // Turned clockwise about the canvas, then moved back onto it.
        let (a, b, c, d, e, f) = match self.rotation {
            90 => (0., 1., -1., 0., width, 0.),
            180 => (-1., 0., 0., -1., width, height),
            270 => (0., -1., 1., 0., 0., height),
            _ => (1., 0., 0., 1., 0., 0.),
        };
        let context = &self.context;
        context.set_transform(a, b, c, d, e, f).map_err(failed)?;
        let (w, h) = (f64::from(self.display.0), f64::from(self.display.1));
        context
            .unchecked_ref::<Canvas2d>()
            .draw_frame(frame, 0., 0., w, h)
            .map_err(failed)?;
        let pixels = context
            .get_image_data(0., 0., width, height)
            .map_err(failed)?
            .data()
            .0;
        RgbaImage::from_raw(self.size.0, self.size.1, pixels)
            .context("The frame's pixels are cut short")
    }
}

fn demuxed(video: &Video) -> Result<(&Arc<Demuxed>, &Contents)> {
    match (&video.source, &video.contents) {
        (Source::Demuxed(demuxed), Some(contents)) => Ok((demuxed, contents)),
        _ => bail!("This video is not decoded by the browser"),
    }
}

/// The keyframe, in decoding order, to decode from to reach a frame showing at `time`.
fn keyframe_before(demuxed: &Demuxed, time: f64) -> usize {
    let samples = &demuxed.video.samples;
    (0..samples.len())
        .rev()
        .find(|&i| samples[i].key && samples[i].time <= time + 1e-9)
        .unwrap_or(0)
}

/// Frame `number` of `video` in the order they show, counting from 0.
pub async fn frame(video: &Video, number: u64) -> Result<RgbaImage> {
    let (demuxed, contents) = demuxed(video)?;
    let track = &demuxed.video;
    let sample = *track
        .shown
        .get(number as usize)
        .context("Frame is outside the video")?;
    let wanted = micros(track.samples[sample].time);
    let mut decoder = Decoder::open(
        demuxed,
        contents,
        keyframe_before(demuxed, track.samples[sample].time),
    )
    .await?;
    let painter = Painter::new(video, demuxed)?;
    while let Some(frame) = decoder.next().await? {
        let found = frame.timestamp() as i64 == wanted;
        let image = found.then(|| painter.paint(&frame));
        frame.close();
        if let Some(image) = image {
            return image;
        }
    }
    bail!("The browser did not decode frame {}", number + 1)
}

/// The frames of `video` that a span at a constant rate takes.
pub struct Frames {
    decoder: Decoder,
    painter: Painter,
    ticks: Ticks,
    /// The latest frame decoded, not yet known to be the last one before a tick.
    latest: Option<VideoFrame>,
    /// The frame to give again, and how many more times.
    held: Option<(RgbaImage, u64)>,
    ended: bool,
}

pub async fn frames(video: &Video, span: &Span) -> Result<Frames> {
    let (demuxed, contents) = demuxed(video)?;
    let fps = span.fps.context("Frames are read at a constant rate")?;
    let from = keyframe_before(demuxed, span.start + 0.5 / fps);
    Ok(Frames {
        decoder: Decoder::open(demuxed, contents, from).await?,
        painter: Painter::new(video, demuxed)?,
        ticks: Ticks::new(span, fps, demuxed.duration),
        latest: None,
        held: None,
        ended: false,
    })
}

impl Frames {
    /// Holds the latest frame for `ticks` ticks, and moves on to `next`.
    fn hold(&mut self, ticks: u64, next: Option<VideoFrame>) -> Result<()> {
        let latest = std::mem::replace(&mut self.latest, next);
        if let Some(frame) = latest {
            if ticks > 0 {
                self.held = Some((self.painter.paint(&frame)?, ticks));
            }
            frame.close();
        }
        Ok(())
    }
}

impl FrameSource for Frames {
    async fn next(&mut self) -> Result<Option<RgbaImage>> {
        loop {
            if let Some((image, left)) = &mut self.held {
                *left -= 1;
                let image = if *left == 0 {
                    self.held.take().map(|(image, _)| image)
                } else {
                    Some(image.clone())
                };
                return Ok(image);
            }
            if self.ended || self.ticks.done() {
                return Ok(None);
            }
            match self.decoder.next().await? {
                Some(frame) => {
                    let ticks = self.ticks.before(frame.timestamp() / 1e6);
                    self.hold(ticks, Some(frame))?;
                }
                None => {
                    let ticks = self.ticks.rest();
                    self.hold(ticks, None)?;
                    self.ended = true;
                }
            }
        }
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        if let Some(frame) = self.latest.take() {
            frame.close();
        }
    }
}
