//! Looks as shader ports: the effect rewritten in a host's own shader language, which the host
//! runs on every frame of its picture. See
//! [the decision](../../../../docs/adr/0003-games-get-shader-ports-not-the-renderer.md).
//!
//! Every port draws what the renderer draws, pass for pass, from the passes in `ports/passes`,
//! which each host's shaders include. This module holds what the ports' exports share: the
//! parameters a look becomes, the images the passes read, the looks a release ships, and
//! writing an export's files. Each host's module writes its own files around them.

pub mod obs;
pub mod reshade;
pub mod retroarch;

use crate::config::{ColorMode, Config, Filter, Fit, MaskRepeats, Phase, Signal};
use crate::settings::{Control, SETTINGS};
use crate::workflow::{Lut, Sampling};
use anyhow::{ensure, Result};
use image::{ImageFormat, Rgba, RgbaImage};
use std::ops::Bound;

/// A file of an export, at its path relative to the folder it is written into.
pub struct File {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The passes every port includes, as `ports/passes` holds them.
const PASSES: &[(&str, &str)] = &[
    (
        "common.inc",
        include_str!("../../../../ports/passes/common.inc"),
    ),
    (
        "resize.inc",
        include_str!("../../../../ports/passes/resize.inc"),
    ),
    (
        "prepare.inc",
        include_str!("../../../../ports/passes/prepare.inc"),
    ),
    (
        "composite.inc",
        include_str!("../../../../ports/passes/composite.inc"),
    ),
    (
        "surface.inc",
        include_str!("../../../../ports/passes/surface.inc"),
    ),
    (
        "bloom.inc",
        include_str!("../../../../ports/passes/bloom.inc"),
    ),
    (
        "present.inc",
        include_str!("../../../../ports/passes/present.inc"),
    ),
];

/// The passes, written into `dir`.
fn passes(dir: &str) -> impl Iterator<Item = File> + '_ {
    PASSES
        .iter()
        .map(move |(name, source)| text(&format!("{dir}/{name}"), source))
}

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

/// Checks the looks an export is asked for: at least one, each valid and named for a file.
fn check_looks(looks: &[(&str, &Config)]) -> Result<()> {
    ensure!(!looks.is_empty(), "Choose at least one look to export");
    for (name, config) in looks {
        ensure!(
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
            "A look's name must be letters, digits, - and _: {name:?}"
        );
        config.validate()?;
        // The renderer reads the signal's name only as it sizes a picture; a port reads it now.
        config.signal()?;
    }
    Ok(())
}

fn text(path: &str, text: &str) -> File {
    File {
        path: path.into(),
        bytes: text.as_bytes().to_vec(),
    }
}

fn png(path: &str, image: &RgbaImage) -> Result<File> {
    Ok(File {
        path: path.into(),
        bytes: encode_png(image)?,
    })
}

fn encode_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

/// How the passes read one of the images.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// Texel by texel.
    Texels,
    /// Texel by texel, and filtered too: the bezel's shape, for its depth's high byte.
    AlsoFiltered,
    /// Filtered, with mipmaps that each average 2×2 texels of the level above.
    Mipmapped,
}

/// An image the passes read, as a file every port's export holds.
pub struct Image {
    pub file: &'static str,
    pub reading: Reading,
    picture: fn() -> Result<RgbaImage>,
}

/// The images the passes read: the artifact pattern, the shadow mask, the table a look without
/// one is given, the bezel's three and the glass's.
pub const IMAGES: [Image; 7] = [
    Image {
        file: "artifacts.png",
        reading: Reading::Texels,
        picture: crate::gpu::artifacts_image,
    },
    Image {
        file: "mask.png",
        reading: Reading::Mipmapped,
        picture: crate::gpu::mask_image,
    },
    Image {
        file: NO_TABLE,
        reading: Reading::Texels,
        picture: || Ok(strip(&identity_table())),
    },
    Image {
        file: "bezel-shape.png",
        reading: Reading::AlsoFiltered,
        picture: || Ok(crate::bezel::maps()?.shape),
    },
    Image {
        file: "bezel-uv.png",
        reading: Reading::Texels,
        picture: || Ok(crate::bezel::maps()?.uv),
    },
    Image {
        file: "bezel-normal.png",
        reading: Reading::Texels,
        picture: || Ok(crate::bezel::maps()?.normal),
    },
    Image {
        file: "glass.png",
        reading: Reading::Texels,
        picture: crate::glass::map,
    },
];

