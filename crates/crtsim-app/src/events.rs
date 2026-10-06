//! Applying what the worker reports: loaded sources, previews, progress and finished exports.
use crate::*;

impl App {
    pub(crate) fn receive(&mut self, ctx: &egui::Context) {
        self.receive_dialogs();
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
                            self.presets.name = file_stem(&imported.path);
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
                            self.error = Some(format!("Cannot import preset: {e:#}"))
                        }
                    }
                }
                Event::Loaded(result) => {
                    self.work = Work::Idle;
                    // Opening a project loads its source first, then restores the rest.
                    let project = self.session.source_loaded();
                    match result {
                        Ok(loaded) => {
                            self.show_loaded(loaded);
                            self.error = None;
                            if let Some(project) = project {
                                self.apply_project(project);
                            }
                        }
                        // Only a video frame can be cancelled.
                        Err(Failure::Cancelled) => {
                            self.status = "Video loading cancelled".into();
                            self.error = None;
                        }
                        Err(Failure::Failed(e)) => self.error = Some(format!("{e:#}")),
                    }
                }
                Event::Preview { revision, result } => {
                    let shown = self.preview.returned(ctx, revision, result);
                    // An export's progress keeps the status line while it runs.
                    if let Some(shown) = shown.filter(|_| !self.work.is_exporting()) {
                        self.status = shown;
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
                        Err(Failure::Failed(e)) => self.error = Some(format!("{e:#}")),
                    }
                }
            }
        }
    }

    /// Makes a loaded image, or frame of a video, the source being edited.
    fn show_loaded(&mut self, loaded: worker::Loaded) {
        let status = match &loaded.timeline {
            Some(_) => "Video frame loaded",
            None => "Image loaded",
        };
        let drawn = loaded
            .timeline
            .as_ref()
            .is_some_and(timeline::Timeline::is_video_test_card);
        self.set_source(
            (!drawn).then_some(loaded.path),
            loaded.name,
            loaded.timeline,
            loaded.image,
            &loaded.thumbnail,
        );
        self.status = status.into();
    }
}
