//! Small widgets shared by the settings panel and the preview.
use crtsim_core::{config::Config, settings};
use eframe::{egui, emath};
use std::ops::RangeInclusive;

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
    range: RangeInclusive<f32>,
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
        let slider = Keyed::new(range).logarithmic(logarithmic).reset_to(default);
        slider.show(ui, value, |slider| slider.clamp_to_range(false).text(label));
    });
}

/// A slider that also answers the keyboard. Clicking or dragging it gives it the keyboard; then
/// ←/→ step it a hundredth of its range, ten times as far with Shift and a tenth as far with
/// Alt, and Delete or Backspace returns it to its default. Esc, or clicking elsewhere, lets go.
pub(crate) struct Keyed<N: emath::Numeric> {
    range: RangeInclusive<N>,
    logarithmic: bool,
    default: Option<N>,
}

/// How far each arrow press moves a slider, as a fraction of its range, by modifier. Shift
/// and Alt come first: a plain arrow press would also match them.
const STEPS: [(egui::Modifiers, f64); 3] = [
    (egui::Modifiers::SHIFT, 0.1),
    (egui::Modifiers::ALT, 0.001),
    (egui::Modifiers::NONE, 0.01),
];

impl<N: emath::Numeric> Keyed<N> {
    pub fn new(range: RangeInclusive<N>) -> Self {
        Self {
            range,
            logarithmic: false,
            default: None,
        }
    }

    pub fn logarithmic(self, logarithmic: bool) -> Self {
        Self {
            logarithmic,
            ..self
        }
    }

    pub fn reset_to(self, default: N) -> Self {
        Self {
            default: Some(default),
            ..self
        }
    }

    /// Adds the slider for `value`, finished by `configure`, and applies any key presses.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        value: &mut N,
        configure: impl for<'v> FnOnce(egui::Slider<'v>) -> egui::Slider<'v>,
    ) -> egui::Response {
        let before = *value;
        let slider =
            egui::Slider::new(&mut *value, self.range.clone()).logarithmic(self.logarithmic);
        let mut response = ui.add(configure(slider));
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if !response.has_focus() {
            return response;
        }
        // The slider already moved a pixel's worth for each arrow press; the step replaces that.
        let mut fraction = 0.;
        let reset = ui.input_mut(|input| {
            for (modifiers, step) in STEPS {
                let right = input.count_and_consume_key(modifiers, egui::Key::ArrowRight);
                let left = input.count_and_consume_key(modifiers, egui::Key::ArrowLeft);
                fraction += step * (right as f64 - left as f64);
            }
            let delete = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete);
            input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace) || delete
        });
        let next = match self.default {
            Some(default) if reset => Some(default),
            _ if fraction != 0. => Some(N::from_f64(stepped(
                before.to_f64(),
                (self.range.start().to_f64(), self.range.end().to_f64()),
                fraction,
                self.logarithmic,
                N::INTEGRAL,
            ))),
            _ => None,
        };
        if let Some(next) = next {
            *value = next;
            response.mark_changed();
        }
        response
    }
}

/// `value` moved `fraction` of the way across `range`, down when negative: evenly, or evenly in
/// proportion on a logarithmic slider. A whole-number value moves by at least one. The value
/// stops at the range's ends, but a typed value already beyond one is not pulled back in.
fn stepped(
    value: f64,
    (min, max): (f64, f64),
    fraction: f64,
    logarithmic: bool,
    integral: bool,
) -> f64 {
    let moved = if logarithmic && min > 0. && value > 0. {
        (value.ln() + fraction * (max.ln() - min.ln())).exp()
    } else {
        value + fraction * (max - min)
    };
    let moved = if integral {
        let rounded = moved.round();
        if rounded == value {
            value + fraction.signum()
        } else {
            rounded
        }
    } else {
        // Keep the value as a person would type it, not 0.30000000000000004.
        (moved * 1e6).round() / 1e6
    };
    moved.clamp(min.min(value), max.max(value))
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
    fn arrow_keys_step_by_a_share_of_the_range() {
        let step = |value, range, fraction, log, int| stepped(value, range, fraction, log, int);
        assert_eq!(step(0.5, (0., 1.), 0.01, false, false), 0.51);
        assert_eq!(step(0.3, (0., 1.), -0.1, false, false), 0.2);
        assert_eq!(step(0.999, (0., 1.), 0.01, false, false), 1.);
        // A value typed beyond the range stays where it is going the other way.
        assert_eq!(step(1.5, (0., 1.), 0.01, false, false), 1.5);
        assert_eq!(step(1.5, (0., 1.), -0.1, false, false), 1.4);
        // Logarithmic: the same proportion at any value; 1% of 1–100 is a factor of 100^0.01.
        let up = step(10., (1., 100.), 0.01, true, false);
        assert!((up - 10. * 100f64.powf(0.01)).abs() < 1e-5);
        // Whole numbers move by at least one.
        assert_eq!(step(10., (0., 240.), 0.001, false, true), 11.);
        assert_eq!(step(10., (0., 240.), -0.1, false, true), 0.);
        assert_eq!(step(0., (0., 240.), -0.01, false, true), 0.);
    }

    #[test]
    fn a_focused_slider_takes_the_arrow_keys_and_delete_resets_it() {
        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(0.5f32);
        let frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut left_over = vec![];
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut shown = value.get();
                    Keyed::new(0.0..=1.)
                        .reset_to(0.2)
                        .show(ui, &mut shown, |slider| slider);
                    value.set(shown);
                    left_over = ui.input(|i| i.events.clone());
                });
            });
            left_over
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frame(vec![]);
        // Without the keyboard, the arrows are left for others, such as frame stepping.
        let events = frame(vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)]);
        assert_eq!((value.get(), events.len()), (0.5, 1));
        frame(vec![key(egui::Key::Tab, egui::Modifiers::NONE)]);
        let events = frame(vec![key(egui::Key::ArrowRight, egui::Modifiers::NONE)]);
        assert_eq!((value.get(), events.len()), (0.51, 0));
        frame(vec![key(egui::Key::ArrowLeft, egui::Modifiers::SHIFT)]);
        assert_eq!(value.get(), 0.41);
        frame(vec![key(egui::Key::ArrowRight, egui::Modifiers::ALT)]);
        assert_eq!(value.get(), 0.411);
        frame(vec![key(egui::Key::Delete, egui::Modifiers::NONE)]);
        assert_eq!(value.get(), 0.2);
    }

    #[test]
    fn reset_tooltips_show_values_as_written() {
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(50.), "50");
        assert_eq!(format_value(-0.115), "-0.115");
        assert_eq!(format_value(8. / 7.), "1.143");
    }
}
