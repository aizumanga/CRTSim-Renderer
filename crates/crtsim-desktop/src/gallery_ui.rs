//! The preset gallery and credits windows.
use crate::{chrome, gallery, model, thumbnails, App, Dialog};
use crtsim_core::config::Config;
use eframe::egui::{self, TextureHandle};

const DISCLAIMER: &str = "This project is what some would call \"vibe-coded slop\", built based on J. Kyle Pittman's public CRTSim. The original CRT simulation, shaders, textures and meshes are his work; this project's AI-assisted renderer port and interface are separate additions. This is an unofficial project, not made or endorsed by him.";
const SUPPORT: &str = "Please support J. Kyle Pittman and Minor Key Games: buy and play their games on itch.io and Steam.";
const ITCH: &str = "https://piratehearts.itch.io/";
const STEAM: &str = "https://store.steampowered.com/developer/MinorKeyGames";
const ARTICLE: &str =
    "https://www.gamedeveloper.com/programming/crt-simulation-in-super-win-the-game";

impl App {
    pub(crate) fn gallery_window(&mut self, ctx: &egui::Context) {
        if !self.show_gallery || self.show_welcome {
            return;
        }
        let mut selected = None;
        let mut hovered = None;
        let mut edit = None;
        let wanted: Vec<(String, Config)> = self
            .gallery_entries
            .iter()
            .map(|e| (e.name.clone(), e.config.clone()))
            .collect();
        let pictures: std::collections::HashMap<String, TextureHandle> = wanted
            .into_iter()
            .filter_map(|(name, config)| {
                let picture = self.preset_thumbnail(&name, &config)?;
                Some((name, picture))
            })
            .collect();
        let mut window = self.window_state("Preset gallery");
        let open = chrome::tool_window(ctx, "Preset gallery", [600., 500.], &mut window, |ui| {
            ui.add_enabled_ui(!self.dialog_open, |ui| {
                ui.label(
                    "Save your current settings here to find them again after restarting the \
                     app.",
                );
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut self.gallery_name);
                    if ui
                        .add_enabled(self.store.is_some(), egui::Button::new("Save current"))
                        .clicked()
                    {
                        self.save_to_gallery();
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Load JSON…").clicked() {
                        self.dialog(Dialog::LoadPreset, ctx);
                    }
                    if ui.button("Refresh gallery").clicked() {
                        self.refresh_gallery();
                    }
                });
                ui.small(
                    "Load an existing JSON, then choose Save current to add it to My presets. \
                     Existing names are never overwritten.",
                );
                match &self.store {
                    Some(store) => {
                        ui.small(format!(
                            "Personal presets: {}",
                            store.root.join("presets").display()
                        ));
                    }
                    None => {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "Personal storage is unavailable. JSON import/export and built-in \
                             presets still work.",
                        );
                    }
                }
                if let Some(error) = &self.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if !self.gallery_warnings.is_empty() {
                    egui::CollapsingHeader::new("Skipped preset files").show(ui, |ui| {
                        for warning in &self.gallery_warnings {
                            ui.label(warning);
                        }
                    });
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height().max(200.))
                    .show(ui, |ui| {
                        for user in [false, true] {
                            ui.heading(if user {
                                "My presets"
                            } else {
                                "Included presets"
                            });
                            let mut count = 0;
                            for entry in self.gallery_entries.iter().filter(|e| e.user == user) {
                                count += 1;
                                let response = preset_entry(
                                    ui,
                                    entry,
                                    pictures.get(&entry.name),
                                    &self.config,
                                );
                                if response.selected {
                                    selected = Some(entry.config.clone());
                                }
                                if response.hovered {
                                    hovered = Some((
                                        format!("preset “{}”", entry.name),
                                        entry.config.clone(),
                                    ));
                                }
                                if response.edit {
                                    edit = Some((entry.name.clone(), entry.description.clone()));
                                }
                            }
                            if count == 0 {
                                ui.label(
                                    "No personal presets yet. Adjust an image and save your \
                                     first look above.",
                                );
                            }
                        }
                    });
            });
            if let Some(edit) = edit.take() {
                self.description_edit = Some(edit);
            }
            // Shown inside the gallery's own window, next to the preset being edited.
            self.description_window(ui.ctx());
        });
        self.store_window_state("Preset gallery", window);
        self.show_gallery = open;
        if let Some((label, config)) = hovered.filter(|_| open) {
            self.offer_audition(label, config);
        }
        if let Some(config) = selected {
            self.replace_config(config);
        }
    }
    /// Reads the personal presets again, which may have changed on disk.
    pub(crate) fn refresh_gallery(&mut self) {
        (self.gallery_entries, self.gallery_warnings) = gallery::entries(self.store.as_ref());
    }
    fn save_to_gallery(&mut self) {
        let Some(store) = &self.store else {
            return;
        };
        match store.save(&self.gallery_name, &self.config, self.input.dimensions()) {
            Ok(()) => {
                self.status = format!("Saved '{}' to My presets", self.gallery_name);
                self.error = None;
                self.refresh_gallery();
            }
            Err(e) => self.error = Some(format!("Cannot save gallery preset: {e:#}")),
        }
    }
    fn description_window(&mut self, ctx: &egui::Context) {
        if let Some((name, mut description)) = self.description_edit.clone() {
            let mut editing = true;
            let mut save = false;
            egui::Window::new(format!("Description — {name}"))
                .open(&mut editing)
                .collapsible(false)
                .default_width(400.)
                .constrain_to(ctx.screen_rect())
                .show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut description)
                            .desired_rows(4)
                            .desired_width(f32::INFINITY),
                    );
                    ui.small("Up to 4096 bytes. Leave blank to remove the description.");
                    save = ui.button("Save description").clicked();
                    if let Some(error) = &self.error {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                    }
                });
            if save {
                let result = self
                    .store
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("Personal storage unavailable"))
                    .and_then(|store| store.set_description(&name, &description));
                match result {
                    Ok(()) => {
                        self.refresh_gallery();
                        self.error = None;
                        editing = false;
                    }
                    Err(e) => self.error = Some(format!("Cannot save description: {e:#}")),
                }
            }
            self.description_edit = editing.then_some((name, description));
        }
    }
    fn credit_text(ui: &mut egui::Ui) {
        ui.label(DISCLAIMER);
        ui.separator();
        ui.strong("Original CRTSim: J. Kyle Pittman");
        ui.label(
            "The CRT simulation and original shaders, textures and \
            screen/frame meshes are provided under CC0. Thank you for \
            sharing them publicly.",
        );
        ui.hyperlink_to(
            "Original CRTSim source",
            "https://github.com/MinorKeyGames/CRTSim",
        );
        ui.separator();
        ui.label(SUPPORT);
        ui.horizontal(|ui| {
            ui.hyperlink_to("J. Kyle Pittman on itch.io", ITCH);
            ui.hyperlink_to("Minor Key Games on Steam", STEAM);
        });
        ui.hyperlink_to("Read: CRT Simulation in Super Win the Game", ARTICLE);
        ui.separator();
        ui.strong("NES LUT collection: Wellington Uemura (wtuemura)");
        ui.label(
            "Shared through MAME Goodies under CC0 1.0. Includes palettes \
            by FirebrandX (FBX) and other creators identified in the \
            original palette names. Thanks to the MAME Goodies \
            contributors.",
        );
        ui.horizontal_wrapped(|ui| {
            ui.hyperlink_to(
                "NES LUTs & license",
                "https://github.com/mamedev/mame-goodies/tree/master/bgfx/lut/nes",
            );
            ui.hyperlink_to(
                "FirebrandX palettes",
                "https://www.firebrandx.com/nespalette.html",
            );
            ui.hyperlink_to(
                "Author's announcement",
                "https://www.reddit.com/r/emulation/comments/1oopf1i/updated_nes_luts_for_mame/",
            );
        });
        ui.separator();
        ui.small(
            "Renderer port and interface: CRTSim-Renderer contributors, \
            with AI assistance. Built with Rust, wgpu, egui/eframe, \
            image and other open-source libraries; see \
            THIRD_PARTY_NOTICES.md in the repository.",
        );
    }
    pub(crate) fn credits_window(&mut self, ctx: &egui::Context) {
        if self.show_welcome {
            egui::Window::new("Welcome to CRTSim Renderer")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .default_width(560.)
                .show(ctx, |ui| {
                    Self::credit_text(ui);
                    ui.add_space(12.);
                    if ui.button("Got it — continue").clicked() {
                        if let Some(ref store) = self.store {
                            if let Err(e) = store.acknowledge() {
                                self.error = Some(format!(
                                    "Could not remember the welcome message: {e:#}. It may appear \
                                next time."
                                ));
                            }
                        }
                        self.show_welcome = false;
                    }
                });
        } else if self.show_credits {
            let mut window = self.window_state("Credits & support");
            // A native window clips instead of growing, so the long credits get a scroll area.
            let open =
                chrome::tool_window(ctx, "Credits & support", [560., 620.], &mut window, |ui| {
                    egui::ScrollArea::vertical().show(ui, Self::credit_text);
                });
            self.store_window_state("Credits & support", window);
            self.show_credits = open;
        }
    }
}

