//! Looks for Godot 4: the shader port in `ports/godot`, an add-on whose `CRTSimEffect`, a
//! `CompositorEffect`, runs the passes over a camera's picture each frame and keeps the signal
//! between frames for the glow. See
//! [the decision](../../../docs/adr/0006-bevy-embeds-the-renderer-godot-gets-a-compositor-port.md).
//!
//! An export is the whole add-on, under `addons/crtsim_renderer`: the effect's script, a
//! shader for each pass, the images they read, imported as they are, and each look as a saved
//! effect, a `.tres` that sets its parameters and names its colour table. Godot's importer
//! follows a shader's own includes but not theirs, so each pass is written whole: its shell,
//! the GLSL dialect the RetroArch port reads the passes through, and the passes it reads.

use super::{File, Host, Kind, Reading, Table};
use anyhow::Result;
use crtsim_core::config::Config;
use std::fmt::Write as _;

/// Where the add-on lives in a Godot project, which its script loads its files from.
const ADDON: &str = "addons/crtsim_renderer";

pub(super) const SCRIPT: &str = include_str!("../../../ports/godot/crtsim_effect.gd");
const README: &str = include_str!("../../../ports/godot/README.md");

/// The port's shaders, as `ports/godot/shaders` holds them: one file per pass, with the
/// vertex stage and the shell they share. They include the passes from `passes/`.
pub(super) const SOURCES: &[(&str, &str)] = &[
    (
        "vertex.inc",
        include_str!("../../../ports/godot/shaders/vertex.inc"),
    ),
    (
        "shell.inc",
        include_str!("../../../ports/godot/shaders/shell.inc"),
    ),
    (
        "capture.glsl",
        include_str!("../../../ports/godot/shaders/capture.glsl"),
    ),
    (
        "rows.glsl",
        include_str!("../../../ports/godot/shaders/rows.glsl"),
    ),
    (
        "columns.glsl",
        include_str!("../../../ports/godot/shaders/columns.glsl"),
    ),
    (
        "prepare.glsl",
        include_str!("../../../ports/godot/shaders/prepare.glsl"),
    ),
    (
        "composite.glsl",
        include_str!("../../../ports/godot/shaders/composite.glsl"),
    ),
    (
        "surface.glsl",
        include_str!("../../../ports/godot/shaders/surface.glsl"),
    ),
    (
        "bloom_down.glsl",
        include_str!("../../../ports/godot/shaders/bloom_down.glsl"),
    ),
    (
        "bloom_up.glsl",
        include_str!("../../../ports/godot/shaders/bloom_up.glsl"),
    ),
    (
        "present.glsl",
        include_str!("../../../ports/godot/shaders/present.glsl"),
    ),
];

/// The script's lines the exporter writes, between these two.
const BEGIN: &str = "# BEGIN PARAMETERS";
const END: &str = "# END PARAMETERS";

/// The look a new effect starts with: the app's for ordinary pictures, since a game's is one.
fn defaults() -> Config {
    Config::general()
}

/// The add-on with `looks` as saved effects, each named by its file name without `.tres`.
pub fn export(looks: &[(&str, &Config)]) -> Result<Vec<File>> {
    super::check_looks(looks)?;
    let mut files = vec![
        super::text(&format!("{ADDON}/README.md"), README),
        super::text(&format!("{ADDON}/crtsim_effect.gd"), &script()),
    ];
    files.extend(
        shaders()
            .into_iter()
            .map(|(name, source)| super::text(&format!("{ADDON}/shaders/{name}"), &source)),
    );
    for image in super::image_files(&format!("{ADDON}/images"))? {
        let mipmapped = super::IMAGES
            .iter()
            .any(|i| image.path.ends_with(i.file) && i.reading == Reading::Mipmapped);
        files.push(import(&image.path, mipmapped));
        files.push(image);
    }
    for (name, config) in looks {
        let table = match Table::of(config, name)? {
            Some(table) => {
                let file = table.at(&format!("{ADDON}/looks"));
                files.push(import(&file.path, false));
                files.push(file);
                Some(format!("res://{ADDON}/looks/{}", table.name))
            }
            None => None,
        };
        files.push(super::text(
            &format!("{ADDON}/looks/{name}.tres"),
            &look(config, table.as_deref()),
        ));
    }
    Ok(files)
}

