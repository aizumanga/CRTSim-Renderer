use crate::{App, Dialog};
use crtsim_media::{Audio, Encoder, EncodingSpeed, Options, Quality, Timing};
use eframe::egui;

#[derive(Clone, Copy, Default, PartialEq)]
pub enum Format {
    #[default]
    Mp4,
    Mkv,
    Webm,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4 · H.264 — recommended",
            Self::Mkv => "MKV · H.264 — preserve more tracks",
            Self::Webm => "WebM · VP9 — web playback",
        }
    }
    fn description(self) -> &'static str {
        match self {
        Self::Mp4=>"H.264 plays on most devices and editing apps. MP4 is the simplest choice for sharing. Text subtitles are converted; attachments and bitmap subtitles are omitted.",
        Self::Mkv=>"The same H.264 picture quality in a more flexible container. MKV can retain subtitles and attachments that MP4 cannot. Some apps have limited MKV support.",
        Self::Webm=>"VP9 is useful for web playback and can compress efficiently, but software encoding can be slower. Audio uses Opus when re-encoding; text subtitles become WebVTT.",
    }
    }
}
pub struct ExportDialog {
    options: Options,
    format: Format,
    batch: bool,
}
impl App {
    pub fn open_video_export(&mut self, batch: bool) {
        self.stop_playback();
        self.workflow.export_dialog = Some(ExportDialog {
            options: self.video_options.clone(),
            format: if batch {
                Format::Mkv
            } else {
                self.workflow.export_format
            },
            batch,
        });
    }
    pub fn video_export_window(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.workflow.export_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut proceed = false;
        let mut cancel = false;
        egui::Window::new(if draft.batch {"Batch video export settings"} else {"Export video"})
            .open(&mut open).collapsible(false).resizable(true).default_width(540.)
            .show(ctx,|ui| {
                ui.label(if draft.batch {"These settings apply to videos added next. Existing jobs keep their captured settings."} else {"Choose how to save your video. The defaults are a good starting point."});
                egui::ScrollArea::vertical().max_height(520.).show(ui,|ui| {
                    ui.heading("Format & codec");
                    if draft.batch {ui.label(Format::Mkv.label());} else {
                        for format in [Format::Mp4,Format::Mkv,Format::Webm] {
                            if ui.radio_value(&mut draft.format,format,format.label()).changed() {draft.options.crf=None;}
                        }
                    }
                    ui.small(draft.format.description());
                    if draft.format==Format::Webm {draft.options.encoder=Encoder::Software;}
                    ui.separator();
                    egui::ComboBox::from_label("Picture quality").selected_text(format!("{:?}",draft.options.quality)).show_ui(ui,|ui| {
                        for (q,label) in [(Quality::Draft,"Draft · smaller, quicker files"),(Quality::Balanced,"Balanced · recommended"),(Quality::High,"High · more detail, larger files"),(Quality::Archival,"Archival · largest files, still lossy")] {
                            if ui.selectable_value(&mut draft.options.quality,q,label).changed() {draft.options.crf=None;draft.options.bitrate_mbps=None;}
                        }
                    });
                    ui.small("Quality changes file size and compression. It does not change export resolution or CRT effects.");
                    timing_controls(ui,&mut draft.options);
                    egui::ComboBox::from_label("Audio in exported file").selected_text(match draft.options.audio {Audio::Auto=>"Preserve when compatible",Audio::Encode=>"Re-encode",Audio::Mute=>"No audio"}).show_ui(ui,|ui| {
                        for (value,label) in [(Audio::Auto,"Preserve when compatible"),(Audio::Encode,"Re-encode AAC / Opus"),(Audio::Mute,"No audio")] {ui.selectable_value(&mut draft.options.audio,value,label);}
                    });
                    ui.checkbox(&mut draft.options.preserve_streams,"Keep additional tracks, subtitles, chapters & metadata");
                    if let Some(video)=&self.video {
                        if draft.options.preserve_streams && !draft.batch {
                            for note in crtsim_media::preservation_notes(video,draft.format.extension()) {ui.small(note);}
                        }
                    }
                    egui::CollapsingHeader::new("Advanced · encoding method & parameters").show(ui,|ui| {
                        if draft.format==Format::Webm {ui.label("VP9 uses software encoding.");} else {
                            egui::ComboBox::from_label("Encoding method").selected_text(encoder_label(draft.options.encoder)).show_ui(ui,|ui| {
                                for e in [Encoder::Software,Encoder::Nvenc,Encoder::Qsv,Encoder::Amf,Encoder::VideoToolbox] {ui.selectable_value(&mut draft.options.encoder,e,encoder_label(e));}
                            });
                        }
                        ui.small(encoder_description(draft.options.encoder));
                        if draft.options.encoder==Encoder::Software {
                            let mut custom=draft.options.crf.is_some();
                            if ui.checkbox(&mut custom,"Custom quality (CRF)").changed() {draft.options.crf=custom.then(||draft.options.effective_crf(draft.format==Format::Webm));}
                            if let Some(crf)=draft.options.crf.as_mut() {ui.add(egui::Slider::new(crf,0..=51).text("CRF"));}
                            ui.small("Lower CRF keeps more detail and usually produces larger files. Leave custom quality off to use the selected profile.");
                            egui::ComboBox::from_label("Compression speed").selected_text(match draft.options.speed {None=>"Profile default",Some(EncodingSpeed::Fast)=>"Fast",Some(EncodingSpeed::Balanced)=>"Balanced",Some(EncodingSpeed::Slow)=>"Slow"}).show_ui(ui,|ui| {
                                for (speed,label) in [(None,"Profile default"),(Some(EncodingSpeed::Fast),"Fast"),(Some(EncodingSpeed::Balanced),"Balanced"),(Some(EncodingSpeed::Slow),"Slow")] {ui.selectable_value(&mut draft.options.speed,speed,label);}
                            });
                            ui.small("Slower compression spends more time finding efficient encoding; CRT rendering speed is separate.");
                        } else {
                            let mut custom=draft.options.bitrate_mbps.is_some();
                            if ui.checkbox(&mut custom,"Custom target bitrate").changed() {draft.options.bitrate_mbps=custom.then_some(12);}
                            if let Some(rate)=draft.options.bitrate_mbps.as_mut() {ui.add(egui::Slider::new(rate,1..=200).text("Mbps"));}
                            ui.small("Higher bitrate allows more detail and larger files. Automatic bitrate adapts the quality profile to resolution and frame rate. Hardware availability is checked before export.");
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    proceed=ui.button(if draft.batch {"Use for new batch jobs"} else {"Choose destination…"}).clicked();
                    cancel=ui.button("Cancel").clicked();
                });
            });
        if proceed {
            if let Err(e) = draft.options.validate() {
                self.error = Some(e.to_string());
                self.workflow.export_dialog = Some(draft);
                return;
            }
            self.video_options = draft.options;
            if !draft.batch {
                self.workflow.export_format = draft.format;
                self.dialog(Dialog::ExportVideo, ctx);
            }
        } else if open && !cancel {
            self.workflow.export_dialog = Some(draft);
        }
    }
}
pub fn timing_controls(ui: &mut egui::Ui, options: &mut Options) {
    egui::ComboBox::from_label("Video timing")
        .selected_text(match options.timing {
            Timing::Stable => "Source rate · stable artifacts",
            Timing::Ntsc60 => "60 Hz · alternating artifacts",
            Timing::Disabled => "Source rate · persistence off",
        })
        .show_ui(ui, |ui| {
            for (t, label) in [
                (Timing::Stable, "Source rate · stable artifacts"),
                (Timing::Ntsc60, "60 Hz · alternating artifacts"),
                (Timing::Disabled, "Source rate · persistence off"),
            ] {
                ui.selectable_value(&mut options.timing, t, label);
            }
        });
}
fn encoder_label(encoder: Encoder) -> &'static str {
    match encoder {
        Encoder::Software => "Software · CPU (recommended)",
        Encoder::Nvenc => "NVIDIA graphics card · NVENC",
        Encoder::Qsv => "Intel graphics · Quick Sync",
        Encoder::Amf => "AMD graphics card · AMF",
        Encoder::VideoToolbox => "Apple hardware · VideoToolbox",
    }
}
fn encoder_description(encoder: Encoder) -> &'static str {
    match encoder {
    Encoder::Software=>"Uses your processor for compression. Predictable quality, no special hardware encoder required.",
    Encoder::Nvenc=>"Uses a supported NVIDIA GPU to encode H.264. Often faster; requires NVENC support in FFmpeg and a working NVIDIA driver.",
    Encoder::Qsv=>"Uses supported Intel graphics to encode H.264. Requires Quick Sync support in FFmpeg and the Intel media driver.",
    Encoder::Amf=>"Uses supported AMD graphics to encode H.264. Requires AMF support in FFmpeg and a compatible AMD driver.",
    Encoder::VideoToolbox=>"Uses Apple's video encoding system for H.264 on supported Macs. Requires a compatible FFmpeg build.",
}
}
