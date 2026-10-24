//! Frames rendered one after another, sharing the CRT's history: a still, or a video's frames at
//! its own rate.
use crate::{
    config::{ColorMode, Config, Phase},
    Workspace,
};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// How a video's frames follow each other in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Timing {
    /// At the video's own rate, with the artifacts held still and the glow's decay adjusted to
    /// that rate.
    #[default]
    Stable,
    /// At 60 frames a second, with the artifacts alternating as an NTSC set's do.
    Ntsc60,
    /// At the video's own rate, with no glow carried from one frame to the next.
    Disabled,
}

/// What a sequence is made for: the size of its signal and of its output, and its color mode.
/// The CRT's history is the signal's, so frames of a new output size go on with it; a new
/// signal size or color mode needs a new sequence, which an editing or host sequence starts by
/// itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub signal: (u32, u32),
    pub output: (u32, u32),
    pub color_mode: ColorMode,
}

impl Shape {
    /// Whether a sequence of this shape goes on into frames of `next` with its history: when
    /// at most their output sizes differ.
    pub fn keeps_history(&self, next: &Shape) -> bool {
        self.signal == next.signal && self.color_mode == next.color_mode
    }
}

/// What becomes of a sequence's CRT for its next frame.
#[derive(Debug, PartialEq)]
pub(crate) enum Follow {
    /// It goes on as it is.
    Same,
    /// It goes on with its history, drawn at another output size.
    Reshape,
    /// It starts again from cleared history.
    Restart,
}

/// Frames that share one CRT's history, so the glow of one persists into the next. A new
/// sequence starts from cleared history and warms up before its first frame.
///
/// A sequence belongs to the renderer that first draws it, and to one signal size and color
/// mode: a new output size goes on with its history. Start a new one after seeking or changing
/// its signal size or color mode; an editing or host sequence starts again by itself.
pub struct Sequence {
    pub(crate) workspace: Option<Workspace>,
    /// Ticks simulated so far.
    pub(crate) tick: u64,
    /// Sixtieths of a second from the first tick to the latest, which TV static keeps time by.
    clock: f64,
    pace: Pace,
}

enum Pace {
    Still,
    Video {
        timing: Timing,
        fps: f64,
    },
    Editing,
    /// How long the host showed the frame before the one being drawn, and whether its first
    /// frame warms up.
    Host {
        seconds: f64,
        warm: bool,
    },
}

/// Ticks each frame of an editing sequence after its first. Even, so the artifact phase and the
/// interlaced field of every frame are the ones its first frame, a still, shows.
const EDITING_TICKS: u32 = 2;

impl Sequence {
    /// A still: one frame, rendered after warm-up with the settings as they are.
    pub fn still() -> Self {
        Self::new(Pace::Still)
    }

    /// The preview of settings being edited, as a game's picture follows its options menu: the
    /// first frame is a still, and each later one runs the same CRT on for a couple of ticks
    /// with the settings as they are by then, so an edit shows at once while the glow of the
    /// settings before it fades. A new output size goes on with its glow; given a new signal
    /// size or color mode, it starts again with a still's warm-up rather than failing.
    pub fn editing() -> Self {
        Self::new(Pace::Editing)
    }

    /// A video's frames, `fps` of them a second, following `timing`.
    pub fn video(timing: Timing, fps: f64) -> Self {
        Self::new(Pace::Video { timing, fps })
    }

    /// A host's own frames, as a game or an emulator draws them, each recorded with
    /// `Renderer::encode`. It starts without warm-up, so the glow builds as a set's would, and
    /// runs one tick a frame, whose glow decays by how long the frame before it lasted. A new
    /// output size keeps its history; a new signal size or color mode starts it again.
    pub fn host() -> Self {
        Self::new(Pace::Host {
            seconds: 1. / 60.,
            warm: false,
        })
    }

    /// A clip's frames, as a video editor's effect renders them: a host's sequence whose first
    /// frame warms up as a still does, so a clip starts with the glow of a picture already on
    /// screen. Each frame then decays by how long the one before it lasted, which
    /// [`Sequence::lasted`] says.
    pub fn clip() -> Self {
        Self::new(Pace::Host {
            seconds: 1. / 60.,
            warm: true,
        })
    }

    /// Whether this is a still, whose one frame a look may draw as the game's screenshots.
    pub(crate) fn still_frame(&self) -> bool {
        matches!(self.pace, Pace::Still)
    }

    /// Whether this is a host's sequence, which `Renderer::encode` draws. A clip's warms up,
    /// which takes waiting on the device, so `Renderer::frame` draws it.
    pub(crate) fn hosted(&self) -> bool {
        matches!(self.pace, Pace::Host { warm: false, .. })
    }

