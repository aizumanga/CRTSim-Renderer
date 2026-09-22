use crate::{files, model, texture, worker, App, Dialog, Job};
use anyhow::{ensure, Result};
use crtsim_core::config::Config;
use crtsim_media::Options;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

const MAX_PROJECT: u64 = 64 * 1024 * 1024;
type BatchSelection = Option<(Vec<PathBuf>, PathBuf)>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    version: u32,
    source: Option<PathBuf>,
    frame: u64,
    config: Config,
    options: Options,
    queue: Vec<QueueItem>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueueItem {
    source: PathBuf,
    output: PathBuf,
    config: Config,
    options: Options,
    status: QueueStatus,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
enum QueueStatus {
    Pending,
    Running,
    Done,
    Cancelled,
    Failed(String),
}

pub fn is_video(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["mp4", "mkv", "mov", "webm", "avi", "m4v"].contains(&e.to_ascii_lowercase().as_str())
    })
}
fn read_project(path: &Path) -> Result<Project> {
    ensure!(
        std::fs::metadata(path)?.len() <= MAX_PROJECT,
        "Project exceeds 64 MB"
    );
    let mut p: Project = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(p.version == 1, "Unsupported project version");
    p.config.validate()?;
    p.options.validate()?;
    ensure!(p.queue.len() <= 256, "Queue exceeds 256 jobs");
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    let resolve = |p: &mut PathBuf| {
        if p.is_relative() {
            *p = root.join(&*p);
        }
    };
    if let Some(source) = p.source.as_mut() {
        resolve(source);
    }
    for item in &mut p.queue {
        item.config.validate()?;
        item.options.validate()?;
        resolve(&mut item.source);
        resolve(&mut item.output);
        if item.status == QueueStatus::Running {
            item.status = QueueStatus::Pending;
        }
    }
    Ok(p)
}
fn save_project(path: &Path, project: &Project) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(project)?;
    ensure!(
        bytes.len() as u64 <= MAX_PROJECT,
        "Project with embedded LUTs exceeds 64 MB"
    );
    files::save_atomic(path, |f| Ok(f.write_all(&bytes)?))
}

pub struct Playback {
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<Result<worker::PlaybackFrame, String>>,
    buffered: VecDeque<worker::PlaybackFrame>,
    clock: Option<(Instant, f64)>,
    ended: bool,
    capacity: usize,
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct State {
    pub source: Option<PathBuf>,
    pub comparison: f32,
    pub playback: Option<Playback>,
    pub play_time: f64,
    pub export_dialog: Option<crate::export_ui::ExportDialog>,
    pub export_format: crate::export_ui::Format,
    pub project_path: Option<PathBuf>,
    recovery: Option<Project>,
    recent: Vec<PathBuf>,
    queue: Vec<QueueItem>,
    queue_running: bool,
    active: Option<usize>,
    pub show_queue: bool,
    last_saved: Option<Project>,
    last_save: Instant,
    pub pending_project: Option<Project>,
    batch: Option<mpsc::Receiver<BatchSelection>>,
    batch_settings: Option<(Config, Options)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            source: None,
            comparison: 0.5,
            playback: None,
            play_time: 0.,
            export_dialog: None,
            export_format: Default::default(),
            project_path: None,
            recovery: None,
            recent: vec![],
            queue: vec![],
            queue_running: false,
            active: None,
            show_queue: false,
            last_saved: None,
            last_save: Instant::now(),
            pending_project: None,
            batch: None,
            batch_settings: None,
        }
    }
}

