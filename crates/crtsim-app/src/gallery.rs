use crate::app_data;
use crtsim_core::config::{ColorMode, Config, Filter, MaskRepeats, Phase};

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
    let general = Config::general();
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
            "Super Win the Game",
            "The game's own CRT options: the public reference with a 30° field of view, NTSC \
             blending at 0.35 and its NTSC palette (Tint 5.18, I 1.75, Q 1.00), made as the \
             game makes it, with the artifact pattern flipped as its Linux build draws it. The \
             palette recolours art in the NES palette the game's is drawn in.",
            Config {
                fov: 30.,
                phase: Phase::Alternating,
                ntsc_blending: 0.35,
                flip_artifacts: true,
                palette: Some(Default::default()),
                ..Config::default()
            },
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
            "An optional hue/chroma grade, not the game's NES palette.",
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

/// Every preset the gallery lists: the included ones, then the personal ones in `store`, with a
/// warning for each personal preset that could not be read.
/// The preset gallery: the presets it offers, the files skipped while reading them, the name
/// the next saved preset takes and a description being edited.
#[derive(Default)]
pub struct PresetGallery {
    pub open: bool,
    /// Included presets, then personal ones.
    pub entries: Vec<Entry>,
    /// Why personal preset files were skipped.
    pub warnings: Vec<String>,
    /// The name Save current gives the settings in use.
    pub name: String,
    /// The personal preset whose description is being edited, and its text so far.
    pub editing: Option<(String, String)>,
    /// A change to a personal preset waiting for the person to confirm it.
    pub confirming: Option<Confirm>,
}

/// A change to a personal preset that cannot be undone, asked about before it is made.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Confirm {
    /// Saving the settings in use over the preset of this name.
    Replace(String),
    /// Deleting the preset of this name.
    Delete(String),
}

impl PresetGallery {
    /// The included presets, and the personal ones in `store`.
    pub fn new(store: Option<&app_data::Store>) -> Self {
        let mut gallery = Self::default();
        gallery.refresh(store);
        gallery
    }

    /// Reads the personal presets again, which may have changed on disk.
    pub fn refresh(&mut self, store: Option<&app_data::Store>) {
        self.entries = builtins();
        self.warnings = match store.map(app_data::Store::presets) {
            Some(Ok((personal, warnings))) => {
                self.entries
                    .extend(personal.into_iter().map(|preset| Entry {
                        name: preset.name,
                        description: preset.description,
                        config: preset.config,
                        user: true,
                    }));
                warnings
            }
            Some(Err(e)) => vec![format!("{e:#}")],
            None => vec![],
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn included_presets_are_valid() {
        for e in builtins() {
            e.config.validate().unwrap();
            e.config.signal_size((1216, 832)).unwrap();
        }
    }

    #[test]
    fn ntsc_and_pal_presets_have_their_line_counts_and_scanning() {
        let presets = builtins();
        for (name, rows, interlaced) in [
            ("NTSC 240p", 240, false),
            ("NTSC 480i", 480, true),
            ("PAL 288p", 288, false),
            ("PAL 576i", 576, true),
        ] {
            let preset = presets.iter().find(|e| e.name == name).unwrap();
            assert_eq!(
                preset.config.signal_size((1920, 1080)).unwrap().1,
                rows,
                "{name}"
            );
            assert_eq!(preset.config.interlace, interlaced, "{name}");
        }
    }

    #[test]
    fn the_game_preset_matches_its_options_screen() {
        let game = builtins()
            .into_iter()
            .find(|e| e.name == "Super Win the Game")
            .unwrap()
            .config;
        // As the game's CRT options show them.
        assert_eq!(game.fov, 30.);
        assert_eq!(game.ntsc_blending, 0.35);
        assert_eq!(game.lut_strength, 1.);
        // As the Linux build, whose frames it was compared with, draws the artifact pattern.
        assert!(game.flip_artifacts);
        let palette = game.palette.unwrap();
        assert_eq!(
            (palette.tint, palette.tint_i, palette.tint_q, palette.model),
            (5.183186, 1.75, 1., crtsim_core::palette::Model::Game)
        );
        let reference = Config::default();
        assert_eq!(
            (
                game.overscan,
                game.barrel,
                game.pixel_aspect,
                game.saturation
            ),
            (
                reference.overscan,
                reference.barrel,
                reference.pixel_aspect,
                1.35
            )
        );
        assert_eq!((game.mask_opacity, game.mask_brightness), (1., 0.45));
        assert_eq!(
            (game.sharpness, game.persistence[0], game.bleed),
            (0.8, 0.7, 0.5)
        );
        assert_eq!(
            (game.bloom, game.bloom_power, game.frame_color),
            (0.25, 2., [0.06; 3])
        );
    }

    #[test]
    fn presets_for_a_line_count_have_the_mask_follow_it() {
        for entry in builtins() {
            let follows = matches!(
                entry.name.as_str(),
                "Original CRTSim"
                    | "Super Win the Game"
                    | "Pixel art 240p"
                    | "NTSC 240p"
                    | "NTSC 480i"
                    | "PAL 288p"
                    | "PAL 576i"
            );
            let expected = if follows {
                MaskRepeats::Signal
            } else {
                MaskRepeats::Fixed([128., 224.])
            };
            assert_eq!(entry.config.mask_repeats, expected, "{}", entry.name);
        }
    }
}
