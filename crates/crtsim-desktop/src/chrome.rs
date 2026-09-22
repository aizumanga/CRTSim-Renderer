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

/// Where a tool window sits on screen, remembered between runs.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Placement {
    /// Outer position, including decorations, as `with_position` expects.
    pub position: [f32; 2],
    /// Inner size, excluding decorations, as `with_inner_size` expects.
    pub size: [f32; 2],
}

/// Remembered geometry and behavior for one tool window.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ToolWindow {
    /// Applied when the window opens, then left alone. egui turns a changed builder value
    /// into a move/resize command, so rewriting this every frame would fight the user
    /// dragging or resizing the window.
    pub placement: Option<Placement>,
    /// Keep the window above the main one.
    pub on_top: bool,
    /// Where the window actually is now; folded into `placement` only when saving.
    /// Only `tool_window` should write this: feeding it back in while the window is open
    /// would make the builder fight the drag it just observed.
    #[serde(skip)]
    pub(crate) live: Option<Placement>,
}
impl ToolWindow {
    /// The state worth writing to disk: where the window ended up, not where it opened.
    pub fn to_save(self) -> Self {
        Self {
            placement: self.live.or(self.placement),
            on_top: self.on_top,
            live: None,
        }
    }
}

/// Shows a tool window (gallery, etc.) as its own native window so it can be moved anywhere,
/// including beside or outside the app, keeping the preview unobstructed. `window` carries the
/// remembered placement and keep-on-top choice in, and the live placement back out.
/// If viewports are embedded (unsupported platform or screenshot smoke tests), it falls back to
/// an in-app window that may cover the whole app instead of only the preview area.
/// Returns `false` once the user closes it.
pub fn tool_window(
    ctx: &egui::Context,
    title: &str,
    default_size: [f32; 2],
    window: &mut ToolWindow,
    contents: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size(window.placement.map_or(default_size, |p| p.size))
        .with_min_inner_size([320., 240.])
        .with_window_level(if window.on_top {
            egui::WindowLevel::AlwaysOnTop
        } else {
            egui::WindowLevel::Normal
        });
    if let Some(placement) = window.placement {
        builder = builder.with_position(placement.position);
    }
    let mut on_top = window.on_top;
    let mut live = None;
    let open = ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of(title),
        builder,
        |ctx, class| {
            if class == egui::ViewportClass::Embedded {
                // An in-app window already floats above the panels, so keep-on-top has
                // nothing to do and no native geometry to remember.
                let mut open = true;
                egui::Window::new(title)
                    .open(&mut open)
                    .default_size(default_size)
                    .constrain_to(ctx.screen_rect())
                    .show(ctx, contents);
                open
            } else {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.checkbox(&mut on_top, "Keep on top")
                        .on_hover_text("Keep this window above the main window");
                    ui.separator();
                    contents(ui);
                });
                live = ctx.input(|i| {
                    let info = i.viewport();
                    Some(Placement {
                        position: info.outer_rect?.min.into(),
                        size: info.inner_rect?.size().into(),
                    })
                });
                !ctx.input(|i| i.viewport().close_requested())
            }
        },
    );
    window.on_top = on_top;
    if live.is_some() {
        window.live = live;
    }
    if !open {
        // Folding the live geometry in on close means reopening the window in this same
        // session brings it back to where it was, not to where it first appeared.
        *window = window.to_save();
    }
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_placement_is_where_the_window_ended_up() {
        let opened_at = Placement {
            position: [10., 20.],
            size: [600., 500.],
        };
        let dragged_to = Placement {
            position: [700., 300.],
            size: [640., 480.],
        };
        // Nothing to remember yet: the window has never been placed.
        assert_eq!(ToolWindow::default().to_save().placement, None);
        // Opened but never moved: keep what it opened with.
        let untouched = ToolWindow {
            placement: Some(opened_at),
            on_top: true,
            live: None,
        };
        assert_eq!(untouched.to_save().placement, Some(opened_at));
        assert!(untouched.to_save().on_top);
        // Moved: the live geometry wins, and is not fed back as an opening placement.
        let moved = ToolWindow {
            placement: Some(opened_at),
            on_top: false,
            live: Some(dragged_to),
        };
        assert_eq!(moved.to_save().placement, Some(dragged_to));
        assert_eq!(moved.to_save().live, None);
    }
}