impl App {
    pub fn init_workflow(&mut self, explicit_input: bool) {
        if self.smoke.is_some() {
            return;
        }
        if let Some(store) = &self.store {
            let recent = store.root.join("recent-projects-v1.json");
            if std::fs::metadata(&recent).is_ok_and(|m| m.len() <= 64 * 1024) {
                match std::fs::read(&recent)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Vec<PathBuf>>(&b).ok())
                {
                    Some(mut paths) => {
                        paths.truncate(10);
                        self.workflow.recent = paths;
                    }
                    None => self.error = Some("Could not read recent projects".into()),
                }
            }
            let recovery = store.root.join("session-v1.crtsim");
            if !explicit_input && recovery.exists() {
                match read_project(&recovery) {
                    Ok(project) => self.workflow.recovery = Some(project),
                    Err(e) => {
                        self.error = Some(format!("Could not recover the last session: {e:#}"))
                    }
                }
            }
        }
    }
    fn snapshot(&self) -> Project {
        Project {
            version: 1,
            source: self.workflow.source.clone(),
            frame: self.video_frame,
            config: self.config.clone(),
            options: self.video_options.clone(),
            queue: self.workflow.queue.clone(),
        }
    }
    pub fn save_session(&mut self) {
        if self.smoke.is_some()
            || self.workflow.recovery.is_some()
            || self.loading
            || self.workflow.pending_project.is_some()
        {
            return;
        }
        let p = self.snapshot();
        if self.workflow.last_saved.as_ref() == Some(&p) {
            return;
        }
        if let Some(store) = &self.store {
            match save_project(&store.root.join("session-v1.crtsim"), &p) {
                Ok(()) => self.workflow.last_saved = Some(p),
                Err(e) => self.error = Some(format!("Session recovery could not be saved: {e:#}")),
            }
        }
    }
    fn remember_project(&mut self, path: PathBuf) {
        self.workflow.recent.retain(|p| p != &path);
        self.workflow.recent.insert(0, path);
        self.workflow.recent.truncate(10);
        if let Some(store) = &self.store {
            let result = serde_json::to_vec(&self.workflow.recent)
                .map_err(anyhow::Error::from)
                .and_then(|b| {
                    files::save_atomic(&store.root.join("recent-projects-v1.json"), |f| {
                        Ok(f.write_all(&b)?)
                    })
                });
            if let Err(e) = result {
                self.error = Some(format!("Cannot save recent projects: {e:#}"));
            }
        }
    }
    pub fn save_project_file(&mut self, path: PathBuf) {
        match save_project(&path, &self.snapshot()) {
            Ok(()) => {
                self.workflow.project_path = Some(path.clone());
                self.status = format!("Saved project {}", path.display());
                self.remember_project(path);
            }
            Err(e) => self.error = Some(format!("Cannot save project: {e:#}")),
        }
    }
    pub fn open_project(&mut self, path: PathBuf) {
        match read_project(&path) {
            Ok(p) => {
                self.workflow.project_path = Some(path.clone());
                self.restore_project(p);
                self.remember_project(path);
            }
            Err(e) => self.error = Some(format!("Cannot open project: {e:#}")),
        }
    }
    fn restore_project(&mut self, p: Project) {
        self.stop_playback();
        if let Some(source) = &p.source {
            if !source.exists() {
                let message=format!("Project source is missing: {}. Edits and queue were recovered. Use Open File to relink the source.",source.display());
                self.workflow.source = Some(source.clone());
                self.apply_project(p);
                self.error = Some(message);
                return;
            }
            self.workflow.pending_project = Some(p.clone());
            if is_video(source) {
                self.load_video(source.clone(), p.frame, false);
            } else {
                self.load(source.clone());
            }
        } else {
            self.video = None;
            self.workflow.source = None;
            self.input = Arc::new(crtsim_core::config::test_card());
            self.original = texture(&self.ui_context, "original", &self.input, 2048);
            self.source_name = "Built-in test card".into();
            self.apply_project(p);
        }
    }
    pub fn apply_project(&mut self, p: Project) {
        self.video_options = p.options;
        self.workflow.queue = p.queue;
        self.workflow.queue_running = false;
        self.workflow.active = None;
        self.replace_config(p.config);
        self.status = "Project restored; queue paused".into();
    }
    pub fn stop_playback(&mut self) {
        if self.workflow.playback.take().is_some() {
            self.status = "Playback paused".into();
        }
    }
    pub fn start_playback(&mut self) {
        let Some(video) = self.video.clone() else {
            return;
        };
        self.stop_playback();
        let config = match model::preview_config(
            &self.config,
            self.input.dimensions(),
            self.preview_limit,
        ) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(format!("Cannot play: {e:#}"));
                return;
            }
        };
        let start = if self.workflow.play_time >= video.duration - 1. / video.fps {
            0.
        } else {
            self.workflow.play_time
        };
        // Target 128 MB across both buffers, with one pair per buffer at minimum.
        let out = config.output_size(video.size).unwrap_or(video.size);
        let pair_bytes = 4
            * (u64::from(video.size.0) * u64::from(video.size.1)
                + u64::from(out.0) * u64::from(out.1));
        let capacity = ((64 * 1024 * 1024) / pair_bytes.max(1)).clamp(1, 4) as usize;
        let (frames, receive) = mpsc::sync_channel(capacity);
        let cancel = Arc::new(AtomicBool::new(false));
        self.workflow.playback = Some(Playback {
            cancel: cancel.clone(),
            receive,
            buffered: VecDeque::new(),
            clock: None,
            ended: false,
            capacity,
        });
        self.dirty = false;
        self.status = "Buffering · warming CRT history…".into();
        self.send(Job::Playback {
            video,
            start,
            config,
            options: self.video_options.clone(),
            cancel,
            frames,
        });
    }
    fn tick_playback(&mut self, ctx: &egui::Context) {
        let Some(p) = self.workflow.playback.as_mut() else {
            return;
        };
        while p.buffered.len() < p.capacity && !p.ended {
            match p.receive.try_recv() {
                Ok(Ok(f)) => p.buffered.push_back(f),
                Ok(Err(e)) => {
                    self.error = Some(e);
                    p.ended = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => p.ended = true,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if p.clock.is_none()
            && (p.buffered.len() >= p.capacity.min(3) || (p.ended && !p.buffered.is_empty()))
        {
            let time = p.buffered.front().unwrap().time;
            p.clock = Some((Instant::now(), time));
            self.status = "Playing".into();
        }
        if let Some((clock, base)) = p.clock {
            let target = base + clock.elapsed().as_secs_f64();
            let mut next = None;
            while p.buffered.front().is_some_and(|f| f.time <= target) {
                next = p.buffered.pop_front();
            }
            if let Some(f) = next {
                self.workflow.play_time = f.time;
                self.original = texture(ctx, "playing-source", &f.source, 2048);
                self.rendered = Some(texture(
                    ctx,
                    "playing-crt",
                    &f.crt,
                    ctx.input(|i| i.max_texture_side) as u32,
                ));
                self.rendered_revision = Some(self.revision);
                self.input = Arc::new(f.source);
                if let Some(v) = &self.video {
                    self.video_frame = (f.time * v.fps).round() as u64;
                    self.video_frame = self.video_frame.min(self.video_frames.saturating_sub(1));
                    self.selected_frame = self.video_frame;
                }
            }
            let frame_duration = self.video.as_ref().map_or(1. / 30., |v| 1. / v.fps);
            if p.buffered.is_empty()
                && !p.ended
                && target > self.workflow.play_time + frame_duration
            {
                p.clock = None;
                self.status = "Buffering · renderer is catching up…".into();
            }
        }
        if p.ended && p.buffered.is_empty() {
            self.stop_playback();
            self.status = "Playback finished".into();
        } else {
            ctx.request_repaint_after(Duration::from_millis(8));
        }
    }
    pub fn queue_finished(&mut self, result: &Result<PathBuf, String>) {
        if let Some(index) = self.workflow.active.take() {
            self.workflow.queue[index].status = match result {
                Ok(_) => QueueStatus::Done,
                Err(_) if self.cancel.load(Ordering::Relaxed) => QueueStatus::Cancelled,
                Err(e) => QueueStatus::Failed(e.clone()),
            };
            if self.cancel.load(Ordering::Relaxed) {
                self.workflow.queue_running = false;
            }
            self.save_session();
        }
    }
    fn dispatch_queue(&mut self) {
        if !self.workflow.queue_running
            || self.exporting
            || self.loading
            || self.rendering
            || self.dialog_open
            || self.workflow.export_dialog.is_some()
            || self.show_welcome
            || self.workflow.recovery.is_some()
            || self.workflow.playback.is_some()
        {
            return;
        }
        let Some(index) = self
            .workflow
            .queue
            .iter()
            .position(|q| q.status == QueueStatus::Pending)
        else {
            self.workflow.queue_running = false;
            return;
        };
        let q = &mut self.workflow.queue[index];
        if q.output.exists() {
            q.status=QueueStatus::Failed("Destination already exists. Remove it or add a new job; batch exports never intentionally replace existing files.".into());
            return;
        }
        q.status = QueueStatus::Running;
        let q = q.clone();
        self.workflow.active = Some(index);
        self.exporting = true;
        self.video_job = is_video(&q.source);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.status = format!("Batch export {}", q.source.display());
        self.save_session();
        self.send(Job::Batch {
            source: q.source,
            path: q.output,
            config: q.config,
            options: q.options,
            cancel: self.cancel.clone(),
        });
    }
    fn batch_dialog(&mut self, ctx: &egui::Context) {
        self.stop_playback();
        self.dialog_open = true;
        self.workflow.batch_settings = Some((self.config.clone(), self.video_options.clone()));
        let (send, receive) = mpsc::channel();
        self.workflow.batch = Some(receive);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .add_filter(
                    "Images and videos",
                    &[
                        "png", "jpg", "jpeg", "webp", "bmp", "mp4", "mkv", "mov", "webm", "avi",
                        "m4v",
                    ],
                )
                .pick_files()
                .and_then(|files| {
                    rfd::FileDialog::new()
                        .set_title("Batch export destination")
                        .pick_folder()
                        .map(|folder| (files, folder))
                });
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }
    pub fn workflow_ui(&mut self, ctx: &egui::Context) {
        self.video_export_window(ctx);
        self.tick_playback(ctx);
        if let Some(receive) = &self.workflow.batch {
            match receive.try_recv() {
                Ok(result) => {
                    self.workflow.batch = None;
                    self.dialog_open = false;
                    let (config, options) = self.workflow.batch_settings.take().unwrap();
                    if let Some((sources, folder)) = result {
                        for source in sources {
                            if self.workflow.queue.len() >= 256 {
                                self.error = Some("Batch queue is limited to 256 jobs".into());
                                break;
                            }
                            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
                            let ext = if is_video(&source) { "mkv" } else { "png" };
                            let mut n = 0;
                            let output = loop {
                                let suffix = if n == 0 {
                                    String::new()
                                } else {
                                    format!("-{n}")
                                };
                                let path = folder.join(format!("{stem}-crt{suffix}.{ext}"));
                                if !path.exists()
                                    && !self.workflow.queue.iter().any(|q| q.output == path)
                                {
                                    break path;
                                }
                                n += 1;
                            };
                            self.workflow.queue.push(QueueItem {
                                source,
                                output,
                                config: config.clone(),
                                options: options.clone(),
                                status: QueueStatus::Pending,
                            });
                        }
                        self.workflow.show_queue = true;
                        self.save_session();
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.workflow.batch = None;
                    self.dialog_open = false;
                    self.error = Some("File chooser closed unexpectedly".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.workflow.recovery.is_some() && !self.show_welcome {
            let mut restore = false;
            let mut discard = false;
            egui::Window::new("Recover last session?")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(
                        "Restore the last source, edits, video options and paused export queue.",
                    );
                    ui.horizontal(|ui| {
                        restore = ui.button("Restore session").clicked();
                        discard = ui.button("Start fresh").clicked();
                    });
                });
            if restore {
                let p = self.workflow.recovery.take().unwrap();
                self.restore_project(p);
            }
            if discard {
                self.workflow.recovery = None;
                self.save_session();
            }
        }
        if self.workflow.show_queue {
            let mut remove = None;
            let mut move_up = None;
            let mut window = self.window_state("Batch export queue");
            let open = crate::chrome::tool_window(
                ctx,
                "Batch export queue",
                [650., 560.],
                &mut window,
                |ui| {
                    ui.label("Each job keeps the settings used when it was added. Videos use MKV to preserve more tracks.");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(
                                !self.dialog_open && self.workflow.export_dialog.is_none(),
                                egui::Button::new("Add files…"),
                            )
                            .clicked()
                        {
                            self.batch_dialog(ctx);
                        }
                        if ui
                            .add_enabled(
                                !self.dialog_open && self.workflow.export_dialog.is_none(),
                                egui::Button::new("Video settings…"),
                            )
                            .clicked()
                        {
                            self.open_video_export(true);
                        }
                        if ui
                            .button(if self.workflow.queue_running {
                                "Pause after current"
                            } else {
                                "Start / resume"
                            })
                            .clicked()
                        {
                            self.stop_playback();
                            self.workflow.queue_running = !self.workflow.queue_running;
                        }
                        if ui
                            .add_enabled(
                                self.workflow.active.is_some(),
                                egui::Button::new("Cancel current"),
                            )
                            .clicked()
                        {
                            self.cancel.store(true, Ordering::Relaxed);
                            self.workflow.queue_running = false;
                        }
                    });
                    egui::ScrollArea::vertical()
                        .max_height(400.)
                        .show(ui, |ui| {
                            for (i, q) in self.workflow.queue.iter_mut().enumerate() {
                                ui.push_id(i, |ui| {
                                    ui.group(|ui| {
                                        ui.label(q.source.display().to_string());
                                        ui.small(format!("→ {}", q.output.display()));
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(match &q.status {
                                                QueueStatus::Pending => "Pending".into(),
                                                QueueStatus::Running => "Exporting…".into(),
                                                QueueStatus::Done => "Saved".into(),
                                                QueueStatus::Cancelled => "Cancelled".into(),
                                                QueueStatus::Failed(e) => format!("Failed: {e}"),
                                            });
                                            if q.status != QueueStatus::Running
                                                && self.workflow.active.is_none()
                                            {
                                                if matches!(
                                                    q.status,
                                                    QueueStatus::Failed(_) | QueueStatus::Cancelled
                                                ) && ui.small_button("Retry").clicked()
                                                {
                                                    q.status = QueueStatus::Pending;
                                                }
                                                if i > 0 && ui.small_button("Move up").clicked() {
                                                    move_up = Some(i);
                                                }
                                                if ui.small_button("Remove").clicked() {
                                                    remove = Some(i);
                                                }
                                            }
                                        });
                                    });
                                });
                            }
                        });
                },
            );
            self.store_window_state("Batch export queue", window);
            if let Some(i) = remove {
                self.workflow.queue.remove(i);
            }
            if let Some(i) = move_up {
                self.workflow.queue.swap(i, i - 1);
            }
            self.workflow.show_queue = open;
        }
        self.dispatch_queue();
        if self.workflow.last_save.elapsed() > Duration::from_secs(2) {
            self.workflow.last_save = Instant::now();
            self.save_session();
            self.save_tool_windows();
        }
        if self.smoke.is_none() {
            ctx.request_repaint_after(Duration::from_secs(2));
        }
    }
    pub fn project_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.menu_button("Project", |ui| {
            if ui.button("Open project…").clicked() {
                self.dialog(Dialog::OpenProject, ctx);
                ui.close_menu();
            }
            if ui.button("Save project as…").clicked() {
                self.dialog(Dialog::SaveProject, ctx);
                ui.close_menu();
            }
            if ui
                .add_enabled(
                    self.workflow.project_path.is_some(),
                    egui::Button::new("Save project"),
                )
                .clicked()
            {
                if let Some(path) = self.workflow.project_path.clone() {
                    self.save_project_file(path);
                }
                ui.close_menu();
            }
            ui.separator();
            ui.label("Recent projects");
            for path in self.workflow.recent.clone() {
                if ui.button(path.display().to_string()).clicked() {
                    self.open_project(path);
                    ui.close_menu();
                }
            }
        });
    }
    pub fn workflow_settings(&mut self, ui: &mut egui::Ui) {
        crate::chrome::Section::new("Source & framing")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.config.screen_only, "Screen only · no bezel");
                egui::CollapsingHeader::new("Crop edges · percent").show(ui, |ui| {
                    for (i, name) in ["Left", "Top", "Right", "Bottom"].iter().enumerate() {
                        let opposite = (i + 2) % 4;
                        let max = (0.98 - self.config.source.crop[opposite]).max(0.);
                        let mut percent = self.config.source.crop[i] * 100.;
                        if ui
                            .add(egui::Slider::new(&mut percent, 0.0..=max * 100.).text(*name))
                            .changed()
                        {
                            self.config.source.crop[i] = percent / 100.;
                        }
                    }
                });
                ui.add(
                    egui::Slider::new(&mut self.config.source.rotation, -180.0..=180.)
                        .text("Rotation °"),
                );
                ui.add(
                    egui::Slider::new(&mut self.config.source.zoom, 0.05..=4.)
                        .logarithmic(true)
                        .text("Source zoom"),
                );
                ui.add(
                    egui::Slider::new(&mut self.config.source.position[0], -1.0..=1.).text("Pan X"),
                );
                ui.add(
                    egui::Slider::new(&mut self.config.source.position[1], -1.0..=1.).text("Pan Y"),
                );
                ui.label("Transparency background (included in exports)");
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            !self.config.source.checkerboard
                                && self.config.source.background == [0; 3],
                            "Black",
                        )
                        .clicked()
                    {
                        self.config.source.checkerboard = false;
                        self.config.source.background = [0; 3];
                    }
                    if ui
                        .selectable_label(
                            !self.config.source.checkerboard
                                && self.config.source.background == [255; 3],
                            "White",
                        )
                        .clicked()
                    {
                        self.config.source.checkerboard = false;
                        self.config.source.background = [255; 3];
                    }
                    ui.checkbox(&mut self.config.source.checkerboard, "Checker");
                });
                if ui
                    .color_edit_button_srgb(&mut self.config.source.background)
                    .changed()
                {
                    self.config.source.checkerboard = false;
                }
                if ui.button("Reset source framing").clicked() {
                    self.config.source = Default::default();
                }
            });
        crate::chrome::Section::new("Color & LUT").show(ui, |ui| {
            ui.label(
                self.config
                    .lut
                    .as_ref()
                    .map_or("No LUT", |l| l.name.as_str()),
            );
            if ui.button("LUT gallery…").clicked() {
                self.stop_playback();
                self.show_lut_gallery = true;
            }
            if ui.button("Import 3D .cube…").clicked() {
                self.dialog(Dialog::Lut, &ui.ctx().clone());
            }
            if self.config.lut.is_some() && ui.button("Remove LUT").clicked() {
                self.config.lut = None;
            }
            ui.small(
                "Applied before CRT simulation. The table is embedded in presets and projects.",
            );
        });
    }
}

