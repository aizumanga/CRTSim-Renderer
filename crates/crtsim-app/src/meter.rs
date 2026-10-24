//! A slider drawn as a meter: a recessed well of cells that light up to its value, with a mark
//! at its default, in the style of the theme. egui's own slider still does the work beneath it,
//! from the pointer and the keyboard to the number box and screen readers; the meter is drawn
//! over its rail and handle.
use crate::sound::{self, Sound};
use eframe::egui::{self, Color32, Rect, Stroke};

/// How a theme draws its meters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Skin {
    /// Glowing cells, as a VU meter's LEDs, warming to amber at the top.
    Led,
    /// One flat bar.
    Flat,
    /// Glossy green blocks, as an early-2000s progress bar.
    Glossy,
    /// Striped cells in grey chrome.
    Striped,
    /// A rounded pill with a star where the value is.
    #[default]
    Pill,
}

fn skin_id() -> egui::Id {
    egui::Id::new("crtsim meter skin")
}

/// Draws meters in `skin` from the next frame.
pub fn set_skin(ctx: &egui::Context, skin: Skin) {
    ctx.data_mut(|data| data.insert_temp(skin_id(), skin));
}

fn skin(ctx: &egui::Context) -> Skin {
    ctx.data(|data| data.get_temp(skin_id()))
        .unwrap_or_default()
}

/// How far into the rail, at each end, the cells begin. egui's handle, which the meter hides,
/// travels this far in from the ends too.
pub const INSET: f32 = 2.;
/// How far apart the cells are, at most.
const PITCH: f32 = 6.;
/// The amber the top cells of an LED meter light.
const HOT: Color32 = Color32::from_rgb(255, 199, 90);

/// The handle egui is asked to draw: as narrow as the meter's inset, so the handle travels the
/// same span as the cells, and the meter covers it.
pub fn handle(thickness: f32) -> egui::style::HandleShape {
    // egui's handle is a fifth of the rail's thickness either side of its middle, by this ratio.
    egui::style::HandleShape::Rect {
        aspect_ratio: INSET / (thickness / 2.5),
    }
}

/// How many cells a meter `width` wide holds.
fn cells(width: f32) -> usize {
    ((width - 2. * INSET) / PITCH).floor().clamp(4., 64.) as usize
}

/// How many of `cells` are lit `fraction` of the way along.
fn lit(fraction: f32, cells: usize) -> usize {
    (fraction.clamp(0., 1.) * cells as f32).round() as usize
}

/// How a meter stands this frame.
pub struct Reading {
    /// How far along the value is, from 0 to 1; beyond either end, a typed value outside the
    /// range.
    pub fraction: f32,
    /// How far along the default is, if the meter has one.
    pub default: Option<f32>,
}

/// Cues the sound of a meter moving from `before` to `after`, by a person's hand: reaching an
/// end, passing its default, or lighting or clearing a cell.
pub fn moved(ui: &egui::Ui, rail: Rect, before: f32, after: Reading) {
    let at = after.fraction;
    let sound = if (at >= 1. && before < 1.) || (at <= 0. && before > 0.) {
        Some(Sound::EndStop)
    } else if after
        .default
        .is_some_and(|default| (before - default) * (at - default) < 0. || at == default)
    {
        Some(Sound::Detent)
    } else {
        let cells = cells(rail.width());
        (lit(before, cells) != lit(at, cells)).then_some(Sound::Tick { at })
    };
    if let Some(sound) = sound {
        sound::cue(ui.ctx(), sound);
    }
}

