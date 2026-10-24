//! The interface's sounds: short tones and bursts of noise the app makes itself, so it ships no
//! audio files. Widgets cue a sound through egui's context as they are drawn; once a frame the
//! app takes what was cued, adds a press for a click no widget cued a sound of its own for, and
//! hands them to its [`Speaker`], which plays those the person chose to hear.
use crate::interface::Sounds;
use eframe::egui;
use std::sync::mpsc;
use web_time::{Duration, Instant};

/// A sound the interface makes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sound {
    /// A meter lit or cleared a cell; `at` is how far along it is now, from 0 to 1, which sets
    /// the pitch.
    Tick { at: f32 },
    /// A meter passed its default, as a notched knob clicks into its notch.
    Detent,
    /// A meter reached one of its ends.
    EndStop,
    /// A setting went back to its default from `from`, how far along its meter it was.
    Reset { from: f32 },
    /// A switch flipped on or off.
    Switch { on: bool },
    /// The pointer reached something to click.
    Hover,
    /// A button, or anything else clickable that cued no sound of its own, was pressed.
    Press,
    /// The preview changed channel: Original, CRT or Compare.
    Channel,
    /// A section of settings opened or closed.
    Fold,
    /// The preview's screen switched on for new media.
    PowerOn,
    /// A preset was applied.
    Preset,
    /// An export finished.
    Done,
}

/// Which switch in the settings a sound answers to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Group {
    /// Meters and switches.
    Controls,
    /// Buttons, channels and sections.
    Navigation,
    /// The screen switching on, presets and finished exports.
    Events,
}

impl Group {
    pub const ALL: [Self; 3] = [Self::Controls, Self::Navigation, Self::Events];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Controls => "controls",
            Self::Navigation => "navigation",
            Self::Events => "events",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Controls => "Meters and switches",
            Self::Navigation => "Buttons, channels and sections",
            Self::Events => "Power-on, presets and finished exports",
        }
    }
}

impl Sound {
    pub fn group(self) -> Group {
        match self {
            Self::Tick { .. }
            | Self::Detent
            | Self::EndStop
            | Self::Reset { .. }
            | Self::Switch { .. } => Group::Controls,
            Self::Hover | Self::Press | Self::Channel | Self::Fold => Group::Navigation,
            Self::PowerOn | Self::Preset | Self::Done => Group::Events,
        }
    }

    /// The shortest time between two of this sound, so a meter dragged quickly across many
    /// cells ticks rather than buzzes.
    fn spacing(self) -> Duration {
        match self {
            Self::Tick { .. } => Duration::from_millis(24),
            Self::Hover => Duration::from_millis(60),
            _ => Duration::ZERO,
        }
    }

    /// What plays: the tones and noise the sound is made of.
    fn voices(self) -> Vec<Voice> {
        use Wave::*;
        let tone = |wave, hz: f32, to: f32, length, gain, start| Voice {
            wave,
            from: hz,
            to,
            start,
            length,
            gain,
        };
        // The pitch of a meter `at` this far along.
        let pitch = |at: f32| 260. + at.clamp(0., 1.) * 900.;
        match self {
            Self::Tick { at } => vec![tone(Square, pitch(at), pitch(at), 0.022, 0.07, 0.)],
            Self::Detent => vec![
                tone(Triangle, 1400., 1400., 0.012, 0.12, 0.),
                tone(Triangle, 1400., 1400., 0.012, 0.08, 0.035),
            ],
            Self::EndStop => vec![tone(Sine, 120., 70., 0.1, 0.3, 0.)],
            Self::Reset { from } => vec![tone(Triangle, pitch(from), 700., 0.17, 0.14, 0.)],
            Self::Switch { on } => {
                let hz = if on { 660. } else { 420. };
                vec![
                    tone(Noise(Pass::High(3500.)), 0., 0., 0.018, 0.18, 0.),
                    tone(Square, hz, hz, 0.035, 0.08, 0.03),
                ]
            }
            Self::Hover => vec![tone(Sine, 2200., 2200., 0.012, 0.025, 0.)],
            Self::Press => vec![
                tone(Noise(Pass::High(3000.)), 0., 0., 0.015, 0.14, 0.),
                tone(Sine, 190., 95., 0.06, 0.2, 0.),
            ],
            Self::Channel => vec![
                tone(Noise(Pass::Band(1600.)), 0., 0., 0.14, 0.2, 0.),
                tone(Square, 60., 60., 0.12, 0.04, 0.),
            ],
            Self::Fold => vec![tone(Triangle, 380., 760., 0.08, 0.1, 0.)],
            Self::PowerOn => vec![
                // The degauss coil's thump, then the flyback transformer's whine.
                tone(Sine, 48., 60., 0.55, 0.4, 0.),
                tone(Noise(Pass::Low(300.)), 0., 0., 0.35, 0.12, 0.),
                tone(Sine, 6000., 9000., 0.5, 0.012, 0.1),
            ],
            Self::Preset => vec![
                tone(Square, 988., 988., 0.07, 0.1, 0.),
                tone(Square, 1319., 1319., 0.28, 0.1, 0.07),
            ],
            Self::Done => [523., 659., 784., 1047.]
                .into_iter()
                .enumerate()
                .map(|(i, hz)| {
                    let length = if i == 3 { 0.35 } else { 0.1 };
                    tone(Triangle, hz, hz, length, 0.14, i as f32 * 0.075)
                })
                .collect(),
        }
    }
}

