//! The preview: from an edit to the CRT picture on screen. It decides when the picture is out
//! of date and asks the worker's preview lane for the next one (see `schedule`), shows what
//! comes back unless settings since replaced it, and holds the picture on screen. It also holds
//! the look a gallery is auditioning: pointing at a preset or LUT previews the image with it,
//! and moving away returns to the settings in use. The settings, their undo history and the
//! source stay the app's; an auditioned look never reaches them.
mod schedule;

pub use schedule::Kind;
use schedule::{Change, Due, Schedule};

use crate::worker::{PreviewJob, PreviewLane, Previewed, Stopped};
use crtsim_core::config::Config;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use std::sync::Arc;
use web_time::Instant;

/// What a frame's tick, or Refresh, found. The two are independent: a change settles whether or
/// not its preview could be sent.
#[must_use]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tick {
    /// The change under way has settled, so it becomes an undo step.
    pub settled: bool,
    /// The preview lane could not take the preview due: the renderer has stopped, and nothing
    /// more will be rendered.
    pub stopped: bool,
}

/// A look previewed from a gallery without being applied.
#[derive(Clone, PartialEq)]
struct Audition {
    label: String,
    config: Config,
}

/// The picture on screen. A frame rendered on the interface's own device is handed to egui as
/// it is; anything else is uploaded as an ordinary texture.
enum Picture {
    Uploaded(TextureHandle),
    Frame {
        /// Held because egui samples it until the registration is freed.
        _texture: wgpu::Texture,
        id: egui::TextureId,
        size: egui::Vec2,
    },
}

pub struct Preview {
    schedule: Schedule,
    lane: PreviewLane,
    /// The interface's device, needed to register and release frames rendered on it.
    render_state: Option<eframe::egui_wgpu::RenderState>,
    picture: Option<Picture>,
    /// A look previewed from a gallery without being applied.
    audition: Option<Audition>,
    /// What a gallery pointed at this frame; the last offer of the frame wins.
    offered: Option<Audition>,
    /// The longest side a preview is rendered at; none for the export's own size.
    pub quality: Option<u32>,
    /// Why the last preview could not be rendered.
    pub error: Option<String>,
}

impl Preview {
    /// A preview whose first picture, of the source as it opens, is already due.
    pub fn new(
        lane: PreviewLane,
        render_state: Option<eframe::egui_wgpu::RenderState>,
        now: Instant,
    ) -> Self {
        Self {
            schedule: Schedule::new(now),
            lane,
            render_state,
            picture: None,
            audition: None,
            offered: None,
            quality: Some(1280),
            error: None,
        }
    }

    /// The settings or the source changed at `now`: the picture on screen is out of date.
    pub fn changed(&mut self, now: Instant) {
        self.schedule.changed(Change::Edit, now);
    }

    /// A gallery reports the look under the pointer, once per frame, for the next tick.
    pub fn offer(&mut self, label: impl Into<String>, config: Config) {
        self.offered = Some(Audition {
            label: label.into(),
            config,
        });
    }

    /// Called once per frame, after the galleries have drawn: starts, switches or ends the
    /// audition, and asks the lane for the preview due, of `config` and `input`, when
    /// `may_render`. A change settles whether or not it could be rendered.
    pub fn tick(
        &mut self,
        now: Instant,
        pointer_down: bool,
        config: &Config,
        input: &Arc<RgbaImage>,
        may_render: bool,
    ) -> Tick {
        self.settle_audition(config, now);
        let kind = match self.schedule.due(now, pointer_down) {
            Due::Nothing => return Tick::default(),
            Due::Commit => {
                return Tick {
                    settled: true,
                    stopped: false,
                }
            }
            Due::Preview(kind) => kind,
        };
        let asked = if may_render {
            self.request(kind, config, input)
        } else {
            Ok(false)
        };
        Tick {
            settled: kind == Kind::Settled,
            stopped: asked.is_err(),
        }
    }

    /// Refresh: renders the settings exactly now, live preview or not. Settled if it was asked
    /// for, which it is unless a preview is still rendering.
    pub fn refresh(&mut self, config: &Config, input: &Arc<RgbaImage>) -> Tick {
        let asked = self.request(Kind::Settled, config, input);
        Tick {
            settled: asked != Ok(false),
            stopped: asked.is_err(),
        }
    }

