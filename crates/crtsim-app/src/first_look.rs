//! The first look: three included presets offered right after the welcome on a first run, each
//! as a thumbnail of the picture open, so the person starts from a look they chose. The preset
//! line then names it, and Presets… offers the rest.
use crate::chrome::Fitted;
use crate::*;

/// The presets offered, the one the app starts with first, each with a line for someone who
/// has not seen it yet.
const LOOKS: [(&str, &str); 3] = [
    (
        "Super Win the Game",
        "The game's own CRT: a strong mask and composite colour fringes.",
    ),
    (
        "General image",
        "Square pixels and smooth resizing, for photos and screenshots.",
    ),
    (
        "Soft television",
        "A gentler mask and ringing, for illustrations.",
    ),
];
/// How tall each look's thumbnail is shown, in points.
const HEIGHT: f32 = 101.;

impl App {
    /// The first-look window, while it is open. Choosing a look applies it, as a change Undo
    /// reverses, and closes the window; closing it keeps the look in use.
    pub(crate) fn first_look_window(&mut self, ctx: &egui::Context) {
        if !self.first_look || self.show_welcome {
            return;
        }
        let looks: Vec<(gallery::Entry, &str)> = LOOKS
            .iter()
            .filter_map(|&(name, line)| {
                let entry = self
                    .presets
                    .entries
                    .iter()
                    .find(|e| !e.user && e.name == name);
                Some((entry?.clone(), line))
            })
            .collect();
        let pictures: Vec<_> = looks
            .iter()
            .map(|(entry, _)| self.preset_thumbnail(&entry.name, false, &entry.config))
            .collect();
        let narrow = chrome::narrow(ctx);
        let mut chosen = None;
        let mut open = true;
        egui::Window::new("Choose a look to start with")
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .fitted(ctx, 3. * HEIGHT * 16. / 9. + 60.)
            .show(ctx, |ui| {
                ui.label(
                    "This picture through three looks. Choose one to start from; Presets… in \
                     the panel has more, and Open… brings your own picture.",
                );
                ui.add_space(4.);
                // Side by side, or one under another on a phone's screen, each beside its name.
                let layout = if narrow {
                    egui::Layout::top_down(egui::Align::Min)
                } else {
                    egui::Layout::left_to_right(egui::Align::Min)
                };
                ui.with_layout(layout, |ui| {
                    for ((entry, line), picture) in looks.iter().zip(&pictures) {
                        let in_use = self.config == entry.config;
                        if look(ui, narrow, entry, line, picture.as_ref(), in_use) {
                            chosen = Some(entry.config.clone());
                        }
                    }
                });
            });
        if let Some(config) = chosen {
            self.replace_config(config);
            self.first_look = false;
        }
        if !open {
            self.first_look = false;
        }
    }
}

/// One look on offer: its thumbnail, with its name and line beneath it, or beside it when
/// `narrow`. Whether it was chosen.
fn look(
    ui: &mut egui::Ui,
    narrow: bool,
    entry: &gallery::Entry,
    line: &str,
    picture: Option<&egui::TextureHandle>,
    in_use: bool,
) -> bool {
    let width = HEIGHT * 16. / 9.;
    let words = |ui: &mut egui::Ui| {
        let chosen = ui.selectable_label(in_use, &entry.name).clicked();
        ui.small(line);
        chosen
    };
    let layout = if narrow {
        egui::Layout::left_to_right(egui::Align::Min)
    } else {
        egui::Layout::top_down(egui::Align::Min)
    };
    // A row on a phone takes the screen's width, so the words beside the picture have room.
    let room = if narrow { ui.available_width() } else { width };
    ui.allocate_ui_with_layout(egui::vec2(room, 0.), layout, |ui| {
        if !narrow {
            ui.set_width(width);
        }
        let picture = thumbnails::show(ui, picture, HEIGHT, 16. / 9.)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();
        let name = if narrow {
            ui.vertical(words).inner
        } else {
            words(ui)
        };
        picture || name
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_looks_offered_are_included_presets_the_default_first() {
        let included = gallery::builtins();
        for (name, _) in LOOKS {
            assert!(included.iter().any(|e| e.name == name), "{name}");
        }
        assert_eq!(LOOKS[0].0, gallery::default_preset().name);
    }

    #[test]
    fn a_first_runs_welcome_leads_to_the_first_look_and_a_returning_ones_does_not() {
        let ctx = egui::Context::default();
        let store = app_data::Store::temporary();
        let open = || {
            App::new(
                &ctx,
                worker::Gpu::Own(wgpu::Backends::PRIMARY),
                Ok(store.clone()),
                None,
                None,
            )
        };
        let mut first = open();
        assert!(first.show_welcome && !first.first_look);
        first.welcome_read();
        assert!(first.first_look && !store.welcome_needed());
        // A look a link opens is the look chosen.
        first.bring_in(Ok(incoming::Brought {
            config: Config::general(),
            options: None,
            preset_name: None,
            status: "Opened a look from a link".into(),
        }));
        assert!(!first.first_look);
        first.save_session();
        // The welcome shown again, as when it cannot be remembered, to someone whose session
        // waits to be recovered.
        let mut returning = open();
        assert!(returning.session.recovery_offered());
        returning.welcome_read();
        assert!(!returning.first_look);
    }
}
