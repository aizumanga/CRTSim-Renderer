use crate::{
    widgets::{choice, Keyed},
    App, Dialog,
};
use crtsim_media::{
    AnimationFormat, AnimationOptions, AnimationSummary, Audio, Container, Dither, Encoder,
    EncodingSpeed, Options, Quality, Timing,
};
use eframe::egui;

/// What an export writes: a video, or an animation without sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Video(Container),
    Animation(AnimationFormat),
}

impl Default for ExportFormat {
    fn default() -> Self {
        Self::Video(Container::default())
    }
}

impl ExportFormat {
    /// The formats this host writes: all of them on the desktop, with FFmpeg; in a browser,
    /// the animations, until it encodes video too.
    fn offered() -> Vec<Self> {
        let animations = AnimationFormat::ALL.map(Self::Animation);
        #[cfg(target_arch = "wasm32")]
        return animations.to_vec();
        #[cfg(not(target_arch = "wasm32"))]
        Container::ALL
            .map(Self::Video)
            .into_iter()
            .chain(animations)
            .collect()
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Video(container) => container.extension(),
            Self::Animation(format) => format.extension(),
        }
    }

    /// The kind of file, as a save dialog's filter names it.
    pub fn filter(self) -> &'static str {
        match self {
            Self::Video(_) => "Video",
            Self::Animation(AnimationFormat::Gif) => "GIF animation",
            Self::Animation(AnimationFormat::Webp) => "WebP animation",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Video(Container::Mp4) => "MP4 · H.264 — recommended",
            Self::Video(Container::Mkv) => "MKV · H.264 — preserve more tracks",
            Self::Video(Container::Webm) => "WebM · VP9 — web playback",
            Self::Animation(AnimationFormat::Gif) => "GIF · plays everywhere, no sound",
            Self::Animation(AnimationFormat::Webp) => {
                "Animated WebP · smaller, full color, no sound"
            }
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Video(Container::Mp4) => {
                "H.264 plays on most devices and editing apps. MP4 is the simplest choice for \
                 sharing. Text subtitles are converted; attachments and bitmap subtitles are \
                 omitted."
            }
            Self::Video(Container::Mkv) => {
                "The same H.264 picture quality in a more flexible container. MKV can retain \
                 subtitles and attachments that MP4 cannot. Some apps have limited MKV support."
            }
            Self::Video(Container::Webm) => {
                "VP9 is useful for web playback and can compress efficiently, but software \
                 encoding can be slower. Audio uses Opus when re-encoding; text subtitles \
                 become WebVTT."
            }
            Self::Animation(AnimationFormat::Gif) => {
                "256 colors for the whole animation, dithered, and delays in hundredths of a \
                 second. GIFs grow fast with size, rate and length, so keep clips short and \
                 small. The renderer preset is not stored in the file."
            }
            Self::Animation(AnimationFormat::Webp) => {
                "Usually several times smaller than a GIF, with full color. Most browsers and \
                 chat apps play it; some image editors do not. The renderer preset is not \
                 stored in the file."
            }
        }
    }
}

/// An animation likely to be larger than this, in bytes, is only exported once confirmed.
const LARGE_ANIMATION: u64 = 25_000_000;

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb < 10. {
        format!("{mb:.1} MB")
    } else {
        format!("{mb:.0} MB")
    }
}

