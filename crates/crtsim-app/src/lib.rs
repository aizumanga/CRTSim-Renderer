//! The CRTSim Renderer app: editing a look, the galleries, playback and exports, whichever
//! host it runs in.
// A browser build still compiles the desktop's folders, threads and native dialogs, which it
// never reaches.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod app_data;
mod audition;
mod batch;
mod chrome;
mod dialogs;
mod events;
mod export_ui;
mod ffmpeg_setup;
mod files;
mod frames;
mod gallery;
mod gallery_ui;
mod lut_gallery;
mod model;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
mod playback;
mod preview_ui;
mod project;
mod schedule;
mod session;
mod settings_ui;
mod smoke;
mod theme;
mod thumbnails;
mod timeline;
mod toolbar;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(target_arch = "wasm32")]
mod web_encode;
#[cfg(target_arch = "wasm32")]
mod web_video;
mod widgets;
mod worker;

use crtsim_core::config::{self, ColorMode, Config, Filter, Fit, MaskRepeats, Phase};
use crtsim_core::settings;
use dialogs::Dialog;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use schedule::{Change, Schedule};
pub use smoke::Smoke;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use web_time::Instant;
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
        progress: Option<crtsim_media::Progress>,
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
    /// The session saved to recover, and the project files opened in it.
    session: session::Session,
    ui_context: egui::Context,
    /// Where the editor is in the video open, if one is.
    timeline: Option<timeline::Timeline>,
    /// The video playing in the preview, if it is.
    playback: Option<playback::Playback>,
    video_options: crtsim_media::Options,
    animation_options: crtsim_media::AnimationOptions,
    work: Work,
    worker_thread: Option<std::thread::JoinHandle<()>>,
    store: Option<app_data::Store>,
    theme: theme::Theme,
    show_welcome: bool,
    show_credits: bool,
    show_queue: bool,
    /// The batch export queue.
    queue: batch::Queue,
    presets: gallery::PresetGallery,
    luts: lut_gallery::LutGallery,
    /// Whether FFmpeg is there for video work, and how to install it.
    ffmpeg: ffmpeg_setup::FfmpegSetup,
    /// A look previewed from a gallery without being applied; see `audition`.
    audition: Option<audition::Audition>,
    /// What a gallery pointed at this frame, for `settle_audition`.
    offered: Option<audition::Audition>,
    thumbnails: thumbnails::Thumbnails,
    tool_windows: app_data::Layout,
    tool_windows_saved: app_data::Layout,
    config: Config,
    history: model::History,
    input: Arc<RgbaImage>,
    source_name: String,
    /// Where the source is on disk; none for the test cards.
    source_path: Option<PathBuf>,
    original: TextureHandle,
    rendered: Option<Displayed>,
    /// The interface's device, needed to register and release preview frames.
    render_state: Option<eframe::egui_wgpu::RenderState>,
    /// Whether the preview is out of date, and when to render the next one.
    schedule: Schedule,
    preview_limit: Option<u32>,
    view: View,
    /// Where the Compare view divides the original from the CRT, from 0 to 1 across.
    comparison: f32,
    /// The video export window, while it is open.
    export_dialog: Option<export_ui::ExportDialog>,
    /// What the last video export was saved as, which the next one starts from.
    export_format: export_ui::ExportFormat,
    zoom: f32,
    fit_preview: bool,
    dialog_open: bool,
    jobs: worker::Jobs,
    events: mpsc::Receiver<Event>,
    dialog_send: mpsc::Sender<dialogs::Answer>,
    dialog_receive: mpsc::Receiver<dialogs::Answer>,
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

/// With Ctrl, or Command on macOS: open a file, save the project, export a PNG.
const SHORTCUT_OPEN: egui::Key = egui::Key::O;
const SHORTCUT_SAVE: egui::Key = egui::Key::S;
const SHORTCUT_EXPORT: egui::Key = egui::Key::E;

