use crate::{batch, files, App, Dialog};
use anyhow::{ensure, Result};
use crtsim_core::config::Config;
use crtsim_media::Options;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MAX_PROJECT: u64 = 64 * 1024 * 1024;
pub const PROJECT_EXTENSION: &str = "crtsim";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    version: u32,
    source: Option<PathBuf>,
    frame: u64,
    config: Config,
    options: Options,
    queue: Vec<batch::Item>,
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
    ensure!(
        p.queue.len() <= batch::MAX_JOBS,
        "Queue exceeds {} jobs",
        batch::MAX_JOBS
    );
    // Paths saved relative to the project are relative to where it is.
    let root = path.parent().unwrap_or_else(|| Path::new("."));
    if let Some(source) = p.source.as_mut().filter(|source| source.is_relative()) {
        *source = root.join(&*source);
    }
    for item in &mut p.queue {
        item.restore(root)?;
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
        let Some(root) = self.app_data().map(|store| store.root.clone()) else {
            return;
        };
        let recent = root.join("recent-projects-v1.json");
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
        let recovery = root.join("session-v1.crtsim");
        if !explicit_input && recovery.exists() {
            match read_project(&recovery) {
                Ok(project) => self.workflow.recovery = Some(project),
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
        let Some(path) = self
            .app_data()
            .map(|store| store.root.join("session-v1.crtsim"))
        else {
            return;
        };
        match save_project(&path, &p) {
            Ok(()) => self.workflow.last_saved = Some(p),
            Err(e) => self.error = Some(format!("Session recovery could not be saved: {e:#}")),
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
        self.receive_batch_files();
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

#[cfg(test)]
mod tests {
    use super::*;
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
            queue: vec![batch::Item {
                source: "input.png".into(),
                output: "output.png".into(),
                config: c,
                options: Options::default(),
                status: batch::Status::Running,
            }],
        };
        save_project(&path, &p).unwrap();
        // Projects and sessions saved before keep opening: a job is saved as it always was.
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains(r#""output": "output.png""#) && saved.contains(r#""status": "Running""#)
        );
        let read = read_project(&path).unwrap();
        assert_eq!(read.frame, 7);
        assert_eq!(read.source, Some(dir.path().join("input.png")));
        assert_eq!(read.queue[0].status, batch::Status::Pending);
        assert_eq!(read.queue[0].output, dir.path().join("output.png"));
        let mut bad = p;
        bad.version = 99;
        save_project(&path, &bad).unwrap();
        assert!(read_project(&path).is_err());
    }
}
