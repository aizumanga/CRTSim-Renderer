//! Playing a video in the preview. The worker renders frames ahead into a small buffer and each
//! is shown when its time comes. Playback waits for a few frames before its clock starts, and
//! waits again whenever the renderer falls behind. Settings edited while it plays reach the
//! frames rendered next, as a game's picture follows its options menu.
use crate::*;
use std::collections::VecDeque;
use worker::{Feed, PlaybackFrame, PlayingSettings};

/// Frames buffered before the clock starts, or all the buffer holds if that is fewer.
const PREROLL: usize = 3;

/// A video playing: what the worker has rendered ahead, and the clock that shows it. Dropping
/// it stops the worker.
pub struct Playback {
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<Result<PlaybackFrame, String>>,
    buffered: VecDeque<PlaybackFrame>,
    /// When the clock started, and the media time it started from; none while buffering.
    clock: Option<(Instant, f64)>,
    /// The worker has sent its last frame, or stopped on an error.
    ended: bool,
    capacity: usize,
    /// How long one frame of the video shows, in seconds.
    frame_duration: f64,
    /// The media time of the frame shown last.
    shown: f64,
    /// What the worker renders with, which edits replace.
    settings: PlayingSettings,
    /// The size of the video's frames.
    source: (u32, u32),
}

/// How playing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Enough is buffered, and the clock has started.
    Playing,
    /// The renderer fell behind, and the clock waits for it.
    CatchingUp,
    /// Every frame has been shown.
    Finished,
}

/// What playback has for one frame of the interface.
#[derive(Default)]
pub struct Tick {
    /// The frame due now.
    pub frame: Option<PlaybackFrame>,
    /// How playing changed, if it did.
    pub status: Option<Status>,
    /// Why the worker stopped early. The frames it rendered before still play.
    pub error: Option<String>,
}

impl Playback {
    /// Playback of `video` from `position` in seconds, or from the beginning when that is its
    /// last frame, with frames rendered with `config` at its `output` size. Returns it with the
    /// worker's end.
    pub fn new(
        video: &crtsim_media::Video,
        position: f64,
        output: (u32, u32),
        config: Config,
    ) -> (Self, Feed) {
        let frame_duration = 1. / video.fps;
        let start = if position >= video.duration - frame_duration {
            0.
        } else {
            position
        };
        // About 128 MB across the channel and the buffer, and at least one source and rendered
        // frame in each.
        let pixels = |(width, height): (u32, u32)| u64::from(width) * u64::from(height);
        let pair_bytes = 4 * (pixels(video.size) + pixels(output));
        let capacity = ((64 * 1024 * 1024) / pair_bytes.max(1)).clamp(1, 4) as usize;
        let (frames, receive) = mpsc::sync_channel(capacity);
        let cancel = Arc::new(AtomicBool::new(false));
        let settings = PlayingSettings::new(config);
        let playback = Self {
            cancel: cancel.clone(),
            receive,
            buffered: VecDeque::new(),
            clock: None,
            ended: false,
            capacity,
            frame_duration,
            shown: start,
            settings: settings.clone(),
            source: video.size,
        };
        (
            playback,
            Feed {
                start,
                settings,
                frames,
                cancel,
            },
        )
    }

    /// Renders the frames from the next one on with `config`, unless it needs a CRT of another
    /// signal size, output size or colour mode than the one playing, which only a new playback
    /// can start. Whether it was taken. The frames already buffered play as they are, so the
    /// edit shows within a few frames without the playback stopping to wait for it.
    pub fn retune(&self, config: Config) -> bool {
        // What the CRT playing is made for.
        let made_for = |c: &Config| {
            (
                c.signal_size(self.source).ok(),
                c.output_size(self.source).ok(),
                c.color_mode,
            )
        };
        let next = made_for(&config);
        let taken = next.0.is_some() && next.1.is_some() && next == made_for(&self.settings.get());
        if taken {
            self.settings.set(config);
        }
        taken
    }

