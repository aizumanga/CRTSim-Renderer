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
                    if ui
                        .add(
                            egui::Button::new("Open media…")
                                .shortcut_text(shortcut(ctx, SHORTCUT_OPEN)),
                        )
                        .clicked()
                    {
                        self.dialog(Dialog::File, ctx);
                        ui.close();
                    }
                    if ui.button("Test card").clicked() {
                        self.show_test_card();
                        ui.close();
                    }
                    if ui
                        .button("Video test card")
                        .on_hover_text(
                            "Ten seconds of original pixel-art gameplay, to see these settings \
                             on moving pixels.",
                        )
                        .clicked()
                    {
                        self.show_video_test_card(0);
                        ui.close();
                    }
                    ui.separator();
                    self.project_menu(ui, ctx);
                });
                ui.menu_button("Presets", |ui| {
                    if ui.button("Preset gallery…").clicked() {
                        self.refresh_gallery();
                        self.presets.open = true;
                        ui.close();
                    }
                    for (label, kind) in [
                        ("Load preset…", Dialog::LoadPreset),
                        ("Save preset…", Dialog::SavePreset),
                        ("Import from image / video…", Dialog::ImportPreset),
                    ] {
                        if ui.button(label).clicked() {
                            self.dialog(kind, ctx);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui
                        .button("Copy link to this look")
                        .on_hover_text(
                            "A link that opens these settings in the web app, to share them.                              The link holds the whole look.",
                        )
                        .clicked()
                    {
                        self.copy_look_link(ctx);
                        ui.close();
                    }
                    if ui.button("Open look from link…").clicked() {
                        self.look_link = Some(String::new());
                        ui.close();
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
                        ui.close();
                    }
                });
                ui.menu_button("Export", |ui| {
                    let png = if self.source.timeline.is_some() {
                        "Current frame as PNG…"
                    } else {
                        "Image as PNG…"
                    };
                    if ui
                        .add(egui::Button::new(png).shortcut_text(shortcut(ctx, SHORTCUT_EXPORT)))
                        .clicked()
                    {
                        self.dialog(Dialog::Export, ctx);
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.source.timeline.is_some(), egui::Button::new("Video…"))
                        .clicked()
                    {
                        self.open_video_export(false);
                        ui.close();
                    }
                    if ui
                        .button("Contact sheet…")
                        .on_hover_text(
                            "This picture through many looks at once, each named, in one PNG.",
                        )
                        .clicked()
                    {
                        self.show_contact_sheet();
                        ui.close();
                    }
                    if ui
                        .button("RetroArch shader…")
                        .on_hover_text(
                            "These settings as a shader preset for RetroArch, drawn over a \
                             game as it plays.",
                        )
                        .clicked()
                    {
                        self.dialog(Dialog::RetroArch, ctx);
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Batch queue…").clicked() {
                        self.show_queue = true;
                        ui.close();
                    }
                    // A browser page uses its own codecs, not FFmpeg.
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button("FFmpeg setup…").clicked() {
                        self.show_ffmpeg_setup();
                        ui.close();
                    }
                });
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Credits & support…").clicked() {
                    self.show_credits = true;
                    ui.close();
                }
                // A browser page is the latest release already.
                #[cfg(not(target_arch = "wasm32"))]
                if ui.button("Check for updates…").clicked() {
                    self.show_updates(ctx);
                    ui.close();
                }
            });
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(release) = self.updates.newer() {
                let tag = release.tag.clone();
                if ui
                    .button(format!("⬆ {tag} is out"))
                    .on_hover_text("A newer release is out. See what changed and where to get it.")
                    .clicked()
                {
                    self.updates.open = true;
                }
            }
        });
    }
}
