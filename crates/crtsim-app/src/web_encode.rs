//! Video and sound encoded by the browser's WebCodecs, for exports the page writes itself.
//!
//! WebCodecs is bound here rather than through web-sys, which offers it only behind an
//! unstable build flag.
use anyhow::{anyhow, bail, ensure, Result};
use crtsim_media::{
    mux::{EncodedAudio, EncodedVideo, Packet},
    page::{EncoderSettings, VideoEncoding},
    Container,
};
use image::RgbaImage;
use std::{cell::RefCell, rc::Rc};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    type VideoEncoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &js_sys::Object) -> Result<VideoEncoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &VideoEncoder, config: &js_sys::Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn encode(
        this: &VideoEncoder,
        frame: &VideoFrame,
        options: &js_sys::Object,
    ) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn flush(this: &VideoEncoder) -> js_sys::Promise;
    #[wasm_bindgen(method, catch)]
    fn close(this: &VideoEncoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = encodeQueueSize)]
    fn encode_queue_size(this: &VideoEncoder) -> u32;
    #[wasm_bindgen(method, setter, js_name = ondequeue)]
    fn set_ondequeue(this: &VideoEncoder, handler: &JsValue);
    #[wasm_bindgen(static_method_of = VideoEncoder, js_name = isConfigSupported)]
    fn is_config_supported(config: &js_sys::Object) -> js_sys::Promise;

    type VideoFrame;
    #[wasm_bindgen(constructor, catch)]
    fn new(data: &js_sys::Uint8Array, init: &js_sys::Object) -> Result<VideoFrame, JsValue>;
    #[wasm_bindgen(method)]
    fn close(this: &VideoFrame);

    type AudioDecoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &js_sys::Object) -> Result<AudioDecoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &AudioDecoder, config: &js_sys::Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn decode(this: &AudioDecoder, chunk: &EncodedAudioChunk) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn flush(this: &AudioDecoder) -> js_sys::Promise;
    #[wasm_bindgen(method, catch)]
    fn close(this: &AudioDecoder) -> Result<(), JsValue>;
    #[wasm_bindgen(method, getter, js_name = decodeQueueSize)]
    fn decode_queue_size(this: &AudioDecoder) -> u32;
    #[wasm_bindgen(static_method_of = AudioDecoder, js_name = isConfigSupported)]
    fn is_config_supported(config: &js_sys::Object) -> js_sys::Promise;

    type AudioEncoder;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &js_sys::Object) -> Result<AudioEncoder, JsValue>;
    #[wasm_bindgen(method, catch)]
    fn configure(this: &AudioEncoder, config: &js_sys::Object) -> Result<(), JsValue>;
    #[wasm_bindgen(method, catch)]
    fn encode(this: &AudioEncoder, data: &AudioData) -> Result<(), JsValue>;
    #[wasm_bindgen(method)]
    fn flush(this: &AudioEncoder) -> js_sys::Promise;
    #[wasm_bindgen(method, catch)]
    fn close(this: &AudioEncoder) -> Result<(), JsValue>;
    #[wasm_bindgen(static_method_of = AudioEncoder, js_name = isConfigSupported)]
    fn is_config_supported(config: &js_sys::Object) -> js_sys::Promise;

    type AudioData;
    #[wasm_bindgen(method)]
    fn close(this: &AudioData);

    type EncodedAudioChunk;
    #[wasm_bindgen(constructor, catch)]
    fn new(init: &js_sys::Object) -> Result<EncodedAudioChunk, JsValue>;

    /// An encoded video or audio chunk, as an encoder hands it over.
    type Chunk;
    #[wasm_bindgen(method, getter, js_name = type)]
    fn kind(this: &Chunk) -> String;
    #[wasm_bindgen(method, getter)]
    fn timestamp(this: &Chunk) -> f64;
    #[wasm_bindgen(method, getter)]
    fn duration(this: &Chunk) -> Option<f64>;
    #[wasm_bindgen(method, getter, js_name = byteLength)]
    fn byte_length(this: &Chunk) -> u32;
    #[wasm_bindgen(method, catch, js_name = copyTo)]
    fn copy_to(this: &Chunk, destination: &js_sys::Uint8Array) -> Result<(), JsValue>;
}