    /// Records how long the host showed the frame before the next one, which that frame's
    /// glow decays by, in a host's or a clip's sequence; others keep their own pace. A frame
    /// of no time, or a negative or unknown one, decays by nothing; one over a second decays
    /// as a second does, which leaves next to nothing either way.
    pub fn lasted(&mut self, seconds: f64) {
        if let Pace::Host { seconds: last, .. } = &mut self.pace {
            *last = if seconds.is_finite() {
                seconds.clamp(0., 1.)
            } else {
                0.
            };
        }
    }

    fn new(pace: Pace) -> Self {
        Self {
            workspace: None,
            tick: 0,
            clock: 0.,
            pace,
        }
    }

    /// Ticks the next frame runs: a new sequence's warm-up and its own, then one a frame, or a
    /// couple while editing. A host's sequence never warms up.
    pub(crate) fn ticks(&self, warmup: u32) -> u32 {
        match self.pace {
            Pace::Host { warm: false, .. } => 1,
            _ if self.tick == 0 => warmup + 1,
            Pace::Host { .. } => 1,
            Pace::Editing => EDITING_TICKS,
            Pace::Still | Pace::Video { .. } => 1,
        }
    }

    /// Starts the sequence again from cleared history, at its own pace.
    pub(crate) fn restart(&mut self) {
        self.workspace = None;
        self.tick = 0;
        self.clock = 0.;
    }

    /// Moves the clock on through the next frame's `ticks` ticks, saying where it stands at
    /// each, in sixtieths of a second: the frame's first tick comes as long after the frame
    /// before as that frame lasted, a video's frame or a host's; any other tick, a sixtieth
    /// after the tick before it.
    pub(crate) fn clocks(&mut self, ticks: u32) -> Vec<f64> {
        (0..ticks)
            .map(|step| {
                if self.tick + u64::from(step) > 0 {
                    self.clock += match self.pace {
                        Pace::Video { fps, .. } if step == 0 => 60. / fps,
                        Pace::Host { seconds, .. } if step == 0 => seconds * 60.,
                        _ => 1.,
                    };
                }
                self.clock
            })
            .collect()
    }

    /// Whether the sequence starts again, rather than failing, when its signal size or color
    /// mode change.
    fn restarts(&self) -> bool {
        matches!(self.pace, Pace::Editing | Pace::Host { .. })
    }

    /// How the sequence, drawn so far at shape `from`, follows into a frame of `to`. A new
    /// output size goes on with the history, as a set's picture would when its window is
    /// resized; a new signal size or color mode starts an editing or host sequence again, and is
    /// refused by a video's, whose frames would otherwise lose their glow partway through.
    pub(crate) fn follow(&self, from: &Shape, to: &Shape) -> Result<Follow> {
        Ok(if from == to {
            Follow::Same
        } else if from.keeps_history(to) {
            Follow::Reshape
        } else if self.restarts() {
            Follow::Restart
        } else {
            bail!("Start a new video sequence after changing signal size or color mode")
        })
    }

