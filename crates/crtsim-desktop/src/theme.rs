use eframe::egui::{self, Color32, Stroke};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Theme {
    #[default]
    CrtDark,
    PaperLight,
    LunaBlue,
    ClassicPlatinum,
    SkyDiary,
}

impl Theme {
    pub const ALL: [Self; 5] = [
        Self::CrtDark,
        Self::PaperLight,
        Self::LunaBlue,
        Self::ClassicPlatinum,
        Self::SkyDiary,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::CrtDark => "crt-dark",
            Self::PaperLight => "paper-light",
            Self::LunaBlue => "luna-blue",
            Self::ClassicPlatinum => "classic-platinum",
            Self::SkyDiary => "sky-diary",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::CrtDark => "CRT Dark",
            Self::PaperLight => "Paper Light",
            Self::LunaBlue => "Luna Blue",
            Self::ClassicPlatinum => "Classic Platinum",
            Self::SkyDiary => "Sky Diary",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::CrtDark => "Low-glare charcoal with a phosphor-green accent.",
            Self::PaperLight => "A clear, neutral light mode for bright rooms.",
            Self::LunaBlue => "Blue-and-olive early-2000s desktop colors.",
            Self::ClassicPlatinum => "Cool gray panels inspired by classic desktop chrome.",
            Self::SkyDiary => "Airy blue windows, pink highlights and soft cloud colors.",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.id() == id)
    }

    pub const fn accent(self) -> Color32 {
        match self {
            Self::CrtDark => Color32::from_rgb(100, 224, 154),
            Self::PaperLight => Color32::from_rgb(44, 102, 184),
            Self::LunaBlue => Color32::from_rgb(49, 106, 197),
            Self::ClassicPlatinum => Color32::from_rgb(92, 87, 153),
            Self::SkyDiary => Color32::from_rgb(239, 116, 166),
        }
    }

    pub fn apply(self, ctx: &egui::Context) {
        let mut visuals = match self {
            Self::CrtDark => egui::Visuals::dark(),
            _ => egui::Visuals::light(),
        };

        let (panel, window, faint, extreme, inactive, hovered, active, text, border, hyperlink) =
            match self {
                Self::CrtDark => (
                    rgb(22, 25, 29),
                    rgb(29, 33, 38),
                    rgb(38, 44, 49),
                    rgb(12, 14, 17),
                    rgb(45, 51, 56),
                    rgb(57, 70, 65),
                    rgb(45, 92, 67),
                    rgb(224, 232, 228),
                    rgb(76, 88, 84),
                    rgb(112, 218, 167),
                ),
                Self::PaperLight => (
                    rgb(238, 241, 245),
                    rgb(250, 251, 253),
                    rgb(226, 231, 238),
                    rgb(255, 255, 255),
                    rgb(225, 231, 239),
                    rgb(210, 224, 242),
                    rgb(178, 205, 238),
                    rgb(31, 39, 51),
                    rgb(162, 174, 190),
                    rgb(35, 91, 168),
                ),
                Self::LunaBlue => (
                    rgb(230, 235, 217),
                    rgb(248, 246, 232),
                    rgb(217, 224, 196),
                    rgb(255, 255, 248),
                    rgb(221, 226, 201),
                    rgb(198, 216, 241),
                    rgb(136, 176, 229),
                    rgb(27, 42, 69),
                    rgb(62, 102, 166),
                    rgb(24, 75, 160),
                ),
                Self::ClassicPlatinum => (
                    rgb(215, 216, 221),
                    rgb(237, 237, 240),
                    rgb(199, 201, 208),
                    rgb(252, 252, 252),
                    rgb(205, 207, 213),
                    rgb(189, 194, 211),
                    rgb(157, 164, 193),
                    rgb(25, 25, 31),
                    rgb(119, 121, 133),
                    rgb(73, 67, 139),
                ),
                Self::SkyDiary => (
                    rgb(218, 239, 252),
                    rgb(247, 251, 255),
                    rgb(205, 229, 248),
                    rgb(255, 255, 255),
                    rgb(225, 240, 252),
                    rgb(255, 218, 234),
                    rgb(244, 170, 204),
                    rgb(38, 60, 91),
                    rgb(105, 157, 207),
                    rgb(50, 119, 186),
                ),
            };

        visuals.panel_fill = panel;
        visuals.window_fill = window;
        visuals.faint_bg_color = faint;
        visuals.extreme_bg_color = extreme;
        visuals.code_bg_color = faint;
        visuals.override_text_color = Some(text);
        visuals.hyperlink_color = hyperlink;
        visuals.selection.bg_fill = self.accent();
        visuals.selection.stroke = Stroke::new(1., extreme);
        visuals.window_stroke = Stroke::new(1., border);
        visuals.widgets.noninteractive.bg_fill = panel;
        visuals.widgets.noninteractive.weak_bg_fill = faint;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1., border);
        visuals.widgets.noninteractive.fg_stroke.color = text;
        visuals.widgets.inactive.bg_fill = inactive;
        visuals.widgets.inactive.weak_bg_fill = inactive;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1., border);
        visuals.widgets.inactive.fg_stroke.color = text;
        visuals.widgets.hovered.bg_fill = hovered;
        visuals.widgets.hovered.weak_bg_fill = hovered;
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.5, self.accent());
        visuals.widgets.hovered.fg_stroke.color = text;
        visuals.widgets.active.bg_fill = active;
        visuals.widgets.active.weak_bg_fill = active;
        visuals.widgets.active.bg_stroke = Stroke::new(1.5, self.accent());
        visuals.widgets.active.fg_stroke.color = text;
        visuals.widgets.open.bg_fill = active;
        visuals.widgets.open.weak_bg_fill = active;
        visuals.widgets.open.bg_stroke = Stroke::new(1., self.accent());
        visuals.widgets.open.fg_stroke.color = text;

        let mut style = (*ctx.style()).clone();
        style.visuals = visuals;
        style.spacing.item_spacing = egui::vec2(8., 8.);
        style.spacing.button_padding = egui::vec2(9., 5.);
        ctx.set_style(style);
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_are_unique_and_round_trip() {
        let ctx = egui::Context::default();
        for (index, theme) in Theme::ALL.into_iter().enumerate() {
            assert_eq!(Theme::from_id(theme.id()), Some(theme));
            assert!(!theme.name().is_empty());
            assert!(!theme.description().is_empty());
            assert!(!Theme::ALL[..index]
                .iter()
                .any(|other| other.id() == theme.id()));
            theme.apply(&ctx);
            let style = ctx.style();
            assert_eq!(style.visuals.selection.bg_fill, theme.accent());
            assert_eq!(style.spacing.item_spacing, egui::vec2(8., 8.));
        }
        assert_eq!(Theme::from_id("unknown"), None);
    }
}