/// Sounds cued this frame.
#[derive(Clone, Default)]
struct Cued(Vec<Sound>);

/// What the pointer was over last frame, to tell when it reaches something new.
#[derive(Clone, Default)]
struct Hovered(egui::IdSet);

fn cued_id() -> egui::Id {
    egui::Id::new("crtsim sounds cued")
}

fn hovered_id() -> egui::Id {
    egui::Id::new("crtsim sounds hovered")
}

/// Plays `sound` once this frame is done, if the person hears its group.
pub fn cue(ctx: &egui::Context, sound: Sound) {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Cued>(cued_id())
            .0
            .push(sound)
    });
}

/// The sounds of the frame just drawn: those widgets cued, then, for a click that cued none, a
/// press, a switch's flip or a section's slide, as egui reports the click; and a hover when the
/// pointer reaches something to click. Call once a frame, after drawing.
pub fn take(ctx: &egui::Context) -> Vec<Sound> {
    let mut sounds = ctx.data_mut(|data| data.remove_temp::<Cued>(cued_id()).unwrap_or_default().0);
    let clicked = ctx.output(|output| {
        output
            .events
            .iter()
            .find_map(|event| match event {
                egui::output::OutputEvent::Clicked(info) => Some(clicked(info)),
                _ => None,
            })
            .flatten()
    });
    // A widget that cued a sound for its own click, such as a channel, has said how it sounds.
    if let Some(sound) = clicked.filter(|_| sounds.is_empty()) {
        sounds.push(sound);
    }
    let hovered = ctx.viewport(|viewport| viewport.interact_widgets.hovered.clone());
    let before = ctx.data_mut(|data| {
        std::mem::replace(
            &mut data.get_temp_mut_or_default::<Hovered>(hovered_id()).0,
            hovered.clone(),
        )
    });
    let reached = hovered.iter().any(|id| {
        !before.contains(id)
            && ctx
                .read_response(*id)
                .is_some_and(|response| response.sense.senses_click() && response.enabled())
    });
    if reached && crate::interface::effects(ctx).screen() {
        sounds.push(Sound::Hover);
    }
    sounds
}

/// The sound of a click egui reports, by what was clicked.
fn clicked(info: &egui::WidgetInfo) -> Option<Sound> {
    use egui::WidgetType;
    match info.typ {
        WidgetType::Checkbox => Some(Sound::Switch {
            on: info.selected.unwrap_or(true),
        }),
        WidgetType::CollapsingHeader => Some(Sound::Fold),
        // A meter sounds as it moves, and typing in a box is heard as the keyboard.
        WidgetType::Slider
        | WidgetType::DragValue
        | WidgetType::TextEdit
        | WidgetType::Label
        | WidgetType::Other => None,
        _ => Some(Sound::Press),
    }
}

/// Plays sounds through the computer's or the browser's audio output, opened the first time
/// one is heard, so a person who never turns sounds on never has one opened.
#[derive(Default)]
pub struct Speaker {
    output: Option<Output>,
    /// Why the output could not be opened; then no sound is tried again.
    failed: Option<String>,
    /// When each spaced sound last played.
    last: Vec<(std::mem::Discriminant<Sound>, Instant)>,
}