    /// A preview the lane rendered, or could not, came back. It is shown unless settings since
    /// replaced it; a line saying so, for the status bar, if it is.
    pub fn returned(
        &mut self,
        ctx: &egui::Context,
        revision: u64,
        result: anyhow::Result<Previewed>,
    ) -> Option<String> {
        if !self.schedule.returned(revision) {
            return None;
        }
        match result {
            Ok(previewed) => {
                let (width, height) = previewed.image.dimensions();
                let picture = self.picture_of(ctx, previewed.image);
                self.show(picture);
                self.schedule.show(revision);
                self.error = None;
                Some(format!(
                    "Preview {width} × {height} · {:.2}s",
                    previewed.seconds
                ))
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                None
            }
        }
    }

    /// Shows a frame of the video playing, which is of the settings as they are.
    pub fn show_played(&mut self, frame: TextureHandle) {
        self.show(Some(Picture::Uploaded(frame)));
        self.schedule.show_current();
    }

    /// Takes the picture off screen, as when another source opens.
    pub fn clear(&mut self) {
        self.show(None);
    }

    /// Forgets the change waiting to be previewed, as when playing takes over the screen.
    pub fn drop_pending(&mut self) {
        self.schedule.drop_pending();
    }

    /// Nothing more will be rendered: the renderer is gone.
    pub fn stop(&mut self) {
        self.schedule.stop();
    }

    /// The picture on screen, if there is one.
    pub fn picture(&self) -> Option<egui::load::SizedTexture> {
        self.picture.as_ref().map(|picture| match picture {
            Picture::Uploaded(handle) => egui::load::SizedTexture::from_handle(handle),
            Picture::Frame { id, size, .. } => egui::load::SizedTexture::new(*id, *size),
        })
    }

    /// The name of the look being auditioned, if one is.
    pub fn audition(&self) -> Option<&str> {
        self.audition
            .as_ref()
            .map(|audition| audition.label.as_str())
    }

    /// Live preview: preview every edit as it happens, rather than on Refresh.
    pub fn live_mut(&mut self) -> &mut bool {
        &mut self.schedule.live
    }

    pub fn rendering(&self) -> bool {
        self.schedule.rendering()
    }

    /// Whether a settled preview is rendering, which the interface shows as busy.
    pub fn rendering_settled(&self) -> bool {
        self.schedule.rendering_settled()
    }

    /// Whether the picture on screen shows the settings as they settled, with nothing more to
    /// render.
    pub fn is_settled(&self) -> bool {
        self.schedule.is_settled()
    }

    /// Whether the picture on screen is out of date with nothing on its way to replace it.
    pub fn stale(&self) -> bool {
        self.picture.is_some() && self.schedule.stale()
    }

    /// Whether frames must keep coming for the preview to move on at `now`.
    pub fn needs_frames(&self, now: Instant) -> bool {
        self.schedule.needs_frames(now)
    }

    /// Starts, switches or ends the audition from what was offered this frame. Pointing at the
    /// look already in use is not an audition of anything.
    fn settle_audition(&mut self, config: &Config, now: Instant) {
        let offered = self.offered.take().filter(|a| a.config != *config);
        if offered != self.audition {
            self.audition = offered;
            self.schedule.changed(Change::Audition, now);
        }
    }

    /// Asks the lane for a preview of this kind, of the auditioned look or else `config`.
    /// Whether it was asked for: not while another preview is rendering. An error once it was,
    /// if the lane could not take it.
    fn request(
        &mut self,
        kind: Kind,
        config: &Config,
        input: &Arc<RgbaImage>,
    ) -> Result<bool, Stopped> {
        let Some(revision) = self.schedule.take(kind) else {
            return Ok(false);
        };
        let shown = self.audition.as_ref().map_or(config, |a| &a.config);
        match shown.with_max_output_side(input.dimensions(), self.quality) {
            Ok(config) => {
                let sent = self.lane.send(PreviewJob::Preview {
                    revision,
                    kind,
                    input: input.clone(),
                    config: Box::new(config),
                });
                if sent.is_err() {
                    self.schedule.stop();
                }
                sent.map(|()| true)
            }
            Err(e) => {
                // Nothing was sent, so nothing will come back.
                self.schedule.returned(revision);
                self.error = Some(format!("Cannot preview: {e:#}"));
                Ok(true)
            }
        }
    }

