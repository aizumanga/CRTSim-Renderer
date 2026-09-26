mod audition;
mod batch;
mod chrome;
mod dialogs;
mod events;
mod export_ui;
mod files;
mod gallery;
mod gallery_ui;
mod lut_gallery;
mod model;
mod playback;
mod preview_ui;
mod schedule;
mod settings_ui;
mod smoke;
mod theme;
mod thumbnails;
mod toolbar;
mod widgets;
mod worker;
mod workflow;

use crtsim_core::config::{self, ColorMode, Config, Filter, Fit, MaskRepeats, Phase};
use crtsim_core::{settings, RenderProgress};
use dialogs::Dialog;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use schedule::{Change, Schedule};
use smoke::Smoke;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use worker::{Event, Failure, Job, PreviewJob};

#[derive(Clone, Copy, PartialEq)]
enum View {
    Crt,
    Original,
    Compare,
}

/// What the work thread is doing for the interface: one load or export at a time.
enum Work {
    Idle,
    /// Opening an image, a frame of a video or a preset's metadata. Only an image cannot be
    /// cancelled.
    Loading(Option<Arc<AtomicBool>>),
    /// An export or a batch job, with the latest progress the worker has reported.
    Exporting {
        cancel: Arc<AtomicBool>,
        progress: Option<RenderProgress>,
    },
}

impl Work {
    fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }
    fn is_loading(&self) -> bool {
        matches!(self, Self::Loading(_))
    }
    fn is_exporting(&self) -> bool {
        matches!(self, Self::Exporting { .. })
    }
    /// The flag that stops the job, where it can be stopped.
    fn cancel(&self) -> Option<&Arc<AtomicBool>> {
        match self {
            Self::Idle => None,
            Self::Loading(cancel) => cancel.as_ref(),
            Self::Exporting { cancel, .. } => Some(cancel),
        }
    }
}

struct App {
    workflow: workflow::State,
    ui_context: egui::Context,
    video: Option<crtsim_media::Video>,
    video_frame: u64,
    selected_frame: u64,
    video_frames: u64,
    /// Where in the video the frame on screen is, in seconds; playing starts from here.
    play_time: f64,
    /// The video playing in the preview, if it is.
    playback: Option<playback::Playback>,
    video_options: crtsim_media::Options,
    animation_options: crtsim_media::AnimationOptions,
    work: Work,
    worker_thread: Option<std::thread::JoinHandle<()>>,
    store: Option<gallery::Store>,
    theme: theme::Theme,
    show_welcome: bool,
    show_credits: bool,
    show_gallery: bool,
    show_lut_gallery: bool,
    show_queue: bool,
    /// The batch export queue.
    queue: batch::Queue,
    /// The file chooser for new batch jobs, while it is open.
    batch_picking: Option<batch::Picking>,
    lut_gallery_search: String,
    gallery_entries: Vec<gallery::Entry>,
    /// A look previewed from a gallery without being applied; see `audition`.
    audition: Option<audition::Audition>,
    /// What a gallery pointed at this frame, for `settle_audition`.
    offered: Option<audition::Audition>,
    included_luts: std::collections::HashMap<usize, Arc<crtsim_core::workflow::Lut>>,
    thumbnails: thumbnails::Thumbnails,
    gallery_warnings: Vec<String>,
    gallery_name: String,
    description_edit: Option<(String, String)>,
    tool_windows: gallery::Layout,
    tool_windows_saved: gallery::Layout,
    config: Config,
    history: model::History,
    input: Arc<RgbaImage>,
    source_name: String,
    original: TextureHandle,
    rendered: Option<Displayed>,
    /// The interface's device, needed to register and release preview frames.
    render_state: Option<eframe::egui_wgpu::RenderState>,
    /// Whether the preview is out of date, and when to render the next one.
    schedule: Schedule,
    preview_limit: Option<u32>,
    view: View,
    zoom: f32,
    fit_preview: bool,
    dialog_open: bool,
    jobs: worker::Jobs,
    events: mpsc::Receiver<Event>,
    dialog_send: mpsc::Sender<(Dialog, Option<PathBuf>)>,
    dialog_receive: mpsc::Receiver<(Dialog, Option<PathBuf>)>,
    status: String,
    error: Option<String>,
    preview_error: Option<String>,
    smoke: Option<Smoke>,
}

