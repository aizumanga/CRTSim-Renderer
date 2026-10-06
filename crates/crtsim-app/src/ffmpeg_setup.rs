//! FFmpeg setup: whether this computer can open and export video, which export formats its
//! FFmpeg can write, and how to install it where it is missing. The app never downloads FFmpeg
//! itself; it finds the one a person installed (see `crtsim_media::Tool::executable`).
use crate::*;
use crtsim_media::{AnimationFormat, Container, Encoder, Tool, ToolCheck};

/// The last check of the FFmpeg programs, and the window that shows it.
#[derive(Default)]
pub struct FfmpegSetup {
    pub open: bool,
    /// The last check, once one has finished.
    check: Option<ToolCheck>,
    /// A check running on a thread of its own: starting the programs can take a moment.
    checking: Option<mpsc::Receiver<ToolCheck>>,
}

impl FfmpegSetup {
    /// Checks the programs again, on a thread of its own. A browser runs no programs, so there
    /// it checks nothing.
    #[cfg(target_arch = "wasm32")]
    pub fn start_check(&mut self, _ctx: &egui::Context) {}

    /// Checks the programs again, on a thread of its own.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn start_check(&mut self, ctx: &egui::Context) {
        let (send, receive) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let check = ToolCheck::run(&Arc::new(AtomicBool::new(false)));
            let _ = send.send(check);
            ctx.request_repaint();
        });
        self.checking = Some(receive);
    }

    /// Takes the check that has just finished, if one has.
    fn receive(&mut self) {
        if let Some(check) = self.checking.as_ref().and_then(|r| r.try_recv().ok()) {
            self.check = Some(check);
            self.checking = None;
        }
    }

    /// Whether the last check found that FFmpeg or ffprobe does not start. Unknown until a
    /// check has finished, so nothing is refused while one runs.
    pub fn missing(&self) -> bool {
        self.check.as_ref().is_some_and(|check| !check.ready())
    }

    /// Whether the last check found FFmpeg running but without the encoder `name`.
    pub fn lacks(&self, name: &str) -> bool {
        self.check
            .as_ref()
            .is_some_and(|check| check.ffmpeg.is_ok() && !check.encodes(name))
    }
}

/// The export formats and the encoders they need, as the setup window lists them.
fn formats() -> [(&'static str, &'static str); 6] {
    let video = |container| {
        Encoder::Software
            .codec(container)
            .expect("software encodes every container")
    };
    [
        ("MP4 and MKV video", video(Container::Mp4)),
        ("WebM video", video(Container::Webm)),
        ("GIF", AnimationFormat::Gif.encoder()),
        ("Animated WebP", AnimationFormat::Webp.encoder()),
        ("AAC audio, for MP4 and MKV", "aac"),
        ("Opus audio, for WebM", "libopus"),
    ]
}

/// How to install FFmpeg on this computer.
struct Install {
    /// A command that installs it, to copy into a terminal.
    command: Option<&'static str>,
    /// Anything to know before running it.
    note: Option<&'static str>,
    /// Where to read more or download it by hand.
    link: (&'static str, &'static str),
}

fn install() -> Install {
    if cfg!(windows) {
        Install {
            command: Some("winget install --id Gyan.FFmpeg -e"),
            note: Some("Run it in PowerShell or Terminal, then choose Check again."),
            link: (
                "FFmpeg builds for Windows",
                "https://www.gyan.dev/ffmpeg/builds/",
            ),
        }
    } else if cfg!(target_os = "macos") {
        Install {
            command: Some("brew install ffmpeg"),
            note: Some("Needs Homebrew, from brew.sh."),
            link: ("Homebrew", "https://brew.sh"),
        }
    } else {
        let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
        let (command, note) = linux_install(&release);
        Install {
            command,
            note,
            link: ("FFmpeg downloads", "https://ffmpeg.org/download.html"),
        }
    }
}

/// The install command for the Linux distribution `/etc/os-release` describes, by its ID or
/// the ones it is like.
fn linux_install(os_release: &str) -> (Option<&'static str>, Option<&'static str>) {
    let ids: Vec<&str> = os_release
        .lines()
        .filter_map(|line| {
            line.strip_prefix("ID=")
                .or_else(|| line.strip_prefix("ID_LIKE="))
        })
        .flat_map(|value| value.trim_matches('"').split_whitespace())
        .collect();
    let is = |id: &str| ids.contains(&id);
    if is("arch") {
        (Some("sudo pacman -S ffmpeg"), None)
    } else if is("debian") || is("ubuntu") {
        (Some("sudo apt install ffmpeg"), None)
    } else if is("fedora") {
        (
            Some("sudo dnf install ffmpeg"),
            Some(
                "Fedora's own ffmpeg-free cannot write H.264 (MP4 and MKV). Enable RPM Fusion \
                 first for the full FFmpeg.",
            ),
        )
    } else if is("suse") || is("opensuse") {
        (
            Some("sudo zypper install ffmpeg"),
            Some("openSUSE's own FFmpeg cannot write H.264; the Packman repository's can."),
        )
    } else {
        (
            None,
            Some("Install ffmpeg with your distribution's package manager."),
        )
    }
}

impl App {
    /// Opens the setup window and checks the programs again.
    pub(crate) fn show_ffmpeg_setup(&mut self) {
        self.ffmpeg.open = true;
        self.ffmpeg.start_check(&self.ui_context);
    }

