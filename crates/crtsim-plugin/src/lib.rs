//! What every video editor's effect shares: one renderer for the editor's process, the look
//! an effect's parameters name, and a clip's frames rendered as the app renders a video's, in
//! whatever order the editor asks for them.
//!
//! An editor renders frames in order when it exports, and in any order while someone scrubs
//! or plays from somewhere. A frame that follows the last one rendered goes on with its glow; any
//! other starts from as many frames before it as it takes for older glow to fade below half an
//! 8-bit step, so a frame comes out the same, to within a step, whichever way it was reached.
//! See docs/EDITORS.md.

use anyhow::{bail, Context, Result};
use crtsim_core::config::{Config, Phase};
use crtsim_core::{presets, Renderer, Sequence};
use image::RgbaImage;
use std::sync::Mutex;

/// One value of a choice an effect offers: the word a frei0r host is given it by, the label
/// an OpenFX host shows for it, and what it chooses.
#[derive(Clone, Copy, Debug)]
pub struct Choice<T> {
    pub key: &'static str,
    pub label: &'static str,
    pub value: T,
}

/// The tube shapes an effect offers: the look's own, or one of these.
pub const TUBES: [Choice<Option<f32>>; 4] = [
    Choice {
        key: "look",
        label: "The look's own",
        value: None,
    },
    Choice {
        key: "4:3",
        label: "4:3",
        value: Some(4. / 3.),
    },
    Choice {
        key: "16:10",
        label: "16:10",
        value: Some(16. / 10.),
    },
    Choice {
        key: "16:9",
        label: "16:9",
        value: Some(16. / 9.),
    },
];

/// How a clip's frames follow each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Timing {
    /// The glow carried from frame to frame, the artifacts held still: the app's default for
    /// video.
    #[default]
    Stable,
    /// The artifacts alternating each frame, as an NTSC set's did at 60 frames a second.
    Alternating,
    /// Each frame on its own, with no glow carried between them.
    Off,
}

impl Timing {
    pub const ALL: [Choice<Timing>; 3] = [
        Choice {
            key: "stable",
            label: "Glow between frames, artifacts still",
            value: Timing::Stable,
        },
        Choice {
            key: "alternating",
            label: "Glow between frames, artifacts alternating",
            value: Timing::Alternating,
        },
        Choice {
            key: "off",
            label: "Each frame on its own",
            value: Timing::Off,
        },
    ];
}

/// The look an effect's parameters name.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    /// An included preset, by name.
    pub preset: String,
    /// A preset's JSON, as the app or the command line saves it, in place of `preset` when
    /// given.
    pub file: Option<std::path::PathBuf>,
    /// The tube's shape, when not the look's own.
    pub tube: Option<f32>,
    pub timing: Timing,
}

/// The look an effect starts with: the app's for ordinary pictures, which a clip's frames are.
pub const DEFAULT_PRESET: &str = "General image";

impl Default for Look {
    fn default() -> Self {
        Self {
            preset: DEFAULT_PRESET.into(),
            file: None,
            tube: None,
            timing: Timing::Stable,
        }
    }
}

/// The included presets' names, in the gallery's order.
pub fn preset_names() -> Vec<&'static str> {
    presets::included().into_iter().map(|p| p.name).collect()
}

impl Look {
    /// The settings a clip of `size` frames renders with: the look's, drawn at the frame's
    /// own size, with its timing.
    pub fn config(&self, size: (u32, u32)) -> Result<Config> {
        let mut config = match &self.file {
            Some(path) => {
                let bytes = std::fs::read(path)
                    .with_context(|| format!("Cannot read the look {}", path.display()))?;
                Config::from_json_slice(&bytes)
                    .with_context(|| format!("{} is not a look", path.display()))?
            }
            None => match presets::named(&self.preset) {
                Some(preset) => preset.config,
                None => bail!("No included look is called {:?}", self.preset),
            },
        };
        config.output = format!("{}x{}", size.0, size.1);
        if let Some(tube) = self.tube {
            config.tube_aspect = tube;
        }
        match self.timing {
            Timing::Stable => config.phase = Phase::Stable,
            Timing::Alternating => config.phase = Phase::Alternating,
            Timing::Off => {
                config.persistence = [0.; 3];
                config.warmup = 0;
            }
        }
        config.validate()?;
        Ok(config)
    }
}

