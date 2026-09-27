//! A CI run: open the window, wait for a rendered preview, save a screenshot and quit.
use crate::*;

/// What a smoke run shows and where it saves the screenshot. It never reads or writes the app
/// data a person's own runs keep.
pub(crate) struct Smoke {
    pub screenshot: PathBuf,
    /// Screenshot the welcome as it first appears, not waiting for a preview.
    pub welcome: bool,
    /// Open the video export dialog once a preview is ready, and screenshot that.
    pub export: bool,
    /// Open the preset gallery, and wait for its thumbnails.
    pub gallery: bool,
    /// Open the LUT gallery, and wait for its thumbnails.
    pub lut_gallery: bool,
    requested: bool,
    started: Instant,
}

impl Smoke {
    pub fn new(screenshot: PathBuf) -> Self {
        Self {
            screenshot,
            welcome: false,
            export: false,
            gallery: false,
            lut_gallery: false,
            requested: false,
            started: Instant::now(),
        }
    }
}

impl App {
    /// Drives a smoke run: once the preview has rendered, opens the export dialog if asked to,
    /// then takes the screenshot, saves it and closes the window. A run that never gets there
    /// fails after two minutes rather than hanging CI.
    pub(crate) fn advance_smoke(&mut self, ctx: &egui::Context) {
        let ready = self.schedule.is_current();
        if ready && self.smoke.as_ref().is_some_and(|smoke| smoke.export) {
            if let Some(smoke) = &mut self.smoke {
                smoke.export = false;
            }
            self.open_video_export(false);
            ctx.request_repaint();
            return;
        }
        let thumbnails_pending = self.thumbnails_pending();
        let (error, preview_error) = (&self.error, &self.preview_error);
        let Some(smoke) = &mut self.smoke else {
            return;
        };
        if smoke.started.elapsed() > Duration::from_secs(120) {
            eprintln!("Desktop smoke test timed out: {error:?}; {preview_error:?}");
            std::process::exit(1);
        }
        if (smoke.welcome || ready && !thumbnails_pending) && !smoke.requested {
            smoke.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let im = RgbaImage::from_raw(image.width() as u32, image.height() as u32, bytes)
                    .unwrap();
                if let Err(e) = files::save_png(&smoke.screenshot, im, None) {
                    eprintln!("{e:#}");
                    std::process::exit(1);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
