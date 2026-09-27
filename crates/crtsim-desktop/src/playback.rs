//! Playing a video in the preview. The worker renders frames ahead into a small buffer and each
//! is shown when its time comes. Playback waits for a few frames before its clock starts, and
//! waits again whenever the renderer falls behind.
use crate::*;
use std::collections::VecDeque;
use worker::{Feed, PlaybackFrame};

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
    /// last frame, with frames rendered at `output` size. Returns it with the worker's end.
    pub fn new(video: &crtsim_media::Video, position: f64, output: (u32, u32)) -> (Self, Feed) {
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
        let playback = Self {
            cancel: cancel.clone(),
            receive,
            buffered: VecDeque::new(),
            clock: None,
            ended: false,
            capacity,
            frame_duration,
            shown: start,
        };
        (
            playback,
            Feed {
                start,
                frames,
                cancel,
            },
        )
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
    pub fn stop_playback(&mut self) {
        if self.playback.take().is_some() {
            self.status = "Playback paused".into();
        }
    }

    pub fn start_playback(&mut self) {
        let Some(video) = self.video.clone() else {
            return;
        };
        self.stop_playback();
        let config = match self
            .config
            .with_max_output_side(self.input.dimensions(), self.preview_limit)
        {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(format!("Cannot play: {e:#}"));
                return;
            }
        };
        let output = config.output_size(video.size).unwrap_or(video.size);
        let (playback, feed) = Playback::new(&video, self.play_time, output);
        self.playback = Some(playback);
        self.schedule.drop_pending();
        self.status = "Buffering · warming CRT history…".into();
        self.send(Job::Playback {
            video,
            config,
            options: self.video_options.clone(),
            feed,
        });
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
        self.play_time = frame.time;
        self.original = texture(ctx, "playing-source", &frame.source, 2048);
        let limit = ctx.input(|i| i.max_texture_side) as u32;
        let crt = texture(ctx, "playing-crt", &frame.crt, limit);
        self.show_preview(Some(Displayed::Uploaded(crt)));
        self.schedule.show_current();
        self.input = Arc::new(frame.source);
        if let Some(video) = &self.video {
            let number = (frame.time * video.fps).round() as u64;
            self.video_frame = number.min(self.video_frames.saturating_sub(1));
            self.selected_frame = self.video_frame;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (mut playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4));
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
        let (mut playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4));
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
        let (mut playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4));
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
        assert_eq!(Playback::new(&clip, 1., (4, 4)).1.start, 1.);
        assert_eq!(Playback::new(&clip, 2. - 1. / 25., (4, 4)).1.start, 0.);
        // A 4K source and picture fill the buffer on their own, so one frame starts the clock.
        let (mut playback, feed) = Playback::new(&video(2., (3840, 2160)), 0., (3840, 2160));
        feed.frames.send(frame(0.)).unwrap();
        assert_eq!(playback.tick(Instant::now()).status, Some(Status::Playing));
    }

    #[test]
    fn stopping_it_stops_the_worker_and_discards_what_it_rendered() {
        let (playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4));
        drop(playback);
        assert!(feed.cancel.load(Ordering::Relaxed));
        assert!(feed.frames.send(frame(0.)).is_err());
    }

    #[test]
    fn a_due_frame_becomes_the_current_preview() {
        let ctx = egui::Context::default();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            None,
            Some(Smoke::new("unused-smoke.png".into())),
        );
        let (playback, feed) = Playback::new(&video(10., (4, 4)), 0., (4, 4));
        app.playback = Some(playback);
        for time in [0., 0.04, 0.08] {
            feed.frames.send(frame(time)).unwrap();
        }
        app.tick_playback(&ctx);
        assert!(app.rendered.is_some() && app.schedule.is_current());
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
        while app.video.is_none() {
            waited(&mut app);
        }
        app.start_playback();
        let mut times = vec![];
        while app.playback.is_some() {
            app.tick_playback(&ctx);
            if times.last() != Some(&app.play_time) {
                times.push(app.play_time);
            }
            waited(&mut app);
        }
        assert_eq!(app.status, "Playback finished");
        assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{times:?}");
        assert!(times.last().is_some_and(|&last| last >= 0.4), "{times:?}");
    }
}