/// The preview currently on screen. A frame rendered on the interface's own device is handed
/// to egui as it is; anything else is uploaded as an ordinary texture.
enum Displayed {
    Uploaded(TextureHandle),
    Frame {
        /// Held because egui samples it until the registration is freed.
        _texture: wgpu::Texture,
        id: egui::TextureId,
        size: egui::Vec2,
    },
}
impl Displayed {
    fn sized(&self) -> egui::load::SizedTexture {
        match self {
            Self::Uploaded(handle) => egui::load::SizedTexture::from_handle(handle),
            Self::Frame { id, size, .. } => egui::load::SizedTexture::new(*id, *size),
        }
    }
}

/// A path's file name for showing a person, which need not be valid Unicode.
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `image` uploaded for drawing, shrunk first where it is larger than `limit` on a side.
fn texture(ctx: &egui::Context, name: &str, image: &RgbaImage, limit: u32) -> TextureHandle {
    let shrunk;
    let image = if image.width().max(image.height()) > limit {
        shrunk = image::DynamicImage::ImageRgba8(image.clone())
            .thumbnail(limit, limit)
            .to_rgba8();
        &shrunk
    } else {
        image
    };
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}

impl App {
    fn new(
        ctx: &egui::Context,
        gpu: worker::Gpu,
        input_path: Option<PathBuf>,
        smoke: Option<Smoke>,
    ) -> Self {
        let input = Arc::new(config::test_card());
        let original = texture(ctx, "original", &input, 2048);
        let config = Config::general();
        let render_state = gpu.render_state().cloned();
        let (jobs, events, worker_thread) = worker::start(ctx.clone(), gpu);
        let (dialog_send, dialog_receive) = mpsc::channel();
        let (store, mut storage_error) = match gallery::Store::discover() {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("App data unavailable: {e:#}"))),
        };
        let theme = match store.as_ref().map(gallery::Store::theme) {
            Some(Ok(Some(theme))) => theme,
            Some(Ok(None)) | None => theme::Theme::default(),
            Some(Err(e)) => {
                storage_error = Some(format!(
                    "Could not load the saved theme: {e:#}. Using Sky Diary."
                ));
                theme::Theme::default()
            }
        };
        theme.apply(ctx);
        let tool_windows = match store.as_ref().map(gallery::Store::tool_windows) {
            Some(Ok(windows)) => windows,
            // Window placement is a convenience; opening at the default size is fine.
            Some(Err(_)) | None => gallery::Layout::new(),
        };
        let show_welcome = match &smoke {
            Some(smoke) => smoke.welcome,
            None => store.as_ref().is_none_or(gallery::Store::welcome_needed),
        };
        let (show_gallery, show_lut_gallery) = smoke
            .as_ref()
            .map_or((false, false), |smoke| (smoke.gallery, smoke.lut_gallery));
        let (gallery_entries, gallery_warnings) = gallery::entries(store.as_ref());
        let mut app = Self {
            workflow: workflow::State::default(),
            ui_context: ctx.clone(),
            video: None,
            video_frame: 0,
            selected_frame: 0,
            video_frames: 0,
            play_time: 0.,
            playback: None,
            video_options: crtsim_media::Options::default(),
            animation_options: Default::default(),
            work: Work::Idle,
            worker_thread: Some(worker_thread),
            store,
            theme,
            show_welcome,
            show_credits: false,
            show_gallery,
            show_lut_gallery,
            show_queue: false,
            queue: Default::default(),
            batch_picking: None,
            lut_gallery_search: String::new(),
            gallery_entries,
            audition: None,
            offered: None,
            included_luts: Default::default(),
            thumbnails: Default::default(),
            gallery_warnings,
            gallery_name: String::new(),
            description_edit: None,
            tool_windows_saved: tool_windows.clone(),
            tool_windows,
            history: model::History::new(config.clone()),
            config,
            input,
            source_name: "Built-in test card".into(),
            original,
            rendered: None,
            render_state,
            schedule: Schedule::new(Instant::now()),
            preview_limit: Some(1280),
            view: View::Crt,
            zoom: 1.,
            fit_preview: true,
            dialog_open: false,
            jobs,
            events,
            dialog_send,
            dialog_receive,
            status: "Preparing preview…".into(),
            error: storage_error,
            preview_error: None,
            smoke,
        };
        app.init_workflow(input_path.is_some());
        if let Some(path) = input_path {
            app.load(path);
        }
        app
    }

    /// Puts a preview on screen, releasing the registration of the one it replaces. egui keeps
    /// no ownership of a frame handed to it, so nothing else frees these.
    fn show_preview(&mut self, next: Option<Displayed>) {
        if let Some(Displayed::Frame { id, .. }) = self.rendered.take() {
            if let Some(state) = &self.render_state {
                state.renderer.write().free_texture(&id);
            }
        }
        self.rendered = next;
    }

    /// Prepares a finished preview for drawing. A frame is registered with egui; pixels are
    /// uploaded as before, which is what happens when the renderer is on its own device.
    fn displayed(&self, ctx: &egui::Context, preview: worker::Preview) -> Option<Displayed> {
        match (preview, self.render_state.as_ref()) {
            (worker::Preview::Pixels(image), _) => {
                let limit = ctx.input(|i| i.max_texture_side).min(u32::MAX as usize) as u32;
                Some(Displayed::Uploaded(texture(ctx, "crt", &image, limit)))
            }
            (worker::Preview::Frame(frame), Some(state)) => {
                let view = frame.texture.create_view(&Default::default());
                let id = state.renderer.write().register_native_texture(
                    &state.device,
                    &view,
                    wgpu::FilterMode::Linear,
                );
                Some(Displayed::Frame {
                    size: egui::vec2(frame.width as f32, frame.height as f32),
                    _texture: frame.texture,
                    id,
                })
            }
            // The worker only renders a frame when the interface has a device to draw it on,
            // so there is nowhere for this to come from.
            (worker::Preview::Frame(_), None) => None,
        }
    }

    fn changed(&mut self) {
        self.stop_playback();
        self.schedule.changed(Change::Edit, Instant::now());
    }
    fn replace_config(&mut self, config: Config) {
        self.history.commit(&self.config);
        self.config = config;
        self.history.commit(&self.config);
        self.changed();
    }
    fn undo(&mut self) {
        if let Some(c) = self.history.undo(&self.config) {
            self.config = c;
            self.changed();
        }
    }
    fn redo(&mut self) {
        if let Some(c) = self.history.redo(&self.config) {
            self.config = c;
            self.changed();
        }
    }
    /// A window that takes over the interface is open: a native file dialog, the video export
    /// settings or the welcome.
    fn modal_open(&self) -> bool {
        self.dialog_open || self.workflow.export_dialog.is_some() || self.show_welcome
    }
    /// Whether a file can be opened or an export started: nothing modal is open and the work
    /// thread is free.
    fn can_start_work(&self) -> bool {
        !self.modal_open() && self.work.is_idle()
    }
    /// Where settings, sessions and window placement are kept; none during a smoke run.
    fn app_data(&self) -> Option<&gallery::Store> {
        self.store.as_ref().filter(|_| self.smoke.is_none())
    }
    /// Makes `input` the image being edited: the file at `path`, a frame of `video`, or with
    /// neither the built-in test card. The original view shows `thumbnail`.
    fn set_source(
        &mut self,
        path: Option<PathBuf>,
        name: String,
        video: Option<crtsim_media::Video>,
        input: RgbaImage,
        thumbnail: &RgbaImage,
    ) {
        self.workflow.source = path;
        self.source_name = name;
        self.video = video;
        self.original = texture(&self.ui_context, "original", thumbnail, 2048);
        self.input = Arc::new(input);
        self.show_preview(None);
        self.changed();
    }
    fn show_test_card(&mut self) {
        let card = config::test_card();
        self.set_source(None, "Built-in test card".into(), None, card.clone(), &card);
    }
    fn load(&mut self, path: PathBuf) {
        self.stop_playback();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(workflow::PROJECT_EXTENSION))
        {
            self.open_project(path);
            return;
        }
        if crtsim_media::MediaKind::of(&path).is_moving() {
            self.load_video(path, 0, false);
            return;
        }
        self.work = Work::Loading(None);
        self.status = format!("Loading {}…", path.display());
        self.send(Job::Load(path));
    }
    fn load_video(&mut self, path: PathBuf, frame: u64, reuse: bool) {
        self.stop_playback();
        let cancel = Arc::new(AtomicBool::new(false));
        self.work = Work::Loading(Some(cancel.clone()));
        self.status = "Loading video frame…".into();
        self.send(Job::LoadVideo {
            path,
            frame,
            cached: if reuse {
                self.video.clone().map(|v| (v, self.video_frames))
            } else {
                None
            },
            cancel,
        });
    }
    fn send(&mut self, job: Job) {
        if self.jobs.send(job).is_err() {
            self.worker_stopped();
        }
    }
    fn send_preview(&mut self, job: PreviewJob) {
        if self.jobs.preview(job).is_err() {
            self.worker_stopped();
        }
    }
    fn worker_stopped(&mut self) {
        self.error =
            Some("Render worker stopped. Save your preset and restart the application.".into());
        self.work = Work::Idle;
        self.schedule.stop();
    }
    /// The image, or frame, on screen, exported as a PNG at full resolution.
    fn export(&mut self, path: PathBuf) {
        self.stop_playback();
        if let Err(e) = self.config.validate_for(self.input.dimensions()) {
            self.error = Some(format!("{e:#}"));
            return;
        }
        let status = format!(
            "Exporting {}… Settings are captured for this export.",
            path.display()
        );
        let export = worker::Export::Image {
            input: self.input.clone(),
            config: self.config.clone(),
        };
        self.start_export(export, path, status, Some("Queued for export"));
    }
    /// Hands `export` to the work thread, which saves it to `path`. It can be cancelled from the
    /// status bar, which shows the `queued` stage until the worker reports its own.
    fn start_export(
        &mut self,
        export: worker::Export,
        path: PathBuf,
        status: String,
        queued: Option<&str>,
    ) {
        let cancel = Arc::new(AtomicBool::new(false));
        self.work = Work::Exporting {
            cancel: cancel.clone(),
            progress: queued.map(|stage| RenderProgress {
                fraction: 0.,
                stage: stage.into(),
            }),
        };
        self.status = status;
        self.send(Job::Export {
            export,
            path,
            cancel,
        });
    }
    /// Also while exporting: previews have their own worker, and the export works from the
    /// settings it captured, so the ones on screen are free to change.
    fn request_preview(&mut self) {
        if self.work.is_loading() || self.playback.is_some() {
            return;
        }
        let Some(revision) = self.schedule.take() else {
            return;
        };
        self.history.commit(&self.config);
        match self
            .shown_config()
            .with_max_output_side(self.input.dimensions(), self.preview_limit)
        {
            Ok(config) => self.send_preview(PreviewJob::Preview {
                revision,
                input: self.input.clone(),
                config: Box::new(config),
            }),
            Err(e) => {
                // Nothing was sent, so nothing will come back.
                self.schedule.returned(revision);
                self.preview_error = Some(format!("Cannot preview: {e:#}"));
            }
        }
    }
    /// Remembered placement for one tool window. Copied out so the window's contents can
    /// still borrow `self`; write it back with `store_window_state` after the window runs.
    fn window_state(&self, title: &str) -> chrome::ToolWindow {
        self.tool_windows.get(title).copied().unwrap_or_default()
    }
    fn store_window_state(&mut self, title: &str, window: chrome::ToolWindow) {
        self.tool_windows.insert(title.to_owned(), window);
    }
    /// Writes the window layout only when it actually changed, so the two-second tick that
    /// calls this does not rewrite the file while nothing moves.
    pub(crate) fn save_tool_windows(&mut self) {
        let layout: gallery::Layout = self
            .tool_windows
            .iter()
            .map(|(title, window)| (title.clone(), window.to_save()))
            .collect();
        if layout == self.tool_windows_saved {
            return;
        }
        let Some(saved) = self.app_data().map(|store| store.set_tool_windows(&layout)) else {
            return;
        };
        match saved {
            Ok(()) => self.tool_windows_saved = layout,
            Err(e) => self.error = Some(format!("Window layout could not be saved: {e:#}")),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.workflow_ui(ctx);
        self.receive(ctx);
        if !self.modal_open() && !ctx.wants_keyboard_input() {
            let mut ctrl_shift = egui::Modifiers::CTRL;
            ctrl_shift.shift = true;
            let mut command_shift = egui::Modifiers::COMMAND;
            command_shift.shift = true;
            let redo = ctx.input_mut(|i| {
                i.consume_shortcut(&egui::KeyboardShortcut::new(ctrl_shift, egui::Key::Z))
                    || i.consume_shortcut(&egui::KeyboardShortcut::new(command_shift, egui::Key::Z))
            });
            let undo = !redo
                && ctx.input_mut(|i| {
                    i.consume_shortcut(&egui::KeyboardShortcut::new(
                        egui::Modifiers::CTRL,
                        egui::Key::Z,
                    )) || i.consume_shortcut(&egui::KeyboardShortcut::new(
                        egui::Modifiers::COMMAND,
                        egui::Key::Z,
                    ))
                });
            if redo {
                self.redo();
            } else if undo {
                self.undo();
            }
        }
        if self.can_start_work() {
            let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
            if let Some(path) = dropped {
                self.load(path);
            }
        }
        // A modal file dialog freezes edits so its eventual result uses the displayed settings.
        egui::TopBottomPanel::top("toolbar")
            .exact_height(42.)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                chrome::gradient(
                    ui.painter(),
                    rect,
                    ui.visuals().extreme_bg_color,
                    ui.visuals().panel_fill,
                );
                chrome::bevel(ui, rect);
                ui.add_enabled_ui(!self.show_welcome, |ui| self.toolbar(ui, ctx));
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                chrome::status_light(
                    ui,
                    self.schedule.rendering() || !self.work.is_idle(),
                    self.error.is_some() || self.preview_error.is_some(),
                );
                ui.label(&self.status);
            });
            if let Work::Exporting {
                progress: Some(p), ..
            } = &self.work
            {
                ui.add(
                    egui::ProgressBar::new(p.fraction)
                        .text(format!("Export: {} — {:.0}%", p.stage, p.fraction * 100.))
                        .animate(true),
                );
            }
            if let Some(cancel) = self.work.cancel().cloned() {
                if ui.button("Cancel").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                    self.status = "Cancelling…".into();
                }
            }
            for error in [self.error.clone(), self.preview_error.clone()]
                .into_iter()
                .flatten()
            {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.button("Dismiss").clicked() {
                        self.error = None;
                        self.preview_error = None;
                    }
                });
            }
        });
        egui::SidePanel::left("settings")
            .default_width(310.)
            .min_width(280.)
            .resizable(true)
            .show(ctx, |ui| {
                ui.add_enabled_ui(!self.modal_open(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(egui::Color32::from_rgb(16, 35, 55))
                    .inner_margin(12.)
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgb(75, 117, 155),
                    )),
            )
            .show(ctx, |ui| {
                chrome::preview_style(ui);
                ui.add_enabled_ui(!self.modal_open(), |ui| self.preview(ui));
            });
        self.gallery_window(ctx);
        self.lut_gallery_window(ctx);
        self.settle_audition();
        self.credits_window(ctx);
        let now = Instant::now();
        match self.schedule.due(now, ctx.input(|i| i.pointer.any_down())) {
            schedule::Due::Nothing => {}
            schedule::Due::Commit => self.history.commit(&self.config),
            schedule::Due::Preview => {
                self.history.commit(&self.config);
                if !self.show_welcome {
                    self.request_preview();
                }
            }
        }
        if self.schedule.needs_frames(now) || !self.work.is_idle() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.advance_smoke(ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop_playback();
        self.show_preview(None);
        self.save_session();
        self.save_tool_windows();
        if let Some(cancel) = self.work.cancel() {
            cancel.store(true, Ordering::Relaxed);
        }
        self.jobs.shutdown();
        if let Some(thread) = self.worker_thread.take() {
            let _ = thread.join();
        }
    }
}

