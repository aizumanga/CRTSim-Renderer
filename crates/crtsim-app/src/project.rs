//! The project file: the source, its settings and the batch queue, saved to pick up later. The
//! session the app recovers after a crash is saved as one too.
use crate::{batch, files};
use anyhow::{ensure, Result};
use crtsim_core::config::Config;
use crtsim_media::Options;
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub const EXTENSION: &str = "crtsim";
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub(crate) version: u32,
    pub(crate) source: Option<PathBuf>,
    /// What stands in for a source file when there is none.
    #[serde(default, skip_serializing_if = "BuiltIn::is_test_card")]
    pub(crate) built_in: BuiltIn,
    pub(crate) frame: u64,
    pub(crate) config: Config,
    pub(crate) options: Options,
    pub(crate) queue: Vec<batch::Item>,
}

/// A source the app makes itself, which a project without a source file opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuiltIn {
    /// The test card, as every project without a source opened before there was a choice.
    /// Left out of the file, so such projects are saved as they always were.
    #[default]
    TestCard,
    VideoTestCard,
}

impl BuiltIn {
    fn is_test_card(&self) -> bool {
        *self == Self::TestCard
    }
}

/// Reads and checks the project at `path`. Paths saved relative to it are resolved against
/// the folder it is in.
pub fn read(path: &Path) -> Result<Project> {
    ensure!(
        std::fs::metadata(path)?.len() <= MAX_BYTES,
        "Project exceeds 64 MB"
    );
    parse(
        &std::fs::read(path)?,
        path.parent().unwrap_or_else(|| Path::new(".")),
    )
}

/// A project from its file's contents. Relative paths in it are resolved against `root`, the
/// folder the file is in.
pub fn parse(bytes: &[u8], root: &Path) -> Result<Project> {
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Project exceeds 64 MB");
    let mut p: Project = serde_json::from_slice(bytes)?;
    ensure!(p.version == 1, "Unsupported project version");
    p.config.validate()?;
    p.options.validate()?;
    ensure!(
        p.queue.len() <= batch::MAX_JOBS,
        "Queue exceeds {} jobs",
        batch::MAX_JOBS
    );
    if let Some(source) = p.source.as_mut().filter(|source| source.is_relative()) {
        *source = root.join(&*source);
    }
    for item in &mut p.queue {
        item.restore(root)?;
    }
    Ok(p)
}

/// Saves `project` to `path`, replacing it only once the whole file is written.
pub fn save(path: &Path, project: &Project) -> Result<()> {
    let bytes = to_bytes(project)?;
    files::save_atomic(path, |f| Ok(f.write_all(&bytes)?))
}

/// `project` as its file's contents.
pub fn to_bytes(project: &Project) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(project)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Project with embedded LUTs exceeds 64 MB"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_roundtrip_recovers_running_jobs_and_relative_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.crtsim");
        let c = Config::default();
        let p = Project {
            version: 1,
            source: Some("input.png".into()),
            built_in: BuiltIn::TestCard,
            frame: 7,
            config: c.clone(),
            options: Options::default(),
            queue: vec![batch::Item {
                source: "input.png".into(),
                output: "output.png".into(),
                config: c,
                options: Options::default(),
                status: batch::Status::Running,
            }],
        };
        save(&path, &p).unwrap();
        // Projects and sessions saved before keep opening: a job is saved as it always was.
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.contains(r#""output": "output.png""#) && saved.contains(r#""status": "Running""#)
        );
        let restored = read(&path).unwrap();
        assert_eq!(restored.frame, 7);
        assert_eq!(restored.source, Some(dir.path().join("input.png")));
        assert_eq!(restored.queue[0].status, batch::Status::Pending);
        assert_eq!(restored.queue[0].output, dir.path().join("output.png"));
        let mut bad = p;
        bad.version = 99;
        save(&path, &bad).unwrap();
        assert!(read(&path).is_err());
    }

    #[test]
    fn a_project_of_the_video_test_card_says_so_and_others_are_saved_as_before() {
        let project = |built_in| Project {
            version: 1,
            source: None,
            built_in,
            frame: 150,
            config: Config::general(),
            options: Options::default(),
            queue: vec![],
        };
        let card = to_bytes(&project(BuiltIn::TestCard)).unwrap();
        assert!(!String::from_utf8(card.clone())
            .unwrap()
            .contains("built_in"));
        let clip = to_bytes(&project(BuiltIn::VideoTestCard)).unwrap();
        assert!(String::from_utf8(clip.clone())
            .unwrap()
            .contains(r#""built_in": "video-test-card""#));
        for (bytes, built_in) in [(card, BuiltIn::TestCard), (clip, BuiltIn::VideoTestCard)] {
            assert_eq!(parse(&bytes, Path::new(".")).unwrap(), project(built_in));
        }
    }
}