pub fn compare(
    ui: &mut egui::Ui,
    original: &egui::TextureHandle,
    rendered: &egui::TextureHandle,
    area: egui::Vec2,
    fit: bool,
    zoom: f32,
    split: &mut f32,
) {
    let native = rendered.size_vec2();
    let size = if fit {
        native * (area.x / native.x).min(area.y / native.y)
    } else {
        native * zoom
    };
    let (area_rect, response) =
        ui.allocate_exact_size(if fit { area } else { size }, egui::Sense::click_and_drag());
    let rect = egui::Rect::from_center_size(area_rect.center(), size);
    if let Some(pos) = response.interact_pointer_pos() {
        *split = ((pos.x - rect.left()) / rect.width()).clamp(0., 1.);
    }
    let x = rect.left() + rect.width() * *split;
    let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
    ui.painter()
        .image(rendered.id(), rect, uv, egui::Color32::WHITE);
    let left = egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.bottom()));
    let painter = ui.painter().with_clip_rect(left.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0., egui::Color32::BLACK);
    let os = original.size_vec2();
    let os = os * (size.x / os.x).min(size.y / os.y);
    painter.image(
        original.id(),
        egui::Rect::from_center_size(rect.center(), os),
        uv,
        egui::Color32::WHITE,
    );
    ui.painter().line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
    );
    ui.painter().circle_filled(
        egui::pos2(x, rect.center().y),
        7.0_f32,
        egui::Color32::WHITE,
    );
    response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag to compare · original on the left, CRT on the right");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn playback_buffers_before_presenting_and_cancel_discards_pending_frames() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            wgpu::Backends::PRIMARY,
            None,
            Some("unused-smoke.png".into()),
        );
        let (send, receive) = mpsc::sync_channel(4);
        let cancel = Arc::new(AtomicBool::new(false));
        app.workflow.playback = Some(Playback {
            cancel: cancel.clone(),
            receive,
            buffered: VecDeque::new(),
            clock: None,
            ended: false,
            capacity: 4,
        });
        let make_frame = |time| {
            Ok(worker::PlaybackFrame {
                time,
                source: image::RgbaImage::new(4, 4),
                crt: image::RgbaImage::new(4, 4),
            })
        };
        send.send(make_frame(0.)).unwrap();
        app.tick_playback(&ctx);
        assert!(app.rendered.is_none());
        send.send(make_frame(0.04)).unwrap();
        send.send(make_frame(0.08)).unwrap();
        app.tick_playback(&ctx);
        assert!(app.rendered.is_some());
        assert!(app.workflow.playback.as_ref().unwrap().clock.is_some());
        app.stop_playback();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(send.send(make_frame(0.12)).is_err());
    }
    #[test]
    fn batch_captures_settings_refuses_existing_outputs_and_pauses_on_cancel() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &ctx,
            wgpu::Backends::PRIMARY,
            None,
            Some("unused-smoke.png".into()),
        );
        app.show_welcome = false;
        let (send, receive) = mpsc::channel();
        app.jobs = send;
        let item = QueueItem {
            source: dir.path().join("source.png"),
            output: dir.path().join("output.png"),
            config: app.config.clone(),
            options: Options::default(),
            status: QueueStatus::Pending,
        };
        let captured = item.config.clone();
        app.workflow.queue.push(item);
        app.config.bloom = 0.;
        app.workflow.queue_running = true;
        app.dispatch_queue();
        match receive.try_recv().unwrap() {
            Job::Batch { config, .. } => assert_eq!(config, captured),
            _ => panic!("Expected batch export"),
        }
        app.cancel.store(true, Ordering::Relaxed);
        app.queue_finished(&Err("Cancelled".into()));
        assert!(!app.workflow.queue_running);
        assert_eq!(app.workflow.queue[0].status, QueueStatus::Cancelled);
        app.exporting = false;
        app.workflow.queue[0].status = QueueStatus::Pending;
        std::fs::write(&app.workflow.queue[0].output, b"keep me").unwrap();
        app.workflow.queue_running = true;
        app.dispatch_queue();
        assert!(matches!(
            app.workflow.queue[0].status,
            QueueStatus::Failed(_)
        ));
        assert!(receive.try_recv().is_err());
        assert_eq!(
            std::fs::read(&app.workflow.queue[0].output).unwrap(),
            b"keep me"
        );
    }
    #[test]
    fn project_roundtrip_recovers_running_jobs_and_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.crtsim");
        let c = Config::default();
        let p = Project {
            version: 1,
            source: Some("input.png".into()),
            frame: 7,
            config: c.clone(),
            options: Options::default(),
            queue: vec![QueueItem {
                source: "input.png".into(),
                output: "output.png".into(),
                config: c,
                options: Options::default(),
                status: QueueStatus::Running,
            }],
        };
        save_project(&path, &p).unwrap();
        let read = read_project(&path).unwrap();
        assert_eq!(read.frame, 7);
        assert_eq!(read.source, Some(dir.path().join("input.png")));
        assert_eq!(read.queue[0].status, QueueStatus::Pending);
        let mut bad = p;
        bad.version = 99;
        save_project(&path, &bad).unwrap();
        assert!(read_project(&path).is_err());
    }
}
