//! Looks as ReShade presets: the shader port in `ports/reshade`, with a preset per look that
//! turns the effect on and sets its uniforms, its preprocessor definitions and its colour
//! table.
//!
//! ReShade hands the effect the game's whole picture, which it resizes to the signal itself.
//! What a texture's size or format depends on -- the signal's size, linear light and the
//! colour table's image -- is a preprocessor definition, which a preset sets for every effect;
//! every other setting is a uniform. ReShade compiles every `.fx` in its shader folders, so
//! the effect's own files sit in a folder of their own beside it.

use super::{File, Host, Kind, Parameter, Reading, Table};
use anyhow::Result;
use crtsim_core::config::Config;

/// Where ReShade looks for effects and their textures, relative to the export.
const SHADERS: &str = "reshade-shaders/Shaders";
const TEXTURES: &str = "reshade-shaders/Textures";
/// The folder of the effect's own files, in each of those.
const OWN: &str = "crtsim-renderer";
const EFFECT: &str = "CRTSimRenderer.fx";
const TECHNIQUE: &str = "CRTSimRenderer";

pub(super) const SOURCE: &str = include_str!("../../../ports/reshade/CRTSimRenderer.fx");
const README: &str = include_str!("../../../ports/reshade/README.md");

/// The parameters ReShade reads as preprocessor definitions, since textures depend on them.
const DEFINITIONS: [&str; 3] = [
    "CRTSIM_LINEAR",
    "CRTSIM_SIGNAL_WIDTH",
    "CRTSIM_SIGNAL_HEIGHT",
];

/// The files that make `looks` ReShade presets, each look named by its preset's file name
/// without `.ini`. They share one copy of the effect.
pub fn export(looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    super::check_looks(looks)?;
    let mut files = vec![
        super::text("README.md", README),
        super::text(&format!("{SHADERS}/{EFFECT}"), SOURCE),
        super::text(&format!("{SHADERS}/{OWN}/parameters.fxh"), &header()?),
    ];
    files.extend(super::passes(&format!("{SHADERS}/{OWN}/passes")));
    files.extend(super::image_files(&format!("{TEXTURES}/{OWN}"))?);
    for (name, config) in looks {
        let table = Table::of(config, name)?;
        if let Some(table) = &table {
            files.push(table.at(&format!("{TEXTURES}/{OWN}")));
        }
        files.push(super::text(
            &format!("{name}.ini"),
            &preset(config, table.as_ref()),
        ));
    }
    Ok(files)
}

/// A number as HLSL writes a float, with a point.
fn float(value: f32) -> String {
    format!("{value:?}")
}

/// `parameters.fxh`: the images' sizes, the definitions' defaults and every uniform, each
/// with the reference look's value.
fn header() -> Result<String> {
    let reference = Config::default();
    let mut out = String::from(
        "// CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not this.\n\n\
         // The sizes of the images in Textures/crtsim-renderer.\n",
    );
    let mut line = |text: String| out.push_str(&(text + "\n"));
    for Declared {
        prefix,
        size: (width, height),
        mipmapped,
    } in declared_images()?
    {
        line(format!("#define {prefix}_WIDTH {width}"));
        line(format!("#define {prefix}_HEIGHT {height}"));
        if mipmapped {
            let levels = 32 - width.max(height).leading_zeros();
            line(format!("#define {prefix}_LEVELS {levels}"));
        }
    }
    line(String::new());
    line("// A look's preset sets these, for the textures that depend on them.".into());
    let parameters = super::parameters(Host::ReShade);
    let definitions = parameters.iter().filter(|p| DEFINITIONS.contains(&p.name));
    let table = [
        (
            "CRTSIM_TABLE_FILE",
            format!("\"{OWN}/{}\"", super::NO_TABLE),
        ),
        ("CRTSIM_TABLE_SIZE", super::NO_TABLE_SIZE.to_string()),
    ];
    let defaults = definitions
        .map(|p| (p.name, (p.of(&reference) as i32).to_string()))
        .chain(table.iter().map(|(name, value)| (*name, value.clone())));
    for (name, value) in defaults {
        line(format!("#ifndef {name}\n#define {name} {value}\n#endif"));
    }
    for p in parameters.iter().filter(|p| !DEFINITIONS.contains(&p.name)) {
        line(String::new());
        line(uniform(p, p.of(&reference)));
    }
    Ok(out)
}

