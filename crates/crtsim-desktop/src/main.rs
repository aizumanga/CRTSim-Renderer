mod audition;
mod chrome;
mod export_ui;
mod files;
mod gallery;
mod lut_gallery;
mod model;
mod theme;
mod thumbnails;
mod worker;
mod workflow;

use crtsim_core::config::{self, ColorMode, Config, Filter, Fit, Phase};
use crtsim_core::RenderProgress;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use worker::{Event, Job};

#[derive(Clone, Copy)]
enum Dialog {
    OpenProject,
    SaveProject,
    Lut,
    File,
    ExportVideo,
    ImportPreset,
    LoadPreset,
    SavePreset,
    Export,
}
#[derive(Clone, Copy, PartialEq)]
enum View {
    Crt,
    Original,
    Compare,
}

struct App {
    workflow: workflow::State,
    ui_context: egui::Context,
    video: Option<crtsim_media::Video>,
    video_frame: u64,
    selected_frame: u64,
    video_frames: u64,
    video_options: crtsim_media::Options,
    cancel: Arc<AtomicBool>,
    video_job: bool,
    worker_thread: Option<std::thread::JoinHandle<()>>,
    store: Option<gallery::Store>,
    theme: theme::Theme,
    show_welcome: bool,
    show_credits: bool,
    show_gallery: bool,
    show_lut_gallery: bool,
    lut_gallery_search: String,
    gallery_entries: Vec<gallery::Entry>,
    /// A look previewed from a gallery without being applied; see `audition`.
    audition: Option<audition::Audition>,
    /// What a gallery pointed at this frame, for `settle_audition`.
    offered: Option<audition::Audition>,
    /// An audition started or ended, so the preview must be redrawn even with live preview off.
    audition_pending: bool,
    included_luts: std::collections::HashMap<usize, Arc<crtsim_core::workflow::Lut>>,
    thumbnails: thumbnails::Thumbnails,
    gallery_warnings: Vec<String>,
    gallery_name: String,
    description_edit: Option<(String, String)>,
    tool_windows: gallery::Layout,
    tool_windows_saved: gallery::Layout,
    export_progress: Option<RenderProgress>,
    smoke_welcome: bool,
    smoke_gallery: bool,
    smoke_export: bool,
    config: Config,
    history: model::History,
    input: Arc<RgbaImage>,
    source_name: String,
    original: TextureHandle,
    rendered: Option<Displayed>,
    /// The interface's device, needed to register and release preview frames.
    render_state: Option<eframe::egui_wgpu::RenderState>,
    rendered_revision: Option<u64>,
    revision: u64,
    preview_limit: Option<u32>,
    view: View,
    zoom: f32,
    fit_preview: bool,
    live: bool,
    dirty: bool,
    changed_at: Instant,
    rendering: bool,
    loading: bool,
    exporting: bool,
    dialog_open: bool,
    jobs: worker::Jobs,
    events: mpsc::Receiver<Event>,
    dialog_send: mpsc::Sender<(Dialog, Option<PathBuf>)>,
    dialog_receive: mpsc::Receiver<(Dialog, Option<PathBuf>)>,
    status: String,
    error: Option<String>,
    preview_error: Option<String>,
    smoke: Option<PathBuf>,
    smoke_requested: bool,
    started: Instant,
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

fn texture(ctx: &egui::Context, name: &str, image: &RgbaImage, limit: u32) -> TextureHandle {
    let image = if image.width().max(image.height()) > limit {
        image::DynamicImage::ImageRgba8(image.clone())
            .thumbnail(limit, limit)
            .to_rgba8()
    } else {
        image.clone()
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
        smoke: Option<PathBuf>,
    ) -> Self {
        let input = Arc::new(config::test_card());
        let original = texture(ctx, "original", &input, 2048);
        let config = model::general();
        let render_state = gpu.render_state().cloned();
        let (jobs, events, worker_thread) = worker::start(ctx.clone(), gpu);
        let (dialog_send, dialog_receive) = mpsc::channel();
        let loading = false;
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
        let show_welcome = smoke.is_none() && store.as_ref().is_none_or(|s| s.welcome_needed());
        let mut gallery_entries = gallery::builtins();
        let mut gallery_warnings = vec![];
        if let Some(ref s) = store {
            match s.scan() {
                Ok((entries, warnings)) => {
                    gallery_entries.extend(entries);
                    gallery_warnings = warnings;
                }
                Err(e) => gallery_warnings.push(format!("{e:#}")),
            }
        }
        let mut app = Self {
            workflow: workflow::State::default(),
            ui_context: ctx.clone(),
            video: None,
            video_frame: 0,
            selected_frame: 0,
            video_frames: 0,
            video_options: crtsim_media::Options::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            video_job: false,
            worker_thread: Some(worker_thread),
            store,
            theme,
            show_welcome,
            show_credits: false,
            show_gallery: false,
            show_lut_gallery: false,
            lut_gallery_search: String::new(),
            gallery_entries,
            audition: None,
            offered: None,
            audition_pending: false,
            included_luts: Default::default(),
            thumbnails: Default::default(),
            gallery_warnings,
            gallery_name: String::new(),
            description_edit: None,
            tool_windows_saved: tool_windows.clone(),
            tool_windows,
            export_progress: None,
            smoke_welcome: false,
            smoke_gallery: false,
            smoke_export: false,
            history: model::History::new(config.clone()),
            config,
            input,
            source_name: "Built-in test card".into(),
            original,
            rendered: None,
            render_state,
            rendered_revision: None,
            revision: 0,
            preview_limit: Some(1280),
            view: View::Crt,
            zoom: 1.,
            fit_preview: true,
            live: true,
            dirty: true,
            changed_at: Instant::now(),
            rendering: false,
            loading,
            exporting: false,
            dialog_open: false,
            jobs,
            events,
            dialog_send,
            dialog_receive,
            status: "Preparing preview…".into(),
            error: storage_error,
            preview_error: None,
            smoke,
            smoke_requested: false,
            started: Instant::now(),
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
        Self::replace_preview(&mut self.rendered, self.render_state.as_ref(), next);
    }

    /// The same, reached through fields rather than through `self`, for callers that are
    /// already holding a borrow of another part of the application.
    fn replace_preview(
        rendered: &mut Option<Displayed>,
        state: Option<&eframe::egui_wgpu::RenderState>,
        next: Option<Displayed>,
    ) {
        if let Some(Displayed::Frame { id, .. }) = rendered.take() {
            if let Some(state) = state {
                state.renderer.write().free_texture(&id);
            }
        }
        *rendered = next;
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
        self.revision += 1;
        self.dirty = true;
        self.changed_at = Instant::now();
    }
    fn replace_config(&mut self, config: Config) {
        self.history.commit(&self.config);
        self.config = config;
        self.history.commit(&self.config);
        self.changed();
    }
    fn dialog(&mut self, kind: Dialog, ctx: &egui::Context) {
        self.stop_playback();
        self.dialog_open = true;
        let send = self.dialog_send.clone();
        let ctx = ctx.clone();
        let video_extension = self.workflow.export_format.extension();
        std::thread::spawn(move || {
            let path = match kind {
                Dialog::OpenProject => rfd::FileDialog::new()
                    .add_filter("CRT project", &["crtsim"])
                    .pick_file(),
                Dialog::SaveProject => rfd::FileDialog::new()
                    .add_filter("CRT project", &["crtsim"])
                    .set_file_name("project.crtsim")
                    .save_file(),
                Dialog::Lut => rfd::FileDialog::new()
                    .add_filter("3D color LUT", &["cube"])
                    .pick_file(),
                Dialog::File => rfd::FileDialog::new()
                    .add_filter(
                        "Images and videos",
                        &[
                            "png", "jpg", "jpeg", "webp", "bmp", "mp4", "mkv", "mov", "webm",
                            "avi", "m4v",
                        ],
                    )
                    .pick_file(),
                Dialog::ExportVideo => rfd::FileDialog::new()
                    .add_filter("Video", &[video_extension])
                    .set_file_name(format!("rendered.{video_extension}"))
                    .save_file(),
                Dialog::ImportPreset => rfd::FileDialog::new()
                    .add_filter(
                        "Rendered image/video",
                        &["png", "mp4", "mkv", "webm", "mov", "m4v", "avi"],
                    )
                    .pick_file(),
                Dialog::LoadPreset => rfd::FileDialog::new()
                    .add_filter("CRT preset", &["json"])
                    .pick_file(),
                Dialog::SavePreset => rfd::FileDialog::new()
                    .add_filter("CRT preset", &["json"])
                    .set_file_name("my-crt.json")
                    .save_file(),
                Dialog::Export => rfd::FileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .set_file_name("rendered.png")
                    .save_file(),
            };
            // Native dialogs confirm their selected path. If we append a missing suffix,
            // confirm the actual destination too, rather than silently replacing another file.
            let path = path.and_then(|mut path| {
                let extension = match kind {
                    Dialog::SaveProject => Some("crtsim"),
                    Dialog::Export => Some("png"),
                    Dialog::SavePreset => Some("json"),
                    Dialog::ExportVideo => Some(video_extension),
                    _ => None,
                };
                if let Some(extension) = extension {
                    if path.extension().is_none() {
                        path.set_extension(extension);
                        if path.exists()
                            && rfd::MessageDialog::new()
                                .set_title("Replace file?")
                                .set_description(format!("Replace {}?", path.display()))
                                .set_buttons(rfd::MessageButtons::YesNo)
                                .show()
                                != rfd::MessageDialogResult::Yes
                        {
                            return None;
                        }
                    }
                }
                Some(path)
            });
            let _ = send.send((kind, path));
            ctx.request_repaint();
        });
    }
    fn load(&mut self, path: PathBuf) {
        self.stop_playback();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("crtsim"))
        {
            self.open_project(path);
            return;
        }
        if path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            ["mp4", "mkv", "mov", "webm", "avi", "m4v"].contains(&e.to_ascii_lowercase().as_str())
        }) {
            self.load_video(path, 0, false);
            return;
        }
        self.loading = true;
        self.status = format!("Loading {}…", path.display());
        self.send(Job::Load(path));
    }
    fn load_video(&mut self, path: PathBuf, frame: u64, reuse: bool) {
        self.stop_playback();
        self.video_job = true;
        self.loading = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status = "Loading video frame…".into();
        self.send(Job::LoadVideo {
            path,
            frame,
            cached: if reuse {
                self.video.clone().map(|v| (v, self.video_frames))
            } else {
                None
            },
            cancel: self.cancel.clone(),
        });
    }
    fn send(&mut self, job: Job) {
        if self.jobs.send(job).is_err() {
            self.error =
                Some("Render worker stopped. Save your preset and restart the application.".into());
            self.rendering = false;
            self.loading = false;
            self.exporting = false;
            self.video_job = false;
            self.export_progress = None;
            self.dirty = false;
        }
    }
    fn export(&mut self, path: PathBuf) {
        self.stop_playback();
        if let Err(e) = model::preview_config(&self.config, self.input.dimensions(), None) {
            self.error = Some(format!("{e:#}"));
            return;
        }
        self.exporting = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.export_progress = Some(RenderProgress {
            fraction: 0.,
            stage: "Queued for export".into(),
        });
        self.status = format!(
            "Exporting {}… Settings are captured for this export.",
            path.display()
        );
        self.send(Job::Export {
            input: self.input.clone(),
            config: self.config.clone(),
            path,
            cancel: self.cancel.clone(),
        });
    }
    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((kind, path)) = self.dialog_receive.try_recv() {
            self.dialog_open = false;
            if let Some(mut path) = path {
                match kind {
                    Dialog::OpenProject => self.open_project(path),
                    Dialog::SaveProject => self.save_project_file(path),
                    Dialog::Lut => {
                        let result = (|| -> anyhow::Result<_> {
                            anyhow::ensure!(
                                path.metadata()?.len() <= 16 * 1024 * 1024,
                                "LUT exceeds 16 MB"
                            );
                            let name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            crtsim_core::workflow::Lut::parse_cube(
                                name,
                                &std::fs::read_to_string(path)?,
                            )
                        })();
                        match result {
                            Ok(lut) => {
                                let mut c = self.config.clone();
                                c.lut = Some(Arc::new(lut));
                                self.replace_config(c);
                                self.status = "LUT imported".into();
                            }
                            Err(e) => self.error = Some(format!("Cannot import LUT: {e:#}")),
                        }
                    }
                    Dialog::File => self.load(path),
                    Dialog::ExportVideo => {
                        if !path.extension().is_some_and(|e| {
                            e.eq_ignore_ascii_case(self.workflow.export_format.extension())
                        }) {
                            self.error = Some(format!(
                                "Choose a .{} filename for the selected format.",
                                self.workflow.export_format.extension()
                            ));
                            continue;
                        }
                        if let Some(video) = self.video.clone() {
                            self.exporting = true;
                            self.video_job = true;
                            self.cancel = Arc::new(AtomicBool::new(false));
                            self.export_progress = Some(RenderProgress {
                                fraction: 0.,
                                stage: "Queued for video export".into(),
                            });
                            self.status = "Exporting video…".into();
                            self.send(Job::ExportVideo {
                                video,
                                options: self.video_options.clone(),
                                config: self.config.clone(),
                                path,
                                cancel: self.cancel.clone(),
                            });
                        }
                    }
                    Dialog::ImportPreset => {
                        self.loading = true;
                        self.video_job = true;
                        self.cancel = Arc::new(AtomicBool::new(false));
                        self.status = "Reading preset metadata…".into();
                        self.send(Job::ImportPreset {
                            path,
                            input: self.input.dimensions(),
                            cancel: self.cancel.clone(),
                        });
                    }
                    Dialog::LoadPreset => {
                        match files::load_preset(&path, self.input.dimensions()) {
                            Ok(c) => {
                                self.gallery_name = path
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                self.replace_config(c);
                                self.status = format!("Loaded preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
                        }
                    }
                    Dialog::SavePreset => {
                        if path.extension().is_none() {
                            path.set_extension("json");
                        }
                        match model::preview_config(&self.config, self.input.dimensions(), None)
                            .and_then(|_| files::save_preset(&path, &self.config))
                        {
                            Ok(()) => {
                                self.status = format!("Saved preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot save preset: {e:#}")),
                        }
                    }
                    Dialog::Export => {
                        if path.extension().is_none() {
                            path.set_extension("png");
                        }
                        self.export(path);
                    }
                }
            }
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Progress { progress } if self.exporting => {
                    self.export_progress = Some(progress)
                }
                Event::Progress { .. } => {}
                Event::PresetImported(result) => {
                    self.loading = false;
                    self.video_job = false;
                    match result {
                        Ok((path, config, options)) => {
                            self.gallery_name = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            self.replace_config(config);
                            if let Some(options) = options {
                                self.video_options = options;
                            }
                            self.status = format!("Imported preset from {}", path.display());
                            self.error = None;
                        }
                        Err(_) if self.cancel.load(Ordering::Relaxed) => {
                            self.status = "Preset import cancelled".into();
                        }
                        Err(e) => self.error = Some(format!("Cannot import preset: {e}")),
                    }
                }
                Event::VideoLoaded(result) => {
                    self.video_job = false;
                    self.loading = false;
                    match result {
                        Ok((video, input, thumb, frame, count)) => {
                            self.workflow.source = Some(video.path.clone());
                            self.workflow.play_time = frame as f64 / video.fps;
                            self.source_name = video
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            self.video = Some(video);
                            self.video_frame = frame;
                            self.selected_frame = frame;
                            self.video_frames = count;
                            self.original = texture(ctx, "original", &thumb, 2048);
                            self.input = Arc::new(input);
                            self.show_preview(None);
                            self.rendered_revision = None;
                            self.error = None;
                            self.changed();
                            self.status = "Video frame loaded".into();
                            if let Some(p) = self.workflow.pending_project.take() {
                                self.apply_project(p);
                            }
                        }
                        Err(_) if self.cancel.load(Ordering::Relaxed) => {
                            self.status = "Video loading cancelled".into();
                            self.workflow.pending_project = None;
                            self.error = None;
                        }
                        Err(e) => {
                            self.workflow.pending_project = None;
                            self.error = Some(e);
                        }
                    }
                }
                Event::Loaded(result) => {
                    self.loading = false;
                    match result {
                        Ok((path, input, thumb)) => {
                            self.workflow.source =
                                Some(path.canonicalize().unwrap_or_else(|_| path.clone()));
                            self.video = None;
                            self.source_name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            self.original = texture(ctx, "original", &thumb, 2048);
                            self.input = Arc::new(input);
                            self.show_preview(None);
                            self.rendered_revision = None;
                            self.error = None;
                            self.changed();
                            self.status = "Image loaded".into();
                            if let Some(p) = self.workflow.pending_project.take() {
                                self.apply_project(p);
                            }
                        }
                        Err(e) => {
                            self.workflow.pending_project = None;
                            self.error = Some(e);
                        }
                    }
                }
                Event::Preview { revision, result } => {
                    self.rendering = false;
                    if revision != self.revision {
                        continue;
                    }
                    match result {
                        Ok((preview, seconds)) => {
                            let (width, height) = preview.dimensions();
                            let shown = self.displayed(ctx, preview);
                            self.show_preview(shown);
                            self.rendered_revision = Some(revision);
                            if !self.exporting {
                                self.status = format!("Preview {width} × {height} · {seconds:.2}s");
                            }
                            self.preview_error = None;
                        }
                        Err(e) => self.preview_error = Some(e),
                    }
                }
                Event::Thumbnail {
                    generation,
                    key,
                    result,
                } => self.thumbnail_ready(ctx, generation, key, result),
                Event::Exported(result) => {
                    self.queue_finished(&result);
                    self.video_job = false;
                    self.export_progress = None;
                    self.exporting = false;
                    match result {
                        Ok(path) => {
                            self.status = format!("Saved {}", path.display());
                            self.error = None;
                        }
                        Err(_) if self.cancel.load(Ordering::Relaxed) => {
                            self.status = "Export cancelled; destination kept unchanged".into();
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }
    /// Also while exporting: previews have their own worker, and the export works from the
    /// settings it captured, so the ones on screen are free to change.
    fn request_preview(&mut self) {
        if self.rendering || self.loading || self.workflow.playback.is_some() {
            return;
        }
        self.history.commit(&self.config);
        self.dirty = false;
        self.audition_pending = false;
        match model::preview_config(
            self.shown_config(),
            self.input.dimensions(),
            self.preview_limit,
        ) {
            Ok(config) => {
                self.rendering = true;
                self.send(Job::Preview {
                    revision: self.revision,
                    input: self.input.clone(),
                    config,
                });
            }
            Err(e) => self.preview_error = Some(format!("Cannot preview: {e:#}")),
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            chrome::monitor(ui);
            ui.label(egui::RichText::new("CRTSim Renderer").strong().size(17.));
            ui.separator();
            let enabled = !self.dialog_open
                && !self.loading
                && !self.exporting
                && self.workflow.export_dialog.is_none();
            ui.add_enabled_ui(enabled, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open media…").clicked() {
                        self.dialog(Dialog::File, ctx);
                        ui.close_menu();
                    }
                    if ui.button("Test card").clicked() {
                        self.video = None;
                        self.workflow.source = None;
                        self.input = Arc::new(config::test_card());
                        self.original = texture(ctx, "original", &self.input, 2048);
                        self.source_name = "Built-in test card".into();
                        self.show_preview(None);
                        self.changed();
                        ui.close_menu();
                    }
                    ui.separator();
                    self.project_menu(ui, ctx);
                });
                ui.menu_button("Presets", |ui| {
                    if ui.button("Preset gallery…").clicked() {
                        self.refresh_gallery();
                        self.show_gallery = true;
                        ui.close_menu();
                    }
                    for (label, kind) in [
                        ("Load preset…", Dialog::LoadPreset),
                        ("Save preset…", Dialog::SavePreset),
                        ("Import from image / video…", Dialog::ImportPreset),
                    ] {
                        if ui.button(label).clicked() {
                            self.dialog(kind, ctx);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.label("Interface theme");
                    let mut selected = self.theme;
                    for theme in theme::Theme::ALL {
                        ui.selectable_value(&mut selected, theme, theme.name())
                            .on_hover_text(theme.description());
                    }
                    if selected != self.theme {
                        self.theme = selected;
                        self.theme.apply(ctx);
                        if let Some(store) = &self.store {
                            if let Err(e) = store.set_theme(self.theme) {
                                self.error =
                                    Some(format!("Could not remember the selected theme: {e:#}"));
                            }
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("Export", |ui| {
                    if ui
                        .button(if self.video.is_some() {
                            "Current frame as PNG…"
                        } else {
                            "Image as PNG…"
                        })
                        .clicked()
                    {
                        self.dialog(Dialog::Export, ctx);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.video.is_some(), egui::Button::new("Video…"))
                        .clicked()
                    {
                        self.open_video_export(false);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Batch queue…").clicked() {
                        self.workflow.show_queue = true;
                        ui.close_menu();
                    }
                });
            });
            if ui.button("Credits").clicked() {
                self.show_credits = true;
            }
        });
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        if let Some(video) = &self.video {
            ui.heading("Video");
            ui.label(format!(
                "{:.2}s · {:.3} FPS · {}",
                video.duration,
                video.fps,
                if video.audio { "with audio" } else { "silent" }
            ));
            ui.small(
                "Preview is silent. Configure exported audio and compression in Export → Video.",
            );
            if video.hdr {
                ui.small("HDR source: FFmpeg tone-maps to SDR.");
            }
            ui.separator();
        }
        ui.horizontal(|ui| {
            chrome::monitor(ui);
            ui.heading("Source");
        });
        egui::Frame::none()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().window_stroke)
            .rounding(3.)
            .inner_margin(9.)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(egui::RichText::new(&self.source_name).strong());
            });
        ui.small(format!(
            "Source: {} × {}",
            self.input.width(),
            self.input.height()
        ));
        ui.horizontal(|ui| {
            if ui.button("Undo").on_hover_text("Ctrl+Z").clicked() {
                if let Some(c) = self.history.undo(&self.config) {
                    self.config = c;
                    self.changed();
                }
            }
            if ui.button("Redo").on_hover_text("Ctrl+Shift+Z").clicked() {
                if let Some(c) = self.history.redo(&self.config) {
                    self.config = c;
                    self.changed();
                }
            }
            if ui
                .button("Reset")
                .on_hover_text("Reset to the general image preset; Undo restores your settings")
                .clicked()
            {
                self.replace_config(model::general());
            }
        });
        ui.horizontal(|ui| {
            if ui.button("General image").clicked() {
                self.replace_config(model::general());
            }
            if ui.button("Original CRTSim").clicked() {
                self.replace_config(Config::default());
            }
        });
        let before = self.config.clone();
        // What each slider's reset returns to: the same baseline as Reset above.
        let defaults = model::general();
        chrome::Section::new("Image & output").show(ui,|ui| {
        resolution(
            ui,
            "Signal",
            &mut self.config.signal,
            &[
                "auto", "native", "original", "240p", "288p", "360p", "480p", "576p",
            ],
        );
        resolution(
            ui,
            "Export size",
            &mut self.config.output,
            &["720p", "1080p", "1440p", "4k", "reference", "match-input"],
        );
        egui::ComboBox::from_label("Fit on 4:3 tube")
            .selected_text(format!("{:?}", self.config.fit))
            .show_ui(ui, |ui| {
                for (value, name) in [
                    (Fit::Contain, "Contain"),
                    (Fit::Cover, "Cover (crop)"),
                    (Fit::Stretch, "Stretch"),
                    (Fit::Reference, "Reference"),
                ] {
                    ui.selectable_value(&mut self.config.fit, value, name);
                }
            });
        egui::ComboBox::from_label("Resize filter")
            .selected_text(format!("{:?}", self.config.filter))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.config.filter, Filter::Lanczos, "Lanczos (smooth)");
                ui.selectable_value(
                    &mut self.config.filter,
                    Filter::Nearest,
                    "Nearest (pixel art)",
                );
            });
        slider(ui, "Pixel aspect", &mut self.config.pixel_aspect, defaults.pixel_aspect, 0.1..=10.);
        if let (Ok(signal), Ok(output)) = (
            self.config.signal_size(self.input.dimensions()),
            self.config.output_size(self.input.dimensions()),
        ) {
            ui.small(format!(
                "Signal: {} × {} → Output: {} × {}",
                signal.0, signal.1, output.0, output.1
            ));
            if output.0 as u64 * output.1 as u64 > 8_300_000 {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Large output: more memory and rendering time.",
                );
            }
        } else {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Choose a preset or enter a valid WIDTHxHEIGHT.",
            );
        }
        if matches!(self.config.fit, Fit::Cover | Fit::Reference) || self.config.overscan > 1. {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Current fit/overscan can crop content and subtitles.",
            );
        }
        ui.small("Rounded glass may hide extreme corners even with Contain. Alpha uses the selected background. SDR output; no ICC color management.");

        });
        self.workflow_settings(ui);
        chrome::Section::new("Color processing").show(ui,|ui| {
        egui::ComboBox::from_label("Color processing")
            .selected_text(match self.config.color_mode {
                ColorMode::Reference => "Original gamma",
                ColorMode::LinearLight => "Linear light (experimental)",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    &mut self.config.color_mode,
                    ColorMode::Reference,
                    "Original gamma",
                );
                ui.selectable_value(
                    &mut self.config.color_mode,
                    ColorMode::LinearLight,
                    "Linear light (experimental)",
                );
            });
        if self.config.color_mode == ColorMode::LinearLight {
            ui.small("Linear-light glass, lighting and bloom; SDR output. The analog signal still uses the original gamma-space model.");
        }
        chrome::Section::new("Optional color grade").show(ui, |ui| {
            slider(ui, "Hue (degrees)", &mut self.config.hue, defaults.hue, -180.0..=180.);
            slider(ui, "Chroma", &mut self.config.chroma, defaults.chroma, 0.0..=2.);
            ui.small("YIQ hue/chroma adjustment. This is an optional grade, not the game's unpublished NES palette LUT or a complete NTSC decoder.");
        });
        ui.checkbox(&mut self.config.mask_antialias, "Filter mask when shrinking").on_hover_text("Mipmapped mask filtering reduces moiré during minification. Turn off for Phase 0/1 reference sampling.");

        });
        ui.separator();
        chrome::Section::new("CRT signal")
            .default_open(true)
            .show(ui, |ui| {
                slider(
                    ui,
                    "Saturation",
                    &mut self.config.saturation,
                    defaults.saturation,
                    0.0..=3.,
                );
                slider(
                    ui,
                    "Sharpness / ringing",
                    &mut self.config.sharpness,
                    defaults.sharpness,
                    0.0..=3.,
                );
                slider(
                    ui,
                    "Color bleed",
                    &mut self.config.bleed,
                    defaults.bleed,
                    0.0..=2.,
                );
                slider(
                    ui,
                    "Composite artifacts",
                    &mut self.config.artifacts,
                    defaults.artifacts,
                    0.0..=2.,
                );
            });
        chrome::Section::new("Glass & mask").show(ui, |ui| {
            slider(
                ui,
                "Barrel distortion",
                &mut self.config.barrel,
                defaults.barrel,
                -2.0..=2.,
            );
            slider(
                ui,
                "Overscan",
                &mut self.config.overscan,
                defaults.overscan,
                0.1..=3.,
            );
            slider(
                ui,
                "Mask opacity",
                &mut self.config.mask_opacity,
                defaults.mask_opacity,
                0.0..=1.,
            );
            slider(
                ui,
                "Mask brightness",
                &mut self.config.mask_brightness,
                defaults.mask_brightness,
                0.0..=2.,
            );
            slider(
                ui,
                "Mask columns",
                &mut self.config.mask_repeats[0],
                defaults.mask_repeats[0],
                1.0..=16384.,
            );
            slider(
                ui,
                "Mask rows",
                &mut self.config.mask_repeats[1],
                defaults.mask_repeats[1],
                1.0..=16384.,
            );
            slider(
                ui,
                "Edge dimming",
                &mut self.config.dimming,
                defaults.dimming,
                0.0..=1.,
            );
            slider(
                ui,
                "Camera field of view",
                &mut self.config.fov,
                defaults.fov,
                5.0..=90.,
            );
        });
        chrome::Section::new("Bloom & reflections").show(ui, |ui| {
            slider(
                ui,
                "Bloom amount",
                &mut self.config.bloom,
                defaults.bloom,
                0.0..=2.,
            );
            slider(
                ui,
                "Bloom power",
                &mut self.config.bloom_power,
                defaults.bloom_power,
                0.1..=8.,
            );
            slider(
                ui,
                "Bloom spread",
                &mut self.config.bloom_spread,
                defaults.bloom_spread,
                0.0..=0.2,
            );
            slider(
                ui,
                "Edge reflection",
                &mut self.config.reflection,
                defaults.reflection,
                0.0..=2.,
            );
        });
        chrome::Section::new("Frame & lighting").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Frame color");
                ui.color_edit_button_rgb(&mut self.config.frame_color);
            });
            slider(
                ui,
                "Diffuse light",
                &mut self.config.diffuse,
                defaults.diffuse,
                0.0..=2.,
            );
            slider(
                ui,
                "Specular light",
                &mut self.config.specular,
                defaults.specular,
                0.0..=2.,
            );
            slider(
                ui,
                "Specular power",
                &mut self.config.specular_power,
                defaults.specular_power,
                1.0..=200.,
            );
            slider(
                ui,
                "Rim light",
                &mut self.config.rim,
                defaults.rim,
                0.0..=2.,
            );
            for (i, name) in ["Light X", "Light Y", "Light Z"].iter().enumerate() {
                slider(
                    ui,
                    name,
                    &mut self.config.light_position[i],
                    defaults.light_position[i],
                    -1000.0..=1000.,
                );
            }
        });
        chrome::Section::new("Persistence & artifact phase").show(ui, |ui| {
            for (i,name) in ["Red persistence","Green persistence","Blue persistence"].iter().enumerate() { slider(ui, name, &mut self.config.persistence[i], defaults.persistence[i], 0.0..=0.999); }
            ui.horizontal(|ui| {
                if ui.add_enabled(self.config.warmup != defaults.warmup, egui::Button::new("↺").small()).on_hover_text(format!("Reset Warm-up ticks to {}", defaults.warmup)).clicked() { self.config.warmup = defaults.warmup; }
                ui.add(egui::Slider::new(&mut self.config.warmup,0..=240).text("Warm-up ticks"));
            });
            egui::ComboBox::from_label("Phase").selected_text(format!("{:?}",self.config.phase)).show_ui(ui,|ui| {
                for phase in [Phase::Stable,Phase::A,Phase::B,Phase::Alternating] { ui.selectable_value(&mut self.config.phase,phase,format!("{phase:?}")); }
            });
            ui.checkbox(&mut self.config.interlace, "Interlaced fields").on_hover_text("Each tick scans every other row, alternating fields; the rows it skips only fade by persistence. Use with a 480- or 576-row signal.");
            ui.small("Each still starts from black. Higher persistence may require more warm-up ticks. Alternating phase depends on tick count.");
        });
        if self.config != before {
            self.changed();
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.view, View::Original, "Original");
            ui.selectable_value(&mut self.view, View::Crt, "CRT");
            ui.selectable_value(&mut self.view, View::Compare, "Compare");
            ui.separator();
            ui.checkbox(&mut self.live, "Live preview");
            if ui
                .add_enabled(
                    !self.rendering && !self.loading,
                    egui::Button::new("Refresh"),
                )
                .clicked()
            {
                self.request_preview();
            }
        });
        ui.horizontal_wrapped(|ui| {
            let previous = self.preview_limit;
            egui::ComboBox::from_label("Preview quality")
                .selected_text(match self.preview_limit {
                    Some(800) => "Fast (800 px)",
                    Some(_) => "Balanced (1280 px)",
                    None => "Export resolution",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preview_limit, Some(800), "Fast (800 px)");
                    ui.selectable_value(&mut self.preview_limit, Some(1280), "Balanced (1280 px)");
                    ui.selectable_value(&mut self.preview_limit, None, "Export resolution");
                });
            if previous != self.preview_limit {
                self.changed();
            }
            ui.checkbox(&mut self.fit_preview, "Fit view");
            if !self.fit_preview {
                ui.add(egui::Slider::new(&mut self.zoom, 0.25..=4.).text("Zoom"));
            }
        });
        ui.small("Preview monitor").on_hover_text("Drag the divider in Compare. Preview quality never changes export resolution. Inspect mask detail at Export resolution and 1× zoom.");
        if let Ok(c) =
            model::preview_config(&self.config, self.input.dimensions(), self.preview_limit)
        {
            if let Ok((w, h)) = c.output_size(self.input.dimensions()) {
                if w as f32 / self.config.mask_repeats[0] < 6.
                    || h as f32 / self.config.mask_repeats[1] < 3.
                {
                    ui.colored_label(ui.visuals().warn_fg_color,"Dense mask at this preview size: aliasing / moiré is possible. Try a higher preview resolution or lower mask density.");
                }
            }
        }
        if self.rendering || self.loading || self.exporting {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if self.exporting {
                    "Exporting…"
                } else if self.loading {
                    "Loading file/frame…"
                } else {
                    "Rendering preview…"
                });
            });
        }
        if let Some(audition) = &self.audition {
            ui.colored_label(
                ui.visuals().selection.bg_fill,
                format!(
                    "Previewing {} — click it to apply, or move away to return to your settings",
                    audition.label
                ),
            );
        }
        if self.rendered.is_some() && self.rendered_revision != Some(self.revision) {
            ui.colored_label(ui.visuals().warn_fg_color, "Preview is out of date.");
        }
        let controls_height = if self.video.is_some() { 136. } else { 0. };
        let available = egui::vec2(
            ui.available_width(),
            (ui.available_height() - controls_height).max(1.),
        );
        egui::ScrollArea::both().max_height(available.y).auto_shrink([false,false]).show(ui,|ui| {
            if self.view == View::Compare {
                if let Some(ref im)=self.rendered {workflow::compare(ui,egui::load::SizedTexture::from_handle(&self.original),im.sized(),available,self.fit_preview,self.zoom,&mut self.workflow.comparison);}
            } else if self.view == View::Original { show_image(ui,egui::load::SizedTexture::from_handle(&self.original),available,self.fit_preview,self.zoom); }
            else if let Some(ref im) = self.rendered { show_image(ui,im.sized(),available,self.fit_preview,self.zoom); }
            else { ui.label("Open an image, video or test card. Your rendered preview will appear here."); }
        });
        self.video_controls(ui);
    }

    fn video_controls(&mut self, ui: &mut egui::Ui) {
        let Some(video) = self.video.clone() else {
            return;
        };
        let last = self.video_frames.saturating_sub(1);
        // A gallery in its own OS window has its own keyboard focus, so it no longer steals
        // the arrow keys below; only the embedded fallback shares this viewport's input.
        let galleries_overlap =
            ui.ctx().embed_viewports() && (self.show_gallery || self.show_lut_gallery);
        let enabled = !self.loading && !self.exporting && !self.dialog_open && !galleries_overlap;
        let mut seek = false;
        ui.separator();
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if self.workflow.playback.is_some() {
                        "Ⅱ Pause"
                    } else {
                        "▶ Play"
                    })
                    .clicked()
                {
                    if self.workflow.playback.is_some() {
                        self.stop_playback();
                    } else {
                        self.start_playback();
                    }
                }
                ui.small("Silent preview");
                ui.monospace(format!(
                    "{:02}:{:02} / {:02}:{:02}",
                    self.workflow.play_time as u64 / 60,
                    self.workflow.play_time as u64 % 60,
                    video.duration as u64 / 60,
                    video.duration as u64 % 60
                ));
                if ui
                    .add_enabled(self.video_frame > 0, egui::Button::new("|◀"))
                    .clicked()
                {
                    self.selected_frame = self.video_frame.saturating_sub(1);
                    seek = true;
                }
                if ui
                    .add_enabled(self.video_frame < last, egui::Button::new("▶|"))
                    .clicked()
                {
                    self.selected_frame = (self.video_frame + 1).min(last);
                    seek = true;
                }
                ui.label("Frame");
                let mut display_frame = self.selected_frame + 1;
                let number = ui.add(
                    egui::DragValue::new(&mut display_frame)
                        .clamp_range(1..=self.video_frames)
                        .speed(1),
                );
                self.selected_frame = display_frame.saturating_sub(1).min(last);
                seek |= number.drag_stopped()
                    || (number.lost_focus() && self.selected_frame != self.video_frame);
                ui.label(format!("/ {}", self.video_frames));
                if ui.button("Go").clicked() {
                    seek = true;
                }
            });
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 20.).max(100.);
                let response =
                    ui.add(egui::Slider::new(&mut self.selected_frame, 0..=last).show_value(false));
                seek |= response.drag_stopped()
                    || (response.changed() && !ui.input(|i| i.pointer.any_down()));
            });
            if !ui.ctx().wants_keyboard_input() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                    if self.workflow.playback.is_some() {
                        self.stop_playback();
                    } else {
                        self.start_playback();
                    }
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
                    self.selected_frame = self.video_frame.saturating_sub(1);
                    seek = true;
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
                    self.selected_frame = (self.video_frame + 1).min(last);
                    seek = true;
                }
            }
        });
        ui.small(format!("Showing frame {} · Left/Right arrow keys step frames · Export frame saves this settled CRT still as PNG.", self.video_frame + 1));
        if seek && enabled && self.selected_frame != self.video_frame {
            self.load_video(video.path, self.selected_frame, true);
        }
    }
}

