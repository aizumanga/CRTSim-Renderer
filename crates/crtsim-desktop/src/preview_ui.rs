//! The preview area: the rendered picture, its comparison split and the video controls.
use crate::widgets::{show_image, Keyed};
use crate::*;

impl App {
    pub(crate) fn preview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.view, View::Original, "Original");
            ui.selectable_value(&mut self.view, View::Crt, "CRT");
            ui.selectable_value(&mut self.view, View::Compare, "Compare");
            ui.separator();
            ui.checkbox(&mut self.schedule.live, "Live preview");
            if ui
                .add_enabled(
                    !self.schedule.rendering() && !self.work.is_loading(),
                    egui::Button::new("Refresh"),
                )
                .clicked()
            {
                self.request_preview();
            }
        });
        ui.horizontal_wrapped(|ui| {
            let previous = self.preview_limit;
            egui::ComboBox::from_label("Preview quality")
                .selected_text(match self.preview_limit {
                    Some(800) => "Fast (800 px)",
                    Some(_) => "Balanced (1280 px)",
                    None => "Export resolution",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preview_limit, Some(800), "Fast (800 px)");
                    ui.selectable_value(&mut self.preview_limit, Some(1280), "Balanced (1280 px)");
                    ui.selectable_value(&mut self.preview_limit, None, "Export resolution");
                });
            if previous != self.preview_limit {
                self.changed();
            }
            ui.checkbox(&mut self.fit_preview, "Fit view");
            if !self.fit_preview {
                Keyed::new(0.25..=4.)
                    .reset_to(1.)
                    .show(ui, &mut self.zoom, |s| s.text("Zoom"));
            }
        });
        ui.small("Preview monitor").on_hover_text(
            "Drag the divider in Compare. Preview quality never changes \
            export resolution. Inspect mask detail at Export resolution \
            and 1× zoom.",
        );
        if let Ok(c) = self
            .config
            .with_max_output_side(self.input.dimensions(), self.preview_limit)
        {
            let input = self.input.dimensions();
            if let (Ok((w, h)), Ok(signal)) = (c.output_size(input), c.signal_size(input)) {
                let [columns, rows] = c.mask_repeats.resolve(signal);
                if w as f32 / columns < 6. || h as f32 / rows < 3. {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        "Dense mask at this preview size: aliasing / moiré is \
                        possible. Try a higher preview resolution or lower mask \
                        density.",
                    );
                }
            }
        }
        if self.schedule.rendering() || !self.work.is_idle() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(match self.work {
                    Work::Exporting { .. } => "Exporting…",
                    Work::Loading(_) => "Loading file/frame…",
                    Work::Idle => "Rendering preview…",
                });
            });
        }
        if let Some(audition) = &self.audition {
            ui.colored_label(
                ui.visuals().selection.bg_fill,
                format!(
                    "Previewing {} — click it to apply, or move away to return to your settings",
                    audition.label
                ),
            );
        }
        if self.rendered.is_some() && !self.schedule.is_current() {
            ui.colored_label(ui.visuals().warn_fg_color, "Preview is out of date.");
        }
        let controls_height = if self.video.is_some() { 136. } else { 0. };
        let available = egui::vec2(
            ui.available_width(),
            (ui.available_height() - controls_height).max(1.),
        );
        egui::ScrollArea::both()
            .max_height(available.y)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.view == View::Compare {
                    if let Some(ref im) = self.rendered {
                        compare(
                            ui,
                            egui::load::SizedTexture::from_handle(&self.original),
                            im.sized(),
                            available,
                            self.fit_preview,
                            self.zoom,
                            &mut self.workflow.comparison,
                        );
                    }
                } else if self.view == View::Original {
                    show_image(
                        ui,
                        egui::load::SizedTexture::from_handle(&self.original),
                        available,
                        self.fit_preview,
                        self.zoom,
                    );
                } else if let Some(ref im) = self.rendered {
                    show_image(ui, im.sized(), available, self.fit_preview, self.zoom);
                } else {
                    ui.label(
                        "Open an image, video or test card. Your rendered preview \
                will appear here.",
                    );
                }
            });
        self.video_controls(ui);
    }

    fn video_controls(&mut self, ui: &mut egui::Ui) {
        let Some(video) = self.video.clone() else {
            return;
        };
        let last = self.video_frames.saturating_sub(1);
        // A gallery in its own OS window has its own keyboard focus, so it no longer steals
        // the arrow keys below; only the embedded fallback shares this viewport's input.
        let galleries_overlap =
            ui.ctx().embed_viewports() && (self.show_gallery || self.show_lut_gallery);
        let enabled = self.can_start_work() && !galleries_overlap;
        let mut seek = false;
        ui.separator();
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .button(if self.workflow.playback.is_some() {
                        "Ⅱ Pause"
                    } else {
                        "▶ Play"
                    })
                    .clicked()
                {
                    if self.workflow.playback.is_some() {
                        self.stop_playback();
                    } else {
                        self.start_playback();
                    }
                }
                ui.small("Silent preview");
                ui.monospace(format!(
                    "{:02}:{:02} / {:02}:{:02}",
                    self.workflow.play_time as u64 / 60,
                    self.workflow.play_time as u64 % 60,
                    video.duration as u64 / 60,
                    video.duration as u64 % 60
                ));
                if ui
                    .add_enabled(self.video_frame > 0, egui::Button::new("|◀"))
                    .clicked()
                {
                    self.selected_frame = self.video_frame.saturating_sub(1);
                    seek = true;
                }
                if ui
                    .add_enabled(self.video_frame < last, egui::Button::new("▶|"))
                    .clicked()
                {
                    self.selected_frame = (self.video_frame + 1).min(last);
                    seek = true;
                }
                ui.label("Frame");
                let mut display_frame = self.selected_frame + 1;
                let number = ui.add(
                    egui::DragValue::new(&mut display_frame)
                        .clamp_range(1..=self.video_frames)
                        .speed(1),
                );
                self.selected_frame = display_frame.saturating_sub(1).min(last);
                seek |= number.drag_stopped()
                    || (number.lost_focus() && self.selected_frame != self.video_frame);
                ui.label(format!("/ {}", self.video_frames));
                if ui.button("Go").clicked() {
                    seek = true;
                }
            });
            ui.scope(|ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 20.).max(100.);
                let response =
                    ui.add(egui::Slider::new(&mut self.selected_frame, 0..=last).show_value(false));
                seek |= response.drag_stopped()
                    || (response.changed() && !ui.input(|i| i.pointer.any_down()));
            });
            if !ui.ctx().wants_keyboard_input() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                    if self.workflow.playback.is_some() {
                        self.stop_playback();
                    } else {
                        self.start_playback();
                    }
                }
                // A slider that has the keyboard takes the arrows for itself.
                let stepping = ui.ctx().memory(|m| m.focused().is_none());
                let arrow =
                    |key| stepping && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
                if arrow(egui::Key::ArrowLeft) {
                    self.selected_frame = self.video_frame.saturating_sub(1);
                    seek = true;
                }
                if arrow(egui::Key::ArrowRight) {
                    self.selected_frame = (self.video_frame + 1).min(last);
                    seek = true;
                }
            }
        });
        ui.small(format!(
            "Showing frame {} · Left/Right arrow keys step frames unless a slider is \
            selected (Esc lets go of it) · Export frame saves this settled CRT still as PNG.",
            self.video_frame + 1
        ));
        if seek && enabled && self.selected_frame != self.video_frame {
            self.load_video(video.path, self.selected_frame, true);
        }
    }
}