struct Output {
    _stream: cpal::Stream,
    voices: mpsc::Sender<Vec<Voice>>,
}

impl Speaker {
    /// Plays those of `sounds` that `settings` hears. The first time the audio output cannot be
    /// opened, says why; after that, stays silent.
    pub fn play(&mut self, sounds: &[Sound], settings: &Sounds) -> Option<String> {
        let voices = self.heard(sounds, settings, Instant::now());
        if voices.is_empty() || self.failed.is_some() {
            return None;
        }
        if self.output.is_none() {
            match Output::open() {
                Ok(output) => self.output = Some(output),
                Err(e) => {
                    let message = format!("Interface sounds are unavailable: {e:#}");
                    self.failed = Some(message.clone());
                    return Some(message);
                }
            }
        }
        // The stream ending, as when its device goes away, leaves nothing to play to; the next
        // sound opens the output again.
        if let Some(output) = &self.output {
            if output.voices.send(voices).is_err() {
                self.output = None;
            }
        }
        None
    }

    /// The voices of those of `sounds` that `settings` hears, at their volume, leaving out any
    /// that come too soon after the last of their kind.
    fn heard(&mut self, sounds: &[Sound], settings: &Sounds, now: Instant) -> Vec<Voice> {
        let mut voices = vec![];
        for &sound in sounds.iter().filter(|sound| settings.hears(sound.group())) {
            let kind = std::mem::discriminant(&sound);
            match self.last.iter_mut().find(|(each, _)| *each == kind) {
                Some((_, last)) if now.duration_since(*last) < sound.spacing() => continue,
                Some((_, last)) => *last = now,
                None => self.last.push((kind, now)),
            }
            voices.extend(sound.voices().into_iter().map(|voice| Voice {
                gain: voice.gain * settings.volume,
                ..voice
            }));
        }
        voices
    }
}

impl Output {
    fn open() -> anyhow::Result<Self> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host()
            .default_output_device()
            .ok_or_else(|| anyhow::anyhow!("no audio output was found"))?;
        let supported = device.default_output_config()?;
        let config = supported.config();
        let (send, receive) = mpsc::channel();
        let mixer = Mixer::new(config.sample_rate as f32, receive);
        let channels = usize::from(config.channels.max(1));
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream::<f32>(&device, config, mixer, channels),
            cpal::SampleFormat::I16 => stream::<i16>(&device, config, mixer, channels),
            cpal::SampleFormat::U16 => stream::<u16>(&device, config, mixer, channels),
            cpal::SampleFormat::I32 => stream::<i32>(&device, config, mixer, channels),
            other => anyhow::bail!("the audio output takes {other} samples"),
        }?;
        stream.play()?;
        Ok(Self {
            _stream: stream,
            voices: send,
        })
    }
}

/// A stream to `device` that `mixer` fills, the same on every channel.
fn stream<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut mixer: Mixer,
    channels: usize,
) -> anyhow::Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::DeviceTrait;
    let mut mono = vec![];
    Ok(device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| {
            mono.resize(data.len() / channels, 0.);
            mixer.fill(&mut mono);
            for (frame, &sample) in data.chunks_mut(channels).zip(&mono) {
                frame.fill(T::from_sample(sample));
            }
        },
        // A dropped buffer is a click at worst; there is no one to tell from the audio thread.
        |_| {},
        None,
    )?)
}

/// The kind of sound a voice makes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Wave {
    Sine,
    Triangle,
    Square,
    /// White noise through a filter.
    Noise(Pass),
}

/// Which frequencies a filter lets through: those below, above or around its own.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pass {
    Low(f32),
    High(f32),
    Band(f32),
}

/// One tone or burst of noise, in seconds from when its sound plays.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Voice {
    wave: Wave,
    /// The pitch it starts at, in hertz; it glides evenly in proportion to `to`.
    from: f32,
    to: f32,
    start: f32,
    length: f32,
    /// How loud it starts, before it fades away.
    gain: f32,
}

/// A voice playing.
struct Playing {
    voice: Voice,
    /// Samples since its sound began.
    sample: u32,
    phase: f32,
    filter: Biquad,
}

