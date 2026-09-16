mod files;
mod model;
mod worker;

use crtsim_core::config::{self, Config, Filter, Fit, Phase};
use eframe::egui::{self, Color32, TextureHandle};
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use worker::{Event, Job};

#[derive(Clone, Copy)]
enum Dialog {
    Image,
    LoadPreset,
    SavePreset,
    Export,
}
#[derive(Clone, Copy, PartialEq)]
enum View {
    Crt,
    Original,
    Compare,
}

struct App {
    config: Config,
    history: model::History,
    input: Arc<RgbaImage>,
    source_name: String,
    original: TextureHandle,
    rendered: Option<TextureHandle>,
    rendered_revision: Option<u64>,
    revision: u64,
    preview_limit: Option<u32>,
    view: View,
    zoom: f32,
    fit_preview: bool,
    live: bool,
    dirty: bool,
    changed_at: Instant,
    rendering: bool,
    loading: bool,
    exporting: bool,
    dialog_open: bool,
    jobs: mpsc::Sender<Job>,
    events: mpsc::Receiver<Event>,
    dialog_send: mpsc::Sender<(Dialog, Option<PathBuf>)>,
    dialog_receive: mpsc::Receiver<(Dialog, Option<PathBuf>)>,
    status: String,
    error: Option<String>,
    adapter: String,
    smoke: Option<PathBuf>,
    smoke_requested: bool,
    started: Instant,
}

fn texture(ctx: &egui::Context, name: &str, image: &RgbaImage, limit: u32) -> TextureHandle {
    let image = if image.width().max(image.height()) > limit {
        image::DynamicImage::ImageRgba8(image.clone())
            .thumbnail(limit, limit)
            .to_rgba8()
    } else {
        image.clone()
    };
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    )
}

