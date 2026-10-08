//! File dialogs: what each one offers, and what happens with what is chosen. Every dialog
//! answers on one channel. On the desktop each opens on a thread of its own, since native
//! dialogs block; in a browser, files are picked as a name and bytes, and saves download.
use crate::export_ui::ExportFormat;
use crate::worker::Export;
use crate::*;
#[cfg(not(target_arch = "wasm32"))]
use std::panic::AssertUnwindSafe;

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
    RetroArch,
}

/// What a dialog is asked for.
pub(crate) enum Request {
    /// One file, as `Dialog` says.
    File(Dialog),
    /// Files for new batch jobs, then the folder their exports go to. The jobs keep these
    /// settings, the ones on screen when the dialog opened.
    Batch(Box<batch::Settings>),
}

/// What a dialog answered.
pub(crate) enum Answer {
    Cancelled,
    File(Dialog, PathBuf),
    Batch {
        settings: Box<batch::Settings>,
        sources: Vec<PathBuf>,
        folder: PathBuf,
    },
    /// The dialog closed without answering, as when it fails to open.
    Failed,
    /// A file a browser handed over: its name and contents, never a path.
    #[cfg(target_arch = "wasm32")]
    Picked {
        kind: Dialog,
        name: String,
        bytes: Vec<u8>,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl Request {
    /// Shows the dialog and waits for its answer.
    fn ask(self, export: ExportFormat) -> Answer {
        match self {
            Self::File(kind) => {
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
                path.map_or(Answer::Cancelled, |path| Answer::File(kind, path))
            }
            Self::Batch(settings) => {
                let Some(sources) = rfd::FileDialog::new()
                    .add_filter("Images and videos", &files::media_extensions())
                    .pick_files()
                else {
                    return Answer::Cancelled;
                };
                // The destination is asked for only once there are files to send to it.
                match rfd::FileDialog::new()
                    .set_title("Batch export destination")
                    .pick_folder()
                {
                    Some(folder) => Answer::Batch {
                        settings,
                        sources,
                        folder,
                    },
                    None => Answer::Cancelled,
                }
            }
        }
    }
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
            Self::OpenProject => ("CRT project", vec![project::EXTENSION], None),
            Self::SaveProject => ("CRT project", vec![project::EXTENSION], Some("project")),
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
            Self::RetroArch => ("RetroArch shader", vec!["zip"], Some("my-crt-retroarch")),
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
#[cfg(not(target_arch = "wasm32"))]
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
    /// Opens a dialog for one file of `kind`.
    pub(crate) fn dialog(&mut self, kind: Dialog, ctx: &egui::Context) {
        self.ask(Request::File(kind), ctx);
    }

    /// Opens the dialog `request` asks for, on a thread of its own, since native dialogs block.
    /// Edits are frozen while it is open, so its answer is applied to the settings that were on
    /// screen.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn ask(&mut self, request: Request, ctx: &egui::Context) {
        self.stop_playback();
        self.dialog_open = true;
        let send = self.dialog_send.clone();
        let ctx = ctx.clone();
        let export = self.export_format;
        std::thread::spawn(move || {
            // A dialog that fails must still answer, or the interface stays frozen.
            let answer = std::panic::catch_unwind(AssertUnwindSafe(|| request.ask(export)))
                .unwrap_or(Answer::Failed);
            let _ = send.send(answer);
            ctx.request_repaint();
        });
    }

    /// Applies the dialogs that have closed since the last frame.
    pub(crate) fn receive_dialogs(&mut self) {
        while let Ok(answer) = self.dialog_receive.try_recv() {
            self.dialog_open = false;
            match answer {
                Answer::Cancelled => {}
                Answer::File(kind, path) => self.chosen(kind, path),
                Answer::Batch {
                    settings,
                    sources,
                    folder,
                } => self.add_batch_jobs(&settings, sources, &folder),
                Answer::Failed => self.error = Some("File chooser closed unexpectedly".into()),
                #[cfg(target_arch = "wasm32")]
                Answer::Picked { kind, name, bytes } => self.picked(kind, name, bytes),
            }
        }
    }

    fn chosen(&mut self, kind: Dialog, path: PathBuf) {
        match kind {
            Dialog::OpenProject => self.open_project(path),
            Dialog::SaveProject => self.save_project_file(path),
            Dialog::Lut => self.bring_file(incoming::Purpose::Lut, incoming::File::Path(path)),
            Dialog::File => self.load(path),
            Dialog::ExportVideo => self.export_video(path),
            Dialog::ImportPreset => {
                let job = Job::ImportPreset {
                    path,
                    input: self.source.input().dimensions(),
                    cancel: Arc::new(AtomicBool::new(false)),
                };
                self.start(job, "Reading preset metadata…".into());
            }
            Dialog::LoadPreset => {
                self.bring_file(incoming::Purpose::LoadPreset, incoming::File::Path(path));
            }
            Dialog::SavePreset => {
                match self
                    .config
                    .validate_for(self.source.input().dimensions())
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
            Dialog::RetroArch => match files::save_retroarch(&path, &self.config) {
                Ok(()) => {
                    self.status = format!("Exported RetroArch shader {}", path.display());
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("Cannot export RetroArch shader: {e:#}")),
            },
        }
    }

    /// The video on screen, exported as the export dialog chose, to `path`.
    fn export_video(&mut self, path: PathBuf) {
        let format = self.export_format;
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
        let Some(video) = self.source.timeline.as_ref().map(|t| t.video.clone()) else {
            return;
        };
        let config = self.config.clone();
        // The subtitle drawn into the picture is the video's to draw as it is decoded.
        let video = video.with_subtitle(self.export_subtitles.burn);
        let (export, status) = match format {
            ExportFormat::Video(_) => (
                Export::Video {
                    video,
                    options: crtsim_media::Options {
                        keep_subtitles: self.export_subtitles.keep.clone(),
                        ..self.video_options.clone()
                    },
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

/// Video files the web app opens: those `crtsim_media::demux` reads.
#[cfg(target_arch = "wasm32")]
const WEB_VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv"];

/// The browser's side of the dialogs. Opening shows the browser's own file picker, which gives a
/// name and bytes; saving shows nothing, since the file is downloaded under the suggested name.
#[cfg(target_arch = "wasm32")]
impl App {
    /// Opens the dialog `request` asks for. Edits are frozen while a picker is open, so its
    /// answer is applied to the settings that were on screen.
    pub(crate) fn ask(&mut self, request: Request, ctx: &egui::Context) {
        self.stop_playback();
        let kind = match request {
            Request::File(kind) => kind,
            Request::Batch(_) => return self.not_yet("Batch export"),
        };
        let chooser = kind.chooser(self.export_format);
        match kind {
            Dialog::OpenProject | Dialog::SaveProject => return self.not_yet("Projects"),
            _ => {}
        }
        if let Some(name) = chooser.save_as {
            let file = format!("{name}.{}", chooser.extensions[0]);
            return self.chosen(kind, PathBuf::from(file));
        }
        // The videos a browser opens are those its container reader knows.
        let extensions = match kind {
            Dialog::File => [crtsim_core::input::IMAGE_EXTENSIONS, WEB_VIDEO_EXTENSIONS].concat(),
            Dialog::ImportPreset => {
                [crtsim_core::input::IMAGE_EXTENSIONS, WEB_VIDEO_EXTENSIONS].concat()
            }
            _ => chooser.extensions,
        };
        self.dialog_open = true;
        let send = self.dialog_send.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let answer = match crate::web::pick(&extensions).await {
                Ok(Some((name, bytes))) => Answer::Picked { kind, name, bytes },
                Ok(None) => Answer::Cancelled,
                Err(e) => {
                    web_sys::console::error_1(&format!("{e:#}").into());
                    Answer::Failed
                }
            };
            let _ = send.send(answer);
            ctx.request_repaint();
        });
    }

    /// A file dropped on the page, read as the picker would have given it.
    pub(crate) fn dropped(&mut self, file: Arc<dyn egui::DroppedFile + Send + Sync>) {
        let send = self.dialog_send.clone();
        let ctx = self.ui_context.clone();
        self.dialog_open = true;
        wasm_bindgen_futures::spawn_local(async move {
            let answer = match file.bytes_async().await {
                Ok(bytes) => Answer::Picked {
                    kind: Dialog::File,
                    name: file.path().to_string_lossy().into_owned(),
                    bytes,
                },
                Err(_) => Answer::Failed,
            };
            let _ = send.send(answer);
            ctx.request_repaint();
        });
    }

    /// Applies a file the browser handed over.
    fn picked(&mut self, kind: Dialog, name: String, bytes: Vec<u8>) {
        match kind {
            Dialog::File => {
                // Videos and animations open with their frames; the worker tells them apart.
                let status = format!("Loading {name}…");
                let opening = self.source.open_bytes(name, bytes);
                self.start_opening(opening, status);
            }
            Dialog::Lut | Dialog::LoadPreset | Dialog::ImportPreset => {
                let purpose = match kind {
                    Dialog::Lut => incoming::Purpose::Lut,
                    Dialog::LoadPreset => incoming::Purpose::LoadPreset,
                    _ => incoming::Purpose::ImportPreset,
                };
                self.bring_file(purpose, incoming::File::Bytes { name, bytes });
            }
            _ => {}
        }
    }

    /// Says that something the desktop app does is not in the web app yet.
    fn not_yet(&mut self, what: &str) {
        self.status = format!("{what} arrives in the web app in a later version");
    }
}

impl App {
    /// Brings in `file` for `purpose`, whichever way it came.
    fn bring_file(&mut self, purpose: incoming::Purpose, file: incoming::File) {
        let input = self.source.input().dimensions();
        let brought = incoming::bring(purpose, &file, &self.config, input);
        self.bring_in(brought);
    }

    /// Applies what a file brought, or says why it brought nothing.
    pub(crate) fn bring_in(&mut self, brought: Result<incoming::Brought, String>) {
        match brought {
            Ok(brought) => {
                if let Some(name) = brought.preset_name {
                    self.presets.name = name;
                }
                self.replace_config(brought.config);
                if let Some(options) = brought.options {
                    self.video_options = options;
                }
                self.status = brought.status;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}