/// The table a look without one is given, so every look binds the same textures.
const NO_TABLE: &str = "no-table.png";

/// The images, written into `dir`.
fn image_files(dir: &str) -> Result<Vec<File>> {
    IMAGES
        .iter()
        .map(|image| png(&join(dir, image.file), &(image.picture)()?))
        .collect()
}

/// `name` in the folder `dir` of an export, or at its top when `dir` is empty.
fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.into()
    } else {
        format!("{dir}/{name}")
    }
}

/// A look's colour table as the ports carry it: a strip of its lattice's slices, in a file
/// named for the look.
struct Table {
    name: String,
    file: Vec<u8>,
    /// Lattice points along each axis, which is how many slices there are.
    size: usize,
}

impl Table {
    /// The table of the look `look`, which `config` holds, when it holds one.
    fn of(config: &Config, look: &str) -> Result<Option<Self>> {
        config
            .lut_in_use()
            .map(|lut| {
                Ok(Self {
                    name: format!("{look}-table.png"),
                    file: encode_png(&strip(&lut))?,
                    size: lut.size,
                })
            })
            .transpose()
    }

    /// The table, written into `dir`.
    fn at(&self, dir: &str) -> File {
        File {
            path: join(dir, &self.name),
            bytes: self.file.clone(),
        }
    }
}

/// A table's lattice as an image of `size` slices side by side: red across each slice, green
/// down it, blue from slice to slice. Sampled over a domain other than 0 to 1, the table is
/// first resampled onto that one, which is the only domain the ports read.
fn strip(lut: &Lut) -> RgbaImage {
    let n = lut.size;
    let last = (n - 1) as f32;
    RgbaImage::from_fn((n * n) as u32, n as u32, |x, y| {
        let (r, b, g) = (x as usize % n, x as usize / n, y as usize);
        let colour = [r, g, b].map(|v| v as f32 / last);
        let [r, g, b] = lut
            .sample(colour)
            .map(|v| (v.clamp(0., 1.) * 255.).round() as u8);
        Rgba([r, g, b, 255])
    })
}

fn identity_table() -> Lut {
    Lut {
        name: "identity".into(),
        size: 2,
        domain_min: [0.; 3],
        domain_max: [1.; 3],
        values: (0..8)
            .map(|i| [i & 1, i >> 1 & 1, i >> 2 & 1].map(|v| v as f32))
            .collect(),
        sampling: Default::default(),
    }
}

/// The size of the table a look without one is given.
const NO_TABLE_SIZE: usize = 2;

/// The programs a port runs in, each reading the parameters its shaders declare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    RetroArch,
    ReShade,
    Obs,
}

impl Host {
    /// What its port is called: what a look is exported as for it.
    pub fn port_name(self) -> &'static str {
        match self {
            Self::RetroArch => "RetroArch shader",
            Self::ReShade => "ReShade effect",
            Self::Obs => "OBS filter",
        }
    }

    /// Its name in a file's or a folder's.
    pub fn slug(self) -> &'static str {
        match self {
            Self::RetroArch => "retroarch",
            Self::ReShade => "reshade",
            Self::Obs => "obs",
        }
    }

    /// The extension of a look's preset.
    pub fn preset_extension(self) -> &'static str {
        match self {
            Self::RetroArch => "slangp",
            Self::ReShade => "ini",
            Self::Obs => "json",
        }
    }
}

