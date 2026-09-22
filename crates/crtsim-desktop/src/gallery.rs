use anyhow::{ensure, Context, Result};
use crtsim_core::config::{ColorMode, Config, Filter};
use std::path::{Path, PathBuf};

use crate::theme::Theme;

pub const DISCLAIMER: &str = "This project is what some would call \"vibe-coded slop\", built based on J. Kyle Pittman's public CRTSim. The original CRT simulation, shaders, textures and meshes are his work; this project's AI-assisted renderer port and interface are separate additions. This is an unofficial project, not made or endorsed by him.";
pub const SUPPORT: &str = "Please support J. Kyle Pittman and Minor Key Games: buy and play their games on itch.io and Steam.";
pub const ITCH: &str = "https://piratehearts.itch.io/";
pub const STEAM: &str = "https://store.steampowered.com/developer/MinorKeyGames";
pub const ARTICLE: &str =
    "https://www.gamedeveloper.com/programming/crt-simulation-in-super-win-the-game";

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub config: Config,
    pub user: bool,
}
fn entry(name: &str, description: &str, config: Config) -> Entry {
    Entry {
        name: name.into(),
        description: description.into(),
        config,
        user: false,
    }
}
pub fn builtins() -> Vec<Entry> {
    let general = crate::model::general();
    vec![
        entry(
            "General image",
            "Square pixels, smooth resizing, balanced CRT effects.",
            general.clone(),
        ),
        entry(
            "Original CRTSim",
            "Public-reference defaults, 256×224 signal, original mask sampling.",
            Config::default(),
        ),
        entry(
            "Soft television",
            "Gentler mask and ringing for illustrations and photos.",
            Config {
                mask_opacity: 0.45,
                artifacts: 0.2,
                sharpness: 0.25,
                bloom: 0.16,
                ..general.clone()
            },
        ),
        entry(
            "Clean RGB",
            "No composite artifacts or persistence; a restrained mask.",
            Config {
                artifacts: 0.,
                sharpness: 0.,
                bleed: 0.,
                persistence: [0.; 3],
                mask_opacity: 0.55,
                bloom: 0.08,
                ..general.clone()
            },
        ),
        entry(
            "Pixel art 240p",
            "Explicit 240-row signal, nearest resizing and square pixels.",
            Config {
                signal: "240p".into(),
                filter: Filter::Nearest,
                mask_opacity: 0.7,
                ..general.clone()
            },
        ),
        entry(
            "Warm analog",
            "An optional hue/chroma grade, not the unpublished game palette.",
            Config {
                hue: -8.,
                chroma: 0.85,
                artifacts: 0.65,
                mask_opacity: 0.65,
                ..general.clone()
            },
        ),
        entry(
            "Linear light",
            "Experimental linear-light glass and bloom with float intermediates.",
            Config {
                color_mode: ColorMode::LinearLight,
                bloom: 0.08,
                diffuse: 0.12,
                specular: 0.08,
                rim: 0.25,
                mask_opacity: 0.65,
                ..general
            },
        ),
    ]
}

/// Remembered tool window placements, keyed by window title.
pub type Layout = std::collections::BTreeMap<String, crate::chrome::ToolWindow>;