    /// Prepares a rendered preview for drawing. A frame is registered with egui; pixels are
    /// uploaded, which is what happens when the renderer is on its own device.
    fn picture_of(&self, ctx: &egui::Context, preview: crate::worker::Preview) -> Option<Picture> {
        match (preview, self.render_state.as_ref()) {
            (crate::worker::Preview::Pixels(image), _) => {
                let limit = ctx.input(|i| i.max_texture_side).min(u32::MAX as usize) as u32;
                Some(Picture::Uploaded(crate::texture(ctx, "crt", &image, limit)))
            }
            (crate::worker::Preview::Frame(frame), Some(state)) => {
                let view = frame.texture.create_view(&Default::default());
                let id = state.renderer.write().register_native_texture(
                    &state.device,
                    &view,
                    wgpu::FilterMode::Linear,
                );
                Some(Picture::Frame {
                    size: egui::vec2(frame.width as f32, frame.height as f32),
                    _texture: frame.texture,
                    id,
                })
            }
            // The worker only renders a frame when the interface has a device to draw it on,
            // so there is nowhere for this to come from.
            (crate::worker::Preview::Frame(_), None) => None,
        }
    }

    /// Puts `next` on screen, releasing the registration of the frame it replaces: egui keeps
    /// no ownership of a frame handed to it, so nothing else frees these.
    fn show(&mut self, next: Option<Picture>) {
        if let Some(Picture::Frame { id, .. }) = self.picture.take() {
            if let Some(state) = &self.render_state {
                state.renderer.write().free_texture(&id);
            }
        }
        self.picture = next;
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.show(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{self, Jobs};
    use std::sync::mpsc;
    use std::time::Duration;

    const QUIET: Tick = Tick {
        settled: false,
        stopped: false,
    };
    const SETTLES: Tick = Tick {
        settled: true,
        stopped: false,
    };

    /// Long enough for any change to settle.
    const SETTLED: Duration = Duration::from_secs(1);

    /// Late enough for any change made now to have settled.
    fn settled() -> Instant {
        Instant::now() + SETTLED
    }

    /// A preview whose lane is a channel the test reads, of the test card with the default
    /// settings; nothing is pending.
    fn preview() -> (Preview, mpsc::Receiver<PreviewJob>, Config, Arc<RgbaImage>) {
        let (jobs, _work, previews) = Jobs::capture();
        let mut preview = Preview::new(jobs.preview_lane(), None, Instant::now());
        preview.drop_pending();
        let input = Arc::new(crtsim_core::config::test_card());
        (preview, previews, Config::general(), input)
    }

    fn rendered(seconds: f32) -> anyhow::Result<Previewed> {
        Ok(Previewed {
            image: worker::Preview::Pixels(crtsim_core::config::test_card()),
            seconds,
        })
    }

    /// The preview job sent: its revision, kind and settings.
    fn sent(previews: &mpsc::Receiver<PreviewJob>) -> (u64, Kind, Config) {
        match previews.try_recv() {
            Ok(PreviewJob::Preview {
                revision,
                kind,
                config,
                ..
            }) => (revision, kind, *config),
            _ => panic!("expected a preview to be asked for"),
        }
    }

    #[test]
    fn a_settled_preview_of_replaced_settings_is_not_shown() {
        let ctx = egui::Context::default();
        let (mut preview, previews, mut config, input) = preview();
        assert!(preview.refresh(&config, &input).settled);
        let (asked, ..) = sent(&previews);
        config.bloom = 0.;
        preview.changed(Instant::now());
        assert_eq!(preview.returned(&ctx, asked, rendered(0.1)), None);
        assert!(preview.picture().is_none() && !preview.rendering());
        // The change is still to be previewed.
        let tick = preview.tick(settled(), false, &config, &input, true);
        assert_eq!(tick, SETTLES);
        let (revision, kind, shown) = sent(&previews);
        assert_eq!((kind, shown.bloom), (Kind::Settled, 0.));
        let summary = preview.returned(&ctx, revision, rendered(0.1));
        assert!(summary.is_some_and(|s| s.starts_with("Preview ")));
        assert!(preview.is_settled() && preview.picture().is_some());
    }

    #[test]
    fn a_drag_is_previewed_as_it_goes_and_settles_once() {
        let ctx = egui::Context::default();
        let (mut preview, previews, mut config, input) = preview();
        // Each frame of the drag: the slider moves, and the frame's due preview is asked for.
        let drag = |preview: &mut Preview, config: &mut Config, bloom| {
            config.bloom = bloom;
            preview.changed(Instant::now());
            let tick = preview.tick(Instant::now(), true, config, &input, true);
            assert_eq!(tick, QUIET, "nothing settles mid-drag");
            let (revision, kind, shown) = sent(&previews);
            assert_eq!((kind, shown.bloom), (Kind::Interactive, bloom));
            revision
        };
        let first = drag(&mut preview, &mut config, 0.5);
        // The slider moves on before the preview comes back; it is still shown.
        config.bloom = 0.25;
        preview.changed(Instant::now());
        assert!(preview.returned(&ctx, first, rendered(0.01)).is_some());
        assert!(preview.picture().is_some() && !preview.is_settled());
        assert!(!preview.stale(), "the drag's next preview is on its way");
        drag(&mut preview, &mut config, 0.);
        // Let go: the next preview to be asked for settles the drag, once.
        let tick = preview.tick(settled(), false, &config, &input, true);
        assert_eq!(tick, SETTLES);
    }

    #[test]
    fn a_change_settles_without_rendering_while_rendering_is_not_allowed() {
        let (mut preview, previews, config, input) = preview();
        preview.changed(Instant::now());
        let tick = preview.tick(settled(), false, &config, &input, false);
        assert_eq!(tick, SETTLES);
        assert!(previews.try_recv().is_err(), "nothing rendered");
        let tick = preview.tick(settled(), false, &config, &input, true);
        assert_eq!(tick, SETTLES, "still to be previewed once allowed");
        sent(&previews);
    }

    #[test]
    fn an_audition_is_previewed_even_without_live_preview() {
        let (mut preview, previews, config, input) = preview();
        *preview.live_mut() = false;
        let candidate = Config {
            bloom: 1.5,
            ..config.clone()
        };
        // A gallery offers the look under the pointer on every frame.
        let start = Instant::now();
        for (at, ticked) in [(start, QUIET), (start + SETTLED, SETTLES)] {
            preview.offer("preset “Bright”", candidate.clone());
            let tick = preview.tick(at, false, &config, &input, true);
            assert_eq!(tick, ticked);
            assert_eq!(preview.audition(), Some("preset “Bright”"));
        }
        assert_eq!(sent(&previews).2.bloom, 1.5);
    }

    #[test]
    fn moving_away_returns_to_the_settings_and_the_current_look_is_not_an_audition() {
        let ctx = egui::Context::default();
        let (mut preview, previews, config, input) = preview();
        let start = Instant::now();
        preview.offer("current", config.clone());
        let _ = preview.tick(start, false, &config, &input, true);
        assert!(preview.audition().is_none());
        let candidate = Config {
            chroma: 0.,
            ..config.clone()
        };
        let point = |preview: &mut Preview, at| {
            preview.offer("LUT “Gray”", candidate.clone());
            let _ = preview.tick(at, false, &config, &input, true);
        };
        point(&mut preview, start);
        point(&mut preview, start + SETTLED);
        let (revision, _, shown) = sent(&previews);
        assert_eq!(shown.chroma, 0.);
        let _ = preview.returned(&ctx, revision, rendered(0.1));
        // Still pointed at on the next frame: nothing new to render.
        point(&mut preview, start + 2 * SETTLED);
        assert!(previews.try_recv().is_err());
        // Nothing offered from here on: the pointer moved away.
        *preview.live_mut() = false;
        let _ = preview.tick(start + 3 * SETTLED, false, &config, &input, true);
        assert!(preview.audition().is_none());
        let _ = preview.tick(start + 4 * SETTLED, false, &config, &input, true);
        assert_eq!(
            sent(&previews).2.chroma,
            config.chroma,
            "the settings in use redrawn"
        );
    }

    #[test]
    fn a_stopped_lane_is_reported_and_asks_for_nothing_more() {
        let (jobs, _work, previews) = Jobs::capture();
        let mut preview = Preview::new(jobs.preview_lane(), None, Instant::now());
        drop(previews);
        let (config, input) = (
            Config::general(),
            Arc::new(crtsim_core::config::test_card()),
        );
        // The change still settles, so it becomes an undo step all the same.
        let tick = preview.tick(settled(), false, &config, &input, true);
        assert_eq!(
            tick,
            Tick {
                settled: true,
                stopped: true
            }
        );
        assert!(!preview.rendering());
        assert_eq!(preview.tick(settled(), false, &config, &input, true), QUIET);
    }
}
