use crate::*;
use crtsim_core::nes_luts;

impl App {
    pub(crate) fn lut_gallery_window(&mut self, ctx: &egui::Context) {
        if !self.show_lut_gallery || self.show_welcome {
            return;
        }
        let mut selected = None;
        let mut hovered = None;
        let mut hovered_none = false;
        let mut remove = false;
        let mut window = self.window_state("LUT gallery");
        let open = crate::chrome::tool_window(
            ctx,
            "LUT gallery",
            [560., 500.],
            &mut window,
            |ui| {
                ui.label("Included NES color LUTs");
                ui.small("Designed for MAME's NES palette. Other images may look different from the named palette. Selecting a LUT changes only color mapping.");
                ui.horizontal_wrapped(|ui| {
                    ui.label("Search");
                    ui.text_edit_singleline(&mut self.lut_gallery_search);
                    if ui.small_button("Clear").clicked() {
                        self.lut_gallery_search.clear();
                    }
                });
                let query = self.lut_gallery_search.trim().to_lowercase();
                let count = nes_luts::ENTRIES
                    .iter()
                    .filter(|e| e.name.to_lowercase().contains(&query))
                    .count();
                ui.small(format!(
                    "{count} of {} included LUTs · available offline",
                    nes_luts::ENTRIES.len()
                ));
                ui.separator();
                ui.add_enabled_ui(
                    !self.dialog_open
                        && !self.loading
                        && !self.exporting
                        && self.workflow.export_dialog.is_none(),
                    |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(format!(
                                "Current: {}",
                                self.config
                                    .lut
                                    .as_ref()
                                    .map_or("No LUT", |lut| lut.name.as_str())
                            ));
                            let button = ui.add_enabled(
                                self.config.lut.is_some(),
                                egui::Button::new("Remove LUT"),
                            );
                            remove = button.clicked();
                            hovered_none = button.hovered() && self.config.lut.is_some();
                        });
                        egui::ScrollArea::vertical()
                            .id_source("nes_lut_gallery")
                            .max_height(330.)
                            .show(ui, |ui| {
                                for (index, entry) in nes_luts::ENTRIES.iter().enumerate() {
                                    if !entry.name.to_lowercase().contains(&query) {
                                        continue;
                                    }
                                    ui.group(|ui| {
                                        ui.set_min_width(ui.available_width());
                                        let active = self
                                            .config
                                            .lut
                                            .as_ref()
                                            .is_some_and(|lut| lut.name == entry.name);
                                        let label = ui
                                            .selectable_label(active, entry.name)
                                            .on_hover_text(
                                                "Point to preview this LUT; click to apply it. CRT and framing settings are kept",
                                            );
                                        if label.clicked() {
                                            selected = Some(index);
                                        }
                                        if label.hovered() && ui.is_enabled() {
                                            hovered = Some(index);
                                        }
                                    });
                                }
                                if count == 0 {
                                    ui.label("No LUTs match your search.");
                                }
                            });
                    },
                );
                ui.separator();
                ui.small(
                    "NES LUT collection by Wellington Uemura (wtuemura) / MAME Goodies · CC0 1.0",
                );
                ui.hyperlink_to(
                    "Collection, credits & license",
                    "https://github.com/mamedev/mame-goodies/tree/master/bgfx/lut/nes",
                );
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            },
        );
        self.store_window_state("LUT gallery", window);
        self.show_lut_gallery = open;
        if open && hovered_none {
            let config = Config {
                lut: None,
                ..self.config.clone()
            };
            self.offer_audition("no LUT", config);
        } else if let Some(index) = hovered.filter(|_| open) {
            // A LUT that cannot be decoded is reported when it is clicked, not while pointed at.
            if let Ok(lut) = self.included_lut(index) {
                let label = format!("LUT “{}”", lut.name);
                let config = Config {
                    lut: Some(lut),
                    ..self.config.clone()
                };
                self.offer_audition(label, config);
            }
        }
        if remove {
            let mut config = self.config.clone();
            config.lut = None;
            self.replace_config(config);
            self.error = None;
            self.status = "LUT removed".into();
        } else if let Some(index) = selected {
            match self.included_lut(index) {
                Ok(lut) => {
                    let name = lut.name.clone();
                    let mut config = self.config.clone();
                    config.lut = Some(lut);
                    if config != self.config {
                        self.replace_config(config);
                    }
                    self.error = None;
                    self.status = format!("Applied {name}");
                }
                Err(error) => self.error = Some(format!("Cannot load included LUT: {error:#}")),
            }
        }
    }
}
