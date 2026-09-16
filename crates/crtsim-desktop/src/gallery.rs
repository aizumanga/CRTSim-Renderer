use anyhow::{ensure, Context, Result};
use crtsim_core::config::{ColorMode, Config, Filter};
use std::path::{Path, PathBuf};

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
                    name,
                    description: "Saved on this computer".into(),
                    config,
                    user: true,
                }),
                Err(e) => warnings.push(format!("{name}: {e:#}")),
            }
        }
        Ok((entries, warnings))
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
        assert_eq!(warnings.len(), 1);
        for e in builtins() {
            e.config.validate().unwrap();
            e.config.signal_size((1216, 832)).unwrap();
        }
    }
}
