//! Looks for the OBS filter: the shader port in `ports/obs`, a native plugin whose filter runs
//! the passes over a source's picture and keeps the signal between frames for the glow, which
//! OBS's own shader filters cannot.
//!
//! The plugin is built from its source against OBS, and reads its effect, the passes and the
//! images from its data folder, which `plugin_data` writes. Its settings are the port's
//! parameters, with the app's starting look for ordinary pictures as their defaults. A look
//! is a JSON file of those settings, with its colour table beside it, which the filter's
//! properties load.

use super::{File, Host, Kind, Reading, Table};
use crate::config::Config;
use anyhow::Result;
use serde_json::{json, Map, Value};
use std::fmt::Write as _;

pub(super) const EFFECT: &str = include_str!("../../../../ports/obs/crtsim.effect");
const README: &str = include_str!("../../../../ports/obs/README.md");
/// The key of a look's colour table, a path relative to the look; empty when it has none.
const TABLE_FILE: &str = "table_file";

/// The look the filter starts with: the app's for ordinary pictures, since a source is one.
fn defaults() -> Config {
    Config::general()
}

/// The files that make `looks` looks for the filter, each named by its file name without
/// `.json`, with the plugin's README.
pub fn export(looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    super::check_looks(looks)?;
    let mut files = vec![super::text("README.md", README)];
    files.extend(look_files("", looks)?);
    Ok(files)
}

/// A look's settings and its colour table, written into `dir`.
fn look_files(dir: &str, looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    let mut files = vec![];
    for (name, config) in looks {
        let table = match Table::of(config, name)? {
            Some(table) => {
                files.push(table.at(dir));
                table.name
            }
            None => String::new(),
        };
        let mut settings = Map::new();
        for p in super::parameters(Host::Obs) {
            settings.insert(p.name.into(), number(p.of(config)));
        }
        settings.insert(TABLE_FILE.into(), Value::String(table));
        let text = serde_json::to_string_pretty(&Value::Object(settings))? + "\n";
        files.push(super::text(
            &super::join(dir, &format!("{name}.json")),
            &text,
        ));
    }
    Ok(files)
}

/// A setting as JSON writes it: as short as the f32 prints, not as its f64 widening does.
fn number(value: f32) -> Value {
    json!(value
        .to_string()
        .parse::<f64>()
        .expect("a float prints as one"))
}

/// The plugin's data folder: its effect, the parameters it declares, the passes, the images
/// they read, and the shipped looks.
pub fn plugin_data() -> Result<Vec<File>> {
    let mut files = vec![
        super::text("crtsim.effect", EFFECT),
        super::text("parameters.inc", &uniforms()),
    ];
    files.extend(super::passes("passes"));
    files.extend(super::image_files("images")?);
    let shipped = super::shipped();
    let looks: Vec<_> = shipped.iter().map(|(name, c)| (name.as_str(), c)).collect();
    files.extend(look_files("looks", &looks)?);
    Ok(files)
}

/// `parameters.inc`: every parameter as a uniform the filter sets.
fn uniforms() -> String {
    let mut out = String::from(
        "// CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not this.\n",
    );
    for p in super::parameters(Host::Obs) {
        writeln!(out, "uniform float {};", p.name).expect("writing to a string");
    }
    out
}

/// A string as C writes it.
fn c_string(text: &str) -> String {
    format!("{text:?}")
}

/// A number as C writes a float.
fn c_float(value: f32) -> String {
    format!("{value:?}f")
}

/// `ports/obs/src/port.h`: the parameters the filter shows and sets, with their ranges and
/// defaults, and the images it loads, each with an index. The plugin is built from it, so it
/// is kept in the repository, and a test checks it is what this writes.
pub fn header() -> String {
    let mut out = String::from(
        "/* CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not\n \
         * this. `cargo test -p crtsim-core obs` says when it is out of date, and\n \
         * CRTSIM_WRITE_OBS_HEADER=1 rewrites it. */\n\n\
         #pragma once\n\n\
         #include <stdbool.h>\n\
         #include <stddef.h>\n",
    );
    parameters_table(&mut out);
    images_table(&mut out);
    out
}

/// The parameters, by index.
fn parameters_table(out: &mut String) {
    let start = defaults();
    let parameters = super::parameters(Host::Obs);
    out.push_str(
        "\n/* A setting: a number in a range, or a choice of the items, numbered from 0. */\n\
         struct crtsim_parameter {\n\
         \tconst char *name;\n\
         \tconst char *label;\n\
         \tfloat min, max, step;\n\
         \tfloat value;\n\
         \tconst char *const *items;\n\
         };\n\n",
    );
    for p in &parameters {
        if let Kind::Choice(items) = p.kind {
            let items: Vec<_> = items.iter().map(|item| c_string(item)).collect();
            writeln!(
                out,
                "static const char *const {}_ITEMS[] = {{{}, NULL}};",
                p.name,
                items.join(", ")
            )
            .expect("writing to a string");
        }
    }
    out.push_str("\nenum crtsim_parameter_index {\n");
    for p in &parameters {
        writeln!(out, "\t{}_INDEX,", p.name).expect("writing to a string");
    }
    out.push_str(
        "\tCRTSIM_PARAMETER_COUNT,\n};\n\n\
         static const struct crtsim_parameter crtsim_parameters[CRTSIM_PARAMETER_COUNT] = {\n",
    );
    for p in &parameters {
        let (min, max, step) = p.range();
        let items = match p.kind {
            Kind::Choice(_) => format!("{}_ITEMS", p.name),
            Kind::Number { .. } => "NULL".into(),
        };
        writeln!(
            out,
            "\t{{{}, {}, {}, {}, {}, {}, {items}}},",
            c_string(p.name),
            c_string(p.label),
            c_float(min),
            c_float(max),
            c_float(step),
            c_float(p.of(&start)),
        )
        .expect("writing to a string");
    }
    out.push_str("};\n");
}