/// An image whose size the effect declares, as definitions named from `prefix`.
struct Declared {
    prefix: &'static str,
    size: (u32, u32),
    /// Whether the effect reads its mipmaps, so declares how many levels it has.
    mipmapped: bool,
}

/// The images whose sizes the effect declares; the bezel's three share one size.
fn declared_images() -> Result<Vec<Declared>> {
    let declared = |prefix, file| -> Result<Declared> {
        let image = super::IMAGES
            .iter()
            .find(|image| image.file == file)
            .expect("an image the ports read");
        Ok(Declared {
            prefix,
            size: (image.picture)()?.dimensions(),
            mipmapped: image.reading == Reading::Mipmapped,
        })
    };
    [
        ("CRTSIM_ARTIFACTS", "artifacts.png"),
        ("CRTSIM_MASK", "mask.png"),
        ("CRTSIM_BEZEL", "bezel-shape.png"),
        ("CRTSIM_GLASS", "glass.png"),
    ]
    .into_iter()
    .map(|(prefix, file)| declared(prefix, file))
    .collect()
}

/// A parameter as a uniform: a choice is an int its menu names the values of.
fn uniform(p: &Parameter, value: f32) -> String {
    match p.kind {
        Kind::Choice(items) => format!(
            "uniform int {} <\n    ui_type = \"combo\";\n    ui_label = \"{}\";\n    \
             ui_items = \"{}\\0\";\n> = {};",
            p.name,
            p.label,
            items.join("\\0"),
            value as i32
        ),
        Kind::Number { min, max, step } => format!(
            "uniform float {} <\n    ui_type = \"slider\";\n    ui_label = \"{}\";\n    \
             ui_min = {}; ui_max = {}; ui_step = {};\n> = {};",
            p.name,
            p.label,
            float(min),
            float(max),
            float(step),
            float(value)
        ),
    }
}

