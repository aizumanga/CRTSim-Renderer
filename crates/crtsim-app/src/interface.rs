//! How lively the interface is: how much it moves, and whether it makes sounds. Remembered
//! between runs like the theme, and shared with the widgets through egui's context, as the
//! theme is through its style.
use crate::sound::Group;
use eframe::egui;

/// How much the interface moves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effects {
    /// Meters ease, switches slide, and the preview's screen switches on and flickers with
    /// static as a set's would.
    Full,
    /// Meters ease and switches slide, but the preview's screen stays still.
    Reduced,
    /// Nothing moves that need not.
    Off,
}

impl Effects {
    pub const ALL: [Self; 3] = [Self::Full, Self::Reduced, Self::Off];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Reduced => "reduced",
            Self::Off => "off",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Reduced => "Reduced",
            Self::Off => "Off",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Full => "Meters ease and switches slide; the preview switches on like a set.",
            Self::Reduced => "Meters ease and switches slide; the preview stays still.",
            Self::Off => "Nothing moves that need not.",
        }
    }

    /// How long, in seconds, a meter or switch takes to move to where it is now.
    pub fn glide(self) -> f32 {
        match self {
            Self::Full | Self::Reduced => 0.09,
            Self::Off => 0.,
        }
    }

    /// Whether the preview plays its screen's power-on and static.
    pub fn screen(self) -> bool {
        self == Self::Full
    }
}

impl Default for Effects {
    /// A browser page holds back, as the page's own reduced-motion setting cannot be read.
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self::Reduced
        } else {
            Self::Full
        }
    }
}

/// Whether the interface makes sounds, how loud, and which.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sounds {
    pub on: bool,
    /// From 0, silent, to 1.
    pub volume: f32,
    pub controls: bool,
    pub navigation: bool,
    pub events: bool,
}

impl Default for Sounds {
    /// Silent until a person turns them on.
    fn default() -> Self {
        Self {
            on: false,
            volume: 0.5,
            controls: true,
            navigation: true,
            events: true,
        }
    }
}

impl Sounds {
    /// Whether sounds of `group` are heard.
    pub fn hears(&self, group: Group) -> bool {
        self.on && self.volume > 0. && self.chosen(group)
    }

    /// Whether `group` is chosen, whether or not sounds are on.
    pub fn chosen(&self, group: Group) -> bool {
        match group {
            Group::Controls => self.controls,
            Group::Navigation => self.navigation,
            Group::Events => self.events,
        }
    }

    pub fn group_mut(&mut self, group: Group) -> &mut bool {
        match group {
            Group::Controls => &mut self.controls,
            Group::Navigation => &mut self.navigation,
            Group::Events => &mut self.events,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Interface {
    pub effects: Effects,
    pub sounds: Sounds,
}

impl Interface {
    /// Makes this how the interface behaves from the next frame: what widgets read with
    /// [`effects`], and how long egui's own animations, such as a section opening, take.
    pub fn apply(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(id(), self.effects));
        let animation_time = match self.effects {
            Effects::Off => 0.,
            Effects::Full | Effects::Reduced => egui::Style::default().animation_time,
        };
        ctx.all_styles_mut(|style| style.animation_time = animation_time);
    }

    /// As written to the app data: a line for each setting.
    pub fn to_text(self) -> String {
        let groups: Vec<&str> = Group::ALL
            .into_iter()
            .filter(|&group| self.sounds.chosen(group))
            .map(Group::id)
            .collect();
        format!(
            "effects: {}\nsounds: {}\nvolume: {}\nsound-groups: {}\n",
            self.effects.id(),
            if self.sounds.on { "on" } else { "off" },
            (self.sounds.volume * 100.).round(),
            groups.join(" "),
        )
    }

    /// As read from the app data. A setting left out keeps its default.
    pub fn from_text(text: &str) -> anyhow::Result<Self> {
        let mut interface = Self::default();
        for line in text.lines() {
            match line.split_once(": ") {
                Some(("effects", id)) => {
                    interface.effects = Effects::ALL
                        .into_iter()
                        .find(|effects| effects.id() == id)
                        .ok_or_else(|| anyhow::anyhow!("Unknown interface effects {id:?}"))?;
                }
                Some(("sounds", "on")) => interface.sounds.on = true,
                Some(("sounds", "off")) => interface.sounds.on = false,
                Some(("volume", percent)) => {
                    let percent: f32 = percent
                        .parse()
                        .map_err(|_| anyhow::anyhow!("Unknown sound volume {percent:?}"))?;
                    interface.sounds.volume = (percent / 100.).clamp(0., 1.);
                }
                Some(("sound-groups", ids)) => {
                    for group in Group::ALL {
                        *interface.sounds.group_mut(group) = false;
                    }
                    for id in ids.split_whitespace() {
                        let group = Group::ALL
                            .into_iter()
                            .find(|group| group.id() == id)
                            .ok_or_else(|| anyhow::anyhow!("Unknown sound group {id:?}"))?;
                        *interface.sounds.group_mut(group) = true;
                    }
                }
                _ => anyhow::bail!("Unknown interface setting {line:?}"),
            }
        }
        Ok(interface)
    }
}

fn id() -> egui::Id {
    egui::Id::new("crtsim interface effects")
}

/// How much the interface moves now: as last applied, or the default before then.
pub fn effects(ctx: &egui::Context) -> Effects {
    ctx.data(|data| data.get_temp(id())).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_round_trip_through_their_text() {
        let chosen = Interface {
            effects: Effects::Off,
            sounds: Sounds {
                on: true,
                volume: 0.3,
                controls: true,
                navigation: false,
                events: true,
            },
        };
        assert_eq!(Interface::from_text(&chosen.to_text()).unwrap(), chosen);
        assert_eq!(Interface::from_text("").unwrap(), Interface::default());
        assert!(Interface::from_text("effects: wobbly\n").is_err());
        assert!(Interface::from_text("sound-groups: whistles\n").is_err());
    }

    #[test]
    fn sounds_are_silent_until_turned_on_and_each_group_can_be_turned_off() {
        let mut sounds = Sounds::default();
        assert!(!sounds.hears(Group::Controls));
        sounds.on = true;
        assert!(Group::ALL.into_iter().all(|group| sounds.hears(group)));
        sounds.navigation = false;
        assert!(!sounds.hears(Group::Navigation) && sounds.hears(Group::Events));
        sounds.volume = 0.;
        assert!(!sounds.hears(Group::Events));
    }

    #[test]
    fn applied_effects_are_read_back_and_off_stops_egui_animating() {
        let ctx = egui::Context::default();
        assert_eq!(effects(&ctx), Effects::default());
        let off = Interface {
            effects: Effects::Off,
            ..Default::default()
        };
        off.apply(&ctx);
        assert_eq!(effects(&ctx), Effects::Off);
        assert_eq!(ctx.global_style().animation_time, 0.);
        Interface::default().apply(&ctx);
        assert!(ctx.global_style().animation_time > 0.);
    }
}
