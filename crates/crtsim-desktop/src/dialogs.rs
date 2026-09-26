//! Native file dialogs: what each one offers, and what happens with the file chosen.
use crate::export_ui::ExportFormat;
use crate::worker::Export;
use crate::*;

#[derive(Clone, Copy)]
pub(crate) enum Dialog {
    OpenProject,
    SaveProject,
    Lut,
    File,
    ExportVideo,
    ImportPreset,
    LoadPreset,
    SavePreset,
    Export,
}

/// What a file dialog offers: the files it filters for and, when it saves, the name it suggests.
struct Chooser {
    filter: &'static str,
    extensions: Vec<&'static str>,
    /// For a save dialog, the suggested file name before its extension, the filter's only one.
    save_as: Option<&'static str>,
}

impl Dialog {
    fn chooser(self, export: ExportFormat) -> Chooser {
        let (filter, extensions, save_as) = match self {
            Self::OpenProject => ("CRT project", vec![workflow::PROJECT_EXTENSION], None),
            Self::SaveProject => (
                "CRT project",
                vec![workflow::PROJECT_EXTENSION],
                Some("project"),
            ),
            Self::Lut => ("3D color LUT", vec!["cube"], None),
            Self::File => ("Images and videos", files::media_extensions(), None),
            Self::ExportVideo => (export.filter(), vec![export.extension()], Some("rendered")),
            Self::ImportPreset => (
                "Rendered image/video",
                [&["png"], crtsim_media::VIDEO_EXTENSIONS].concat(),
                None,
            ),
            Self::LoadPreset => ("CRT preset", vec!["json"], None),
            Self::SavePreset => ("CRT preset", vec!["json"], Some("my-crt")),
            Self::Export => ("PNG image", vec!["png"], Some("rendered")),
        };
        Chooser {
            filter,
            extensions,
            save_as,
        }
    }
}

/// Native dialogs confirm the path they return. Where it lacks the extension and one is appended,
/// the actual destination is confirmed too, rather than silently replacing another file.
fn with_extension_confirmed(mut path: PathBuf, extension: &str) -> Option<PathBuf> {
    if path.extension().is_some() {
        return Some(path);
    }
    path.set_extension(extension);
    let replace = !path.exists()
        || rfd::MessageDialog::new()
            .set_title("Replace file?")
            .set_description(format!("Replace {}?", path.display()))
            .set_buttons(rfd::MessageButtons::YesNo)
            .show()
            == rfd::MessageDialogResult::Yes;
    replace.then_some(path)
}

impl App {
    /// Opens `kind` on a thread of its own, since native dialogs block. Edits are frozen
    /// while it is open, so its answer is applied to the settings that were on screen.
    pub(crate) fn dialog(&mut self, kind: Dialog, ctx: &egui::Context) {
        self.stop_playback();
        self.dialog_open = true;
        let send = self.dialog_send.clone();
        let ctx = ctx.clone();
        let export = self.workflow.export_format;
        std::thread::spawn(move || {
            let chooser = kind.chooser(export);
            let dialog = rfd::FileDialog::new().add_filter(chooser.filter, &chooser.extensions);
            let path = match chooser.save_as {
                None => dialog.pick_file(),
                Some(name) => {
                    let extension = chooser.extensions[0];
                    dialog
                        .set_file_name(format!("{name}.{extension}"))
                        .save_file()
                        .and_then(|path| with_extension_confirmed(path, extension))
                }
            };
            let _ = send.send((kind, path));
            ctx.request_repaint();
        });
    }

    /// Applies the dialogs that have closed since the last frame.
    pub(crate) fn receive_dialogs(&mut self) {
        while let Ok((kind, path)) = self.dialog_receive.try_recv() {
            self.dialog_open = false;
            if let Some(path) = path {
                self.chosen(kind, path);
            }
        }
    }

    fn chosen(&mut self, kind: Dialog, path: PathBuf) {
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
            Dialog::ExportVideo => self.export_video(path),
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
            Dialog::LoadPreset => match files::load_preset(&path, self.input.dimensions()) {
                Ok(c) => {
                    self.gallery_name = file_stem(&path);
                    self.replace_config(c);
                    self.status = format!("Loaded preset {}", path.display());
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
            },
            Dialog::SavePreset => {
                match self
                    .config
                    .validate_for(self.input.dimensions())
                    .and_then(|()| files::save_preset(&path, &self.config))
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

    /// The video on screen, exported as the export dialog chose, to `path`.
    fn export_video(&mut self, path: PathBuf) {
        let format = self.workflow.export_format;
        let extension = format.extension();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(extension))
        {
            self.error = Some(format!(
                "Choose a .{extension} filename for the selected format."
            ));
            return;
        }
        let Some(video) = self.video.clone() else {
            return;
        };
        let config = self.config.clone();
        let (export, status) = match format {
            ExportFormat::Video(_) => (
                Export::Video {
                    video,
                    options: self.video_options.clone(),
                    config,
                },
                "Exporting video…".into(),
            ),
            ExportFormat::Animation(animation) => (
                Export::Animation {
                    video,
                    options: self.animation_options.clone(),
                    config,
                },
                format!("Exporting {}…", animation.name()),
            ),
        };
        self.start_export(export, path, status, Some("Queued for video export"));
    }
}
