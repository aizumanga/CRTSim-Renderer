//! Where the editor is in an open video: the frame on screen, the frame the controls have
//! picked to go to, and the media time playing starts from. None of it means anything without
//! a video, so it is all one value, there only while a video is open.
use crtsim_media::Video;

#[derive(Clone)]
pub struct Timeline {
    pub video: Video,
    /// How many frames the video decodes to.
    pub frames: u64,
    /// The frame on screen, from 0.
    pub shown: u64,
    /// The frame the controls have picked, loaded once the pick settles.
    pub picked: u64,
    /// Where the frame on screen is, in seconds; playing starts from here.
    pub time: f64,
}

impl Timeline {
    /// `video`, which decodes to `frames` frames, showing `frame`.
    pub fn new(video: Video, frame: u64, frames: u64) -> Self {
        Self {
            time: frame as f64 / video.fps,
            video,
            frames,
            shown: frame,
            picked: frame,
        }
    }

    /// Whether the video is the video test card, which is drawn rather than read from a file.
    pub fn is_video_test_card(&self) -> bool {
        matches!(self.video.source, crtsim_media::Source::TestClip)
    }

    pub fn last(&self) -> u64 {
        self.frames.saturating_sub(1)
    }

    /// Picks the frame before the one on screen.
    pub fn pick_previous(&mut self) {
        self.picked = self.shown.saturating_sub(1);
    }

    /// Picks the frame after the one on screen.
    pub fn pick_next(&mut self) {
        self.picked = (self.shown + 1).min(self.last());
    }

    /// Whether the frame picked is not the one on screen, and so has to be loaded.
    pub fn seeking(&self) -> bool {
        self.picked != self.shown
    }

    /// Playback has reached `time`: the frame there is the one on screen, and picked.
    pub fn played(&mut self, time: f64) {
        self.time = time;
        self.shown = ((time * self.video.fps).round() as u64).min(self.last());
        self.picked = self.shown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip() -> Video {
        Video {
            metadata: Default::default(),
            tracks: vec![],
            start: 0.,
            path: "clip.mkv".into(),
            size: (4, 4),
            fps: 25.,
            rate: "25/1".into(),
            duration: 2.,
            audio: false,
            audio_offset: 0.,
            hdr: false,
            stream: 0,
            frames: None,
            source: crtsim_media::Source::Ffmpeg,
            contents: None,
            subtitle: None,
        }
    }

    #[test]
    fn picks_stay_inside_the_video_and_playing_moves_the_frame_on_screen() {
        let mut timeline = Timeline::new(clip(), 49, 50);
        assert_eq!(timeline.time, 49. / 25.);
        timeline.pick_next();
        assert!(!timeline.seeking(), "already the last frame");
        timeline.pick_previous();
        assert_eq!((timeline.picked, timeline.seeking()), (48, true));
        timeline.played(0.5);
        assert_eq!(
            (timeline.shown, timeline.picked, timeline.time),
            (13, 13, 0.5)
        );
        timeline.played(10.);
        assert_eq!(timeline.shown, 49, "never past the last frame");
        let mut first = Timeline::new(clip(), 0, 50);
        first.pick_previous();
        assert!(!first.seeking());
    }
}
