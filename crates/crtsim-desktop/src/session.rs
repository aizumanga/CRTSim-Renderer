//! The session: the project the app saves every two seconds and offers to recover on the next
//! start, and the project files opened and saved during it.
use crate::{app_data, project, App, Dialog};
use anyhow::Result;
use app_data::Store;
use eframe::egui;
use project::Project;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

/// How often the session is saved.
const AUTOSAVE: Duration = Duration::from_secs(2);

pub struct Session {
    /// The project file open now, which Save project writes to.
    project_path: Option<PathBuf>,
    /// The projects opened or saved most recently, newest first.
    recent: Vec<PathBuf>,
    /// The last run's session, on offer until it is recovered or declined.
    recovery: Option<Project>,
    /// A project opened, waiting for its source to load before the rest of it is applied.
    restoring: Option<Project>,
    /// The session as last saved, so an unchanged one is not written again.
    last_saved: Option<Project>,
    last_save: Instant,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            project_path: None,
            recent: vec![],
            recovery: None,
            restoring: None,
            last_saved: None,
            last_save: Instant::now(),
        }
    }
}

impl Session {
    /// Reads what the last run left in `store`: the recent projects and, when `recover`, the
    /// session to offer. The error to show, if reading failed.
    pub fn load(&mut self, store: &Store, recover: bool) -> Option<String> {
        let mut error = None;
        match store.recent_projects() {
            Ok(paths) => self.recent = paths,
            Err(_) => error = Some("Could not read recent projects".into()),
        }
        if recover {
            match store.session() {
                Ok(session) => self.recovery = session,
                Err(e) => error = Some(format!("Could not recover the last session: {e:#}")),
            }
        }
        error
    }

    pub fn recovery_offered(&self) -> bool {
        self.recovery.is_some()
    }

    /// Takes the session on offer, to recover it or to decline it.
    pub fn take_recovery(&mut self) -> Option<Project> {
        self.recovery.take()
    }

    /// Keeps `project` until its source has loaded.
    pub fn restoring(&mut self, project: Project) {
        self.restoring = Some(project);
    }

    /// The project whose source has just loaded, or failed to, if one was waiting for it.
    pub fn source_loaded(&mut self) -> Option<Project> {
        self.restoring.take()
    }

    /// Whether it is time at `now` to save the session again.
    pub fn autosave_due(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_save) <= AUTOSAVE {
            return false;
        }
        self.last_save = now;
        true
    }

    /// Saves `session` to `store`, unless it is unchanged or saving would be early: while the
    /// last run's session is on offer, which saving would replace, or while `loading` or
    /// restoring a project leaves the settings half applied.
    pub fn save(&mut self, store: Option<&Store>, loading: bool, session: Project) -> Result<()> {
        let Some(store) = store else {
            return Ok(());
        };
        if self.recovery.is_some()
            || loading
            || self.restoring.is_some()
            || self.last_saved.as_ref() == Some(&session)
        {
            return Ok(());
        }
        store.set_session(&session)?;
        self.last_saved = Some(session);
        Ok(())
    }

    pub fn project_path(&self) -> Option<&PathBuf> {
        self.project_path.as_ref()
    }

    pub fn recent(&self) -> &[PathBuf] {
        &self.recent
    }

    /// `path` is the project open now, and the most recent one, which `store` remembers.
    pub fn opened(&mut self, path: PathBuf, store: Option<&Store>) -> Result<()> {
        self.recent.retain(|p| p != &path);
        self.recent.insert(0, path.clone());
        self.recent.truncate(app_data::RECENT_PROJECTS);
        self.project_path = Some(path);
        store.map_or(Ok(()), |store| store.set_recent_projects(&self.recent))
    }
}

impl App {
    pub fn load_session(&mut self, explicit_input: bool) {
        let Some(store) = self.app_data().cloned() else {
            return;
        };
        if let Some(error) = self.session.load(&store, !explicit_input) {
            self.error = Some(error);
        }
    }

    fn snapshot(&self) -> Project {
        Project {
            version: 1,
            source: self.source_path.clone(),
            frame: self.video_frame,
            config: self.config.clone(),
            options: self.video_options.clone(),
            queue: self.queue.items().to_vec(),
        }
    }

    pub fn save_session(&mut self) {
        let snapshot = self.snapshot();
        let store = self.app_data().cloned();
        let loading = self.work.is_loading();
        if let Err(e) = self.session.save(store.as_ref(), loading, snapshot) {
            self.error = Some(format!("Session recovery could not be saved: {e:#}"));
        }
    }

    /// Saves the session every two seconds, and with it where tool windows are.
    pub(crate) fn autosave(&mut self) {
        if self.session.autosave_due(Instant::now()) {
            self.save_session();
            self.save_tool_windows();
        }
    }

