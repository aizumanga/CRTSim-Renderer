//! Runs the ReShade port in vkBasalt, the Vulkan layer that runs ReShade FX effects on Linux:
//! this crate's `present` binary shows each picture in an X window for its frames, vkBasalt
//! draws a look's effect over every frame as it is presented, and the binary reads back what
//! the window shows. It needs an X display at depth 24, which Xvfb provides, and Vulkan.
//!
//! vkBasalt reads a look's settings from its own configuration rather than from a ReShade
//! preset: each uniform by its name, as a specialization constant, and no preprocessor
//! definitions at all, so the look's definitions are written into an effect that includes the
//! port's. And it tells the effect `frametime` as it measured it, between presents, so the
//! effect it runs is told instead how long the case says each frame lasts.

use anyhow::{bail, ensure, Context, Result};
use crtsim_core::config::Config;
use crtsim_shader_ports as ports;
use image::RgbaImage;
use std::path::{Path, PathBuf};
use std::process::Command;

const SHADERS: &str = "reshade-shaders/Shaders";
const TEXTURES: &str = "reshade-shaders/Textures";
const EFFECT: &str = "CRTSimRenderer.fx";
/// The port's frame time, which vkBasalt measures.
const FRAME_TIME: &str = "uniform float crtsim_frame_time < source = \"frametime\"; >;";

/// The `present` binary, run with vkBasalt enabled.
pub struct Layer {
    present: PathBuf,
}

impl Layer {
    /// vkBasalt, whose layer `manifest` is installed where the Vulkan loader finds it,
    /// presenting through `present`, this crate's binary.
    pub fn new(manifest: &Path, present: &Path) -> Result<Self> {
        let layer = std::fs::read_to_string(manifest)
            .with_context(|| format!("vkBasalt is not installed: no {}", manifest.display()))?;
        ensure!(
            layer.contains("\"ENABLE_VKBASALT\""),
            "{} is not vkBasalt's layer",
            manifest.display()
        );
        ensure!(present.exists(), "{} is not built", present.display());
        Ok(Self {
            present: present.into(),
        })
    }

    /// Shows each picture for its frames, each lasting `frame_seconds`, through the effect set
    /// to `look`, and returns the window as the last left it. Works in `folder`, which must not
    /// exist yet.
    pub fn render(
        &self,
        look: &Config,
        steps: &[(RgbaImage, u32)],
        frame_seconds: f64,
        folder: &Path,
    ) -> Result<RgbaImage> {
        let size = steps.first().context("no picture to show")?.0.dimensions();
        ensure!(
            steps
                .iter()
                .all(|(picture, _)| picture.dimensions() == size),
            "a window shows pictures of one size"
        );
        let mut files = ports::reshade::export(&[("look", look)])?;
        let effect = files
            .iter_mut()
            .find(|file| file.path == format!("{SHADERS}/{EFFECT}"))
            .context("the export has no effect")?;
        let source = String::from_utf8(effect.bytes.clone())?;
        ensure!(
            source.contains(FRAME_TIME),
            "the effect reads no frame time"
        );
        let milliseconds = (frame_seconds * 1000.) as f32;
        effect.bytes = source
            .replace(
                FRAME_TIME,
                &format!("static const float crtsim_frame_time = {milliseconds:?};"),
            )
            .into();
        let preset = files
            .iter()
            .find(|file| file.path == "look.ini")
            .context("the export has no look.ini")?;
        let preset = Preset::read(std::str::from_utf8(&preset.bytes)?)?;
        let defined: String = preset
            .definitions
            .iter()
            .map(|(name, value)| format!("#define {name} {value}\n"))
            .collect();
        files.push(ports::File {
            path: format!("{SHADERS}/look.fx"),
            bytes: format!(
                "// The look's preprocessor definitions, which vkBasalt cannot set.\n\
                 {defined}#include \"{EFFECT}\"\n"
            )
            .into(),
        });
        ports::write(folder, &files)?;
        // Paths quoted, since vkBasalt drops every space outside quotes.
        let mut config = format!(
            "effects = look\nlook = \"{}\"\nreshadeIncludePath = \"{}\"\n\
             reshadeTexturePath = \"{}\"\ndepthCapture = off\nenableOnLaunch = True\n",
            folder.join(SHADERS).join("look.fx").display(),
            folder.join(SHADERS).display(),
            folder.join(TEXTURES).display(),
        );
        for (name, value) in &preset.uniforms {
            config.push_str(&format!("{name} = {value}\n"));
        }
        std::fs::write(folder.join("vkBasalt.conf"), config)?;
        let shown = folder.join("shown.png");
        let mut command = Command::new(&self.present);
        command
            .current_dir(folder)
            .env("ENABLE_VKBASALT", "1")
            .env("VKBASALT_CONFIG_FILE", folder.join("vkBasalt.conf"))
            .env("VKBASALT_LOG_LEVEL", "debug")
            .arg(&shown);
        for (index, (picture, frames)) in steps.iter().enumerate() {
            let file = folder.join(format!("picture-{index}.png"));
            picture.save(&file)?;
            command.arg(file).arg(frames.to_string());
        }
        let output = command.output().context("cannot run present")?;
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(folder.join("present.log"), &log)?;
        ensure!(output.status.success(), "present failed:\n{log}");
        // vkBasalt carries on without an effect it cannot build, and says so only in its log.
        let errors: Vec<_> = log
            .lines()
            .filter(|line| line.contains("vkBasalt err"))
            .collect();
        ensure!(errors.is_empty(), "vkBasalt complained: {errors:#?}\n{log}");
        ensure!(
            log.contains("created ReshadeEffect"),
            "vkBasalt made no effect:\n{log}"
        );
        let picture = image::open(&shown)
            .with_context(|| format!("present saved nothing:\n{log}"))?
            .to_rgba8();
        ensure!(
            picture.dimensions() == size,
            "the window showed {:?}, not {size:?}",
            picture.dimensions()
        );
        Ok(picture)
    }
}

/// A ReShade preset's preprocessor definitions and the uniforms of its one effect.
struct Preset {
    definitions: Vec<(String, String)>,
    uniforms: Vec<(String, String)>,
}

impl Preset {
    fn read(ini: &str) -> Result<Self> {
        let mut definitions = vec![];
        let mut uniforms = vec![];
        let mut section = String::new();
        for line in ini.lines().map(str::trim).filter(|line| !line.is_empty()) {
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.into();
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .with_context(|| format!("{line:?} is not a setting"))?;
            match (section.as_str(), key) {
                ("", "PreprocessorDefinitions") => {
                    for definition in value.split(',').filter(|d| !d.is_empty()) {
                        let (name, value) = definition
                            .split_once('=')
                            .with_context(|| format!("{definition:?} defines nothing"))?;
                        definitions.push((name.into(), value.into()));
                    }
                }
                ("", _) => {}
                (EFFECT, _) => uniforms.push((key.into(), value.into())),
                _ => bail!("the preset sets another effect, {section}"),
            }
        }
        Ok(Self {
            definitions,
            uniforms,
        })
    }
}
