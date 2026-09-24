//! The settings panel: CRT settings by section, then source framing and color.
use crate::widgets::{numbers, resolution, Keyed};
use crate::*;

impl App {
    pub(crate) fn settings(&mut self, ui: &mut egui::Ui) {
        if let Some(video) = &self.video {
            ui.heading("Video");
            ui.label(format!(
                "{:.2}s · {:.3} FPS · {}",
                video.duration,
                video.fps,
                if video.audio { "with audio" } else { "silent" }
            ));
            ui.small(
                "Preview is silent. Configure exported audio and compression in Export → Video.",
            );
            if video.hdr {
                ui.small("HDR source: FFmpeg tone-maps to SDR.");
            }
            ui.separator();
        }
        ui.horizontal(|ui| {
            chrome::monitor(ui);
            ui.heading("Source");
        });
        egui::Frame::none()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().window_stroke)
            .rounding(3.)
            .inner_margin(9.)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.label(egui::RichText::new(&self.source_name).strong());
            });
        ui.small(format!(
            "Source: {} × {}",
            self.input.width(),
            self.input.height()
        ));
        ui.horizontal(|ui| {
            if ui.button("Undo").on_hover_text("Ctrl+Z").clicked() {
                self.undo();
            }
            if ui.button("Redo").on_hover_text("Ctrl+Shift+Z").clicked() {
                self.redo();
            }
            if ui
                .button("Reset")
                .on_hover_text("Reset to the general image preset; Undo restores your settings")
                .clicked()
            {
                self.replace_config(Config::general());
            }
        });
        ui.horizontal(|ui| {
            if ui.button("General image").clicked() {
                self.replace_config(Config::general());
            }
            if ui.button("Original CRTSim").clicked() {
                self.replace_config(Config::default());
            }
        });
        let before = self.config.clone();
        // What each slider's reset returns to: the same baseline as Reset above.
        let defaults = Config::general();
        chrome::Section::new("Image & output").show(ui, |ui| {
            resolution(
                ui,
                "Signal",
                &mut self.config.signal,
                &[
                    "auto", "native", "original", "240p", "288p", "360p", "480p", "576p",
                ],
            );
            resolution(
                ui,
                "Export size",
                &mut self.config.output,
                &["720p", "1080p", "1440p", "4k", "reference", "match-input"],
            );
            egui::ComboBox::from_label("Fit on 4:3 tube")
                .selected_text(format!("{:?}", self.config.fit))
                .show_ui(ui, |ui| {
                    for (value, name) in [
                        (Fit::Contain, "Contain"),
                        (Fit::Cover, "Cover (crop)"),
                        (Fit::Stretch, "Stretch"),
                        (Fit::Reference, "Reference"),
                    ] {
                        ui.selectable_value(&mut self.config.fit, value, name);
                    }
                });
            egui::ComboBox::from_label("Resize filter")
                .selected_text(format!("{:?}", self.config.filter))
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.filter,
                        Filter::Lanczos,
                        "Lanczos (smooth)",
                    );
                    ui.selectable_value(
                        &mut self.config.filter,
                        Filter::Nearest,
                        "Nearest (pixel art)",
                    );
                });
            numbers(ui, &mut self.config, &defaults, settings::Section::Image);
            if let (Ok(signal), Ok(output)) = (
                self.config.signal_size(self.input.dimensions()),
                self.config.output_size(self.input.dimensions()),
            ) {
                ui.small(format!(
                    "Signal: {} × {} → Output: {} × {}",
                    signal.0, signal.1, output.0, output.1
                ));
                if output.0 as u64 * output.1 as u64 > 8_300_000 {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Large output: more memory and rendering time.",
                    );
                }
            } else {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Choose a preset or enter a valid WIDTHxHEIGHT.",
                );
            }
            if matches!(self.config.fit, Fit::Cover | Fit::Reference) || self.config.overscan > 1. {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Current fit/overscan can crop content and subtitles.",
                );
            }
            ui.small(
                "Rounded glass may hide extreme corners even with Contain. Alpha uses the \
                 selected background. SDR output; no ICC color management.",
            );
        });
        self.workflow_settings(ui);
        chrome::Section::new("Color processing").show(ui, |ui| {
            egui::ComboBox::from_label("Color processing")
                .selected_text(match self.config.color_mode {
                    ColorMode::Reference => "Original gamma",
                    ColorMode::LinearLight => "Linear light (experimental)",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.config.color_mode,
                        ColorMode::Reference,
                        "Original gamma",
                    );
                    ui.selectable_value(
                        &mut self.config.color_mode,
                        ColorMode::LinearLight,
                        "Linear light (experimental)",
                    );
                });
            if self.config.color_mode == ColorMode::LinearLight {
                ui.small(
                    "Linear-light glass, lighting and bloom; SDR output. The analog signal \
                     still uses the original gamma-space model.",
                );
            }
            chrome::Section::new("Optional color grade").show(ui, |ui| {
                numbers(ui, &mut self.config, &defaults, settings::Section::Grade);
                ui.small(
                    "YIQ hue/chroma adjustment. This is an optional grade, not the game's \
                     unpublished NES palette LUT or a complete NTSC decoder.",
                );
            });
            ui.checkbox(
                &mut self.config.mask_antialias,
                "Filter mask when shrinking",
            )
            .on_hover_text(
                "Samples the mask from averaged, smaller copies of itself, as the original \
                 did, so it stays smooth where it is drawn smaller than it is. Off samples only \
                 the full-size mask, which can shimmer into moiré.",
            );
        });
        ui.separator();
        chrome::Section::new("CRT signal")
            .default_open(true)
            .show(ui, |ui| {
                numbers(ui, &mut self.config, &defaults, settings::Section::Signal)
            });
        chrome::Section::new("Glass & mask").show(ui, |ui| {
            numbers(ui, &mut self.config, &defaults, settings::Section::Glass);
            ui.separator();
            self.mask_density(ui);
            numbers(ui, &mut self.config, &defaults, settings::Section::Mask);
        });
        chrome::Section::new("Bloom & reflections").show(ui, |ui| {
            numbers(ui, &mut self.config, &defaults, settings::Section::Bloom)
        });
        chrome::Section::new("Frame & lighting").show(ui, |ui| {
            numbers(ui, &mut self.config, &defaults, settings::Section::Lighting)
        });
        chrome::Section::new("Persistence & artifact phase").show(ui, |ui| {
            numbers(
                ui,
                &mut self.config,
                &defaults,
                settings::Section::Persistence,
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.config.warmup != defaults.warmup,
                        egui::Button::new("↺").small(),
                    )
                    .on_hover_text(format!("Reset Warm-up ticks to {}", defaults.warmup))
                    .clicked()
                {
                    self.config.warmup = defaults.warmup;
                }
                Keyed::new(0..=240).reset_to(defaults.warmup).show(
                    ui,
                    &mut self.config.warmup,
                    |s| s.text("Warm-up ticks"),
                );
            });
            egui::ComboBox::from_label("Phase")
                .selected_text(format!("{:?}", self.config.phase))
                .show_ui(ui, |ui| {
                    for phase in [Phase::Stable, Phase::A, Phase::B, Phase::Alternating] {
                        ui.selectable_value(&mut self.config.phase, phase, format!("{phase:?}"));
                    }
                });
            ui.checkbox(&mut self.config.interlace, "Interlaced fields")
                .on_hover_text(
                    "Each tick scans every other row, alternating fields; the rows it skips \
                     only fade by persistence. Use with a 480- or 576-row signal.",
                );
            ui.small(
                "Each still starts from black. Higher persistence may require more warm-up \
                 ticks. Alternating phase depends on tick count.",
            );
        });
        if self.config != before {
            self.changed();
        }
    }
    /// Whether the mask follows the signal, as the original's did, or has columns and rows of
    /// its own, which start from the density in use so the picture does not jump.
    fn mask_density(&mut self, ui: &mut egui::Ui) {
        let mut follows = self.config.mask_repeats == MaskRepeats::Signal;
        let toggled = ui
            .checkbox(&mut follows, "Mask follows the signal")
            .on_hover_text(
                "A mask column for every two signal columns and a row for every signal row, as \
                 the original drew it, so a finer signal gets a finer mask. Off sets the mask's \
                 columns and rows yourself, whatever the signal.",
            )
            .changed();
        if toggled {
            self.config.mask_repeats = if follows {
                MaskRepeats::Signal
            } else {
                let signal = self.config.signal_size(self.input.dimensions());
                MaskRepeats::Fixed(
                    self.config
                        .mask_repeats
                        .resolve(signal.unwrap_or((256, 224))),
                )
            };
        }
    }
    pub fn workflow_settings(&mut self, ui: &mut egui::Ui) {
        crate::chrome::Section::new("Source & framing")
            .default_open(true)
            .show(ui, |ui| {
                ui.checkbox(&mut self.config.screen_only, "Screen only · no bezel");
                egui::CollapsingHeader::new("Crop edges · percent").show(ui, |ui| {
                    for (i, name) in ["Left", "Top", "Right", "Bottom"].iter().enumerate() {
                        let opposite = (i + 2) % 4;
                        let max = (0.98 - self.config.source.crop[opposite]).max(0.);
                        let mut percent = self.config.source.crop[i] * 100.;
                        if Keyed::new(0.0..=max * 100.)
                            .reset_to(0.)
                            .show(ui, &mut percent, |s| s.text(*name))
                            .changed()
                        {
                            self.config.source.crop[i] = percent / 100.;
                        }
                    }
                });
                let source = &mut self.config.source;
                Keyed::new(-180.0..=180.)
                    .reset_to(0.)
                    .show(ui, &mut source.rotation, |s| s.text("Rotation °"));
                Keyed::new(0.05..=4.).logarithmic(true).reset_to(1.).show(
                    ui,
                    &mut source.zoom,
                    |s| s.text("Source zoom"),
                );
                for (position, label) in source.position.iter_mut().zip(["Pan X", "Pan Y"]) {
                    Keyed::new(-1.0..=1.)
                        .reset_to(0.)
                        .show(ui, position, |s| s.text(label));
                }
                ui.label("Transparency background (included in exports)");
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            !self.config.source.checkerboard
                                && self.config.source.background == [0; 3],
                            "Black",
                        )
                        .clicked()
                    {
                        self.config.source.checkerboard = false;
                        self.config.source.background = [0; 3];
                    }
                    if ui
                        .selectable_label(
                            !self.config.source.checkerboard
                                && self.config.source.background == [255; 3],
                            "White",
                        )
                        .clicked()
                    {
                        self.config.source.checkerboard = false;
                        self.config.source.background = [255; 3];
                    }
                    ui.checkbox(&mut self.config.source.checkerboard, "Checker");
                });
                if ui
                    .color_edit_button_srgb(&mut self.config.source.background)
                    .changed()
                {
                    self.config.source.checkerboard = false;
                }
                if ui.button("Reset source framing").clicked() {
                    self.config.source = Default::default();
                }
            });
        crate::chrome::Section::new("Color & LUT").show(ui, |ui| {
            ui.label(match (&self.config.palette, &self.config.lut) {
                (Some(_), _) => "NES palette from the composite signal",
                (None, Some(lut)) => lut.name.as_str(),
                (None, None) => "No LUT",
            });
            if ui.button("LUT gallery…").clicked() {
                self.stop_playback();
                self.show_lut_gallery = true;
            }
            if ui.button("Import 3D .cube…").clicked() {
                self.dialog(Dialog::Lut, &ui.ctx().clone());
            }
            if self.config.lut.is_some() && ui.button("Remove LUT").clicked() {
                self.config.lut = None;
            }
            let mut generated = self.config.palette.is_some();
            let toggled = ui
                .checkbox(&mut generated, "NES palette from the composite signal")
                .on_hover_text(
                    "Super Win the Game's NTSC palette: the NES's colours decoded from its \
                     signal, turned by Tint and scaled along I and Q. It recolours images drawn \
                     in MAME's NES palette, and replaces any LUT.",
                )
                .changed();
            if toggled {
                self.config.palette = generated.then(Default::default);
                if generated {
                    self.config.lut = None;
                }
            }
            if self.config.lut.is_some() || self.config.palette.is_some() {
                let defaults = Config {
                    palette: Some(Default::default()),
                    ..Config::general()
                };
                numbers(ui, &mut self.config, &defaults, settings::Section::Color);
            }
            ui.small(
                "Applied before CRT simulation. The table is embedded in presets and projects.",
            );
        });
    }
}
