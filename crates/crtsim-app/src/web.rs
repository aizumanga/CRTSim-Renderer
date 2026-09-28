//! The web app: the whole app in a browser page, drawing and rendering on WebGPU. The page
//! (`web/index.html`) checks for WebGPU and then calls `start`.
use crate::{app_data, worker, App};
use anyhow::{anyhow, Context, Result};
use std::{cell::RefCell, collections::BTreeMap, sync::Arc};
use wasm_bindgen::{prelude::*, JsCast};
use wasm_bindgen_futures::JsFuture;

/// Starts the app on `canvas`, with the app data this browser keeps for the site.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement) -> Result<(), JsValue> {
    let store = app_data::Store::browser().await;
    // Which video formats the export window can offer.
    crate::web_encode::check_containers().await;
    let options = eframe::WebOptions {
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                eframe::egui_wgpu::WgpuSetupCreateNew {
                    // WebGPU only: see ADR 2 for why there is no WebGL2 fallback.
                    instance_descriptor: wgpu::InstanceDescriptor {
                        backends: wgpu::Backends::BROWSER_WEBGPU,
                        ..wgpu::InstanceDescriptor::new_without_display_handle()
                    },
                    // The interface and the renderer share the device, so ask for what a
                    // full-resolution export needs.
                    device_descriptor: Arc::new(|adapter: &wgpu::Adapter| wgpu::DeviceDescriptor {
                        label: Some("CRTSim"),
                        required_limits: crtsim_core::Renderer::limits(adapter),
                        ..Default::default()
                    }),
                    ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
                },
            ),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(move |cc| {
                let state = cc
                    .wgpu_render_state
                    .clone()
                    .ok_or("This browser did not give the page a WebGPU device")?;
                // What the GPU refuses goes to the browser's console, where a page's
                // problems are looked for.
                state
                    .device
                    .on_uncaptured_error(Arc::new(|error: wgpu::Error| {
                        web_sys::console::error_1(&format!("WebGPU: {error}").into());
                    }));
                // A lost device draws nothing more, so the page says so and offers a reload.
                state.device.set_device_lost_callback(|reason, message| {
                    web_sys::console::error_1(
                        &format!("WebGPU device lost ({reason:?}): {message}").into(),
                    );
                    if reason != wgpu::DeviceLostReason::Destroyed {
                        lost(&message);
                    }
                });
                Ok(Box::new(App::new(
                    &cc.egui_ctx,
                    worker::Gpu::Shared(state),
                    store,
                    None,
                    None,
                )))
            }),
        )
        .await
}

/// Tells the page the graphics device was lost, with the browser's `message`.
fn lost(message: &str) {
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&JsValue::from_str(message));
    let event = web_sys::CustomEvent::new_with_event_init_dict("crtsim-lost", &init);
    if let (Some(window), Ok(event)) = (web_sys::window(), event) {
        let _ = window.dispatch_event(&event);
    }
}

/// Saves `bytes` as a download called `name`: how a page saves a file.
pub fn download(name: &str, bytes: &[u8], mime: &str) -> Result<()> {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(failed)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(failed)?;
    let window = web_sys::window().context("No browser window")?;
    let link: web_sys::HtmlAnchorElement = window
        .document()
        .context("No page")?
        .create_element("a")
        .map_err(failed)?
        .unchecked_into();
    link.set_href(&url);
    link.set_download(name);
    link.click();
    // The download has started from the URL by the time this runs; then it can go.
    let forget = Closure::once_into_js(move || {
        let _ = web_sys::Url::revoke_object_url(&url);
    });
    window
        .set_timeout_with_callback_and_timeout_and_arguments_0(forget.unchecked_ref(), 60_000)
        .map_err(failed)?;
    Ok(())
}

/// Shows the browser's file picker for one file ending in one of `extensions`, and reads the
/// file picked: its name and contents, or nothing when the picker is cancelled. The browser
/// shows it only soon after a click or key press, which is how every picker here is asked for.
pub async fn pick(extensions: &[&str]) -> Result<Option<(String, Vec<u8>)>> {
    let document = web_sys::window()
        .and_then(|window| window.document())
        .context("No page")?;
    let body = document.body().context("No page body")?;
    let input: web_sys::HtmlInputElement = document
        .create_element("input")
        .map_err(failed)?
        .unchecked_into();
    input.set_type("file");
    input.set_hidden(true);
    let accept: Vec<String> = extensions.iter().map(|e| format!(".{e}")).collect();
    input.set_accept(&accept.join(","));
    let mut settle = None;
    let answered = js_sys::Promise::new(&mut |resolve, _| settle = Some(resolve));
    let settle = settle.context("The browser made no promise")?;
    // Every browser with WebGPU says when its picker is cancelled, so the wait always ends.
    let done = Closure::<dyn Fn()>::new(move || {
        let _ = settle.call0(&JsValue::NULL);
    });
    for event in ["change", "cancel"] {
        input
            .add_event_listener_with_callback(event, done.as_ref().unchecked_ref())
            .map_err(failed)?;
    }
    body.append_child(&input).map_err(failed)?;
    input.click();
    let _ = JsFuture::from(answered).await;
    input.remove();
    let Some(file) = input.files().and_then(|files| files.get(0)) else {
        return Ok(None);
    };
    let bytes = JsFuture::from(file.array_buffer())
        .await
        .map_err(|_| anyhow!("The browser could not read {}", file.name()))?;
    Ok(Some((
        file.name(),
        js_sys::Uint8Array::new(&bytes).to_vec(),
    )))
}

