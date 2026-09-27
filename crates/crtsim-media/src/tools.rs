//! Finding FFmpeg and ffprobe, and checking what they can do. Nothing is downloaded: the
//! programs are looked for where a person or a package manager put them.
use crate::process::Process;
use anyhow::{ensure, Result};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

/// The FFmpeg programs video work runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// Decodes, encodes and muxes.
    Ffmpeg,
    /// Reads what a file holds.
    Ffprobe,
}

impl Tool {
    pub const ALL: [Self; 2] = [Self::Ffmpeg, Self::Ffprobe];

    pub fn name(self) -> &'static str {
        match self {
            Self::Ffmpeg => "ffmpeg",
            Self::Ffprobe => "ffprobe",
        }
    }

    /// The environment variable naming its executable, for a portable FFmpeg installation,
    /// which may have spaces in its path.
    pub fn variable(self) -> &'static str {
        match self {
            Self::Ffmpeg => "CRTSIM_FFMPEG",
            Self::Ffprobe => "CRTSIM_FFPROBE",
        }
    }

    /// Its executable: the one its variable names; else one beside this program, in an
    /// `ffmpeg` or `ffmpeg/bin` folder there, or in a folder package managers install to;
    /// else its bare name, for the system to find on PATH.
    ///
    /// The package managers' folders are there because a program started from a desktop
    /// launcher may not have them on its PATH -- Homebrew's, on a Mac, never is -- and
    /// because a program already running does not see PATH change when FFmpeg is installed.
    pub fn executable(self) -> OsString {
        if let Some(named) = std::env::var_os(self.variable()) {
            return named;
        }
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        self.find_in(&search_folders(beside.as_deref()))
            .map_or_else(|| self.name().into(), PathBuf::into_os_string)
    }

    /// The first of `folders` holding its executable.
    fn find_in(self, folders: &[PathBuf]) -> Option<PathBuf> {
        let file = format!("{}{}", self.name(), std::env::consts::EXE_SUFFIX);
        folders
            .iter()
            .map(|folder| folder.join(&file))
            .find(|path| path.is_file())
    }

    /// A command running it quietly, with nothing read from the terminal.
    pub(crate) fn command(self) -> Command {
        let mut command = Command::new(self.executable());
        if std::env::var_os("CRTSIM_APPIMAGE").is_some() {
            if let Some(original) = std::env::var_os("CRTSIM_HOST_LD_LIBRARY_PATH") {
                command.env("LD_LIBRARY_PATH", original);
            }
        }
        command.args(["-v", "error"]);
        if self == Self::Ffmpeg {
            command.arg("-nostdin");
        }
        command
    }
}

/// Where to look for the programs before PATH: beside this program first, then where package
/// managers put them on this system.
fn search_folders(beside: Option<&Path>) -> Vec<PathBuf> {
    let mut folders = vec![];
    if let Some(beside) = beside {
        folders.push(beside.to_path_buf());
        folders.push(beside.join("ffmpeg"));
        folders.push(beside.join("ffmpeg").join("bin"));
    }
    if cfg!(target_os = "macos") {
        // Homebrew on Apple silicon and on Intel, then MacPorts.
        folders
            .extend(["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"].map(PathBuf::from));
    }
    if cfg!(windows) {
        let under = |variable: &str, path: &[&str]| {
            std::env::var_os(variable).map(|root| {
                path.iter()
                    .fold(PathBuf::from(root), |p, part| p.join(part))
            })
        };
        folders.extend(
            [
                under("LOCALAPPDATA", &["Microsoft", "WinGet", "Links"]),
                under("USERPROFILE", &["scoop", "shims"]),
                under("ProgramData", &["chocolatey", "bin"]),
            ]
            .into_iter()
            .flatten(),
        );
    }
    folders
}

/// The encoders FFmpeg offers, by name.
pub(crate) fn encoders(cancel: &Arc<AtomicBool>) -> Result<Vec<String>> {
    let mut cmd = Tool::Ffmpeg.command();
    cmd.args(["-hide_banner", "-encoders"]);
    let bytes = Process::output(
        &mut cmd,
        cancel,
        2 * 1024 * 1024,
        "FFmpeg encoder list is too large",
    )?;
    Ok(parse_encoders(&String::from_utf8_lossy(&bytes)))
}