/// The longest any frame starts from before itself.
pub const MOST_HISTORY: u32 = 60;

/// How many frames before a frame its render starts from, at `fps` frames a second, so the
/// glow of any older frame has faded below half an 8-bit step: none for a look with no glow,
/// and at most `MOST_HISTORY`, past which the slowest glows still differ a little.
pub fn history(config: &Config, fps: f64) -> u32 {
    let strongest = config.persistence.iter().copied().fold(0_f32, f32::max);
    if strongest <= 0. || fps <= 0. {
        return 0;
    }
    // Persistence is a tick's at 60 a second; a frame lasts 60 / fps ticks.
    let per_frame = f64::from(strongest).powf(60. / fps);
    if per_frame <= 0. {
        return 0;
    }
    let frames = (1_f64 / 512.).ln() / per_frame.ln();
    (frames.ceil().max(0.) as u32).min(MOST_HISTORY)
}

/// Runs `render` on the process's renderer, made the first time it is needed and shared by
/// every effect in it. One render runs at a time. A renderer that fails is dropped, and the
/// next render makes a new one.
pub fn with_renderer<T>(render: impl FnOnce(&Renderer) -> Result<T>) -> Result<T> {
    static RENDERER: Mutex<Option<Renderer>> = Mutex::new(None);
    let mut renderer = RENDERER.lock().unwrap_or_else(|e| e.into_inner());
    if renderer.is_none() {
        let made = pollster::block_on(Renderer::new(wgpu::Backends::PRIMARY))
            .context("CRTSim Renderer could not start")?;
        *renderer = Some(made);
    }
    let result = render(renderer.as_ref().expect("made above"));
    if result.is_err() {
        *renderer = None;
    }
    result
}

/// One effect's clip: the sequence its last frame was rendered in, to go on from.
#[derive(Default)]
pub struct Clip {
    sequence: Option<Sequence>,
    /// The settings and size the sequence was made with.
    key: String,
    /// The last frame rendered, by number.
    last: Option<i64>,
}

/// What a frame's render starts from.
#[derive(Debug, PartialEq, Eq)]
pub enum Start {
    /// The frame before was the last rendered, with these settings: go on.
    Following,
    /// A new sequence from this frame, `start` up to the frame itself.
    From(i64),
}

impl Clip {
    /// Where frame `frame` of a clip whose first frame is `first` starts, for settings `key`,
    /// going `history` frames back. A sequence started later than `first` starts on a frame
    /// of the same parity, so its artifacts and fields alternate in step with one started
    /// at `first`.
    pub fn start(&self, key: &str, frame: i64, first: i64, history: u32) -> Start {
        if self.sequence.is_some() && self.key == key && self.last == Some(frame - 1) {
            return Start::Following;
        }
        let mut start = (frame - i64::from(history)).max(first).min(frame);
        if (start - first).rem_euclid(2) == 1 {
            start -= 1;
        }
        Start::From(start)
    }

    /// Renders frame `frame` of a clip `fps` frames a second whose first frame is `first`:
    /// `picture`, after the frames before it that its glow needs, which `fetch` gives by
    /// number, or none if the editor cannot. `cancelled` is asked between frames.
    #[allow(clippy::too_many_arguments)]
    pub fn frame(
        &mut self,
        renderer: &Renderer,
        config: &Config,
        fps: f64,
        frame: i64,
        first: i64,
        picture: &RgbaImage,
        mut fetch: impl FnMut(i64) -> Result<Option<RgbaImage>>,
        cancelled: impl Fn() -> bool,
    ) -> Result<RgbaImage> {
        let key = format!(
            "{}x{} {}",
            picture.width(),
            picture.height(),
            serde_json::to_string(config)?
        );
        let seconds = 1. / fps.max(1.);
        let mut sequence = match self.start(&key, frame, first, history(config, fps)) {
            Start::Following => self.sequence.take().expect("following a sequence"),
            Start::From(start) => {
                let mut sequence = Sequence::clip();
                for earlier in start..frame {
                    if cancelled() {
                        bail!("Render cancelled");
                    }
                    match fetch(earlier)? {
                        Some(before) => step(renderer, &mut sequence, &before, config, seconds)?,
                        // A frame the editor cannot give breaks the glow; start after it.
                        None => sequence = Sequence::clip(),
                    }
                }
                sequence
            }
        };
        step(renderer, &mut sequence, picture, config, seconds)?;
        let image = pollster::block_on(renderer.read(&mut sequence))?;
        self.sequence = Some(sequence);
        self.key = key;
        self.last = Some(frame);
        Ok(image)
    }