/// A coder's output callback, given each chunk and what comes with it.
type Output = Closure<dyn FnMut(JsValue, JsValue)>;
/// A callback given one value.
type Callback = Closure<dyn FnMut(JsValue)>;

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

/// The bytes of an `ArrayBuffer`, or of the part of one an `ArrayBufferView` shows.
fn buffer_bytes(source: &JsValue) -> Vec<u8> {
    if source.is_instance_of::<js_sys::ArrayBuffer>() {
        return js_sys::Uint8Array::new(source).to_vec();
    }
    let number = |name: &str| {
        js_sys::Reflect::get(source, &name.into())
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(0.) as u32
    };
    let buffer = js_sys::Reflect::get(source, &"buffer".into()).unwrap_or(JsValue::UNDEFINED);
    js_sys::Uint8Array::new_with_byte_offset_and_length(
        &buffer,
        number("byteOffset"),
        number("byteLength"),
    )
    .to_vec()
}

fn micros(seconds: f64) -> f64 {
    (seconds * 1e6).round()
}

/// Whether the browser says it can use `config`.
async fn supported(check: js_sys::Promise) -> bool {
    let Ok(support) = JsFuture::from(check).await else {
        return false;
    };
    js_sys::Reflect::get(&support, &"supported".into())
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// What an encoder's callbacks leave for the page: packets, the decoder configuration that
/// comes with the first, an error, and a way to wake the page when there is more.
#[derive(Default)]
struct Collected {
    packets: RefCell<Vec<Packet>>,
    description: RefCell<Option<Vec<u8>>>,
    error: RefCell<Option<String>>,
    wake: RefCell<Option<js_sys::Function>>,
}

impl Collected {
    fn wake(&self) {
        if let Some(resolve) = self.wake.borrow_mut().take() {
            let _ = resolve.call0(&JsValue::NULL);
        }
    }

    async fn changed(&self) {
        let changed = js_sys::Promise::new(&mut |resolve, _| {
            *self.wake.borrow_mut() = Some(resolve);
        });
        let _ = JsFuture::from(changed).await;
    }

    fn check(&self) -> Result<()> {
        match self.error.borrow_mut().take() {
            Some(error) => bail!("{error}"),
            None => Ok(()),
        }
    }

    /// Callbacks for an encoder: its output and its errors.
    fn callbacks(self: &Rc<Self>) -> (Output, Callback) {
        let output = {
            let collected = self.clone();
            Closure::<dyn FnMut(JsValue, JsValue)>::new(move |chunk: JsValue, metadata: JsValue| {
                let chunk: Chunk = chunk.unchecked_into();
                let data = js_sys::Uint8Array::new_with_length(chunk.byte_length());
                if let Err(error) = chunk.copy_to(&data) {
                    *collected.error.borrow_mut() = Some(failed(error).to_string());
                }
                collected.packets.borrow_mut().push(Packet {
                    data: data.to_vec(),
                    time: chunk.timestamp() / 1e6,
                    duration: chunk.duration().unwrap_or(0.) / 1e6,
                    key: chunk.kind() == "key",
                });
                let description = js_sys::Reflect::get(&metadata, &"decoderConfig".into())
                    .ok()
                    .filter(|config| !config.is_undefined())
                    .and_then(|config| js_sys::Reflect::get(&config, &"description".into()).ok())
                    .filter(|description| !description.is_undefined());
                if let Some(description) = description {
                    *collected.description.borrow_mut() = Some(buffer_bytes(&description));
                }
                collected.wake();
            })
        };
        let error = {
            let collected = self.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |error: JsValue| {
                *collected.error.borrow_mut() = Some(failed(error).to_string());
                collected.wake();
            })
        };
        (output, error)
    }
}

