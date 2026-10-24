//! Small widgets shared by the settings panel and the preview.
use crate::{meter, sound};
use crtsim_core::{config::Config, settings};
use eframe::{egui, egui::Rect, emath};
use std::{collections::HashSet, ops::RangeInclusive};

/// Which settings the panel shows, by their keys in `settings::SETTINGS`: every one, or, with
/// Changed only on, the ones that differ from the preset the panel names. Also which differ,
/// whether shown alone or not, for the lamps on the panel's sections.
pub(crate) struct Shown {
    only: Option<HashSet<&'static str>>,
    differ: HashSet<&'static str>,
}

impl Shown {
    pub fn all() -> Self {
        Self {
            only: None,
            differ: HashSet::new(),
        }
    }
    pub fn only(keys: HashSet<&'static str>) -> Self {
        Self {
            only: Some(keys),
            differ: HashSet::new(),
        }
    }
    /// The same, knowing that `keys` differ from the preset.
    pub fn differing(self, keys: HashSet<&'static str>) -> Self {
        Self {
            differ: keys,
            ..self
        }
    }
    /// Whether only changed settings show.
    pub fn changes(&self) -> bool {
        self.only.is_some()
    }
    /// Whether only changed settings show, and none has changed.
    pub fn none(&self) -> bool {
        self.only.as_ref().is_some_and(HashSet::is_empty)
    }
    pub fn key(&self, key: &str) -> bool {
        self.only.as_ref().is_none_or(|keys| keys.contains(key))
    }
    /// Whether any of `keys`, or of the numeric settings of `sections`, shows: a section of the
    /// panel holding them is shown when one does.
    pub fn any(&self, keys: &[&str], sections: &[settings::Section]) -> bool {
        any_of(keys, sections, |key| self.key(key))
    }
    /// Whether any of `keys`, or of the numeric settings of `sections`, differs from the preset.
    pub fn differs(&self, keys: &[&str], sections: &[settings::Section]) -> bool {
        any_of(keys, sections, |key| self.differ.contains(key))
    }
}

/// Whether `test` holds for any of `keys`, or of the numeric settings of `sections`.
fn any_of(keys: &[&str], sections: &[settings::Section], test: impl Fn(&str) -> bool) -> bool {
    keys.iter().any(|key| test(key))
        || settings::SETTINGS.iter().any(|setting| {
            setting
                .numbers
                .as_ref()
                .is_some_and(|n| sections.contains(&n.section))
                && test(setting.key)
        })
}

/// Notes that hold whatever the settings are, behind a hover rather than always on screen.
pub(crate) fn notes(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.small("ℹ Notes").on_hover_text(text)
}