    /// Forgets the sequence, so the next frame starts afresh.
    pub fn forget(&mut self) {
        *self = Self::default();
    }
}

fn step(
    renderer: &Renderer,
    sequence: &mut Sequence,
    picture: &RgbaImage,
    config: &Config,
    seconds: f64,
) -> Result<()> {
    sequence.lasted(seconds);
    pollster::block_on(renderer.frame(sequence, picture, config, None, |_| {}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_is_drawn_at_the_frames_size_with_its_timing_and_tube() {
        let look = Look {
            tube: Some(16. / 9.),
            timing: Timing::Off,
            ..Look::default()
        };
        let config = look.config((1920, 1080)).unwrap();
        assert_eq!(config.output_size((1920, 1080)).unwrap(), (1920, 1080));
        assert_eq!(
            (config.tube_aspect, config.persistence),
            (16. / 9., [0.; 3])
        );
        let ntsc = Look {
            preset: "NTSC 480i".into(),
            timing: Timing::Alternating,
            ..Look::default()
        };
        assert_eq!(ntsc.config((720, 480)).unwrap().phase, Phase::Alternating);
        let missing = Look {
            preset: "Not a look".into(),
            ..Look::default()
        };
        assert!(missing.config((8, 8)).is_err());
        assert!(preset_names().contains(&DEFAULT_PRESET));
    }

    #[test]
    fn a_saved_preset_is_a_look_drawn_at_the_frames_size() {
        let mut saved = Config::general();
        saved.output = "1600x900".into();
        saved.tube_aspect = 16. / 10.;
        let path = std::env::temp_dir().join(format!("crtsim-plugin-{}.json", std::process::id()));
        std::fs::write(&path, serde_json::to_string_pretty(&saved).unwrap()).unwrap();
        let look = Look {
            preset: "Not a look, and not read".into(),
            file: Some(path.clone()),
            ..Look::default()
        };
        let config = look.config((720, 480));
        std::fs::remove_file(&path).unwrap();
        let config = config.unwrap();
        assert_eq!(
            (config.output.as_str(), config.tube_aspect),
            ("720x480", 16. / 10.)
        );
        let missing = Look {
            file: Some(path),
            ..Look::default()
        };
        assert!(missing.config((8, 8)).is_err());
    }

    #[test]
    fn history_reaches_back_until_the_glow_has_faded() {
        let config = Config::general();
        // 0.7 a tick: 2.5 ticks a frame at 24 a second fades below 1/512 in 7 frames.
        assert_eq!(history(&config, 24.), 7);
        assert!(history(&config, 60.) > history(&config, 24.));
        let none = Config {
            persistence: [0.; 3],
            ..config.clone()
        };
        assert_eq!(history(&none, 24.), 0);
        let lingering = Config {
            persistence: [0.999; 3],
            ..config
        };
        assert_eq!(history(&lingering, 60.), MOST_HISTORY);
    }

    #[test]
    fn a_frame_goes_on_from_the_last_or_starts_back_in_step() {
        let clip = Clip::default();
        assert_eq!(
            clip.start("look", 100, 0, 7),
            Start::From(92),
            "even, as frame 0"
        );
        assert_eq!(clip.start("look", 101, 0, 7), Start::From(94));
        assert_eq!(
            clip.start("look", 3, 0, 7),
            Start::From(0),
            "not before the clip"
        );
        assert_eq!(clip.start("look", 5, 1, 0), Start::From(5));
        assert_eq!(
            clip.start("look", 6, 1, 0),
            Start::From(5),
            "in step with frame 1"
        );
        let following = Clip {
            sequence: Some(Sequence::clip()),
            key: "look".into(),
            last: Some(41),
        };
        assert_eq!(following.start("look", 42, 0, 7), Start::Following);
        assert_eq!(following.start("other", 42, 0, 7), Start::From(34));
        assert_eq!(following.start("look", 44, 0, 7), Start::From(36));
    }
}
