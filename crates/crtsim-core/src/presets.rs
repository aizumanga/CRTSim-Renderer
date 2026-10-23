//! The included presets: the looks the app's gallery offers, by name, and the CLI renders with
//! `--preset`. Personal presets are the app's own, kept in its app data.
use crate::config::{ColorMode, Config, Filter, MaskRepeats, Phase};

/// A look, named and described for a person choosing it.
#[derive(Clone)]
pub struct Preset {
    pub name: &'static str,
    pub description: &'static str,
    pub config: Config,
}

fn entry(name: &'static str, description: &'static str, config: Config) -> Preset {
    Preset {
        name,
        description,
        config,
    }
}
/// The preset the app starts with, and Reset and each setting's own reset return to: the game's
/// CRT options without its NES palette, which recolours only art drawn in the NES palette, so
/// whatever is opened keeps its colours.
pub fn default() -> Preset {
    entry(
        "Super Win the Game",
        "The game's own CRT options: the public reference with a 30° field of view, NTSC \
         blending at 0.35, the artifact pattern as its Windows build draws it, the bezel \
         reflecting the picture as the screen shows it, a grey backdrop, the bloom read \
         through the picture's mipmaps and the glow held at its 8-bit floor. Its NES palette \
         is off, so a picture keeps its colours.",
        Config {
            palette: None,
            ..Config::super_win_the_game()
        },
    )
}

/// Every included preset, the default first, in the order the gallery shows them.
pub fn included() -> Vec<Preset> {
    let general = Config::general();
    vec![
        default(),
        entry(
            "Super Win the Game NES palette",
            "Super Win the Game with its NTSC palette (Tint 5.18, I 1.75, Q 1.00) made as the \
             game makes it, which recolours art drawn in the NES palette the game's is drawn \
             in, as the game shows it.",
            Config::super_win_the_game(),
        ),
        entry(
            "General image",
            "Square pixels, smooth resizing, balanced CRT effects.",
            general.clone(),
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
                mask_repeats: MaskRepeats::Signal,
                ..general.clone()
            },
        ),
        entry(
            "NTSC 240p",
            "240 progressive rows, as consoles and computers drove an NTSC set, with the composite \
             phase alternating each tick. Nearest resizing for pixel art.",
            Config {
                signal: "240p".into(),
                filter: Filter::Nearest,
                phase: Phase::Alternating,
                mask_opacity: 0.7,
                mask_repeats: MaskRepeats::Signal,
                ..general.clone()
            },
        ),
        entry(
            "NTSC 480i",
            "480 interlaced rows with the composite phase alternating each field, as NTSC \
             broadcast and video were shown.",
            Config {
                signal: "480p".into(),
                interlace: true,
                phase: Phase::Alternating,
                mask_repeats: MaskRepeats::Signal,
                ..general.clone()
            },
        ),
        entry(
            "Widescreen television",
            "A 16:9 tube, as the last sets had, showing 480 interlaced rows with the composite \
             phase alternating each field: for widescreen video.",
            Config {
                tube_aspect: 16. / 9.,
                signal: "480p".into(),
                interlace: true,
                phase: Phase::Alternating,
                mask_repeats: MaskRepeats::Signal,
                ..general.clone()
            },
        ),
        entry(
            "PAL 288p",
            "288 progressive rows, as consoles and computers drove a PAL set. PAL's \
             line-alternating color is not simulated; gentler, stable artifacts stand in for it.",
            Config {
                signal: "288p".into(),
                filter: Filter::Nearest,
                phase: Phase::Stable,
                artifacts: 0.25,
                mask_opacity: 0.7,
                mask_repeats: MaskRepeats::Signal,
                ..general.clone()
            },
        ),
        entry(
            "PAL 576i",
            "576 interlaced rows, as PAL broadcast and video were shown. PAL's line-alternating \
             color is not simulated; gentler, stable artifacts stand in for it.",
            Config {
                signal: "576p".into(),
                interlace: true,
                phase: Phase::Stable,
                artifacts: 0.25,
                mask_repeats: MaskRepeats::Signal,
                ..general.clone()
            },
        ),
        entry(
            "Warm analog",
            "An optional hue/chroma grade, not Super Win the Game's NES palette.",
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

/// The included preset called `name`, in any letter case.
pub fn named(name: &str) -> Option<Preset> {
    included()
        .into_iter()
        .find(|preset| preset.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn included_presets_are_valid_and_named_once() {
        let presets = included();
        for preset in &presets {
            preset.config.validate().unwrap();
            preset.config.signal_size((1216, 832)).unwrap();
            assert_eq!(
                named(&preset.name.to_uppercase()).unwrap().name,
                preset.name
            );
        }
        let mut names: Vec<_> = presets.iter().map(|p| p.name.to_lowercase()).collect();
        names.dedup();
        assert_eq!(names.len(), presets.len());
        assert_eq!(presets[0].name, default().name);
    }
}
