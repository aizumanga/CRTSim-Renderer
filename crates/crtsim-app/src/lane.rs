//! The work lane: the worker's thread for loading, importing presets, exporting and playing,
//! which runs one job at a time. This holds what it is doing, the flag that stops that, and an
//! export's latest progress. Every job starts here and is finished here, so nothing else keeps
//! its own account of whether the lane is free.
//!
//! Playing holds the lane too, but gives way: any other job stops it first. It ends when the
//! video stops playing, which the worker does not report, so the app says so (`stopped_playing`).
use crate::worker::{Job, Stopped, WorkLane};
use crtsim_media::Progress;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// What a job on the lane is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Opening an image or a frame of a video.
    Loading,
    /// Reading a preset from a rendered image or video.
    ImportingPreset,
    /// An export, or a batch job.
    Exporting,
    Playing,
}

impl Task {
    fn of(job: &Job) -> Option<Self> {
        Some(match job {
            Job::Load(_) | Job::LoadBytes { .. } | Job::LoadVideo { .. } => Self::Loading,
            Job::ImportPreset { .. } => Self::ImportingPreset,
            Job::Export { .. } => Self::Exporting,
            Job::Playback { .. } => Self::Playing,
            Job::Shutdown => return None,
        })
    }
}

/// Why a job did not start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Another job is running; one runs at a time.
    Busy,
    /// The worker has stopped.
    Stopped,
}

impl From<Stopped> for Refused {
    fn from(_: Stopped) -> Self {
        Self::Stopped
    }
}

/// A job that has finished: what it was for, and whether the person stopped it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finished {
    pub task: Option<Task>,
    pub cancelled: bool,
}

struct Running {
    task: Task,
    cancel: Option<Arc<AtomicBool>>,
    progress: Option<Progress>,
}

pub struct Lane {
    send: WorkLane,
    running: Option<Running>,
}

impl Lane {
    pub fn new(send: WorkLane) -> Self {
        Self {
            send,
            running: None,
        }
    }

    /// Sends `job` to the worker. Refused while another job runs, unless that is a video
    /// playing, which the app stops first; and if the worker has stopped, which leaves the lane
    /// free.
    pub fn start(&mut self, job: Job) -> Result<(), Refused> {
        if !self.may_start() {
            return Err(Refused::Busy);
        }
        self.running = Task::of(&job).map(|task| Running {
            task,
            cancel: job.cancel().cloned(),
            progress: None,
        });
        let sent = self.send.send(job);
        if sent.is_err() {
            self.running = None;
        }
        Ok(sent?)
    }

    /// The job running has finished, or failed, and the lane is free.
    pub fn finished(&mut self) -> Finished {
        let running = self.running.take();
        Finished {
            task: running.as_ref().map(|r| r.task),
            cancelled: running
                .and_then(|r| r.cancel)
                .is_some_and(|cancel| cancel.load(Ordering::Relaxed)),
        }
    }

    /// The video playing has stopped, so the lane is free, if it was playing.
    pub fn stopped_playing(&mut self) {
        if self.is_playing() {
            self.running = None;
        }
    }

    /// The worker has stopped: nothing is running, or ever will be.
    pub fn stop(&mut self) {
        self.running = None;
    }

    /// The latest progress the export running has reported.
    pub fn progress(&mut self, progress: Progress) {
        if let Some(running) = self.running.as_mut().filter(|r| r.task == Task::Exporting) {
            running.progress = Some(progress);
        }
    }

    /// What an export says before the worker reports its own progress.
    pub fn queued(&mut self, stage: &str) {
        self.progress(Progress {
            fraction: 0.,
            stage: stage.into(),
        });
    }

    /// The export's latest progress, to show.
    pub fn shown_progress(&self) -> Option<&Progress> {
        self.running.as_ref()?.progress.as_ref()
    }

    /// What the job running is for.
    pub fn task(&self) -> Option<Task> {
        self.running.as_ref().map(|r| r.task)
    }

    /// The flag that stops the job running, for a person to cancel it with: not a video
    /// playing, which pausing stops.
    pub fn cancel(&self) -> Option<&Arc<AtomicBool>> {
        let running = self.running.as_ref().filter(|r| r.task != Task::Playing)?;
        running.cancel.as_ref()
    }

    /// Nothing runs, not even a video playing.
    pub fn is_idle(&self) -> bool {
        self.running.is_none()
    }

    /// Another job can start: nothing runs but perhaps a video playing, which gives way.
    pub fn may_start(&self) -> bool {
        matches!(self.task(), None | Some(Task::Playing))
    }

