//! Files coming in: what a file picked or dropped for a LUT or a preset brings, read the same
//! way whichever way it came. On the desktop a file is a path; in a browser, the name and bytes
//! the browser handed over. Each is read only up to the size its kind allows, then checked,
//! and what it brings is applied in one place (`App::bring_in`).
use crate::files;
use anyhow::{ensure, Context, Result};
use crtsim_core::config::Config;
use std::{
    borrow::Cow,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A file coming in.
pub enum File {
    /// On disk, as a desktop's dialog chooses it.
    Path(PathBuf),
    /// Handed over by a browser: its name and contents.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop reads files by path")
    )]
    Bytes { name: String, bytes: Vec<u8> },
}

impl File {
    /// What to call it: its file name.
    pub fn name(&self) -> String {
        match self {
            Self::Path(path) => crate::file_name(path),
            Self::Bytes { name, .. } => crate::file_name(Path::new(name)),
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Path(path) => path,
            Self::Bytes { name, .. } => Path::new(name),
        }
    }

    /// Its contents, if they are at most `limit` bytes; `what` is what it is, for the error.
    fn read(&self, limit: u64, what: &str) -> Result<Cow<'_, [u8]>> {
        let too_large = || format!("{what} exceeds {} MB", limit / (1024 * 1024));
        match self {
            Self::Path(path) => {
                let file = std::fs::File::open(path)
                    .with_context(|| format!("Cannot read {}", crate::file_name(path)))?;
                ensure!(file.metadata()?.len() <= limit, too_large());
                let mut bytes = vec![];
                file.take(limit + 1).read_to_end(&mut bytes)?;
                ensure!(bytes.len() as u64 <= limit, too_large());
                Ok(Cow::Owned(bytes))
            }
            Self::Bytes { bytes, .. } => {
                ensure!(bytes.len() as u64 <= limit, too_large());
                Ok(Cow::Borrowed(bytes))
            }
        }
    }
}

/// What a file is brought in for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A 3D `.cube` LUT, added to the settings in use.
    Lut,
    /// A preset saved as JSON.
    LoadPreset,
    /// The preset a rendered PNG, or a video, carries. The desktop reads it on its worker.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop imports presets on its worker")
    )]
    ImportPreset,
}

/// What a file brings: the settings to use, the video export settings a video was made with,
/// the name a preset saved next is offered, and what the status bar says.
#[derive(Debug, PartialEq)]
pub struct Brought {
    pub config: Config,
    pub options: Option<crtsim_media::Options>,
    pub preset_name: Option<String>,
    pub status: String,
}

const LUT_LIMIT: u64 = 16 * 1024 * 1024;
const PRESET_LIMIT: u64 = 32 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 512 * 1024 * 1024;

/// What `file`, brought in for `purpose`, brings to the settings in use, `current`, for a
/// source of size `input`; or why it cannot, as the status bar's error says it.
pub fn bring(
    purpose: Purpose,
    file: &File,
    current: &Config,
    input: (u32, u32),
) -> Result<Brought, String> {
    let name = file.name();
    let failed = |verb: &str, error: anyhow::Error| format!("Cannot {verb}: {error:#}");
    match purpose {
        Purpose::Lut => lut(file, current).map_err(|e| failed("import LUT", e)),
        Purpose::LoadPreset => {
            let read = file.read(PRESET_LIMIT, "Preset");
            let config = read
                .and_then(|bytes| files::preset_from_json(&bytes, input))
                .map_err(|e| failed("load preset", e))?;
            Ok(Brought::preset(
                config,
                None,
                &name,
                format!("Loaded preset {name}"),
            ))
        }
        Purpose::ImportPreset => {
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let (config, options) =
                imported(file, input, &cancel).map_err(|e| failed("import preset", e))?;
            Ok(Brought::imported(config, options, &name))
        }
    }
}

/// The preset a rendered PNG or a video carries, and a video's export settings. A video on
/// disk is read by FFmpeg, which `cancel` stops; the desktop's worker does that.
pub fn imported(
    file: &File,
    input: (u32, u32),
    cancel: &Arc<std::sync::atomic::AtomicBool>,
) -> Result<(Config, Option<crtsim_media::Options>)> {
    let path = file.path();
    let preset = match file {
        // Only a PNG is read as an image on the desktop; FFmpeg reads any other file's
        // comment, a video's, without reading the whole of it.
        File::Path(path)
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png")) =>
        {
            Some(crtsim_media::import_preset(path, input, cancel)?)
        }
        // A browser hands over the whole file, which is read here when it is a video.
        File::Bytes { bytes, .. }
            if crtsim_media::MediaKind::of(path) == crtsim_media::MediaKind::Video =>
        {
            Some(crtsim_media::import_preset_from_bytes(path, bytes, input)?)
        }
        _ => None,
    };
    if let Some(preset) = preset {
        return Ok((preset.config, Some(preset.video_options)));
    }
    let bytes = file.read(IMAGE_LIMIT, "Image file")?;
    Ok((files::preset_from_png(&bytes, input)?, None))
}