/// The numeric settings of one section of the panel that `shown` shows, in the order
/// `settings::SETTINGS` lists them.
pub(crate) fn numbers(
    ui: &mut egui::Ui,
    config: &mut Config,
    defaults: &Config,
    section: settings::Section,
    shown: &Shown,
) {
    for setting in settings::SETTINGS {
        let Some(numbers) = setting.numbers.as_ref().filter(|n| n.section == section) else {
            continue;
        };
        if !shown.key(setting.key) {
            continue;
        }
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
            let from = Keyed::new(range.clone())
                .logarithmic(logarithmic)
                .fraction(*value);
            sound::cue(ui.ctx(), sound::Sound::Reset { from });
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

impl Pace {
    /// The pace for the modifiers held: a plain one matches any.
    fn held(modifiers: egui::Modifiers) -> &'static Pace {
        PACES
            .iter()
            .find(|pace| modifiers.contains(pace.modifiers))
            .expect("the plain pace matches any modifiers")
    }
}

/// What the keys pressed did to a slider with the keyboard.
enum Keys<N> {
    /// ←/→ stepped it to this.
    Stepped(N),
    /// Delete or Backspace returned it to its default.
    Reset(N),
}

/// Where a slider being dragged along its rail has got to: its value, before it is rounded to
/// show, and the pointer's position along the rail then. Each frame moves on from it at the
/// pace held then, so a whole-number slider gathers movements too small to show. Kept from the
/// press until the slider is let go.
#[derive(Clone, Copy)]
struct Anchor {
    value: f64,
    x: f32,
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
    /// presses. It is drawn as a meter.
    pub fn show(
        self,
        ui: &mut egui::Ui,
        value: &mut N,
        configure: impl for<'v> FnOnce(egui::Slider<'v>) -> egui::Slider<'v>,
    ) -> egui::Response {
        let before = *value;
        // The slider's own id, which its rail takes when added.
        let id = ui.next_auto_id();
        let anchor_id = id.with("drag anchor");
        // The slider is drawn at the value as it stands. What it proposes, from the pointer,
        // its number box or assistive technology, is decided on once it is added.
        let mut proposed = None;
        let (start, end) = self.span();
        let slider = egui::Slider::from_get_set(start..=end, |set| {
            proposed = set.or(proposed);
            before.to_f64()
        })
        .logarithmic(self.logarithmic)
        .handle_shape(meter::handle(thickness(ui)));
        let slider = if N::INTEGRAL {
            slider.integer()
        } else {
            slider
        };
        let mut response = ui.add(configure(slider));
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let (next, reset) = match self.keyed(ui, &response, before) {
            Some(Keys::Reset(default)) => (Some(default), true),
            Some(Keys::Stepped(next)) => (Some(next), false),
            None => (
                self.rail_dragged(ui, &response, anchor_id, proposed)
                    .or(proposed.map(N::from_f64)),
                false,
            ),
        };
        let next = next.filter(|next| *next != before);
        // The rail egui laid out first in the slider's row: its full height, as wide as set.
        let rail = Rect::from_center_size(
            egui::pos2(
                response.rect.left() + ui.spacing().slider_width / 2.,
                response.rect.center().y,
            ),
            egui::vec2(ui.spacing().slider_width, thickness(ui)),
        );
        if let Some(next) = next {
            *value = next;
            response.mark_changed();
            if reset {
                sound::cue(
                    ui.ctx(),
                    sound::Sound::Reset {
                        from: self.fraction(before),
                    },
                );
            } else {
                meter::moved(ui, rail, self.fraction(before), self.reading(next));
            }
        }
        meter::paint(ui, rail, id.with("meter"), self.reading(*value), &response);
        response
    }

    /// How far along the slider `value` is, from 0 to 1, as its meter shows it.
    fn fraction(&self, value: N) -> f32 {
        let (min, max) = self.span();
        let value = value.to_f64();
        let fraction = if self.logarithmic && min > 0. && value > 0. {
            (value.ln() - min.ln()) / (max.ln() - min.ln())
        } else {
            (value - min) / (max - min)
        };
        fraction as f32
    }

    fn reading(&self, value: N) -> meter::Reading {
        meter::Reading {
            fraction: self.fraction(value),
            default: self.default.map(|default| self.fraction(default)),
        }
    }

    /// What the keys pressed make the slider while it has the keyboard: ←/→ step it from
    /// `before`, replacing the pixel's worth the slider itself moves for each, and Delete or
    /// Backspace returns it to its default.
    fn keyed(&self, ui: &egui::Ui, response: &egui::Response, before: N) -> Option<Keys<N>> {
        if !response.has_focus() {
            return None;
        }
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
        match self.default {
            Some(default) if reset => Some(Keys::Reset(default)),
            _ if fraction != 0. => Some(Keys::Stepped(N::from_f64(stepped(
                before.to_f64(),
                self.span(),
                fraction,
                self.logarithmic,
                N::INTEGRAL,
            )))),
            _ => None,
        }
    }

    /// What a drag along the rail makes the slider, in place of the value under the pointer
    /// that the slider proposes. The press itself jumps to the pointer, as `proposed`; the drag
    /// moves on from there at the pace held, for as long as the pointer moves, on or off the
    /// rail. Dragging the number box beside it is the box's own.
    fn rail_dragged(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        anchor_id: egui::Id,
        proposed: Option<f64>,
    ) -> Option<N> {
        // The slider also puts itself under the pointer on the frame it is let go.
        if !response.dragged() && !response.drag_stopped() {
            return None;
        }
        let (origin, pointer, modifiers) = ui.input(|input| {
            let pointer = &input.pointer;
            (
                pointer.press_origin(),
                pointer.interact_pos(),
                input.modifiers,
            )
        });
        let x = pointer?.x;
        // A drag let go where the slider was not drawn leaves its anchor behind.
        let kept = ui.data(|data| data.get_temp::<Anchor>(anchor_id));
        let anchor = match kept.filter(|_| !response.drag_started()) {
            Some(anchor) => anchor,
            // A drag starting: along the rail, from where the press put the slider.
            None => {
                let left = response.rect.left();
                if !(left..=left + ui.spacing().slider_width).contains(&origin?.x) {
                    return None;
                }
                Anchor {
                    value: proposed?,
                    x,
                }
            }
        };
        let fraction = f64::from(x - anchor.x) / travel(ui) * Pace::held(modifiers).drag;
        let value = moved(anchor.value, self.span(), fraction, self.logarithmic);
        ui.data_mut(|data| {
            if response.drag_stopped() {
                data.remove::<Anchor>(anchor_id);
            } else {
                data.insert_temp(anchor_id, Anchor { value, x });
            }
        });
        Some(N::from_f64(tidy(value, N::INTEGRAL)))
    }
}