/// Draws the meter over a slider's `rail`, in the theme's skin, easing to where it now stands.
pub fn paint(ui: &egui::Ui, rail: Rect, id: egui::Id, reading: Reading, response: &egui::Response) {
    if !ui.is_rect_visible(rail) {
        return;
    }
    let glide = crate::interface::effects(ui.ctx()).glide();
    let shown = ui
        .ctx()
        .animate_value_with_time(id, reading.fraction.clamp(0., 1.), glide);
    let skin = skin(ui.ctx());
    let visuals = ui.visuals();
    let accent = visuals.selection.bg_fill;
    let painter = ui.painter();
    // A little wider than the rail, to cover egui's handle at either end.
    let well = rail
        .expand2(egui::vec2(1.5, 0.))
        .shrink2(egui::vec2(0., 1.));
    let round = match skin {
        Skin::Pill => well.height() / 2.,
        Skin::Glossy => 4.,
        Skin::Led => 2.,
        Skin::Flat | Skin::Striped => 0.,
    };
    let border = if response.has_focus() || response.dragged() || response.hovered() {
        Stroke::new(1.5, accent)
    } else {
        visuals.widgets.inactive.bg_stroke
    };
    painter.rect(
        well,
        round,
        visuals.extreme_bg_color,
        border,
        egui::StrokeKind::Inside,
    );
    if skin == Skin::Striped {
        // Platinum's well is sunk into the panel: dark above and to the left, light below.
        let shade = Stroke::new(1., visuals.widgets.inactive.bg_stroke.color);
        painter.line_segment([well.left_top(), well.right_top()], shade);
        painter.line_segment([well.left_top(), well.left_bottom()], shade);
    }

    let span = Rect::from_x_y_ranges(
        rail.left() + INSET..=rail.right() - INSET,
        well.top() + 3.5..=well.bottom() - 3.5,
    );
    let count = cells(rail.width());
    let lit = lit(shown, count);
    let gap = match skin {
        Skin::Flat | Skin::Pill => 0.,
        Skin::Led | Skin::Glossy | Skin::Striped => 1.5,
    };
    let width = span.width() / count as f32;
    let off = blend(
        visuals.extreme_bg_color,
        visuals.widgets.inactive.bg_stroke.color,
        0.3,
    );
    let lively = response.hovered() || response.dragged();
    for index in 0..count {
        let x = span.left() + index as f32 * width;
        let cell = Rect::from_x_y_ranges(x..=x + width - gap, span.y_range());
        let corner = match skin {
            Skin::Pill if index == 0 => egui::CornerRadius {
                nw: 255,
                sw: 255,
                ..Default::default()
            },
            Skin::Pill if index + 1 == count => egui::CornerRadius {
                ne: 255,
                se: 255,
                ..Default::default()
            },
            Skin::Led => egui::CornerRadius::same(1),
            _ => egui::CornerRadius::ZERO,
        };
        if index >= lit {
            painter.rect_filled(cell, corner, off);
            continue;
        }
        let mut color = match skin {
            Skin::Led if index + 4 >= count => HOT,
            Skin::Glossy => Color32::from_rgb(63, 168, 54),
            Skin::Striped if index % 2 == 1 => blend(accent, Color32::WHITE, 0.25),
            _ => accent,
        };
        let head = index + 1 == lit;
        if head && lively {
            color = blend(color, Color32::WHITE, 0.35);
        }
        if skin == Skin::Led {
            // A soft glow around each lit cell.
            painter.rect_filled(cell.expand(1.5), corner, color.gamma_multiply(0.25));
        }
        painter.rect_filled(cell, corner, color);
        if matches!(skin, Skin::Glossy | Skin::Pill) {
            // A highlight across the top half, as on glass.
            let shine = Rect::from_min_max(cell.min, egui::pos2(cell.max.x, cell.center().y));
            let top_corner = egui::CornerRadius {
                sw: 0,
                se: 0,
                ..corner
            };
            painter.rect_filled(shine, top_corner, Color32::from_white_alpha(70));
        }
    }
    if let Some(default) = reading.default.filter(|d| (0. ..=1.).contains(d)) {
        let x = span.left() + default * span.width();
        let color = visuals.text_color().gamma_multiply(0.8);
        painter.line_segment(
            [
                egui::pos2(x, well.top() - 1.),
                egui::pos2(x, well.bottom() + 1.),
            ],
            Stroke::new(1.5, color),
        );
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - 3., well.top() - 2.),
                egui::pos2(x + 3., well.top() - 2.),
                egui::pos2(x, well.top() + 2.),
            ],
            color,
            Stroke::NONE,
        ));
    }
    if skin == Skin::Pill && lit > 0 {
        let x = span.left() + lit as f32 * width;
        star(
            painter,
            egui::pos2(x, span.center().y),
            well.height() * 0.42,
            accent,
        );
    }
}

/// A five-pointed white star outlined in `edge`, as Sky Diary's meters mark their value.
fn star(painter: &egui::Painter, center: egui::Pos2, radius: f32, edge: Color32) {
    let points: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let r = if i % 2 == 0 { radius } else { radius * 0.45 };
            let angle = std::f32::consts::PI * (i as f32 / 5. - 0.5);
            center + r * egui::vec2(angle.cos(), angle.sin())
        })
        .collect();
    // Each point's triangle from the middle, as a star is not convex.
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, Color32::WHITE);
    for point in &points {
        mesh.colored_vertex(*point, Color32::WHITE);
    }
    for i in 0..10u32 {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
    }
    painter.add(egui::Shape::mesh(mesh));
    let mut outline = points;
    outline.push(outline[0]);
    painter.add(egui::Shape::line(outline, Stroke::new(1., edge)));
}

/// `a` mixed `t` of the way to `b`.
pub fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_lights_cells_in_proportion_and_no_more_than_it_has() {
        let count = cells(120.);
        assert_eq!(count, 19);
        assert_eq!(lit(0., count), 0);
        assert_eq!(lit(0.5, count), 10);
        assert_eq!(lit(1., count), count);
        // A typed value beyond the range lights every cell, or none.
        assert_eq!(lit(3., count), count);
        assert_eq!(lit(-1., count), 0);
    }

    fn sound_of(before: f32, after: f32, default: Option<f32>) -> Vec<Sound> {
        let ctx = egui::Context::default();
        let mut sounds = vec![];
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let rail = Rect::from_min_size(egui::pos2(0., 0.), egui::vec2(120., 25.));
            let reading = Reading {
                fraction: after,
                default,
            };
            moved(ui, rail, before, reading);
            sounds = sound::take(ui.ctx());
        });
        output.textures_delta.clear();
        sounds
    }

    #[test]
    fn a_meter_sounds_its_cells_its_default_and_its_ends() {
        // Within a cell: nothing.
        assert_eq!(sound_of(0.50, 0.51, None), vec![]);
        assert_eq!(sound_of(0.50, 0.60, None), vec![Sound::Tick { at: 0.60 }]);
        assert_eq!(sound_of(0.30, 0.60, Some(0.5)), vec![Sound::Detent]);
        assert_eq!(sound_of(0.90, 1.00, Some(0.5)), vec![Sound::EndStop]);
        // Pushed on against an end it already reached: no more thuds.
        assert_eq!(sound_of(1.00, 1.00, Some(0.5)), vec![]);
        assert_eq!(sound_of(0.05, 0.00, None), vec![Sound::EndStop]);
    }

    #[test]
    fn blending_reaches_both_ends() {
        let (a, b) = (
            Color32::from_rgb(0, 100, 200),
            Color32::from_rgb(200, 100, 0),
        );
        assert_eq!(blend(a, b, 0.), a);
        assert_eq!(blend(a, b, 1.), b);
        assert_eq!(blend(a, b, 0.5), Color32::from_rgb(100, 100, 100));
    }
}