/// The most voices that play at once; the oldest give way to new ones.
const VOICES: usize = 48;

/// Adds up the voices playing, sample by sample, on the audio output's own thread.
struct Mixer {
    rate: f32,
    playing: Vec<Playing>,
    incoming: mpsc::Receiver<Vec<Voice>>,
    /// The noise's state: xorshift, which needs no more than a word.
    noise: u32,
}

impl Mixer {
    fn new(rate: f32, incoming: mpsc::Receiver<Vec<Voice>>) -> Self {
        Self {
            rate,
            playing: vec![],
            incoming,
            noise: 0x9e37_79b9,
        }
    }

    fn fill(&mut self, out: &mut [f32]) {
        while let Ok(voices) = self.incoming.try_recv() {
            for voice in voices {
                let filter = match voice.wave {
                    Wave::Noise(filter) => Biquad::new(filter, self.rate),
                    _ => Biquad::default(),
                };
                self.playing.push(Playing {
                    voice,
                    sample: 0,
                    phase: 0.,
                    filter,
                });
            }
            let excess = self.playing.len().saturating_sub(VOICES);
            self.playing.drain(..excess);
        }
        for sample in out.iter_mut() {
            let mut sum = 0.;
            for playing in &mut self.playing {
                sum += playing.next(self.rate, &mut self.noise);
            }
            // Loud chords stay within the output's range, bending rather than clipping.
            *sample = sum.tanh();
        }
        let rate = self.rate;
        self.playing.retain(|playing| {
            (playing.sample as f32) < (playing.voice.start + playing.voice.length) * rate
        });
    }
}

impl Playing {
    fn next(&mut self, rate: f32, noise: &mut u32) -> f32 {
        let voice = self.voice;
        let t = self.sample as f32 / rate - voice.start;
        self.sample += 1;
        if t < 0. || t >= voice.length {
            return 0.;
        }
        let along = t / voice.length;
        let hz = voice.from * (voice.to / voice.from.max(1.)).powf(along);
        self.phase = (self.phase + hz / rate).fract();
        let wave = match voice.wave {
            Wave::Sine => (self.phase * std::f32::consts::TAU).sin(),
            Wave::Triangle => 1. - 4. * (self.phase - 0.5).abs(),
            Wave::Square => {
                if self.phase < 0.5 {
                    1.
                } else {
                    -1.
                }
            }
            Wave::Noise(_) => {
                *noise ^= *noise << 13;
                *noise ^= *noise >> 17;
                *noise ^= *noise << 5;
                self.filter.next(*noise as f32 / u32::MAX as f32 * 2. - 1.)
            }
        };
        wave * envelope(t, voice.length, voice.gain)
    }
}

/// How loud a voice is `t` seconds in: rising over 4 ms, then dying away evenly in proportion,
/// to a ten-thousandth by its end.
fn envelope(t: f32, length: f32, gain: f32) -> f32 {
    const ATTACK: f32 = 0.004;
    if t < ATTACK {
        gain * t / ATTACK
    } else {
        gain * (1e-4_f32 / gain.max(1e-4)).powf((t - ATTACK) / (length - ATTACK).max(1e-3))
    }
}

