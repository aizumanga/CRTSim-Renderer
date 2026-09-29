//! Looks as RetroArch shader presets: the shader port in `ports/retroarch`, with a `.slangp`
//! per look that sets its parameters and names its colour table. See
//! [the decision](../../../docs/adr/0003-games-get-shader-ports-not-the-renderer.md).
//!
//! The port draws what the renderer draws, pass for pass, at the size the host gives it: the
//! core's own picture is the signal, and the screen is the viewport. What the port leaves to
//! RetroArch -- resizing, source edits, the export size and warm-up -- a preset does not hold.
//! Until the bezel is ported, every look is drawn screen-only.

use crate::config::{ColorMode, Config, Fit, MaskRepeats, Phase};
use crate::settings::{Control, SETTINGS};
use crate::workflow::Lut;
use anyhow::{ensure, Result};
use image::{ImageFormat, Rgba, RgbaImage};
use std::fmt::Write as _;

/// Where a preset finds the port's shaders and textures, relative to it.
const SHADERS: &str = "shaders/crtsim-renderer";

/// The port's sources, as `ports/retroarch` holds them.
const SOURCES: &[(&str, &str)] = &[
    (
        "common.inc",
        include_str!("../../../ports/retroarch/common.inc"),
    ),
    (
        "bloom.inc",
        include_str!("../../../ports/retroarch/bloom.inc"),
    ),
    (
        "prepare.slang",
        include_str!("../../../ports/retroarch/prepare.slang"),
    ),
    (
        "composite.slang",
        include_str!("../../../ports/retroarch/composite.slang"),
    ),
    (
        "glass.slang",
        include_str!("../../../ports/retroarch/glass.slang"),
    ),
    (
        "bloom-down.slang",
        include_str!("../../../ports/retroarch/bloom-down.slang"),
    ),
    (
        "bloom-up.slang",
        include_str!("../../../ports/retroarch/bloom-up.slang"),
    ),
    (
        "present.slang",
        include_str!("../../../ports/retroarch/present.slang"),
    ),
];

const README: &str = include_str!("../../../ports/retroarch/README.md");

/// The looks a release ships: the original's settings, the same in linear light, the app's
/// starting point for ordinary images, and the original with the NES palette.
pub fn shipped() -> Vec<(String, Config)> {
    let reference = Config::default();
    vec![
        ("crtsim-renderer".into(), reference.clone()),
        (
            "crtsim-renderer-linear".into(),
            Config {
                color_mode: ColorMode::LinearLight,
                ..reference.clone()
            },
        ),
        ("crtsim-renderer-general".into(), Config::general()),
        (
            "crtsim-renderer-nes-palette".into(),
            Config {
                palette: Some(Default::default()),
                ..reference
            },
        ),
    ]
}

/// A file of an export, at its path relative to the folder it is written into.
pub struct File {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The files that make `looks` RetroArch presets, each look named by its preset's file name
/// without `.slangp`. They share one copy of the shaders.
pub fn export(looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    ensure!(!looks.is_empty(), "Choose at least one look to export");
    let mut files = vec![
        text("README.md", README),
        text(&format!("{SHADERS}/parameters.inc"), &pragmas()),
        png(
            &format!("{SHADERS}/artifacts.png"),
            &crate::gpu::artifacts_image()?,
        )?,
        png(&format!("{SHADERS}/mask.png"), &crate::gpu::mask_image()?)?,
        png(
            &format!("{SHADERS}/no-table.png"),
            &strip(&identity_table()),
        )?,
    ];
    files.extend(
        SOURCES
            .iter()
            .map(|(name, source)| text(&format!("{SHADERS}/{name}"), source)),
    );
    for (name, config) in looks {
        ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
            "A look's name must be letters, digits, - and _: {name:?}"
        );
        config.validate()?;
        let table = match config.lut_in_use() {
            Some(lut) => {
                let path = format!("{name}-table.png");
                files.push(png(&path, &strip(&lut))?);
                path
            }
            None => format!("{SHADERS}/no-table.png"),
        };
        files.push(text(&format!("{name}.slangp"), &preset(config, &table)));
    }
    Ok(files)
}

fn text(path: &str, text: &str) -> File {
    File {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
    }
}

fn png(path: &str, image: &RgbaImage) -> Result<File> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png)?;
    Ok(File {
        path: path.into(),
        bytes: bytes.into_inner(),
    })
}

/// One of the port's parameters: what RetroArch's menu shows, and where its value comes from.
struct Parameter {
    name: &'static str,
    label: &'static str,
    min: f32,
    max: f32,
    step: f32,
    value: Value,
}

