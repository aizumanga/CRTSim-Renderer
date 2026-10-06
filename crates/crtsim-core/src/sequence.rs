//! Frames rendered one after another, sharing the CRT's history: a still, or a video's frames at
//! its own rate.
use crate::{
    config::{Config, Phase},
    Workspace,
};
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

/// Frames that share one CRT's history, so the glow of one persists into the next. A new
/// sequence starts from cleared history and warms up before its first frame.
///
/// A sequence belongs to the renderer that first draws it, and to one signal size, output size
/// and color mode. Start a new one after seeking or changing those; an editing sequence starts
/// again by itself.
pub struct Sequence {
    pub(crate) workspace: Option<Workspace>,
    /// Ticks simulated so far.
    pub(crate) tick: u64,
    pace: Pace,
}

enum Pace {
    Still,
    Video { timing: Timing, fps: f64 },
    Editing,
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
    /// settings before it fades. Given a new signal size, output size or color mode, it starts
    /// again with a still's warm-up rather than failing.
    pub fn editing() -> Self {
        Self::new(Pace::Editing)
    }

    /// A video's frames, `fps` of them a second, following `timing`.
    pub fn video(timing: Timing, fps: f64) -> Self {
        Self::new(Pace::Video { timing, fps })
    }

    fn new(pace: Pace) -> Self {
        Self {
            workspace: None,
            tick: 0,
            pace,
        }
    }

    /// Ticks the next frame runs: a new sequence's warm-up and its own, then one a frame, or a
    /// couple while editing.
    pub(crate) fn ticks(&self, warmup: u32) -> u32 {
        match self.pace {
            _ if self.tick == 0 => warmup + 1,
            Pace::Editing => EDITING_TICKS,
            Pace::Still | Pace::Video { .. } => 1,
        }
    }

    /// Whether the sequence starts again, rather than failing, when its sizes or color mode
    /// change.
    pub(crate) fn restarts(&self) -> bool {
        matches!(self.pace, Pace::Editing)
    }

    /// The settings this sequence renders `config` with: a still's and an editing sequence's
    /// as they are, a video's adjusted to its timing and rate.
    pub fn timed(&self, config: &Config) -> Config {
        let mut c = config.clone();
        let Pace::Video { timing, fps } = self.pace else {
            return c;
        };
        match timing {
            Timing::Stable => {
                c.phase = Phase::Stable;
                for weight in &mut c.persistence {
                    *weight = weight.powf((60. / fps) as f32);
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
}
