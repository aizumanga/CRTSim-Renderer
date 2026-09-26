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
                Event::Loaded(result) => {
                    self.work = Work::Idle;
                    // Opening a project loads its source first, then restores the rest.
                    let project = self.workflow.pending_project.take();
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
                        Err(Failure::Failed(e)) => self.error = Some(e),
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

    /// Makes a loaded image, or frame of a video, the source being edited.
    fn show_loaded(&mut self, loaded: worker::Loaded) {
        let status = match &loaded.video {
            Some(at) => {
                self.workflow.play_time = at.frame as f64 / at.video.fps;
                self.video_frame = at.frame;
                self.selected_frame = at.frame;
                self.video_frames = at.frames;
                "Video frame loaded"
            }
            None => "Image loaded",
        };
        self.set_source(
            Some(loaded.path),
            loaded.name,
            loaded.video.map(|at| at.video),
            loaded.image,
            &loaded.thumbnail,
        );
        self.status = status.into();
    }
}
