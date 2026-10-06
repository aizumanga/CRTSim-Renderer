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
        slider.show(ui, value, |slider| {
            slider.clamping(egui::SliderClamping::Never).text(label)
        });
    });
}

/// A slider that moves finely and also answers the keyboard. A press jumps it to the pointer,
/// as any slider; dragging on from there moves it a quarter as far as the pointer, so it can be
/// set more finely than its width allows, as far as the pointer with Shift and a tenth as far
/// again with Alt. Clicking or dragging it gives it the keyboard; then ←/→ step it a
/// thousandth of its range, ten times as far with Shift and a tenth as far with Alt, and
/// Delete or Backspace returns it to its default. Esc, or clicking elsewhere, lets go.
pub(crate) struct Keyed<N: emath::Numeric> {
    range: RangeInclusive<N>,
    logarithmic: bool,
    default: Option<N>,
}

/// How far a slider moves with a modifier held: for each arrow press, as a fraction of its
/// range, and while dragged, as a fraction of the pointer's movement along it.
struct Pace {
    modifiers: egui::Modifiers,
    key: f64,
    drag: f64,
}

/// Shift and Alt come first: a plain arrow press or drag would also match them.
const PACES: [Pace; 3] = [
    Pace {
        modifiers: egui::Modifiers::SHIFT,
        key: 0.01,
        drag: 1.,
    },
    Pace {
        modifiers: egui::Modifiers::ALT,
        key: 0.0001,
        drag: 0.025,
    },
    Pace {
        modifiers: egui::Modifiers::NONE,
        key: 0.001,
        drag: 0.25,
    },
];

/// Where the slider being dragged has got to: its value, before it is rounded to show, and the
/// pointer's position along it then. Each frame moves on from it at the pace held then, so a
/// whole-number slider gathers movements too small to show. Only one slider is dragged at a
/// time, so one is kept for them all.
#[derive(Clone, Copy)]
struct Anchor {
    value: f64,
    x: f32,
}

