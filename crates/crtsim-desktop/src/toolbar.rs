//! The toolbar along the top of the window, and its menus.
use crate::*;

impl App {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            chrome::monitor(ui);
            ui.label(egui::RichText::new("CRTSim Renderer").strong().size(17.));
            ui.separator();
            ui.add_enabled_ui(self.can_start_work(), |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open media…").clicked() {
                        self.dialog(Dialog::File, ctx);
                        ui.close_menu();
                    }
                    if ui.button("Test card").clicked() {
                        self.show_test_card();
                        ui.close_menu();
                    }
                    ui.separator();
                    self.project_menu(ui, ctx);
                });
                ui.menu_button("Presets", |ui| {
                    if ui.button("Preset gallery…").clicked() {
                        self.refresh_gallery();
                        self.show_gallery = true;
                        ui.close_menu();
                    }
                    for (label, kind) in [
                        ("Load preset…", Dialog::LoadPreset),
                        ("Save preset…", Dialog::SavePreset),
                        ("Import from image / video…", Dialog::ImportPreset),
                    ] {
                        if ui.button(label).clicked() {
                            self.dialog(kind, ctx);
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.label("Interface theme");
                    let mut selected = self.theme;
                    for theme in theme::Theme::ALL {
                        ui.selectable_value(&mut selected, theme, theme.name())
                            .on_hover_text(theme.description());
                    }
                    if selected != self.theme {
                        self.theme = selected;
                        self.theme.apply(ctx);
                        if let Some(store) = &self.store {
                            if let Err(e) = store.set_theme(self.theme) {
                                self.error =
                                    Some(format!("Could not remember the selected theme: {e:#}"));
                            }
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("Export", |ui| {
                    if ui
                        .button(if self.video.is_some() {
                            "Current frame as PNG…"
                        } else {
                            "Image as PNG…"
                        })
                        .clicked()
                    {
                        self.dialog(Dialog::Export, ctx);
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(self.video.is_some(), egui::Button::new("Video…"))
                        .clicked()
                    {
                        self.open_video_export(false);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Batch queue…").clicked() {
                        self.show_queue = true;
                        ui.close_menu();
                    }
                });
            });
            if ui.button("Credits").clicked() {
                self.show_credits = true;
            }
        });
    }
}