    /// A job other than playing runs, which the interface shows as busy.
    pub fn is_working(&self) -> bool {
        !self.may_start()
    }

    /// Opening a source or reading a preset, which leaves the settings half applied.
    pub fn is_loading(&self) -> bool {
        matches!(self.task(), Some(Task::Loading | Task::ImportingPreset))
    }

    pub fn is_exporting(&self) -> bool {
        self.task() == Some(Task::Exporting)
    }

    pub fn is_playing(&self) -> bool {
        self.task() == Some(Task::Playing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{self, Jobs};
    use std::{path::PathBuf, sync::mpsc};

    fn lane() -> (Lane, mpsc::Receiver<Job>) {
        let (jobs, work, _previews) = Jobs::capture();
        (Lane::new(jobs.work_lane()), work)
    }

    fn export() -> (Job, Arc<AtomicBool>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job::Export {
            export: worker::Export::Image {
                input: Arc::new(image::RgbaImage::new(1, 1)),
                config: Default::default(),
            },
            path: PathBuf::from("out.png"),
            cancel: cancel.clone(),
        };
        (job, cancel)
    }

    #[test]
    fn a_job_holds_the_lane_until_it_finishes_saying_whether_it_was_cancelled() {
        let (mut lane, work) = lane();
        assert!(lane.is_idle() && lane.may_start());
        lane.start(Job::Load("photo.png".into())).unwrap();
        assert!(
            matches!(work.try_recv(), Ok(Job::Load(_))),
            "sent to the worker"
        );
        assert!(lane.is_loading() && lane.is_working() && !lane.may_start());
        assert!(lane.cancel().is_none(), "an image loads to the end");
        let finished = lane.finished();
        assert_eq!(
            (finished.task, finished.cancelled),
            (Some(Task::Loading), false)
        );
        assert!(lane.is_idle());

        let (job, flag) = export();
        lane.start(job).unwrap();
        lane.queued("Queued for export");
        assert_eq!(lane.shown_progress().unwrap().stage, "Queued for export");
        lane.cancel().unwrap().store(true, Ordering::Relaxed);
        assert!(flag.load(Ordering::Relaxed), "the job's own flag");
        assert!(lane.finished().cancelled);
        assert!(lane.shown_progress().is_none());
    }

    #[test]
    fn a_job_is_refused_while_another_runs_and_leaves_it_be() {
        let (mut lane, work) = lane();
        let (job, flag) = export();
        lane.start(job).unwrap();
        let (playback, feed) = crate::playback::Playback::new(
            &crtsim_media::test_clip(),
            0.,
            (4, 4),
            Default::default(),
        );
        let playing = Job::Playback {
            video: crtsim_media::test_clip(),
            options: Default::default(),
            feed,
        };
        assert_eq!(lane.start(playing), Err(Refused::Busy));
        assert_eq!(
            lane.start(Job::Load("photo.png".into())),
            Err(Refused::Busy)
        );
        assert!(lane.is_exporting(), "the export still runs");
        assert!(Arc::ptr_eq(lane.cancel().unwrap(), &flag));
        assert_eq!(work.try_iter().count(), 1, "only the export was sent");
        drop(playback);
    }

    #[test]
    fn playing_holds_the_lane_but_gives_way_and_cannot_be_cancelled_as_a_job() {
        let (mut lane, _work) = lane();
        let (playback, feed) = crate::playback::Playback::new(
            &crtsim_media::test_clip(),
            0.,
            (4, 4),
            Default::default(),
        );
        lane.start(Job::Playback {
            video: crtsim_media::test_clip(),
            options: Default::default(),
            feed,
        })
        .unwrap();
        assert!(lane.is_playing() && !lane.is_idle());
        assert!(
            lane.may_start() && !lane.is_working(),
            "another job may stop it"
        );
        assert!(lane.cancel().is_none(), "pausing stops it, not Cancel");
        // Progress is an export's alone.
        lane.queued("Queued for export");
        assert!(lane.shown_progress().is_none());
        drop(playback);
        lane.stopped_playing();
        assert!(lane.is_idle());
    }

    #[test]
    fn a_stopped_worker_leaves_the_lane_free() {
        let (mut lane, work) = lane();
        drop(work);
        let (job, _) = export();
        assert_eq!(lane.start(job), Err(Refused::Stopped));
        assert!(lane.is_idle());
        let (job, _) = export();
        let (mut running, _work) = self::lane();
        running.start(job).unwrap();
        running.stop();
        assert!(running.is_idle());
    }
}