    /// `path` is the project open now; remembers it among the recent ones.
    fn opened_project(&mut self, path: PathBuf) {
        if let Err(e) = self.session.opened(path, self.store.as_ref()) {
            self.error = Some(format!("Cannot save recent projects: {e:#}"));
        }
    }

    pub fn save_project_file(&mut self, path: PathBuf) {
        match project::save(&path, &self.snapshot()) {
            Ok(()) => {
                self.status = format!("Saved project {}", path.display());
                self.opened_project(path);
            }
            Err(e) => self.error = Some(format!("Cannot save project: {e:#}")),
        }
    }

    pub fn open_project(&mut self, path: PathBuf) {
        match project::read(&path) {
            Ok(p) => {
                self.restore_project(p);
                self.opened_project(path);
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
                self.source_path = Some(source.clone());
                self.apply_project(p);
                self.error = Some(message);
                return;
            }
            self.session.restoring(p.clone());
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

    pub(crate) fn recovery_window(&mut self, ctx: &egui::Context) {
        if !self.session.recovery_offered() || self.show_welcome {
            return;
        }
        let mut restore = false;
        let mut discard = false;
        egui::Window::new("Recover last session?")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Restore the last source, edits, video options and paused export queue.");
                ui.horizontal(|ui| {
                    restore = ui.button("Restore session").clicked();
                    discard = ui.button("Start fresh").clicked();
                });
            });
        if restore {
            if let Some(p) = self.session.take_recovery() {
                self.restore_project(p);
            }
        }
        if discard {
            self.session.take_recovery();
            self.save_session();
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
                    self.session.project_path().is_some(),
                    egui::Button::new("Save project"),
                )
                .clicked()
            {
                if let Some(path) = self.session.project_path().cloned() {
                    self.save_project_file(path);
                }
                ui.close_menu();
            }
            ui.separator();
            ui.label("Recent projects");
            for path in self.session.recent().to_vec() {
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

    fn project(frame: u64) -> Project {
        Project {
            version: 1,
            source: None,
            frame,
            config: crtsim_core::config::Config::general(),
            options: Default::default(),
            queue: vec![],
        }
    }

    #[test]
    fn the_session_is_saved_when_it_changes_and_never_half_applied_or_over_one_on_offer() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_folder(dir.path());
        let mut session = Session::default();
        session.save(None, false, project(1)).unwrap();
        assert!(
            store.session().unwrap().is_none(),
            "no app data, as in a smoke run"
        );
        session.save(Some(&store), true, project(1)).unwrap();
        assert!(store.session().unwrap().is_none(), "not while loading");
        session.restoring(project(2));
        session.save(Some(&store), false, project(1)).unwrap();
        assert!(store.session().unwrap().is_none(), "not while restoring");
        assert_eq!(session.source_loaded(), Some(project(2)));
        session.save(Some(&store), false, project(1)).unwrap();
        assert_eq!(store.session().unwrap(), Some(project(1)));
        // The next start offers it, and saves nothing over it until it is taken.
        let mut next = Session::default();
        assert_eq!(next.load(&store, true), None);
        assert!(next.recovery_offered());
        next.save(Some(&store), false, project(3)).unwrap();
        assert_eq!(store.session().unwrap(), Some(project(1)));
        assert_eq!(next.take_recovery(), Some(project(1)));
        next.save(Some(&store), false, project(3)).unwrap();
        assert_eq!(store.session().unwrap(), Some(project(3)));
        // An unchanged session is not written again.
        std::fs::remove_file(dir.path().join("session-v1.crtsim")).unwrap();
        next.save(Some(&store), false, project(3)).unwrap();
        assert!(store.session().unwrap().is_none());
        // Opening with a file skips recovery.
        let mut explicit = Session::default();
        next.save(Some(&store), false, project(4)).unwrap();
        assert_eq!(explicit.load(&store, false), None);
        assert!(!explicit.recovery_offered());
    }

    #[test]
    fn opened_projects_come_first_once_each_and_are_remembered() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::in_folder(dir.path());
        let mut session = Session::default();
        for n in 0..12 {
            session
                .opened(format!("{n}.crtsim").into(), Some(&store))
                .unwrap();
        }
        session.opened("5.crtsim".into(), Some(&store)).unwrap();
        assert_eq!(session.project_path(), Some(&"5.crtsim".into()));
        let expected: Vec<PathBuf> = ["5", "11", "10", "9", "8", "7", "6", "4", "3", "2"]
            .map(|n| format!("{n}.crtsim").into())
            .into();
        assert_eq!(session.recent(), expected);
        let mut next = Session::default();
        next.load(&store, false);
        assert_eq!(next.recent(), expected);
    }

    #[test]
    fn the_session_is_saved_every_two_seconds() {
        let mut session = Session::default();
        let start = session.last_save;
        assert!(!session.autosave_due(start + AUTOSAVE));
        assert!(session.autosave_due(start + AUTOSAVE + Duration::from_millis(1)));
        assert!(!session.autosave_due(start + AUTOSAVE * 2));
    }
}