    /// The settings this sequence renders `config` with: a still's and an editing sequence's
    /// as they are, a video's adjusted to its timing and rate, and a host's glow to how long its
    /// last frame lasted, unless the look's glow decays once a frame, as the game's does.
    pub fn timed(&self, config: &Config) -> Config {
        let mut c = config.clone();
        let (timing, fps) = match self.pace {
            Pace::Video { timing, fps } => (timing, fps),
            Pace::Host { seconds, .. } => {
                // The original's persistence is a tick's at 60 a second.
                if !c.glow_per_frame {
                    for weight in &mut c.persistence {
                        *weight = weight.powf((seconds * 60.) as f32);
                    }
                }
                return c;
            }
            Pace::Still | Pace::Editing => return c,
        };
        match timing {
            Timing::Stable => {
                c.phase = Phase::Stable;
                if !c.glow_per_frame {
                    for weight in &mut c.persistence {
                        *weight = weight.powf((60. / fps) as f32);
                    }
                }
            }
            Timing::Ntsc60 => c.phase = Phase::Alternating,
            Timing::Disabled => {
                c.phase = Phase::Stable;
                c.persistence = [0.; 3];
                c.warmup = 0;
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_video_decays_by_its_rate_and_a_still_keeps_its_settings() {
        let config = Config::general();
        let at30 = Sequence::video(Timing::Stable, 30.).timed(&config);
        let at60 = Sequence::video(Timing::Stable, 60.).timed(&config);
        // Half the frames a second, so each keeps its glow for twice as long.
        assert!((at30.persistence[0] - at60.persistence[0].powi(2)).abs() < 0.00001);
        assert_eq!(at60.persistence, config.persistence);
        let disabled = Sequence::video(Timing::Disabled, 30.).timed(&config);
        assert_eq!((disabled.persistence, disabled.warmup), ([0.; 3], 0));
        let ntsc = Sequence::video(Timing::Ntsc60, 60.).timed(&config);
        assert_eq!(ntsc.phase, Phase::Alternating);
        assert_eq!(Sequence::still().timed(&config), config);
        assert_eq!(Sequence::editing().timed(&config), config);
    }

    #[test]
    fn the_clock_keeps_time_by_each_frames_length() {
        let mut video = Sequence::video(Timing::Stable, 30.);
        let mut clocks = vec![];
        for _ in 0..3 {
            let ticks = video.ticks(2);
            clocks.extend(video.clocks(ticks));
            video.tick += u64::from(ticks);
        }
        // Warm-up ticks a sixtieth apart, then frames a thirtieth apart.
        assert_eq!(clocks, [0., 1., 2., 4., 6.]);
        let mut host = Sequence::host();
        assert_eq!(host.clocks(1), [0.]);
        host.tick += 1;
        host.lasted(0.05);
        assert_eq!(host.clocks(1), [3.]);
        host.restart();
        assert_eq!(host.clocks(1), [0.]);
    }

    #[test]
    fn a_glow_decaying_per_frame_keeps_its_persistence_at_any_rate() {
        let config = Config {
            glow_per_frame: true,
            ..Config::general()
        };
        let at30 = Sequence::video(Timing::Stable, 30.).timed(&config);
        assert_eq!(at30.persistence, config.persistence);
        let mut host = Sequence::host();
        host.lasted(1. / 144.);
        assert_eq!(host.timed(&config).persistence, config.persistence);
        // By time, a 144 Hz frame keeps more of the glow than a sixtieth of a second does.
        let by_time = host.timed(&Config::general()).persistence;
        assert!(by_time[0] > config.persistence[0]);
    }

    #[test]
    fn an_editing_sequence_starts_as_a_still_and_keeps_its_phase() {
        let mut editing = Sequence::editing();
        assert_eq!(editing.ticks(16), Sequence::still().ticks(16));
        editing.tick = 17;
        assert_eq!(
            editing.ticks(16) % 2,
            0,
            "the last tick keeps the still's parity"
        );
        let mut video = Sequence::video(Timing::Stable, 60.);
        video.tick = 17;
        assert_eq!(video.ticks(16), 1);
        assert!(editing.restarts() && !video.restarts() && !Sequence::still().restarts());
    }

    #[test]
    fn a_new_output_size_keeps_the_history_and_a_new_signal_restarts_or_is_refused() {
        let shape = Shape {
            signal: (256, 224),
            output: (640, 480),
            color_mode: ColorMode::Reference,
        };
        let resized = Shape {
            output: (1280, 960),
            ..shape
        };
        let resampled = Shape {
            signal: (320, 240),
            ..shape
        };
        let linear = Shape {
            color_mode: ColorMode::LinearLight,
            ..shape
        };
        let sequences = [
            Sequence::video(Timing::Stable, 30.),
            Sequence::editing(),
            Sequence::host(),
            Sequence::clip(),
        ];
        for sequence in &sequences {
            assert_eq!(sequence.follow(&shape, &shape).unwrap(), Follow::Same);
            assert_eq!(sequence.follow(&shape, &resized).unwrap(), Follow::Reshape);
        }
        for other in [resampled, linear] {
            assert!(sequences[0].follow(&shape, &other).is_err(), "a video's");
            for sequence in &sequences[1..] {
                assert_eq!(sequence.follow(&shape, &other).unwrap(), Follow::Restart);
            }
        }
    }

    #[test]
    fn a_host_sequence_never_warms_up_and_decays_by_each_frames_length() {
        let config = Config::general();
        let mut host = Sequence::host();
        assert_eq!(host.ticks(16), 1, "no warm-up, even on the first frame");
        assert!(host.restarts() && host.hosted());
        assert_eq!(host.timed(&config), config, "a 60th of a second is a tick");
        host.lasted(1. / 30.);
        let slow = host.timed(&config);
        assert!((slow.persistence[0] - config.persistence[0].powi(2)).abs() < 0.00001);
        host.lasted(-1.);
        assert_eq!(host.timed(&config).persistence, [1.; 3]);
        host.lasted(f64::NAN);
        assert_eq!(host.timed(&config).persistence, [1.; 3]);
        assert!(!Sequence::video(Timing::Stable, 60.).hosted());
    }

    #[test]
    fn a_clip_warms_up_its_first_frame_then_runs_as_a_host_does() {
        let mut clip = Sequence::clip();
        assert_eq!(clip.ticks(16), 17, "its first frame warms up");
        clip.tick = 17;
        assert_eq!(clip.ticks(16), 1);
        assert!(!clip.hosted() && clip.restarts());
        clip.lasted(1. / 30.);
        let config = Config::general();
        let timed = clip.timed(&config);
        assert!((timed.persistence[0] - config.persistence[0].powi(2)).abs() < 0.00001);
    }
}