impl Anchor {
    fn id() -> egui::Id {
        egui::Id::new("slider drag anchor")
    }
}

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

    fn span(&self) -> (f64, f64) {
        (self.range.start().to_f64(), self.range.end().to_f64())
    }

    /// Adds the slider for `value`, finished by `configure`, and applies any drag and key
    /// presses.
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
        // The slider puts itself under the pointer on the frame it is let go too.
        if response.dragged() || response.drag_stopped() {
            if let Some(dragged) = self.dragged(ui, &response, *value) {
                *value = dragged;
            }
        }
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        if !response.has_focus() {
            return response;
        }
        // The slider already moved a pixel's worth for each arrow press; the step replaces that.
        let mut fraction = 0.;
        let reset = ui.input_mut(|input| {
            for pace in &PACES {
                let right = input.count_and_consume_key(pace.modifiers, egui::Key::ArrowRight);
                let left = input.count_and_consume_key(pace.modifiers, egui::Key::ArrowLeft);
                fraction += pace.key * (right as f64 - left as f64);
            }
            let delete = input.consume_key(egui::Modifiers::NONE, egui::Key::Delete);
            input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace) || delete
        });
        let next = match self.default {
            Some(default) if reset => Some(default),
            _ if fraction != 0. => Some(N::from_f64(stepped(
                before.to_f64(),
                self.span(),
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

    /// The value a drag under way gives in place of the slider's own, which follows the
    /// pointer: `jumped` is where the slider put it this frame.
    fn dragged(&self, ui: &egui::Ui, response: &egui::Response, jumped: N) -> Option<N> {
        let (x, modifiers) =
            ui.input(|input| Some((input.pointer.interact_pos()?.x, input.modifiers)))?;
        // The press itself jumps to the pointer; the drag moves on from there.
        let kept = ui.data(|data| data.get_temp::<Anchor>(Anchor::id()));
        let anchor = match kept {
            Some(anchor) if !response.drag_started() => anchor,
            _ => Anchor {
                value: jumped.to_f64(),
                x,
            },
        };
        let drag = PACES
            .iter()
            .find(|pace| modifiers.contains(pace.modifiers))?
            .drag;
        let fraction = (x - anchor.x) as f64 / ui.spacing().slider_width as f64 * drag;
        let value = moved(anchor.value, self.span(), fraction, self.logarithmic);
        ui.data_mut(|data| data.insert_temp(Anchor::id(), Anchor { value, x }));
        Some(N::from_f64(tidy(value, N::INTEGRAL)))
    }
}

/// `value` moved `fraction` of the way across `range`, down when negative: evenly, or evenly in
/// proportion on a logarithmic slider. The value stops at the range's ends, but a typed value
/// already beyond one is not pulled back in.
fn moved(value: f64, (min, max): (f64, f64), fraction: f64, logarithmic: bool) -> f64 {
    let moved = if logarithmic && min > 0. && value > 0. {
        (value.ln() + fraction * (max.ln() - min.ln())).exp()
    } else {
        value + fraction * (max - min)
    };
    moved.clamp(min.min(value), max.max(value))
}

/// `value` as a person would type it: a whole number for a whole-number slider, else no more
/// than six decimals, not 0.30000000000000004.
fn tidy(value: f64, integral: bool) -> f64 {
    if integral {
        value.round()
    } else {
        (value * 1e6).round() / 1e6
    }
}

/// `value` stepped by an arrow press: [`moved`], but a whole-number value moves by at least one.
fn stepped(value: f64, range: (f64, f64), fraction: f64, logarithmic: bool, integral: bool) -> f64 {
    let next = tidy(moved(value, range, fraction, logarithmic), integral);
    if integral && next == value {
        (value + fraction.signum()).clamp(range.0.min(value), range.1.max(value))
    } else {
        next
    }
}

/// A number as a person would write it: no trailing zeros, at most three decimals.
pub(crate) fn format_value(value: f32) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}
/// A drop-down that chooses `value` from `options`, each shown by its name, with the chosen
/// one's name on the closed box too. Whether the choice changed.
pub(crate) fn choice<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let chosen = options
        .iter()
        .find(|(option, _)| option == value)
        .map_or("", |(_, name)| name);
    let mut changed = false;
    egui::ComboBox::from_label(label)
        .selected_text(chosen)
        .show_ui(ui, |ui| {
            for (option, name) in options {
                changed |= ui.selectable_value(value, option.clone(), *name).changed();
            }
        });
    changed
}

pub(crate) fn resolution(ui: &mut egui::Ui, label: &str, value: &mut String, presets: &[&str]) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(label)
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
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut shown = value.get();
                    Keyed::new(0.0..=1.)
                        .reset_to(0.2)
                        .show(ui, &mut shown, |slider| slider);
                    value.set(shown);
                    left_over = ui.input(|i| i.events.clone());
                });
            });
            // No renderer takes the font atlas in a test, so its upload is dropped deliberately.
            output.textures_delta.clear();
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
        assert_eq!((value.get(), events.len()), (0.501, 0));
        frame(vec![key(egui::Key::ArrowLeft, egui::Modifiers::SHIFT)]);
        assert_eq!(value.get(), 0.491);
        frame(vec![key(egui::Key::ArrowRight, egui::Modifiers::ALT)]);
        assert_eq!(value.get(), 0.4911);
        frame(vec![key(egui::Key::Delete, egui::Modifiers::NONE)]);
        assert_eq!(value.get(), 0.2);
    }

    #[test]
    fn dragging_moves_a_slider_a_quarter_as_far_as_the_pointer() {
        let ctx = egui::Context::default();
        let value = std::cell::Cell::new(0.5f32);
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let width = std::cell::Cell::new(0f32);
        let frame = |mut events: Vec<egui::Event>, modifiers| {
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut shown = value.get();
                    let response = Keyed::new(0.0..=1.).show(ui, &mut shown, |s| s);
                    value.set(shown);
                    rect.set(response.rect);
                    width.set(ui.spacing().slider_width);
                });
            });
            // No renderer takes the font atlas in a test, so its upload is dropped deliberately.
            output.textures_delta.clear();
        };
        let none = egui::Modifiers::NONE;
        let button = |pos, pressed, modifiers| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(vec![], none);
        let start = rect.get().left_center() + egui::vec2(width.get() / 2., 0.);
        frame(vec![egui::Event::PointerMoved(start)], none);
        frame(vec![button(start, true, none)], none);
        // The press jumps to the pointer, about halfway along.
        let pressed = value.get();
        assert!((pressed - 0.5).abs() < 0.1, "{pressed}");
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        let along = |fraction: f32| start + egui::vec2(width.get() * fraction, 0.);
        frame(vec![egui::Event::PointerMoved(along(0.2))], none);
        assert!(close(value.get(), pressed + 0.05), "{}", value.get());
        // Shift moves as far as the pointer, from where the slider is.
        frame(
            vec![egui::Event::PointerMoved(along(0.3))],
            egui::Modifiers::SHIFT,
        );
        assert!(close(value.get(), pressed + 0.15), "{}", value.get());
        // Alt a tenth as far as without it.
        frame(
            vec![egui::Event::PointerMoved(along(0.7))],
            egui::Modifiers::ALT,
        );
        assert!(close(value.get(), pressed + 0.16), "{}", value.get());
        frame(vec![button(along(0.7), false, none)], none);
        assert!(close(value.get(), pressed + 0.16), "{}", value.get());
    }

    #[test]
    fn a_choice_shows_the_chosen_name_on_the_closed_box() {
        let ctx = egui::Context::default();
        let mut value = 2;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let changed = choice(ui, "Number", &mut value, &[(1, "One"), (2, "Two")]);
                assert!(!changed);
            });
        });
        let texts: Vec<String> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.text().to_owned()),
                _ => None,
            })
            .collect();
        assert!(texts.iter().any(|text| text == "Two"), "{texts:?}");
        // No renderer takes the font atlas in a test, so its upload is dropped deliberately.
        output.textures_delta.clear();
    }

    #[test]
    fn reset_tooltips_show_values_as_written() {
        assert_eq!(format_value(0.25), "0.25");
        assert_eq!(format_value(50.), "50");
        assert_eq!(format_value(-0.115), "-0.115");
        assert_eq!(format_value(8. / 7.), "1.143");
    }
}
