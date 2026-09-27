//! Everything the app remembers between runs, in one folder: the welcome acknowledged, the
//! interface theme, where tool windows were, personal presets, recent projects and the session
//! to recover. Each file has a size it may not exceed, and each is replaced only once the whole
//! of its new contents is written.
use crate::{project, theme::Theme};
use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use std::{
    fs::File,
    io::Write,
    path::{Path, PathBuf},
};

const WELCOME: &str = "welcome-v1.txt";
const THEME: &str = "theme-v1.txt";
const WINDOWS: &str = "windows-v1.json";
const PRESETS: &str = "presets";
const RECENT: &str = "recent-projects-v1.json";
const SESSION: &str = "session-v1.crtsim";

/// How many recent projects are remembered.
pub const RECENT_PROJECTS: usize = 10;
/// The longest description a personal preset has, in bytes.
const DESCRIPTION_BYTES: u64 = 4096;

/// Remembered tool window placements, keyed by window title.
pub type Layout = std::collections::BTreeMap<String, crate::chrome::ToolWindow>;

/// A preset saved in the gallery's personal presets.
pub struct Preset {
    pub name: String,
    pub description: String,
    pub config: Config,
}

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// A store in `root`, for tests.
    #[cfg(test)]
    pub fn in_folder(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// The folder `CRTSIM_DATA_DIR` names, or the platform's app data folder.
    pub fn discover() -> Result<Self> {
        if let Some(path) = std::env::var_os("CRTSIM_DATA_DIR") {
            ensure!(
                !path.is_empty() && Path::new(&path).is_absolute(),
                "CRTSIM_DATA_DIR must be an absolute directory"
            );
            return Ok(Self { root: path.into() });
        }
        let dirs = directories::ProjectDirs::from("", "", "CRTSim-Renderer")
            .context("Cannot locate app data directory")?;
        Ok(Self {
            root: dirs.data_dir().to_path_buf(),
        })
    }

    /// The contents of `file`, or `None` when there is none. A file larger than `limit` bytes
    /// is refused with `too_large`.
    fn read(&self, file: impl AsRef<Path>, limit: u64, too_large: &str) -> Result<Option<Vec<u8>>> {
        let path = self.root.join(file);
        if !path.exists() {
            return Ok(None);
        }
        ensure!(path.metadata()?.len() <= limit, "{too_large}");
        Ok(Some(std::fs::read(path)?))
    }

    /// Replaces `file` with what `write` writes, making the folders it goes in if needed.
    fn replace(
        &self,
        file: impl AsRef<Path>,
        write: impl FnOnce(&mut File) -> Result<()>,
    ) -> Result<()> {
        let path = self.root.join(file);
        std::fs::create_dir_all(path.parent().unwrap_or(&self.root))?;
        crate::files::save_atomic(&path, write)
    }

    pub fn welcome_needed(&self) -> bool {
        std::fs::read(self.root.join(WELCOME)).ok().as_deref() != Some(b"acknowledged\n")
    }

    pub fn acknowledge(&self) -> Result<()> {
        self.replace(WELCOME, |file| Ok(file.write_all(b"acknowledged\n")?))
    }

    pub fn theme(&self) -> Result<Option<Theme>> {
        let Some(bytes) = self.read(THEME, 64, "Theme setting is too large")? else {
            return Ok(None);
        };
        let value = String::from_utf8(bytes)?;
        let id = value.trim();
        Theme::from_id(id)
            .map(Some)
            .with_context(|| format!("Unknown saved theme {id:?}"))
    }

    pub fn set_theme(&self, theme: Theme) -> Result<()> {
        self.replace(THEME, |file| Ok(writeln!(file, "{}", theme.id())?))
    }

    /// Tool window placements from the last run, keyed by window title.
    pub fn tool_windows(&self) -> Result<Layout> {
        match self.read(WINDOWS, 8192, "Saved window layout is too large")? {
            Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
            None => Ok(Layout::new()),
        }
    }

    pub fn set_tool_windows(&self, windows: &Layout) -> Result<()> {
        self.replace(WINDOWS, |file| {
            Ok(serde_json::to_writer_pretty(file, windows)?)
        })
    }

    /// The projects opened or saved most recently, newest first.
    pub fn recent_projects(&self) -> Result<Vec<PathBuf>> {
        let Some(bytes) = self.read(RECENT, 64 * 1024, "Recent projects list is too large")? else {
            return Ok(vec![]);
        };
        let mut paths: Vec<PathBuf> = serde_json::from_slice(&bytes)?;
        paths.truncate(RECENT_PROJECTS);
        Ok(paths)
    }

    pub fn set_recent_projects(&self, paths: &[PathBuf]) -> Result<()> {
        let bytes = serde_json::to_vec(paths)?;
        self.replace(RECENT, |file| Ok(file.write_all(&bytes)?))
    }

    /// The session saved when the app last ran, to offer to recover.
    pub fn session(&self) -> Result<Option<project::Project>> {
        let path = self.root.join(SESSION);
        path.exists().then(|| project::read(&path)).transpose()
    }

    pub fn set_session(&self, session: &project::Project) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        project::save(&self.root.join(SESSION), session)
    }

    /// Where personal presets are saved, for showing a person.
    pub fn presets_folder(&self) -> PathBuf {
        self.root.join(PRESETS)
    }

    /// The personal presets, by name, with a warning for each file that could not be read.
    pub fn presets(&self) -> Result<(Vec<Preset>, Vec<String>)> {
        let dir = self.presets_folder();
        if !dir.exists() {
            return Ok((vec![], vec![]));
        }
        let mut presets = vec![];
        let mut warnings = vec![];
        let mut paths = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        paths.sort_by_key(|p| p.file_name());
        for item in paths {
            if !item.file_type()?.is_file()
                || !item
                    .path()
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
            {
                continue;
            }
            let name = crate::file_stem(&item.path());
            match crate::files::load_preset(&item.path(), (256, 224)) {
                Ok(config) => presets.push(Preset {
                    description: self.description(&name).unwrap_or_default(),
                    name,
                    config,
                }),
                Err(e) => warnings.push(format!("{name}: {e:#}")),
            }
        }
        Ok((presets, warnings))
    }

    fn description(&self, name: &str) -> Result<String> {
        let file = Path::new(PRESETS).join(format!("{name}.txt"));
        let too_large = format!("Description exceeds {DESCRIPTION_BYTES} bytes");
        match self.read(file, DESCRIPTION_BYTES, &too_large)? {
            Some(bytes) => Ok(String::from_utf8(bytes)?),
            None => Ok(String::new()),
        }
    }

    pub fn set_description(&self, name: &str, description: &str) -> Result<()> {
        validate_name(name)?;
        ensure!(
            description.len() as u64 <= DESCRIPTION_BYTES,
            "Description exceeds {DESCRIPTION_BYTES} bytes"
        );
        ensure!(
            self.presets_folder().join(format!("{name}.json")).is_file(),
            "Preset no longer exists"
        );
        // Kept beside the preset, so its JSON stays readable by older versions and the CLI.
        let file = Path::new(PRESETS).join(format!("{name}.txt"));
        self.replace(file, |file| Ok(file.write_all(description.as_bytes())?))
    }

    /// Saves `config` as a new personal preset. A name already taken, in any letter case, is
    /// refused rather than replaced.
    pub fn save_preset(&self, name: &str, config: &Config, input: (u32, u32)) -> Result<()> {
        validate_name(name)?;
        config.validate_for(input)?;
        let dir = self.presets_folder();
        std::fs::create_dir_all(&dir)?;
        // Case-insensitive collisions are refused on every OS for portable galleries.
        for item in std::fs::read_dir(&dir)? {
            ensure!(
                item?.file_name().to_string_lossy().to_lowercase()
                    != format!("{name}.json").to_lowercase(),
                "A preset with this name already exists. Choose another name."
            );
        }
        crate::files::save_preset(&dir.join(format!("{name}.json")), config)
    }
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 120
            && name == name.trim()
            && name
                .chars()
                .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_'),
        "Use a name of 1–120 bytes containing letters, numbers, spaces, - or _"
    );
    let upper = name.to_ascii_uppercase();
    ensure!(
        !["CON", "PRN", "AUX", "NUL"].contains(&upper.as_str())
            && !(1..=9).any(|n| upper == format!("COM{n}") || upper == format!("LPT{n}")),
        "This name is reserved by Windows"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: &Path) -> Store {
        Store::in_folder(root)
    }

    #[test]
    fn settings_persist_and_oversized_files_are_refused() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path());
        assert!(s.welcome_needed());
        s.acknowledge().unwrap();
        assert!(!s.welcome_needed());
        assert_eq!(s.theme().unwrap(), None);
        s.set_theme(Theme::SkyDiary).unwrap();
        assert_eq!(s.theme().unwrap(), Some(Theme::SkyDiary));
        assert!(s.tool_windows().unwrap().is_empty());
        let mut layout = Layout::new();
        layout.insert(
            "LUT gallery".into(),
            crate::chrome::ToolWindow {
                placement: Some(crate::chrome::Placement {
                    position: [120., 48.],
                    size: [640., 520.],
                }),
                on_top: true,
                ..Default::default()
            },
        );
        s.set_tool_windows(&layout).unwrap();
        assert_eq!(s.tool_windows().unwrap(), layout);
        std::fs::write(temp.path().join(WINDOWS), "x".repeat(8193)).unwrap();
        assert!(s.tool_windows().is_err());
        s.set_tool_windows(&Layout::new()).unwrap();
        assert!(s.tool_windows().unwrap().is_empty());
    }

    #[test]
    fn personal_presets_persist_and_bad_files_do_not_hide_good_ones() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path());
        let c = Config::general();
        s.save_preset("My CRT", &c, (1216, 832)).unwrap();
        assert!(s.save_preset("my crt", &c, (1, 1)).is_err());
        for name in ["../oops", "CON", "", "nested/file", "trailing "] {
            assert!(s.save_preset(name, &c, (1, 1)).is_err());
        }
        std::fs::write(temp.path().join("presets/broken.json"), "not json").unwrap();
        let reopened = store(temp.path());
        let (presets, warnings) = reopened.presets().unwrap();
        assert_eq!(presets.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(presets[0].config, c);
        assert_eq!(presets[0].description, "");
        s.set_description("My CRT", "Soft mask — for animation\nMy own look")
            .unwrap();
        let updated = reopened.presets().unwrap().0;
        assert_eq!(
            updated[0].description,
            "Soft mask — for animation\nMy own look"
        );
        assert_eq!(updated[0].config, c);
        assert!(s.set_description("../outside", "oops").is_err());
        assert!(s.set_description("My CRT", &"x".repeat(4097)).is_err());
        s.set_description("My CRT", "").unwrap();
        assert_eq!(reopened.presets().unwrap().0[0].description, "");
        assert_eq!(s.presets_folder(), temp.path().join("presets"));
    }

    #[test]
    fn recent_projects_and_the_session_persist_before_the_folder_exists() {
        let temp = tempfile::tempdir().unwrap();
        // A first run: nothing has made the folder yet, since the welcome is still showing.
        let s = store(&temp.path().join("not yet made"));
        assert!(s.recent_projects().unwrap().is_empty());
        assert!(s.session().unwrap().is_none());
        let paths: Vec<PathBuf> = (0..12).map(|n| format!("{n}.crtsim").into()).collect();
        s.set_recent_projects(&paths).unwrap();
        assert_eq!(s.recent_projects().unwrap(), paths[..RECENT_PROJECTS]);
        let session = project::Project {
            version: 1,
            source: None,
            frame: 3,
            config: Config::general(),
            options: Default::default(),
            queue: vec![],
        };
        s.set_session(&session).unwrap();
        assert_eq!(s.session().unwrap(), Some(session));
        std::fs::write(s.root.join(RECENT), "x".repeat(64 * 1024 + 1)).unwrap();
        assert!(s.recent_projects().is_err());
        std::fs::write(s.root.join(SESSION), "not a project").unwrap();
        assert!(s.session().is_err());
    }
}
