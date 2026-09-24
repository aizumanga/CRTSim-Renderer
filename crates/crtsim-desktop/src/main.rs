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

use crtsim_core::config::{self, ColorMode, Config, Filter, Fit, MaskRepeats, Phase};
use crtsim_core::{settings, RenderProgress};
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
use worker::{Event, Failure, Job, PreviewJob};

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

/// What a file dialog offers: the files it filters for and, when it saves, the name it suggests.
struct Chooser {
    filter: &'static str,
    extensions: Vec<&'static str>,
    /// For a save dialog, the suggested file name before its extension, the filter's only one.
    save_as: Option<&'static str>,
}

impl Dialog {
    fn chooser(self, video_extension: &'static str) -> Chooser {
        let (filter, extensions, save_as) = match self {
            Self::OpenProject => ("CRT project", vec![workflow::PROJECT_EXTENSION], None),
            Self::SaveProject => (
                "CRT project",
                vec![workflow::PROJECT_EXTENSION],
                Some("project"),
            ),
            Self::Lut => ("3D color LUT", vec!["cube"], None),
            Self::File => ("Images and videos", files::media_extensions(), None),
            Self::ExportVideo => ("Video", vec![video_extension], Some("rendered")),
            Self::ImportPreset => (
                "Rendered image/video",
                [&["png"], crtsim_media::VIDEO_EXTENSIONS].concat(),
                None,
            ),
            Self::LoadPreset => ("CRT preset", vec!["json"], None),
            Self::SavePreset => ("CRT preset", vec!["json"], Some("my-crt")),
            Self::Export => ("PNG image", vec!["png"], Some("rendered")),
        };
        Chooser {
            filter,
            extensions,
            save_as,
        }
    }
}

/// Native dialogs confirm the path they return. Where it lacks the extension and one is appended,
/// the actual destination is confirmed too, rather than silently replacing another file.
fn with_extension_confirmed(mut path: PathBuf, extension: &str) -> Option<PathBuf> {
    if path.extension().is_some() {
        return Some(path);
    }
    path.set_extension(extension);
    let replace = !path.exists()
        || rfd::MessageDialog::new()
            .set_title("Replace file?")
            .set_description(format!("Replace {}?", path.display()))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes;
    replace.then_some(path)
}
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

/// A CI run: open the window, wait for a rendered preview, save a screenshot and quit. It
/// never reads or writes the app data a person's own runs keep.
struct Smoke {
    screenshot: PathBuf,
    /// Screenshot the welcome as it first appears, not waiting for a preview.
    welcome: bool,
    /// Open the video export dialog once a preview is ready, and screenshot that.
    export: bool,
    requested: bool,
    started: Instant,
}

