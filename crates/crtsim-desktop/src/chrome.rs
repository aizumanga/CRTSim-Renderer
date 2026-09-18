//! Native workbench chrome; all decoration stays outside rendered media.
use eframe::egui::{self, Color32, Rect, Stroke};

pub fn gradient(painter: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    for (pos, color) in [
        (rect.left_top(), top),
        (rect.right_top(), top),
        (rect.right_bottom(), bottom),
        (rect.left_bottom(), bottom),
    ] {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}
pub fn bevel(ui: &egui::Ui, rect: Rect) {
    let light = if ui.visuals().dark_mode {
        Color32::from_white_alpha(45)
    } else {
        Color32::from_white_alpha(180)
    };
    let shadow = Color32::from_black_alpha(65);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.left_top()],
        Stroke::new(1.0_f32, light),
    );
    ui.painter().line_segment(
        [rect.left_top(), rect.right_top()],
        Stroke::new(1.0_f32, light),
    );
    ui.painter().line_segment(
        [rect.right_top(), rect.right_bottom()],
        Stroke::new(1.0_f32, shadow),
    );
    ui.painter().line_segment(
        [rect.right_bottom(), rect.left_bottom()],
        Stroke::new(1.0_f32, shadow),
    );
}
pub fn monitor(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(25., 23.), egui::Sense::hover());
    let screen = Rect::from_min_size(r.min, egui::vec2(24., 17.));
    ui.painter().rect(
        screen,
        3.,
        Color32::from_rgb(23, 51, 80),
        Stroke::new(1.0_f32, ui.visuals().text_color()),
    );
    gradient(
        ui.painter(),
        screen.shrink(3.),
        Color32::from_rgb(164, 212, 239),
        Color32::from_rgb(71, 121, 169),
    );
    ui.painter().line_segment(
        [r.min + egui::vec2(12., 17.), r.min + egui::vec2(12., 22.)],
        Stroke::new(2.0_f32, ui.visuals().text_color()),
    );
    ui.painter().line_segment(
        [r.min + egui::vec2(6., 22.), r.min + egui::vec2(18., 22.)],
        Stroke::new(2.0_f32, ui.visuals().text_color()),
    );
}
pub fn status_light(ui: &mut egui::Ui, busy: bool, error: bool) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(12., 16.), egui::Sense::hover());
    let color = if error {
        Color32::from_rgb(235, 105, 120)
    } else if busy {
        Color32::from_rgb(234, 189, 96)
    } else {
        Color32::from_rgb(90, 216, 153)
    };
    ui.painter().circle(
        r.center(),
        5.0_f32,
        color,
        Stroke::new(1.0_f32, Color32::from_black_alpha(100)),
    );
    ui.painter().circle_filled(
        r.center() + egui::vec2(-1.5, -1.5),
        1.5_f32,
        Color32::from_white_alpha(150),
    );
}

pub struct Section<'a> {
    title: &'a str,
    open: bool,
}
impl<'a> Section<'a> {
    pub fn new(title: &'a str) -> Self {
        Self { title, open: false }
    }
    pub fn default_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
    pub fn show<R>(self, ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui) -> R) {
        let response = egui::Frame::none()
            .fill(ui.visuals().faint_bg_color)
            .stroke(ui.visuals().window_stroke)
            .rounding(3.)
            .inner_margin(7.)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::CollapsingHeader::new(egui::RichText::new(self.title).strong().size(14.))
                    .show_background(true)
                    .default_open(self.open)
                    .show(ui, contents);
            });
        bevel(ui, response.response.rect);
    }
}

pub fn preview_style(ui: &mut egui::Ui) {
    // The monitor surround stays dark in every theme, independently of the media.
    let v = ui.visuals_mut();
    v.override_text_color = Some(Color32::from_rgb(220, 233, 246));
    v.extreme_bg_color = Color32::from_rgb(12, 29, 47);
    v.faint_bg_color = Color32::from_rgb(25, 48, 74);
    v.widgets.noninteractive.fg_stroke.color = Color32::from_rgb(207, 225, 242);
    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.bg_fill = Color32::from_rgb(39, 76, 113);
        w.weak_bg_fill = w.bg_fill;
        w.fg_stroke.color = Color32::from_rgb(232, 241, 250);
        w.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(83, 126, 168));
    }
    v.selection.bg_fill = Color32::from_rgb(139, 78, 112);
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
}