    /// Takes what the worker has rendered, and moves the clock on to `now`.
    pub fn tick(&mut self, now: Instant) -> Tick {
        let mut tick = Tick::default();
        while self.buffered.len() < self.capacity && !self.ended {
            match self.receive.try_recv() {
                Ok(Ok(frame)) => self.buffered.push_back(frame),
                Ok(Err(error)) => {
                    tick.error = Some(error);
                    self.ended = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.ended = true,
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        let prerolled = self.buffered.len() >= self.capacity.min(PREROLL)
            || (self.ended && !self.buffered.is_empty());
        if self.clock.is_none() && prerolled {
            self.clock = Some((now, self.buffered[0].time));
            tick.status = Some(Status::Playing);
        }
        if let Some((started, from)) = self.clock {
            let target = from + now.duration_since(started).as_secs_f64();
            // Frames already overdue are skipped rather than shown late.
            while self
                .buffered
                .front()
                .is_some_and(|frame| frame.time <= target)
            {
                tick.frame = self.buffered.pop_front();
            }
            if let Some(frame) = &tick.frame {
                self.shown = frame.time;
            }
            if self.buffered.is_empty() && !self.ended && target > self.shown + self.frame_duration
            {
                self.clock = None;
                tick.status = Some(Status::CatchingUp);
            }
        }
        if self.ended && self.buffered.is_empty() {
            tick.status = Some(Status::Finished);
        }
        tick
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl App {
    /// Stops the video playing, if one is, which frees the work lane.
    pub fn stop_playback(&mut self) {
        if self.playback.take().is_some() {
            self.status = "Playback paused".into();
        }
        self.lane.stopped_playing();
    }

    /// Plays the video from the frame on screen, or pauses it where it is.
    pub fn toggle_playback(&mut self) {
        if self.playback.is_some() {
            self.stop_playback();
        } else {
            self.start_playback();
        }
    }

    pub fn start_playback(&mut self) {
        let Some(timeline) = &self.source.timeline else {
            return;
        };
        let (video, time) = (timeline.video.clone(), timeline.time);
        self.stop_playback();
        let config = match self
            .config
            .with_max_output_side(self.source.input().dimensions(), self.preview.quality)
        {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(format!("Cannot play: {e:#}"));
                return;
            }
        };
        let output = config.output_size(video.size).unwrap_or(video.size);
        let (playback, feed) = Playback::new(&video, time, output, config);
        self.playback = Some(playback);
        self.preview.drop_pending();
        let job = Job::Playback {
            video,
            options: self.video_options.clone(),
            feed,
        };
        self.start(job, "Buffering · warming CRT history…".into());
    }

    /// Hands the settings in use to the video playing, if one is, for its next frames. Ones
    /// it cannot take play from the frame on screen again; ones that cannot be played at all
    /// pause it, leaving the preview to say why.
    pub(crate) fn retune_playback(&mut self) {
        let Some(playback) = &self.playback else {
            return;
        };
        let config = self
            .config
            .with_max_output_side(self.source.input().dimensions(), self.preview.quality);
        if config.is_ok_and(|config| playback.retune(config)) {
            return;
        }
        let restart = self.config.validate().is_ok();
        self.stop_playback();
        if restart {
            self.start_playback();
        }
    }

    pub(crate) fn tick_playback(&mut self, ctx: &egui::Context) {
        let Some(playback) = &mut self.playback else {
            return;
        };
        let tick = playback.tick(Instant::now());
        if let Some(error) = tick.error {
            self.error = Some(error);
        }
        if let Some(frame) = tick.frame {
            self.show_playing(ctx, frame);
        }
        match tick.status {
            Some(Status::Playing) => self.status = "Playing".into(),
            Some(Status::CatchingUp) => {
                self.status = "Buffering · renderer is catching up…".into();
            }
            Some(Status::Finished) => {
                self.stop_playback();
                self.status = "Playback finished".into();
                return;
            }
            None => {}
        }
        ctx.request_repaint_after(Duration::from_millis(8));
    }

    /// Shows a frame that has come due: its source as the original, and its CRT picture as the
    /// preview.
    fn show_playing(&mut self, ctx: &egui::Context, frame: PlaybackFrame) {
        let limit = ctx.input(|i| i.max_texture_side) as u32;
        let crt = texture(ctx, "playing-crt", &frame.crt, limit);
        self.preview.show_played(crt);
        self.source.played(frame.time, frame.source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crtsim_core::config::ColorMode;

    const MS: Duration = Duration::from_millis(1);

    /// A video of `duration` seconds at 25 frames per second.
    fn video(duration: f64, size: (u32, u32)) -> crtsim_media::Video {
        crtsim_media::Video {
            metadata: Default::default(),
            tracks: vec![],
            start: 0.,
            path: "clip.mkv".into(),
            size,
            fps: 25.,
            rate: "25/1".into(),
            duration,
            audio: false,
            audio_offset: 0.,
            hdr: false,
            stream: 0,
            frames: None,
            source: crtsim_media::Source::Ffmpeg,
            contents: None,
        }
    }

    fn frame(time: f64) -> Result<PlaybackFrame, String> {
        Ok(PlaybackFrame {
            time,
            source: RgbaImage::new(4, 4),
            crt: RgbaImage::new(4, 4),
        })
    }

    fn shown(tick: &Tick) -> Option<f64> {
        tick.frame.as_ref().map(|frame| frame.time)
    }

    #[test]
    fn it_buffers_before_it_starts_and_shows_each_frame_when_it_is_due() {
        let (mut playback, feed) =
            Playback::new(&video(10., (4, 4)), 0., (4, 4), Config::default());
        let start = Instant::now();
        feed.frames.send(frame(0.)).unwrap();
        let tick = playback.tick(start);
        assert!(
            tick.frame.is_none() && tick.status.is_none(),
            "still buffering"
        );
        feed.frames.send(frame(0.04)).unwrap();
        feed.frames.send(frame(0.08)).unwrap();
        let tick = playback.tick(start);
        assert_eq!(
            (tick.status, shown(&tick)),
            (Some(Status::Playing), Some(0.))
        );
        assert_eq!(shown(&playback.tick(start + 20 * MS)), None, "not due yet");
        assert_eq!(shown(&playback.tick(start + 40 * MS)), Some(0.04));
        feed.frames.send(frame(0.12)).unwrap();
        assert_eq!(
            shown(&playback.tick(start + 130 * MS)),
            Some(0.12),
            "an overdue frame is skipped"
        );
    }

    #[test]
    fn it_waits_when_the_renderer_falls_behind_and_resumes_from_the_next_frame() {
        let (mut playback, feed) =
            Playback::new(&video(10., (4, 4)), 0., (4, 4), Config::default());
        let start = Instant::now();
        for time in [0., 0.04, 0.08] {
            feed.frames.send(frame(time)).unwrap();
        }
        playback.tick(start);
        assert_eq!(shown(&playback.tick(start + 100 * MS)), Some(0.08));
        // A frame past the one shown and still nothing from the renderer.
        let tick = playback.tick(start + 130 * MS);
        assert_eq!(tick.status, Some(Status::CatchingUp));
        assert!(playback.tick(start + 500 * MS).status.is_none());
        for time in [0.12, 0.16, 0.2] {
            feed.frames.send(frame(time)).unwrap();
        }
        let tick = playback.tick(start + 600 * MS);
        assert_eq!(
            (tick.status, shown(&tick)),
            (Some(Status::Playing), Some(0.12))
        );
    }

    #[test]
    fn frames_before_an_error_still_play_and_then_it_finishes() {
        let (mut playback, feed) =
            Playback::new(&video(10., (4, 4)), 0., (4, 4), Config::default());
        let start = Instant::now();
        feed.frames.send(frame(0.)).unwrap();
        feed.frames.send(frame(0.04)).unwrap();
        feed.frames.send(Err("driver failed".into())).unwrap();
        let tick = playback.tick(start);
        assert_eq!(tick.error.as_deref(), Some("driver failed"));
        assert_eq!(
            (tick.status, shown(&tick)),
            (Some(Status::Playing), Some(0.))
        );
        let tick = playback.tick(start + 40 * MS);
        assert_eq!(
            (tick.status, shown(&tick)),
            (Some(Status::Finished), Some(0.04))
        );
    }

    #[test]
    fn it_starts_over_from_the_last_frame_and_buffers_less_of_larger_frames() {
        let clip = video(2., (4, 4));
        assert_eq!(
            Playback::new(&clip, 1., (4, 4), Config::default()).1.start,
            1.
        );
        assert_eq!(
            Playback::new(&clip, 2. - 1. / 25., (4, 4), Config::default())
                .1
                .start,
            0.
        );
        // A 4K source and picture fill the buffer on their own, so one frame starts the clock.
        let (mut playback, feed) = Playback::new(
            &video(2., (3840, 2160)),
            0.,
            (3840, 2160),
            Config::default(),
        );
        feed.frames.send(frame(0.)).unwrap();
        assert_eq!(playback.tick(Instant::now()).status, Some(Status::Playing));
    }

    #[test]
    fn stopping_it_stops_the_worker_and_discards_what_it_rendered() {
        let (playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4), Config::default());
        drop(playback);
        assert!(feed.cancel.load(Ordering::Relaxed));
        assert!(feed.frames.send(frame(0.)).is_err());
    }

    #[test]
    fn it_takes_a_new_look_but_not_a_new_size_or_colour_mode() {
        let config = Config {
            output: "320x240".into(),
            ..Config::default()
        };
        let (playback, feed) = Playback::new(&video(10., (4, 4)), 0., (320, 240), config.clone());
        let brighter = Config {
            bloom: 1.5,
            ..config.clone()
        };
        assert!(playback.retune(brighter.clone()));
        assert_eq!(
            *feed.settings.get(),
            brighter,
            "the worker's next frame takes it"
        );
        for refused in [
            Config {
                output: "640x480".into(),
                ..brighter.clone()
            },
            Config {
                signal: "128x112".into(),
                ..brighter.clone()
            },
            Config {
                color_mode: ColorMode::LinearLight,
                ..brighter.clone()
            },
            Config {
                output: "nonsense".into(),
                ..brighter.clone()
            },
        ] {
            assert!(!playback.retune(refused));
            assert_eq!(*feed.settings.get(), brighter);
        }
    }

    #[test]
    fn an_edit_while_playing_keeps_it_playing_and_a_new_size_stops_it_to_start_again() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            Some(Smoke::new("unused-smoke.png".into())),
        );
        let input = app.source.input().dimensions();
        let config = app
            .config
            .with_max_output_side(input, app.preview.quality)
            .unwrap();
        let output = config.output_size(input).unwrap();
        let (playback, feed) = Playback::new(&video(10., input), 0., output, config);
        app.playback = Some(playback);
        app.config.bloom = 0.;
        app.changed();
        assert!(app.playback.is_some() && !feed.cancel.load(Ordering::Relaxed));
        assert_eq!(feed.settings.get().bloom, 0.);
        // A smaller preview needs a new sequence. Without a video open there is nothing to
        // start again, which leaves it stopped.
        app.preview.quality = Some(800);
        app.changed();
        assert!(app.playback.is_none() && feed.cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn a_due_frame_becomes_the_current_preview() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            Ok(app_data::Store::temporary()),
            None,
            Some(Smoke::new("unused-smoke.png".into())),
        );
        let (playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4), Config::default());
        app.playback = Some(playback);
        // As starting playback does: it renders its own frames.
        app.preview.drop_pending();
        for time in [0., 0.04, 0.08] {
            feed.frames.send(frame(time)).unwrap();
        }
        app.tick_playback(&ctx);
        assert!(app.preview.picture().is_some() && app.preview.is_settled());
        assert_eq!(app.status, "Playing");
        app.stop_playback();
        assert!(feed.cancel.load(Ordering::Relaxed));
    }

    /// A real video, decoded by FFmpeg and rendered by the worker, plays to its last frame.
    #[test]
    #[ignore = "requires a Vulkan adapter and FFmpeg"]
    fn a_video_plays_through_the_worker_to_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .args(["testsrc2=size=64x48:rate=10", "-t", "0.5", "-c:v", "ffv1"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::VULKAN),
            Ok(app_data::Store::temporary()),
            Some(path),
            Some(Smoke::new("unused-smoke.png".into())),
        );
        let started = Instant::now();
        let waited = |app: &mut App| {
            app.receive(&ctx);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert!(started.elapsed() < Duration::from_secs(120), "stalled");
            std::thread::sleep(5 * MS);
        };
        while app.source.timeline.is_none() {
            waited(&mut app);
        }
        app.start_playback();
        let mut times = vec![];
        while app.playback.is_some() {
            app.tick_playback(&ctx);
            let time = app.source.timeline.as_ref().unwrap().time;
            if times.last() != Some(&time) {
                times.push(time);
            }
            waited(&mut app);
        }
        assert_eq!(app.status, "Playback finished");
        assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{times:?}");
        assert!(times.last().is_some_and(|&last| last >= 0.4), "{times:?}");
    }

    /// Edited while playing, a real video plays on with the new look, and starts again from
    /// the frame on screen for a new preview size, to play to its end.
    #[test]
    #[ignore = "requires a Vulkan adapter and FFmpeg"]
    fn edits_while_playing_reach_the_next_frames_without_stopping() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clip.mkv");
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .args(["testsrc2=size=64x48:rate=10", "-t", "2", "-c:v", "ffv1"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::VULKAN),
            Ok(app_data::Store::temporary()),
            Some(path),
            Some(Smoke::new("unused-smoke.png".into())),
        );
        app.show_welcome = false;
        let started = Instant::now();
        let waited = |app: &mut App| {
            app.receive(&ctx);
            app.tick_playback(&ctx);
            assert!(app.error.is_none(), "{:?}", app.error);
            assert!(started.elapsed() < Duration::from_secs(120), "stalled");
            std::thread::sleep(5 * MS);
        };
        while app.source.timeline.is_none() {
            waited(&mut app);
        }
        app.start_playback();
        let time = |app: &App| app.source.timeline.as_ref().unwrap().time;
        while app.status != "Playing" {
            waited(&mut app);
        }
        app.config.bloom = 0.;
        app.changed();
        assert_eq!(app.status, "Playing", "the look changes without stopping");
        while time(&app) < 0.6 {
            waited(&mut app);
        }
        app.preview.quality = Some(320);
        app.changed();
        let restarted_at = time(&app);
        assert!(app.playback.is_some(), "a new size starts it again");
        while app.playback.is_some() {
            waited(&mut app);
        }
        assert_eq!(app.status, "Playback finished");
        assert!(
            time(&app) >= 1.9 && restarted_at >= 0.6,
            "from the frame on screen to the end"
        );
    }
}
