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
    pub(crate) frame: u64,
    pub(crate) config: Config,
    pub(crate) options: Options,
    pub(crate) queue: Vec<batch::Item>,
}

/// Reads and checks the project at `path`. Paths saved relative to it are resolved against
/// the folder it is in.
pub fn read(path: &Path) -> Result<Project> {
    ensure!(
        std::fs::metadata(path)?.len() <= MAX_BYTES,
        "Project exceeds 64 MB"
    );
    let mut p: Project = serde_json::from_slice(&std::fs::read(path)?)?;
    ensure!(p.version == 1, "Unsupported project version");
    p.config.validate()?;
    p.options.validate()?;
    ensure!(
        p.queue.len() <= batch::MAX_JOBS,
        "Queue exceeds {} jobs",
        batch::MAX_JOBS
    );
    let root = path.parent().unwrap_or_else(|| Path::new("."));
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
    let bytes = serde_json::to_vec_pretty(project)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "Project with embedded LUTs exceeds 64 MB"
    );
    files::save_atomic(path, |f| Ok(f.write_all(&bytes)?))
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
}
