//! Everything the app remembers between runs: the welcome acknowledged, the interface theme,
//! where tool windows were, personal presets, recent projects and the session to recover. On
//! the desktop they are files in one folder, each replaced only once the whole of its new
//! contents is written; in a browser, entries in its storage for the site. Each has a size it
//! may not exceed.
use crate::{project, theme::Theme};
use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use std::{
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
    place: Place,
    /// The temporary folder a test's store is in, removed with the store's last clone.
    #[cfg(test)]
    _temporary: Option<std::sync::Arc<tempfile::TempDir>>,
}

/// Where the files are.
#[derive(Clone)]
enum Place {
    /// A folder on disk: the desktop.
    Folder(PathBuf),
    /// A browser's storage for this site.
    #[cfg(target_arch = "wasm32")]
    Browser(std::rc::Rc<crate::web::Files>),
}

impl Store {
    fn at(place: Place) -> Self {
        Self {
            place,
            #[cfg(test)]
            _temporary: None,
        }
    }

    /// A store in `root`, for tests.
    #[cfg(test)]
    pub fn in_folder(root: &Path) -> Self {
        Self::at(Place::Folder(root.to_path_buf()))
    }

    /// A store in a new, empty temporary folder, so a test never reads or writes a person's
    /// own app data.
    #[cfg(test)]
    pub fn temporary() -> Self {
        let folder = tempfile::tempdir().expect("a temporary app data folder");
        Self {
            place: Place::Folder(folder.path().to_path_buf()),
            _temporary: Some(std::sync::Arc::new(folder)),
        }
    }

    /// The folder `CRTSIM_DATA_DIR` names, or the platform's app data folder.
    pub fn discover() -> Result<Self> {
        if let Some(path) = std::env::var_os("CRTSIM_DATA_DIR") {
            ensure!(
                !path.is_empty() && Path::new(&path).is_absolute(),
                "CRTSIM_DATA_DIR must be an absolute directory"
            );
            return Ok(Self::at(Place::Folder(path.into())));
        }
        let dirs = directories::ProjectDirs::from("", "", "CRTSim-Renderer")
            .context("Cannot locate app data directory")?;
        Ok(Self::at(Place::Folder(dirs.data_dir().to_path_buf())))
    }

    /// The store kept in this browser's storage for the site, read whole now.
    #[cfg(target_arch = "wasm32")]
    pub async fn browser() -> Result<Self> {
        let files = crate::web::Files::open().await?;
        Ok(Self::at(Place::Browser(std::rc::Rc::new(files))))
    }

    /// The contents of `file`, or `None` when there is none. A file larger than `limit` bytes
    /// is refused with `too_large`.
    fn read(&self, file: &str, limit: u64, too_large: &str) -> Result<Option<Vec<u8>>> {
        match &self.place {
            Place::Folder(root) => {
                let path = root.join(file);
                if !path.exists() {
                    return Ok(None);
                }
                ensure!(path.metadata()?.len() <= limit, "{too_large}");
                Ok(Some(std::fs::read(path)?))
            }
            #[cfg(target_arch = "wasm32")]
            Place::Browser(files) => match files.read(file) {
                Some(bytes) => {
                    ensure!(bytes.len() as u64 <= limit, "{too_large}");
                    Ok(Some(bytes))
                }
                None => Ok(None),
            },
        }
    }

    /// Replaces `file` with `bytes`, making the folders it goes in if needed. A file on disk
    /// is replaced only once the whole of it is written.
    fn write(&self, file: &str, bytes: &[u8]) -> Result<()> {
        match &self.place {
            Place::Folder(root) => {
                let path = root.join(file);
                std::fs::create_dir_all(path.parent().unwrap_or(root))?;
                crate::files::save_atomic(&path, |out| Ok(out.write_all(bytes)?))
            }
            #[cfg(target_arch = "wasm32")]
            Place::Browser(files) => {
                files.write(file, bytes);
                Ok(())
            }
        }
    }

    /// Removes `file`, if there is one.
    fn remove(&self, file: &str) -> Result<()> {
        match &self.place {
            Place::Folder(root) => match std::fs::remove_file(root.join(file)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                _ => Ok(()),
            },
            #[cfg(target_arch = "wasm32")]
            Place::Browser(files) => {
                files.remove(file);
                Ok(())
            }
        }
    }

