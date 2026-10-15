//! Looks as RetroArch shader presets: the shader port in `ports/retroarch`, with a `.slangp`
//! per look that sets its parameters and names its colour table.
//!
//! The port draws what the renderer draws, pass for pass, at the size the host gives it: the
//! core's own picture is the signal, and the screen is the viewport. What the port leaves to
//! RetroArch -- resizing, source edits, the export size and warm-up -- a preset does not hold.

use super::{File, Host, Kind, Parameter};
use crate::config::{ColorMode, Config};
use anyhow::Result;
use std::fmt::Write as _;

/// Where a preset finds the port's shaders and textures, relative to it.
const SHADERS: &str = "shaders/crtsim-renderer";

/// The port's own sources, as `ports/retroarch` holds them. The shaders include the passes
/// every port shares from `passes/`.
const SOURCES: &[(&str, &str)] = &[
    (
        "dialect.inc",
        include_str!("../../../../ports/retroarch/dialect.inc"),
    ),
    (
        "bloom.inc",
        include_str!("../../../../ports/retroarch/bloom.inc"),
    ),
    (
        "prepare.slang",
        include_str!("../../../../ports/retroarch/prepare.slang"),
    ),
    (
        "composite.slang",
        include_str!("../../../../ports/retroarch/composite.slang"),
    ),
    (
        "surface.slang",
        include_str!("../../../../ports/retroarch/surface.slang"),
    ),
    (
        "bloom-down.slang",
        include_str!("../../../../ports/retroarch/bloom-down.slang"),
    ),
    (
        "bloom-up.slang",
        include_str!("../../../../ports/retroarch/bloom-up.slang"),
    ),
    (
        "present.slang",
        include_str!("../../../../ports/retroarch/present.slang"),
    ),
];

const README: &str = include_str!("../../../../ports/retroarch/README.md");

/// The files that make `looks` RetroArch presets, each look named by its preset's file name
/// without `.slangp`. They share one copy of the shaders.
pub fn export(looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    super::check_looks(looks)?;
    let mut files = vec![
        super::text("README.md", README),
        super::text(&format!("{SHADERS}/parameters.inc"), &pragmas()),
    ];
    files.extend(super::image_files(SHADERS)?);
    files.extend(
        SOURCES
            .iter()
            .map(|(name, source)| super::text(&format!("{SHADERS}/{name}"), source)),
    );
    files.extend(super::passes(&format!("{SHADERS}/passes")));
    for (name, config) in looks {
        let table = match super::table_file(config, &format!("{name}-table.png"))? {
            Some((file, _)) => {
                let path = file.path.clone();
                files.push(file);
                path
            }
            None => format!("{SHADERS}/no-table.png"),
        };
        files.push(super::text(
            &format!("{name}.slangp"),
            &preset(config, &table),
        ));
    }
    Ok(files)
}

/// What RetroArch's menu shows: a choice lists its items, since a parameter is a number.
fn label(p: &Parameter) -> String {
    match p.kind {
        Kind::Number { .. } => p.label.into(),
        Kind::Choice(items) => format!("{} ({})", p.label, items.join(", ")),
    }
}

/// `parameters.inc`: every parameter, with the reference look's value as its default.
fn pragmas() -> String {
    let reference = Config::default();
    let mut out = String::from(
        "// CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not this.\n",
    );
    for p in super::parameters(Host::RetroArch) {
        let (min, max, step) = p.range();
        writeln!(
            out,
            "#pragma parameter {} \"{}\" {} {min} {max} {step}",
            p.name,
            label(&p),
            p.of(&reference),
        )
        .expect("writing to a string");
    }
    out
}

/// The textures a preset loads. None may share a parameter's name: RetroArch and librashader
/// read a preset's `NAME = value` lines by name alone, so a clash turns one into the other.
const TEXTURES: [&str; 7] = [
    "CRTSIM_COLOUR_TABLE",
    "CRTSIM_ARTIFACT_PATTERN",
    "CRTSIM_SHADOW_MASK",
    "CRTSIM_BEZEL_SHAPE",
    "CRTSIM_BEZEL_UV",
    "CRTSIM_BEZEL_NORMAL",
    "CRTSIM_GLASS",
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
        shader: "surface.slang",
        alias: Some("CRTSIM_SURFACE"),
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
    // The bezel's and the glass's images are read texel by texel, and the bezel's shape
    // filtered as well, for the depth's high byte a ray steps on.
    for (name, file, filtered) in [
        ("CRTSIM_BEZEL_SHAPE", "bezel-shape.png", true),
        ("CRTSIM_BEZEL_UV", "bezel-uv.png", false),
        ("CRTSIM_BEZEL_NORMAL", "bezel-normal.png", false),
        ("CRTSIM_GLASS", "glass.png", false),
    ] {
        line(format!("{name} = {SHADERS}/{file}"));
        line(format!("{name}_linear = {filtered}"));
        line(format!("{name}_wrap_mode = clamp_to_edge"));
    }
    let parameters = super::parameters(Host::RetroArch);
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
    fn every_parameter_the_shaders_read_is_declared() {
        let names: Vec<_> = super::super::parameters(Host::RetroArch)
            .iter()
            .map(|p| p.name)
            .collect();
        for texture in TEXTURES {
            assert!(
                !names.contains(&texture),
                "{texture} is a texture and a parameter"
            );
        }
        let aliases = [
            "CRTSIM_SIGNAL",
            "CRTSIM_SIGNALFeedback",
            "CRTSIM_SIGNALFeedbackSize",
            "CRTSIM_SURFACE",
        ];
        let not_parameters: Vec<_> = TEXTURES.iter().chain(&aliases).copied().collect();
        for word in super::super::tests::parameters_read(SOURCES, &not_parameters) {
            assert!(
                names.contains(&word.as_str()),
                "the shaders read {word}, not declared"
            );
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
        assert!(
            !preset.contains("CRTSIM_SIGNAL_HEIGHT"),
            "RetroArch does not resize"
        );
        let pragmas = String::from_utf8(
            file(&files, "shaders/crtsim-renderer/parameters.inc")
                .bytes
                .clone(),
        )
        .unwrap();
        assert!(pragmas.contains(
            "#pragma parameter CRTSIM_PERSISTENCE_R \"Red persistence\" 0.7 0 0.999 0.01\n"
        ));
        assert!(pragmas.contains(
            "#pragma parameter CRTSIM_FIT \"Fit on 4:3 tube (original, contain, cover, stretch)\" 0 0 3 1\n"
        ));
        assert!(pragmas.contains("CRTSIM_MASK_ROWS \"Mask rows (0 follows the signal)\" 0 "));
        file(&files, "shaders/crtsim-renderer/passes/surface.inc");
    }

    #[test]
    fn a_look_with_a_colour_table_carries_it() {
        let with_palette = Config {
            palette: Some(Default::default()),
            ..Config::default()
        };
        let files = export(&[("nes", &with_palette)]).unwrap();
        file(&files, "nes-table.png");
        let preset = String::from_utf8(file(&files, "nes.slangp").bytes.clone()).unwrap();
        assert!(preset.contains("CRTSIM_TABLE = 1\n"));
        assert!(preset.contains("CRTSIM_COLOUR_TABLE = nes-table.png\n"));
    }
}
