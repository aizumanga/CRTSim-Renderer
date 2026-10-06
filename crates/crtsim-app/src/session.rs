//! The session: the project the app saves every two seconds and offers to recover on the next
//! start, and the project files opened and saved during it.
use crate::{app_data, project, source, App, Dialog};
use anyhow::Result;
use app_data::Store;
use eframe::egui;
use project::Project;
use std::{path::PathBuf, time::Duration};
use web_time::Instant;

/// How often the session is saved.
const AUTOSAVE: Duration = Duration::from_secs(2);

pub struct Session {
    /// The project file open now, which Save project writes to.
    project_path: Option<PathBuf>,
    /// The projects opened or saved most recently, newest first.
    recent: Vec<PathBuf>,
    /// The last run's session, on offer until it is recovered or declined.
    recovery: Option<Project>,
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

    /// Whether it is time at `now` to save the session again.
    pub fn autosave_due(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_save) <= AUTOSAVE {
            return false;
        }
        self.last_save = now;
        true
    }

    /// Saves `session` to `store`, unless it is unchanged or saving would be early: while the
    /// last run's session is on offer, which saving would replace, or while `loading`, or a
    /// project's source loading, leaves the settings half applied.
    pub fn save(&mut self, store: Option<&Store>, loading: bool, session: Project) -> Result<()> {
        let Some(store) = store else {
            return Ok(());
        };
        if self.recovery.is_some() || loading || self.last_saved.as_ref() == Some(&session) {
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
        let (source, built_in, frame) = self.source.saved();
        Project {
            version: 1,
            source,
            built_in,
            frame,
            config: self.config.clone(),
            options: self.video_options.clone(),
            queue: self.queue.items().to_vec(),
        }
    }

    pub fn save_session(&mut self) {
        let snapshot = self.snapshot();
        let store = self.app_data().cloned();
        // Nor while a project waits for its source, its settings not yet applied.
        let loading = self.lane.is_loading() || self.source.restoring();
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
        let status = p.source.as_ref().map_or_else(String::new, |source| {
            format!("Loading {}…", source.display())
        });
        match self.source.restore(p) {
            source::Restoring::Open(opening) => self.start_opening(opening, status),
            source::Restoring::Apply { project, error } => {
                self.source_changed();
                self.apply_project(project);
                // Any error already showing stays, unless this one replaces it.
                if error.is_some() {
                    self.error = error;
                }
            }
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
                ui.close();
            }
            if ui.button("Save project as…").clicked() {
                self.dialog(Dialog::SaveProject, ctx);
                ui.close();
            }
            if ui
                .add_enabled(
                    self.session.project_path().is_some(),
                    egui::Button::new("Save project")
                        .shortcut_text(crate::shortcut(ctx, crate::SHORTCUT_SAVE)),
                )
                .clicked()
            {
                if let Some(path) = self.session.project_path().cloned() {
                    self.save_project_file(path);
                }
                ui.close();
            }
            ui.separator();
            ui.label("Recent projects");
            for path in self.session.recent().to_vec() {
                if ui.button(path.display().to_string()).clicked() {
                    self.open_project(path);
                    ui.close();
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
            built_in: project::BuiltIn::TestCard,
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

    /// The video test card opens without a file, plays, and a project saved with it opens it
    /// again at the same frame rather than looking for a file.
    #[test]
    #[ignore = "requires a Vulkan adapter"]
    fn the_video_test_card_plays_and_a_project_opens_it_again() {
        use crate::{worker, Smoke};
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let open = || {
            App::new(
                &ctx,
                worker::Gpu::Own(wgpu::Backends::VULKAN),
                Ok(Store::temporary()),
                None,
                Some(Smoke::new("unused-smoke.png".into())),
            )
        };
        let started = Instant::now();
        let waited = |app: &mut App| {
            app.receive(&ctx);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert!(started.elapsed() < Duration::from_secs(180), "stalled");
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut app = open();
        app.show_video_test_card(540);
        while !app.lane.is_idle() {
            waited(&mut app);
        }
        assert_eq!(app.source.name(), "Video test card");
        assert_eq!(*app.source.kind(), crate::source::Kind::VideoTestCard);
        assert_eq!(**app.source.input(), crtsim_core::test_clip::frame(540));
        app.config.output = "320x240".into();
        app.config.warmup = 3;
        let project = dir.path().join("clip.crtsim");
        app.save_project_file(project.clone());
        // The last second plays to the end.
        app.start_playback();
        while app.playback.is_some() {
            app.tick_playback(&ctx);
            waited(&mut app);
        }
        assert_eq!(app.status, "Playback finished");
        assert!(app.source.timeline.as_ref().unwrap().shown > 580);

        let mut reopened = open();
        reopened.open_project(project);
        while reopened.source.timeline.is_none() || !reopened.lane.is_idle() {
            waited(&mut reopened);
        }
        assert_eq!(reopened.source.name(), "Video test card");
        assert_eq!(reopened.source.timeline.as_ref().unwrap().shown, 540);
        assert_eq!(reopened.config.output, "320x240");
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