enum Value {
    /// The `index`th number of a setting in `SETTINGS`.
    Setting { key: &'static str, index: usize },
    /// Worked out from settings the table does not hold as numbers.
    Derived(fn(&Config) -> f32),
}

/// The settings the port reads as numbers, by key, with a parameter name for each value.
const NUMBERS: &[(&str, &[&str])] = &[
    ("lut_strength", &["CRTSIM_LUT_STRENGTH"]),
    ("hue", &["CRTSIM_HUE"]),
    ("chroma", &["CRTSIM_CHROMA"]),
    ("saturation", &["CRTSIM_SATURATION"]),
    ("sharpness", &["CRTSIM_SHARPNESS"]),
    ("bleed", &["CRTSIM_BLEED"]),
    ("artifacts", &["CRTSIM_ARTIFACTS"]),
    ("ntsc_blending", &["CRTSIM_NTSC_BLENDING"]),
    ("barrel", &["CRTSIM_BARREL"]),
    ("overscan", &["CRTSIM_OVERSCAN"]),
    ("dimming", &["CRTSIM_DIMMING"]),
    ("fov", &["CRTSIM_FOV"]),
    ("mask_opacity", &["CRTSIM_MASK_OPACITY"]),
    ("mask_brightness", &["CRTSIM_MASK_BRIGHTNESS"]),
    ("bloom", &["CRTSIM_BLOOM"]),
    ("bloom_power", &["CRTSIM_BLOOM_POWER"]),
    ("bloom_spread", &["CRTSIM_BLOOM_SPREAD"]),
    ("diffuse", &["CRTSIM_DIFFUSE"]),
    ("specular", &["CRTSIM_SPECULAR"]),
    ("specular_power", &["CRTSIM_SPECULAR_POWER"]),
    ("rim", &["CRTSIM_RIM"]),
    (
        "light_position",
        &["CRTSIM_LIGHT_X", "CRTSIM_LIGHT_Y", "CRTSIM_LIGHT_Z"],
    ),
    (
        "persistence",
        &[
            "CRTSIM_PERSISTENCE_R",
            "CRTSIM_PERSISTENCE_G",
            "CRTSIM_PERSISTENCE_B",
        ],
    ),
];

fn flag(on: bool) -> f32 {
    if on {
        1.
    } else {
        0.
    }
}

/// Every parameter, in the order RetroArch's menu lists them.
fn parameters() -> Vec<Parameter> {
    let choice = |name, label, max: f32, value| Parameter {
        name,
        label,
        min: 0.,
        max,
        step: 1.,
        value: Value::Derived(value),
    };
    let mut all = vec![
        choice("CRTSIM_TABLE", "Colour table (off, on)", 1., |c| {
            flag(c.lut_in_use().is_some())
        }),
        choice(
            "CRTSIM_LINEAR",
            "Linear light (off, on; the linear preset's buffers)",
            1.,
            |c| flag(c.color_mode == ColorMode::LinearLight),
        ),
        choice(
            "CRTSIM_PHASE",
            "Phase (stable, A, B, alternating)",
            3.,
            |c| match c.phase {
                Phase::Stable => 0.,
                Phase::A => 1.,
                Phase::B => 2.,
                Phase::Alternating => 3.,
            },
        ),
        // A look made on a still says nothing about a core that switches to 480 rows, so a
        // look that does not interlace leaves it to the picture's size.
        choice(
            "CRTSIM_INTERLACE",
            "Interlaced fields (off, on, auto)",
            2.,
            |c| if c.interlace { 1. } else { 2. },
        ),
        choice(
            "CRTSIM_FIT",
            "Fit on 4:3 tube (original, contain, cover, stretch)",
            3.,
            |c| match c.fit {
                Fit::Reference => 0.,
                Fit::Contain => 1.,
                Fit::Cover => 2.,
                Fit::Stretch => 3.,
            },
        ),
        Parameter {
            name: "CRTSIM_ASPECT",
            label: "Picture aspect, when the core gives none",
            min: 0.25,
            max: 4.,
            step: 0.01,
            value: Value::Derived(|_| 4. / 3.),
        },
        choice(
            "CRTSIM_MASK_ANTIALIAS",
            "Filter mask when shrinking (off, on)",
            1.,
            |c| flag(c.mask_antialias),
        ),
        // Zero follows the picture: a column for every two of its columns, a row for each row.
        Parameter {
            name: "CRTSIM_MASK_COLUMNS",
            label: "Mask columns (0 follows the picture)",
            min: 0.,
            max: 4096.,
            step: 1.,
            value: Value::Derived(|c| c.mask_repeats.fixed().map_or(0., |[x, _]| x)),
        },
        Parameter {
            name: "CRTSIM_MASK_ROWS",
            label: "Mask rows (0 follows the picture)",
            min: 0.,
            max: 4096.,
            step: 1.,
            value: Value::Derived(|c| c.mask_repeats.fixed().map_or(0., |[_, y]| y)),
        },
    ];
    for (key, names) in NUMBERS {
        let setting = SETTINGS
            .iter()
            .find(|s| s.key == *key)
            .expect("a setting the port reads is in the table");
        let numbers = setting.numbers.as_ref().expect("a numeric setting");
        let (min, max) = match &numbers.control {
            Control::Slider { span, .. } => (*span.start(), *span.end()),
            Control::Color => (0., 1.),
        };
        for (index, name) in names.iter().enumerate() {
            all.push(Parameter {
                name,
                label: setting.value_name(index),
                min,
                max,
                step: step(min, max),
                value: Value::Setting { key, index },
            });
        }
    }
    all
}

/// A hundred steps across the range, rounded to one significant figure. Worked in f64, so
/// the f32 prints as that figure and RetroArch's menu shows it as written.
fn step(min: f32, max: f32) -> f32 {
    let raw = f64::from(max - min) / 100.;
    let scale = 10_f64.powf(raw.log10().floor());
    ((raw / scale).round() * scale) as f32
}

impl Parameter {
    fn of(&self, c: &Config) -> f32 {
        match self.value {
            Value::Setting { key, index } => {
                let setting = SETTINGS
                    .iter()
                    .find(|s| s.key == key)
                    .expect("in the table");
                let numbers = setting.numbers.as_ref().expect("a numeric setting");
                (numbers.access.get)(c)[index]
            }
            Value::Derived(value) => value(c),
        }
    }
}

impl MaskRepeats {
    fn fixed(self) -> Option<[f32; 2]> {
        match self {
            Self::Signal => None,
            Self::Fixed(repeats) => Some(repeats),
        }
    }
}

/// `parameters.inc`: every parameter, with the reference look's value as its default.
fn pragmas() -> String {
    let reference = Config::default();
    let mut out = String::from(
        "// CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not this.\n",
    );
    for p in parameters() {
        writeln!(
            out,
            "#pragma parameter {} \"{}\" {} {} {} {}",
            p.name,
            p.label,
            p.of(&reference),
            p.min,
            p.max,
            p.step
        )
        .expect("writing to a string");
    }
    out
}

/// The textures a preset loads. None may share a parameter's name: RetroArch and librashader
/// read a preset's `NAME = value` lines by name alone, so a clash turns one into the other.
const TEXTURES: [&str; 3] = [
    "CRTSIM_COLOUR_TABLE",
    "CRTSIM_ARTIFACT_PATTERN",
    "CRTSIM_SHADOW_MASK",
];

/// A pass of the preset: its shader and how RetroArch sizes, stores and samples it.
struct Pass {
    shader: &'static str,
    alias: Option<&'static str>,
    /// `source` or `viewport`, and the scale; the last pass has none and draws to the screen.
    scale: Option<(&'static str, f32)>,
    /// Glass, bloom and present sample what they read smoothly; the signal passes fetch texels.
    linear: bool,
    /// The passes after the signal keep float values in linear light, as the renderer does.
    surface: bool,
}

const PASSES: &[Pass] = &[
    Pass {
        shader: "prepare.slang",
        alias: None,
        scale: Some(("source", 1.)),
        linear: false,
        surface: false,
    },
    Pass {
        shader: "composite.slang",
        alias: Some("CRTSIM_SIGNAL"),
        scale: Some(("source", 1.)),
        linear: false,
        surface: false,
    },
    Pass {
        shader: "glass.slang",
        alias: Some("CRTSIM_GLASS"),
        scale: Some(("viewport", 1.)),
        linear: true,
        surface: true,
    },
    Pass {
        shader: "bloom-down.slang",
        alias: None,
        scale: Some(("viewport", 0.0625)),
        linear: true,
        surface: true,
    },
    Pass {
        shader: "bloom-up.slang",
        alias: None,
        scale: Some(("viewport", 1.)),
        linear: true,
        surface: true,
    },
    Pass {
        shader: "present.slang",
        alias: None,
        scale: None,
        linear: true,
        surface: false,
    },
];

/// A look's `.slangp`, reading its colour table from `table`.
fn preset(c: &Config, table: &str) -> String {
    let linear = c.color_mode == ColorMode::LinearLight;
    let mut out = String::from(
        "# CRTSim Renderer, a port of J. Kyle Pittman's CRTSim. CC0.\n\
         # Set Settings > Video > Scaling > Aspect Ratio to Full: the preset draws the tube.\n\n",
    );
    let mut line = |text: String| out.push_str(&(text + "\n"));
    line(format!("shaders = {}", PASSES.len()));
    for (i, pass) in PASSES.iter().enumerate() {
        line(String::new());
        line(format!("shader{i} = {SHADERS}/{}", pass.shader));
        if let Some(alias) = pass.alias {
            line(format!("alias{i} = {alias}"));
        }
        if let Some((kind, scale)) = pass.scale {
            line(format!("scale_type{i} = {kind}"));
            line(format!("scale{i} = {scale}"));
        }
        line(format!("filter_linear{i} = {}", pass.linear));
        line(format!("wrap_mode{i} = clamp_to_edge"));
        if pass.surface && linear {
            line(format!("float_framebuffer{i} = true"));
        }
    }
    line(String::new());
    line(format!("textures = \"{}\"", TEXTURES.join(";")));
    line(format!("CRTSIM_COLOUR_TABLE = {table}"));
    line("CRTSIM_COLOUR_TABLE_linear = false".into());
    line(format!("CRTSIM_ARTIFACT_PATTERN = {SHADERS}/artifacts.png"));
    line("CRTSIM_ARTIFACT_PATTERN_linear = false".into());
    line("CRTSIM_ARTIFACT_PATTERN_wrap_mode = repeat".into());
    line(format!("CRTSIM_SHADOW_MASK = {SHADERS}/mask.png"));
    // Filtered with mipmaps, so RetroArch and librashader both build each level as the
    // average of the one above, as the renderer does.
    line("CRTSIM_SHADOW_MASK_linear = true".into());
    line("CRTSIM_SHADOW_MASK_mipmap = true".into());
    line("CRTSIM_SHADOW_MASK_wrap_mode = repeat".into());
    let parameters = parameters();
    line(String::new());
    line(format!(
        "parameters = \"{}\"",
        parameters
            .iter()
            .map(|p| p.name)
            .collect::<Vec<_>>()
            .join(";")
    ));
    for p in &parameters {
        line(format!("{} = {}", p.name, p.of(c)));
    }
    out
}

/// A table's lattice as an image of `size` slices side by side: red across each slice, green
/// down it, blue from slice to slice. Sampled over a domain other than 0 to 1, the table is
/// first resampled onto that one, which is the only domain the port reads.
fn strip(lut: &Lut) -> RgbaImage {
    let n = lut.size;
    let last = (n - 1) as f32;
    RgbaImage::from_fn((n * n) as u32, n as u32, |x, y| {
        let (r, b, g) = (x as usize % n, x as usize / n, y as usize);
        let colour = [r, g, b].map(|v| v as f32 / last);
        let [r, g, b] = sample(lut, colour).map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        Rgba([r, g, b, 255])
    })
}

/// `lut` at `colour`, blended between its lattice as `Lut::apply_with_strength` blends.
fn sample(lut: &Lut, colour: [f32; 3]) -> [f32; 3] {
    let n = lut.size;
    let xyz: [f32; 3] = std::array::from_fn(|i| {
        ((colour[i] - lut.domain_min[i]) / (lut.domain_max[i] - lut.domain_min[i])).clamp(0., 1.)
            * (n - 1) as f32
    });
    let lo = xyz.map(|v| v.floor() as usize);
    let hi = lo.map(|v| (v + 1).min(n - 1));
    let f: [f32; 3] = std::array::from_fn(|i| xyz[i] - lo[i] as f32);
    let mut out = [0.; 3];
    for corner in 0..8 {
        let pick = |axis: usize| corner >> axis & 1 == 1;
        let at: [usize; 3] = std::array::from_fn(|i| if pick(i) { hi[i] } else { lo[i] });
        let weight: f32 = (0..3)
            .map(|i| if pick(i) { f[i] } else { 1. - f[i] })
            .product();
        let value = lut.values[at[0] + n * at[1] + n * n * at[2]];
        for c in 0..3 {
            out[c] += value[c] * weight;
        }
    }
    out
}

/// The table a look without one is given, so every preset binds the same textures.
fn identity_table() -> Lut {
    Lut {
        name: "identity".into(),
        size: 2,
        domain_min: [0.; 3],
        domain_max: [1.; 3],
        values: (0..8)
            .map(|i| [i & 1, i >> 1 & 1, i >> 2 & 1].map(|v| v as f32))
            .collect(),
    }
}

/// Writes `files` into `dir`, which must not exist yet, so an export never mixes with or
/// replaces files already there.
#[cfg(not(target_arch = "wasm32"))]
pub fn write(dir: &std::path::Path, files: &[File]) -> Result<()> {
    use anyhow::Context as _;
    std::fs::create_dir(dir)
        .with_context(|| format!("{} must be a new folder whose parent exists", dir.display()))?;
    for file in files {
        let path = dir.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.bytes)
            .with_context(|| format!("Cannot write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file<'a>(files: &'a [File], path: &str) -> &'a File {
        files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} exported"))
    }

    #[test]
    fn every_parameter_the_shaders_read_is_declared_once() {
        let names: Vec<_> = parameters().iter().map(|p| p.name).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "a parameter is declared twice");
        for texture in TEXTURES {
            assert!(
                !names.contains(&texture),
                "{texture} is a texture and a parameter"
            );
        }
        for (shader, source) in SOURCES {
            for word in source.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                let is_texture = TEXTURES.contains(&word)
                    || word.starts_with("CRTSIM_SIGNAL")
                    || word == "CRTSIM_GLASS";
                if word.starts_with("CRTSIM_") && !is_texture {
                    assert!(names.contains(&word), "{shader} reads {word}, not declared");
                }
            }
        }
    }

