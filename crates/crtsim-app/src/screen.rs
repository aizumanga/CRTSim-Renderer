//! The preview's screen behaving as a set's: switching on when new media first shows, and a
//! burst of static when the channel or the preset changes. Drawn over the picture on screen only,
//! never into what is exported, and only with the interface's effects full. A browser plays no
//! sound before the person's first click, so the web app's screen waits dark for it, asking to be
//! switched on, and switches on with its sound then.
use crate::{interface, sound};
use eframe::egui::{self, Color32, Rect};
use web_time::Instant;

/// How long the screen takes to switch on, in seconds.
const POWER_ON: f32 = 0.65;
/// How long a burst of static lasts, in seconds.
const STATIC: f32 = 0.16;

#[derive(Default)]
pub struct Screen {
    /// Whether new media is waiting for its first picture, to switch on for it.
    armed: bool,
    /// Whether the screen waits dark for the person's first click, touch or key press.
    standby: bool,
    /// When the screen switched on, while it still is.
    powered: Option<Instant>,
    /// When the static began, while it lasts.
    flicker: Option<Instant>,
    noise: Option<egui::TextureHandle>,
}

impl Screen {
    /// New media opened: switch on when its first picture shows.
    pub fn new_media(&mut self) {
        self.armed = true;
    }

    /// Waits dark, asking to be switched on, until [`wake`](Self::wake).
    pub fn stand_by(&mut self) {
        self.standby = true;
    }

    /// The person clicked, touched or pressed a key: a screen standing by switches on.
    pub fn wake(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.standby) {
            self.armed = true;
            ctx.request_repaint();
        }
    }

    /// Whether the screen is switching on or flickering.
    pub fn busy(&self) -> bool {
        self.powered.is_some() || self.flicker.is_some()
    }

    /// The channel or the preset changed: a burst of static.
    pub fn flicker(&mut self, ctx: &egui::Context) {
        if interface::effects(ctx).screen() {
            self.flicker = Some(Instant::now());
            ctx.request_repaint();
        }
    }

    /// Draws what the screen is doing over the picture, shown at `picture`.
    pub fn paint(&mut self, ui: &egui::Ui, picture: Rect) {
        let ctx = ui.ctx();
        if self.standby {
            let blinks = interface::effects(ctx).screen();
            standby(ui, picture, blinks);
            if blinks {
                ctx.request_repaint_after(web_time::Duration::from_millis(250));
            }
            return;
        }
        if std::mem::take(&mut self.armed) {
            sound::cue(ctx, sound::Sound::PowerOn);
            if interface::effects(ctx).screen() {
                self.powered = Some(Instant::now());
            }
        }
        let painter = ui
            .painter()
            .with_clip_rect(picture.intersect(ui.clip_rect()));
        if let Some(t) = elapsed(&mut self.powered, POWER_ON) {
            power_on(&painter, picture, t);
            ctx.request_repaint();
        }
        if let Some(t) = elapsed(&mut self.flicker, STATIC) {
            let noise = self.noise.get_or_insert_with(|| noise(ctx)).id();
            snow(&painter, picture, t, noise);
            ctx.request_repaint();
        }
    }
}

/// The screen dark, with a line across it asking to be switched on, blinking as a game's title
/// screen does when `blinks`.
fn standby(ui: &egui::Ui, picture: Rect, blinks: bool) {
    let painter = ui
        .painter()
        .with_clip_rect(picture.intersect(ui.clip_rect()));
    painter.rect_filled(picture, 0., Color32::from_rgb(6, 8, 10));
    let seconds = ui.input(|i| i.time);
    let shown = !blinks || seconds.fract() < 0.65;
    let size = (picture.height() * 0.06).clamp(14., 30.);
    if shown {
        painter.text(
            picture.center(),
            egui::Align2::CENTER_CENTER,
            "CLICK TO SWITCH ON",
            egui::FontId::monospace(size),
            Color32::from_rgb(214, 238, 222),
        );
    }
    painter.text(
        picture.center() + egui::vec2(0., size * 1.6),
        egui::Align2::CENTER_CENTER,
        "Sounds can be turned off at the top of the settings",
        egui::FontId::proportional((size * 0.5).max(11.)),
        Color32::from_gray(120),
    );
    // The set's standby lamp, in the corner of the glass.
    painter.circle_filled(
        picture.right_bottom() - egui::vec2(16., 16.),
        4.,
        Color32::from_rgb(235, 72, 64),
    );
}

/// How far through an effect begun at `start` and lasting `length` seconds it is, from 0 to 1,
/// forgetting it once over.
fn elapsed(start: &mut Option<Instant>, length: f32) -> Option<f32> {
    let t = start.as_ref()?.elapsed().as_secs_f32() / length;
    if t >= 1. {
        *start = None;
        return None;
    }
    Some(t)
}