    /// The names of the files in `folder`, sorted.
    fn names(&self, folder: &str) -> Result<Vec<String>> {
        match &self.place {
            Place::Folder(root) => {
                let dir = root.join(folder);
                if !dir.exists() {
                    return Ok(vec![]);
                }
                let mut names = vec![];
                for item in std::fs::read_dir(dir)? {
                    let item = item?;
                    if item.file_type()?.is_file() {
                        names.push(item.file_name().to_string_lossy().into_owned());
                    }
                }
                names.sort();
                Ok(names)
            }
            #[cfg(target_arch = "wasm32")]
            Place::Browser(files) => Ok(files.names(&format!("{folder}/"))),
        }
    }

    pub fn welcome_needed(&self) -> bool {
        self.read(WELCOME, 64, "").ok().flatten().as_deref() != Some(b"acknowledged\n")
    }

    pub fn acknowledge(&self) -> Result<()> {
        self.write(WELCOME, b"acknowledged\n")
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
        self.write(THEME, format!("{}\n", theme.id()).as_bytes())
    }

    /// Tool window placements from the last run, keyed by window title.
    pub fn tool_windows(&self) -> Result<Layout> {
        match self.read(WINDOWS, 8192, "Saved window layout is too large")? {
            Some(bytes) => Ok(serde_json::from_slice(&bytes)?),
            None => Ok(Layout::new()),
        }
    }

    pub fn set_tool_windows(&self, windows: &Layout) -> Result<()> {
        self.write(WINDOWS, &serde_json::to_vec_pretty(windows)?)
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
        self.write(RECENT, &serde_json::to_vec(paths)?)
    }

    /// The session saved when the app last ran, to offer to recover.
    pub fn session(&self) -> Result<Option<project::Project>> {
        let root = match &self.place {
            Place::Folder(root) => root.as_path(),
            #[cfg(target_arch = "wasm32")]
            Place::Browser(_) => Path::new("."),
        };
        self.read(SESSION, 64 * 1024 * 1024, "Project exceeds 64 MB")?
            .map(|bytes| project::parse(&bytes, root))
            .transpose()
    }

    pub fn set_session(&self, session: &project::Project) -> Result<()> {
        self.write(SESSION, &project::to_bytes(session)?)
    }

    /// Where personal presets are kept, for telling a person.
    pub fn presets_location(&self) -> String {
        match &self.place {
            Place::Folder(root) => root.join(PRESETS).display().to_string(),
            #[cfg(target_arch = "wasm32")]
            Place::Browser(_) => "kept in this browser".into(),
        }
    }

