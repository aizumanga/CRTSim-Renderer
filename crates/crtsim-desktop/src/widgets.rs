//! Small widgets shared by the settings panel and the preview.
use crtsim_core::{config::Config, settings};
use eframe::egui;

/// The numeric settings of one section of the panel, in the order `settings::SETTINGS` lists them.
pub(crate) fn numbers(
    ui: &mut egui::Ui,
    config: &mut Config,
    defaults: &Config,
    section: settings::Section,
) {
    for setting in settings::SETTINGS {
        let Some(numbers) = setting.numbers.as_ref().filter(|n| n.section == section) else {
            continue;
        };
        let values = (numbers.access.get_mut)(config);
        match &numbers.control {
            settings::Control::Slider { span, logarithmic } => {
                let defaults = (numbers.access.get)(defaults);
                for (index, value) in values.iter_mut().enumerate() {
                    // A default with no number here, such as a mask that follows the signal,
                    // leaves nothing to reset to.
                    let default = defaults.get(index).copied().unwrap_or(*value);
                    slider(
                        ui,
                        setting.value_name(index),
                        value,
                        default,
                        span.clone(),
                        *logarithmic,
                    );
                }
            }
            settings::Control::Color => {
                let rgb: &mut [f32; 3] = values.try_into().expect("a color is three values");
                ui.horizontal(|ui| {
                    ui.label(setting.label);
                    ui.color_edit_button_rgb(rgb);
                });
            }
        }
    }
}

/// A setting's slider, with a button that returns it alone to `default`. The button keeps its
/// place while disabled, so the panel does not shift as values move on and off their defaults.
pub(crate) fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    default: f32,
    range: std::ops::RangeInclusive<f32>,
    logarithmic: bool,
) {
    ui.horizontal(|ui| {
        if ui
            .add_enabled(*value != default, egui::Button::new("↺").small())
            .on_hover_text(format!("Reset {label} to {}", format_value(default)))
            .clicked()
        {
            *value = default;
        }
        ui.add(
            egui::Slider::new(value, range)
                .logarithmic(logarithmic)
                .clamp_to_range(false)
                .text(label),
        );
    });
}

/// A number as a person would write it: no trailing zeros, at most three decimals.
pub(crate) fn format_value(value: f32) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
pub(crate) fn resolution(ui: &mut egui::Ui, label: &str, value: &mut String, presets: &[&str]) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_source(label)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for preset in presets {
                    ui.selectable_value(value, (*preset).into(), *preset);
                }
            });
        ui.add(egui::TextEdit::singleline(value).desired_width(110.))
            .on_hover_text("Preset name or custom WIDTHxHEIGHT");
    });
}
pub(crate) fn show_image(
    ui: &mut egui::Ui,
    im: egui::load::SizedTexture,
    available: egui::Vec2,
    fit: bool,
    zoom: f32,
) {
    let size = im.size;
    let factor = if fit {
        (available.x / size.x).min(available.y / size.y).max(0.01)
    } else {
        zoom / ui.ctx().pixels_per_point()
    };
    let display = size * factor;
    let (area, _) =
        ui.allocate_exact_size(if fit { available } else { display }, egui::Sense::hover());
    let rect = egui::Rect::from_center_size(area.center(), display);
    ui.painter().image(
        im.id,
        rect,
        egui::Rect::from_min_max(egui::pos2(0., 0.), egui::pos2(1., 1.)),
        egui::Color32::WHITE,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_tooltips_show_values_as_written() {
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(50.), "50");
        assert_eq!(format_value(-0.115), "-0.115");
        assert_eq!(format_value(8. / 7.), "1.143");
    }
}