/// Each pass's shader, with everything it includes written in its place.
fn shaders() -> Vec<(&'static str, String)> {
    let uniforms = uniforms();
    let mut sources: Vec<(String, &str)> = vec![
        ("parameters.inc".into(), &uniforms),
        ("dialect.inc".into(), super::retroarch::DIALECT),
    ];
    sources.extend(
        SOURCES
            .iter()
            .map(|(name, source)| (name.to_string(), *source)),
    );
    sources.extend(
        super::PASSES
            .iter()
            .map(|(name, source)| (format!("passes/{name}"), *source)),
    );
    SOURCES
        .iter()
        .filter(|(name, _)| name.ends_with(".glsl"))
        .map(|(name, source)| (*name, flatten(source, &sources)))
        .collect()
}

/// `text` with every include written in its place, from `sources` by name.
fn flatten(text: &str, sources: &[(String, &str)]) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let included = line
            .trim()
            .strip_prefix("#include \"")
            .and_then(|rest| rest.strip_suffix('"'));
        match included {
            Some(name) => {
                let (_, source) = sources
                    .iter()
                    .find(|(known, _)| known == name)
                    .unwrap_or_else(|| panic!("the Godot port includes {name}, which it has"));
                out.push_str(&flatten(source, sources));
            }
            None => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out
}

/// The import settings of an image the passes read: as it is, its colours untouched and
/// uncompressed, so each texel reads as the renderer's; with mipmaps only where the passes
/// filter it so.
fn import(path: &str, mipmapped: bool) -> File {
    super::text(
        &format!("{path}.import"),
        &format!(
            "[remap]\n\n\
             importer=\"texture\"\n\
             type=\"CompressedTexture2D\"\n\n\
             [params]\n\n\
             compress/mode=0\n\
             compress/normal_map=2\n\
             compress/channel_pack=0\n\
             mipmaps/generate={mipmapped}\n\
             mipmaps/limit=-1\n\
             roughness/mode=1\n\
             process/fix_alpha_border=false\n\
             process/premult_alpha=false\n\
             process/size_limit=0\n\
             detect_3d/compress_to=0\n"
        ),
    )
}

/// A look as a saved effect: every parameter, and its colour table where it has one.
fn look(config: &Config, table: Option<&str>) -> String {
    let mut out = format!(
        "[gd_resource type=\"CompositorEffect\" script_class=\"CRTSimEffect\" load_steps={} \
         format=3]\n\n\
         [ext_resource type=\"Script\" path=\"res://{ADDON}/crtsim_effect.gd\" id=\"1\"]\n",
        if table.is_some() { 3 } else { 2 }
    );
    if let Some(table) = table {
        writeln!(
            out,
            "[ext_resource type=\"Texture2D\" path=\"{table}\" id=\"2\"]"
        )
        .expect("writing to a string");
    }
    out.push_str("\n[resource]\nscript = ExtResource(\"1\")\n");
    for p in super::parameters(Host::Godot) {
        writeln!(out, "{} = {}", property(p.name), value(&p, p.of(config)))
            .expect("writing to a string");
    }
    if table.is_some() {
        out.push_str("colour_table = ExtResource(\"2\")\n");
    }
    out
}

/// A parameter's property on the effect: its name without `CRTSIM_`, in lower case.
fn property(name: &str) -> String {
    name.trim_start_matches("CRTSIM_").to_lowercase()
}

/// A value as GDScript writes it: a choice as its item's number, a number as short as the
/// f32 prints, with a point so it reads as a float.
fn value(p: &super::Parameter, value: f32) -> String {
    match p.kind {
        Kind::Choice(_) => format!("{}", value.round() as i32),
        Kind::Number { .. } => format!("{value:?}"),
    }
}

/// `parameters.inc`: the block every pass reads, the sizes and the frame first, then every
/// parameter, in the order the effect's script fills it.
fn uniforms() -> String {
    let mut out = String::from(
        "// CC0. Written by CRTSim Renderer's exporter from its settings table; edit that, not this.\n\
         layout(set = 0, binding = 0, std140) uniform Parameters {\n\
         \x20   vec2 crtsim_original_size;\n\
         \x20   vec2 crtsim_signal_size;\n\
         \x20   vec2 crtsim_output_size;\n\
         \x20   float crtsim_frame_count;\n\
         \x20   float crtsim_frame_ticks;\n\
         \x20   float crtsim_has_previous;\n",
    );
    for p in super::parameters(Host::Godot) {
        writeln!(out, "    float {};", p.name).expect("writing to a string");
    }
    out.push_str("} u;\n");
    out
}

