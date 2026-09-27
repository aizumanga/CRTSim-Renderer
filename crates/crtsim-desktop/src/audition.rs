//! Trying a look before choosing it: pointing at a preset or LUT in a gallery previews the
//! image with it, and moving away returns to the settings in use. Nothing about the auditioned
//! look reaches the settings, their undo history or an export until it is clicked.
use crate::*;
use crtsim_core::{nes_luts, workflow::Lut};

/// A look being previewed from a gallery without being applied.
#[derive(Clone, PartialEq)]
pub struct Audition {
    pub label: String,
    pub config: Config,
}

impl App {
    /// The settings the preview shows: an auditioned look while one is offered, otherwise the
    /// settings in use.
    pub(crate) fn shown_config(&self) -> &Config {
        self.audition.as_ref().map_or(&self.config, |a| &a.config)
    }

    /// A gallery reports the entry under the pointer, once per frame. The last offer of the
    /// frame wins; `settle_audition` then acts on it.
    pub(crate) fn offer_audition(&mut self, label: impl Into<String>, config: Config) {
        self.offered = Some(Audition {
            label: label.into(),
            config,
        });
    }

    /// Called once per frame after the galleries have drawn. Starts, switches or ends the
    /// audition, and asks for a preview when that changes what should be on screen.
    pub(crate) fn settle_audition(&mut self) {
        // Pointing at the look already in use is not an audition of anything.
        let offered = self.offered.take().filter(|a| a.config != self.config);
        if offered == self.audition {
            return;
        }
        self.audition = offered;
        // Like an edit, but without stopping playback or touching the settings.
        self.schedule.changed(Change::Audition, Instant::now());
    }

    /// An included LUT, decoded once and then shared, so hovering back and forth does not
    /// decode it again and the renderer's LUT cache sees the same table each time.
    pub(crate) fn included_lut(&mut self, index: usize) -> anyhow::Result<Arc<Lut>> {
        if let Some(lut) = self.included_luts.get(&index) {
            return Ok(lut.clone());
        }
        let lut = Arc::new(nes_luts::load(index)?);
        self.included_luts.insert(index, lut.clone());
        Ok(lut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::Due;

    /// Late enough for any change to have settled.
    fn settled() -> Instant {
        Instant::now() + Duration::from_secs(1)
    }

    fn app() -> (App, mpsc::Receiver<Job>, mpsc::Receiver<PreviewJob>) {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (jobs, work, previews) = worker::Jobs::capture();
        app.jobs = jobs;
        app.show_welcome = false;
        (app, work, previews)
    }

    #[test]
    fn an_audition_is_previewed_without_touching_the_settings() {
        let (mut app, work, previews) = app();
        app.schedule.live = false;
        let settings = app.config.clone();
        let candidate = Config {
            bloom: 1.5,
            ..settings.clone()
        };
        app.offer_audition("preset “Bright”", candidate.clone());
        app.settle_audition();
        assert_eq!(
            app.schedule.due(settled(), false),
            Due::Preview,
            "previewed even without live preview"
        );
        app.request_preview();
        match previews.try_recv() {
            Ok(PreviewJob::Preview { config, .. }) => assert_eq!(config.bloom, 1.5),
            _ => panic!("expected a preview of the auditioned look"),
        }
        assert_eq!(app.config, settings);
        assert_eq!(app.history.undo(&app.config), None, "no undo step recorded");
        // Exports take the settings in use, never the look being pointed at.
        let dir = tempfile::tempdir().unwrap();
        app.export(dir.path().join("out.png"));
        match work.try_recv() {
            Ok(Job::Export {
                export: worker::Export::Image { config, .. },
                ..
            }) => assert_eq!(config, settings),
            _ => panic!("expected an export"),
        }
    }

    #[test]
    fn moving_away_returns_to_the_settings_and_the_current_look_is_not_an_audition() {
        let (mut app, _work, previews) = app();
        app.offer_audition("current", app.config.clone());
        app.settle_audition();
        assert!(app.audition.is_none());
        let candidate = Config {
            chroma: 0.,
            ..app.config.clone()
        };
        app.offer_audition("LUT “Gray”", candidate.clone());
        app.settle_audition();
        app.request_preview();
        let Ok(PreviewJob::Preview { revision, .. }) = previews.try_recv() else {
            panic!("expected a preview of the auditioned look");
        };
        app.schedule.returned(revision);
        // Still pointed at on the next frame: nothing new to render.
        app.offer_audition("LUT “Gray”", candidate);
        app.settle_audition();
        assert_eq!(app.schedule.due(settled(), false), Due::Nothing);
        // Nothing offered this frame: the pointer moved away.
        app.settle_audition();
        app.schedule.live = false;
        assert!(app.audition.is_none());
        assert_eq!(app.schedule.due(settled(), false), Due::Preview);
        app.request_preview();
        match previews.try_recv() {
            Ok(PreviewJob::Preview { config, .. }) => assert_eq!(config.chroma, app.config.chroma),
            _ => panic!("ending an audition redraws the settings in use"),
        }
    }
}