/// How `key` with Ctrl, or Command on macOS, is written on this computer, for a menu.
fn shortcut(ctx: &egui::Context, key: egui::Key) -> String {
    ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key))
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
    /// The app, remembering between runs in `store`: the person's own app data when run, a
    /// temporary folder in tests.
    fn new(
        ctx: &egui::Context,
        gpu: worker::Gpu,
        store: anyhow::Result<app_data::Store>,
        input_path: Option<PathBuf>,
        smoke: Option<Smoke>,
    ) -> Self {
        let input = Arc::new(config::test_card());
        let original = texture(ctx, "original", &input, 2048);
        let config = Config::general();
        let render_state = gpu.render_state().cloned();
        let worker::Worker {
            jobs,
            events,
            threads: worker_thread,
        } = worker::start(ctx.clone(), gpu, worker::Runtime::host());
        let (dialog_send, dialog_receive) = mpsc::channel();
        let (store, mut storage_error) = match store {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(format!("App data unavailable: {e:#}"))),
        };
        let theme = match store.as_ref().map(app_data::Store::theme) {
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
        let tool_windows = match store.as_ref().map(app_data::Store::tool_windows) {
            Some(Ok(windows)) => windows,
            // Window placement is a convenience; opening at the default size is fine.
            Some(Err(_)) | None => app_data::Layout::new(),
        };
        let show_welcome = match &smoke {
            Some(smoke) => smoke.welcome,
            None => store.as_ref().is_none_or(app_data::Store::welcome_needed),
        };
        let mut presets = gallery::PresetGallery::new(store.as_ref());
        let mut luts = lut_gallery::LutGallery::default();
        if let Some(smoke) = &smoke {
            (presets.open, luts.open) = (smoke.gallery, smoke.lut_gallery);
        }
        let mut app = Self {
            session: Default::default(),
            ui_context: ctx.clone(),
            timeline: None,
            playback: None,
            video_options: crtsim_media::Options::default(),
            animation_options: Default::default(),
            work: Work::Idle,
            worker_thread,
            store,
            theme,
            show_welcome,
            show_credits: false,
            show_queue: false,
            queue: Default::default(),
            presets,
            luts,
            ffmpeg: Default::default(),
            audition: None,
            offered: None,
            thumbnails: Default::default(),
            tool_windows_saved: tool_windows.clone(),
            tool_windows,
            history: model::History::new(config.clone()),
            config,
            input,
            source_name: "Built-in test card".into(),
            source_path: None,
            original,
            rendered: None,
            render_state,
            schedule: Schedule::new(Instant::now()),
            preview_limit: Some(1280),
            view: View::Crt,
            comparison: 0.5,
            export_dialog: None,
            export_format: Default::default(),
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
        app.ffmpeg.start_check(ctx);
        app.load_session(input_path.is_some());
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
        self.dialog_open || self.export_dialog.is_some() || self.show_welcome
    }
    /// Whether a file can be opened or an export started: nothing modal is open and the work
    /// thread is free.
    fn can_start_work(&self) -> bool {
        !self.modal_open() && self.work.is_idle()
    }
    /// Where settings, sessions and window placement are kept; none during a smoke run.
    fn app_data(&self) -> Option<&app_data::Store> {
        self.store.as_ref().filter(|_| self.smoke.is_none())
    }
    /// Makes `input` the image being edited: the file at `path`, a frame of a video at
    /// `timeline`, or with neither the built-in test card. The original view shows `thumbnail`.
    fn set_source(
        &mut self,
        path: Option<PathBuf>,
        name: String,
        timeline: Option<timeline::Timeline>,
        input: RgbaImage,
        thumbnail: &RgbaImage,
    ) {
        self.source_path = path;
        self.source_name = name;
        self.timeline = timeline;
        self.original = texture(&self.ui_context, "original", thumbnail, 2048);
        self.input = Arc::new(input);
        self.show_preview(None);
        self.changed();
    }
    fn show_test_card(&mut self) {
        let card = config::test_card();
        self.set_source(None, "Built-in test card".into(), None, card.clone(), &card);
    }
    /// Opens the video test card at `frame`. It is drawn rather than read, so it opens as an
    /// animation does, without a file or FFmpeg.
    fn show_video_test_card(&mut self, frame: u64) {
        let video = crtsim_media::test_clip();
        let frames = video.frames.unwrap_or(1);
        self.request_video(video.path.clone(), frame, Some((video, frames)));
    }
    fn load(&mut self, path: PathBuf) {
        self.stop_playback();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(project::EXTENSION))
        {
            self.open_project(path);
            return;
        }
        match crtsim_media::MediaKind::of(&path) {
            crtsim_media::MediaKind::Video if self.ffmpeg.missing() => {
                self.status = format!("Opening {} needs FFmpeg", file_name(&path));
                self.show_ffmpeg_setup();
                return;
            }
            kind if kind.is_moving() => {
                self.load_video(path, 0, false);
                return;
            }
            _ => {}
        }
        self.work = Work::Loading(None);
        self.status = format!("Loading {}…", path.display());
        self.send(Job::Load(path));
    }
    fn load_video(&mut self, path: PathBuf, frame: u64, reuse: bool) {
        let cached = self
            .timeline
            .as_ref()
            .filter(|_| reuse)
            .map(|t| (t.video.clone(), t.frames));
        self.request_video(path, frame, cached);
    }
    /// Loads `frame` of the video at `path`, or of `cached`, a video already probed, with its
    /// frame count.
    fn request_video(
        &mut self,
        path: PathBuf,
        frame: u64,
        cached: Option<(crtsim_media::Video, u64)>,
    ) {
        self.stop_playback();
        let cancel = Arc::new(AtomicBool::new(false));
        self.work = Work::Loading(Some(cancel.clone()));
        self.status = "Loading video frame…".into();
        self.send(Job::LoadVideo {
            path,
            frame,
            cached,
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
            progress: queued.map(|stage| crtsim_media::Progress {
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
    fn request_preview(&mut self, kind: schedule::Kind) {
        if self.work.is_loading() || self.playback.is_some() {
            return;
        }
        let Some(revision) = self.schedule.take(kind) else {
            return;
        };
        // An edit under way becomes an undo step once it settles, not at every frame of a drag.
        if kind == schedule::Kind::Settled {
            self.history.commit(&self.config);
        }
        match self
            .shown_config()
            .with_max_output_side(self.input.dimensions(), self.preview_limit)
        {
            Ok(config) => self.send_preview(PreviewJob::Preview {
                revision,
                kind,
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
    /// The keyboard shortcuts, with Ctrl or, on macOS, Command: Z undoes and Shift+Z redoes;
    /// O opens a file, S saves the project and E exports a PNG, when a file dialog could open.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let pressed = |shift: egui::Modifiers, key: egui::Key| {
            ctx.input_mut(|i| {
                [egui::Modifiers::CTRL, egui::Modifiers::COMMAND]
                    .into_iter()
                    .any(|command| {
                        i.consume_shortcut(&egui::KeyboardShortcut::new(command | shift, key))
                    })
            })
        };
        // Redo first: its shortcut would also match undo's.
        if pressed(egui::Modifiers::SHIFT, egui::Key::Z) {
            self.redo();
        } else if pressed(egui::Modifiers::NONE, egui::Key::Z) {
            self.undo();
        }
        if !self.can_start_work() {
            return;
        }
        if pressed(egui::Modifiers::NONE, SHORTCUT_OPEN) {
            self.dialog(Dialog::File, ctx);
        } else if pressed(egui::Modifiers::NONE, SHORTCUT_SAVE) {
            match self.session.project_path().cloned() {
                Some(path) => self.save_project_file(path),
                None => self.dialog(Dialog::SaveProject, ctx),
            }
        } else if pressed(egui::Modifiers::NONE, SHORTCUT_EXPORT) {
            self.dialog(Dialog::Export, ctx);
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
        let layout: app_data::Layout = self
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.video_export_window(ctx);
        self.tick_playback(ctx);
        self.recovery_window(ctx);
        self.queue_window(ctx);
        self.dispatch_queue();
        self.autosave();
        ctx.request_repaint_after(Duration::from_secs(2));
        self.receive(ctx);
        if !self.modal_open() && !ctx.egui_wants_keyboard_input() {
            self.shortcuts(ctx);
        }
        if self.can_start_work() {
            let dropped = ctx.input(|i| i.raw.dropped_files.first().cloned());
            // A browser gives a dropped file's name and bytes; the desktop, its path.
            #[cfg(target_arch = "wasm32")]
            if let Some(file) = dropped {
                self.dropped(file);
            }
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(path) = dropped
                .map(|f| f.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
            {
                self.load(path);
            }
        }
        // A modal file dialog freezes edits so its eventual result uses the displayed settings.
        egui::Panel::top("toolbar").exact_size(42.).show(ui, |ui| {
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
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                chrome::status_light(
                    ui,
                    self.schedule.rendering_settled() || !self.work.is_idle(),
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
        egui::Panel::left("settings")
            .default_size(310.)
            .min_size(280.)
            .resizable(true)
            .show(ui, |ui| {
                ui.add_enabled_ui(!self.modal_open(), |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(egui::Color32::from_rgb(16, 35, 55))
                    .inner_margin(12.)
                    .stroke(egui::Stroke::new(
                        1.0_f32,
                        egui::Color32::from_rgb(75, 117, 155),
                    )),
            )
            .show(ui, |ui| {
                chrome::preview_style(ui);
                ui.add_enabled_ui(!self.modal_open(), |ui| self.preview(ui));
            });
        self.gallery_window(ctx);
        self.lut_gallery_window(ctx);
        self.settle_audition();
        self.credits_window(ctx);
        self.ffmpeg_window(ctx);
        let now = Instant::now();
        match self.schedule.due(now, ctx.input(|i| i.pointer.any_down())) {
            schedule::Due::Nothing => {}
            schedule::Due::Commit => self.history.commit(&self.config),
            schedule::Due::Preview(kind) => {
                if kind == schedule::Kind::Settled {
                    self.history.commit(&self.config);
                }
                if !self.show_welcome {
                    self.request_preview(kind);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_preview_cannot_replace_current_settings() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        let (send, receive) = mpsc::channel();
        app.events = receive;
        let asked = app.schedule.take(schedule::Kind::Settled).unwrap();
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
            schedule::Due::Preview(schedule::Kind::Settled),
            "the change is still to be previewed"
        );
        send.send(Event::Loaded(Err(Failure::Failed(anyhow::anyhow!(
            "Cannot decode selected file"
        )))))
        .unwrap();
        send.send(Event::Preview {
            revision: app.schedule.take(schedule::Kind::Settled).unwrap(),
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
    fn a_drag_is_previewed_as_it_goes_and_becomes_one_undo_step() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        app.show_welcome = false;
        let (jobs, _work, previews) = worker::Jobs::capture();
        app.jobs = jobs;
        let (send, receive) = mpsc::channel();
        app.events = receive;
        let original = app.config.clone();
        // Each frame of the drag: the slider moves, and the frame's due preview is asked for.
        let drag = |app: &mut App, bloom| {
            app.config.bloom = bloom;
            app.changed();
            let due = app.schedule.due(Instant::now(), true);
            assert_eq!(due, schedule::Due::Preview(schedule::Kind::Interactive));
            app.request_preview(schedule::Kind::Interactive);
            match previews.try_recv() {
                Ok(PreviewJob::Preview {
                    revision,
                    kind: schedule::Kind::Interactive,
                    config,
                    ..
                }) => {
                    assert_eq!(config.bloom, bloom);
                    revision
                }
                _ => panic!("expected an interactive preview mid-drag"),
            }
        };
        let first = drag(&mut app, 0.5);
        // The slider moves on before the preview comes back; it is still shown.
        app.config.bloom = 0.25;
        app.changed();
        send.send(Event::Preview {
            revision: first,
            result: Ok(worker::Previewed {
                image: worker::Preview::Pixels(config::test_card()),
                seconds: 0.01,
            }),
        })
        .unwrap();
        app.receive(&ctx);
        assert!(app.rendered.is_some() && !app.schedule.is_current());
        assert!(
            !app.schedule.stale(),
            "the drag's next preview is on its way"
        );
        drag(&mut app, 0.);
        let settled = Instant::now() + Duration::from_secs(1);
        assert_eq!(
            app.schedule.due(settled, false),
            schedule::Due::Preview(schedule::Kind::Settled)
        );
        app.history.commit(&app.config);
        assert_eq!(
            app.history.undo(&app.config),
            Some(original),
            "the whole drag is one step"
        );
    }

    #[test]
    fn settings_changed_during_an_export_are_previewed() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        let (jobs, work, previews) = worker::Jobs::capture();
        app.jobs = jobs;
        let dir = tempfile::tempdir().unwrap();
        app.export(dir.path().join("rendered.png"));
        assert!(matches!(work.try_recv(), Ok(Job::Export { .. })));
        app.config.bloom = 0.;
        app.changed();
        app.request_preview(schedule::Kind::Settled);
        match previews.try_recv() {
            Ok(PreviewJob::Preview { config, .. }) => assert_eq!(config.bloom, 0.),
            _ => panic!("expected a preview while exporting"),
        }
        assert!(app.work.is_exporting() && app.schedule.rendering());
    }

    #[test]
    fn ctrl_s_saves_the_open_project() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            Some(Smoke::new("unused-smoke.png".into())),
        );
        app.show_welcome = false;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("look.crtsim");
        app.session.opened(path.clone(), None).unwrap();
        let ctrl_s = egui::Event::Key {
            key: SHORTCUT_SAVE,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        };
        let input = egui::RawInput {
            events: vec![ctrl_s],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| app.shortcuts(ui.ctx()))
            .textures_delta
            .clear();
        assert!(project::read(&path).is_ok(), "{:?}", app.error);
    }

    #[test]
    fn export_captures_full_resolution_and_original_source() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
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
