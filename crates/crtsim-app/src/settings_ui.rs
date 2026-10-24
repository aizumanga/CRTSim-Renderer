//! The settings panel: CRT settings by section, then source framing and color.
use crate::widgets::{choice, notes, numbers, resolution, Keyed, Shown};
use crate::*;
use crtsim_core::palette::Model;
use crtsim_core::settings::Section::*;
use std::collections::HashSet;

impl App {
    pub(crate) fn settings(&mut self, ui: &mut egui::Ui) {
        if let Some(video) = self.source.timeline.as_ref().map(|t| &t.video) {
            ui.heading("Video");
            ui.label(format!(
                "{:.2}s · {:.3} FPS · {}",
                video.duration,
                video.fps,
                if video.audio { "with audio" } else { "silent" }
            ));
            if video.hdr {
                ui.small("HDR source: FFmpeg tone-maps to SDR.");
            }
            notes(
                ui,
                "Preview is silent. Configure exported audio and compression in Export → Video.",
            );
            ui.separator();
        }
        ui.horizontal(|ui| {
            chrome::monitor(ui);
            ui.heading("Source");
        });
        egui::Frame::NONE
            .fill(ui.visuals().extreme_bg_color)
            .stroke(ui.visuals().window_stroke)
            .corner_radius(3.)
            .inner_margin(9.)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let name = egui::RichText::new(self.source.name()).strong();
                // A project's source that is not open says so.
                match self.source.kind() {
                    source::Kind::Missing { .. } => {
                        ui.colored_label(ui.visuals().warn_fg_color, name)
                    }
                    _ => ui.label(name),
                };
            });
        ui.small(format!(
            "Source: {} × {}",
            self.source.input().width(),
            self.source.input().height()
        ));
        // The two steps every session takes, beside what they act on.
        ui.add_enabled_ui(self.can_start_work(), |ui| {
            ui.horizontal_wrapped(|ui| {
                let ctx = ui.ctx().clone();
                if ui
                    .button("Open…")
                    .on_hover_text(format!(
                        "Open an image, video or animation · {}",
                        shortcut(&ctx, SHORTCUT_OPEN)
                    ))
                    .clicked()
                {
                    self.dialog(Dialog::File, &ctx);
                }
                if self.source.timeline.is_some() {
                    if ui
                        .button("Export video…")
                        .on_hover_text("Choose the format, then where to save it")
                        .clicked()
                    {
                        self.open_video_export(false);
                    }
                } else if ui
                    .button("Export PNG…")
                    .on_hover_text(format!(
                        "Save the CRT picture at full resolution · {}",
                        shortcut(&ctx, SHORTCUT_EXPORT)
                    ))
                    .clicked()
                {
                    self.dialog(Dialog::Export, &ctx);
                }
            });
        });
        // What Reset, and each setting's own reset, return to.
        let default = gallery::default_preset();
        ui.horizontal(|ui| {
            if ui.button("Undo").on_hover_text("Ctrl+Z").clicked() {
                self.undo();
            }
            if ui.button("Redo").on_hover_text("Ctrl+Shift+Z").clicked() {
                self.redo();
            }
            if ui
                .button("Reset")
                .on_hover_text(format!(
                    "Reset to the {} preset; Undo restores your settings",
                    default.name
                ))
                .clicked()
            {
                self.replace_config(default.config.clone());
            }
        });
        self.preset_in_use(ui);
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Presets…")
                .on_hover_text("Point at a preset to see it on your picture; click to apply it")
                .clicked()
            {
                self.refresh_gallery();
                self.presets.open = true;
            }
            if ui
                .button("General image")
                .on_hover_text(
                    "Apply the General image preset: square pixels and smooth resizing, for \
                     photos and screenshots",
                )
                .clicked()
            {
                self.replace_config(Config::general());
            }
        });
        let before = self.config.clone();
        let defaults = &default.config;
        let shown = self.shown_settings(ui);
        if shown.none() {
            ui.label("No setting differs from this preset.");
        }
        let image = ["signal", "output", "tube_aspect", "fit", "filter"];
        section(ui, &shown, "Image & output", &image, &[Image], |ui| {
            if shown.key("signal") {
                resolution(
                    ui,
                    "Signal",
                    &mut self.config.signal,
                    &[
                        "auto", "native", "original", "240p", "288p", "360p", "480p", "576p",
                    ],
                );
            }
            if shown.key("output") {
                resolution(
                    ui,
                    "Export size",
                    &mut self.config.output,
                    &["720p", "1080p", "1440p", "4k", "reference", "match-input"],
                );
            }
            if shown.key("tube_aspect") {
                // Any other shape is the Glass section's slider's.
                choice(
                    ui,
                    "Tube",
                    &mut self.config.tube_aspect,
                    &[
                        (4. / 3., "4:3"),
                        (16. / 10., "16:10"),
                        (16. / 9., "16:9"),
                        (3. / 4., "3:4, portrait"),
                    ],
                );
            }
            if shown.key("fit") {
                choice(
                    ui,
                    "Fit on the tube",
                    &mut self.config.fit,
                    &[
                        (Fit::Contain, "Contain"),
                        (Fit::Cover, "Cover (crop)"),
                        (Fit::Stretch, "Stretch"),
                        (Fit::Reference, "Reference"),
                    ],
                );
            }
            if shown.key("filter") {
                choice(
                    ui,
                    "Resize filter",
                    &mut self.config.filter,
                    &[
                        (Filter::Lanczos, "Lanczos (smooth)"),
                        (Filter::Nearest, "Nearest (pixel art)"),
                    ],
                );
            }
            numbers(ui, &mut self.config, defaults, Image, &shown);
            if let (Ok(signal), Ok(output)) = (
                self.config.signal_size(self.source.input().dimensions()),
                self.config.output_size(self.source.input().dimensions()),
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
            notes(
                ui,
                "Rounded glass may hide extreme corners even with Contain. Alpha uses the \
                 selected background. SDR output, in sRGB or the PNG colour space chosen \
                 under Export.",
            );
        });
        self.framing_and_color(ui, defaults, &shown);
        let color = ["color_mode", "mask_antialias"];
        section(ui, &shown, "Color processing", &color, &[Grade], |ui| {
            if shown.key("color_mode") {
                choice(
                    ui,
                    "Color processing",
                    &mut self.config.color_mode,
                    &[
                        (ColorMode::Reference, "Original gamma"),
                        (ColorMode::LinearLight, "Linear light (experimental)"),
                    ],
                );
                if self.config.color_mode == ColorMode::LinearLight {
                    ui.small(
                        "Linear-light glass, lighting and bloom; SDR output. The analog signal \
                         still uses the original gamma-space model.",
                    );
                }
            }
            section(ui, &shown, "Optional color grade", &[], &[Grade], |ui| {
                numbers(ui, &mut self.config, defaults, Grade, &shown);
                notes(
                    ui,
                    "YIQ hue/chroma adjustment. This is an optional grade, not Super Win the \
                     Game's NES palette or a complete NTSC decoder.",
                );
            });
            if shown.key("mask_antialias") {
                ui.checkbox(
                    &mut self.config.mask_antialias,
                    "Filter mask when shrinking",
                )
                .on_hover_text(
                    "Samples the mask from averaged, smaller copies of itself, as the original \
                     did, so it stays smooth where it is drawn smaller than it is. Off samples \
                     only the full-size mask, which can shimmer into moiré.",
                );
            }
        });
        ui.separator();
        if shown.any(&[], &[Signal]) {
            chrome::Section::new("CRT signal")
                .default_open(true)
                .changes(shown.changes())
                .show(ui, |ui| {
                    numbers(ui, &mut self.config, defaults, Signal, &shown)
                });
        }
        section(
            ui,
            &shown,
            "Glass & mask",
            &["faceted_glass"],
            &[Glass, Mask],
            |ui| {
                numbers(ui, &mut self.config, defaults, Glass, &shown);
                if shown.key("faceted_glass") {
                    ui.checkbox(&mut self.config.faceted_glass, "Glass as the mesh's facets")
                        .on_hover_text(
                            "The glass as Super Win the Game's mesh draws it, each triangle flat \
                         between corners on the curve, so the picture shifts a hair from facet \
                         to facet, as the game's mask does. Off is the smooth curve the mesh was \
                         cut from.",
                        );
                }
                if shown.key("mask_repeats") {
                    ui.separator();
                    self.mask_density(ui);
                }
                numbers(ui, &mut self.config, defaults, Mask, &shown);
            },
        );
        let bloom = ["bloom_from_mipmaps", "reflection_as_screen"];
        section(ui, &shown, "Bloom & reflections", &bloom, &[Bloom], |ui| {
            numbers(ui, &mut self.config, defaults, Bloom, &shown);
            if shown.key("bloom_from_mipmaps") {
                ui.checkbox(&mut self.config.bloom_from_mipmaps, "Bloom from mipmaps")
                    .on_hover_text(
                        "The bloom reads the picture through averaged, smaller copies of \
                         itself, as Super Win the Game does, so each of its taps averages \
                         16×16 pixels. Off is the public CRTSim source: each tap reads the \
                         picture itself, where the mask can flicker into the glow.",
                    );
            }
            if shown.key("reflection_as_screen") {
                ui.checkbox(
                    &mut self.config.reflection_as_screen,
                    "Reflection as on the screen",
                )
                .on_hover_text(
                    "The bezel reflects the picture with the screen's overscan and mask \
                     density, as Super Win the Game draws it. Off is the public CRTSim source: \
                     the overscan the other way round and half the mask's rows.",
                );
            }
        });
        section(ui, &shown, "Frame & lighting", &[], &[Lighting], |ui| {
            numbers(ui, &mut self.config, defaults, Lighting, &shown)
        });
        let persistence = [
            "warmup",
            "phase",
            "flip_artifacts",
            "interlace",
            "glow_floor",
            "glow_per_frame",
        ];
        let title = "Persistence & artifact phase";
        section(ui, &shown, title, &persistence, &[Persistence], |ui| {
            numbers(ui, &mut self.config, defaults, Persistence, &shown);
            if shown.key("warmup") {
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
            }
            if shown.key("phase") {
                choice(
                    ui,
                    "Phase",
                    &mut self.config.phase,
                    &[
                        (Phase::Stable, "Stable"),
                        (Phase::A, "A"),
                        (Phase::B, "B"),
                        (Phase::Alternating, "Alternating"),
                    ],
                );
            }
            if shown.key("flip_artifacts") {
                ui.checkbox(&mut self.config.flip_artifacts, "Flip artifact pattern")
                    .on_hover_text(
                        "The composite artifact pattern upside down, each tick blending in the \
                         row above rather than below, as Super Win the Game's Linux build draws \
                         it. Off is the public CRTSim source's orientation, which the game's \
                         Windows build draws.",
                    );
            }
            if shown.key("interlace") {
                ui.checkbox(&mut self.config.interlace, "Interlaced fields")
                    .on_hover_text(
                        "Each tick scans every other row, alternating fields; the rows it skips \
                         only fade by persistence. Use with a 480- or 576-row signal.",
                    );
            }
            if shown.key("glow_floor") {
                ui.checkbox(&mut self.config.glow_floor, "Glow floor")
                    .on_hover_text(
                        "The glow never fades below the level 8-bit history rounds it back to, \
                         1 of 255 in red and green at the default persistence, as every pixel \
                         stays once Super Win the Game's title screen has lit it. Off fades \
                         to black.",
                    );
            }
            if shown.key("glow_per_frame") {
                ui.checkbox(&mut self.config.glow_per_frame, "Glow fades per frame")
                    .on_hover_text(
                        "The glow fades by persistence once each frame drawn, as Super Win the \
                         Game does, so on a display faster than 60 Hz it fades faster. Off \
                         fades it by the time that passes, the same at any rate.",
                    );
            }
            notes(
                ui,
                "Each still starts from a dark tube. Higher persistence may require more \
                 warm-up ticks. Alternating phase depends on tick count.",
            );
        });
        let finish = ["fxaa", "tate", "crt_off", "screenshot_stills"];
        section(ui, &shown, "Picture finish", &finish, &[Finish], |ui| {
            numbers(ui, &mut self.config, defaults, Finish, &shown);
            if shown.key("fxaa") {
                choice(
                    ui,
                    "FXAA",
                    &mut self.config.fxaa,
                    &[
                        (Fxaa::Off, "Off"),
                        (Fxaa::Low, "Low"),
                        (Fxaa::Medium, "Medium"),
                        (Fxaa::High, "High"),
                        (Fxaa::Ultra, "Ultra"),
                    ],
                );
            }
            if shown.key("tate") {
                choice(
                    ui,
                    "Tate",
                    &mut self.config.tate,
                    &[
                        (Tate::Off, "Off"),
                        (Tate::Quarter, "A quarter turn left"),
                        (Tate::Half, "Upside down"),
                        (Tate::ThreeQuarters, "A quarter turn right"),
                    ],
                );
            }
            if shown.key("crt_off") {
                ui.checkbox(&mut self.config.crt_off, "CRT off")
                    .on_hover_text(
                        "The clean picture without the CRT: square-cut pixels at the signal's \
                         aspect, fitted inside the frame, as Super Win the Game draws it with \
                         its CRT simulation off.",
                    );
            }
            if shown.key("screenshot_stills") {
                ui.checkbox(&mut self.config.screenshot_stills, "Stills as screenshots")
                    .on_hover_text(
                        "With alternating phase, a still's last tick draws both artifact \
                         patterns at once, as Super Win the Game's screenshots do. Video \
                         keeps alternating.",
                    );
            }
            notes(
                ui,
                "Super Win the Game's options screen: brightness, FXAA and tate act on the \
                 finished picture.",
            );
        });
        section(ui, &shown, "TV static", &[], &[Static], |ui| {
            numbers(ui, &mut self.config, defaults, Static, &shown);
            notes(
                ui,
                "Snow, a rolling band and a wavering picture, for switching channels. With a \
                 cycle, the static comes and goes over that many seconds of video.",
            );
        });
        if self.config != before {
            self.changed();
        }
    }
    /// Which preset the settings are, or are closest to, and how many of them differ from it,
    /// listed when pointed at, with the toggle that shows only those.
    fn preset_in_use(&mut self, ui: &mut egui::Ui) {
        let Some(closest) = self.presets.closest(&self.config) else {
            return;
        };
        let text = match closest.changes.len() {
            0 => format!("Preset: {}", closest.name),
            1 => format!("Preset: {} · 1 change", closest.name),
            n => format!("Preset: {} · {n} changes", closest.name),
        };
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(egui::RichText::new(text).strong());
            ui.checkbox(&mut self.changed_only, "Changed only")
                .on_hover_text("Show only the settings that differ from this preset");
            if closest.changes.is_empty() {
                return;
            }
            label.on_hover_ui(|ui| {
                ui.label(format!("What differs from {}:", closest.name));
                egui::Grid::new("changes from the preset")
                    .striped(true)
                    .show(ui, |ui| {
                        for change in &closest.changes {
                            ui.label(&change.setting);
                            ui.weak(&change.from);
                            ui.label("→");
                            ui.strong(&change.to);
                            ui.end_row();
                        }
                    });
            });
        });
    }
    /// Which settings the panel shows: every one, or with Changed only on, those that differ
    /// from the preset named above them. While a pointer is down, one that showed keeps showing,
    /// so a slider dragged across the preset's value does not vanish from under it.
    fn shown_settings(&mut self, ui: &egui::Ui) -> Shown {
        if !self.changed_only {
            return Shown::all();
        }
        let changes: HashSet<&'static str> = self
            .presets
            .closest(&self.config)
            .map(|closest| closest.changes.iter().map(|change| change.key).collect())
            .unwrap_or_default();
        if ui.input(|i| i.pointer.any_down()) {
            self.changes_shown.extend(changes);
        } else {
            self.changes_shown = changes;
        }
        Shown::only(self.changes_shown.clone())
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
                let signal = self.config.signal_size(self.source.input().dimensions());
                MaskRepeats::Fixed(
                    self.config
                        .mask_repeats
                        .resolve(signal.unwrap_or((256, 224))),
                )
            };
        }
    }
    /// The source's framing, and its colour mapping before the CRT.
    fn framing_and_color(&mut self, ui: &mut egui::Ui, defaults: &Config, shown: &Shown) {
        let framing = [
            "screen_only",
            "source.crop",
            "source.rotation",
            "source.zoom",
            "source.position",
            "source.background",
            "source.checkerboard",
        ];
        if shown.any(&framing, &[]) {
            chrome::Section::new("Source & framing")
                .default_open(true)
                .changes(shown.changes())
                .show(ui, |ui| self.framing(ui, shown));
        }
        section(
            ui,
            shown,
            "Color & LUT",
            &["lut", "palette.model"],
            &[Color],
            |ui| {
                ui.label(match (&self.config.palette, &self.config.lut) {
                    (Some(palette), _) => format!("NES palette, {}", palette.model.name()),
                    (None, Some(lut)) => lut.name.clone(),
                    (None, None) => "No LUT".into(),
                });
                if ui.button("LUT gallery…").clicked() {
                    self.stop_playback();
                    self.luts.open = true;
                }
                if ui.button("Import 3D .cube…").clicked() {
                    self.dialog(Dialog::Lut, &ui.ctx().clone());
                }
                if self.config.lut.is_some() && ui.button("Remove LUT").clicked() {
                    self.config.set_lut(None);
                }
                if shown.key("palette.model") {
                    self.nes_palette(ui);
                }
                if self.config.lut.is_some() || self.config.palette.is_some() {
                    numbers(ui, &mut self.config, defaults, Color, shown);
                }
                notes(
                    ui,
                    "Applied before CRT simulation. The table is embedded in presets and projects.",
                );
            },
        );
    }
    /// The source's crop, rotation, zoom and pan, and what shows through its transparency.
    fn framing(&mut self, ui: &mut egui::Ui, shown: &Shown) {
        if shown.key("screen_only") {
            ui.checkbox(&mut self.config.screen_only, "Screen only · no bezel");
        }
        if shown.key("source.crop") {
            egui::CollapsingHeader::new("Crop edges · percent")
                .default_open(shown.changes())
                .show(ui, |ui| {
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
        }
        let source = &mut self.config.source;
        if shown.key("source.rotation") {
            Keyed::new(-180.0..=180.)
                .reset_to(0.)
                .show(ui, &mut source.rotation, |s| s.text("Rotation °"));
        }
        if shown.key("source.zoom") {
            Keyed::new(0.05..=4.)
                .logarithmic(true)
                .reset_to(1.)
                .show(ui, &mut source.zoom, |s| s.text("Source zoom"));
        }
        if shown.key("source.position") {
            for (position, label) in source.position.iter_mut().zip(["Pan X", "Pan Y"]) {
                Keyed::new(-1.0..=1.)
                    .reset_to(0.)
                    .show(ui, position, |s| s.text(label));
            }
        }
        if shown.key("source.background") || shown.key("source.checkerboard") {
            ui.label("Transparency background (included in exports)");
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(!source.checkerboard && source.background == [0; 3], "Black")
                    .clicked()
                {
                    source.checkerboard = false;
                    source.background = [0; 3];
                }
                if ui
                    .selectable_label(
                        !source.checkerboard && source.background == [255; 3],
                        "White",
                    )
                    .clicked()
                {
                    source.checkerboard = false;
                    source.background = [255; 3];
                }
                ui.checkbox(&mut source.checkerboard, "Checker");
            });
            if ui.color_edit_button_srgb(&mut source.background).changed() {
                source.checkerboard = false;
            }
        }
        if ui.button("Reset source framing").clicked() {
            self.config.source = Default::default();
        }
    }
    /// The NES palette in place of a LUT, and how it is made.
    fn nes_palette(&mut self, ui: &mut egui::Ui) {
        let mut generated = self.config.palette.is_some();
        let toggled = ui
            .checkbox(&mut generated, "NES palette")
            .on_hover_text(
                "Super Win the Game's NTSC palette, set by Tint, Tint I and Tint Q. It recolours \
                 images drawn in the NES palette and replaces any LUT.",
            )
            .changed();
        if toggled {
            self.config.set_palette(generated.then(Default::default));
        }
        if let Some(palette) = &mut self.config.palette {
            ui.radio_value(&mut palette.model, Model::Game, Model::Game.name())
                .on_hover_text(
                    "Exactly as Super Win the Game makes it, for art in the NES palette its own \
                     is drawn in, which begins 7C7C7C 0000FC 0000BC. Tint I and Q steer each hue \
                     there, not its saturation.",
                );
            ui.radio_value(
                &mut palette.model,
                Model::Signal,
                "From the composite signal",
            )
            .on_hover_text(
                "The NES's colours decoded from its measured signal, turned by Tint and scaled \
                 along I and Q, for art in MAME's NES palette, which the included NES LUTs also \
                 expect.",
            );
        }
    }
}

