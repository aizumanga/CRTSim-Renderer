//! Applying what the worker reports: loaded sources, previews, progress and finished exports.
use crate::*;

impl App {
    pub(crate) fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((kind, path)) = self.dialog_receive.try_recv() {
            self.dialog_open = false;
            if let Some(path) = path {
                match kind {
                    Dialog::OpenProject => self.open_project(path),
                    Dialog::SaveProject => self.save_project_file(path),
                    Dialog::Lut => {
                        let result = (|| -> anyhow::Result<_> {
                            anyhow::ensure!(
                                path.metadata()?.len() <= 16 * 1024 * 1024,
                                "LUT exceeds 16 MB"
                            );
                            crtsim_core::workflow::Lut::parse_cube(
                                file_name(&path),
                                &std::fs::read_to_string(&path)?,
                            )
                        })();
                        match result {
                            Ok(lut) => {
                                let mut c = self.config.clone();
                                c.lut = Some(Arc::new(lut));
                                c.palette = None;
                                self.replace_config(c);
                                self.status = "LUT imported".into();
                            }
                            Err(e) => self.error = Some(format!("Cannot import LUT: {e:#}")),
                        }
                    }
                    Dialog::File => self.load(path),
                    Dialog::ExportVideo => {
                        let container = self.workflow.export_container;
                        if crtsim_media::Container::of(&path) != Some(container) {
                            self.error = Some(format!(
                                "Choose a .{} filename for the selected format.",
                                container.extension()
                            ));
                            continue;
                        }
                        if let Some(video) = self.video.clone() {
                            let cancel = Arc::new(AtomicBool::new(false));
                            self.work = Work::Exporting {
                                cancel: cancel.clone(),
                                progress: Some(RenderProgress {
                                    fraction: 0.,
                                    stage: "Queued for video export".into(),
                                }),
                            };
                            self.status = "Exporting video…".into();
                            self.send(Job::ExportVideo {
                                video,
                                options: self.video_options.clone(),
                                config: self.config.clone(),
                                path,
                                cancel,
                            });
                        }
                    }
                    Dialog::ImportPreset => {
                        let cancel = Arc::new(AtomicBool::new(false));
                        self.work = Work::Loading(Some(cancel.clone()));
                        self.status = "Reading preset metadata…".into();
                        self.send(Job::ImportPreset {
                            path,
                            input: self.input.dimensions(),
                            cancel,
                        });
                    }
                    Dialog::LoadPreset => {
                        match files::load_preset(&path, self.input.dimensions()) {
                            Ok(c) => {
                                self.gallery_name = file_stem(&path);
                                self.replace_config(c);
                                self.status = format!("Loaded preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
                        }
                    }
                    Dialog::SavePreset => {
                        match model::preview_config(&self.config, self.input.dimensions(), None)
                            .and_then(|_| files::save_preset(&path, &self.config))
                        {
                            Ok(()) => {
                                self.status = format!("Saved preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot save preset: {e:#}")),
                        }
                    }
                    Dialog::Export => self.export(path),
                }
            }
        }
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
                Event::VideoLoaded(result) => {
                    self.work = Work::Idle;
                    match result {
                        Ok(loaded) => {
                            self.workflow.play_time = loaded.frame as f64 / loaded.video.fps;
                            self.video_frame = loaded.frame;
                            self.selected_frame = loaded.frame;
                            self.video_frames = loaded.frames;
                            let path = loaded.video.path.clone();
                            let name = file_name(&path);
                            self.set_source(
                                Some(path),
                                name,
                                Some(loaded.video),
                                loaded.image,
                                &loaded.thumbnail,
                            );
                            self.error = None;
                            self.status = "Video frame loaded".into();
                            if let Some(p) = self.workflow.pending_project.take() {
                                self.apply_project(p);
                            }
                        }
                        Err(Failure::Cancelled) => {
                            self.status = "Video loading cancelled".into();
                            self.workflow.pending_project = None;
                            self.error = None;
                        }
                        Err(Failure::Failed(e)) => {
                            self.workflow.pending_project = None;
                            self.error = Some(e);
                        }
                    }
                }
                Event::Loaded(result) => {
                    self.work = Work::Idle;
                    match result {
                        Ok(loaded) => {
                            let name = file_name(&loaded.path);
                            let source = loaded.path.canonicalize().unwrap_or(loaded.path);
                            self.set_source(
                                Some(source),
                                name,
                                None,
                                loaded.image,
                                &loaded.thumbnail,
                            );
                            self.error = None;
                            self.status = "Image loaded".into();
                            if let Some(p) = self.workflow.pending_project.take() {
                                self.apply_project(p);
                            }
                        }
                        Err(e) => {
                            self.workflow.pending_project = None;
                            self.error = Some(e);
                        }
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
}
