//! Asking before the window closes while an export runs, which closing would stop: the file
//! would not be saved, and only a batch queue comes back with the session to finish later. A
//! browser has no close to cancel, so there the page asks the browser to ask instead.
use crate::chrome::Fitted;
use crate::*;

/// Where closing the window has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Closing {
    /// Nothing asked, or the person chose to keep exporting.
    #[default]
    Open,
    /// The window was asked to close while an export ran; the person is being asked.
    Asking,
    /// The window closes once the exports running have finished.
    WhenDone,
    /// The window is closing now; a request to close goes through.
    Now,
}

impl App {
    /// An export runs, or a batch queue has jobs still to go.
    pub(crate) fn exporting(&self) -> bool {
        self.lane.is_exporting() || self.queue.running()
    }

    /// Holds a request to close the window while an export runs, and asks about it; closes it
    /// once the exports finish, when asked to wait for them.
    pub(crate) fn ask_before_closing(&mut self, ctx: &egui::Context) {
        #[cfg(target_arch = "wasm32")]
        if self.leaving_asks != self.exporting() {
            self.leaving_asks = self.exporting();
            web::ask_before_leaving(self.leaving_asks);
        }
        let requested = ctx.input(|i| i.viewport().close_requested());
        // A smoke run closes itself once it has its screenshot.
        if requested && self.closing != Closing::Now && self.exporting() && self.smoke.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.closing = Closing::Asking;
        }
        if self.closing == Closing::WhenDone && !self.exporting() {
            // An export that failed stays in view, its notice saying why, rather than closing
            // over it. Each export starts with no saving notice, so one now is its own.
            if self.notices.error(Topic::Saving).is_some() {
                self.closing = Closing::Open;
            } else {
                self.close(ctx);
            }
        }
        if matches!(self.closing, Closing::Asking | Closing::WhenDone) {
            self.closing_window(ctx);
        }
    }

    /// Closes the window, which stops any export still running.
    fn close(&mut self, ctx: &egui::Context) {
        self.closing = Closing::Now;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn closing_window(&mut self, ctx: &egui::Context) {
        let waiting = self.closing == Closing::WhenDone;
        let batch = self.queue.running();
        let mut choice = None;
        egui::Window::new("An export is still running")
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(false)
            .fitted(ctx, 420.)
            .show(ctx, |ui| {
                if waiting {
                    ui.label(if batch {
                        "The app closes when the batch queue has finished."
                    } else {
                        "The app closes when the export has finished."
                    });
                    if ui.button("Don't close").clicked() {
                        choice = Some(Closing::Open);
                    }
                    return;
                }
                ui.label(if batch {
                    "Closing now stops the batch export. The queue comes back, paused, when \
                     you next open the app, to finish then."
                } else {
                    "Closing now stops the export, and its file is not saved. Your settings \
                     come back when you next open the app."
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Keep exporting").clicked() {
                        choice = Some(Closing::Open);
                    }
                    if ui.button("Close when it finishes").clicked() {
                        choice = Some(Closing::WhenDone);
                    }
                    if ui.button("Stop and close").clicked() {
                        choice = Some(Closing::Now);
                    }
                });
            });
        match choice {
            Some(Closing::Now) => self.close(ctx),
            Some(choice) => self.closing = choice,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One pass of the interface asking `app` about a request to close, if `requested`; whether
    /// the window was told to stay open, and whether to close.
    fn pass(ctx: &egui::Context, app: &mut App, requested: bool) -> (bool, bool) {
        let mut input = egui::RawInput::default();
        if requested {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let mut output = ctx.run_ui(input, |ui| app.ask_before_closing(ui.ctx()));
        output.textures_delta.clear();
        let commands = output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|viewport| viewport.commands.clone())
            .unwrap_or_default();
        (
            commands.contains(&egui::ViewportCommand::CancelClose),
            commands.contains(&egui::ViewportCommand::Close),
        )
    }

    #[test]
    fn closing_during_an_export_asks_and_can_wait_for_it() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            None,
        );
        let (_work, _previews) = app.capture_jobs();
        // With nothing exporting, the window closes as asked.
        assert_eq!(pass(&ctx, &mut app, true), (false, false));
        let dir = tempfile::tempdir().unwrap();
        app.export(dir.path().join("crt.png"));
        assert_eq!(pass(&ctx, &mut app, true), (true, false));
        assert_eq!(app.closing, Closing::Asking);
        app.closing = Closing::WhenDone;
        assert_eq!(pass(&ctx, &mut app, false), (false, false));
        app.lane.finished();
        assert_eq!(pass(&ctx, &mut app, false), (false, true));
        // The close it asked for goes through.
        assert_eq!(pass(&ctx, &mut app, true), (false, false));
        // An export that fails while the app waits to close stays open to say so.
        app.closing = Closing::Open;
        app.export(dir.path().join("again.png"));
        assert_eq!(pass(&ctx, &mut app, true), (true, false));
        app.closing = Closing::WhenDone;
        app.lane.finished();
        app.notices
            .failed(Topic::Saving, "Cannot save again.png: disk full");
        assert_eq!(pass(&ctx, &mut app, false), (false, false));
        assert_eq!(app.closing, Closing::Open);
    }
}
