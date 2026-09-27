use crate::{app_data, project, App, Dialog};
use eframe::egui;
use project::Project;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub struct State {
    pub source: Option<PathBuf>,
    pub comparison: f32,
    pub export_dialog: Option<crate::export_ui::ExportDialog>,
    pub export_format: crate::export_ui::ExportFormat,
    pub project_path: Option<PathBuf>,
    recovery: Option<Project>,
    recent: Vec<PathBuf>,
    last_saved: Option<Project>,
    last_save: Instant,
    pub pending_project: Option<Project>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            source: None,
            comparison: 0.5,
            export_dialog: None,
            export_format: Default::default(),
            project_path: None,
            recovery: None,
            recent: vec![],
            last_saved: None,
            last_save: Instant::now(),
            pending_project: None,
        }
    }
}
impl State {
    /// Whether the last session is on offer to be recovered, which nothing else may run
    /// before, since recovering replaces the batch queue.
    pub fn recovery_offered(&self) -> bool {
        self.recovery.is_some()
    }
}

impl App {
    pub fn init_workflow(&mut self, explicit_input: bool) {
        let Some(store) = self.app_data().cloned() else {
            return;
        };
        match store.recent_projects() {
            Ok(paths) => self.workflow.recent = paths,
            Err(_) => self.error = Some("Could not read recent projects".into()),
        }
        if !explicit_input {
            match store.session() {
                Ok(session) => self.workflow.recovery = session,
                Err(e) => self.error = Some(format!("Could not recover the last session: {e:#}")),
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
            queue: self.queue.items().to_vec(),
        }
    }
    pub fn save_session(&mut self) {
        if self.workflow.recovery.is_some()
            || self.work.is_loading()
            || self.workflow.pending_project.is_some()
        {
            return;
        }
        let p = self.snapshot();
        if self.workflow.last_saved.as_ref() == Some(&p) {
            return;
        }
        let Some(saved) = self.app_data().map(|store| store.set_session(&p)) else {
            return;
        };
        match saved {
            Ok(()) => self.workflow.last_saved = Some(p),
            Err(e) => self.error = Some(format!("Session recovery could not be saved: {e:#}")),
        }
    }
    /// Puts `path` first among the recent projects.
    fn remember_project(&mut self, path: PathBuf) {
        self.workflow.recent.retain(|p| p != &path);
        self.workflow.recent.insert(0, path);
        self.workflow.recent.truncate(app_data::RECENT_PROJECTS);
        if let Some(store) = &self.store {
            if let Err(e) = store.set_recent_projects(&self.workflow.recent) {
                self.error = Some(format!("Cannot save recent projects: {e:#}"));
            }
        }
    }
    pub fn save_project_file(&mut self, path: PathBuf) {
        match project::save(&path, &self.snapshot()) {
            Ok(()) => {
                self.workflow.project_path = Some(path.clone());
                self.status = format!("Saved project {}", path.display());
                self.remember_project(path);
            }
            Err(e) => self.error = Some(format!("Cannot save project: {e:#}")),
        }
    }
    pub fn open_project(&mut self, path: PathBuf) {
        match project::read(&path) {
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
                let message = format!(
                    "Project source is missing: {}. Edits and queue were \
                    recovered. Use Open File to relink the source.",
                    source.display()
                );
                self.workflow.source = Some(source.clone());
                self.apply_project(p);
                self.error = Some(message);
                return;
            }
            self.workflow.pending_project = Some(p.clone());
            if crtsim_media::MediaKind::of(source).is_moving() {
                self.load_video(source.clone(), p.frame, false);
            } else {
                self.load(source.clone());
            }
        } else {
            self.show_test_card();
            self.apply_project(p);
        }
    }
    pub fn apply_project(&mut self, p: Project) {
        self.video_options = p.options;
        self.queue.replace(p.queue);
        self.replace_config(p.config);
        self.status = "Project restored; queue paused".into();
    }
    pub fn workflow_ui(&mut self, ctx: &egui::Context) {
        self.video_export_window(ctx);
        self.tick_playback(ctx);
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
        self.queue_window(ctx);
        self.dispatch_queue();
        if self.workflow.last_save.elapsed() > Duration::from_secs(2) {
            self.workflow.last_save = Instant::now();
            self.save_session();
            self.save_tool_windows();
        }
        ctx.request_repaint_after(Duration::from_secs(2));
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
}