    #[test]
    fn a_preset_holds_its_look_and_the_pragmas_the_reference() {
        let mut look = Config::general();
        look.bloom = 0.5;
        look.persistence = [0.6, 0.5, 0.4];
        look.color_mode = ColorMode::LinearLight;
        let files = export(&[("general", &look)]).unwrap();
        let preset = String::from_utf8(file(&files, "general.slangp").bytes.clone()).unwrap();
        for expected in [
            "CRTSIM_BLOOM = 0.5\n",
            "CRTSIM_PERSISTENCE_G = 0.5\n",
            "CRTSIM_FIT = 1\n",
            "CRTSIM_MASK_COLUMNS = 128\n",
            "CRTSIM_LINEAR = 1\n",
            "float_framebuffer2 = true\n",
            "CRTSIM_COLOUR_TABLE = shaders/crtsim-renderer/no-table.png\n",
        ] {
            assert!(preset.contains(expected), "{expected:?} in\n{preset}");
        }
        let pragmas = String::from_utf8(
            file(&files, "shaders/crtsim-renderer/parameters.inc")
                .bytes
                .clone(),
        )
        .unwrap();
        assert!(pragmas.contains(
            "#pragma parameter CRTSIM_PERSISTENCE_R \"Red persistence\" 0.7 0 0.999 0.01\n"
        ));
        assert!(pragmas.contains("#pragma parameter CRTSIM_MASK_COLUMNS"));
        assert!(pragmas.contains("CRTSIM_MASK_ROWS \"Mask rows (0 follows the picture)\" 0 "));
    }