/// The effect's script with its parameters written in, as `ports/godot/crtsim_effect.gd`
/// holds it. The add-on is used from it, so it is kept in the repository, and a test checks
/// it is what this writes.
pub fn script() -> String {
    let begin = SCRIPT.find(BEGIN).expect("the script marks its parameters");
    let begin = begin + SCRIPT[begin..].find('\n').expect("a line") + 1;
    // The marker's own comment runs on to the first line that is not one.
    let begin = begin
        + SCRIPT[begin..]
            .lines()
            .take_while(|line| line.starts_with("# ") && !line.starts_with(END))
            .map(|line| line.len() + 1)
            .sum::<usize>();
    let end = SCRIPT.find(END).expect("the script ends its parameters");
    format!("{}{}{}", &SCRIPT[..begin], exports(), &SCRIPT[end..])
}

/// Each parameter as a property the inspector shows, with its range and the starting look's
/// value, and their order in the block the passes read.
fn exports() -> String {
    let start = defaults();
    let parameters = super::parameters(Host::Godot);
    let mut out = String::new();
    for p in &parameters {
        writeln!(out, "## {}", p.label).expect("writing to a string");
        let name = property(p.name);
        let default = value(p, p.of(&start));
        match p.kind {
            Kind::Choice(items) => {
                let items: Vec<_> = items.iter().map(|item| format!("{item:?}")).collect();
                writeln!(
                    out,
                    "@export_enum({}) var {name}: int = {default}",
                    items.join(", ")
                )
            }
            Kind::Number { min, max, step } => writeln!(
                out,
                "@export_range({min:?}, {max:?}, {step:?}) var {name}: float = {default}"
            ),
        }
        .expect("writing to a string");
    }
    out.push_str("\n## The parameters in the order parameters.inc declares them.\n");
    out.push_str("const _PARAMETERS: Array[StringName] = [\n");
    for p in &parameters {
        writeln!(out, "\t&\"{}\",", property(p.name)).expect("writing to a string");
    }
    out.push_str("]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_in_the_repository_is_what_the_exporter_writes() {
        let written = script();
        if std::env::var_os("CRTSIM_WRITE_GODOT_SCRIPT").is_some() {
            let path = concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../ports/godot/crtsim_effect.gd"
            );
            std::fs::write(path, &written).unwrap();
            return;
        }
        assert!(
            written == SCRIPT,
            "ports/godot/crtsim_effect.gd is out of date: run with CRTSIM_WRITE_GODOT_SCRIPT=1"
        );
    }

    #[test]
    fn a_look_is_a_saved_effect_with_its_table() {
        let nes = Config {
            palette: Some(Default::default()),
            ..Config::default()
        };
        let files = export(&[("plain", &Config::general()), ("nes", &nes)]).unwrap();
        let named = |path: &str| files.iter().find(|f| f.path == format!("{ADDON}/{path}"));
        let plain = String::from_utf8(named("looks/plain.tres").unwrap().bytes.clone()).unwrap();
        assert!(plain.contains("sharpness = 0.8\n"), "{plain}");
        assert!(!plain.contains("colour_table"));
        let nes = String::from_utf8(named("looks/nes.tres").unwrap().bytes.clone()).unwrap();
        assert!(nes.contains("table = 1\n") && nes.contains("colour_table = ExtResource"));
        assert!(named("looks/nes-table.png").is_some());
        assert!(named("looks/nes-table.png.import").is_some());
        let mask = String::from_utf8(named("images/mask.png.import").unwrap().bytes.clone());
        assert!(mask.unwrap().contains("mipmaps/generate=true"));
        for (name, _) in SOURCES.iter().filter(|(name, _)| name.ends_with(".glsl")) {
            let shader = named(&format!("shaders/{name}")).expect(name);
            let shader = String::from_utf8(shader.bytes.clone()).unwrap();
            assert!(!shader.contains("#include"), "{name} is written whole");
            assert!(shader.contains("#[vertex]") && shader.contains("#[fragment]"));
        }
        let surface = String::from_utf8(named("shaders/surface.glsl").unwrap().bytes.clone());
        assert!(surface.unwrap().contains("float4 crtsim_surface("));
    }

    #[test]
    fn every_parameter_is_a_property_the_block_reads_in_order() {
        let block = uniforms();
        let script = script();
        let mut at = 0;
        for p in super::super::parameters(Host::Godot) {
            let found = block[at..]
                .find(&format!("float {};", p.name))
                .unwrap_or_else(|| panic!("{} in parameters.inc, in order", p.name));
            at += found;
            assert!(script.contains(&format!("var {}:", property(p.name))));
        }
    }
}