impl App {
    fn refresh_gallery(&mut self) {
        self.gallery_entries = gallery::builtins();
        self.gallery_warnings.clear();
        if let Some(ref store) = self.store {
            match store.scan() {
                Ok((entries, warnings)) => {
                    self.gallery_entries.extend(entries);
                    self.gallery_warnings = warnings;
                }
                Err(e) => self.gallery_warnings.push(format!("{e:#}")),
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
        if self.smoke.is_some() {
            return;
        }
        let layout: gallery::Layout = self
            .tool_windows
            .iter()
            .map(|(title, window)| (title.clone(), window.to_save()))
            .collect();
        if layout == self.tool_windows_saved {
            return;
        }
        if let Some(store) = &self.store {
            match store.set_tool_windows(&layout) {
                Ok(()) => self.tool_windows_saved = layout,
                Err(e) => self.error = Some(format!("Window layout could not be saved: {e:#}")),
            }
        }
    }
    fn gallery_window(&mut self, ctx: &egui::Context) {
        if !self.show_gallery || self.show_welcome {
            return;
        }
        let mut selected = None;
        let mut hovered = None;
        let mut edit = None;
        let wanted: Vec<(String, Config)> = self
            .gallery_entries
            .iter()
            .map(|e| (e.name.clone(), e.config.clone()))
            .collect();
        let pictures: std::collections::HashMap<String, TextureHandle> = wanted
            .into_iter()
            .filter_map(|(name, config)| {
                let picture = self.preset_thumbnail(&name, &config)?;
                Some((name, picture))
            })
            .collect();
        let mut window = self.window_state("Preset gallery");
        let open = chrome::tool_window(ctx, "Preset gallery", [600., 500.], &mut window, |ui| {
            ui.add_enabled_ui(!self.dialog_open, |ui| {
                ui.label("Save your current settings here to find them again after restarting the app.");
                ui.horizontal(|ui| {
                    ui.label("Name"); ui.text_edit_singleline(&mut self.gallery_name);
                    if ui.add_enabled(self.store.is_some(), egui::Button::new("Save current")).clicked() {
                        let result = self.store.as_ref().unwrap().save(&self.gallery_name, &self.config, self.input.dimensions());
                        match result {
                            Ok(()) => { self.status = format!("Saved '{}' to My presets",self.gallery_name); self.error = None; self.refresh_gallery(); }
                            Err(e) => self.error = Some(format!("Cannot save gallery preset: {e:#}")),
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Load JSON…").clicked() { self.dialog(Dialog::LoadPreset,ctx); }
                    if ui.button("Refresh gallery").clicked() { self.refresh_gallery(); }
                });
                ui.small("Load an existing JSON, then choose Save current to add it to My presets. Existing names are never overwritten.");
                if let Some(ref store) = self.store { ui.small(format!("Personal presets: {}",store.root.join("presets").display())); }
                else { ui.colored_label(ui.visuals().warn_fg_color,"Personal storage is unavailable. JSON import/export and built-in presets still work."); }
                if let Some(ref error) = self.error { ui.colored_label(ui.visuals().error_fg_color,error); }
                if !self.gallery_warnings.is_empty() { egui::CollapsingHeader::new("Skipped preset files").show(ui,|ui| { for warning in &self.gallery_warnings { ui.label(warning); } }); }
                ui.separator();
                egui::ScrollArea::vertical().max_height(ui.available_height().max(200.)).show(ui, |ui| {
                    for user in [false,true] {
                        ui.heading(if user { "My presets" } else { "Included presets" });
                        let mut count = 0;
                        for entry in self.gallery_entries.iter().filter(|e| e.user == user) {
                            count += 1;
                            ui.group(|ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    let picture = thumbnails::show(ui, pictures.get(&entry.name), 72., 16. / 9.)
                                        .on_hover_text("Point to preview this preset on your image; click to apply it");
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            let label = ui.selectable_label(self.config == entry.config, &entry.name);
                                            if label.clicked() || picture.clicked() { selected = Some(entry.config.clone()); }
                                            if (label.hovered() || picture.hovered()) && ui.is_enabled() { hovered = Some((format!("preset “{}”", entry.name), entry.config.clone())); }
                                            if entry.user && ui.small_button("Edit description").clicked() {
                                                edit = Some((entry.name.clone(), entry.description.clone()));
                                            }
                                        });
                                        ui.add(egui::Label::new(if entry.description.is_empty() { "No description" } else { &entry.description }).wrap(true));
                                        if entry.config == self.config {
                                            ui.small("Matches your settings");
                                        } else {
                                            let changes = model::differences(&self.config, &entry.config);
                                            egui::CollapsingHeader::new(format!(
                                                "Differs from your settings in {} {}",
                                                changes.len(),
                                                if changes.len() == 1 { "setting" } else { "settings" }
                                            ))
                                            .id_source(("preset differences", &entry.name, entry.user))
                                            .show(ui, |ui| {
                                                egui::Grid::new(("preset difference grid", &entry.name, entry.user))
                                                    .striped(true)
                                                    .show(ui, |ui| {
                                                        ui.strong("Setting");
                                                        ui.strong("Yours");
                                                        ui.strong("Preset");
                                                        ui.end_row();
                                                        for change in &changes {
                                                            ui.label(&change.setting);
                                                            ui.label(&change.from);
                                                            ui.label(&change.to);
                                                            ui.end_row();
                                                        }
                                                    });
                                            });
                                        }
                                    });
                                });
                            });
                        }
                        if count == 0 { ui.label("No personal presets yet. Adjust an image and save your first look above."); }
                    }
                });
            });
            if let Some(edit) = edit.take() {
                self.description_edit = Some(edit);
            }
            // Shown inside the gallery's own window, next to the preset being edited.
            self.description_window(ui.ctx());
        });
        self.store_window_state("Preset gallery", window);
        self.show_gallery = open;
        if let Some((label, config)) = hovered.filter(|_| open) {
            self.offer_audition(label, config);
        }
        if let Some(config) = selected {
            self.replace_config(config);
        }
    }
    fn description_window(&mut self, ctx: &egui::Context) {
        if let Some((name, mut description)) = self.description_edit.clone() {
            let mut editing = true;
            let mut save = false;
            egui::Window::new(format!("Description — {name}"))
                .open(&mut editing)
                .collapsible(false)
                .default_width(400.)
                .constrain_to(ctx.screen_rect())
                .show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut description)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                    ui.small("Up to 4096 bytes. Leave blank to remove the description.");
                    save = ui.button("Save description").clicked();
                    if let Some(error) = &self.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
            if save {
                let result = self
                    .store
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Personal storage unavailable"))
                    .and_then(|store| store.set_description(&name, &description));
                match result {
                    Ok(()) => {
                        self.refresh_gallery();
                        self.error = None;
                        editing = false;
                    }
                    Err(e) => self.error = Some(format!("Cannot save description: {e:#}")),
                }
            }
            self.description_edit = editing.then_some((name, description));
        }
    }
    fn credit_text(ui: &mut egui::Ui) {
        ui.label(gallery::DISCLAIMER);
        ui.separator();
        ui.strong("Original CRTSim: J. Kyle Pittman");
        ui.label("The CRT simulation and original shaders, textures and screen/frame meshes are provided under CC0. Thank you for sharing them publicly.");
        ui.hyperlink_to(
            "Original CRTSim source",
            "https://github.com/MinorKeyGames/CRTSim",
        );
        ui.separator();
        ui.label(gallery::SUPPORT);
        ui.horizontal(|ui| {
            ui.hyperlink_to("J. Kyle Pittman on itch.io", gallery::ITCH);
            ui.hyperlink_to("Minor Key Games on Steam", gallery::STEAM);
        });
        ui.hyperlink_to(
            "Read: CRT Simulation in Super Win the Game",
            gallery::ARTICLE,
        );
        ui.separator();
        ui.strong("NES LUT collection: Wellington Uemura (wtuemura)");
        ui.label("Shared through MAME Goodies under CC0 1.0. Includes palettes by FirebrandX (FBX) and other creators identified in the original palette names. Thanks to the MAME Goodies contributors.");
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(
                "NES LUTs & license",
                "https://github.com/mamedev/mame-goodies/tree/master/bgfx/lut/nes",
            );
            ui.hyperlink_to(
                "FirebrandX palettes",
                "https://www.firebrandx.com/nespalette.html",
            );
            ui.hyperlink_to(
                "Author's announcement",
                "https://www.reddit.com/r/emulation/comments/1oopf1i/updated_nes_luts_for_mame/",
            );
        });
        ui.separator();
        ui.small("Renderer port and interface: CRTSim-Renderer contributors, with AI assistance. Built with Rust, wgpu, egui/eframe, image and other open-source libraries; see THIRD_PARTY_NOTICES.md in the repository.");
    }
    fn credits_window(&mut self, ctx: &egui::Context) {
        if self.show_welcome {
            egui::Window::new("Welcome to CRTSim Renderer").anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO)
                .collapsible(false).resizable(false).default_width(560.).show(ctx, |ui| {
                    Self::credit_text(ui);
                    ui.add_space(12.);
                    if ui.button("Got it — continue").clicked() {
                        if let Some(ref store) = self.store {
                            if let Err(e) = store.acknowledge() { self.error = Some(format!("Could not remember the welcome message: {e:#}. It may appear next time.")); }
                        }
                        self.show_welcome = false;
                    }
                });
        } else if self.show_credits {
            let mut window = self.window_state("Credits & support");
            // A native window clips instead of growing, so the long credits get a scroll area.
            let open =
                chrome::tool_window(ctx, "Credits & support", [560., 620.], &mut window, |ui| {
                    egui::ScrollArea::vertical().show(ui, Self::credit_text);
                });
            self.store_window_state("Credits & support", window);
            self.show_credits = open;
        }
    }
}