/// The screen switching on, `t` of the way: a bright line across the middle that widens, then
/// opens out to the whole picture, flashing as it does.
fn power_on(painter: &egui::Painter, picture: Rect, t: f32) {
    let center = picture.center();
    // The beam's sweep: the line reaches across first, then opens up and down.
    let across = (t / 0.35).min(1.);
    let open = ((t - 0.35) / 0.3).clamp(0., 1.);
    let half_height = egui::lerp(1.5..=picture.height() / 2., open * open * (3. - 2. * open));
    let half_width = picture.width() / 2. * egui::lerp(0.55..=1., across);
    let lit = Rect::from_center_size(center, egui::vec2(2. * half_width, 2. * half_height));
    // Dark wherever the beam has not reached.
    for dark in [
        Rect::from_min_max(picture.min, egui::pos2(picture.max.x, lit.top())),
        Rect::from_min_max(egui::pos2(picture.min.x, lit.bottom()), picture.max),
        Rect::from_min_max(
            egui::pos2(picture.min.x, lit.top()),
            egui::pos2(lit.left(), lit.bottom()),
        ),
        Rect::from_min_max(
            egui::pos2(lit.right(), lit.top()),
            egui::pos2(picture.max.x, lit.bottom()),
        ),
    ] {
        painter.rect_filled(dark, 0., Color32::BLACK);
    }
    let flash = if open < 1. {
        1.
    } else {
        1. - (t - 0.65) / 0.35
    };
    painter.rect_filled(
        lit,
        0.,
        Color32::from_white_alpha(
            (flash.clamp(0., 1.) * if open == 0. { 255. } else { 140. }) as u8,
        ),
    );
}

/// Static over the picture, `t` of the way through its burst: snow, rolling and fading.
fn snow(painter: &egui::Painter, picture: Rect, t: f32, noise: egui::TextureId) {
    // A fresh stretch of the noise each frame, so the snow moves.
    let seed = (t * 1000.) as u32;
    let offset = egui::vec2(hash(seed) as f32, hash(seed ^ 0x5bd1) as f32) / u32::MAX as f32;
    let uv = Rect::from_min_size(offset.to_pos2(), egui::vec2(1., 1.));
    let alpha = ((1. - t) * 220.) as u8;
    painter.image(noise, picture, uv, Color32::from_white_alpha(alpha));
    // A brighter band rolling down, as the hold slips.
    let band = picture.height() * 0.12;
    let y = picture.top() + (t * 2.).fract() * picture.height();
    painter.rect_filled(
        Rect::from_x_y_ranges(picture.x_range(), y..=y + band),
        0.,
        Color32::from_white_alpha(alpha / 4),
    );
}

/// A small tile of grey noise, repeated across the picture.
fn noise(ctx: &egui::Context) -> egui::TextureHandle {
    let (width, height) = (160, 120);
    let pixels = (0..width * height)
        .map(|i| {
            let v = (hash(i as u32) >> 24) as u8;
            Color32::from_gray(v)
        })
        .collect();
    ctx.load_texture(
        "screen static",
        egui::ColorImage::new([width, height], pixels),
        egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::NEAREST
        },
    )
}

/// A well-mixed number from `x`.
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_end_and_are_forgotten() {
        let mut start = Some(Instant::now() - web_time::Duration::from_secs(1));
        assert_eq!(elapsed(&mut start, 0.5), None);
        assert!(start.is_none());
        let mut start = Some(Instant::now());
        assert!(elapsed(&mut start, 10.).is_some_and(|t| t < 0.1));
    }

    #[test]
    fn a_screen_standing_by_switches_on_only_when_woken() {
        let ctx = egui::Context::default();
        let mut screen = Screen::default();
        screen.stand_by();
        screen.new_media();
        let frame = |screen: &mut Screen| {
            let mut heard = vec![];
            let mut output = ctx.run_ui(Default::default(), |ui| {
                screen.paint(ui, ui.max_rect());
                heard = sound::take(ui.ctx());
            });
            output.textures_delta.clear();
            heard
        };
        assert_eq!(frame(&mut screen), vec![], "dark until woken");
        assert_eq!(frame(&mut screen), vec![]);
        screen.wake(&ctx);
        assert_eq!(frame(&mut screen), vec![sound::Sound::PowerOn]);
        // Woken again, by the next click, it does not switch on again.
        screen.wake(&ctx);
        assert_eq!(frame(&mut screen), vec![]);
    }

    #[test]
    fn new_media_sounds_its_power_on_once() {
        let ctx = egui::Context::default();
        let mut screen = Screen::default();
        screen.new_media();
        let mut heard = vec![];
        for _ in 0..2 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                screen.paint(ui, ui.max_rect());
                heard.extend(sound::take(ui.ctx()));
            });
            output.textures_delta.clear();
        }
        assert_eq!(heard, vec![sound::Sound::PowerOn]);
        assert!(
            screen.powered.is_some(),
            "the default effects switch the screen on"
        );
    }
}
