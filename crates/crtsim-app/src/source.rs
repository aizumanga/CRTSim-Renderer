//! The source: what is open to edit, and the picture it gives. It is the built-in test card,
//! the video test card, a file on disk, a file a browser handed over, or a project's source
//! that is missing, with the test card in its place. A video's comes with its timeline.
//!
//! Opening one asks the worker to load it (`Opening`), and what comes back becomes the source
//! (`loaded`). A project opens its source first and is held here until that has loaded; then
//! its settings and queue apply, whether the source opened or not. The source is saved into a
//! project or session as the file to open again, the built-in source, and the frame.
use crate::{
    file_name,
    project::{BuiltIn, Project},
    texture,
    timeline::Timeline,
    worker::{Failure, Job, Loaded},
};
use crtsim_core::config;
use eframe::egui::{self, TextureHandle};
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

/// What kind of source is open.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    TestCard,
    VideoTestCard,
    /// An image, video or animation on disk. A video or animation has a timeline.
    File(PathBuf),
    /// A file a browser handed over, which cannot be opened again by name.
    Picked,
    /// A project's source that is not there or could not be opened. The test card shows in its
    /// place, and saving keeps the file, so the project opens it again once it is back.
    Missing {
        path: PathBuf,
        frame: u64,
    },
}

/// A load for the work lane to run: the job, and the flag that cancels it, when it can be.
pub struct Opening {
    pub job: Job,
    pub cancel: Option<Arc<AtomicBool>>,
}

/// What restoring a project needs next.
pub enum Restoring {
    /// Its source loading. The project is held until it has, then applies.
    Open(Opening),
    /// It applies now, its source already in place, with why its source is not open, if not.
    Apply {
        project: Project,
        error: Option<String>,
    },
}

/// What a load that came back did.
pub struct Arrived {
    /// Whether the picture changed.
    pub changed: bool,
    /// What the status bar says if it opened, or why it did not.
    pub opened: Result<&'static str, Failure>,
    /// The project that was waiting for it, to apply now.
    pub project: Option<Project>,
}

/// The load under way: whether it is of bytes a browser handed over, and the project waiting
/// for it.
struct Pending {
    picked: bool,
    project: Option<Project>,
}

pub struct Source {
    ctx: egui::Context,
    kind: Kind,
    name: String,
    input: Arc<RgbaImage>,
    original: TextureHandle,
    /// Where the editor is in the video open, if one is.
    pub timeline: Option<Timeline>,
    pending: Option<Pending>,
}

