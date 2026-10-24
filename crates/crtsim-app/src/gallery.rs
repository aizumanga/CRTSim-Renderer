use crate::{app_data, model};
use crtsim_core::config::Config;

#[derive(Clone)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub config: Config,
    pub user: bool,
}
fn entry(preset: crtsim_core::presets::Preset) -> Entry {
    Entry {
        name: preset.name.into(),
        description: preset.description.into(),
        config: preset.config,
        user: false,
    }
}
/// The preset the app starts with, and Reset and each setting's own reset return to.
pub fn default_preset() -> Entry {
    entry(crtsim_core::presets::default())
}

pub fn builtins() -> Vec<Entry> {
    crtsim_core::presets::included()
        .into_iter()
        .map(entry)
        .collect()
}

/// Whether `name` is an included preset's, in any letter case. No personal preset is saved
/// under one, so the gallery never shows two presets by the same name, nor one that seems to
/// replace an included preset.
pub fn is_included(name: &str) -> bool {
    builtins()
        .iter()
        .any(|entry| entry.name.to_lowercase() == name.to_lowercase())
}

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
    /// The preset closest to some settings, and the settings it was found for.
    closest: Option<(Config, Option<Closest>)>,
}

/// The preset a look is closest to, and the settings that differ from it.
pub struct Closest {
    pub name: String,
    pub changes: Vec<model::Difference>,
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

    /// The preset `config` is closest to, the one it differs from in fewest settings, found
    /// again only once `config` or the presets change. Of presets as close, the first wins, so
    /// an included one before a personal one.
    pub fn closest(&mut self, config: &Config) -> Option<&Closest> {
        if self.closest.as_ref().is_none_or(|(of, _)| of != config) {
            let closest = self
                .entries
                .iter()
                .map(|entry| Closest {
                    name: entry.name.clone(),
                    changes: model::differences(&entry.config, config),
                })
                .min_by_key(|closest| closest.changes.len());
            self.closest = Some((config.clone(), closest));
        }
        self.closest.as_ref()?.1.as_ref()
    }

    /// Reads the personal presets again, which may have changed on disk.
    pub fn refresh(&mut self, store: Option<&app_data::Store>) {
        self.closest = None;
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
    use crtsim_core::config::MaskRepeats;
    #[test]
    fn included_presets_are_valid() {
        for e in builtins() {
            e.config.validate().unwrap();
            e.config.signal_size((1216, 832)).unwrap();
        }
    }

    #[test]
    fn the_closest_preset_is_the_one_the_settings_differ_least_from() {
        let mut gallery = PresetGallery::new(None);
        let mut config = default_preset().config;
        let closest = gallery.closest(&config).unwrap();
        assert_eq!(closest.name, "Super Win the Game");
        assert!(closest.changes.is_empty());
        config.bloom += 0.25;
        let closest = gallery.closest(&config).unwrap();
        assert_eq!(closest.name, "Super Win the Game");
        assert_eq!(closest.changes.len(), 1);
        assert_eq!(
            gallery.closest(&Config::general()).unwrap().name,
            "General image"
        );
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
        let named = |name| builtins().into_iter().find(|e| e.name == name).unwrap();
        let game = named("Super Win the Game NES palette").config;
        // The app starts with the game's options without its palette, which recolours only
        // art drawn in the NES palette.
        assert_eq!(named("Super Win the Game").config, default_preset().config);
        assert_eq!(
            default_preset().config,
            Config {
                palette: None,
                ..game.clone()
            }
        );
        // As the game's CRT options show them.
        assert_eq!(game.fov, 30.);
        assert_eq!(game.ntsc_blending, 0.35);
        assert_eq!(game.lut_strength, 1.);
        // As the Windows build draws the artifact pattern; the Linux build flips it.
        assert!(!game.flip_artifacts);
        // Its monitor pass reflects the picture with the screen's overscan and mask density.
        assert!(game.reflection_as_screen);
        // Its engine draws a backdrop at 1/16 grey, seen past the bezel in wide windows.
        assert_eq!(game.backdrop_color, [0.0625; 3]);
        // Its bloom reads the picture's mipmaps, and its history rounds black to a floor.
        assert!(game.bloom_from_mipmaps && game.glow_floor && game.glow_per_frame);
        assert!(game.screenshot_stills && game.faceted_glass);
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
                "Super Win the Game"
                    | "Super Win the Game NES palette"
                    | "Pixel art 240p"
                    | "NTSC 240p"
                    | "NTSC 480i"
                    | "Widescreen television"
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