/// How tall a slider's rail is, as egui lays it out: a line of text, or a button if taller.
fn thickness(ui: &egui::Ui) -> f32 {
    ui.text_style_height(&egui::TextStyle::Body)
        .max(ui.spacing().interact_size.y)
}

/// How far a slider's handle travels from one end to the other: its width, less the meter's
/// inset at each end, where the handle stops.
fn travel(ui: &egui::Ui) -> f64 {
    f64::from(ui.spacing().slider_width - 2. * meter::INSET)
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
/// A switch that turns a setting on or off: a slide in a recessed track, with a lamp beside it
/// that lights while on. To a screen reader it is a checkbox.
pub(crate) struct Switch<'a> {
    on: &'a mut bool,
    text: egui::WidgetText,
}

impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool, text: impl Into<egui::WidgetText>) -> Self {
        Self {
            on,
            text: text.into(),
        }
    }
}

impl egui::Widget for Switch<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let track = egui::vec2(30., 16.);
        let lamp = 7.;
        let spacing = ui.spacing().icon_spacing + 2.;
        let galley = self.text.into_galley(
            ui,
            None,
            ui.available_width() - track.x - lamp - 2. * spacing,
            egui::TextStyle::Button,
        );
        let size = egui::vec2(
            track.x + lamp + 2. * spacing + galley.size().x,
            galley
                .size()
                .y
                .max(track.y)
                .max(ui.spacing().interact_size.y),
        );
        let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let (on, enabled, text) = (*self.on, ui.is_enabled(), galley.text().to_owned());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, &text)
        });
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let glide = crate::interface::effects(ui.ctx()).glide();
        let along = ui.ctx().animate_bool_with_time(response.id, on, glide);
        let visuals = ui.style().interact(&response);
        let accent = ui.visuals().selection.bg_fill;
        let border = ui.visuals().widgets.inactive.bg_stroke.color;
        let well = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.center().y - track.y / 2.),
            track,
        );
        let painter = ui.painter();
        let round = well.height() / 2.;
        painter.rect(
            well,
            round,
            ui.visuals().extreme_bg_color,
            if response.hovered() || response.has_focus() {
                egui::Stroke::new(1.5, accent)
            } else {
                egui::Stroke::new(1., border)
            },
            egui::StrokeKind::Inside,
        );
        let knob_size = well.height() - 4.;
        let knob = egui::Rect::from_center_size(
            egui::pos2(
                egui::lerp(
                    well.left() + 2. + knob_size / 2. ..=well.right() - 2. - knob_size / 2.,
                    along,
                ),
                well.center().y,
            ),
            egui::Vec2::splat(knob_size + visuals.expansion.min(0.) * 2.),
        );
        painter.circle(
            knob.center(),
            knob.width() / 2.,
            meter::blend(ui.visuals().widgets.inactive.bg_fill, accent, along),
            egui::Stroke::new(1., border),
        );
        // A highlight on the knob, as on a moulded switch.
        painter.circle_filled(
            knob.center() - egui::vec2(1.5, 1.5),
            knob.width() / 5.,
            egui::Color32::from_white_alpha(110),
        );
        let lamp_center = egui::pos2(well.right() + spacing + lamp / 2., rect.center().y);
        lamp_light(painter, lamp_center, lamp / 2., along, accent, ui.visuals());
        let text_pos = egui::pos2(
            lamp_center.x + lamp / 2. + spacing,
            rect.center().y - galley.size().y / 2.,
        );
        painter.galley(text_pos, galley, visuals.text_color());
        response
    }
}

/// A round lamp, lit `how` much in `color`, glowing when lit.
pub(crate) fn lamp_light(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    how: f32,
    color: egui::Color32,
    visuals: &egui::Visuals,
) {
    let border = visuals.widgets.inactive.bg_stroke.color;
    let dark = meter::blend(visuals.extreme_bg_color, border, 0.35);
    if how > 0. {
        painter.circle_filled(center, radius + 2.5, color.gamma_multiply(0.3 * how));
    }
    painter.circle(
        center,
        radius,
        meter::blend(dark, color, how),
        egui::Stroke::new(1., border),
    );
}