/// A setting's slider, with a button that returns it alone to `default`. The button keeps its
/// place while disabled, so the panel does not shift as values move on and off their defaults.
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    default: f32,
    range: std::ops::RangeInclusive<f32>,
) {
    let logarithmic = *range.start() >= 1. && *range.end() >= 200.;
    ui.horizontal(|ui| {
        if ui
            .add_enabled(*value != default, egui::Button::new("↺").small())
            .on_hover_text(format!("Reset {label} to {}", format_value(default)))
            .clicked()
        {
            *value = default;
        }
        ui.add(
            egui::Slider::new(value, range)
                .logarithmic(logarithmic)
                .clamp_to_range(false)
                .text(label),
        );
    });
}

/// A number as a person would write it: no trailing zeros, at most three decimals.
fn format_value(value: f32) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
fn resolution(ui: &mut egui::Ui, label: &str, value: &mut String, presets: &[&str]) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_source(label)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for preset in presets {
                    ui.selectable_value(value, (*preset).into(), *preset);
                }
            });
        ui.add(egui::TextEdit::singleline(value).desired_width(110.))
            .on_hover_text("Preset name or custom WIDTHxHEIGHT");
    });
}
fn show_image(
    ui: &mut egui::Ui,
    im: egui::load::SizedTexture,
    available: egui::Vec2,
    fit: bool,
    zoom: f32,
) {
    let size = im.size;
    let factor = if fit {
        (available.x / size.x).min(available.y / size.y).max(0.01)
    } else {
        zoom / ui.ctx().pixels_per_point()
    };
    let display = size * factor;
    let (area, _) =
        ui.allocate_exact_size(if fit { available } else { display }, egui::Sense::hover());
    let rect = egui::Rect::from_center_size(area.center(), display);
    ui.painter().image(
        im.id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
        egui::Color32::WHITE,
    );
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.workflow_ui(ctx);
        self.receive(ctx);
        if !self.dialog_open
            && self.workflow.export_dialog.is_none()
            && !self.show_welcome
            && !ctx.wants_keyboard_input()
        {
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
                if let Some(c) = self.history.redo(&self.config) {
                    self.config = c;
                    self.changed();
                }
            } else if undo {
                if let Some(c) = self.history.undo(&self.config) {
                    self.config = c;
                    self.changed();
                }
            }
        }
        if !self.loading
            && !self.dialog_open
            && self.workflow.export_dialog.is_none()
            && !self.show_welcome
            && !self.exporting
        {
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
                    self.rendering || self.loading || self.exporting,
                    self.error.is_some() || self.preview_error.is_some(),
                );
                ui.label(&self.status);
            });
            if let Some(p) = &self.export_progress {
                ui.add(
                    egui::ProgressBar::new(p.fraction)
                        .text(format!("Export: {} — {:.0}%", p.stage, p.fraction * 100.))
                        .animate(true),
                );
            }
            if (self.video_job || self.exporting) && ui.button("Cancel").clicked() {
                self.cancel.store(true, Ordering::Relaxed);
                self.status = "Cancelling…".into();
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
                ui.add_enabled_ui(
                    !self.dialog_open
                        && self.workflow.export_dialog.is_none()
                        && !self.show_welcome,
                    |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
                    },
                );
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
                ui.add_enabled_ui(
                    !self.show_welcome
                        && !self.dialog_open
                        && self.workflow.export_dialog.is_none(),
                    |ui| self.preview(ui),
                );
            });
        self.gallery_window(ctx);
        self.lut_gallery_window(ctx);
        self.settle_audition();
        self.credits_window(ctx);
        if self.dirty
            && self.changed_at.elapsed() >= Duration::from_millis(180)
            && !ctx.input(|i| i.pointer.any_down())
        {
            self.history.commit(&self.config);
            if (self.live || self.audition_pending) && !self.show_welcome {
                self.request_preview();
            }
        }
        if (self.dirty
            && (self.live
                || self.audition_pending
                || self.changed_at.elapsed() < Duration::from_millis(180)))
            || self.rendering
            || self.loading
            || self.exporting
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.smoke_export && self.rendered_revision == Some(self.revision) {
            self.smoke_export = false;
            self.open_video_export(false);
            ctx.request_repaint();
            return;
        }
        // Reproducible CI screenshot after an actual preview, without platform-specific mouse coordinates.
        if let Some(ref path) = self.smoke {
            if self.started.elapsed() > Duration::from_secs(120) {
                eprintln!(
                    "Desktop smoke test timed out: {:?}; {:?}",
                    self.error, self.preview_error
                );
                std::process::exit(1);
            }
            if (self.smoke_welcome
                || self.rendered_revision == Some(self.revision) && !self.thumbnails_pending())
                && !self.smoke_requested
            {
                self.smoke_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let im =
                        RgbaImage::from_raw(image.width() as u32, image.height() as u32, bytes)
                            .unwrap();
                    if let Err(e) = files::save_png(path, im, None) {
                        eprintln!("{e:#}");
                        std::process::exit(1);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop_playback();
        self.show_preview(None);
        self.save_session();
        self.save_tool_windows();
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.jobs.send(Job::Shutdown);
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
                println!("crtsim-desktop [IMAGE_OR_VIDEO] [--backend auto|vulkan|dx12|metal]\nOpen media, adjust effects, load/save presets and export PNG or video from the window.\nVideo requires FFmpeg and ffprobe on PATH.");
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
            let mut app = App::new(&cc.egui_ctx, gpu, input, smoke);
            if app.smoke.is_some() {
                app.smoke_welcome = smoke_welcome;
                app.smoke_gallery = smoke_gallery;
                app.smoke_export = smoke_export;
                app.show_welcome = smoke_welcome;
                app.show_gallery = smoke_gallery;
                app.show_lut_gallery = smoke_lut_gallery;
                // Keep galleries inside the main window so the smoke screenshot captures them.
                cc.egui_ctx.set_embed_viewports(true);
            }
            Box::new(app)
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
        app.rendering = true;
        app.config.bloom = 0.;
        app.changed();
        send.send(Event::Preview {
            revision: 0,
            result: Ok((worker::Preview::Pixels(config::test_card()), 0.1)),
        })
        .unwrap();
        app.receive(&ctx);
        assert!(app.rendered.is_none());
        assert!(!app.rendering);
        assert!(app.dirty);
        send.send(Event::Loaded(Err("Cannot decode selected file".into())))
            .unwrap();
        send.send(Event::Preview {
            revision: app.revision,
            result: Ok((worker::Preview::Pixels(config::test_card()), 0.1)),
        })
        .unwrap();
        app.receive(&ctx);
        assert_eq!(app.rendered_revision, Some(app.revision));
        assert_eq!(app.error.as_deref(), Some("Cannot decode selected file"));
    }

    #[test]
    fn reset_tooltips_show_values_as_written() {
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(50.), "50");
        assert_eq!(format_value(-0.115), "-0.115");
        assert_eq!(format_value(8. / 7.), "1.143");
    }

    #[test]
    fn settings_changed_during_an_export_are_previewed() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (send, receive) = mpsc::channel();
        app.jobs = worker::Jobs::capture(send);
        let dir = tempfile::tempdir().unwrap();
        app.export(dir.path().join("rendered.png"));
        assert!(matches!(receive.try_recv(), Ok(Job::Export { .. })));
        app.config.bloom = 0.;
        app.changed();
        app.request_preview();
        match receive.try_recv() {
            Ok(Job::Preview { config, .. }) => assert_eq!(config.bloom, 0.),
            _ => panic!("expected a preview while exporting"),
        }
        assert!(app.exporting && app.rendering);
    }

    #[test]
    fn export_captures_full_resolution_and_original_source() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (send, receive) = mpsc::channel();
        app.jobs = worker::Jobs::capture(send);
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
                input,
                config,
                cancel,
                ..
            } => {
                assert!(Arc::ptr_eq(&input, &source));
                assert!(Arc::ptr_eq(&cancel, &app.cancel));
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