    pub(crate) fn ffmpeg_window(&mut self, ctx: &egui::Context) {
        self.ffmpeg.receive();
        if !self.ffmpeg.open || self.show_welcome {
            return;
        }
        let mut again = false;
        let mut window = self.window_state("FFmpeg setup");
        let setup = &self.ffmpeg;
        let open = chrome::tool_window(ctx, "FFmpeg setup", [520., 520.], &mut window, |ui| {
            ui.label(
                "Images, GIFs and animated WebPs open without FFmpeg. Opening videos, and \
                 exporting any video or animation, need FFmpeg and ffprobe.",
            );
            ui.separator();
            match &setup.check {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Checking…");
                    });
                }
                Some(check) => {
                    egui::Grid::new("ffmpeg programs").show(ui, |ui| {
                        for (tool, found) in [
                            (Tool::Ffmpeg, &check.ffmpeg),
                            (Tool::Ffprobe, &check.ffprobe),
                        ] {
                            ui.strong(tool.name());
                            match found {
                                Ok(found) => ui
                                    .label(format!("✔ {}", found.version))
                                    .on_hover_text(found.executable.display().to_string()),
                                Err(_) => {
                                    ui.colored_label(ui.visuals().error_fg_color, "✘ Not found")
                                }
                            };
                            ui.end_row();
                        }
                    });
                    if check.ffmpeg.is_ok() {
                        ui.separator();
                        ui.label("Export formats");
                        egui::Grid::new("ffmpeg formats").show(ui, |ui| {
                            for (format, encoder) in formats() {
                                ui.label(format);
                                if check.encodes(encoder) {
                                    ui.label("✔");
                                } else {
                                    ui.colored_label(
                                        ui.visuals().warn_fg_color,
                                        format!("✘ needs {encoder}"),
                                    );
                                }
                                ui.end_row();
                            }
                        });
                    }
                    if !check.ready() || formats().iter().any(|(_, e)| !check.encodes(e)) {
                        ui.separator();
                        install_help(ui);
                    }
                }
            }
            ui.separator();
            again = ui
                .add_enabled(setup.checking.is_none(), egui::Button::new("Check again"))
                .clicked();
            ui.small(
                "CRTSim Renderer never downloads FFmpeg itself. It uses the one on your PATH, \
                 one in a folder named ffmpeg next to the app, or the ones CRTSIM_FFMPEG and \
                 CRTSIM_FFPROBE name.",
            );
        });
        self.store_window_state("FFmpeg setup", window);
        self.ffmpeg.open = open;
        if again {
            self.ffmpeg.start_check(ctx);
        }
    }
}

/// How to install FFmpeg here, with the command to copy.
fn install_help(ui: &mut egui::Ui) {
    let install = install();
    ui.strong("Install FFmpeg");
    if let Some(command) = install.command {
        ui.horizontal(|ui| {
            ui.code(command);
            if ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(command.into());
            }
        });
    }
    if let Some(note) = install.note {
        ui.small(note);
    }
    ui.hyperlink_to(install.link.0, install.link.1);
    if let Some(folder) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("ffmpeg")))
    {
        ui.small("Or put ffmpeg and ffprobe in this folder, then choose Check again:");
        ui.horizontal(|ui| {
            let folder = folder.display().to_string();
            ui.code(&folder);
            if ui.small_button("Copy").clicked() {
                ui.ctx().copy_text(folder);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_distributions_get_their_own_install_command() {
        let command = |release: &str| linux_install(release).0;
        assert_eq!(
            command("NAME=\"EndeavourOS\"\nID=\"endeavouros\"\nID_LIKE=\"arch\"\n"),
            Some("sudo pacman -S ffmpeg")
        );
        assert_eq!(
            command("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"),
            Some("sudo apt install ffmpeg")
        );
        let (fedora, note) = linux_install("ID=fedora\n");
        assert_eq!(fedora, Some("sudo dnf install ffmpeg"));
        assert!(note.unwrap().contains("RPM Fusion"));
        assert_eq!(command("ID=gentoo\n"), None);
    }

    fn missing() -> ToolCheck {
        ToolCheck {
            ffmpeg: Err("Cannot start ffmpeg".into()),
            ffprobe: Err("Cannot start ffprobe".into()),
            encoders: vec![],
        }
    }

    #[test]
    fn a_video_opened_without_ffmpeg_shows_how_to_set_it_up_instead() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            Some(Smoke::new("unused-smoke.png".into())),
        );
        let (work, _previews) = app.capture_jobs();
        app.show_welcome = false;
        app.ffmpeg.checking = None;
        app.ffmpeg.check = Some(missing());
        app.load("holiday.mp4".into());
        assert!(work.try_recv().is_err(), "nothing is sent to fail");
        assert!(app.ffmpeg.open && app.lane.is_idle());
        assert_eq!(app.status, "Opening holiday.mp4 needs FFmpeg");
        // The window draws the missing programs and how to install them.
        app.ffmpeg.checking = None;
        app.ffmpeg.check = Some(missing());
        ctx.run_ui(Default::default(), |ui| app.ffmpeg_window(ui.ctx()))
            .textures_delta
            .clear();
        assert!(app.ffmpeg.open);
    }

    #[test]
    fn nothing_is_missing_until_a_check_says_so() {
        let mut setup = FfmpegSetup::default();
        assert!(!setup.missing() && !setup.lacks("libx264"));
        setup.check = Some(ToolCheck {
            ffmpeg: Ok(crtsim_media::Found {
                executable: "ffmpeg".into(),
                version: "ffmpeg version 7".into(),
            }),
            ffprobe: Err("Cannot start ffprobe".into()),
            encoders: vec!["gif".into()],
        });
        assert!(setup.missing());
        assert!(setup.lacks("libx264") && !setup.lacks("gif"));
    }
}
