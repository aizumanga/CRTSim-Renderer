//! The contact sheet window: which looks to render the picture on screen through, at what
//! size, and in how many columns, saved as one labelled PNG (`crtsim_core::sheet`).
use crate::widgets::choice;
use crate::*;
use std::collections::BTreeSet;

/// The tile sizes offered, each a 4:3 picture.
const TILES: [((u32, u32), &str); 3] = [
    ((320, 240), "Small (320 × 240)"),
    ((480, 360), "Medium (480 × 360)"),
    ((640, 480), "Large (640 × 480)"),
];

/// What the next contact sheet holds.
pub struct ContactSheet {
    pub open: bool,
    /// The gallery's presets left out, by name: every preset is in until unticked.
    left_out: BTreeSet<String>,
    /// Whether the settings in use are a tile of their own, first.
    current: bool,
    tile: (u32, u32),
    columns: u32,
}

impl Default for ContactSheet {
    fn default() -> Self {
        Self {
            open: false,
            left_out: BTreeSet::new(),
            current: true,
            tile: TILES[1].0,
            columns: 4,
        }
    }
}

impl ContactSheet {
    /// The looks ticked, the settings in use first, named as the gallery names them.
    fn looks(&self, gallery: &[gallery::Entry], current: &Config) -> Vec<crtsim_core::sheet::Look> {
        let current = self.current.then(|| crtsim_core::sheet::Look {
            name: "Settings in use".into(),
            config: current.clone(),
        });
        current
            .into_iter()
            .chain(
                gallery
                    .iter()
                    .filter(|entry| !self.left_out.contains(&entry.name))
                    .map(|entry| crtsim_core::sheet::Look {
                        name: entry.name.clone(),
                        config: entry.config.clone(),
                    }),
            )
            .collect()
    }
}

impl App {
    pub(crate) fn show_contact_sheet(&mut self) {
        self.refresh_gallery();
        self.sheet.open = true;
    }

    pub(crate) fn contact_sheet_window(&mut self, ctx: &egui::Context) {
        if !self.sheet.open || self.show_welcome {
            return;
        }
        let mut save = false;
        let can_save = self.can_start_work();
        let mut window = self.window_state("Contact sheet");
        let sheet = &mut self.sheet;
        let entries = &self.presets.entries;
        let open = chrome::tool_window(ctx, "Contact sheet", [420., 560.], &mut window, |ui| {
            ui.label(
                "The picture on screen through each look ticked, as settled stills in a grid, \
                 each named beneath it. Saved as one PNG.",
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.small_button("All").clicked() {
                    sheet.left_out.clear();
                    sheet.current = true;
                }
                if ui.small_button("None").clicked() {
                    sheet.left_out = entries.iter().map(|e| e.name.clone()).collect();
                    sheet.current = false;
                }
            });
            egui::ScrollArea::vertical()
                .max_height(280.)
                .show(ui, |ui| {
                    ui.checkbox(&mut sheet.current, "Settings in use");
                    for entry in entries {
                        let mut ticked = !sheet.left_out.contains(&entry.name);
                        let label = if entry.user {
                            format!("{} (mine)", entry.name)
                        } else {
                            entry.name.clone()
                        };
                        if ui
                            .checkbox(&mut ticked, label)
                            .on_hover_text(&entry.description)
                            .changed()
                        {
                            if ticked {
                                sheet.left_out.remove(&entry.name);
                            } else {
                                sheet.left_out.insert(entry.name.clone());
                            }
                        }
                    }
                });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                choice(ui, "Tile size", &mut sheet.tile, &TILES);
            });
            ui.add(egui::Slider::new(&mut sheet.columns, 1..=8).text("Columns"));
            let count = usize::from(sheet.current)
                + entries
                    .iter()
                    .filter(|e| !sheet.left_out.contains(&e.name))
                    .count();
            ui.separator();
            ui.add_enabled_ui(can_save && count > 0, |ui| {
                save = ui
                    .button(format!("Save contact sheet of {count} looks…"))
                    .clicked();
            });
        });
        self.store_window_state("Contact sheet", window);
        self.sheet.open = open;
        if save {
            self.dialog(Dialog::ContactSheet, ctx);
        }
    }

    /// Renders the contact sheet the window describes, saving it to `path`.
    pub(crate) fn export_contact_sheet(&mut self, path: PathBuf) {
        self.stop_playback();
        let looks = self.sheet.looks(&self.presets.entries, &self.config);
        let input = self.source.input().clone();
        if let Some(e) = looks.iter().find_map(|look| {
            look.config
                .validate_for(input.dimensions())
                .err()
                .map(|e| (look, e))
        }) {
            self.notices.failed(
                Topic::Saving,
                format!("Cannot render {}: {:#}", e.0.name, e.1),
            );
            return;
        }
        let count = looks.len();
        let export = worker::Export::Sheet {
            input,
            looks,
            tile: self.sheet.tile,
            columns: self.sheet.columns,
        };
        self.start_export(
            export,
            path,
            format!("Rendering a contact sheet of {count} looks…"),
            Some("Queued for contact sheet"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_in_use_come_first_then_the_presets_ticked() {
        let gallery = gallery::builtins();
        let mut sheet = ContactSheet::default();
        let mut current = Config::general();
        current.bloom = 0.;
        let looks = sheet.looks(&gallery, &current);
        assert_eq!(looks.len(), gallery.len() + 1);
        assert_eq!(looks[0].name, "Settings in use");
        assert_eq!(looks[0].config, current);
        sheet.current = false;
        sheet.left_out.insert(gallery[0].name.clone());
        let looks = sheet.looks(&gallery, &current);
        assert_eq!(looks.len(), gallery.len() - 1);
        assert_eq!(looks[0].name, gallery[1].name);
    }
}
