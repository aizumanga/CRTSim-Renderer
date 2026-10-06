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
                        c.set_lut(Some(Arc::new(lut)));
                        self.replace_config(c);
                        self.status = "LUT imported".into();
                    }
                    Err(e) => self.error = Some(format!("Cannot import LUT: {e:#}")),
                }
            }
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
            Dialog::LoadPreset => match files::load_preset(&path, self.source.input().dimensions())
            {
                Ok(c) => {
                    self.presets.name = file_stem(&path);
                    self.replace_config(c);
                    self.status = format!("Loaded preset {}", path.display());
                    self.error = None;
                }
                Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
            },
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
        let input = self.source.input().dimensions();
        match kind {
            Dialog::File => {
                // Videos and animations open with their frames; the worker tells them apart.
                let status = format!("Loading {name}…");
                let opening = self.source.open_bytes(name, bytes);
                self.start_opening(opening, status);
            }
            Dialog::Lut => {
                let lut = String::from_utf8(bytes)
                    .map_err(anyhow::Error::from)
                    .and_then(|text| crtsim_core::workflow::Lut::parse_cube(name, &text));
                match lut {
                    Ok(lut) => {
                        let mut c = self.config.clone();
                        c.set_lut(Some(Arc::new(lut)));
                        self.replace_config(c);
                        self.status = "LUT imported".into();
                    }
                    Err(e) => self.error = Some(format!("Cannot import LUT: {e:#}")),
                }
            }
            Dialog::LoadPreset | Dialog::ImportPreset => {
                match preset_in(kind, &name, &bytes, input) {
                    Ok((c, options)) => {
                        self.presets.name = file_stem(Path::new(&name));
                        self.replace_config(c);
                        if let Some(options) = options {
                            self.video_options = options;
                        }
                        self.status = format!("Loaded preset {name}");
                        self.error = None;
                    }
                    Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
                }
            }
            _ => {}
        }
    }

    /// Says that something the desktop app does is not in the web app yet.
    fn not_yet(&mut self, what: &str) {
        self.status = format!("{what} arrives in the web app in a later version");
    }
}

/// The preset in the file called `name` that was picked to load or import one: a JSON
/// preset, or the one a rendered PNG or video holds. A video's also holds the video export
/// settings it was made with.
#[cfg_attr(
    all(not(test), not(target_arch = "wasm32")),
    expect(dead_code, reason = "the desktop imports presets from files by path")
)]
fn preset_in(
    kind: Dialog,
    name: &str,
    bytes: &[u8],
    input: (u32, u32),
) -> anyhow::Result<(Config, Option<crtsim_media::Options>)> {
    let path = Path::new(name);
    match kind {
        Dialog::LoadPreset => files::preset_from_json(bytes, input).map(|c| (c, None)),
        _ if crtsim_media::MediaKind::of(path) == crtsim_media::MediaKind::Video => {
            let preset = crtsim_media::import_preset_from_bytes(path, bytes, input)?;
            Ok((preset.config, Some(preset.video_options)))
        }
        _ => files::preset_from_png(bytes, input).map(|c| (c, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crtsim_media::mux;

    /// A video's preset, picked as a browser hands it over, brings its settings and its video
    /// export settings.
    #[test]
    fn a_preset_imports_from_a_picked_video() {
        let config = Config {
            bloom: 0.5,
            ..Config::general()
        };
        let options = crtsim_media::Options {
            audio: crtsim_media::Audio::Mute,
            ..Default::default()
        };
        let preset = serde_json::json!({"version": 1, "config": config, "video_options": options});
        let video = mux::EncodedVideo {
            codec: "vp09.00.10.08".into(),
            description: None,
            size: (2, 2),
            packets: vec![mux::Packet {
                data: vec![0; 4],
                time: 0.,
                duration: 0.04,
                key: true,
            }],
        };
        let comment = format!("CRTSim-Renderer-Preset:{preset}");
        let file = mux::write(crtsim_media::Container::Webm, &video, None, Some(&comment));
        let file = file.unwrap();
        let (imported, options) =
            preset_in(Dialog::ImportPreset, "made.webm", &file, (256, 224)).unwrap();
        assert_eq!(imported, config);
        assert_eq!(options.unwrap().audio, crtsim_media::Audio::Mute);
        // A video without one says so.
        let plain = mux::write(crtsim_media::Container::Webm, &video, None, None).unwrap();
        let error = preset_in(Dialog::ImportPreset, "plain.webm", &plain, (256, 224));
        assert!(error.unwrap_err().to_string().contains("does not contain"));
    }
}
