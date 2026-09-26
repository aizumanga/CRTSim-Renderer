//! Gallery thumbnails made from the image being edited rather than from a stock picture, so
//! each entry shows what it would do to this image. Presets get a small CRT render; LUTs get
//! only their color mapping, which is all a LUT changes and costs no GPU time. Both are made
//! on the preview thread, behind any preview someone is waiting for.
use crate::*;
use std::collections::HashMap;
use worker::{Look, PreviewJob, ThumbnailKey};

/// Longest side of a preset thumbnail's render, in pixels.
const PRESET_SIDE: u32 = 192;
/// Longest side of the image a LUT thumbnail maps.
const LUT_SIDE: u32 = 128;

#[derive(Default)]
pub struct Thumbnails {
    /// The input these were made from. Compared by pointer: every load, frame seek or test card
    /// replaces the `Arc`, so a new one means a new source without hooking each of those paths.
    source: Option<Arc<RgbaImage>>,
    /// That input at LUT thumbnail size, made once per source.
    small: Option<Arc<RgbaImage>>,
    generation: u64,
    /// Present once asked for. `config` is what a preset thumbnail was rendered with, so a
    /// preset saved again under the same name is rendered again.
    entries: HashMap<ThumbnailKey, Thumbnail>,
}

struct Thumbnail {
    config: Option<Config>,
    state: State,
}

enum State {
    Pending,
    Ready(TextureHandle),
    /// Not retried until the source changes; the entry simply shows no picture.
    Failed,
}
impl Thumbnail {
    fn texture(&self) -> Option<TextureHandle> {
        match &self.state {
            State::Ready(texture) => Some(texture.clone()),
            _ => None,
        }
    }
}

impl App {
    fn thumbnails_current(&mut self) {
        let thumbnails = &mut self.thumbnails;
        if thumbnails
            .source
            .as_ref()
            .is_some_and(|source| Arc::ptr_eq(source, &self.input))
        {
            return;
        }
        thumbnails.source = Some(self.input.clone());
        thumbnails.small = None;
        thumbnails.generation += 1;
        thumbnails.entries.clear();
    }

    /// The thumbnail for a preset, asking for it if it is missing or out of date. `None` while
    /// it is being made, or if it could not be.
    pub(crate) fn preset_thumbnail(
        &mut self,
        name: &str,
        config: &Config,
    ) -> Option<TextureHandle> {
        self.thumbnails_current();
        let key = ThumbnailKey::Preset(name.to_owned());
        if let Some(entry) = self.thumbnails.entries.get(&key) {
            if entry.config.as_ref() == Some(config) {
                return entry.texture();
            }
        }
        let small = config.with_max_output_side(self.input.dimensions(), Some(PRESET_SIDE));
        self.thumbnails.entries.insert(
            key.clone(),
            Thumbnail {
                config: Some(config.clone()),
                state: if small.is_ok() {
                    State::Pending
                } else {
                    State::Failed
                },
            },
        );
        if let Ok(small) = small {
            self.send_preview(PreviewJob::Thumbnail {
                generation: self.thumbnails.generation,
                key,
                input: self.input.clone(),
                look: Look::Crt(Box::new(small)),
            });
        }
        None
    }

    /// The thumbnail for an included LUT, asking for it if it is missing.
    pub(crate) fn lut_thumbnail(&mut self, index: usize) -> Option<TextureHandle> {
        self.thumbnails_current();
        let key = ThumbnailKey::Lut(index);
        if let Some(entry) = self.thumbnails.entries.get(&key) {
            return entry.texture();
        }
        let small = self
            .thumbnails
            .small
            .get_or_insert_with(|| Arc::new(opaque_thumbnail(&self.input, LUT_SIDE)))
            .clone();
        self.thumbnails.entries.insert(
            key.clone(),
            Thumbnail {
                config: None,
                state: State::Pending,
            },
        );
        self.send_preview(PreviewJob::Thumbnail {
            generation: self.thumbnails.generation,
            key,
            input: small,
            look: Look::Lut(index),
        });
        None
    }

    pub(crate) fn thumbnail_ready(
        &mut self,
        ctx: &egui::Context,
        generation: u64,
        key: ThumbnailKey,
        result: Result<RgbaImage, String>,
    ) {
        if generation != self.thumbnails.generation {
            return;
        }
        let Some(entry) = self.thumbnails.entries.get_mut(&key) else {
            return;
        };
        entry.state = match result {
            Ok(image) => State::Ready(ctx.load_texture(
                format!("thumbnail {key:?}"),
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            )),
            Err(_) => State::Failed,
        };
    }