#[derive(Clone)]
pub struct Store {
    pub root: PathBuf,
}
impl Store {
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
    pub fn welcome_needed(&self) -> bool {
        std::fs::read(self.root.join("welcome-v1.txt"))
            .ok()
            .as_deref()
            != Some(b"acknowledged\n")
    }
    pub fn acknowledge(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join("welcome-v1.txt");
        // Atomic replacement of app-owned state. Personal preset files use no-clobber saves.
        let mut file = tempfile::NamedTempFile::new_in(&self.root)?;
        use std::io::Write;
        file.write_all(b"acknowledged\n")?;
        file.as_file_mut().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
    pub fn theme(&self) -> Result<Option<Theme>> {
        let path = self.root.join("theme-v1.txt");
        if !path.exists() {
            return Ok(None);
        }
        ensure!(path.metadata()?.len() <= 64, "Theme setting is too large");
        let value = std::fs::read_to_string(path)?;
        let id = value.trim();
        Theme::from_id(id)
            .map(Some)
            .with_context(|| format!("Unknown saved theme {id:?}"))
    }
    pub fn set_theme(&self, theme: Theme) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        crate::files::save_atomic(&self.root.join("theme-v1.txt"), |file| {
            use std::io::Write;
            writeln!(file, "{}", theme.id())?;
            Ok(())
        })
    }
    /// Tool window placements from the last run, keyed by window title. A missing or
    /// unreadable file is not worth an error: windows simply open at their default size.
    pub fn tool_windows(&self) -> Result<Layout> {
        let path = self.root.join("windows-v1.json");
        if !path.exists() {
            return Ok(Layout::new());
        }
        ensure!(
            path.metadata()?.len() <= 8192,
            "Saved window layout is too large"
        );
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
    pub fn set_tool_windows(&self, windows: &Layout) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        crate::files::save_atomic(&self.root.join("windows-v1.json"), |file| {
            serde_json::to_writer_pretty(file, windows)?;
            Ok(())
        })
    }
    pub fn scan(&self) -> Result<(Vec<Entry>, Vec<String>)> {
        let dir = self.root.join("presets");
        if !dir.exists() {
            return Ok((vec![], vec![]));
        }
        let mut entries = vec![];
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
            let name = item
                .path()
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            match crate::files::load_preset(&item.path(), (256, 224)) {
                Ok(config) => entries.push(Entry {
                    name: name.clone(),
                    description: self.description(&name).unwrap_or_default(),
                    config,
                    user: true,
                }),
                Err(e) => warnings.push(format!("{name}: {e:#}")),
            }
        }
        Ok((entries, warnings))
    }
    fn description(&self, name: &str) -> Result<String> {
        let path = self.root.join("presets").join(format!("{name}.txt"));
        if !path.exists() {
            return Ok(String::new());
        }
        ensure!(
            path.metadata()?.len() <= 4096,
            "Description exceeds 4096 bytes"
        );
        Ok(std::fs::read_to_string(path)?)
    }
    pub fn set_description(&self, name: &str, description: &str) -> Result<()> {
        validate_name(name)?;
        ensure!(description.len() <= 4096, "Description exceeds 4096 bytes");
        let dir = self.root.join("presets");
        ensure!(
            dir.join(format!("{name}.json")).is_file(),
            "Preset no longer exists"
        );
        // Keep the JSON compatible with older versions and the CLI.
        crate::files::save_atomic(&dir.join(format!("{name}.txt")), |file| {
            use std::io::Write;
            file.write_all(description.as_bytes())?;
            Ok(())
        })
    }
    pub fn save(&self, name: &str, config: &Config, input: (u32, u32)) -> Result<()> {
        validate_name(name)?;
        config.validate()?;
        config.signal_size(input)?;
        config.output_size(input)?;
        let dir = self.root.join("presets");
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
    #[test]
    fn gallery_persists_and_bad_files_do_not_hide_good_presets() {
        let temp = tempfile::tempdir().unwrap();
        let s = Store {
            root: temp.path().to_path_buf(),
        };
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
        std::fs::write(temp.path().join("windows-v1.json"), "x".repeat(8193)).unwrap();
        assert!(s.tool_windows().is_err());
        s.set_tool_windows(&Layout::new()).unwrap();
        assert!(s.tool_windows().unwrap().is_empty());
        let c = crate::model::general();
        s.save("My CRT", &c, (1216, 832)).unwrap();
        assert!(s.save("my crt", &c, (1, 1)).is_err());
        for name in ["../oops", "CON", "", "nested/file", "trailing "] {
            assert!(s.save(name, &c, (1, 1)).is_err());
        }
        std::fs::write(temp.path().join("presets/broken.json"), "not json").unwrap();
        let reopened = Store {
            root: temp.path().to_path_buf(),
        };
        let (entries, warnings) = reopened.scan().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].config, c);
        assert_eq!(entries[0].description, "");
        s.set_description("My CRT", "Soft mask — for animation\nMy own look")
            .unwrap();
        let updated = reopened.scan().unwrap().0;
        assert_eq!(
            updated[0].description,
            "Soft mask — for animation\nMy own look"
        );
        assert_eq!(updated[0].config, c);
        assert!(s.set_description("../outside", "oops").is_err());
        assert!(s.set_description("My CRT", &"x".repeat(4097)).is_err());
        s.set_description("My CRT", "").unwrap();
        assert_eq!(reopened.scan().unwrap().0[0].description, "");
        assert_eq!(warnings.len(), 1);
        for e in builtins() {
            e.config.validate().unwrap();
            e.config.signal_size((1216, 832)).unwrap();
        }
    }
}