impl App {
    fn new(
        ctx: &egui::Context,
        backend: wgpu::Backends,
        input_path: Option<PathBuf>,
        smoke: Option<PathBuf>,
    ) -> Self {
        ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8., 8.);
        ctx.set_style(style);
        let input = Arc::new(config::test_card());
        let original = texture(ctx, "original", &input, 2048);
        let config = model::general();
        let (jobs, events) = worker::start(ctx.clone(), backend);
        let (dialog_send, dialog_receive) = mpsc::channel();
        let loading = input_path.is_some();
        if let Some(path) = input_path {
            let _ = jobs.send(Job::Load(path));
        }
        Self {
            history: model::History::new(config.clone()),
            config,
            input,
            source_name: "Built-in test card".into(),
            original,
            rendered: None,
            rendered_revision: None,
            revision: 0,
            preview_limit: Some(1280),
            view: View::Crt,
            zoom: 1.,
            fit_preview: true,
            live: true,
            dirty: true,
            changed_at: Instant::now(),
            rendering: false,
            loading,
            exporting: false,
            dialog_open: false,
            jobs,
            events,
            dialog_send,
            dialog_receive,
            status: "Preparing preview…".into(),
            error: None,
            adapter: String::new(),
            smoke,
            smoke_requested: false,
            started: Instant::now(),
        }
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.dirty = true;
        self.changed_at = Instant::now();
    }
    fn replace_config(&mut self, config: Config) {
        self.history.commit(&self.config);
        self.config = config;
        self.history.commit(&self.config);
        self.changed();
    }
    fn dialog(&mut self, kind: Dialog, ctx: &egui::Context) {
        self.dialog_open = true;
        let send = self.dialog_send.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = match kind {
                Dialog::Image => rfd::FileDialog::new()
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_file(),
                Dialog::LoadPreset => rfd::FileDialog::new()
                    .add_filter("CRT preset", &["json"])
                    .pick_file(),
                Dialog::SavePreset => rfd::FileDialog::new()
                    .add_filter("CRT preset", &["json"])
                    .set_file_name("my-crt.json")
                    .save_file(),
                Dialog::Export => rfd::FileDialog::new()
                    .add_filter("PNG image", &["png"])
                    .set_file_name("rendered.png")
                    .save_file(),
            };
            let _ = send.send((kind, path));
            ctx.request_repaint();
        });
    }
    fn load(&mut self, path: PathBuf) {
        self.loading = true;
        self.status = format!("Loading {}…", path.display());
        self.send(Job::Load(path));
    }
    fn send(&mut self, job: Job) {
        if self.jobs.send(job).is_err() {
            self.error =
                Some("Render worker stopped. Save your preset and restart the application.".into());
            self.rendering = false;
            self.loading = false;
            self.exporting = false;
            self.dirty = false;
        }
    }
    fn export(&mut self, path: PathBuf) {
        if path.exists() {
            self.error = Some(
                "That file already exists. Choose a new filename; exports never overwrite files."
                    .into(),
            );
            return;
        }
        if let Err(e) = model::preview_config(&self.config, self.input.dimensions(), None) {
            self.error = Some(format!("{e:#}"));
            return;
        }
        self.exporting = true;
        self.status = format!(
            "Exporting {}… Settings are captured for this export.",
            path.display()
        );
        self.send(Job::Export {
            input: self.input.clone(),
            config: self.config.clone(),
            path,
        });
    }
    fn receive(&mut self, ctx: &egui::Context) {
        while let Ok((kind, path)) = self.dialog_receive.try_recv() {
            self.dialog_open = false;
            if let Some(mut path) = path {
                match kind {
                    Dialog::Image => self.load(path),
                    Dialog::LoadPreset => {
                        match files::load_preset(&path, self.input.dimensions()) {
                            Ok(c) => {
                                self.replace_config(c);
                                self.status = format!("Loaded preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot load preset: {e:#}")),
                        }
                    }
                    Dialog::SavePreset => {
                        if path.extension().is_none() {
                            path.set_extension("json");
                        }
                        match model::preview_config(&self.config, self.input.dimensions(), None)
                            .and_then(|_| files::save_preset(&path, &self.config))
                        {
                            Ok(()) => {
                                self.status = format!("Saved preset {}", path.display());
                                self.error = None;
                            }
                            Err(e) => self.error = Some(format!("Cannot save preset: {e:#}")),
                        }
                    }
                    Dialog::Export => {
                        if path.extension().is_none() {
                            path.set_extension("png");
                        }
                        self.export(path);
                    }
                }
            }
        }
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Loaded(result) => {
                    self.loading = false;
                    match result {
                        Ok((path, input, thumb)) => {
                            self.source_name = path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            self.original = texture(ctx, "original", &thumb, 2048);
                            self.input = Arc::new(input);
                            self.rendered = None;
                            self.rendered_revision = None;
                            self.error = None;
                            self.changed();
                            self.status = "Image loaded".into();
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Preview { revision, result } => {
                    self.rendering = false;
                    if revision != self.revision {
                        continue;
                    }
                    match result {
                        Ok((im, adapter, seconds)) => {
                            let max_texture =
                                ctx.input(|i| i.max_texture_side).min(u32::MAX as usize) as u32;
                            self.rendered = Some(texture(ctx, "crt", &im, max_texture));
                            self.rendered_revision = Some(revision);
                            self.adapter = adapter;
                            if !self.exporting {
                                self.status = format!(
                                    "Preview {} × {} · {:.2}s",
                                    im.width(),
                                    im.height(),
                                    seconds
                                );
                            }
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
                Event::Exported(result) => {
                    self.exporting = false;
                    match result {
                        Ok(path) => {
                            self.status = format!("Saved {}", path.display());
                            self.error = None;
                        }
                        Err(e) => self.error = Some(e),
                    }
                }
            }
        }
    }
    fn request_preview(&mut self) {
        if self.rendering || self.loading || self.exporting {
            return;
        }
        self.history.commit(&self.config);
        self.dirty = false;
        match model::preview_config(&self.config, self.input.dimensions(), self.preview_limit) {
            Ok(config) => {
                self.rendering = true;
                self.send(Job::Preview {
                    revision: self.revision,
                    input: self.input.clone(),
                    config,
                });
            }
            Err(e) => self.error = Some(format!("Cannot preview: {e:#}")),
        }
    }
    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("CRTSim Renderer");
            ui.separator();
            let enabled = !self.dialog_open && !self.loading;
            if ui
                .add_enabled(enabled, egui::Button::new("Open image…"))
                .clicked()
            {
                self.dialog(Dialog::Image, ctx);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Test card"))
                .clicked()
            {
                self.input = Arc::new(config::test_card());
                self.original = texture(ctx, "original", &self.input, 2048);
                self.source_name = "Built-in test card".into();
                self.rendered = None;
                self.changed();
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Load preset…"))
                .clicked()
            {
                self.dialog(Dialog::LoadPreset, ctx);
            }
            if ui
                .add_enabled(enabled, egui::Button::new("Save preset…"))
                .clicked()
            {
                self.dialog(Dialog::SavePreset, ctx);
            }
            if ui
                .add_enabled(enabled && !self.exporting, egui::Button::new("Export PNG…"))
                .clicked()
            {
                self.dialog(Dialog::Export, ctx);
            }
        });
    }
    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Image & output");
        ui.label(&self.source_name);
        ui.small(format!(
            "Source: {} × {}",
            self.input.width(),
            self.input.height()
        ));
        ui.horizontal(|ui| {
            if ui.button("Undo").clicked() {
                if let Some(c) = self.history.undo(&self.config) {
                    self.config = c;
                    self.changed();
                }
            }
            if ui.button("Redo").clicked() {
                if let Some(c) = self.history.redo(&self.config) {
                    self.config = c;
                    self.changed();
                }
            }
            if ui
                .button("Reset")
                .on_hover_text("Reset to the general image preset; Undo restores your settings")
                .clicked()
            {
                self.replace_config(model::general());
            }
        });
        ui.horizontal(|ui| {
            if ui.button("General image").clicked() {
                self.replace_config(model::general());
            }
            if ui.button("Original CRTSim").clicked() {
                self.replace_config(Config::default());
            }
        });
        let before = self.config.clone();
        resolution(
            ui,
            "Signal",
            &mut self.config.signal,
            &["auto", "native", "original", "240p", "360p", "480p"],
        );
        resolution(
            ui,
            "Export size",
            &mut self.config.output,
            &["720p", "1080p", "1440p", "4k", "reference", "match-input"],
        );
        egui::ComboBox::from_label("Fit on 4:3 tube")
            .selected_text(format!("{:?}", self.config.fit))
            .show_ui(ui, |ui| {
                for (value, name) in [
                    (Fit::Contain, "Contain"),
                    (Fit::Cover, "Cover (crop)"),
                    (Fit::Stretch, "Stretch"),
                    (Fit::Reference, "Reference"),
                ] {
                    ui.selectable_value(&mut self.config.fit, value, name);
                }
            });
        egui::ComboBox::from_label("Resize filter")
            .selected_text(format!("{:?}", self.config.filter))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.config.filter, Filter::Lanczos, "Lanczos (smooth)");
                ui.selectable_value(
                    &mut self.config.filter,
                    Filter::Nearest,
                    "Nearest (pixel art)",
                );
            });
        slider(ui, "Pixel aspect", &mut self.config.pixel_aspect, 0.1..=10.);
        if let (Ok(signal), Ok(output)) = (
            self.config.signal_size(self.input.dimensions()),
            self.config.output_size(self.input.dimensions()),
        ) {
            ui.small(format!(
                "Signal: {} × {} → PNG: {} × {}",
                signal.0, signal.1, output.0, output.1
            ));
            if output.0 as u64 * output.1 as u64 > 8_300_000 {
                ui.colored_label(
                    Color32::YELLOW,
                    "Large output: more memory and rendering time.",
                );
            }
        } else {
            ui.colored_label(
                Color32::YELLOW,
                "Choose a preset or enter a valid WIDTHxHEIGHT.",
            );
        }
        if matches!(self.config.fit, Fit::Cover | Fit::Reference) || self.config.overscan > 1. {
            ui.colored_label(
                Color32::YELLOW,
                "Current fit/overscan can crop content and subtitles.",
            );
        }
        ui.small("Rounded glass may hide extreme corners even with Contain. Alpha is flattened onto black. SDR; no ICC/HDR conversion.");
        ui.separator();
        egui::CollapsingHeader::new("Color & signal")
            .default_open(true)
            .show(ui, |ui| {
                slider(ui, "Saturation", &mut self.config.saturation, 0.0..=3.);
                slider(
                    ui,
                    "Sharpness / ringing",
                    &mut self.config.sharpness,
                    0.0..=3.,
                );
                slider(ui, "Color bleed", &mut self.config.bleed, 0.0..=2.);
                slider(
                    ui,
                    "Composite artifacts",
                    &mut self.config.artifacts,
                    0.0..=2.,
                );
            });
        egui::CollapsingHeader::new("Glass & mask")
            .default_open(true)
            .show(ui, |ui| {
                slider(ui, "Barrel distortion", &mut self.config.barrel, -2.0..=2.);
                slider(ui, "Overscan", &mut self.config.overscan, 0.1..=3.);
                slider(ui, "Mask opacity", &mut self.config.mask_opacity, 0.0..=1.);
                slider(
                    ui,
                    "Mask brightness",
                    &mut self.config.mask_brightness,
                    0.0..=2.,
                );
                slider(
                    ui,
                    "Mask columns",
                    &mut self.config.mask_repeats[0],
                    1.0..=16384.,
                );
                slider(
                    ui,
                    "Mask rows",
                    &mut self.config.mask_repeats[1],
                    1.0..=16384.,
                );
                slider(ui, "Edge dimming", &mut self.config.dimming, 0.0..=1.);
                slider(ui, "Camera field of view", &mut self.config.fov, 5.0..=90.);
            });
        egui::CollapsingHeader::new("Bloom & reflections").show(ui, |ui| {
            slider(ui, "Bloom amount", &mut self.config.bloom, 0.0..=2.);
            slider(ui, "Bloom power", &mut self.config.bloom_power, 0.1..=8.);
            slider(ui, "Bloom spread", &mut self.config.bloom_spread, 0.0..=0.2);
            slider(ui, "Edge reflection", &mut self.config.reflection, 0.0..=2.);
        });
        egui::CollapsingHeader::new("Frame & lighting").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Frame color");
                ui.color_edit_button_rgb(&mut self.config.frame_color);
            });
            slider(ui, "Diffuse light", &mut self.config.diffuse, 0.0..=2.);
            slider(ui, "Specular light", &mut self.config.specular, 0.0..=2.);
            slider(
                ui,
                "Specular power",
                &mut self.config.specular_power,
                1.0..=200.,
            );
            slider(ui, "Rim light", &mut self.config.rim, 0.0..=2.);
            for (i, name) in ["Light X", "Light Y", "Light Z"].iter().enumerate() {
                slider(
                    ui,
                    name,
                    &mut self.config.light_position[i],
                    -1000.0..=1000.,
                );
            }
        });
        egui::CollapsingHeader::new("Persistence & artifact phase").show(ui, |ui| {
            for (i,name) in ["Red persistence","Green persistence","Blue persistence"].iter().enumerate() { slider(ui,name,&mut self.config.persistence[i],0.0..=0.999); }
            ui.add(egui::Slider::new(&mut self.config.warmup,0..=240).text("Warm-up ticks"));
            egui::ComboBox::from_label("Phase").selected_text(format!("{:?}",self.config.phase)).show_ui(ui,|ui| {
                for phase in [Phase::Stable,Phase::A,Phase::B,Phase::Alternating] { ui.selectable_value(&mut self.config.phase,phase,format!("{phase:?}")); }
            });
            ui.small("Each still starts from black. Higher persistence may require more warm-up ticks. Alternating phase depends on tick count.");
        });
        if self.config != before {
            self.changed();
        }
    }
    fn preview(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(&mut self.view, View::Crt, "CRT");
            ui.selectable_value(&mut self.view, View::Original, "Original");
            ui.selectable_value(&mut self.view, View::Compare, "Side by side");
            ui.separator();
            ui.checkbox(&mut self.live, "Live preview");
            if ui
                .add_enabled(
                    !self.rendering && !self.loading && !self.exporting,
                    egui::Button::new("Refresh"),
                )
                .clicked()
            {
                self.request_preview();
            }
        });
        ui.horizontal_wrapped(|ui| {
            let previous = self.preview_limit;
            egui::ComboBox::from_label("Preview quality")
                .selected_text(match self.preview_limit {
                    Some(800) => "Fast (800 px)",
                    Some(_) => "Balanced (1280 px)",
                    None => "Export resolution",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.preview_limit, Some(800), "Fast (800 px)");
                    ui.selectable_value(&mut self.preview_limit, Some(1280), "Balanced (1280 px)");
                    ui.selectable_value(&mut self.preview_limit, None, "Export resolution");
                });
            if previous != self.preview_limit {
                self.changed();
            }
            ui.checkbox(&mut self.fit_preview, "Fit view");
            if !self.fit_preview {
                ui.add(egui::Slider::new(&mut self.zoom, 0.25..=4.).text("Zoom"));
            }
        });
        ui.small("Preview size does not change export size. For mask detail, use Export resolution and 1× zoom; display scaling may still affect sampling.");
        if let Ok(c) =
            model::preview_config(&self.config, self.input.dimensions(), self.preview_limit)
        {
            if let Ok((w, h)) = c.output_size(self.input.dimensions()) {
                if w as f32 / self.config.mask_repeats[0] < 6.
                    || h as f32 / self.config.mask_repeats[1] < 3.
                {
                    ui.colored_label(Color32::YELLOW,"Dense mask at this preview size: aliasing / moiré is possible. Try a higher preview resolution or lower mask density.");
                }
            }
        }
        if self.rendering || self.loading || self.exporting {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if self.exporting {
                    "Exporting…"
                } else if self.loading {
                    "Loading image…"
                } else {
                    "Rendering preview…"
                });
            });
        }
        if self.rendered.is_some() && self.rendered_revision != Some(self.revision) {
            ui.colored_label(Color32::YELLOW, "Preview is out of date.");
        }
        let available = ui.available_size();
        egui::ScrollArea::both().auto_shrink([false,false]).show(ui,|ui| {
            if self.view == View::Compare {
                ui.horizontal_top(|ui| {
                    let area = egui::vec2((available.x-16.).max(1.)/2.,(available.y-24.).max(1.));
                    ui.vertical(|ui| { ui.label("Original"); show_image(ui,&self.original,area,self.fit_preview,self.zoom); });
                    if let Some(ref im) = self.rendered { ui.vertical(|ui| { ui.label("CRT"); show_image(ui,im,area,self.fit_preview,self.zoom); }); }
                });
            } else if self.view == View::Original { show_image(ui,&self.original,available,self.fit_preview,self.zoom); }
            else if let Some(ref im) = self.rendered { show_image(ui,im,available,self.fit_preview,self.zoom); }
            else { ui.label("Open an image or use the test card. Your rendered preview will appear here."); }
        });
    }
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>) {
    let logarithmic = *range.start() >= 1. && *range.end() >= 200.;
    ui.add(
        egui::Slider::new(value, range)
            .logarithmic(logarithmic)
            .clamp_to_range(false)
            .text(label),
    );
}
fn resolution(ui: &mut egui::Ui, label: &str, value: &mut String, presets: &[&str]) {
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_source(label)
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
fn show_image(ui: &mut egui::Ui, im: &TextureHandle, available: egui::Vec2, fit: bool, zoom: f32) {
    let size = im.size_vec2();
    let factor = if fit {
        (available.x / size.x).min(available.y / size.y).max(0.01)
    } else {
        zoom / ui.ctx().pixels_per_point()
    };
    ui.add(egui::Image::new(im).fit_to_exact_size(size * factor));
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.receive(ctx);
        if !self.loading && !self.dialog_open {
            let dropped = ctx.input(|i| i.raw.dropped_files.first().and_then(|f| f.path.clone()));
            if let Some(path) = dropped {
                self.load(path);
            }
        }
        // A modal file dialog freezes edits so its eventual result uses the displayed settings.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| self.toolbar(ui, ctx));
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.label(&self.status);
            if !self.adapter.is_empty() {
                ui.small(&self.adapter);
            }
            if let Some(error) = self.error.clone() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::LIGHT_RED, error);
                    if ui.button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
            }
        });
        egui::SidePanel::left("settings")
            .default_width(330.)
            .min_width(300.)
            .resizable(true)
            .show(ctx, |ui| {
                ui.add_enabled_ui(!self.dialog_open, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.settings(ui));
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| self.preview(ui));
        if self.dirty
            && self.changed_at.elapsed() >= Duration::from_millis(180)
            && !ctx.input(|i| i.pointer.any_down())
        {
            self.history.commit(&self.config);
            if self.live {
                self.request_preview();
            }
        }
        if (self.dirty && (self.live || self.changed_at.elapsed() < Duration::from_millis(180)))
            || self.rendering
            || self.loading
            || self.exporting
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        // Reproducible CI screenshot after an actual preview, without platform-specific mouse coordinates.
        if let Some(ref path) = self.smoke {
            if self.started.elapsed() > Duration::from_secs(120) {
                eprintln!("Desktop smoke test timed out: {:?}", self.error);
                std::process::exit(1);
            }
            if self.rendered_revision == Some(self.revision) && !self.smoke_requested {
                self.smoke_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let im =
                        RgbaImage::from_raw(image.width() as u32, image.height() as u32, bytes)
                            .unwrap();
                    if let Err(e) = files::save_png(path, im) {
                        eprintln!("{e:#}");
                        std::process::exit(1);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}

fn main() -> eframe::Result<()> {
    let mut input = None;
    let mut smoke = None;
    let mut backends = wgpu::Backends::PRIMARY;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => {
                backends = match args.next().as_deref() {
                    Some("vulkan") => wgpu::Backends::VULKAN,
                    Some("dx12") => wgpu::Backends::DX12,
                    Some("metal") => wgpu::Backends::METAL,
                    Some("auto") => wgpu::Backends::PRIMARY,
                    _ => {
                        eprintln!("Expected --backend auto|vulkan|dx12|metal");
                        std::process::exit(2);
                    }
                }
            }
            "--smoke-test" => {
                smoke = Some(PathBuf::from(
                    args.next().expect("--smoke-test needs a new PNG path"),
                ))
            }
            "--help" | "-h" => {
                println!("crtsim-desktop [IMAGE] [--backend auto|vulkan|dx12|metal]\nOpen images, adjust effects, load/save JSON presets and export PNG from the window.");
                return Ok(());
            }
            s if s.starts_with('-') => {
                eprintln!("Unknown option: {s}");
                std::process::exit(2);
            }
            _ if input.is_none() => input = Some(PathBuf::from(arg)),
            _ => {
                eprintln!("Only one image can be opened at startup");
                std::process::exit(2);
            }
        }
    }
    eframe::run_native(
        "CRTSim Renderer",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280., 850.])
                .with_min_inner_size([900., 620.]),
            renderer: eframe::Renderer::Glow,
            ..Default::default()
        },
        Box::new(move |cc| Box::new(App::new(&cc.egui_ctx, backends, input, smoke))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsolete_preview_cannot_replace_current_settings() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, wgpu::Backends::PRIMARY, None, None);
        let (send, receive) = mpsc::channel();
        app.events = receive;
        app.rendering = true;
        app.config.bloom = 0.;
        app.changed();
        send.send(Event::Preview {
            revision: 0,
            result: Ok((config::test_card(), "obsolete".into(), 0.1)),
        })
        .unwrap();
        app.receive(&ctx);
        assert!(app.rendered.is_none());
        assert!(!app.rendering);
        assert!(app.dirty);
        send.send(Event::Preview {
            revision: app.revision,
            result: Ok((config::test_card(), "current".into(), 0.1)),
        })
        .unwrap();
        app.receive(&ctx);
        assert_eq!(app.rendered_revision, Some(app.revision));
        assert_eq!(app.adapter, "current");
    }

    #[test]
    fn export_captures_full_resolution_and_original_source() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, wgpu::Backends::PRIMARY, None, None);
        let (send, receive) = mpsc::channel();
        app.jobs = send;
        let dir = tempfile::tempdir().unwrap();
        app.config.output = "4k".into();
        app.preview_limit = Some(800);
        let source = app.input.clone();
        let expected = app.config.clone();
        app.export(dir.path().join("rendered.png"));
        app.config.bloom = 0.;
        app.input = Arc::new(RgbaImage::new(1, 1));
        match receive.recv().unwrap() {
            Job::Export { input, config, .. } => {
                assert!(Arc::ptr_eq(&input, &source));
                assert_eq!(config, expected);
                assert_eq!(
                    config.output_size(input.dimensions()).unwrap(),
                    (3840, 2160)
                );
            }
            _ => panic!("expected export job"),
        }
    }
}