fn main() -> eframe::Result<()> {
    let mut input = None;
    let mut smoke = None;
    let mut backends = wgpu::Backends::PRIMARY;
    let mut pinned_backend = false;
    let mut smoke_welcome = false;
    let mut smoke_gallery = false;
    let mut smoke_lut_gallery = false;
    let mut smoke_export = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke-welcome" => smoke_welcome = true,
            "--smoke-gallery" => smoke_gallery = true,
            "--smoke-lut-gallery" => smoke_lut_gallery = true,
            "--smoke-export" => smoke_export = true,
            "--backend" => {
                pinned_backend = true;
                backends = match args.next().as_deref() {
                    Some("vulkan") => wgpu::Backends::VULKAN,
                    Some("dx12") => wgpu::Backends::DX12,
                    Some("metal") => wgpu::Backends::METAL,
                    Some("auto") => {
                        pinned_backend = false;
                        wgpu::Backends::PRIMARY
                    }
                    _ => {
                        eprintln!("Expected --backend auto|vulkan|dx12|metal");
                        std::process::exit(2);
                    }
                }
            }
            "--smoke-test" => {
                smoke = Some(PathBuf::from(
                    args.next().expect("--smoke-test needs a new PNG path"),
                ))
            }
            "--help" | "-h" => {
                println!(
                    "crtsim-desktop [IMAGE_OR_VIDEO] [--backend auto|vulkan|dx12|metal]\n\
                     Open media, adjust effects, load/save presets and export PNG or video from \
                     the window.\n\
                     Video requires FFmpeg and ffprobe on PATH."
                );
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("Unknown option: {s}");
                std::process::exit(2);
            }
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("Only one image or video can be opened at startup");
                std::process::exit(2);
            }
        }
    }
    eframe::run_native(
        "CRTSim Renderer",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280., 850.])
                .with_min_inner_size([900., 620.]),
            // wgpu, so the interface draws on the same device the CRT frames are rendered on.
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                // Honour --backend, which egui would otherwise pick for itself. Without one
                // pinned, OpenGL stays available so the window still opens on a machine with
                // no modern backend and can say so, as it could when the interface drew with
                // GL; rendering there falls back to its own device, exactly as before.
                supported_backends: if pinned_backend {
                    backends
                } else {
                    backends | wgpu::Backends::GL
                },
                // The window only needs a device big enough for the window; the renderer needs
                // one big enough for a full-resolution export, so ask for the larger of the
                // two. Not of a GL adapter, which cannot meet them -- asking would stop the
                // window opening at all, which is the failure this fallback exists to avoid.
                device_descriptor: std::sync::Arc::new(|adapter: &wgpu::Adapter| {
                    wgpu::DeviceDescriptor {
                        label: Some("CRTSim"),
                        required_features: wgpu::Features::empty(),
                        required_limits: if adapter.get_info().backend == wgpu::Backend::Gl {
                            wgpu::Limits::downlevel_webgl2_defaults()
                                .using_resolution(adapter.limits())
                        } else {
                            crtsim_core::Renderer::limits(adapter)
                        },
                    }
                }),
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
                    worker::Gpu::Shared(state.clone())
                }
                _ => worker::Gpu::Own(backends),
            };
            let smoke = smoke.map(|screenshot| {
                let mut smoke = Smoke::new(screenshot);
                smoke.welcome = smoke_welcome;
                smoke.export = smoke_export;
                smoke.gallery = smoke_gallery;
                smoke.lut_gallery = smoke_lut_gallery;
                smoke
            });
            if smoke.is_some() {
                // Keep galleries inside the main window so the smoke screenshot captures them.
                cc.egui_ctx.set_embed_viewports(true);
            }
            Box::new(App::new(&cc.egui_ctx, gpu, input, smoke))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_preview_cannot_replace_current_settings() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (send, receive) = mpsc::channel();
        app.events = receive;
        let asked = app.schedule.take().unwrap();
        app.config.bloom = 0.;
        app.changed();
        send.send(Event::Preview {
            revision: asked,
            result: Ok(worker::Previewed {
                image: worker::Preview::Pixels(config::test_card()),
                seconds: 0.1,
            }),
        })
        .unwrap();
        app.receive(&ctx);
        assert!(app.rendered.is_none());
        assert!(!app.schedule.rendering());
        let settled = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            app.schedule.due(settled, false),
            schedule::Due::Preview,
            "the change is still to be previewed"
        );
        send.send(Event::Loaded(Err(Failure::Failed(
            "Cannot decode selected file".into(),
        ))))
        .unwrap();
        send.send(Event::Preview {
            revision: app.schedule.take().unwrap(),
            result: Ok(worker::Previewed {
                image: worker::Preview::Pixels(config::test_card()),
                seconds: 0.1,
            }),
        })
        .unwrap();
        app.receive(&ctx);
        assert!(app.schedule.is_current());
        assert_eq!(app.error.as_deref(), Some("Cannot decode selected file"));
    }

    #[test]
    fn settings_changed_during_an_export_are_previewed() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (jobs, work, previews) = worker::Jobs::capture();
        app.jobs = jobs;
        let dir = tempfile::tempdir().unwrap();
        app.export(dir.path().join("rendered.png"));
        assert!(matches!(work.try_recv(), Ok(Job::Export { .. })));
        app.config.bloom = 0.;
        app.changed();
        app.request_preview();
        match previews.try_recv() {
            Ok(PreviewJob::Preview { config, .. }) => assert_eq!(config.bloom, 0.),
            _ => panic!("expected a preview while exporting"),
        }
        assert!(app.work.is_exporting() && app.schedule.rendering());
    }

    #[test]
    fn export_captures_full_resolution_and_original_source() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (jobs, receive, _previews) = worker::Jobs::capture();
        app.jobs = jobs;
        let dir = tempfile::tempdir().unwrap();
        app.config.output = "4k".into();
        app.preview_limit = Some(800);
        let source = app.input.clone();
        let expected = app.config.clone();
        app.export(dir.path().join("rendered.png"));
        app.config.bloom = 0.;
        app.input = Arc::new(RgbaImage::new(1, 1));
        match receive.recv().unwrap() {
            Job::Export {
                export: worker::Export::Image { input, config },
                cancel,
                ..
            } => {
                assert!(Arc::ptr_eq(&input, &source));
                assert!(Arc::ptr_eq(&cancel, app.work.cancel().unwrap()));
                assert_eq!(config, expected);
                assert_eq!(
                    config.output_size(input.dimensions()).unwrap(),
                    (3840, 2160)
                );
            }
            _ => panic!("expected export job"),
        }
    }
}