pub fn compare(
    ui: &mut egui::Ui,
    original: egui::load::SizedTexture,
    rendered: egui::load::SizedTexture,
    area: egui::Vec2,
    fit: bool,
    zoom: f32,
    split: &mut f32,
) {
    let native = rendered.size;
    let size = if fit {
        native * (area.x / native.x).min(area.y / native.y)
    } else {
        native * zoom
    };
    let (area_rect, response) =
        ui.allocate_exact_size(if fit { area } else { size }, egui::Sense::click_and_drag());
    let rect = egui::Rect::from_center_size(area_rect.center(), size);
    if let Some(pos) = response.interact_pointer_pos() {
        *split = ((pos.x - rect.left()) / rect.width()).clamp(0., 1.);
    }
    let x = rect.left() + rect.width() * *split;
    let uv = egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.));
    ui.painter()
        .image(rendered.id, rect, uv, egui::Color32::WHITE);
    let left = egui::Rect::from_min_max(rect.min, egui::pos2(x, rect.bottom()));
    let painter = ui.painter().with_clip_rect(left.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 0., egui::Color32::BLACK);
    let os = original.size;
    let os = os * (size.x / os.x).min(size.y / os.y);
    painter.image(
        original.id,
        egui::Rect::from_center_size(rect.center(), os),
        uv,
        egui::Color32::WHITE,
    );
    ui.painter().line_segment(
        [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
        egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
    );
    ui.painter().circle_filled(
        egui::pos2(x, rect.center().y),
        7.0_f32,
        egui::Color32::WHITE,
    );
    response
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
        .on_hover_text("Drag to compare · original on the left, CRT on the right");
}