/// The files that make `looks` presets of `host`'s port, each look named by its preset's file
/// name without its extension. They share one copy of the port.
pub fn export(host: Host, looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    match host {
        Host::RetroArch => retroarch::export(looks),
        Host::ReShade => reshade::export(looks),
        Host::Obs => obs::export(looks),
    }
}

const ALL: &[Host] = &[Host::RetroArch, Host::ReShade, Host::Obs];
/// The hosts that hand the port a picture of any size, which it resizes to the signal itself;
/// RetroArch hands over the core's own.
const RESIZING: &[Host] = &[Host::ReShade, Host::Obs];

/// One of a port's parameters: what its host's menu shows, and where its value comes from.
pub struct Parameter {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: Kind,
    value: Value,
    hosts: &'static [Host],
}

pub enum Kind {
    /// A number in a range, moved in steps.
    Number { min: f32, max: f32, step: f32 },
    /// One of these, numbered from 0.
    Choice(&'static [&'static str]),
}

enum Value {
    /// The `index`th number of a setting in `SETTINGS`.
    Setting { key: &'static str, index: usize },
    /// Worked out from settings the table does not hold as numbers.
    Derived(fn(&Config) -> f32),
}

/// The settings the ports read as numbers, by key, with a parameter name for each value, and
/// the hosts that read them.
const NUMBERS: &[(&str, &[&str], &[Host])] = &[
    ("pixel_aspect", &["CRTSIM_PIXEL_ASPECT"], RESIZING),
    ("lut_strength", &["CRTSIM_LUT_STRENGTH"], ALL),
    ("hue", &["CRTSIM_HUE"], ALL),
    ("chroma", &["CRTSIM_CHROMA"], ALL),
    ("saturation", &["CRTSIM_SATURATION"], ALL),
    ("sharpness", &["CRTSIM_SHARPNESS"], ALL),
    ("bleed", &["CRTSIM_BLEED"], ALL),
    ("artifacts", &["CRTSIM_ARTIFACTS"], ALL),
    ("ntsc_blending", &["CRTSIM_NTSC_BLENDING"], ALL),
    ("barrel", &["CRTSIM_BARREL"], ALL),
    ("overscan", &["CRTSIM_OVERSCAN"], ALL),
    ("dimming", &["CRTSIM_DIMMING"], ALL),
    ("fov", &["CRTSIM_FOV"], ALL),
    ("mask_opacity", &["CRTSIM_MASK_OPACITY"], ALL),
    ("mask_brightness", &["CRTSIM_MASK_BRIGHTNESS"], ALL),
    ("bloom", &["CRTSIM_BLOOM"], ALL),
    ("bloom_power", &["CRTSIM_BLOOM_POWER"], ALL),
    ("bloom_spread", &["CRTSIM_BLOOM_SPREAD"], ALL),
    ("diffuse", &["CRTSIM_DIFFUSE"], ALL),
    ("specular", &["CRTSIM_SPECULAR"], ALL),
    ("specular_power", &["CRTSIM_SPECULAR_POWER"], ALL),
    ("rim", &["CRTSIM_RIM"], ALL),
    ("reflection", &["CRTSIM_REFLECTION"], ALL),
    (
        "frame_color",
        &["CRTSIM_BEZEL_R", "CRTSIM_BEZEL_G", "CRTSIM_BEZEL_B"],
        ALL,
    ),
    (
        "backdrop_color",
        &[
            "CRTSIM_BACKDROP_R",
            "CRTSIM_BACKDROP_G",
            "CRTSIM_BACKDROP_B",
        ],
        ALL,
    ),
    (
        "light_position",
        &["CRTSIM_LIGHT_X", "CRTSIM_LIGHT_Y", "CRTSIM_LIGHT_Z"],
        ALL,
    ),
    (
        "persistence",
        &[
            "CRTSIM_PERSISTENCE_R",
            "CRTSIM_PERSISTENCE_G",
            "CRTSIM_PERSISTENCE_B",
        ],
        ALL,
    ),
];

fn flag(on: bool) -> f32 {
    if on {
        1.
    } else {
        0.
    }
}

const OFF_ON: &[&str] = &["off", "on"];

/// The signal's columns and rows as the resizing hosts read them: either 0 to follow the
/// picture, the columns then keeping its shape. A look made for any picture's rows (`auto`)
/// takes 480, the most it would.
fn signal(c: &Config) -> [f32; 2] {
    match c
        .signal()
        .expect("a look's signal is checked as it is exported")
    {
        Signal::Native => [0., 0.],
        Signal::Original => [256., 224.],
        Signal::Auto => [0., 480.],
        Signal::Rows(rows) => [0., rows as f32],
        Signal::Size(width, height) => [width as f32, height as f32],
    }
}

/// Every parameter `host` reads, in the order its menu lists them.
pub fn parameters(host: Host) -> Vec<Parameter> {
    let choice = |name, label, items, hosts, value| Parameter {
        name,
        label,
        kind: Kind::Choice(items),
        value: Value::Derived(value),
        hosts,
    };
    let number = |name, label, min, max, step, hosts, value| Parameter {
        name,
        label,
        kind: Kind::Number { min, max, step },
        value: Value::Derived(value),
        hosts,
    };
    let mut all = vec![
        number(
            "CRTSIM_SIGNAL_WIDTH",
            "Signal columns (0 keeps the picture's shape)",
            0.,
            4096.,
            1.,
            RESIZING,
            |c| signal(c)[0],
        ),
        number(
            "CRTSIM_SIGNAL_HEIGHT",
            "Signal rows (0 for the picture's own)",
            0.,
            4096.,
            1.,
            RESIZING,
            |c| signal(c)[1],
        ),
        choice(
            "CRTSIM_RESIZE",
            "Resize to the signal",
            &["nearest", "Lanczos"],
            RESIZING,
            |c| flag(c.filter == Filter::Lanczos),
        ),
        choice("CRTSIM_TABLE", "Colour table", OFF_ON, ALL, |c| {
            flag(c.lut_in_use().is_some())
        }),
        // Super Win the Game's palette is read as the game reads it; other tables blend.
        choice(
            "CRTSIM_TABLE_NEAREST",
            "Colour table red and green",
            &["blended", "nearest"],
            ALL,
            |c| {
                flag(
                    c.lut_in_use()
                        .is_some_and(|lut| lut.sampling == Sampling::NearestRedGreen),
                )
            },
        ),
        choice("CRTSIM_LINEAR", "Linear light", OFF_ON, ALL, |c| {
            flag(c.color_mode == ColorMode::LinearLight)
        }),
        choice(
            "CRTSIM_PHASE",
            "Phase",
            &["stable", "A", "B", "alternating"],
            ALL,
            |c| match c.phase {
                Phase::Stable => 0.,
                Phase::A => 1.,
                Phase::B => 2.,
                Phase::Alternating => 3.,
            },
        ),
        choice(
            "CRTSIM_FLIP_ARTIFACTS",
            "Artifact pattern",
            &["as the public source", "flipped"],
            ALL,
            |c| flag(c.flip_artifacts),
        ),
        // A look made on a still says nothing about a picture that switches to 480 rows, so a
        // look that does not interlace leaves it to the picture's size.
        choice(
            "CRTSIM_INTERLACE",
            "Interlaced fields",
            &["off", "on", "auto"],
            ALL,
            |c| if c.interlace { 1. } else { 2. },
        ),
        choice(
            "CRTSIM_FIT",
            "Fit on 4:3 tube",
            &["original", "contain", "cover", "stretch"],
            ALL,
            |c| match c.fit {
                Fit::Reference => 0.,
                Fit::Contain => 1.,
                Fit::Cover => 2.,
                Fit::Stretch => 3.,
            },
        ),
        // RetroArch says the picture's shape itself, from 1.20 on.
        number(
            "CRTSIM_ASPECT",
            "Picture aspect, when the core gives none",
            0.25,
            4.,
            0.01,
            &[Host::RetroArch],
            |_| 4. / 3.,
        ),
        choice(
            "CRTSIM_SCREEN_ONLY",
            "Screen only, without the bezel",
            OFF_ON,
            ALL,
            |c| flag(c.screen_only),
        ),
        choice(
            "CRTSIM_REFLECTION_AS_SCREEN",
            "Bezel reflection",
            &["as the public source", "as the screen"],
            ALL,
            |c| flag(c.reflection_as_screen),
        ),
        choice(
            "CRTSIM_MASK_ANTIALIAS",
            "Filter mask when shrinking",
            OFF_ON,
            ALL,
            |c| flag(c.mask_antialias),
        ),
        // Zero follows the signal: a column for every two of its columns, a row for each row.
        number(
            "CRTSIM_MASK_COLUMNS",
            "Mask columns (0 follows the signal)",
            0.,
            4096.,
            1.,
            ALL,
            |c| c.mask_repeats.fixed().map_or(0., |[x, _]| x),
        ),
        number(
            "CRTSIM_MASK_ROWS",
            "Mask rows (0 follows the signal)",
            0.,
            4096.,
            1.,
            ALL,
            |c| c.mask_repeats.fixed().map_or(0., |[_, y]| y),
        ),
    ];
    for (key, names, hosts) in NUMBERS {
        let setting = SETTINGS
            .iter()
            .find(|s| s.key == *key)
            .expect("a setting the ports read is in the table");
        let numbers = setting.numbers.as_ref().expect("a numeric setting");
        let (min, mut max) = match &numbers.control {
            Control::Slider { span, .. } => (*span.start(), *span.end()),
            Control::Color => (0., 1.),
        };
        // A look may hold more than its slider shows; the host's range has to reach it.
        if let Bound::Included(valid) = numbers.valid.1 {
            max = max.max(valid);
        }
        for (index, name) in names.iter().enumerate() {
            all.push(Parameter {
                name,
                label: setting.value_name(index),
                kind: Kind::Number {
                    min,
                    max,
                    step: step(min, max),
                },
                value: Value::Setting { key, index },
                hosts,
            });
        }
    }
    all.retain(|p| p.hosts.contains(&host));
    all
}

/// A hundred steps across the range, rounded to one significant figure. Worked in f64, so
/// the f32 prints as that figure and a host's menu shows it as written.
fn step(min: f32, max: f32) -> f32 {
    let raw = f64::from(max - min) / 100.;
    let scale = 10_f64.powf(raw.log10().floor());
    ((raw / scale).round() * scale) as f32
}

impl Parameter {
    /// Its value in a look.
    pub fn of(&self, c: &Config) -> f32 {
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

    /// Its range: a choice's runs from 0 to its last item, in steps of 1.
    pub fn range(&self) -> (f32, f32, f32) {
        match self.kind {
            Kind::Number { min, max, step } => (min, max, step),
            Kind::Choice(items) => (0., (items.len() - 1) as f32, 1.),
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

    /// The parameters' words a host's shaders read: every `CRTSIM_` word that is not one of
    /// `textures`.
    pub(super) fn parameters_read(sources: &[(&str, &str)], textures: &[&str]) -> Vec<String> {
        let mut words: Vec<String> = sources
            .iter()
            .flat_map(|(_, source)| {
                source.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            })
            .filter(|word| word.starts_with("CRTSIM_") && !textures.contains(word))
            .map(String::from)
            .collect();
        words.sort();
        words.dedup();
        words
    }

    /// `text` with every include it names among `sources` written in its place, and its
    /// comments taken out. Includes the exporter writes, which no source holds, are left out.
    fn expand(text: &str, sources: &[(String, &str)]) -> String {
        let mut out = String::new();
        for line in text.lines() {
            let code = line.split("//").next().unwrap_or_default();
            let included = code
                .trim()
                .strip_prefix("#include \"")
                .and_then(|rest| rest.strip_suffix('"'))
                .map(|path| path.trim_start_matches("crtsim-renderer/"));
            match included {
                Some(path) => {
                    if let Some((_, source)) = sources.iter().find(|(name, _)| name == path) {
                        out.push_str(&expand(source, sources));
                    }
                }
                None => {
                    out.push_str(code);
                    out.push('\n');
                }
            }
        }
        out
    }

    #[test]
    fn every_shell_defines_the_host_macros_its_passes_read() {
        let sources: Vec<(String, &str)> = PASSES
            .iter()
            .map(|(name, source)| (format!("passes/{name}"), *source))
            .chain(
                retroarch::SOURCES
                    .iter()
                    .map(|(name, source)| (name.to_string(), *source)),
            )
            .collect();
        let shells = retroarch::SOURCES
            .iter()
            .filter(|(name, _)| name.ends_with(".slang"))
            .copied()
            .chain([
                ("CRTSimRenderer.fx", reshade::SOURCE),
                ("crtsim.effect", obs::EFFECT),
            ]);
        for (shell, source) in shells {
            let code = expand(source, &sources);
            let defined: Vec<&str> = code
                .lines()
                .filter_map(|line| line.trim().strip_prefix("#define "))
                .map(|rest| rest.split(['(', ' ']).next().unwrap_or_default())
                .collect();
            let words = code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
            for word in words.filter(|word| word.starts_with("HOST_")) {
                assert!(
                    defined.contains(&word),
                    "{shell} reads {word}, which it never defines"
                );
            }
        }
    }

    #[test]
    fn every_host_declares_each_parameter_once() {
        for host in ALL {
            let names: Vec<_> = parameters(*host).iter().map(|p| p.name).collect();
            let mut unique = names.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), names.len(), "{host:?} declares one twice");
        }
    }

    #[test]
    fn the_passes_read_only_parameters_every_host_declares() {
        for word in parameters_read(PASSES, &[]) {
            for host in ALL {
                assert!(
                    parameters(*host).iter().any(|p| p.name == word),
                    "the passes read {word}, which {host:?} does not declare"
                );
            }
        }
    }

    #[test]
    fn a_choice_holds_one_of_its_items() {
        let looks = shipped();
        for host in ALL {
            for p in parameters(*host) {
                let (min, max, _) = p.range();
                for (name, look) in &looks {
                    let value = p.of(look);
                    assert!(
                        (min..=max).contains(&value),
                        "{} is {value} in {name}, outside {min}..={max}",
                        p.name
                    );
                }
            }
        }
    }

    #[test]
    fn a_signal_size_becomes_columns_and_rows() {
        let with = |size: &str| {
            signal(&Config {
                signal: size.into(),
                ..Config::default()
            })
        };
        assert_eq!(with("native"), [0., 0.]);
        assert_eq!(with("original"), [256., 224.]);
        assert_eq!(with("auto"), [0., 480.]);
        assert_eq!(with("240p"), [0., 240.]);
        assert_eq!(with("320x200"), [320., 200.]);
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
        assert_eq!(identity.dimensions(), (4, NO_TABLE_SIZE as u32));
        assert_eq!(identity.get_pixel(3, 1).0, [255, 255, 255, 255]);
        assert_eq!(identity.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn a_look_is_named_for_a_file() {
        let c = Config::default();
        assert!(check_looks(&[("../escape", &c)]).is_err());
        assert!(check_looks(&[]).is_err());
        assert!(check_looks(&[("my-look_2", &c)]).is_ok());
        let unknown = Config {
            signal: "1080i".into(),
            ..Config::default()
        };
        assert!(check_looks(&[("look", &unknown)]).is_err());
    }
}