/// The containers this browser can write video into, checked once when the page starts.
pub async fn check_containers() {
    let mut found = vec![];
    for container in [Container::Mp4, Container::Webm] {
        let Ok(settings) = EncoderSettings::new(container, (1920, 1080), 30., Default::default())
        else {
            continue;
        };
        if supported(VideoEncoder::is_config_supported(&config(&settings))).await {
            found.push(container);
        }
    }
    CONTAINERS.with(|containers| *containers.borrow_mut() = found);
}

thread_local!(static CONTAINERS: RefCell<Vec<Container>> = const { RefCell::new(vec![]) });

/// The containers `check_containers` found this browser can write.
pub fn containers() -> Vec<Container> {
    CONTAINERS.with(|containers| containers.borrow().clone())
}

fn config(settings: &EncoderSettings) -> js_sys::Object {
    let mut fields = vec![
        ("codec", JsValue::from_str(settings.codec)),
        ("width", settings.size.0.into()),
        ("height", settings.size.1.into()),
        ("bitrate", (settings.bitrate as f64).into()),
        ("framerate", settings.fps.into()),
        ("bitrateMode", "variable".into()),
        ("latencyMode", "quality".into()),
    ];
    if settings.codec.starts_with("avc1") {
        // Length-prefixed frames, with the parameter sets as the decoder configuration: as
        // MP4 stores them.
        fields.push(("avc", object(&[("format", "avc".into())]).into()));
    }
    object(&fields)
}

/// A WebCodecs video encoder, with the frames it has encoded so far.
pub struct Encoder {
    encoder: VideoEncoder,
    collected: Rc<Collected>,
    _callbacks: (Output, Callback, Callback),
    settings: EncoderSettings,
}

impl Encoder {
    pub async fn open(settings: &EncoderSettings) -> Result<Self> {
        let config = config(settings);
        ensure!(
            supported(VideoEncoder::is_config_supported(&config)).await,
            "This browser cannot encode {} at {}x{}",
            settings.codec,
            settings.size.0,
            settings.size.1
        );
        let collected = Rc::new(Collected::default());
        let (output, error) = collected.callbacks();
        let dequeue = {
            let collected = collected.clone();
            Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| collected.wake())
        };
        let encoder = VideoEncoder::new(&object(&[
            ("output", output.as_ref().clone()),
            ("error", error.as_ref().clone()),
        ]))
        .map_err(failed)?;
        encoder.set_ondequeue(dequeue.as_ref());
        encoder.configure(&config).map_err(failed)?;
        Ok(Self {
            encoder,
            collected,
            _callbacks: (output, error, dequeue),
            settings: settings.clone(),
        })
    }
}

impl VideoEncoding for Encoder {
    async fn encode(
        &mut self,
        frame: &RgbaImage,
        time: f64,
        duration: f64,
        key: bool,
    ) -> Result<()> {
        // A couple of frames queued keep the encoder busy without holding more in memory.
        while self.encoder.encode_queue_size() >= 2 {
            self.collected.check()?;
            self.collected.changed().await;
        }
        self.collected.check()?;
        let pixels = js_sys::Uint8Array::from(frame.as_raw().as_slice());
        let frame = VideoFrame::new(
            &pixels,
            &object(&[
                ("format", "RGBA".into()),
                ("codedWidth", frame.width().into()),
                ("codedHeight", frame.height().into()),
                ("timestamp", micros(time).into()),
                ("duration", micros(duration).into()),
            ]),
        )
        .map_err(failed)?;
        let encoded = self
            .encoder
            .encode(&frame, &object(&[("keyFrame", key.into())]));
        frame.close();
        encoded.map_err(failed)
    }

    async fn finish(self) -> Result<EncodedVideo> {
        let flushed = JsFuture::from(self.encoder.flush()).await;
        self.collected.check()?;
        flushed.map_err(failed)?;
        Ok(EncodedVideo {
            codec: self.settings.codec.into(),
            description: self.collected.description.borrow_mut().take(),
            size: self.settings.size,
            packets: std::mem::take(&mut *self.collected.packets.borrow_mut()),
        })
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        self.encoder.set_ondequeue(&JsValue::NULL);
        let _ = self.encoder.close();
    }
}

