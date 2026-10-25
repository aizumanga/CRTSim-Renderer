//! The web app's power button. A browser plays no sound before the person's first click, so with
//! sounds on the web app starts as a computer does, at its power button; pressing it boots the
//! interface, and the screen switches on with its sound.
use crate::meter::blend;
use eframe::egui::{self, Color32, Stroke};

/// The power button, alone in the middle of the window, as on the front of a computer. Whether it
/// was pressed: by a click or a touch, or by Enter or Space, as it has the keyboard from the
/// start.
pub fn power_button(ui: &mut egui::Ui) -> bool {
    let visuals = ui.visuals().clone();
    let accent = visuals.selection.bg_fill;
    let area = ui.max_rect();
    let painter = ui.painter().clone();
    crate::chrome::gradient(&painter, area, visuals.panel_fill, visuals.extreme_bg_color);

    // The computer's front: a plate with the button in it.
    let plate = egui::Rect::from_center_size(area.center(), egui::vec2(260., 300.));
    painter.rect(
        plate,
        14.,
        visuals.faint_bg_color,
        visuals.window_stroke,
        egui::StrokeKind::Inside,
    );
    crate::chrome::bevel(ui, plate.shrink(1.));
    painter.text(
        plate.center_top() + egui::vec2(0., 34.),
        egui::Align2::CENTER_CENTER,
        "CRTSim Renderer",
        egui::FontId::proportional(20.),
        visuals.strong_text_color(),
    );

    // A disk drive's slot, under the name.
    let slot = egui::Rect::from_center_size(
        plate.center_top() + egui::vec2(0., 70.),
        egui::vec2(150., 7.),
    );
    painter.rect(
        slot,
        3.,
        visuals.extreme_bg_color,
        Stroke::new(1., visuals.window_stroke.color),
        egui::StrokeKind::Inside,
    );

    let center = plate.center() + egui::vec2(0., 6.);
    let radius = 40.;
    let rect = egui::Rect::from_center_size(center, egui::Vec2::splat(2. * radius));
    let response = ui
        .interact(rect, ui.id().with("power button"), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Power"));
    if ui.memory(|memory| memory.focused().is_none()) {
        response.request_focus();
    }
    let held = response.is_pointer_button_down_on();
    let lit = response.hovered() || response.has_focus();
    // The button sinks while held.
    let face = center + egui::vec2(0., if held { 2. } else { 0. });
    if lit {
        painter.circle_filled(center, radius + 7., accent.gamma_multiply(0.25));
    }
    painter.circle_filled(
        center + egui::vec2(0., 3.),
        radius,
        Color32::from_black_alpha(70),
    );
    painter.circle(
        face,
        radius,
        if held {
            visuals.widgets.active.bg_fill
        } else {
            visuals.widgets.inactive.bg_fill
        },
        Stroke::new(
            if lit { 2. } else { 1. },
            if lit {
                accent
            } else {
                visuals.window_stroke.color
            },
        ),
    );
    // A highlight across the top of the button, as on moulded plastic.
    painter.circle_filled(
        face - egui::vec2(0., radius * 0.35),
        radius * 0.55,
        Color32::from_white_alpha(if visuals.dark_mode { 10 } else { 40 }),
    );
    power_symbol(
        &painter,
        face,
        radius * 0.45,
        if lit { accent } else { visuals.text_color() },
    );

    // The power lamp, dark until pressed.
    let lamp = plate.center_bottom() - egui::vec2(0., 74.);
    painter.circle(
        lamp,
        4.,
        blend(visuals.extreme_bg_color, visuals.window_stroke.color, 0.4),
        Stroke::new(1., visuals.window_stroke.color),
    );
    painter.text(
        plate.center_bottom() - egui::vec2(0., 46.),
        egui::Align2::CENTER_CENTER,
        "Press to switch on",
        egui::FontId::proportional(15.),
        visuals.text_color(),
    );
    painter.text(
        plate.center_bottom() - egui::vec2(0., 24.),
        egui::Align2::CENTER_CENTER,
        "Sounds can be turned off in the settings",
        egui::FontId::proportional(11.),
        visuals.weak_text_color(),
    );
    response.clicked()
}

/// The power symbol: a circle broken at the top, with a line down through the break.
fn power_symbol(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let stroke = Stroke::new(radius * 0.22, color);
    let points: Vec<egui::Pos2> = (0..=40)
        .map(|i| {
            // From 60° to 300° past the top, clockwise.
            let angle = (60. + 240. * i as f32 / 40.).to_radians();
            center + radius * egui::vec2(angle.sin(), -angle.cos())
        })
        .collect();
    painter.add(egui::Shape::line(points, stroke));
    painter.line_segment(
        [
            center - egui::vec2(0., radius * 1.15),
            center - egui::vec2(0., radius * 0.15),
        ],
        stroke,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_presses_the_power_button_it_has_the_keyboard() {
        let ctx = egui::Context::default();
        let frame = |events: Vec<egui::Event>| {
            let mut pressed = false;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| pressed = power_button(ui));
                },
            );
            // No renderer takes the font atlas in a test, so its upload is dropped deliberately.
            output.textures_delta.clear();
            pressed
        };
        assert!(!frame(vec![]));
        let enter = egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(frame(vec![enter]));
    }
}