    /// The personal presets, by name, with a warning for each file that could not be read.
    pub fn presets(&self) -> Result<(Vec<Preset>, Vec<String>)> {
        let mut presets = vec![];
        let mut warnings = vec![];
        for file in self.names(PRESETS)? {
            let Some(name) = file
                .rsplit_once('.')
                .filter(|(_, extension)| extension.eq_ignore_ascii_case("json"))
                .map(|(name, _)| name.to_owned())
            else {
                continue;
            };
            let read = self
                .read(
                    &format!("{PRESETS}/{file}"),
                    32 * 1024 * 1024,
                    "Preset exceeds 32 MB",
                )
                .and_then(|bytes| {
                    let bytes = bytes.context("Preset no longer exists")?;
                    crate::files::preset_from_json(&bytes, (256, 224))
                });
            match read {
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
        let too_large = format!("Description exceeds {DESCRIPTION_BYTES} bytes");
        match self.read(
            &format!("{PRESETS}/{name}.txt"),
            DESCRIPTION_BYTES,
            &too_large,
        )? {
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
        let saved = self
            .saved_preset(name)?
            .context("Preset no longer exists")?;
        // Kept beside the preset, so its JSON stays readable by older versions and the CLI.
        self.write(&format!("{PRESETS}/{saved}.txt"), description.as_bytes())
    }

    /// The files under personal preset name `name`, in any letter case. Names that differ only
    /// in case are one name on every OS, so galleries stay portable.
    fn preset_files(&self, name: &str) -> Result<PresetFiles> {
        let (json, txt) = (
            format!("{name}.json").to_lowercase(),
            format!("{name}.txt").to_lowercase(),
        );
        let mut files = PresetFiles::default();
        for file in self.names(PRESETS)? {
            let lower = file.to_lowercase();
            if lower == json {
                files.settings = Some(file);
            } else if lower == txt {
                files.descriptions.push(file);
            }
        }
        Ok(files)
    }

    /// The personal preset `name` names, in any letter case, as it is saved, if there is one.
    pub fn saved_preset(&self, name: &str) -> Result<Option<String>> {
        Ok(self.preset_files(name)?.saved())
    }

    /// Saves `config` as personal preset `name`, and returns the name it is saved under. A
    /// preset with that name already, in any letter case, is refused unless `replace` is set;
    /// then it keeps its name and description and takes the new settings. An included
    /// preset's name is always refused, so a personal preset never stands in for one.
    pub fn save_preset(
        &self,
        name: &str,
        config: &Config,
        input: (u32, u32),
        replace: bool,
    ) -> Result<String> {
        validate_name(name)?;
        ensure!(
            !crate::gallery::is_included(name),
            "“{name}” is an included preset, which cannot be replaced. Choose another name."
        );
        config.validate_for(input)?;
        let files = self.preset_files(name)?;
        let (saved, file) = match (files.saved(), files.settings) {
            (Some(saved), Some(file)) => {
                ensure!(
                    replace,
                    "A preset with this name already exists. Choose another name."
                );
                (saved, file)
            }
            _ => {
                // A description left behind by a preset no longer here is not this one's.
                for orphan in &files.descriptions {
                    self.remove(&format!("{PRESETS}/{orphan}"))?;
                }
                (name.to_owned(), format!("{name}.json"))
            }
        };
        self.write(
            &format!("{PRESETS}/{file}"),
            &crate::files::preset_json(config)?,
        )?;
        Ok(saved)
    }

    /// Deletes personal preset `name`, in any letter case, and its description. The
    /// description goes first: should the settings then fail to go, the preset is still there
    /// without it, where the other way round would leave its description behind for the next
    /// preset saved under the name.
    pub fn delete_preset(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        let files = self.preset_files(name)?;
        let settings = files.settings.context("Preset no longer exists")?;
        for description in &files.descriptions {
            self.remove(&format!("{PRESETS}/{description}"))?;
        }
        self.remove(&format!("{PRESETS}/{settings}"))
    }
}

/// The files under one personal preset name.
#[derive(Default)]
struct PresetFiles {
    /// The settings, if a preset of that name is saved.
    settings: Option<String>,
    /// Its description, or ones left behind by a preset of that name since gone.
    descriptions: Vec<String>,
}

impl PresetFiles {
    /// The preset's name, as it is saved.
    fn saved(&self) -> Option<String> {
        let file = self.settings.as_ref()?;
        file.get(..file.len() - ".json".len()).map(str::to_owned)
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
        s.save_preset("My CRT", &c, (1216, 832), false).unwrap();
        assert!(s.save_preset("my crt", &c, (1, 1), false).is_err());
        for name in ["../oops", "CON", "", "nested/file", "trailing "] {
            assert!(s.save_preset(name, &c, (1, 1), false).is_err());
            assert!(s.save_preset(name, &c, (1, 1), true).is_err());
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
        assert_eq!(
            s.presets_location(),
            temp.path().join("presets").display().to_string()
        );
    }

    #[test]
    fn personal_presets_are_replaced_and_deleted_on_request() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path());
        let first = Config::general();
        let second = Config::default();
        assert_ne!(first, second);
        s.save_preset("My CRT", &first, (256, 224), false).unwrap();
        s.set_description("My CRT", "Kept").unwrap();
        assert_eq!(s.saved_preset("my crt").unwrap().as_deref(), Some("My CRT"));
        assert_eq!(s.saved_preset("Other").unwrap(), None);
        // Replacing keeps the saved name, in its own letter case, and its description.
        let saved = s.save_preset("my crt", &second, (256, 224), true).unwrap();
        assert_eq!(saved, "My CRT");
        let presets = s.presets().unwrap().0;
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "My CRT");
        assert_eq!(presets[0].config, second);
        assert_eq!(presets[0].description, "Kept");
        // A new name is saved as typed, whether or not replacing was allowed.
        assert_eq!(
            s.save_preset("Other", &first, (256, 224), true).unwrap(),
            "Other"
        );
        s.delete_preset("My CRT").unwrap();
        assert!(!temp.path().join("presets/My CRT.json").exists());
        assert!(!temp.path().join("presets/My CRT.txt").exists());
        let names: Vec<_> = s.presets().unwrap().0.into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Other"]);
        assert!(s.delete_preset("My CRT").is_err());
        assert!(s.delete_preset("../oops").is_err());
        // Deleted, the name is free again.
        s.save_preset("my crt", &first, (256, 224), false).unwrap();
    }

    #[test]
    fn included_presets_cannot_be_saved_over_in_any_letter_case() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path());
        let c = Config::general();
        for included in crate::gallery::builtins() {
            for name in [
                included.name.clone(),
                included.name.to_lowercase(),
                included.name.to_uppercase(),
            ] {
                for replace in [false, true] {
                    let refused = s.save_preset(&name, &c, (256, 224), replace);
                    assert!(refused.is_err(), "{name} was saved");
                }
            }
        }
        assert!(s.presets().unwrap().0.is_empty(), "nothing was written");
        // Even over a personal preset saved under one before they were refused.
        std::fs::create_dir_all(temp.path().join("presets")).unwrap();
        std::fs::write(
            temp.path().join("presets/General image.json"),
            crate::files::preset_json(&c).unwrap(),
        )
        .unwrap();
        assert!(s
            .save_preset("General image", &c, (256, 224), true)
            .is_err());
        // It is still the person's own to delete.
        s.delete_preset("general image").unwrap();
        assert!(s.presets().unwrap().0.is_empty());
    }

    #[test]
    fn a_description_left_behind_never_reaches_a_new_preset() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path());
        let c = Config::general();
        // As a deletion that stopped after the settings, before this one: only the
        // description is left, under another letter case.
        s.save_preset("Gone", &c, (256, 224), false).unwrap();
        s.set_description("Gone", "Not yours").unwrap();
        std::fs::remove_file(temp.path().join("presets/Gone.json")).unwrap();
        assert!(
            s.set_description("Gone", "x").is_err(),
            "no preset to describe"
        );
        s.save_preset("gone", &c, (256, 224), false).unwrap();
        let presets = s.presets().unwrap().0;
        assert_eq!(presets[0].name, "gone");
        assert_eq!(presets[0].description, "");
        assert!(!temp.path().join("presets/Gone.txt").exists());
    }

    #[test]
    fn a_preset_is_deleted_by_any_letter_case_with_its_description() {
        let temp = tempfile::tempdir().unwrap();
        let s = store(temp.path());
        let c = Config::general();
        s.save_preset("My CRT", &c, (256, 224), false).unwrap();
        s.set_description("my crt", "Mine").unwrap();
        assert!(temp.path().join("presets/My CRT.txt").exists());
        s.delete_preset("MY CRT").unwrap();
        let left: Vec<_> = std::fs::read_dir(temp.path().join("presets"))
            .unwrap()
            .collect();
        assert!(left.is_empty(), "{left:?}");
        // A preset put there by hand, with its extension in capitals, is listed, found,
        // replaced in place and deleted like any other.
        std::fs::write(
            temp.path().join("presets/Shouty.JSON"),
            crate::files::preset_json(&c).unwrap(),
        )
        .unwrap();
        assert_eq!(s.saved_preset("shouty").unwrap().as_deref(), Some("Shouty"));
        assert_eq!(
            s.save_preset("shouty", &Config::default(), (256, 224), true)
                .unwrap(),
            "Shouty"
        );
        let presets = s.presets().unwrap().0;
        assert_eq!(presets.len(), 1, "replaced, not saved beside it");
        assert_eq!(presets[0].config, Config::default());
        s.delete_preset("Shouty").unwrap();
        assert!(s.presets().unwrap().0.is_empty());
    }

    #[test]
    fn recent_projects_and_the_session_persist_before_the_folder_exists() {
        let temp = tempfile::tempdir().unwrap();
        // A first run: nothing has made the folder yet, since the welcome is still showing.
        let folder = temp.path().join("not yet made");
        let s = store(&folder);
        assert!(s.recent_projects().unwrap().is_empty());
        assert!(s.session().unwrap().is_none());
        let paths: Vec<PathBuf> = (0..12).map(|n| format!("{n}.crtsim").into()).collect();
        s.set_recent_projects(&paths).unwrap();
        assert_eq!(s.recent_projects().unwrap(), paths[..RECENT_PROJECTS]);
        let session = project::Project {
            version: 1,
            source: None,
            built_in: project::BuiltIn::VideoTestCard,
            frame: 3,
            config: Config::general(),
            options: Default::default(),
            queue: vec![],
        };
        s.set_session(&session).unwrap();
        assert_eq!(s.session().unwrap(), Some(session));
        std::fs::write(folder.join(RECENT), "x".repeat(64 * 1024 + 1)).unwrap();
        assert!(s.recent_projects().is_err());
        std::fs::write(folder.join(SESSION), "not a project").unwrap();
        assert!(s.session().is_err());
    }
}