    #[test]
    fn a_colour_table_becomes_a_strip_of_its_lattice() {
        let lut = crate::nes_luts::load(0).unwrap();
        let image = strip(&lut);
        let n = lut.size as u32;
        assert_eq!(image.dimensions(), (n * n, n));
        // Red across a slice, green down it, blue by slice, as the lattice is indexed.
        let (r, g, b) = (3, 5, 7);
        let value = lut.values[r + lut.size * g + lut.size * lut.size * b];
        let pixel = image.get_pixel((r + b * lut.size) as u32, g as u32);
        for c in 0..3 {
            assert_eq!(pixel[c], (value[c].clamp(0., 1.) * 255.).round() as u8);
        }
        let identity = strip(&identity_table());
        assert_eq!(identity.get_pixel(3, 1).0, [255, 255, 255, 255]);
        assert_eq!(identity.get_pixel(1, 0).0, [255, 0, 0, 255]);
        let with_palette = Config {
            palette: Some(Default::default()),
            ..Config::default()
        };
        let files = export(&[("nes", &with_palette)]).unwrap();
        file(&files, "nes-table.png");
        let preset = String::from_utf8(file(&files, "nes.slangp").bytes.clone()).unwrap();
        assert!(preset.contains("CRTSIM_TABLE = 1\n"));
    }

    #[test]
    fn a_look_is_named_for_a_file() {
        let c = Config::default();
        assert!(export(&[("../escape", &c)]).is_err());
        assert!(export(&[]).is_err());
    }
}