/// The settings in use with the LUT `file` holds.
fn lut(file: &File, current: &Config) -> Result<Brought> {
    let bytes = file.read(LUT_LIMIT, "LUT")?;
    let text = std::str::from_utf8(&bytes).context("A .cube LUT is text")?;
    let lut = crtsim_core::workflow::Lut::parse_cube(file.name(), text)?;
    let mut config = current.clone();
    config.set_lut(Some(Arc::new(lut)));
    Ok(Brought {
        config,
        options: None,
        preset_name: None,
        status: "LUT imported".into(),
    })
}

impl Brought {
    /// What a preset imported from the file called `name` brings.
    pub fn imported(config: Config, options: Option<crtsim_media::Options>, name: &str) -> Self {
        Self::preset(
            config,
            options,
            name,
            format!("Imported preset from {name}"),
        )
    }

    fn preset(
        config: Config,
        options: Option<crtsim_media::Options>,
        name: &str,
        status: String,
    ) -> Self {
        Self {
            config,
            options,
            preset_name: Some(crate::file_stem(Path::new(name))),
            status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `file` as a path and as the same bytes a browser would hand over.
    fn both_ways(path: &Path) -> [File; 2] {
        let bytes = std::fs::read(path).unwrap();
        [
            File::Path(path.to_owned()),
            File::Bytes {
                name: crate::file_name(path),
                bytes,
            },
        ]
    }

    #[test]
    fn a_file_brings_the_same_by_path_or_as_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let input = (1216, 832);
        let saved = Config {
            bloom: 0.5,
            ..Config::general()
        };
        let json = dir.path().join("my look.json");
        files::save_preset(&json, &saved).unwrap();
        let png = dir.path().join("render.png");
        files::save_png(&png, crtsim_core::config::test_card(), Some(&saved)).unwrap();
        let cube = dir.path().join("warm.cube");
        std::fs::write(
            &cube,
            "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n",
        )
        .unwrap();
        let current = Config::general();
        for (purpose, path, status) in [
            (Purpose::LoadPreset, &json, "Loaded preset my look.json"),
            (
                Purpose::ImportPreset,
                &png,
                "Imported preset from render.png",
            ),
            (Purpose::Lut, &cube, "LUT imported"),
        ] {
            let [by_path, as_bytes] = both_ways(path);
            let by_path = bring(purpose, &by_path, &current, input).unwrap();
            assert_eq!(by_path, bring(purpose, &as_bytes, &current, input).unwrap());
            assert_eq!(by_path.status, status, "{purpose:?}");
            match purpose {
                Purpose::Lut => {
                    assert!(by_path.config.lut.is_some() && by_path.preset_name.is_none());
                }
                _ => {
                    assert_eq!(by_path.config, saved);
                    assert!(by_path.preset_name.is_some());
                }
            }
        }
    }

    /// A video's preset, as a browser hands it over, brings its video export settings too.
    #[test]
    fn a_preset_imports_from_a_picked_video() {
        use crtsim_media::mux;
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
        let webm = |comment: Option<&str>| {
            mux::write(crtsim_media::Container::Webm, &video, None, comment).unwrap()
        };
        let comment = format!("CRTSim-Renderer-Preset:{preset}");
        let file = File::Bytes {
            name: "made.webm".into(),
            bytes: webm(Some(&comment)),
        };
        let current = Config::general();
        let brought = bring(Purpose::ImportPreset, &file, &current, (256, 224)).unwrap();
        assert_eq!(brought.config, config);
        assert_eq!(brought.options.unwrap().audio, crtsim_media::Audio::Mute);
        assert_eq!(brought.preset_name.as_deref(), Some("made"));
        // A video without one says so.
        let plain = File::Bytes {
            name: "plain.webm".into(),
            bytes: webm(None),
        };
        let error = bring(Purpose::ImportPreset, &plain, &current, (256, 224)).unwrap_err();
        assert!(error.contains("does not contain"), "{error}");
    }

    #[test]
    fn a_file_too_large_or_not_what_it_should_be_says_why() {
        let current = Config::general();
        let large = File::Bytes {
            name: "huge.cube".into(),
            bytes: vec![b' '; LUT_LIMIT as usize + 1],
        };
        let error = bring(Purpose::Lut, &large, &current, (1, 1)).unwrap_err();
        assert_eq!(error, "Cannot import LUT: LUT exceeds 16 MB");
        let binary = File::Bytes {
            name: "odd.cube".into(),
            bytes: vec![0xff, 0xfe],
        };
        let error = bring(Purpose::Lut, &binary, &current, (1, 1)).unwrap_err();
        assert!(
            error.starts_with("Cannot import LUT: A .cube LUT is text"),
            "{error}"
        );
        let plain = File::Bytes {
            name: "plain.png".into(),
            bytes: files::png_bytes(image::RgbaImage::new(1, 1), None).unwrap(),
        };
        let error = bring(Purpose::ImportPreset, &plain, &current, (1, 1)).unwrap_err();
        assert!(error.starts_with("Cannot import preset: "), "{error}");
        let missing = File::Path("/nowhere/look.json".into());
        let error = bring(Purpose::LoadPreset, &missing, &current, (1, 1)).unwrap_err();
        assert!(
            error.starts_with("Cannot load preset: Cannot read"),
            "{error}"
        );
    }
}