/// `audio` decoded and encoded again as Opus, which MP4 and WebM both hold.
pub async fn opus(audio: &EncodedAudio) -> Result<EncodedAudio> {
    let mut fields = vec![
        ("codec", JsValue::from_str(&audio.codec)),
        ("sampleRate", audio.sample_rate.into()),
        ("numberOfChannels", audio.channels.into()),
    ];
    if let Some(description) = &audio.description {
        fields.push((
            "description",
            js_sys::Uint8Array::from(&description[..]).into(),
        ));
    }
    let decoding = object(&fields);
    ensure!(
        supported(AudioDecoder::is_config_supported(&decoding)).await,
        "this browser cannot decode {}",
        audio.codec
    );
    let encoding = object(&[
        ("codec", "opus".into()),
        ("sampleRate", audio.sample_rate.into()),
        ("numberOfChannels", audio.channels.into()),
        (
            "bitrate",
            (64_000 * audio.channels.max(1)).min(256_000).into(),
        ),
    ]);
    ensure!(
        supported(AudioEncoder::is_config_supported(&encoding)).await,
        "this browser cannot encode Opus at {} Hz",
        audio.sample_rate
    );
    let collected = Rc::new(Collected::default());
    let (output, error) = collected.callbacks();
    let encoder = AudioEncoder::new(&object(&[
        ("output", output.as_ref().clone()),
        ("error", error.as_ref().clone()),
    ]))
    .map_err(failed)?;
    encoder.configure(&encoding).map_err(failed)?;
    let encoder = Rc::new(encoder);
    // Each piece of sound decoded goes straight on to the encoder.
    let decoded = {
        let (encoder, collected) = (encoder.clone(), collected.clone());
        Closure::<dyn FnMut(JsValue)>::new(move |data: JsValue| {
            let data: AudioData = data.unchecked_into();
            if let Err(error) = encoder.encode(&data) {
                *collected.error.borrow_mut() = Some(failed(error).to_string());
            }
            data.close();
        })
    };
    let decoder = AudioDecoder::new(&object(&[
        ("output", decoded.as_ref().clone()),
        ("error", error.as_ref().clone()),
    ]))
    .map_err(failed)?;
    decoder.configure(&decoding).map_err(failed)?;
    let result = async {
        for packet in &audio.packets {
            collected.check()?;
            if decoder.decode_queue_size() > 32 {
                // Let the decoder catch up before queueing more.
                crate::web::sleep(1).await;
            }
            let chunk = EncodedAudioChunk::new(&object(&[
                ("type", "key".into()),
                ("timestamp", micros(packet.time).into()),
                ("duration", micros(packet.duration).into()),
                ("data", js_sys::Uint8Array::from(&packet.data[..]).into()),
            ]))
            .map_err(failed)?;
            decoder.decode(&chunk).map_err(failed)?;
        }
        JsFuture::from(decoder.flush()).await.map_err(failed)?;
        JsFuture::from(encoder.flush()).await.map_err(failed)?;
        collected.check()
    }
    .await;
    let _ = decoder.close();
    let _ = encoder.close();
    result?;
    let packets = std::mem::take(&mut *collected.packets.borrow_mut());
    ensure!(!packets.is_empty(), "the browser produced no sound");
    let description = collected
        .description
        .borrow_mut()
        .take()
        .unwrap_or_else(|| {
            // A plain OpusHead: version 1, the channels, 312 samples of pre-skip and the rate.
            let mut head = b"OpusHead".to_vec();
            head.extend_from_slice(&[1, audio.channels as u8]);
            head.extend_from_slice(&312u16.to_le_bytes());
            head.extend_from_slice(&audio.sample_rate.to_le_bytes());
            head.extend_from_slice(&[0, 0, 0]);
            head
        });
    // The callbacks live until the coders are closed.
    drop((output, error, decoded));
    Ok(EncodedAudio {
        codec: "opus".into(),
        description: Some(description),
        // Opus always counts time at 48 kHz, whatever rate went in.
        sample_rate: 48_000,
        channels: audio.channels,
        packets,
    })
}
