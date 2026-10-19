//! The queue: jobs waiting for the one worker that owns the GPU, how far each has got, and the
//! files finished jobs leave behind for a while. A job is known only by its id, 128 random bits,
//! so one person's job cannot be guessed by another.

use crate::http::Refused;
use crate::spec::{Format, Spec};
use crtsim_core::Renderer;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub type Id = String;

/// How long jobs may take and their results are kept, and how many may wait.
#[derive(Clone, Debug)]
pub struct Settings {
    pub waiting: usize,
    /// The longest a still may render.
    pub still_timeout: Duration,
    /// The longest an animation or a video may render.
    pub moving_timeout: Duration,
    /// How long a finished job's result is kept after it finishes.
    pub keep: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            waiting: 16,
            still_timeout: Duration::from_secs(120),
            moving_timeout: Duration::from_secs(900),
            keep: Duration::from_secs(600),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// Why a job did not finish, with the status a request for its result answers.
#[derive(Clone, Debug)]
pub struct Failure {
    pub status: u16,
    pub message: String,
}

struct Job {
    /// Until the worker takes it.
    spec: Option<Spec>,
    format: Format,
    status: Status,
    progress: f32,
    stage: String,
    failure: Option<Failure>,
    result: Option<PathBuf>,
    cancel: Arc<AtomicBool>,
    /// When a running job is stopped.
    deadline: Option<Instant>,
    /// Set once it finishes, from when its result is kept.
    finished: Option<Instant>,
}

struct State {
    waiting: VecDeque<Id>,
    jobs: HashMap<Id, Job>,
}

pub struct Queue {
    state: Mutex<State>,
    changed: Condvar,
    settings: Settings,
    /// Where inputs and results are written, removed with the queue.
    folder: tempfile::TempDir,
}

/// A job the worker has taken.
pub struct Taken {
    pub id: Id,
    pub spec: Spec,
    pub cancel: Arc<AtomicBool>,
}

impl Queue {
    pub fn new(settings: Settings) -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                waiting: VecDeque::new(),
                jobs: HashMap::new(),
            }),
            changed: Condvar::new(),
            settings,
            folder: tempfile::Builder::new()
                .prefix("crtsim-service-")
                .tempdir()?,
        }))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding it leaves nothing half written that matters more than serving.
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Where a job writes its files.
    pub fn folder(&self) -> &std::path::Path {
        self.folder.path()
    }

    /// Queues `spec`, or refuses it when the queue is full.
    pub fn submit(&self, spec: Spec) -> Result<Id, Refused> {
        let mut state = self.lock();
        if state.waiting.len() >= self.settings.waiting {
            return Err(Refused {
                status: 503,
                message: format!(
                    "{} jobs are already waiting; try again shortly",
                    state.waiting.len()
                ),
            });
        }
        let id = new_id();
        state.jobs.insert(
            id.clone(),
            Job {
                format: spec.format,
                spec: Some(spec),
                status: Status::Queued,
                progress: 0.,
                stage: "Waiting".into(),
                failure: None,
                result: None,
                cancel: Arc::new(AtomicBool::new(false)),
                deadline: None,
                finished: None,
            },
        );
        state.waiting.push_back(id.clone());
        self.changed.notify_all();
        Ok(id)
    }

    /// Waits until job `id` finishes or `until` passes, and returns its status then.
    pub fn wait(&self, id: &str, until: Instant) -> Option<Status> {
        let mut state = self.lock();
        loop {
            let status = state.jobs.get(id)?.status;
            let left = until.saturating_duration_since(Instant::now());
            if status.finished() || left.is_zero() {
                return Some(status);
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// How job `id` is getting on, as JSON.
    pub fn describe(&self, id: &str) -> Option<serde_json::Value> {
        let state = self.lock();
        let job = state.jobs.get(id)?;
        let mut description = serde_json::json!({
            "id": id,
            "status": job.status.name(),
            "progress": job.progress,
            "stage": job.stage,
        });
        if job.status == Status::Queued {
            let place = state.waiting.iter().position(|w| w == id).unwrap_or(0);
            description["ahead"] = (place).into();
        }
        if let Some(failure) = &job.failure {
            description["error"] = failure.message.clone().into();
        }
        if job.status == Status::Done {
            description["result"] = format!("/jobs/{id}/result").into();
        }
        Some(description)
    }

    /// Job `id`'s result and its format, or why there is none: it is not finished, or failed.
    pub fn result(&self, id: &str) -> Result<(PathBuf, Format), Failure> {
        let state = self.lock();
        let job = state.jobs.get(id).ok_or_else(|| Failure {
            status: 404,
            message: "No such job; results are kept only for a while".into(),
        })?;
        match (&job.result, &job.failure) {
            (Some(path), _) => Ok((path.clone(), job.format)),
            (None, Some(failure)) => Err(failure.clone()),
            (None, None) => Err(Failure {
                status: 409,
                message: format!("The job is {}", job.status.name()),
            }),
        }
    }

    /// Stops job `id` and forgets it, removing its result. Whether there was such a job.
    pub fn cancel(&self, id: &str) -> bool {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(id) else {
            return false;
        };
        job.cancel.store(true, Ordering::Relaxed);
        if job.status == Status::Running {
            // The worker finishes it as cancelled, and the sweep forgets it.
            job.deadline = Some(Instant::now());
            return true;
        }
        state.waiting.retain(|w| w != id);
        if let Some(job) = state.jobs.remove(id) {
            remove(job.result);
        }
        self.changed.notify_all();
        true
    }

    /// How many jobs are waiting, and whether one is running.
    pub fn load(&self) -> (usize, bool) {
        let state = self.lock();
        let running = state.jobs.values().any(|j| j.status == Status::Running);
        (state.waiting.len(), running)
    }

    /// The next job, once there is one: for the worker.
    pub fn next(&self) -> Taken {
        let mut state = self.lock();
        loop {
            while let Some(id) = state.waiting.pop_front() {
                let Some(job) = state.jobs.get_mut(&id) else {
                    continue;
                };
                let Some(spec) = job.spec.take() else {
                    continue;
                };
                job.status = Status::Running;
                job.stage = "Starting".into();
                let limit = match spec.input {
                    crate::spec::Input::Still(_) => self.settings.still_timeout,
                    crate::spec::Input::Moving { .. } => self.settings.moving_timeout,
                };
                job.deadline = Some(Instant::now() + limit);
                let cancel = job.cancel.clone();
                self.changed.notify_all();
                return Taken { id, spec, cancel };
            }
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Job `id` has got this far.
    pub fn progress(&self, id: &str, fraction: f32, stage: String) {
        if let Some(job) = self.lock().jobs.get_mut(id) {
            job.progress = fraction;
            job.stage = stage;
        }
    }

    /// Job `id` has finished, with its result or why it has none.
    pub fn finish(&self, id: &str, outcome: Result<PathBuf, Failure>) {
        let mut state = self.lock();
        let Some(job) = state.jobs.get_mut(id) else {
            if let Ok(path) = outcome {
                remove(Some(path));
            }
            return;
        };
        let timed_out = job.deadline.is_some_and(|d| Instant::now() >= d);
        job.deadline = None;
        job.finished = Some(Instant::now());
        match outcome {
            Ok(path) if !job.cancel.load(Ordering::Relaxed) => {
                job.status = Status::Done;
                job.progress = 1.;
                job.stage = "Done".into();
                job.result = Some(path);
            }
            other => {
                if let Ok(path) = other.as_ref() {
                    remove(Some(path.clone()));
                }
                let cancelled = job.cancel.load(Ordering::Relaxed);
                job.status = if cancelled {
                    Status::Cancelled
                } else {
                    Status::Failed
                };
                job.failure = Some(match other {
                    _ if cancelled && timed_out => Failure {
                        status: 504,
                        message: "The render took longer than this service allows".into(),
                    },
                    _ if cancelled => Failure {
                        status: 409,
                        message: "The job was cancelled".into(),
                    },
                    Err(failure) => failure,
                    Ok(_) => unreachable!("a result that was not cancelled is done"),
                });
            }
        }
        self.changed.notify_all();
    }

    /// Stops jobs past their deadline, and forgets finished ones kept long enough, removing
    /// their files. The service calls it every second.
    pub fn sweep(&self) {
        let mut state = self.lock();
        let now = Instant::now();
        let keep = self.settings.keep;
        for job in state.jobs.values_mut() {
            if job.deadline.is_some_and(|d| now >= d) {
                job.cancel.store(true, Ordering::Relaxed);
            }
        }
        let expired: Vec<Id> = state
            .jobs
            .iter()
            .filter(|(_, job)| job.finished.is_some_and(|f| now.duration_since(f) >= keep))
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            if let Some(job) = state.jobs.remove(&id) {
                remove(job.result);
            }
        }
    }
}

fn remove(path: Option<PathBuf>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

/// 128 random bits, in hex.
fn new_id() -> Id {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).expect("the system's random numbers");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Runs jobs one at a time on `renderer`, the only thread that uses it, until the process ends.
pub fn work(queue: Arc<Queue>, renderer: Renderer) {
    loop {
        let Taken { id, spec, cancel } = queue.next();
        let report = |fraction: f32, stage: String| queue.progress(&id, fraction, stage);
        let outcome = crate::render::run(&renderer, spec, queue.folder(), &id, &cancel, report);
        queue.finish(&id, outcome);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Spec {
        Spec {
            input: crate::spec::Input::Still(vec![]),
            config: Default::default(),
            format: Format::Png,
            space: Default::default(),
            profiles: Default::default(),
            animation: Default::default(),
            video: Default::default(),
            seconds: 30.,
        }
    }

    #[test]
    fn a_job_past_its_deadline_is_stopped_and_its_result_forgotten_in_time() {
        let queue = Queue::new(Settings {
            waiting: 1,
            still_timeout: Duration::ZERO,
            keep: Duration::ZERO,
            ..Default::default()
        })
        .unwrap();
        let id = queue.submit(spec()).ok().unwrap();
        assert_eq!(
            queue.submit(spec()).err().map(|r| r.status),
            Some(503),
            "one may wait"
        );
        assert_eq!(queue.describe(&id).unwrap()["status"], "queued");
        let taken = queue.next();
        assert_eq!(taken.id, id);
        assert_eq!(queue.load(), (0, true));
        queue.sweep();
        assert!(taken.cancel.load(Ordering::Relaxed), "past its deadline");
        queue.finish(
            &id,
            Err(Failure {
                status: 500,
                message: "Render cancelled".into(),
            }),
        );
        assert_eq!(queue.result(&id).err().map(|f| f.status), Some(504));
        assert_eq!(queue.describe(&id).unwrap()["status"], "cancelled");
        queue.sweep();
        assert!(queue.describe(&id).is_none(), "kept no longer than asked");
    }

    #[test]
    fn a_finished_job_keeps_its_result_until_it_is_cancelled() {
        let queue = Queue::new(Settings::default()).unwrap();
        let id = queue.submit(spec()).ok().unwrap();
        let taken = queue.next();
        let path = queue.folder().join("done.png");
        std::fs::write(&path, b"png").unwrap();
        queue.finish(&taken.id, Ok(path.clone()));
        assert_eq!(queue.wait(&id, Instant::now()), Some(Status::Done));
        assert_eq!(queue.result(&id).unwrap(), (path.clone(), Format::Png));
        assert!(queue.cancel(&id));
        assert!(!path.exists() && queue.describe(&id).is_none());
        assert!(!queue.cancel(&id));
        let waiting = queue.submit(spec()).ok().unwrap();
        assert!(queue.cancel(&waiting), "a waiting job leaves the queue");
        assert_eq!(queue.load(), (0, false));
    }
}