impl Smoke {
    fn new(screenshot: PathBuf) -> Self {
        Self {
            screenshot,
            welcome: false,
            export: false,
            requested: false,
            started: Instant::now(),
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
    video_options: crtsim_media::Options,
    work: Work,
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
            work: Work::Idle,
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
        self.rendered_revision = None;
        self.changed();
    }
    fn show_test_card(&mut self) {
        let card = config::test_card();
        self.set_source(None, "Built-in test card".into(), None, card.clone(), &card);
    }
    fn dialog(&mut self, kind: Dialog, ctx: &egui::Context) {
        self.stop_playback();
        self.dialog_open = true;
        let send = self.dialog_send.clone();
        let ctx = ctx.clone();
        let video_extension = self.workflow.export_container.extension();
        std::thread::spawn(move || {
            let chooser = kind.chooser(video_extension);
            let dialog = rfd::FileDialog::new().add_filter(chooser.filter, &chooser.extensions);
            let path = match chooser.save_as {
                None => dialog.pick_file(),
                Some(name) => {
                    let extension = chooser.extensions[0];
                    dialog
                        .set_file_name(format!("{name}.{extension}"))
                        .save_file()
                        .and_then(|path| with_extension_confirmed(path, extension))
                }
            };
            let _ = send.send((kind, path));
            ctx.request_repaint();
        });
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
        if crtsim_media::is_video(&path) {
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
        self.rendering = false;
        self.work = Work::Idle;
        self.dirty = false;
    }
    fn export(&mut self, path: PathBuf) {
        self.stop_playback();
        if let Err(e) = model::preview_config(&self.config, self.input.dimensions(), None) {
            self.error = Some(format!("{e:#}"));
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.work = Work::Exporting {
            cancel: cancel.clone(),
            progress: Some(RenderProgress {
                fraction: 0.,
                stage: "Queued for export".into(),
            }),
        };
        self.status = format!(
            "Exporting {}… Settings are captured for this export.",
            path.display()
        );
        self.send(Job::Export {
            input: self.input.clone(),
            config: self.config.clone(),
            path,
            cancel,
        });
    }
    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((kind, path)) = self.dialog_receive.try_recv() {
            self.dialog_open = false;
            if let Some(path) = path {
                match kind {
                    Dialog::OpenProject => self.open_project(path),
                    Dialog::SaveProject => self.save_project_file(path),
                    Dialog::Lut => {
                        let result = (|| -> anyhow::Result<_> {
                            anyhow::ensure!(
                                path.metadata()?.len() <= 16 * 1024 * 1024,
                                "LUT exceeds 16 MB"
                            );
                            crtsim_core::workflow::Lut::parse_cube(
                                file_name(&path),
                                &std::fs::read_to_string(&path)?,
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
                        let container = self.workflow.export_container;
                        if crtsim_media::Container::of(&path) != Some(container) {
                            self.error = Some(format!(
                                "Choose a .{} filename for the selected format.",
                                container.extension()
                            ));
                            continue;
                        }
                        if let Some(video) = self.video.clone() {
                            let cancel = Arc::new(AtomicBool::new(false));
                            self.work = Work::Exporting {
                                cancel: cancel.clone(),
                                progress: Some(RenderProgress {
                                    fraction: 0.,
                                    stage: "Queued for video export".into(),
                                }),
                            };
                            self.status = "Exporting video…".into();
                            self.send(Job::ExportVideo {
                                video,
                                options: self.video_options.clone(),
                                config: self.config.clone(),
                                path,
                                cancel,
                            });
                        }
                    }
                    Dialog::ImportPreset => {
                        let cancel = Arc::new(AtomicBool::new(false));
                        self.work = Work::Loading(Some(cancel.clone()));
                        self.status = "Reading preset metadata…".into();
                        self.send(Job::ImportPreset {
                            path,
                            input: self.input.dimensions(),
                            cancel,
                        });
                    }
                    Dialog::LoadPreset => {
                        match files::load_preset(&path, self.input.dimensions()) {
                            Ok(c) => {
                                self.gallery_name = file_stem(&path);
                                self.replace_config(c);
                                self.status = format!("Loaded preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
                        }
                    }
                    Dialog::SavePreset => {
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
                    Dialog::Export => self.export(path),
                }
            }
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Progress(progress) => {
                    if let Work::Exporting {
                        progress: shown, ..
                    } = &mut self.work
                    {
                        *shown = Some(progress);
                    }
                }
                Event::PresetImported(result) => {
                    self.work = Work::Idle;
                    match result {
                        Ok(imported) => {
                            self.gallery_name = file_stem(&imported.path);
                            self.replace_config(imported.config);
                            if let Some(options) = imported.options {
                                self.video_options = options;
                            }
                            self.status =
                                format!("Imported preset from {}", imported.path.display());
                            self.error = None;
                        }
                        Err(Failure::Cancelled) => {
                            self.status = "Preset import cancelled".into();
                        }
                        Err(Failure::Failed(e)) => {
                            self.error = Some(format!("Cannot import preset: {e}"))
                        }
                    }
                }
                Event::VideoLoaded(result) => {
                    self.work = Work::Idle;
                    match result {
                        Ok(loaded) => {
                            self.workflow.play_time = loaded.frame as f64 / loaded.video.fps;
                            self.video_frame = loaded.frame;
                            self.selected_frame = loaded.frame;
                            self.video_frames = loaded.frames;
                            let path = loaded.video.path.clone();
                            let name = file_name(&path);
                            self.set_source(
                                Some(path),
                                name,
                                Some(loaded.video),
                                loaded.image,
                                &loaded.thumbnail,
                            );
                            self.error = None;
                            self.status = "Video frame loaded".into();
                            if let Some(p) = self.workflow.pending_project.take() {
                                self.apply_project(p);
                            }
                        }
                        Err(Failure::Cancelled) => {
                            self.status = "Video loading cancelled".into();
                            self.workflow.pending_project = None;
                            self.error = None;
                        }
                        Err(Failure::Failed(e)) => {
                            self.workflow.pending_project = None;
                            self.error = Some(e);
                        }
                    }
                }
                Event::Loaded(result) => {
                    self.work = Work::Idle;
                    match result {
                        Ok(loaded) => {
                            let name = file_name(&loaded.path);
                            let source = loaded.path.canonicalize().unwrap_or(loaded.path);
                            self.set_source(
                                Some(source),
                                name,
                                None,
                                loaded.image,
                                &loaded.thumbnail,
                            );
                            self.error = None;
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
                        Ok(previewed) => {
                            let (width, height) = previewed.image.dimensions();
                            let shown = self.displayed(ctx, previewed.image);
                            self.show_preview(shown);
                            self.rendered_revision = Some(revision);
                            if !self.work.is_exporting() {
                                self.status = format!(
                                    "Preview {width} × {height} · {:.2}s",
                                    previewed.seconds
                                );
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
                    self.work = Work::Idle;
                    match result {
                        Ok(path) => {
                            self.status = format!("Saved {}", path.display());
                            self.error = None;
                        }
                        Err(Failure::Cancelled) => {
                            self.status = "Export cancelled; destination kept unchanged".into();
                            self.error = None;
                        }
                        Err(Failure::Failed(e)) => self.error = Some(e),
                    }
                }
            }
        }
    }
    /// Also while exporting: previews have their own worker, and the export works from the
    /// settings it captured, so the ones on screen are free to change.
    fn request_preview(&mut self) {
        if self.rendering || self.work.is_loading() || self.workflow.playback.is_some() {
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
                self.send_preview(PreviewJob::Preview {
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
            ui.add_enabled_ui(self.can_start_work(), |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open media…").clicked() {
                        self.dialog(Dialog::File, ctx);
                        ui.close_menu();
                    }
                    if ui.button("Test card").clicked() {
                        self.show_test_card();
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
                self.undo();
            }
            if ui.button("Redo").on_hover_text("Ctrl+Shift+Z").clicked() {
                self.redo();
            }
            if ui
                .button("Reset")
                .on_hover_text("Reset to the general image preset; Undo restores your settings")
                .clicked()
            {
                self.replace_config(Config::general());
            }
        });
        ui.horizontal(|ui| {
            if ui.button("General image").clicked() {
                self.replace_config(Config::general());
            }
            if ui.button("Original CRTSim").clicked() {
                self.replace_config(Config::default());
            }
        });
        let before = self.config.clone();
        // What each slider's reset returns to: the same baseline as Reset above.
        let defaults = Config::general();
        chrome::Section::new("Image & output").show(ui, |ui| {
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
                    ui.selectable_value(
                        &mut self.config.filter,
                        Filter::Lanczos,
                        "Lanczos (smooth)",
                    );
                    ui.selectable_value(
                        &mut self.config.filter,
                        Filter::Nearest,
                        "Nearest (pixel art)",
                    );
                });
            numbers(ui, &mut self.config, &defaults, settings::Section::Image);
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
            ui.small(
                "Rounded glass may hide extreme corners even with Contain. Alpha uses the \
                 selected background. SDR output; no ICC color management.",
            );
        });
        self.workflow_settings(ui);
        chrome::Section::new("Color processing").show(ui, |ui| {
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
                ui.small(
                    "Linear-light glass, lighting and bloom; SDR output. The analog signal \
                     still uses the original gamma-space model.",
                );
            }
            chrome::Section::new("Optional color grade").show(ui, |ui| {
                numbers(ui, &mut self.config, &defaults, settings::Section::Grade);
                ui.small(
                    "YIQ hue/chroma adjustment. This is an optional grade, not the game's \
                     unpublished NES palette LUT or a complete NTSC decoder.",
                );
            });
            ui.checkbox(
                &mut self.config.mask_antialias,
                "Filter mask when shrinking",
            )
            .on_hover_text(
                "Samples the mask from averaged, smaller copies of itself, as the original \
                 did, so it stays smooth where it is drawn smaller than it is. Off samples only \
                 the full-size mask, which can shimmer into moiré.",
            );
        });
        ui.separator();
        chrome::Section::new("CRT signal")
            .default_open(true)
            .show(ui, |ui| {
                numbers(ui, &mut self.config, &defaults, settings::Section::Signal)
            });
        chrome::Section::new("Glass & mask").show(ui, |ui| {
            numbers(ui, &mut self.config, &defaults, settings::Section::Glass);
            ui.separator();
            self.mask_density(ui);
            numbers(ui, &mut self.config, &defaults, settings::Section::Mask);
        });
        chrome::Section::new("Bloom & reflections").show(ui, |ui| {
            numbers(ui, &mut self.config, &defaults, settings::Section::Bloom)
        });
        chrome::Section::new("Frame & lighting").show(ui, |ui| {
            numbers(ui, &mut self.config, &defaults, settings::Section::Lighting)
        });
        chrome::Section::new("Persistence & artifact phase").show(ui, |ui| {
            numbers(
                ui,
                &mut self.config,
                &defaults,
                settings::Section::Persistence,
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.config.warmup != defaults.warmup,
                        egui::Button::new("↺").small(),
                    )
                    .on_hover_text(format!("Reset Warm-up ticks to {}", defaults.warmup))
                    .clicked()
                {
                    self.config.warmup = defaults.warmup;
                }
                ui.add(egui::Slider::new(&mut self.config.warmup, 0..=240).text("Warm-up ticks"));
            });
            egui::ComboBox::from_label("Phase")
                .selected_text(format!("{:?}", self.config.phase))
                .show_ui(ui, |ui| {
                    for phase in [Phase::Stable, Phase::A, Phase::B, Phase::Alternating] {
                        ui.selectable_value(&mut self.config.phase, phase, format!("{phase:?}"));
                    }
                });
            ui.checkbox(&mut self.config.interlace, "Interlaced fields")
                .on_hover_text(
                    "Each tick scans every other row, alternating fields; the rows it skips \
                     only fade by persistence. Use with a 480- or 576-row signal.",
                );
            ui.small(
                "Each still starts from black. Higher persistence may require more warm-up \
                 ticks. Alternating phase depends on tick count.",
            );
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
                    !self.rendering && !self.work.is_loading(),
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
        ui.small("Preview monitor").on_hover_text(
            "Drag the divider in Compare. Preview quality never changes \
            export resolution. Inspect mask detail at Export resolution \
            and 1× zoom.",
        );
        if let Ok(c) =
            model::preview_config(&self.config, self.input.dimensions(), self.preview_limit)
        {
            let input = self.input.dimensions();
            if let (Ok((w, h)), Ok(signal)) = (c.output_size(input), c.signal_size(input)) {
                let [columns, rows] = c.mask_repeats.resolve(signal);
                if w as f32 / columns < 6. || h as f32 / rows < 3. {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Dense mask at this preview size: aliasing / moiré is \
                        possible. Try a higher preview resolution or lower mask \
                        density.",
                    );
                }
            }
        }
        if self.rendering || !self.work.is_idle() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(match self.work {
                    Work::Exporting { .. } => "Exporting…",
                    Work::Loading(_) => "Loading file/frame…",
                    Work::Idle => "Rendering preview…",
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
        egui::ScrollArea::both()
            .max_height(available.y)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.view == View::Compare {
                    if let Some(ref im) = self.rendered {
                        workflow::compare(
                            ui,
                            egui::load::SizedTexture::from_handle(&self.original),
                            im.sized(),
                            available,
                            self.fit_preview,
                            self.zoom,
                            &mut self.workflow.comparison,
                        );
                    }
                } else if self.view == View::Original {
                    show_image(
                        ui,
                        egui::load::SizedTexture::from_handle(&self.original),
                        available,
                        self.fit_preview,
                        self.zoom,
                    );
                } else if let Some(ref im) = self.rendered {
                    show_image(ui, im.sized(), available, self.fit_preview, self.zoom);
                } else {
                    ui.label(
                        "Open an image, video or test card. Your rendered preview \
                will appear here.",
                    );
                }
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
        let enabled = self.can_start_work() && !galleries_overlap;
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
        ui.small(format!(
            "Showing frame {} · Left/Right arrow keys step frames · \
            Export frame saves this settled CRT still as PNG.",
            self.video_frame + 1
        ));
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
                ui.label(
                    "Save your current settings here to find them again after restarting the \
                     app.",
                );
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.gallery_name);
                    if ui
                        .add_enabled(self.store.is_some(), egui::Button::new("Save current"))
                        .clicked()
                    {
                        self.save_to_gallery();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Load JSON…").clicked() {
                        self.dialog(Dialog::LoadPreset, ctx);
                    }
                    if ui.button("Refresh gallery").clicked() {
                        self.refresh_gallery();
                    }
                });
                ui.small(
                    "Load an existing JSON, then choose Save current to add it to My presets. \
                     Existing names are never overwritten.",
                );
                match &self.store {
                    Some(store) => {
                        ui.small(format!(
                            "Personal presets: {}",
                            store.root.join("presets").display()
                        ));
                    }
                    None => {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "Personal storage is unavailable. JSON import/export and built-in \
                             presets still work.",
                        );
                    }
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if !self.gallery_warnings.is_empty() {
                    egui::CollapsingHeader::new("Skipped preset files").show(ui, |ui| {
                        for warning in &self.gallery_warnings {
                            ui.label(warning);
                        }
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height().max(200.))
                    .show(ui, |ui| {
                        for user in [false, true] {
                            ui.heading(if user {
                                "My presets"
                            } else {
                                "Included presets"
                            });
                            let mut count = 0;
                            for entry in self.gallery_entries.iter().filter(|e| e.user == user) {
                                count += 1;
                                let response = preset_entry(
                                    ui,
                                    entry,
                                    pictures.get(&entry.name),
                                    &self.config,
                                );
                                if response.selected {
                                    selected = Some(entry.config.clone());
                                }
                                if response.hovered {
                                    hovered = Some((
                                        format!("preset “{}”", entry.name),
                                        entry.config.clone(),
                                    ));
                                }
                                if response.edit {
                                    edit = Some((entry.name.clone(), entry.description.clone()));
                                }
                            }
                            if count == 0 {
                                ui.label(
                                    "No personal presets yet. Adjust an image and save your \
                                     first look above.",
                                );
                            }
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
    /// Whether the mask follows the signal, as the original's did, or has columns and rows of
    /// its own, which start from the density in use so the picture does not jump.
    fn mask_density(&mut self, ui: &mut egui::Ui) {
        let mut follows = self.config.mask_repeats == MaskRepeats::Signal;
        let toggled = ui
            .checkbox(&mut follows, "Mask follows the signal")
            .on_hover_text(
                "A mask column for every two signal columns and a row for every signal row, as \
                 the original drew it, so a finer signal gets a finer mask. Off sets the mask's \
                 columns and rows yourself, whatever the signal.",
            )
            .changed();
        if toggled {
            self.config.mask_repeats = if follows {
                MaskRepeats::Signal
            } else {
                let signal = self.config.signal_size(self.input.dimensions());
                MaskRepeats::Fixed(
                    self.config
                        .mask_repeats
                        .resolve(signal.unwrap_or((256, 224))),
                )
            };
        }
    }
    fn save_to_gallery(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        match store.save(&self.gallery_name, &self.config, self.input.dimensions()) {
            Ok(()) => {
                self.status = format!("Saved '{}' to My presets", self.gallery_name);
                self.error = None;
                self.refresh_gallery();
            }
            Err(e) => self.error = Some(format!("Cannot save gallery preset: {e:#}")),
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
        ui.label(
            "The CRT simulation and original shaders, textures and \
            screen/frame meshes are provided under CC0. Thank you for \
            sharing them publicly.",
        );
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
        ui.label(
            "Shared through MAME Goodies under CC0 1.0. Includes palettes \
            by FirebrandX (FBX) and other creators identified in the \
            original palette names. Thanks to the MAME Goodies \
            contributors.",
        );
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
        ui.small(
            "Renderer port and interface: CRTSim-Renderer contributors, \
            with AI assistance. Built with Rust, wgpu, egui/eframe, \
            image and other open-source libraries; see \
            THIRD_PARTY_NOTICES.md in the repository.",
        );
    }
    fn credits_window(&mut self, ctx: &egui::Context) {
        if self.show_welcome {
            egui::Window::new("Welcome to CRTSim Renderer")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .default_width(560.)
                .show(ctx, |ui| {
                    Self::credit_text(ui);
                    ui.add_space(12.);
                    if ui.button("Got it — continue").clicked() {
                        if let Some(ref store) = self.store {
                            if let Err(e) = store.acknowledge() {
                                self.error = Some(format!(
                                    "Could not remember the welcome message: {e:#}. It may appear \
                                next time."
                                ));
                            }
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

/// The numeric settings of one section of the panel, in the order `settings::SETTINGS` lists them.
fn numbers(ui: &mut egui::Ui, config: &mut Config, defaults: &Config, section: settings::Section) {
    for setting in settings::SETTINGS {
        let Some(numbers) = setting.numbers.as_ref().filter(|n| n.section == section) else {
            continue;
        };
        let values = (numbers.access.get_mut)(config);
        match &numbers.control {
            settings::Control::Slider { span, logarithmic } => {
                let defaults = (numbers.access.get)(defaults);
                for (index, value) in values.iter_mut().enumerate() {
                    // A default with no number here, such as a mask that follows the signal,
                    // leaves nothing to reset to.
                    let default = defaults.get(index).copied().unwrap_or(*value);
                    slider(
                        ui,
                        setting.value_name(index),
                        value,
                        default,
                        span.clone(),
                        *logarithmic,
                    );
                }
            }
            settings::Control::Color => {
                let rgb: &mut [f32; 3] = values.try_into().expect("a color is three values");
                ui.horizontal(|ui| {
                    ui.label(setting.label);
                    ui.color_edit_button_rgb(rgb);
                });
            }
        }
    }
}

/// What happened to one preset in the gallery this frame.
#[derive(Default)]
struct EntryResponse {
    selected: bool,
    hovered: bool,
    edit: bool,
}

/// One preset in the gallery: its thumbnail, name and description, and how it differs from the
/// settings in use.
fn preset_entry(
    ui: &mut egui::Ui,
    entry: &gallery::Entry,
    picture: Option<&TextureHandle>,
    current: &Config,
) -> EntryResponse {
    let mut response = EntryResponse::default();
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let picture = thumbnails::show(ui, picture, 72., 16. / 9.)
                .on_hover_text("Point to preview this preset on your image; click to apply it");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let label = ui.selectable_label(*current == entry.config, &entry.name);
                    response.selected = label.clicked() || picture.clicked();
                    response.hovered = (label.hovered() || picture.hovered()) && ui.is_enabled();
                    response.edit = entry.user && ui.small_button("Edit description").clicked();
                });
                let description = if entry.description.is_empty() {
                    "No description"
                } else {
                    &entry.description
                };
                ui.add(egui::Label::new(description).wrap(true));
                preset_differences(ui, entry, current);
            });
        });
    });
    response
}

/// How a preset differs from the settings in use, as a table that opens on request.
fn preset_differences(ui: &mut egui::Ui, entry: &gallery::Entry, current: &Config) {
    if entry.config == *current {
        ui.small("Matches your settings");
        return;
    }
    let changes = model::differences(current, &entry.config);
    let noun = if changes.len() == 1 {
        "setting"
    } else {
        "settings"
    };
    egui::CollapsingHeader::new(format!(
        "Differs from your settings in {} {noun}",
        changes.len()
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

/// A setting's slider, with a button that returns it alone to `default`. The button keeps its
/// place while disabled, so the panel does not shift as values move on and off their defaults.
fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    default: f32,
    range: std::ops::RangeInclusive<f32>,
    logarithmic: bool,
) {
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
                    self.rendering || !self.work.is_idle(),
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
            || !self.work.is_idle()
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.advance_smoke(ctx);
    }
}

impl App {
    /// Drives a smoke run: once the preview has rendered, opens the export dialog if asked to,
    /// then takes the screenshot, saves it and closes the window. A run that never gets there
    /// fails after two minutes rather than hanging CI.
    fn advance_smoke(&mut self, ctx: &egui::Context) {
        let ready = self.rendered_revision == Some(self.revision);
        if ready && self.smoke.as_ref().is_some_and(|smoke| smoke.export) {
            if let Some(smoke) = &mut self.smoke {
                smoke.export = false;
            }
            self.open_video_export(false);
            ctx.request_repaint();
            return;
        }
        let thumbnails_pending = self.thumbnails_pending();
        let (error, preview_error) = (&self.error, &self.preview_error);
        let Some(smoke) = &mut self.smoke else {
            return;
        };
        if smoke.started.elapsed() > Duration::from_secs(120) {
            eprintln!("Desktop smoke test timed out: {error:?}; {preview_error:?}");
            std::process::exit(1);
        }
        if (smoke.welcome || ready && !thumbnails_pending) && !smoke.requested {
            smoke.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let im = RgbaImage::from_raw(image.width() as u32, image.height() as u32, bytes)
                    .unwrap();
                if let Err(e) = files::save_png(&smoke.screenshot, im, None) {
                    eprintln!("{e:#}");
                    std::process::exit(1);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
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
            let smoke = smoke.map(|screenshot| Smoke {
                welcome: smoke_welcome,
                export: smoke_export,
                ..Smoke::new(screenshot)
            });
            let mut app = App::new(&cc.egui_ctx, gpu, input, smoke);
            if app.smoke.is_some() {
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
            result: Ok(worker::Previewed {
                image: worker::Preview::Pixels(config::test_card()),
                seconds: 0.1,
            }),
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
            result: Ok(worker::Previewed {
                image: worker::Preview::Pixels(config::test_card()),
                seconds: 0.1,
            }),
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
        assert!(app.work.is_exporting() && app.rendering);
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
                input,
                config,
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