impl Source {
    /// The test card, its picture drawn on `ctx`.
    pub fn new(ctx: &egui::Context) -> Self {
        let card = config::test_card();
        Self {
            ctx: ctx.clone(),
            kind: Kind::TestCard,
            name: TEST_CARD.into(),
            original: texture(ctx, "original", &card, 2048),
            input: Arc::new(card),
            timeline: None,
            pending: None,
        }
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The image being edited, at full size.
    pub fn input(&self) -> &Arc<RgbaImage> {
        &self.input
    }

    /// The image as the original view shows it.
    pub fn original(&self) -> &TextureHandle {
        &self.original
    }

    /// Shows the built-in test card.
    pub fn test_card(&mut self) {
        let card = config::test_card();
        self.show(Kind::TestCard, TEST_CARD.into(), None, card.clone(), &card);
    }

    /// Opens the file at `path`: a video or animation at its first frame, else an image.
    pub fn open(&mut self, path: PathBuf) -> Opening {
        self.open_at(path, 0)
    }

    /// Opens the file at `path`: a video or animation at `frame`, else an image.
    fn open_at(&mut self, path: PathBuf, frame: u64) -> Opening {
        self.pending = Some(Pending {
            picked: false,
            project: None,
        });
        if crtsim_media::MediaKind::of(&path).is_moving() {
            return video(path, frame, None);
        }
        Opening {
            job: Job::Load(path),
            cancel: None,
        }
    }

    /// Opens a file a browser handed over, by its `name` and contents.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop opens files by path")
    )]
    pub fn open_bytes(&mut self, name: String, bytes: Vec<u8>) -> Opening {
        self.pending = Some(Pending {
            picked: true,
            project: None,
        });
        Opening {
            job: Job::LoadBytes { name, bytes },
            cancel: None,
        }
    }

    /// Opens frame `frame` of the video open, if there is one.
    pub fn open_frame(&mut self, frame: u64) -> Option<Opening> {
        let timeline = self.timeline.as_ref()?;
        let cached = (timeline.video.clone(), timeline.frames);
        self.pending = Some(Pending {
            picked: self.kind == Kind::Picked,
            project: None,
        });
        Some(video(timeline.video.path.clone(), frame, Some(cached)))
    }

    /// Opens the video test card at `frame`. It is drawn rather than read, so it opens as an
    /// animation does, without a file or FFmpeg.
    pub fn open_video_test_card(&mut self, frame: u64) -> Opening {
        let clip = crtsim_media::test_clip();
        let frames = clip.frames.unwrap_or(1);
        self.pending = Some(Pending {
            picked: false,
            project: None,
        });
        video(clip.path.clone(), frame, Some((clip, frames)))
    }

    /// Opens `project`'s source, holding the project until it has loaded, or, when its source
    /// is built in or missing, puts that in place for the project to apply now.
    pub fn restore(&mut self, project: Project) -> Restoring {
        let opening = match (&project.source, project.built_in) {
            (Some(path), _) if !path.exists() => {
                let error = format!(
                    "Project source is missing: {}. Edits and queue were recovered. Use Open \
                     File to relink the source.",
                    path.display()
                );
                self.missing(path.clone(), project.frame, "Missing");
                return Restoring::Apply {
                    project,
                    error: Some(error),
                };
            }
            (Some(path), _) => self.open_at(path.clone(), project.frame),
            (None, BuiltIn::VideoTestCard) => {
                let frame = project.frame.min(crtsim_core::test_clip::FRAMES - 1);
                self.open_video_test_card(frame)
            }
            (None, BuiltIn::TestCard) => {
                self.test_card();
                return Restoring::Apply {
                    project,
                    error: None,
                };
            }
        };
        if let Some(pending) = &mut self.pending {
            pending.project = Some(project);
        }
        Restoring::Open(opening)
    }

    /// Whether a project is waiting for its source to load, its settings not yet applied.
    pub fn restoring(&self) -> bool {
        self.pending.as_ref().is_some_and(|p| p.project.is_some())
    }

    /// A load came back. What loaded becomes the source; a project's source that could not
    /// be opened is missing, and the project applies either way.
    pub fn loaded(&mut self, result: Result<Loaded, Failure>) -> Arrived {
        let pending = self.pending.take().unwrap_or(Pending {
            picked: false,
            project: None,
        });
        let project = pending.project;
        let loaded = match result {
            Ok(loaded) => loaded,
            Err(failure) => {
                // The project's source stays its source, to open once it can be.
                let missing = project
                    .as_ref()
                    .and_then(|p| Some((p.source.clone()?, p.frame)));
                let changed = missing.is_some();
                if let Some((path, frame)) = missing {
                    self.missing(path, frame, "Not opened");
                }
                return Arrived {
                    changed,
                    opened: Err(failure),
                    project,
                };
            }
        };
        let status = match &loaded.timeline {
            Some(_) => "Video frame loaded",
            None => "Image loaded",
        };
        let kind = match &loaded.timeline {
            Some(timeline) if timeline.is_video_test_card() => Kind::VideoTestCard,
            _ if pending.picked => Kind::Picked,
            _ => Kind::File(loaded.path.clone()),
        };
        self.show(
            kind,
            loaded.name,
            loaded.timeline,
            loaded.image,
            &loaded.thumbnail,
        );
        Arrived {
            changed: true,
            opened: Ok(status),
            project,
        }
    }

    /// A frame of the video playing has come due: its time, and its picture, which becomes
    /// the image edited and the original shown.
    pub fn played(&mut self, time: f64, frame: RgbaImage) {
        if let Some(timeline) = &mut self.timeline {
            timeline.played(time);
        }
        self.original = texture(&self.ctx, "playing-source", &frame, 2048);
        self.input = Arc::new(frame);
    }

    /// The source as a project or session saves it: the file to open again, if there is one
    /// it can open by name, what stands in when there is not, and the frame.
    pub fn saved(&self) -> (Option<PathBuf>, BuiltIn, u64) {
        let frame = self.timeline.as_ref().map_or(0, |t| t.shown);
        // A browser cannot open a file again by name, so its session keeps the settings and
        // leaves the picture to be picked again. A built-in source opens again anywhere.
        let file = |path: &PathBuf| (!cfg!(target_arch = "wasm32")).then(|| path.clone());
        match &self.kind {
            Kind::File(path) => (file(path), BuiltIn::TestCard, frame),
            Kind::Missing { path, frame } => (file(path), BuiltIn::TestCard, *frame),
            Kind::VideoTestCard => (None, BuiltIn::VideoTestCard, frame),
            Kind::TestCard | Kind::Picked => (None, BuiltIn::TestCard, 0),
        }
    }

    /// `image`, as though a file of it had loaded.
    #[cfg(test)]
    pub fn set_image(&mut self, image: RgbaImage) {
        let path = PathBuf::from("image.png");
        let name = file_name(&path);
        self.show(Kind::File(path), name, None, image.clone(), &image);
    }

    /// The test card in place of `path`, which is said to be `why`.
    fn missing(&mut self, path: PathBuf, frame: u64, why: &str) {
        let card = config::test_card();
        let name = format!("{why}: {} — use Open File to relink", file_name(&path));
        self.show(
            Kind::Missing { path, frame },
            name,
            None,
            card.clone(),
            &card,
        );
    }

    /// Makes `input` the image being edited, the original view showing `thumbnail`.
    fn show(
        &mut self,
        kind: Kind,
        name: String,
        timeline: Option<Timeline>,
        input: RgbaImage,
        thumbnail: &RgbaImage,
    ) {
        self.kind = kind;
        self.name = name;
        self.timeline = timeline;
        self.original = texture(&self.ctx, "original", thumbnail, 2048);
        self.input = Arc::new(input);
    }
}