/// A section of the panel, shown when any of `keys`, or of the numeric settings of `sections`,
/// shows; with Changed only on, open and holding only what changed.
fn section(
    ui: &mut egui::Ui,
    shown: &Shown,
    title: &str,
    keys: &[&str],
    sections: &[settings::Section],
    contents: impl FnOnce(&mut egui::Ui),
) {
    if shown.any(keys, sections) {
        chrome::Section::new(title)
            .changes(shown.changes())
            .show(ui, contents);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_only_shows_what_differs_and_keeps_a_held_slider() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        let shown = |app: &mut App, pressed: Option<bool>| {
            let pos = egui::pos2(10., 10.);
            let events = pressed.map_or(vec![], |pressed| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ]
            });
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut shown = None;
            ctx.run_ui(input, |ui| shown = Some(app.shown_settings(ui)))
                .textures_delta
                .clear();
            shown.unwrap()
        };
        assert!(
            shown(&mut app, None).key("fov"),
            "every setting shows when off"
        );
        app.changed_only = true;
        let bloom = app.config.bloom;
        app.config.bloom += 0.25;
        let now = shown(&mut app, None);
        assert!(now.key("bloom") && !now.key("fov"));
        assert!(now.any(&[], &[Bloom]) && !now.any(&["warmup"], &[Persistence]));
        // Dragged back to the preset's value, it stays until the pointer lets go.
        app.config.bloom = bloom;
        assert!(shown(&mut app, Some(true)).key("bloom"));
        assert!(shown(&mut app, Some(false)).none());
    }
}