/// What happened to one preset in the gallery this frame.
#[derive(Default)]
pub(crate) struct EntryResponse {
    pub(crate) selected: bool,
    pub(crate) hovered: bool,
    pub(crate) edit: bool,
}

/// One preset in the gallery: its thumbnail, name and description, and how it differs from the
/// settings in use.
fn preset_entry(
    ui: &mut egui::Ui,
    entry: &gallery::Entry,
    picture: Option<&TextureHandle>,
    current: &Config,
) -> EntryResponse {
    let mut response = EntryResponse::default();
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            let picture = thumbnails::show(ui, picture, 72., 16. / 9.)
                .on_hover_text("Point to preview this preset on your image; click to apply it");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let label = ui.selectable_label(*current == entry.config, &entry.name);
                    response.selected = label.clicked() || picture.clicked();
                    response.hovered = (label.hovered() || picture.hovered()) && ui.is_enabled();
                    response.edit = entry.user && ui.small_button("Edit description").clicked();
                });
                let description = if entry.description.is_empty() {
                    "No description"
                } else {
                    &entry.description
                };
                ui.add(egui::Label::new(description).wrap(true));
                preset_differences(ui, entry, current);
            });
        });
    });
    response
}

/// How a preset differs from the settings in use, as a table that opens on request.
fn preset_differences(ui: &mut egui::Ui, entry: &gallery::Entry, current: &Config) {
    if entry.config == *current {
        ui.small("Matches your settings");
        return;
    }
    let changes = model::differences(current, &entry.config);
    let noun = if changes.len() == 1 {
        "setting"
    } else {
        "settings"
    };
    egui::CollapsingHeader::new(format!(
        "Differs from your settings in {} {noun}",
        changes.len()
    ))
    .id_source(("preset differences", &entry.name, entry.user))
    .show(ui, |ui| {
        egui::Grid::new(("preset difference grid", &entry.name, entry.user))
            .striped(true)
            .show(ui, |ui| {
                ui.strong("Setting");
                ui.strong("Yours");
                ui.strong("Preset");
                ui.end_row();
                for change in &changes {
                    ui.label(&change.setting);
                    ui.label(&change.from);
                    ui.label(&change.to);
                    ui.end_row();
                }
            });
    });
}