/// The names in FFmpeg's encoder list: the second word of each line. The legend's lines
/// give `=` or nothing there, which no encoder is called.
fn parse_encoders(list: &str) -> Vec<String> {
    list.lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|name| *name != "=")
        .map(str::to_owned)
        .collect()
}

/// Fails unless FFmpeg offers the encoder `name`.
pub(crate) fn require_encoder(name: &str, cancel: &Arc<AtomicBool>) -> Result<()> {
    ensure!(
        encoders(cancel)?.iter().any(|encoder| encoder == name),
        "This FFmpeg installation does not provide the required {name} video encoder"
    );
    Ok(())
}

/// One of the programs, found and started.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// What was run: a path, or a bare name the system found on PATH.
    pub executable: PathBuf,
    /// The first line it prints about its version.
    pub version: String,
}

/// What this computer has for video work: each program, or why it could not be started, and
/// the encoders FFmpeg offers.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCheck {
    pub ffmpeg: std::result::Result<Found, String>,
    pub ffprobe: std::result::Result<Found, String>,
    /// Empty when FFmpeg could not be started.
    pub encoders: Vec<String>,
}

impl ToolCheck {
    /// Runs each program once to see that it starts and what it is.
    pub fn run(cancel: &Arc<AtomicBool>) -> Self {
        let version = |tool: Tool| {
            let mut cmd = tool.command();
            cmd.arg("-version");
            Process::output(&mut cmd, cancel, 1024 * 1024, "Version text is too large")
                .map(|bytes| Found {
                    executable: tool.executable().into(),
                    version: String::from_utf8_lossy(&bytes)
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                })
                .map_err(|e| format!("{e:#}"))
        };
        let ffmpeg = version(Tool::Ffmpeg);
        let encoders = match &ffmpeg {
            Ok(_) => encoders(cancel).unwrap_or_default(),
            Err(_) => vec![],
        };
        Self {
            ffprobe: version(Tool::Ffprobe),
            ffmpeg,
            encoders,
        }
    }

    /// Whether both programs start, which opening a video needs.
    pub fn ready(&self) -> bool {
        self.ffmpeg.is_ok() && self.ffprobe.is_ok()
    }

    /// Whether FFmpeg offers the encoder `name`.
    pub fn encodes(&self, name: &str) -> bool {
        self.encoders.iter().any(|encoder| encoder == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_programs_are_found_beside_this_one_or_in_its_ffmpeg_folder() {
        let dir = tempfile::tempdir().unwrap();
        let beside = dir.path().join("app");
        let bundled = beside.join("ffmpeg").join("bin");
        std::fs::create_dir_all(&bundled).unwrap();
        let file = |name: &str| format!("{name}{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(bundled.join(file("ffmpeg")), b"").unwrap();
        std::fs::write(beside.join(file("ffprobe")), b"").unwrap();
        let folders = &search_folders(Some(&beside))[..3];
        // The `ffmpeg` folder beside the program is not taken for the program itself.
        assert_eq!(
            Tool::Ffmpeg.find_in(folders),
            Some(bundled.join(file("ffmpeg")))
        );
        assert_eq!(
            Tool::Ffprobe.find_in(folders),
            Some(beside.join(file("ffprobe")))
        );
        std::fs::write(bundled.join(file("ffprobe")), b"").unwrap();
        assert_eq!(
            Tool::Ffprobe.find_in(folders),
            Some(beside.join(file("ffprobe"))),
            "right beside the program comes first"
        );
        assert_eq!(Tool::Ffmpeg.find_in(&[dir.path().into()]), None);
    }

    #[test]
    fn the_encoder_list_is_read_past_its_legend() {
        let list = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n \
                    V....D libx264              libx264 H.264\n \
                    A....D aac                  AAC (Advanced Audio Coding)\n";
        assert_eq!(parse_encoders(list), ["libx264", "aac"]);
    }

    #[test]
    fn a_program_that_cannot_start_is_reported_not_found() {
        let check = ToolCheck {
            ffmpeg: Err("Cannot start ffmpeg".into()),
            ffprobe: Ok(Found {
                executable: "ffprobe".into(),
                version: "ffprobe version 7".into(),
            }),
            encoders: vec!["gif".into()],
        };
        assert!(!check.ready());
        assert!(check.encodes("gif") && !check.encodes("libx264"));
    }
}