/// `image` as a lossy still WebP at `quality` (0–100), from the browser's own encoder. A
/// browser without one gives another format, which the caller refuses.
pub async fn lossy_webp(image: &image::RgbaImage, quality: u8) -> Result<Vec<u8>> {
    let (width, height) = image.dimensions();
    let canvas = web_sys::OffscreenCanvas::new(width, height).map_err(failed)?;
    let context: web_sys::OffscreenCanvasRenderingContext2d = canvas
        .get_context("2d")
        .map_err(failed)?
        .context("The browser gave no canvas to encode on")?
        .unchecked_into();
    let pixels = web_sys::ImageData::new_with_u8_clamped_array_and_sh(
        wasm_bindgen::Clamped(image.as_raw()),
        width,
        height,
    )
    .map_err(failed)?;
    context.put_image_data(&pixels, 0., 0.).map_err(failed)?;
    let options = web_sys::ImageEncodeOptions::new();
    options.set_type("image/webp");
    options.set_quality(f64::from(quality) / 100.);
    let blob: web_sys::Blob = JsFuture::from(
        canvas
            .convert_to_blob_with_options(&options)
            .map_err(failed)?,
    )
    .await
    .map_err(failed)?
    .unchecked_into();
    let bytes = JsFuture::from(blob.array_buffer()).await.map_err(failed)?;
    Ok(js_sys::Uint8Array::new(&bytes).to_vec())
}

/// Waits `milliseconds`, letting the page draw meanwhile.
pub async fn sleep(milliseconds: i32) {
    let waited = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            let _ = window
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, milliseconds);
        }
    });
    let _ = JsFuture::from(waited).await;
}

fn failed(error: JsValue) -> anyhow::Error {
    anyhow!("{error:?}")
}

/// The app's files in this browser's storage for the site. They are read whole when the app
/// starts, so the store can answer at once as a folder does, and each write goes to storage
/// straight away.
pub struct Files {
    entries: RefCell<BTreeMap<String, Vec<u8>>>,
    database: web_sys::IdbDatabase,
}

const DATABASE: &str = "crtsim-renderer";
const FILES: &str = "app-data";

impl Files {
    pub async fn open() -> Result<Self> {
        let factory = web_sys::window()
            .context("No browser window")?
            .indexed_db()
            .map_err(failed)?
            .context("This browser keeps no data for pages")?;
        let opening = factory.open_with_u32(DATABASE, 1).map_err(failed)?;
        let create = Closure::once_into_js(move |event: web_sys::IdbVersionChangeEvent| {
            let database = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::IdbOpenDbRequest>().ok())
                .and_then(|request| request.result().ok())
                .and_then(|result| result.dyn_into::<web_sys::IdbDatabase>().ok());
            if let Some(database) = database {
                let _ = database.create_object_store(FILES);
            }
        });
        opening.set_onupgradeneeded(Some(create.unchecked_ref()));
        let database: web_sys::IdbDatabase = settled(&opening).await?.unchecked_into();
        let files = database
            .transaction_with_str(FILES)
            .and_then(|transaction| transaction.object_store(FILES))
            .map_err(failed)?;
        let names = settled(&files.get_all_keys().map_err(failed)?).await?;
        let contents = settled(&files.get_all().map_err(failed)?).await?;
        let names: js_sys::Array = names.unchecked_into();
        let contents: js_sys::Array = contents.unchecked_into();
        let entries = names
            .iter()
            .zip(contents.iter())
            .filter_map(|(name, bytes)| {
                let bytes = bytes.dyn_into::<js_sys::Uint8Array>().ok()?;
                Some((name.as_string()?, bytes.to_vec()))
            })
            .collect();
        Ok(Self {
            entries: RefCell::new(entries),
            database,
        })
    }

    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        self.entries.borrow().get(name).cloned()
    }

    /// Keeps `bytes` as `name`, and puts them in storage.
    pub fn write(&self, name: &str, bytes: &[u8]) {
        self.entries
            .borrow_mut()
            .insert(name.to_owned(), bytes.to_vec());
        let stored = self
            .database
            .transaction_with_str_and_mode(FILES, web_sys::IdbTransactionMode::Readwrite)
            .and_then(|transaction| transaction.object_store(FILES))
            .and_then(|files| {
                files.put_with_key(&js_sys::Uint8Array::from(bytes), &JsValue::from_str(name))
            });
        if let Err(error) = stored {
            web_sys::console::warn_1(&error);
        }
    }

    /// The names under `prefix`, without it, sorted.
    pub fn names(&self, prefix: &str) -> Vec<String> {
        self.entries
            .borrow()
            .keys()
            .filter_map(|name| name.strip_prefix(prefix))
            .filter(|name| !name.contains('/'))
            .map(str::to_owned)
            .collect()
    }
}

/// The result of `request`, once it has one.
async fn settled(request: &web_sys::IdbRequest) -> Result<JsValue> {
    let done = js_sys::Promise::new(&mut |resolve, reject| {
        let success = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        let failure = Closure::once_into_js(move || {
            let _ = reject.call0(&JsValue::NULL);
        });
        request.set_onsuccess(Some(success.unchecked_ref()));
        request.set_onerror(Some(failure.unchecked_ref()));
    });
    JsFuture::from(done)
        .await
        .map_err(|_| anyhow!("The browser's storage refused a request"))?;
    request.result().map_err(failed)
}
