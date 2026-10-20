//! The worker's two lanes as the interface and the lanes themselves see them: the channels jobs
//! go through, the bells that wake a lane waiting for one, and how the lanes run.

use super::{Job, PreviewJob};
use std::{
    future::Future,
    pin::Pin,
    rc::Rc,
    sync::{mpsc, Arc, Mutex},
    task::{Poll, Waker},
};

/// Where the interface sends work. The worker has two lanes: previews have one of their own,
/// so they keep coming while an export holds the other for minutes. A preview queued behind an
/// export used to wait for all of it, and settings edited meanwhile could not be seen until it
/// finished.
#[derive(Clone)]
pub struct Jobs {
    work: WorkLane,
    preview: PreviewLane,
    /// Held only to ring once the senders above are gone, so it comes last.
    _bells: Bells,
}

/// A lane's sending end, which rings the lane's bell with each job.
pub struct Outbox<T> {
    jobs: mpsc::Sender<T>,
    bell: Arc<Bell>,
}

impl<T> Clone for Outbox<T> {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
            bell: self.bell.clone(),
        }
    }
}

impl<T> Outbox<T> {
    pub fn send(&self, job: T) -> Result<(), Stopped> {
        self.jobs.send(job).map_err(|_| Stopped)?;
        self.bell.ring();
        Ok(())
    }
}

/// The work lane's sending end, which the app's `lane::Lane` holds.
pub type WorkLane = Outbox<Job>;

/// The preview lane's sending end: the Preview holds one, and thumbnails go through `Jobs`.
pub type PreviewLane = Outbox<PreviewJob>;

/// A job could not be sent because its worker has stopped.
#[derive(Debug, PartialEq, Eq)]
pub struct Stopped;

impl Jobs {
    /// The interface's end of both lanes, and each lane's own end.
    pub(super) fn open() -> (Self, Inbox<Job>, Inbox<PreviewJob>) {
        let bells = Bells::default();
        let (work, jobs) = lane(&bells.0[0]);
        let (preview, previews) = lane(&bells.0[1]);
        (
            Self {
                work,
                preview,
                _bells: bells,
            },
            jobs,
            previews,
        )
    }

    pub fn send(&self, job: Job) -> Result<(), Stopped> {
        self.work.send(job)
    }
    /// The work lane, for the app's `Lane` to send its jobs on.
    pub fn work_lane(&self) -> WorkLane {
        self.work.clone()
    }
    pub fn preview(&self, job: PreviewJob) -> Result<(), Stopped> {
        self.preview.send(job)
    }
    /// The preview lane, for the Preview to send its own jobs on.
    pub fn preview_lane(&self) -> PreviewLane {
        self.preview.clone()
    }
    /// Asks both lanes to stop once their current job is done.
    pub fn shutdown(&self) {
        let _ = self.preview(PreviewJob::Shutdown);
        let _ = self.send(Job::Shutdown);
    }
    /// Channels in place of the lanes, for a test to inspect what the interface sends.
    #[cfg(test)]
    pub fn capture() -> (Self, mpsc::Receiver<Job>, mpsc::Receiver<PreviewJob>) {
        let (jobs, work, previews) = Self::open();
        (jobs, work.jobs, previews.jobs)
    }
}

/// A lane's two ends, rung by `bell`.
fn lane<T>(bell: &Arc<Bell>) -> (Outbox<T>, Inbox<T>) {
    let (send, receive) = mpsc::channel();
    (
        Outbox {
            jobs: send,
            bell: bell.clone(),
        },
        Inbox {
            jobs: receive,
            bell: bell.clone(),
        },
    )
}

/// The work lane's bell and the preview lane's. Dropped with the interface's end of the
/// channels, after them, they wake the lanes to see the channels closed and stop: a lane waiting
/// for a job would otherwise wait forever.
#[derive(Clone, Default)]
struct Bells([Arc<Bell>; 2]);

impl Drop for Bells {
    fn drop(&mut self) {
        for bell in &self.0 {
            bell.ring();
        }
    }
}

/// Wakes a lane waiting for its next job. A lane on a thread of its own sleeps until then; one
/// sharing the host's thread as a task gives it back.
#[derive(Default)]
struct Bell(Mutex<Option<Waker>>);

impl Bell {
    fn ring(&self) {
        if let Some(waker) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            waker.wake();
        }
    }
}

/// A lane's end of its channel.
pub(super) struct Inbox<T> {
    jobs: mpsc::Receiver<T>,
    bell: Arc<Bell>,
}

impl<T> Inbox<T> {
    /// The next job, or `None` once the interface has gone.
    pub(super) async fn next(&self) -> Option<T> {
        std::future::poll_fn(|cx| {
            let take = || match self.jobs.try_recv() {
                Ok(job) => Some(Some(job)),
                Err(mpsc::TryRecvError::Disconnected) => Some(None),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(job) = take() {
                return Poll::Ready(job);
            }
            *self.bell.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(cx.waker().clone());
            // A job sent between the look and the waker going in has already rung.
            take().map_or(Poll::Pending, Poll::Ready)
        })
        .await
    }

    /// Every job already waiting.
    pub(super) fn waiting(&self) -> impl Iterator<Item = T> + '_ {
        self.jobs.try_iter()
    }
}

/// A lane as a task, which runs on the thread it was made on.
pub type Task = Pin<Box<dyn Future<Output = ()>>>;

/// How the worker's two lanes run.
pub enum Runtime {
    /// Each lane on a thread of its own, sleeping between jobs: the desktop.
    Threads,
    /// Each lane a task on the host's own thread, handed to this to run. The browser has no
    /// threads; the lanes take turns there, at every await.
    #[cfg_attr(
        all(not(test), not(target_arch = "wasm32")),
        expect(dead_code, reason = "the desktop runs its lanes on threads")
    )]
    Tasks(Rc<dyn Fn(Task)>),
}

impl Runtime {
    /// Threads on the desktop; tasks in the browser, which has only the page's thread.
    pub fn host() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Self::Threads;
        #[cfg(target_arch = "wasm32")]
        return Self::Tasks(Rc::new(wasm_bindgen_futures::spawn_local));
    }
}