    /// Whether a gallery on screen is still waiting for thumbnails, so the smoke screenshot can
    /// wait for them rather than capture spinners in some places and pictures in others.
    pub(crate) fn thumbnails_pending(&self) -> bool {
        (self.show_gallery || self.show_lut_gallery)
            && self
                .thumbnails
                .entries
                .values()
                .any(|entry| matches!(entry.state, State::Pending))
    }
}

/// The input at thumbnail size, with transparency flattened onto black as the source view does.
fn opaque_thumbnail(input: &RgbaImage, side: u32) -> RgbaImage {
    let scale = (side as f32 / input.width().max(input.height()) as f32).min(1.);
    let size = (
        ((input.width() as f32 * scale).round() as u32).max(1),
        ((input.height() as f32 * scale).round() as u32).max(1),
    );
    let mut small = image::imageops::thumbnail(input, size.0, size.1);
    crtsim_core::config::flatten_alpha(&mut small, [0; 3]);
    small
}

/// A thumbnail at `height`, or a placeholder of the same size while it is being made, so the
/// list does not jump when it arrives. The response is for hovering and clicking.
pub(crate) fn show(
    ui: &mut egui::Ui,
    texture: Option<&TextureHandle>,
    height: f32,
    aspect: f32,
) -> egui::Response {
    let size = egui::vec2(height * aspect, height);
    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let size = size * (height / size.y);
            ui.add(egui::Image::new((texture.id(), size)).sense(egui::Sense::click()))
        }
        None => {
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            ui.painter()
                .rect_filled(rect, 2., ui.visuals().extreme_bg_color);
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(jobs: &mpsc::Receiver<PreviewJob>) -> Vec<(u64, ThumbnailKey)> {
        jobs.try_iter()
            .filter_map(|job| match job {
                PreviewJob::Thumbnail {
                    generation, key, ..
                } => Some((generation, key)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn thumbnails_are_asked_for_once_per_source_and_preset() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (captured, _work, jobs) = worker::Jobs::capture();
        app.jobs = captured;
        let preset = Config::general();
        assert!(app.lut_thumbnail(4).is_none());
        assert!(app.preset_thumbnail("General image", &preset).is_none());
        let first = asked(&jobs);
        assert_eq!(first.len(), 2);
        // Asking again, as every frame does, sends nothing new.
        app.lut_thumbnail(4);
        app.preset_thumbnail("General image", &preset);
        assert!(asked(&jobs).is_empty());
        // A preset saved again under the same name is rendered again.
        let edited = Config {
            bloom: 0.,
            ..preset.clone()
        };
        app.preset_thumbnail("General image", &edited);
        assert_eq!(asked(&jobs).len(), 1);
        // A new source asks for everything again, under a new generation...
        let generation = first[0].0;
        app.input = Arc::new(RgbaImage::new(32, 32));
        app.lut_thumbnail(4);
        assert_eq!(asked(&jobs), vec![(generation + 1, ThumbnailKey::Lut(4))]);
        // ...and a late result for the old source is not shown.
        app.thumbnail_ready(
            &ctx,
            generation,
            ThumbnailKey::Lut(4),
            Ok(RgbaImage::new(2, 2)),
        );
        assert!(app.lut_thumbnail(4).is_none());
        app.thumbnail_ready(
            &ctx,
            generation + 1,
            ThumbnailKey::Lut(4),
            Ok(RgbaImage::new(2, 2)),
        );
        assert!(app.lut_thumbnail(4).is_some());
    }

    #[test]
    fn a_failed_thumbnail_does_not_keep_the_smoke_screenshot_waiting() {
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, worker::Gpu::Own(wgpu::Backends::PRIMARY), None, None);
        let (captured, _work, _previews) = worker::Jobs::capture();
        app.jobs = captured;
        app.show_lut_gallery = true;
        app.lut_thumbnail(0);
        assert!(app.thumbnails_pending());
        let generation = app.thumbnails.generation;
        app.thumbnail_ready(&ctx, generation, ThumbnailKey::Lut(0), Err("bad".into()));
        assert!(!app.thumbnails_pending());
    }
}
