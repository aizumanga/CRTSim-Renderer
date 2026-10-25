//! The CRTSim Renderer app: editing a look, the galleries, playback and exports, whichever
//! host it runs in.
// A browser build still compiles the desktop's folders, threads and native dialogs, which it
// never reaches.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
mod app_data;
mod batch;
mod chrome;
mod closing;
mod dialogs;
mod events;
mod export_ui;
mod ffmpeg_setup;
mod files;
mod first_look;
mod frames;
mod gallery;
mod gallery_ui;
mod incoming;
mod interface;
mod lane;
mod look_link;
mod lut_gallery;
mod meter;
mod model;
#[cfg(not(target_arch = "wasm32"))]
pub mod native;
mod notices;
mod playback;
mod preview;
mod preview_ui;
mod project;
mod screen;
mod session;
mod settings_ui;
mod sheet_ui;
mod smoke;
mod sound;
mod source;
mod theme;
mod thumbnails;
mod timeline;
mod toolbar;
#[cfg(not(target_arch = "wasm32"))]
mod updates;
#[cfg(target_arch = "wasm32")]
pub mod web;
#[cfg(target_arch = "wasm32")]
mod web_encode;
#[cfg(target_arch = "wasm32")]
mod web_video;
mod widgets;
mod worker;

use crtsim_core::config::{ColorMode, Config, Filter, Fit, Fxaa, MaskRepeats, Phase, Tate};
use crtsim_core::settings;
use dialogs::Dialog;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use notices::Topic;
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
    /// How much the interface moves, and the sounds it makes.
    interface: interface::Interface,
    /// The interface's settings as last remembered.
    interface_saved: interface::Interface,
    speaker: sound::Speaker,
    /// What pictures' colour profiles do, and the space PNG stills are written in.
    colors: worker::Colors,
    show_welcome: bool,
    /// Whether the looks offered after a first run's welcome are on screen.
    first_look: bool,
    show_credits: bool,
    show_queue: bool,
    /// The batch export queue.
    queue: batch::Queue,
    presets: gallery::PresetGallery,
    luts: lut_gallery::LutGallery,
    /// Whether FFmpeg is there for video work, and how to install it.
    ffmpeg: ffmpeg_setup::FfmpegSetup,
    /// Whether a newer release is out, checked when asked.
    #[cfg(not(target_arch = "wasm32"))]
    updates: updates::Updates,
    thumbnails: thumbnails::Thumbnails,
    /// The contact sheet window and what the next sheet holds.
    sheet: sheet_ui::ContactSheet,
    tool_windows: app_data::Layout,
    tool_windows_saved: app_data::Layout,
    config: Config,
    history: model::History,
    /// Whether the panel shows only the settings that differ from the preset it names.
    changed_only: bool,
    /// The settings Changed only shows, kept while a pointer is down.
    changes_shown: std::collections::HashSet<&'static str>,
    /// What is open, and the picture it gives.
    source: source::Source,
    /// The CRT picture on screen, and when to render the next one.
    preview: preview::Preview,
    /// The preview's screen switching on and flickering, as a set's.
    screen: screen::Screen,
    view: View,
    /// Where the Compare view divides the original from the CRT, from 0 to 1 across.
    comparison: f32,
    /// The video export window, while it is open.
    export_dialog: Option<export_ui::ExportDialog>,
    /// What the last video export was saved as, which the next one starts from.
    export_format: export_ui::ExportFormat,
    /// What the video export under way is to do with the subtitle tracks of the video open.
    export_subtitles: export_ui::SubtitleChoice,
    zoom: f32,
    fit_preview: bool,
    /// Whether a video plays again from the start when it ends.
    loop_playback: bool,
    dialog_open: bool,
    /// The link typed into Open look from link, while that window is open.
    look_link: Option<String>,
    jobs: worker::Jobs,
    events: mpsc::Receiver<Event>,
    dialog_send: mpsc::Sender<dialogs::Answer>,
    dialog_receive: mpsc::Receiver<dialogs::Answer>,
    status: String,
    /// What failed, and the file the last export saved, by topic: the status bar's notices.
    notices: notices::Notices,
    /// Whether the window was asked to close while an export ran, and what was chosen.
    closing: closing::Closing,
    /// Whether the browser asks before the page is left, as it does while an export runs.
    #[cfg(target_arch = "wasm32")]
    leaving_asks: bool,
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
        ctx.set_fonts(theme::fonts());
        theme.apply(ctx);
        let interface = match store.as_ref().map(app_data::Store::interface) {
            Some(Ok(interface)) => interface,
            Some(Err(e)) => {
                storage_error = Some(format!(
                    "Could not load the interface's effects and sounds: {e:#}. Using the \
                     defaults."
                ));
                interface::Interface::default()
            }
            None => interface::Interface::default(),
        };
        interface.apply(ctx);
        let colors = match store.as_ref().map(app_data::Store::colors) {
            Some(Ok(colors)) => colors,
            Some(Err(e)) => {
                storage_error = Some(format!(
                    "Could not load the colour settings: {e:#}. Using sRGB."
                ));
                worker::Colors::default()
            }
            None => worker::Colors::default(),
        };
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
            interface,
            interface_saved: interface,
            speaker: Default::default(),
            colors,
            show_welcome,
            first_look: smoke.as_ref().is_some_and(|smoke| smoke.first_look),
            show_credits: false,
            show_queue: false,
            queue: Default::default(),
            presets,
            luts,
            ffmpeg: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            updates: Default::default(),
            thumbnails: Default::default(),
            sheet: Default::default(),
            tool_windows_saved: tool_windows.clone(),
            tool_windows,
            history: model::History::new(config.clone()),
            changed_only: false,
            changes_shown: Default::default(),
            config,
            source: source::Source::new(ctx),
            preview: preview::Preview::new(jobs.preview_lane(), render_state, Instant::now()),
            screen: Default::default(),
            view: View::Crt,
            comparison: 0.5,
            export_dialog: None,
            export_format: Default::default(),
            export_subtitles: Default::default(),
            zoom: 1.,
            fit_preview: true,
            loop_playback: false,
            dialog_open: false,
            look_link: None,
            jobs,
            events,
            dialog_send,
            dialog_receive,
            status: "Preparing preview…".into(),
            notices: Default::default(),
            closing: Default::default(),
            #[cfg(target_arch = "wasm32")]
            leaving_asks: false,
            smoke,
        };
        if let Some(error) = storage_error {
            app.notices.failed(Topic::AppData, error);
        }
        app.source.profiles = app.colors.profiles;
        // The screen switches on for the first picture, as a set does when the app starts. A
        // browser plays no sound before the first click, so there the screen waits for it.
        app.screen.new_media();
        if cfg!(target_arch = "wasm32") && app.interface.sounds.on {
            app.screen.stand_by();
        }
        app.ffmpeg.start_check(ctx);
        #[cfg(not(target_arch = "wasm32"))]
        app.check_for_updates_at_start(ctx);
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
        self.screen.new_media();
        self.changed();
    }
    /// A preset chosen from a gallery replaces the settings, to the sound of a coin and a
    /// flicker of the screen.
    fn apply_preset(&mut self, config: Config) {
        self.replace_config(config);
        sound::cue(&self.ui_context, sound::Sound::Preset);
        self.screen.flicker(&self.ui_context);
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
        self.notices.failed(
            Topic::Renderer,
            "Render worker stopped. Save your preset and restart the application.",
        );
        self.lane.stop();
        self.preview.stop();
    }
    /// The image, or frame, on screen, exported as a PNG at full resolution.
    fn export(&mut self, path: PathBuf) {
        self.stop_playback();
        if let Err(e) = self.config.validate_for(self.source.input().dimensions()) {
            self.notices.failed(Topic::Saving, format!("{e:#}"));
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
        self.notices.forget(Topic::Saving);
        self.start(
            Job::Export {
                export,
                path,
                cancel,
                colors: self.colors.clone(),
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
    /// The notices, one row each, newest last, with each one's button and its own Dismiss.
    fn notices_ui(&mut self, ui: &mut egui::Ui) {
        self.notices.follow_preview(self.preview.error.as_ref());
        let mut act = None;
        let mut dismiss = None;
        for notice in self.notices.iter() {
            ui.horizontal_wrapped(|ui| {
                let text = match notice.severity {
                    notices::Severity::Error => {
                        ui.colored_label(ui.visuals().error_fg_color, &notice.text)
                    }
                    notices::Severity::Done => ui.label(&notice.text),
                };
                if let Some(detail) = &notice.detail {
                    text.on_hover_text(detail);
                }
                let label = match &notice.action {
                    // A browser downloads what it saves; there is no folder to show.
                    Some(notices::Action::ShowInFolder(_)) if cfg!(target_arch = "wasm32") => None,
                    Some(notices::Action::ShowInFolder(_)) => Some("Show in folder"),
                    Some(notices::Action::FfmpegSetup) => Some("FFmpeg setup…"),
                    None => None,
                };
                if label.is_some_and(|label| ui.small_button(label).clicked()) {
                    act = notice.action.clone();
                }
                if ui.small_button("Dismiss").clicked() {
                    dismiss = Some(notice.topic);
                }
            });
        }
        if let Some(topic) = dismiss {
            self.notices.dismiss(topic);
            // The preview keeps its own error, which would otherwise come straight back.
            if topic == Topic::Preview {
                self.preview.error = None;
            }
        }
        match act {
            #[cfg(not(target_arch = "wasm32"))]
            Some(notices::Action::ShowInFolder(path)) => {
                if let Err(e) = files::show_in_folder(&path) {
                    self.notices
                        .failed(Topic::Opening, format!("Cannot show the folder: {e:#}"));
                }
            }
            Some(notices::Action::FfmpegSetup) => self.show_ffmpeg_setup(),
            _ => {}
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
            Err(e) => self.notices.failed(
                Topic::AppData,
                format!("Window layout could not be saved: {e:#}"),
            ),
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
        self.ask_before_closing(ctx);
        if !self.modal_open() && !ctx.egui_wants_keyboard_input() {
            self.shortcuts(ctx);
        }
        if self.can_start_work() {
            // A browser gives a dropped file's name and bytes; the desktop, its path.
            #[cfg(target_arch = "wasm32")]
            if let Some(file) = ctx.input(|i| i.raw.dropped_files.first().cloned()) {
                self.dropped(file);
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let paths = ctx.input(|i| {
                    i.raw
                        .dropped_files
                        .iter()
                        .map(|file| file.path().to_path_buf())
                        .filter(|path| !path.as_os_str().is_empty())
                        .collect()
                });
                self.dropped_paths(paths, ctx);
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
                    self.notices.any_error() || self.preview.error.is_some(),
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
            self.notices_ui(ui);
        });
        // Beside the picture, or below it on a phone, where the picture still shows each change.
        let settings = if chrome::narrow(ctx) {
            let height = ctx.content_rect().height();
            egui::Panel::bottom("settings below")
                .default_size(height * 0.45)
                .min_size(120.)
                .max_size(height * 0.75)
        } else {
            egui::Panel::left("settings")
                .default_size(310.)
                .min_size(280.)
        };
        settings.resizable(true).show(ui, |ui| {
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
        self.first_look_window(ctx);
        self.ffmpeg_window(ctx);
        self.look_link_window(ctx);
        self.contact_sheet_window(ctx);
        #[cfg(target_arch = "wasm32")]
        if !self.modal_open() {
            self.look_from_address_bar();
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.updates_window(ctx);
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
        self.save_interface(ctx);
        let sounds = sound::take(ctx);
        let gesture = ctx.input(|i| {
            i.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerButton { pressed: true, .. }
                        | egui::Event::Key { pressed: true, .. }
                        | egui::Event::Touch { .. }
                )
            })
        });
        if gesture {
            self.speaker.woken();
            self.screen.wake(ctx);
        }
        // A smoke run's screenshot needs no sound, and CI has no audio output to play it on.
        if self.smoke.is_none() {
            self.speaker.play(&sounds, &self.interface.sounds);
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
    fn a_saved_file_stays_in_view_until_the_next_export() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        let (_work, _previews) = app.capture_jobs();
        let (events, received) = mpsc::channel();
        app.events = received;
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.png");
        app.export(first.clone());
        events.send(Event::Exported(Ok(first.clone()))).unwrap();
        app.receive(&ctx);
        assert_eq!(app.notices.saved_file(), Some(first.as_path()));
        app.export(dir.path().join("second.png"));
        assert_eq!(app.notices.saved_file(), None);
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
        assert!(project::read(&path).is_ok(), "{:?}", app.notices);
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
        app.preview.quality = preview::Quality::new(preview::Choice::Fixed(Some(800)));
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
