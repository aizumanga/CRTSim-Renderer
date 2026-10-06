//! The CRTSim Renderer app: editing a look, the galleries, playback and exports, whichever
//! host it runs in.
// A browser build still compiles the desktop's folders, threads and native dialogs, which it
// never reaches.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod app_data;
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
mod incoming;
mod lane;
mod lut_gallery;
mod model;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
mod playback;
mod preview;
mod preview_ui;
mod project;
mod session;
mod settings_ui;
mod smoke;
mod source;
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

use crtsim_core::config::{ColorMode, Config, Filter, Fit, MaskRepeats, Phase};
use crtsim_core::settings;
use dialogs::Dialog;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
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

struct App {
    /// The session saved to recover, and the project files opened in it.
    session: session::Session,
    ui_context: egui::Context,
    /// The video playing in the preview, if it is.
    playback: Option<playback::Playback>,
    video_options: crtsim_media::Options,
    animation_options: crtsim_media::AnimationOptions,
    /// The job the worker's work lane is doing, if any.
    lane: lane::Lane,
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
    thumbnails: thumbnails::Thumbnails,
    tool_windows: app_data::Layout,
    tool_windows_saved: app_data::Layout,
    config: Config,
    history: model::History,
    /// What is open, and the picture it gives.
    source: source::Source,
    /// The CRT picture on screen, and when to render the next one.
    preview: preview::Preview,
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
    smoke: Option<Smoke>,
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
        let config = gallery::default_preset().config;
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
            playback: None,
            video_options: crtsim_media::Options::default(),
            animation_options: Default::default(),
            lane: lane::Lane::new(jobs.work_lane()),
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
            thumbnails: Default::default(),
            tool_windows_saved: tool_windows.clone(),
            tool_windows,
            history: model::History::new(config.clone()),
            config,
            source: source::Source::new(ctx),
            preview: preview::Preview::new(jobs.preview_lane(), render_state, Instant::now()),
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
            smoke,
        };
        app.ffmpeg.start_check(ctx);
        app.load_session(input_path.is_some());
        if let Some(path) = input_path {
            app.load(path);
        }
        app
    }

    fn changed(&mut self) {
        self.retune_playback();
        self.preview.changed(Instant::now());
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
        !self.modal_open() && self.lane.may_start()
    }
    /// Where settings, sessions and window placement are kept; none during a smoke run.
    fn app_data(&self) -> Option<&app_data::Store> {
        self.store.as_ref().filter(|_| self.smoke.is_none())
    }
    /// The picture changed: what was on screen is of another source.
    fn source_changed(&mut self) {
        self.preview.clear();
        self.changed();
    }
    fn show_test_card(&mut self) {
        self.source.test_card();
        self.source_changed();
    }
    fn show_video_test_card(&mut self, frame: u64) {
        let opening = self.source.open_video_test_card(frame);
        self.start_opening(opening, "Loading video frame…".into());
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
        if crtsim_media::MediaKind::of(&path) == crtsim_media::MediaKind::Video
            && self.ffmpeg.missing()
        {
            self.status = format!("Opening {} needs FFmpeg", file_name(&path));
            self.show_ffmpeg_setup();
            return;
        }
        let status = format!("Loading {}…", path.display());
        let opening = self.source.open(path);
        self.start_opening(opening, status);
    }
    /// Has the work lane load what `opening` asks for, saying so: a video's frame, or else
    /// what `status` says.
    fn start_opening(&mut self, opening: Job, status: String) {
        let status = match opening {
            Job::LoadVideo { .. } => "Loading video frame…".into(),
            _ => status,
        };
        self.start(opening, status);
    }
    /// Starts `job` on the work lane, saying `status`. Any other job stops the video playing
    /// first, which holds the lane until then.
    fn start(&mut self, job: Job, status: String) {
        if !matches!(job, Job::Playback { .. }) {
            self.stop_playback();
        }
        match self.lane.start(job) {
            Ok(()) => self.status = status,
            // Only reachable by a slip: what starts work asks `can_start_work` first.
            Err(lane::Refused::Busy) => {}
            Err(lane::Refused::Stopped) => self.worker_stopped(),
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
        self.lane.stop();
        self.preview.stop();
    }
    /// The image, or frame, on screen, exported as a PNG at full resolution.
    fn export(&mut self, path: PathBuf) {
        self.stop_playback();
        if let Err(e) = self.config.validate_for(self.source.input().dimensions()) {
            self.error = Some(format!("{e:#}"));
            return;
        }
        let status = format!(
            "Exporting {}… Settings are captured for this export.",
            path.display()
        );
        let export = worker::Export::Image {
            input: self.source.input().clone(),
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
        self.start(
            Job::Export {
                export,
                path,
                cancel,
            },
            status,
        );
        if let Some(stage) = queued {
            self.lane.queued(stage);
        }
    }
    /// Channels in place of the worker's lanes, the Preview's included, for a test to inspect
    /// what the interface sends.
    #[cfg(test)]
    fn capture_jobs(&mut self) -> (mpsc::Receiver<Job>, mpsc::Receiver<PreviewJob>) {
        let (jobs, work, previews) = worker::Jobs::capture();
        self.preview = preview::Preview::new(jobs.preview_lane(), None, Instant::now());
        self.lane = lane::Lane::new(jobs.work_lane());
        self.jobs = jobs;
        (work, previews)
    }
    /// Whether a preview can be rendered now: not behind the welcome, nor while a file loads
    /// or a video plays. Also while exporting: previews have their own lane, and the export
    /// works from the settings it captured, so the ones on screen are free to change.
    fn may_preview(&self) -> bool {
        !self.show_welcome && !self.lane.is_loading() && !self.lane.is_playing()
    }
    /// Refresh: renders the settings on screen exactly now.
    fn refresh_preview(&mut self) {
        if self.may_preview() {
            let refreshed = self.preview.refresh(&self.config, self.source.input());
            self.ticked(refreshed);
        }
    }
    /// Acts on what the preview found: a change that settled becomes an undo step, and a
    /// stopped renderer is reported.
    fn ticked(&mut self, tick: preview::Tick) {
        if tick.settled {
            self.history.commit(&self.config);
        }
        if tick.stopped {
            self.worker_stopped();
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
                    self.preview.rendering_settled() || self.lane.is_working(),
                    self.error.is_some() || self.preview.error.is_some(),
                );
                ui.label(&self.status);
                chrome::version(ui);
            });
            if let Some(p) = self.lane.shown_progress() {
                ui.add(
                    egui::ProgressBar::new(p.fraction)
                        .text(format!("Export: {} — {:.0}%", p.stage, p.fraction * 100.))
                        .animate(true),
                );
            }
            if let Some(cancel) = self.lane.cancel().cloned() {
                if ui.button("Cancel").clicked() {
                    cancel.store(true, Ordering::Relaxed);
                    self.status = "Cancelling…".into();
                }
            }
            for error in [self.error.clone(), self.preview.error.clone()]
                .into_iter()
                .flatten()
            {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                    if ui.button("Dismiss").clicked() {
                        self.error = None;
                        self.preview.error = None;
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
        self.credits_window(ctx);
        self.ffmpeg_window(ctx);
        let now = Instant::now();
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        let may_preview = self.may_preview();
        let tick = self.preview.tick(
            now,
            pointer_down,
            &self.config,
            self.source.input(),
            may_preview,
        );
        self.ticked(tick);
        if self.preview.needs_frames(now) || self.lane.is_working() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.advance_smoke(ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop_playback();
        self.save_session();
        self.save_tool_windows();
        if let Some(cancel) = self.lane.cancel() {
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
    fn settings_changed_during_an_export_are_previewed() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        app.show_welcome = false;
        let (work, previews) = app.capture_jobs();
        let dir = tempfile::tempdir().unwrap();
        app.export(dir.path().join("rendered.png"));
        assert!(matches!(work.try_recv(), Ok(Job::Export { .. })));
        app.config.bloom = 0.;
        app.changed();
        app.refresh_preview();
        match previews.try_recv() {
            Ok(PreviewJob::Preview { config, .. }) => assert_eq!(config.bloom, 0.),
            _ => panic!("expected a preview while exporting"),
        }
        assert!(app.lane.is_exporting() && app.preview.rendering());
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
        let (receive, _previews) = app.capture_jobs();
        let dir = tempfile::tempdir().unwrap();
        app.config.output = "4k".into();
        app.preview.quality = Some(800);
        let source = app.source.input().clone();
        let expected = app.config.clone();
        app.export(dir.path().join("rendered.png"));
        app.config.bloom = 0.;
        app.source.set_image(RgbaImage::new(1, 1));
        match receive.recv().unwrap() {
            Job::Export {
                export: worker::Export::Image { input, config },
                cancel,
                ..
            } => {
                assert!(Arc::ptr_eq(&input, &source));
                assert!(Arc::ptr_eq(&cancel, app.lane.cancel().unwrap()));
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
