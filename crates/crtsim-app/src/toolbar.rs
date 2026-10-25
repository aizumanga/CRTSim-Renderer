//! The toolbar along the top of the window, and its menus.
use crate::*;
use crtsim_core::{color::Space, input::Profiles};
use crtsim_shader_ports::Host;

impl App {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            chrome::monitor(ui);
            // A phone's screen has room for the menus or the name, not both.
            if !chrome::narrow(ctx) {
                ui.label(egui::RichText::new("CRTSim Renderer").strong().size(17.));
            }
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
                            "A link that opens these settings in the web app, to share them. \
                             The link holds the whole look.",
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
                                self.notices.failed(
                                    Topic::AppData,
                                    format!("Could not remember the selected theme: {e:#}"),
                                );
                            }
                        }
                        ui.close();
                    }
                    ui.separator();
                    self.interface_menu(ui, ctx);
                    ui.separator();
                    ui.label("Pictures' colour profiles");
                    let mut profiles = self.colors.profiles;
                    ui.selectable_value(&mut profiles, Profiles::Convert, "Convert to sRGB")
                        .on_hover_text(
                            "A picture whose file carries a colour profile, such as Display P3 \
                             or Adobe RGB, has its colours converted into sRGB, the CRT's own.",
                        );
                    ui.selectable_value(&mut profiles, Profiles::Ignore, "Ignore")
                        .on_hover_text(
                            "Every picture's numbers are read as sRGB, whatever profile it \
                             carries: for art drawn in exact palette values, such as the NES \
                             palette, that a program tagged with a profile of its own.",
                        );
                    if profiles != self.colors.profiles {
                        self.set_colors(worker::Colors {
                            profiles,
                            ..self.colors.clone()
                        });
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
                    ui.menu_button("PNG colour space", |ui| {
                        let mut space = self.colors.space.clone();
                        for (_, named) in Space::NAMED {
                            let name = named.name();
                            ui.selectable_value(&mut space, named, name);
                        }
                        if let Space::Icc(profile) = &self.colors.space {
                            let name = profile.name.clone();
                            ui.selectable_value(&mut space, self.colors.space.clone(), name);
                        }
                        if space != self.colors.space {
                            self.set_colors(worker::Colors {
                                space,
                                ..self.colors.clone()
                            });
                            ui.close();
                        }
                        ui.separator();
                        if ui
                            .button("ICC profile…")
                            .on_hover_text(
                                "An RGB profile from a file, such as a display's or a print \
                                 service's, to write PNGs in.",
                            )
                            .clicked()
                        {
                            self.dialog(Dialog::ColorProfile, ctx);
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text(
                        "The colour space PNG stills and contact sheets are written in. Other \
                         than sRGB, a PNG is written at 16 bits with the space's profile \
                         embedded. Videos and animations are sRGB.",
                    );
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
                    for (host, hover) in [
                        (
                            Host::RetroArch,
                            "These settings as a shader preset for RetroArch, drawn over an \
                             emulated game as it plays.",
                        ),
                        (
                            Host::ReShade,
                            "These settings as a ReShade preset, drawn over almost any PC game \
                             as it plays.",
                        ),
                        (
                            Host::Obs,
                            "These settings as a look for the CRTSim Renderer filter in OBS \
                             Studio, drawn over a source as it streams or records.",
                        ),
                        (
                            Host::Godot,
                            "These settings as a CRTSim effect for Godot 4, drawn over a \
                             camera's picture as the game runs.",
                        ),
                    ] {
                        let label = format!("{}…", host.port_name());
                        if ui.button(label).on_hover_text(hover).clicked() {
                            self.dialog(Dialog::Port(host), ctx);
                            ui.close();
                        }
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

impl App {
    /// The View menu's choice of which sounds the interface makes, and how loud.
    fn interface_menu(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut interface = self.interface;
        let sounds = &mut interface.sounds;
        ui.checkbox(&mut sounds.on, "Interface sounds")
            .on_hover_text(
                "Short clicks and tones as you use the controls, which the app makes itself. \
                 The preview and exports stay as they are.",
            );
        ui.add_enabled_ui(sounds.on, |ui| {
            widgets::Keyed::new(0.0..=1.)
                .reset_to(interface::Sounds::default().volume)
                .show(ui, &mut sounds.volume, |slider| slider.text("Volume"));
            for group in sound::Group::ALL {
                ui.checkbox(sounds.group_mut(group), group.name());
            }
        });
        if let Some(why) = self.speaker.failure().filter(|_| sounds.on) {
            ui.colored_label(ui.visuals().warn_fg_color, format!("No sound: {why}"));
        }
        self.set_interface(interface, ctx);
    }

    /// Makes `interface` how the interface behaves, if it changed. Sounds turned back on try the
    /// audio output again, which may have come back since it failed.
    pub(crate) fn set_interface(&mut self, interface: interface::Interface, ctx: &egui::Context) {
        if interface == self.interface {
            return;
        }
        if interface.sounds.on && !self.interface.sounds.on {
            self.speaker.retry();
        }
        self.interface = interface;
        interface.apply(ctx);
    }

    /// Remembers the interface's effects and sounds once they have changed and nothing is held,
    /// so dragging the volume writes it once, when let go.
    pub(crate) fn save_interface(&mut self, ctx: &egui::Context) {
        if self.interface == self.interface_saved || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.interface_saved = self.interface;
        let Some(saved) = self
            .app_data()
            .map(|store| store.set_interface(self.interface))
        else {
            return;
        };
        if let Err(e) = saved {
            self.notices.failed(
                Topic::AppData,
                format!("Could not remember the interface's effects and sounds: {e:#}"),
            );
        }
    }

    /// Uses `colors` from now on, and remembers them. A picture opened from a file opens again
    /// when what its colour profile does has changed, so the change shows.
    pub(crate) fn set_colors(&mut self, colors: worker::Colors) {
        let reread = colors.profiles != self.colors.profiles;
        self.colors = colors;
        self.source.profiles = self.colors.profiles;
        if let Some(store) = &self.store {
            if let Err(e) = store.set_colors(&self.colors) {
                self.notices.failed(
                    Topic::AppData,
                    format!("Could not remember the colour settings: {e:#}"),
                );
            }
        }
        self.status = format!("PNG stills are written in {}", self.colors.space.name());
        if reread {
            match self.source.kind() {
                source::Kind::File(path) if self.source.timeline.is_none() => {
                    self.load(path.clone())
                }
                _ => self.status = "Pictures opened from now on use this".into(),
            }
        }
    }

    /// Writes PNG stills in the RGB profile in `file`, or says why it cannot.
    pub(crate) fn choose_png_profile(&mut self, file: &incoming::File) {
        match file
            .read(16 * 1024 * 1024, "The profile")
            .and_then(|bytes| Space::from_icc(bytes.into_owned()))
        {
            Ok(space) => self.set_colors(worker::Colors {
                space,
                ..self.colors.clone()
            }),
            Err(e) => self
                .notices
                .failed(Topic::Opening, format!("Cannot use the profile: {e:#}")),
        }
    }
}