/// A look's preset, reading its colour table, when it has one, from the effect's textures.
fn preset(c: &Config, table: Option<&Table>) -> String {
    let parameters = super::parameters(Host::ReShade);
    let mut definitions: Vec<String> = parameters
        .iter()
        .filter(|p| DEFINITIONS.contains(&p.name))
        .map(|p| format!("{}={}", p.name, p.of(c) as i32))
        .collect();
    if let Some(table) = table {
        definitions.push(format!("CRTSIM_TABLE_FILE=\"{OWN}/{}\"", table.name));
        definitions.push(format!("CRTSIM_TABLE_SIZE={}", table.size));
    }
    let mut out = String::new();
    let mut line = |text: String| out.push_str(&(text + "\n"));
    line(format!("PreprocessorDefinitions={}", definitions.join(",")));
    line(format!("Techniques={TECHNIQUE}@{EFFECT}"));
    line(format!("TechniqueSorting={TECHNIQUE}@{EFFECT}"));
    line(String::new());
    line(format!("[{EFFECT}]"));
    for p in parameters.iter().filter(|p| !DEFINITIONS.contains(&p.name)) {
        let value = match p.kind {
            Kind::Choice(_) => (p.of(c) as i32).to_string(),
            Kind::Number { .. } => p.of(c).to_string(),
        };
        line(format!("{}={value}", p.name));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crtsim_core::config::ColorMode;

    fn text<'a>(files: &'a [File], path: &str) -> &'a str {
        let file = files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} exported"));
        std::str::from_utf8(&file.bytes).unwrap()
    }

    #[test]
    fn every_parameter_the_effect_reads_is_declared() {
        let names: Vec<_> = super::super::parameters(Host::ReShade)
            .iter()
            .map(|p| p.name)
            .collect();
        let mut not_parameters = vec!["CRTSIM_TABLE_FILE", "CRTSIM_TABLE_SIZE"];
        let defines: Vec<String> = declared_images()
            .unwrap()
            .iter()
            .flat_map(|image| {
                ["WIDTH", "HEIGHT", "LEVELS"].map(|s| format!("{}_{s}", image.prefix))
            })
            .collect();
        not_parameters.extend(defines.iter().map(String::as_str));
        let read = super::super::tests::parameters_read(&[(EFFECT, SOURCE)], &not_parameters);
        for word in read {
            assert!(
                names.contains(&word.as_str()),
                "the effect reads {word}, not declared"
            );
        }
    }

    #[test]
    fn a_preset_holds_its_look_and_the_header_the_reference() {
        let mut look = Config::general();
        look.bloom = 0.5;
        look.fit = crtsim_core::config::Fit::Cover;
        look.color_mode = ColorMode::LinearLight;
        let files = export(&[("general", &look)]).unwrap();
        let preset = text(&files, "general.ini");
        for expected in [
            "PreprocessorDefinitions=CRTSIM_SIGNAL_WIDTH=0,CRTSIM_SIGNAL_HEIGHT=480,CRTSIM_LINEAR=1\n",
            "Techniques=CRTSimRenderer@CRTSimRenderer.fx\n",
            "[CRTSimRenderer.fx]\n",
            "CRTSIM_BLOOM=0.5\n",
            "CRTSIM_FIT=2\n",
            "CRTSIM_RESIZE=1\n",
            "CRTSIM_PIXEL_ASPECT=1\n",
        ] {
            assert!(preset.contains(expected), "{expected:?} in\n{preset}");
        }
        assert!(
            !preset.contains("CRTSIM_ASPECT="),
            "ReShade has the pixel aspect instead"
        );
        let header = text(
            &files,
            "reshade-shaders/Shaders/crtsim-renderer/parameters.fxh",
        );
        for expected in [
            "#define CRTSIM_MASK_LEVELS 7\n",
            "#define CRTSIM_BEZEL_WIDTH 1024\n",
            "#ifndef CRTSIM_SIGNAL_HEIGHT\n#define CRTSIM_SIGNAL_HEIGHT 224\n#endif\n",
            "#define CRTSIM_TABLE_FILE \"crtsim-renderer/no-table.png\"\n",
            "uniform int CRTSIM_FIT <\n    ui_type = \"combo\";\n    ui_label = \"Fit on 4:3 tube\";\n    ui_items = \"original\\0contain\\0cover\\0stretch\\0\";\n> = 0;\n",
            "uniform float CRTSIM_BLOOM <\n    ui_type = \"slider\";\n    ui_label = \"Bloom amount\";\n",
        ] {
            assert!(header.contains(expected), "{expected:?} in\n{header}");
        }
        text(
            &files,
            "reshade-shaders/Shaders/crtsim-renderer/passes/resize.inc",
        );
        text(&files, "reshade-shaders/Shaders/CRTSimRenderer.fx");
        assert!(files
            .iter()
            .any(|f| f.path == "reshade-shaders/Textures/crtsim-renderer/glass.png"));
    }

    #[test]
    fn a_look_with_a_colour_table_names_it_and_its_size() {
        let with_palette = Config {
            palette: Some(Default::default()),
            ..Config::default()
        };
        let size = with_palette.lut_in_use().unwrap().size;
        let files = export(&[("nes", &with_palette)]).unwrap();
        assert!(files
            .iter()
            .any(|f| f.path == "reshade-shaders/Textures/crtsim-renderer/nes-table.png"));
        let preset = text(&files, "nes.ini");
        assert!(preset.contains(&format!(
            "CRTSIM_TABLE_FILE=\"crtsim-renderer/nes-table.png\",CRTSIM_TABLE_SIZE={size}\n"
        )));
        assert!(preset.contains("CRTSIM_TABLE=1\n"));
    }
}