const TEST_CARD: &str = "Built-in test card";

/// Frame `frame` of the video at `path`, or of `cached`, a video already probed, with its
/// frame count.
fn video(path: PathBuf, frame: u64, cached: Option<(crtsim_media::Video, u64)>) -> Opening {
    let cancel = Arc::new(AtomicBool::new(false));
    Opening {
        job: Job::LoadVideo {
            path,
            frame,
            cached,
            cancel: cancel.clone(),
        },
        cancel: Some(cancel),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crtsim_core::config::Config;

    fn source() -> Source {
        Source::new(&egui::Context::default())
    }

    fn loaded(path: &str, timeline: Option<Timeline>) -> Result<Loaded, Failure> {
        let image = RgbaImage::new(4, 3);
        Ok(Loaded {
            path: path.into(),
            name: path.into(),
            thumbnail: image.clone(),
            image,
            timeline,
        })
    }

    fn project(source: Option<PathBuf>, built_in: BuiltIn, frame: u64) -> Project {
        Project {
            version: 1,
            source,
            built_in,
            frame,
            config: Config::general(),
            options: Default::default(),
            queue: vec![],
        }
    }

    #[test]
    fn opening_asks_for_an_image_or_a_video_frame_which_can_be_cancelled() {
        let mut source = source();
        let image = source.open("photo.png".into());
        assert!(matches!(image.job, Job::Load(_)) && image.cancel.is_none());
        let video = source.open("clip.mp4".into());
        assert!(matches!(video.job, Job::LoadVideo { frame: 0, .. }));
        assert!(video.cancel.is_some());
        assert!(source.open_frame(3).is_none(), "no video is open");
        let card = source.open_video_test_card(5);
        assert!(matches!(
            card.job,
            Job::LoadVideo {
                frame: 5,
                cached: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn what_loads_becomes_the_source_and_is_saved_as_what_it_is() {
        let mut source = source();
        assert_eq!(source.saved(), (None, BuiltIn::TestCard, 0));
        let _ = source.open("photo.png".into());
        let arrived = source.loaded(loaded("photo.png", None));
        assert!(arrived.changed && arrived.project.is_none());
        assert_eq!(arrived.opened.ok(), Some("Image loaded"));
        assert_eq!(*source.kind(), Kind::File("photo.png".into()));
        assert_eq!(
            (source.name(), source.input().dimensions()),
            ("photo.png", (4, 3))
        );
        assert_eq!(
            source.saved(),
            (Some("photo.png".into()), BuiltIn::TestCard, 0)
        );
        // A browser's file cannot be opened again by name.
        let _ = source.open_bytes("picked.png".into(), vec![]);
        let _ = source.loaded(loaded("picked.png", None));
        assert_eq!(*source.kind(), Kind::Picked);
        assert_eq!(source.saved(), (None, BuiltIn::TestCard, 0));
        // The video test card opens again anywhere, at the frame on screen.
        let clip = crtsim_media::test_clip();
        let frames = clip.frames.unwrap();
        let _ = source.open_video_test_card(7);
        let _ = source.loaded(loaded("clip", Some(Timeline::new(clip, 7, frames))));
        assert_eq!(*source.kind(), Kind::VideoTestCard);
        assert_eq!(source.saved(), (None, BuiltIn::VideoTestCard, 7));
        // Played frames move it on.
        source.played(9. / 60., RgbaImage::new(2, 2));
        assert_eq!(source.saved().2, 9);
        assert_eq!(source.input().dimensions(), (2, 2));
    }

    #[test]
    fn a_project_waits_for_its_source_and_applies_even_when_that_does_not_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.png");
        std::fs::write(&path, b"not really a PNG").unwrap();
        for failure in [Failure::Cancelled, Failure::Failed(anyhow::anyhow!("bad"))] {
            let mut source = source();
            let restoring = source.restore(project(Some(path.clone()), BuiltIn::TestCard, 4));
            assert!(matches!(restoring, Restoring::Open(_)));
            assert!(source.restoring(), "the project waits for its source");
            let arrived = source.loaded(Err(failure));
            assert!(arrived.changed && arrived.opened.is_err());
            assert!(arrived.project.is_some(), "its edits apply all the same");
            assert!(!source.restoring());
            assert_eq!(
                *source.kind(),
                Kind::Missing {
                    path: path.clone(),
                    frame: 4
                }
            );
            assert!(source.name().starts_with("Not opened: photo.png"));
            // Saved, it opens the same source again next time.
            assert_eq!(source.saved(), (Some(path.clone()), BuiltIn::TestCard, 4));
        }
        // One whose source opens applies once it has.
        let mut source = source();
        let _ = source.restore(project(Some(path.clone()), BuiltIn::TestCard, 0));
        let arrived = source.loaded(loaded("photo.png", None));
        assert!(arrived.project.is_some() && arrived.opened.is_ok());
    }

    #[test]
    fn a_project_without_its_source_applies_at_once() {
        let mut source = source();
        let gone = PathBuf::from("/nowhere/clip.mp4");
        let Restoring::Apply {
            project: applied,
            error,
        } = source.restore(project(Some(gone.clone()), BuiltIn::TestCard, 2))
        else {
            panic!("a missing source cannot be waited for");
        };
        assert_eq!(applied.frame, 2);
        assert!(error.unwrap().contains("missing"));
        assert_eq!(
            *source.kind(),
            Kind::Missing {
                path: gone,
                frame: 2
            }
        );
        assert!(source.name().starts_with("Missing: clip.mp4"));
        // The test card applies at once, the video test card once it has opened.
        let restoring = source.restore(project(None, BuiltIn::TestCard, 0));
        assert!(matches!(restoring, Restoring::Apply { error: None, .. }));
        assert_eq!(*source.kind(), Kind::TestCard);
        let restoring = source.restore(project(None, BuiltIn::VideoTestCard, 9));
        assert!(matches!(restoring, Restoring::Open(_)) && source.restoring());
    }
}
