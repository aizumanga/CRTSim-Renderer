//! Applying what the worker reports: loaded sources, previews, progress and finished exports.
use crate::*;

impl App {
    pub(crate) fn receive(&mut self, ctx: &egui::Context) {
        self.receive_dialogs();
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Progress(progress) => self.lane.progress(progress),
                Event::PresetImported(result) => {
                    self.lane.finished();
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
                    self.lane.finished();
                    let arrived = self.source.loaded(result);
                    if arrived.changed {
                        self.source_changed();
                    }
                    match arrived.opened {
                        Ok(status) => {
                            self.status = status.into();
                            self.error = None;
                        }
                        // Only a video frame can be cancelled.
                        Err(Failure::Cancelled) => {
                            self.status = "Video loading cancelled".into();
                            self.error = None;
                        }
                        Err(Failure::Failed(e)) => self.error = Some(format!("{e:#}")),
                    }
                    // A project waiting for its source applies whether that opened or not.
                    if let Some(project) = arrived.project {
                        self.apply_project(project);
                    }
                }
                Event::Preview { revision, result } => {
                    let shown = self.preview.returned(ctx, revision, result);
                    // An export's progress keeps the status line while it runs.
                    if let Some(shown) = shown.filter(|_| !self.lane.is_exporting()) {
                        self.status = shown;
                    }
                }
                Event::Thumbnail {
                    generation,
                    key,
                    result,
                } => self.thumbnail_ready(ctx, generation, key, result),
                Event::Exported(result) => {
                    let finished = self.lane.finished();
                    self.queue_finished(&result, finished.cancelled);
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
}
