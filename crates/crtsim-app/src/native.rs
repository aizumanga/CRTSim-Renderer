//! The desktop app in a native window: eframe on wgpu, sharing its device with the renderer.
use crate::{app_data, smoke::Smoke, worker, App};
use eframe::egui;
use std::{path::PathBuf, sync::Arc};

/// What the desktop app opens with, as its command line chose.
pub struct Launch {
    /// An image or video to open at startup.
    pub input: Option<PathBuf>,
    /// The graphics backend to use; `None` lets wgpu choose among the primary ones.
    pub backend: Option<wgpu::Backends>,
    /// A CI run that screenshots the window and quits, instead of a person's own session.
    pub smoke: Option<Smoke>,
}

/// Opens the app's window and runs it until it closes.
pub fn run(launch: Launch) -> eframe::Result<()> {
    let Launch {
        input,
        backend,
        smoke,
    } = launch;
    eframe::run_native(
        "CRTSim Renderer",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280., 850.])
                .with_min_inner_size([900., 620.]),
            // wgpu, so the interface draws on the same device the CRT frames are rendered on.
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(
                    eframe::egui_wgpu::WgpuSetupCreateNew {
                        // Honour --backend, which egui would otherwise pick for itself. Without
                        // one pinned, OpenGL stays available so the window still opens on a
                        // machine with no modern backend and can say so, as it could when the
                        // interface drew with GL; rendering there falls back to its own device,
                        // exactly as before.
                        instance_descriptor: wgpu::InstanceDescriptor {
                            backends: backend
                                .unwrap_or(wgpu::Backends::PRIMARY | wgpu::Backends::GL),
                            ..wgpu::InstanceDescriptor::new_without_display_handle()
                        },
                        // The window only needs a device big enough for the window; the
                        // renderer needs one big enough for a full-resolution export, so ask
                        // for the larger of the two. Not of a GL adapter, which cannot meet
                        // them -- asking would stop the window opening at all, which is the
                        // failure this fallback exists to avoid.
                        device_descriptor: std::sync::Arc::new(|adapter: &wgpu::Adapter| {
                            wgpu::DeviceDescriptor {
                                label: Some("CRTSim"),
                                required_features: wgpu::Features::empty(),
                                required_limits: if adapter.get_info().backend == wgpu::Backend::Gl
                                {
                                    wgpu::Limits::downlevel_webgl2_defaults()
                                        .using_resolution(adapter.limits())
                                } else {
                                    crtsim_core::Renderer::limits(adapter)
                                },
                                ..Default::default()
                            }
                        }),
                        ..eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle()
                    },
                ),
                ..Default::default()
            },
            ..Default::default()
        },
        Box::new(move |cc| {
            // Without a wgpu device -- an unsupported platform, or a backend that failed to
            // start -- the worker falls back to making its own, as it always did.
            let gpu = match cc.wgpu_render_state.as_ref() {
                // Sharing is only worth it on a device that can also do the rendering. A GL
                // fallback window keeps the interface alive; the renderer makes its own.
                Some(state) if state.adapter.get_info().backend != wgpu::Backend::Gl => {
                    tolerate_busy_reconfigure(&state.device);
                    worker::Gpu::Shared(state.clone())
                }
                _ => worker::Gpu::Own(backend.unwrap_or(wgpu::Backends::PRIMARY)),
            };
            if smoke.is_some() {
                // Keep galleries inside the main window so the smoke screenshot captures them,
                // and show windows at once rather than fading in, so it captures them whole.
                cc.egui_ctx.set_embed_viewports(true);
                cc.egui_ctx
                    .all_styles_mut(|style| style.animation_time = 0.);
            }
            Ok(Box::new(App::new(
                &cc.egui_ctx,
                gpu,
                app_data::Store::discover(),
                input,
                smoke,
            )))
        }),
    )
}

/// wgpu fails a window's surface reconfigure, on a resize say, when another thread submits work
/// while it waits for the device to go idle: here, the worker rendering on the window's device.
/// The surface keeps its old configuration and egui configures it again on the next frame, so
/// that one error is only logged. Every other error still panics, as wgpu's own handler does.
fn tolerate_busy_reconfigure(device: &wgpu::Device) {
    device.on_uncaptured_error(Arc::new(|error: wgpu::Error| {
        let text = error.to_string();
        if text.contains("before reconfiguring the Surface") {
            eprintln!("Window resize deferred while rendering: {text}");
            return;
        }
        panic!("wgpu error: {text}");
    }));
}