/// The images the plugin loads from its data folder, by index, with the effect's texture for
/// each. An image the passes also read filtered is loaded twice, since OpenGL filters a texture
/// as the last sampler it was read with says. The table a look without one is given has no
/// texture of its own: it stands in for a look's table.
fn images_table(out: &mut String) {
    struct Loaded {
        index: String,
        file: &'static str,
        param: Option<String>,
        mipmapped: bool,
    }
    let mut loaded = vec![];
    for image in &super::IMAGES {
        let stem = image.file.trim_end_matches(".png").replace('-', "_");
        let index = format!("CRTSIM_IMAGE_{}", stem.to_uppercase());
        loaded.push(Loaded {
            index: index.clone(),
            file: image.file,
            param: (image.file != super::NO_TABLE).then(|| format!("{stem}_image")),
            mipmapped: image.reading == Reading::Mipmapped,
        });
        if image.reading == Reading::AlsoFiltered {
            loaded.push(Loaded {
                index: format!("{index}_FILTERED"),
                file: image.file,
                param: Some(format!("{stem}_filtered")),
                mipmapped: false,
            });
        }
    }
    out.push_str(
        "\n/* An image the passes read, from the data folder, and the effect's texture for it. */\n\
         struct crtsim_image {\n\
         \tconst char *file;\n\
         \tconst char *param;\n\
         \tbool mipmapped;\n\
         };\n\n\
         enum crtsim_image_index {\n",
    );
    for image in &loaded {
        writeln!(out, "\t{},", image.index).expect("writing to a string");
    }
    out.push_str(
        "\tCRTSIM_IMAGE_COUNT,\n};\n\n\
         static const struct crtsim_image crtsim_images[CRTSIM_IMAGE_COUNT] = {\n",
    );
    for image in &loaded {
        let param = image.param.as_deref().map_or("NULL".into(), c_string);
        let file = c_string(&format!("images/{}", image.file));
        writeln!(out, "\t{{{file}, {param}, {}}},", image.mipmapped).expect("writing to a string");
    }
    out.push_str("};\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugins_header_is_the_tables() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../ports/obs/src/port.h");
        let written = header();
        if std::env::var_os("CRTSIM_WRITE_OBS_HEADER").is_some() {
            std::fs::write(path, &written).unwrap();
        }
        let kept = std::fs::read_to_string(path).unwrap_or_default();
        assert!(
            kept == written,
            "ports/obs/src/port.h is out of date: rerun with CRTSIM_WRITE_OBS_HEADER=1"
        );
    }

    #[test]
    fn every_parameter_the_effect_reads_is_declared() {
        let names: Vec<_> = super::super::parameters(Host::Obs)
            .iter()
            .map(|p| p.name)
            .collect();
        for word in super::super::tests::parameters_read(&[("crtsim.effect", EFFECT)], &[]) {
            assert!(
                names.contains(&word.as_str()),
                "the effect reads {word}, not declared"
            );
        }
    }

    #[test]
    fn a_look_is_its_settings_and_its_colour_table() {
        let mut look = Config::general();
        look.bloom = 0.5;
        look.palette = Some(Default::default());
        let files = export(&[("warm", &look)]).unwrap();
        let settings: Value =
            serde_json::from_slice(&files.iter().find(|f| f.path == "warm.json").unwrap().bytes)
                .unwrap();
        assert_eq!(settings["CRTSIM_BLOOM"], json!(0.5));
        assert_eq!(settings["CRTSIM_TABLE"], json!(1.0));
        assert_eq!(settings["CRTSIM_SIGNAL_HEIGHT"], json!(480.0));
        assert_eq!(settings[TABLE_FILE], json!("warm-table.png"));
        assert!(files.iter().any(|f| f.path == "warm-table.png"));
        assert!(
            settings.get("CRTSIM_ASPECT").is_none(),
            "OBS has the pixel aspect instead"
        );
    }

    #[test]
    fn the_plugins_data_holds_what_its_effect_includes() {
        let files = plugin_data().unwrap();
        for path in [
            "crtsim.effect",
            "parameters.inc",
            "passes/surface.inc",
            "images/bezel-shape.png",
            "looks/crtsim-renderer.json",
            "looks/crtsim-renderer-nes-palette-table.png",
        ] {
            assert!(files.iter().any(|f| f.path == path), "{path} in the data");
        }
        let looks = files
            .iter()
            .find(|f| f.path == "looks/crtsim-renderer.json")
            .unwrap();
        let settings: Value = serde_json::from_slice(&looks.bytes).unwrap();
        assert_eq!(settings[TABLE_FILE], json!(""));
    }
}