pub struct ExportDialog {
    options: Options,
    animation: AnimationOptions,
    format: ExportFormat,
    batch: bool,
    /// Start an animation at the frame on screen instead of the beginning.
    from_current_frame: bool,
    /// The likely size of the animation the export was asked about, until it is confirmed or
    /// the settings change.
    confirming: Option<u64>,
}
impl App {
    pub fn open_video_export(&mut self, batch: bool) {
        self.stop_playback();
        self.export_dialog = Some(ExportDialog {
            options: self.video_options.clone(),
            animation: self.animation_options.clone(),
            format: if batch {
                ExportFormat::Video(Container::Mkv)
            } else if ExportFormat::offered().contains(&self.export_format) {
                self.export_format
            } else {
                ExportFormat::offered()[0]
            },
            batch,
            from_current_frame: false,
            confirming: None,
        });
    }
    pub fn video_export_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.export_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut proceed = false;
        let mut cancel = false;
        let mut summary = None;
        let mut setup = false;
        egui::Window::new(if draft.batch {
            "Batch video export settings"
        } else {
            "Export video or animation"
        })
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(540.)
        .default_height(680.)
        .show(ctx, |ui| {
            ui.label(if draft.batch {
                "These settings apply to videos added next. Existing jobs \
                    keep their captured settings."
            } else {
                "Choose how to save your video. The defaults are a good \
                    starting point."
            });
            // The settings scroll; the size estimate's warning and the buttons stay in view.
            egui::ScrollArea::vertical()
                .max_height((ui.available_height() - 90.).max(160.))
                .show(ui, |ui| {
                    ui.heading("Format & codec");
                    if draft.batch {
                        ui.label(draft.format.label());
                    } else {
                        for format in ExportFormat::offered() {
                            if ui
                                .radio_value(&mut draft.format, format, format.label())
                                .changed()
                            {
                                draft.options.crf = None;
                            }
                        }
                    }
                    ui.small(draft.format.description());
                    let encoder = match draft.format {
                        ExportFormat::Video(container) => draft.options.encoder.codec(container),
                        ExportFormat::Animation(format) => Ok(format.encoder()),
                    };
                    let unavailable = if self.ffmpeg.missing() {
                        Some("Exporting needs FFmpeg, which was not found.".to_owned())
                    } else {
                        encoder.ok().filter(|e| self.ffmpeg.lacks(e)).map(|e| {
                            format!("This FFmpeg cannot write this format: it has no {e} encoder.")
                        })
                    };
                    if let Some(problem) = unavailable {
                        ui.horizontal_wrapped(|ui| {
                            ui.colored_label(ui.visuals().warn_fg_color, problem);
                            setup |= ui.small_button("FFmpeg setup…").clicked();
                        });
                    }
                    match draft.format {
                        ExportFormat::Video(container) => {
                            let notes = match &self.timeline {
                                Some(timeline) if !draft.batch => {
                                    crtsim_media::preservation_notes(&timeline.video, container)
                                }
                                _ => vec![],
                            };
                            video_settings(ui, &mut draft.options, container, &notes);
                        }
                        ExportFormat::Animation(format) => {
                            let frame = self.timeline.as_ref().map_or(0, |t| t.shown);
                            animation_settings(
                                ui,
                                &mut draft.animation,
                                format,
                                &mut draft.from_current_frame,
                                frame,
                            );
                            if let Some(timeline) = &self.timeline {
                                let video = &timeline.video;
                                draft.animation.start = if draft.from_current_frame {
                                    video.frame_time(frame)
                                } else {
                                    0.
                                };
                                let result = draft.animation.summary(video, &self.config, format);
                                size_estimate(ui, format, &result);
                                summary = result.ok();
                            }
                        }
                    }
                });
            ui.separator();
            let likely = summary.map(|s| s.bytes.1);
            if draft.confirming.is_some() && draft.confirming != likely {
                // The settings changed since the question: ask again for the new size.
                draft.confirming = None;
            }
            if let Some(bytes) = draft.confirming {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!(
                        "This file could be up to {}. Export it anyway?",
                        megabytes(bytes)
                    ),
                );
            }
            ui.horizontal(|ui| {
                proceed = ui
                    .button(if draft.batch {
                        "Use for new batch jobs"
                    } else if draft.confirming.is_some() {
                        "Export anyway · choose destination…"
                    } else {
                        "Choose destination…"
                    })
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if setup {
            self.show_ffmpeg_setup();
        }
        if proceed {
            let valid = match draft.format {
                ExportFormat::Video(_) => draft.options.validate(),
                ExportFormat::Animation(format) => draft.animation.validate(format),
            };
            if let Err(e) = valid {
                self.error = Some(e.to_string());
                self.export_dialog = Some(draft);
                return;
            }
            if let (ExportFormat::Animation(_), Some(summary)) = (draft.format, summary) {
                if summary.bytes.1 > LARGE_ANIMATION && draft.confirming.is_none() {
                    draft.confirming = Some(summary.bytes.1);
                    self.export_dialog = Some(draft);
                    return;
                }
            }
            self.video_options = draft.options;
            self.animation_options = draft.animation;
            if !draft.batch {
                self.export_format = draft.format;
                self.dialog(Dialog::ExportVideo, ctx);
            }
        } else if open && !cancel {
            self.export_dialog = Some(draft);
        }
    }
}

/// A video's codec, quality, timing, audio and track settings.
fn video_settings(
    ui: &mut egui::Ui,
    options: &mut Options,
    container: Container,
    notes: &[String],
) {
    if container == Container::Webm {
        options.encoder = Encoder::Software;
    }
    ui.separator();
    let quality = choice(
        ui,
        "Picture quality",
        &mut options.quality,
        &[
            (Quality::Draft, "Draft · smaller, quicker files"),
            (Quality::Balanced, "Balanced · recommended"),
            (Quality::High, "High · more detail, larger files"),
            (Quality::Archival, "Archival · largest files, still lossy"),
        ],
    );
    if quality {
        options.crf = None;
        options.bitrate_mbps = None;
    }
    ui.small(
        "Quality changes file size and compression. It does not \
        change export resolution or CRT effects.",
    );
    timing_controls(ui, options);
    choice(
        ui,
        "Audio in exported file",
        &mut options.audio,
        &[
            (Audio::Auto, "Preserve when compatible"),
            (Audio::Encode, "Re-encode AAC / Opus"),
            (Audio::Mute, "No audio"),
        ],
    );
    ui.checkbox(
        &mut options.preserve_streams,
        "Keep additional tracks, subtitles, chapters & metadata",
    );
    if options.preserve_streams {
        for note in notes {
            ui.small(note);
        }
    }
    crate::chrome::Section::new("Advanced encoding settings").show(ui, |ui| {
        if container == Container::Webm {
            ui.label("VP9 uses software encoding.");
        } else {
            choice(ui, "Encoding method", &mut options.encoder, &ENCODERS);
        }
        ui.small(encoder_description(options.encoder));
        if options.encoder == Encoder::Software {
            let mut custom = options.crf.is_some();
            if ui.checkbox(&mut custom, "Custom quality (CRF)").changed() {
                options.crf = custom.then(|| options.effective_crf(container));
            }
            if let Some(crf) = options.crf.as_mut() {
                Keyed::new(0..=51).show(ui, crf, |s| s.text("CRF"));
            }
            ui.small(
                "Lower CRF keeps more detail and usually produces larger \
                files. Leave custom quality off to use the selected profile.",
            );
            choice(
                ui,
                "Compression speed",
                &mut options.speed,
                &[
                    (None, "Profile default"),
                    (Some(EncodingSpeed::Fast), "Fast"),
                    (Some(EncodingSpeed::Balanced), "Balanced"),
                    (Some(EncodingSpeed::Slow), "Slow"),
                ],
            );
            ui.small(
                "Slower compression spends more time finding efficient \
                encoding; CRT rendering speed is separate.",
            );
        } else {
            let mut custom = options.bitrate_mbps.is_some();
            if ui.checkbox(&mut custom, "Custom target bitrate").changed() {
                options.bitrate_mbps = custom.then_some(12);
            }
            if let Some(rate) = options.bitrate_mbps.as_mut() {
                Keyed::new(1..=200).show(ui, rate, |s| s.text("Mbps"));
            }
            ui.small(
                "Higher bitrate allows more detail and larger files. \
                Automatic bitrate adapts the quality profile to resolution \
                and frame rate. Hardware availability is checked before \
                export.",
            );
        }
    });
}

/// An animation's size, rate, span and encoding. `frame` is the frame on screen, from 0.
fn animation_settings(
    ui: &mut egui::Ui,
    options: &mut AnimationOptions,
    format: AnimationFormat,
    from_current_frame: &mut bool,
    frame: u64,
) {
    ui.separator();
    ui.horizontal(|ui| {
        Keyed::new(64..=1920)
            .reset_to(640)
            .show(ui, &mut options.max_side, |s| s.text("Longest side · px"));
        for side in [480, 640, 800, 1024] {
            ui.selectable_value(&mut options.max_side, side, side.to_string());
        }
    });
    ui.small(
        "The CRT is rendered at this size, so its mask and scanlines stay sharp. \
         It is the setting that changes the file size most.",
    );
    let top = format.max_fps();
    options.fps = options.fps.min(top);
    ui.horizontal(|ui| {
        Keyed::new(1..=top)
            .reset_to(24)
            .show(ui, &mut options.fps, |s| s.text("Frames per second"));
        for fps in [12, 15, 24, 30].into_iter().chain((top > 30).then_some(60)) {
            ui.selectable_value(&mut options.fps, fps, fps.to_string());
        }
    });
    if format == AnimationFormat::Gif {
        ui.small(
            "GIF times frames in hundredths of a second: at 24 per second they alternate \
             between 4 and 5, which averages to 24.",
        );
    }
    if options.timing == Timing::Ntsc60 {
        options.timing = Timing::Stable;
    }
    choice(
        ui,
        "Timing",
        &mut options.timing,
        &[
            (Timing::Stable, "Stable artifacts"),
            (Timing::Disabled, "Persistence off"),
        ],
    );
    ui.checkbox(
        from_current_frame,
        format!("Start at the frame shown ({})", frame + 1),
    );
    ui.horizontal(|ui| {
        let mut limited = options.max_seconds.is_some();
        if ui.checkbox(&mut limited, "Only the first").changed() {
            options.max_seconds = limited.then_some(10.);
        }
        if let Some(seconds) = options.max_seconds.as_mut() {
            ui.add(
                egui::DragValue::new(seconds)
                    .range(0.1..=600.)
                    .speed(0.1)
                    .suffix(" s"),
            );
        } else {
            ui.label("seconds: off, the whole video");
        }
    });
    match format {
        AnimationFormat::Gif => {
            choice(
                ui,
                "Dithering",
                &mut options.dither,
                &[
                    (Dither::Bayer, "Pattern · smaller files"),
                    (Dither::Diffusion, "Diffusion · smoother, larger"),
                    (Dither::None, "None · smallest, banding"),
                ],
            );
        }
        AnimationFormat::Webp => {
            ui.checkbox(
                &mut options.lossless,
                "Lossless · exact colors, larger files",
            );
            if !options.lossless {
                Keyed::new(0..=100)
                    .reset_to(75)
                    .show(ui, &mut options.quality, |s| s.text("Quality"));
                ui.small(
                    "Lossy WebP keeps color at half resolution, which softens the \
                     mask's colored stripes. Lossless keeps them.",
                );
            }
        }
    }
}

/// What the animation will be: its size, its length and roughly how large a file it makes.
fn size_estimate(
    ui: &mut egui::Ui,
    format: AnimationFormat,
    summary: &anyhow::Result<AnimationSummary>,
) {
    ui.separator();
    let summary = match summary {
        Ok(summary) => summary,
        Err(error) => {
            ui.colored_label(ui.visuals().error_fg_color, format!("{error:#}"));
            return;
        }
    };
    ui.label(format!(
        "{}×{} · {} frames",
        summary.size.0, summary.size.1, summary.frames
    ));
    let (low, high) = summary.bytes;
    let text = format!(
        "Likely {} to {}, depending on how much moves",
        megabytes(low),
        megabytes(high)
    );
    if high > LARGE_ANIMATION {
        ui.colored_label(ui.visuals().warn_fg_color, text);
        ui.small(format!(
            "That is large for {}. A smaller longest side, fewer frames per second or a \
             shorter clip shrink it most{}.",
            if format == AnimationFormat::Gif {
                "a GIF"
            } else {
                "an animation"
            },
            if format == AnimationFormat::Gif {
                "; animated WebP is usually several times smaller"
            } else {
                ""
            }
        ));
    } else {
        ui.label(text);
    }
    if summary.temporary > 0 {
        ui.small(format!(
            "Needs up to {} of temporary space next to the destination while exporting.",
            megabytes(summary.temporary)
        ));
    }
}

pub fn timing_controls(ui: &mut egui::Ui, options: &mut Options) {
    choice(
        ui,
        "Video timing",
        &mut options.timing,
        &[
            (Timing::Stable, "Source rate · stable artifacts"),
            (Timing::Ntsc60, "60 Hz · alternating artifacts"),
            (Timing::Disabled, "Source rate · persistence off"),
        ],
    );
}

/// The ways a video can be compressed, by what does it.
const ENCODERS: [(Encoder, &str); 5] = [
    (Encoder::Software, "Software · CPU (recommended)"),
    (Encoder::Nvenc, "NVIDIA graphics card · NVENC"),
    (Encoder::Qsv, "Intel graphics · Quick Sync"),
    (Encoder::Amf, "AMD graphics card · AMF"),
    (Encoder::VideoToolbox, "Apple hardware · VideoToolbox"),
];

fn encoder_description(encoder: Encoder) -> &'static str {
    match encoder {
        Encoder::Software => {
            "Uses your processor for compression. Predictable quality, no \
        special hardware encoder required."
        }
        Encoder::Nvenc => {
            "Uses a supported NVIDIA GPU to encode H.264. Often faster; \
        requires NVENC support in FFmpeg and a working NVIDIA \
        driver."
        }
        Encoder::Qsv => {
            "Uses supported Intel graphics to encode H.264. Requires \
        Quick Sync support in FFmpeg and the Intel media driver."
        }
        Encoder::Amf => {
            "Uses supported AMD graphics to encode H.264. Requires AMF \
        support in FFmpeg and a compatible AMD driver."
        }
        Encoder::VideoToolbox => {
            "Uses Apple's video encoding system for H.264 on supported \
        Macs. Requires a compatible FFmpeg build."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{worker, Job};
    use eframe::egui;
    use std::sync::{atomic::AtomicBool, Arc};

    #[test]
    fn an_animation_is_exported_with_its_own_options_to_its_own_extension() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(crate::app_data::Store::temporary()),
            None,
            None,
        );
        let (jobs, receive, _previews) = worker::Jobs::capture();
        app.jobs = jobs;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("clip.gif");
        let mut encoder =
            image::codecs::gif::GifEncoder::new(std::fs::File::create(&source).unwrap());
        for level in [0, 200] {
            let frame = image::RgbaImage::from_pixel(8, 6, image::Rgba([level, level, level, 255]));
            encoder.encode_frame(image::Frame::new(frame)).unwrap();
        }
        drop(encoder);
        let cancel = Arc::new(AtomicBool::new(false));
        let video = crtsim_media::probe(&source, &cancel).unwrap();
        app.timeline = Some(crate::timeline::Timeline::new(video, 0, 2));
        app.export_format = ExportFormat::Animation(AnimationFormat::Gif);
        app.animation_options.fps = 12;

        app.dialog_send
            .send(crate::dialogs::Answer::File(
                Dialog::ExportVideo,
                dir.path().join("out.mp4"),
            ))
            .unwrap();
        app.receive(&ctx);
        assert!(app.error.take().unwrap().contains(".gif"));
        assert!(receive.try_recv().is_err());

        app.dialog_send
            .send(crate::dialogs::Answer::File(
                Dialog::ExportVideo,
                dir.path().join("out.GIF"),
            ))
            .unwrap();
        app.receive(&ctx);
        match receive.try_recv() {
            Ok(Job::Export {
                export: worker::Export::Animation { options, .. },
                path,
                ..
            }) => {
                assert_eq!(options.fps, 12);
                assert_eq!(path, dir.path().join("out.GIF"));
            }
            _ => panic!("expected an animation export"),
        }
        assert_eq!(app.status, "Exporting GIF…");
    }
}
