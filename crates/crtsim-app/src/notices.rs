//! The status bar's notices: what failed, and what an export saved, each about one topic, with
//! its own Dismiss and, where one helps, a button that acts on it. News of a topic replaces the
//! older news of that topic and leaves the rest, so a preview that fails no longer hides an
//! export that failed, and opening a file that works no longer clears it.
use std::path::PathBuf;

/// What a notice is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    /// Opening a file, a frame, a project or a link's look, and the system's folders.
    Opening,
    /// Exports and the other files the app writes: presets, projects, contact sheets.
    Saving,
    /// The video playing.
    Playback,
    /// The preview on screen, whose error the preview keeps itself.
    Preview,
    /// The galleries and the looks in them: presets, LUTs and links to looks.
    Presets,
    /// What the app remembers between runs: the session, theme, window layout and choices.
    AppData,
    /// The renderer's thread.
    Renderer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    /// Something finished, worth keeping in view: a file an export saved.
    Done,
}

/// What a notice's button does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Shows the file saved in the system's file manager.
    ShowInFolder(PathBuf),
    /// Opens FFmpeg setup, for a failure FFmpeg had a part in.
    FfmpegSetup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub topic: Topic,
    pub severity: Severity,
    pub text: String,
    /// The full text, shown when the notice is pointed at, where `text` is shorter.
    pub detail: Option<String>,
    pub action: Option<Action>,
}

/// The notices, oldest first, at most one for each topic.
#[derive(Debug, Default)]
pub struct Notices(Vec<Notice>);

impl Notices {
    /// `text` failed. An FFmpeg failure on the desktop offers FFmpeg setup; a browser has no
    /// FFmpeg to set up.
    pub fn failed(&mut self, topic: Topic, text: impl Into<String>) {
        let text = text.into();
        let ffmpeg = !cfg!(target_arch = "wasm32") && text.to_lowercase().contains("ffmpeg");
        self.put(Notice {
            topic,
            severity: Severity::Error,
            action: ffmpeg.then_some(Action::FfmpegSetup),
            detail: None,
            text,
        });
    }

    /// An export saved `path`, which stays in view until the next export, with a way to it.
    pub fn saved(&mut self, path: PathBuf) {
        self.put(Notice {
            topic: Topic::Saving,
            severity: Severity::Done,
            text: format!("Saved {}", crate::file_name(&path)),
            detail: Some(path.display().to_string()),
            action: Some(Action::ShowInFolder(path)),
        });
    }

    /// The `topic` worked this time: its error goes, and a finished notice of it stays.
    pub fn fixed(&mut self, topic: Topic) {
        self.0
            .retain(|n| n.topic != topic || n.severity != Severity::Error);
    }

    /// Every notice of `topic` goes, as when a new export starts.
    pub fn forget(&mut self, topic: Topic) {
        self.0.retain(|n| n.topic != topic);
    }

    /// The error of `topic`, if one shows.
    pub fn error(&self, topic: Topic) -> Option<&str> {
        self.0
            .iter()
            .find(|n| n.topic == topic && n.severity == Severity::Error)
            .map(|n| n.text.as_str())
    }

    pub fn any_error(&self) -> bool {
        self.0.iter().any(|n| n.severity == Severity::Error)
    }

    /// The file the last export saved, while its notice shows.
    #[cfg(test)]
    pub fn saved_file(&self) -> Option<&std::path::Path> {
        self.0.iter().find_map(|n| match &n.action {
            Some(Action::ShowInFolder(path)) => Some(path.as_path()),
            _ => None,
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &Notice> {
        self.0.iter()
    }

    pub fn dismiss(&mut self, topic: Topic) {
        self.forget(topic);
    }

    /// Follows the preview's own error, which goes by itself once a preview renders.
    pub fn follow_preview(&mut self, error: Option<&String>) {
        match error {
            Some(error) if self.error(Topic::Preview) != Some(error) => {
                self.failed(Topic::Preview, error.clone())
            }
            Some(_) => {}
            None => self.forget(Topic::Preview),
        }
    }

    /// `notice` in place of any of its topic, last, as the newest.
    fn put(&mut self, notice: Notice) {
        self.forget(notice.topic);
        self.0.push(notice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_of_a_topic_replaces_its_own_and_leaves_the_rest() {
        let mut notices = Notices::default();
        notices.failed(Topic::Saving, "Cannot export: disk full");
        notices.failed(Topic::Preview, "Cannot preview");
        // A file that opens says nothing about the export that failed.
        notices.fixed(Topic::Opening);
        assert_eq!(
            notices.error(Topic::Saving),
            Some("Cannot export: disk full")
        );
        notices.saved("out/crt.png".into());
        assert_eq!(notices.error(Topic::Saving), None);
        assert_eq!(
            notices.saved_file(),
            Some(std::path::Path::new("out/crt.png"))
        );
        // A preset saved fine keeps the export's notice; dismissing the preview's keeps it too.
        notices.fixed(Topic::Saving);
        notices.dismiss(Topic::Preview);
        let left: Vec<_> = notices.iter().map(|n| n.text.as_str()).collect();
        assert_eq!(left, ["Saved crt.png"]);
        notices.forget(Topic::Saving);
        assert!(notices.iter().next().is_none());
    }

    #[test]
    fn an_ffmpeg_failure_offers_its_setup_on_the_desktop() {
        let mut notices = Notices::default();
        notices.failed(Topic::Opening, "FFmpeg could not decode this file");
        notices.failed(Topic::Presets, "Cannot load included LUT");
        let actions: Vec<_> = notices.iter().map(|n| n.action.clone()).collect();
        let setup = (!cfg!(target_arch = "wasm32")).then_some(Action::FfmpegSetup);
        assert_eq!(actions, [setup, None]);
    }

    #[test]
    fn the_preview_notice_follows_the_preview() {
        let mut notices = Notices::default();
        let error = "Cannot preview: too large".to_owned();
        notices.follow_preview(Some(&error));
        notices.follow_preview(Some(&error));
        assert_eq!(notices.iter().count(), 1);
        notices.follow_preview(None);
        assert!(!notices.any_error());
    }
}