/// A two-pole filter, after Robert Bristow-Johnson's cookbook, with a Q of 0.7.
#[derive(Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(filter: Pass, rate: f32) -> Self {
        let hz = match filter {
            Pass::Low(hz) | Pass::High(hz) | Pass::Band(hz) => hz,
        };
        let w = std::f32::consts::TAU * hz.min(rate * 0.45) / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2. * 0.7);
        let b = match filter {
            Pass::Low(_) => [(1. - cos) / 2., 1. - cos, (1. - cos) / 2.],
            Pass::High(_) => [(1. + cos) / 2., -(1. + cos), (1. + cos) / 2.],
            Pass::Band(_) => [alpha, 0., -alpha],
        };
        let a0 = 1. + alpha;
        Self {
            b: b.map(|b| b / a0),
            a: [-2. * cos / a0, (1. - alpha) / a0],
            ..Default::default()
        }
    }

    fn next(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: [Sound; 12] = [
        Sound::Tick { at: 0.5 },
        Sound::Detent,
        Sound::EndStop,
        Sound::Reset { from: 0.8 },
        Sound::Switch { on: true },
        Sound::Hover,
        Sound::Press,
        Sound::Channel,
        Sound::Fold,
        Sound::PowerOn,
        Sound::Preset,
        Sound::Done,
    ];

    /// `sounds` as the mixer plays them, at 48 kHz, for a second.
    fn rendered(sounds: &[Sound]) -> Vec<f32> {
        let (send, receive) = mpsc::channel();
        let mut mixer = Mixer::new(48_000., receive);
        for sound in sounds {
            send.send(sound.voices()).unwrap();
        }
        let mut out = vec![0.; 48_000];
        mixer.fill(&mut out);
        assert!(
            mixer.playing.is_empty(),
            "every sound is over within a second"
        );
        out
    }

    #[test]
    fn every_sound_is_heard_stays_in_range_and_ends() {
        for sound in EVERY {
            let out = rendered(&[sound]);
            let loudest = out.iter().fold(0f32, |max, s| max.max(s.abs()));
            assert!(out.iter().all(|s| s.is_finite()), "{sound:?}");
            assert!(loudest > 0.005 && loudest <= 1., "{sound:?}: {loudest}");
        }
        // All at once, as a busy frame might ask, still within the output's range.
        let all = rendered(&EVERY);
        assert!(all.iter().all(|s| s.abs() <= 1.));
    }

    #[test]
    fn a_meter_ticks_higher_the_further_along_it_is() {
        let pitch = |at| Sound::Tick { at }.voices()[0].from;
        assert!(pitch(0.) < pitch(0.5) && pitch(0.5) < pitch(1.));
    }

    #[test]
    fn only_chosen_groups_are_heard_and_quick_ticks_are_spaced() {
        let mut speaker = Speaker::default();
        let mut settings = Sounds {
            on: true,
            volume: 0.5,
            ..Default::default()
        };
        let start = Instant::now();
        let tick = Sound::Tick { at: 0.2 };
        let heard = speaker.heard(&[tick], &settings, start);
        assert_eq!(heard.len(), 1);
        assert_eq!(heard[0].gain, tick.voices()[0].gain * 0.5);
        let soon = start + Duration::from_millis(10);
        assert!(speaker.heard(&[tick], &settings, soon).is_empty());
        let later = start + Duration::from_millis(30);
        assert_eq!(speaker.heard(&[tick], &settings, later).len(), 1);
        // A press is never spaced: two buttons pressed quickly are both heard.
        assert_eq!(
            speaker
                .heard(&[Sound::Press, Sound::Press], &settings, later)
                .len(),
            4
        );
        settings.navigation = false;
        assert!(speaker.heard(&[Sound::Press], &settings, later).is_empty());
        settings.on = false;
        assert!(speaker.heard(&[Sound::Done], &settings, later).is_empty());
    }

    #[test]
    fn a_click_sounds_by_what_was_clicked_unless_its_widget_cued_its_own() {
        let ctx = egui::Context::default();
        let frame = |clicked: Option<egui::WidgetType>, own: Option<Sound>| {
            let mut sounds = vec![];
            let mut output = ctx.run_ui(Default::default(), |ui| {
                if let Some(typ) = clicked {
                    let info = egui::WidgetInfo::selected(typ, true, false, "Clicked");
                    ui.ctx().output_mut(|o| {
                        o.events.push(egui::output::OutputEvent::Clicked(info));
                    });
                }
                if let Some(own) = own {
                    cue(ui.ctx(), own);
                }
                // As the app does, at the end of the frame, before its output is handed on.
                sounds = take(ui.ctx());
            });
            // No renderer takes the font atlas in a test, so its upload is dropped deliberately.
            output.textures_delta.clear();
            sounds
        };
        use egui::WidgetType;
        assert_eq!(frame(None, None), vec![]);
        assert_eq!(frame(Some(WidgetType::Button), None), vec![Sound::Press]);
        assert_eq!(
            frame(Some(WidgetType::Checkbox), None),
            vec![Sound::Switch { on: false }]
        );
        assert_eq!(frame(Some(WidgetType::TextEdit), None), vec![]);
        assert_eq!(frame(Some(WidgetType::Slider), None), vec![]);
        assert_eq!(
            frame(Some(WidgetType::RadioButton), Some(Sound::Channel)),
            vec![Sound::Channel]
        );
    }
}