/// [`Switch`] in place of a checkbox.
pub(crate) trait Switches {
    fn switch(&mut self, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response;
}

impl Switches for egui::Ui {
    fn switch(&mut self, on: &mut bool, text: impl Into<egui::WidgetText>) -> egui::Response {
        self.add(Switch::new(on, text))
    }
}

/// A row of channels, one chosen, with a lit block that slides to the one chosen, as a set's
/// channel selector. Whether the choice changed.
pub(crate) fn channels<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let padding = egui::vec2(10., 4.);
    let galleys: Vec<_> = options
        .iter()
        .map(|(_, name)| {
            egui::WidgetText::from(*name).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            )
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|g| g.size().x + 2. * padding.x)
        .collect();
    let height = ui.spacing().interact_size.y;
    let size = egui::vec2(widths.iter().sum::<f32>() + 4., height);
    let id = ui.next_auto_id();
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let mut changed = false;
    let mut left = rect.left() + 2.;
    let mut segments = vec![];
    for (index, ((option, name), width)) in options.iter().zip(&widths).enumerate() {
        let segment = egui::Rect::from_min_size(
            egui::pos2(left, rect.top() + 2.),
            egui::vec2(*width, height - 4.),
        );
        let response = ui.interact(segment, id.with(index), egui::Sense::click());
        if response.clicked() && *value != *option {
            *value = option.clone();
            changed = true;
            sound::cue(ui.ctx(), sound::Sound::Channel);
        }
        let selected = *value == *option;
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, *name)
        });
        segments.push((segment, response));
        left += width;
    }
    if !ui.is_rect_visible(rect) {
        return changed;
    }
    let visuals = ui.visuals();
    let accent = visuals.selection.bg_fill;
    let painter = ui.painter();
    painter.rect(
        rect,
        4.,
        visuals.extreme_bg_color,
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let chosen = options
        .iter()
        .position(|(option, _)| option == value)
        .unwrap_or(0);
    // Where the lamp is, from the row's left, so moving the row does not slide it.
    let glide = crate::interface::effects(ui.ctx()).glide() * 2.;
    let target = segments[chosen].0;
    let x = ui
        .ctx()
        .animate_value_with_time(id.with("x"), target.left() - rect.left(), glide);
    let width = ui
        .ctx()
        .animate_value_with_time(id.with("width"), target.width(), glide);
    let lamp = egui::Rect::from_min_size(
        egui::pos2(rect.left() + x, target.top()),
        egui::vec2(width, target.height()),
    );
    painter.rect_filled(lamp.expand(2.), 5., accent.gamma_multiply(0.2));
    painter.rect(
        lamp,
        3.,
        visuals.widgets.active.bg_fill,
        egui::Stroke::new(1., accent),
        egui::StrokeKind::Inside,
    );
    for (index, ((segment, response), galley)) in segments.iter().zip(galleys).enumerate() {
        let color = if index == chosen || response.hovered() {
            visuals.strong_text_color()
        } else {
            visuals.weak_text_color()
        };
        let pos = segment.center() - galley.size() / 2.;
        painter.galley(pos, galley, color);
        if response.has_focus() {
            painter.rect_stroke(
                *segment,
                3.,
                egui::Stroke::new(1.5, accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    changed
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
/// Draws `im` to fit `available`, or at `zoom`, and says where.
pub(crate) fn show_image(
    ui: &mut egui::Ui,
    im: egui::load::SizedTexture,
    available: egui::Vec2,
    fit: bool,
    zoom: f32,
) -> egui::Rect {
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
    rect
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
        let travel_px = std::cell::Cell::new(0f32);
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
                    travel_px.set(travel(ui) as f32);
                });
            });
            // No renderer takes the font atlas in a test, so its upload is dropped deliberately.
            output.textures_delta.clear();
            // The numbers drawn: the slider's number box.
            output
                .shapes
                .iter()
                .filter_map(|clipped| match &clipped.shape {
                    egui::epaint::Shape::Text(text) => text.galley.text().parse::<f32>().ok(),
                    _ => None,
                })
                .collect::<Vec<_>>()
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
        let along = |fraction: f32| start + egui::vec2(travel_px.get() * fraction, 0.);
        frame(vec![egui::Event::PointerMoved(along(0.2))], none);
        assert!(close(value.get(), pressed + 0.05), "{}", value.get());
        // The slider shows where it is, not where the pointer is.
        let drawn = frame(vec![], none);
        assert!(
            drawn
                .iter()
                .any(|number| (number - value.get()).abs() < 0.01),
            "{drawn:?}"
        );
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
