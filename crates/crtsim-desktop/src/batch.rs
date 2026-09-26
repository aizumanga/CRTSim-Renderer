//! The batch export queue: jobs that each render one file with the settings in use when it was
//! added, one at a time, in order. A batch export never replaces an existing file.
use crate::*;
use anyhow::{ensure, Result};
use crtsim_media::Options;
use serde::{Deserialize, Serialize};

/// The most jobs a queue, or a project's saved queue, holds.
pub const MAX_JOBS: usize = 256;

/// One file to export, with the settings captured when it was added. Saved in projects and
/// sessions as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub(crate) source: PathBuf,
    pub(crate) output: PathBuf,
    pub(crate) config: Config,
    pub(crate) options: Options,
    pub(crate) status: Status,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Status {
    Pending,
    Running,
    Done,
    Cancelled,
    Failed(String),
}

impl Status {
    /// Whether the job can be run again.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Failed(_) | Self::Cancelled)
    }
}

impl Item {
    /// Checks a job read back from a saved project, whose paths may be relative to `root`. A
    /// job that was exporting when the project was saved is pending again.
    pub fn restore(&mut self, root: &Path) -> Result<()> {
        self.config.validate()?;
        self.options.validate()?;
        for path in [&mut self.source, &mut self.output] {
            if path.is_relative() {
                *path = root.join(&*path);
            }
        }
        if self.status == Status::Running {
            self.status = Status::Pending;
        }
        Ok(())
    }
}

/// A change to the queue made from its window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Retry,
    MoveUp,
    Remove,
}

#[derive(Default)]
pub struct Queue {
    items: Vec<Item>,
    /// Start the next pending job whenever the work thread is free.
    running: bool,
    /// The job exporting now.
    active: Option<usize>,
}

impl Queue {
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// Replaces the jobs with a saved project's, paused.
    pub fn replace(&mut self, items: Vec<Item>) {
        *self = Self {
            items,
            ..Self::default()
        };
    }

    /// Adds a job for each of `sources`, into `folder`, keeping these settings. Stops with an
    /// error once the queue is full; the jobs added before that stay.
    pub fn add(
        &mut self,
        sources: Vec<PathBuf>,
        folder: &Path,
        config: &Config,
        options: &Options,
    ) -> Result<()> {
        for source in sources {
            ensure!(
                self.items.len() < MAX_JOBS,
                "Batch queue is limited to {MAX_JOBS} jobs"
            );
            let output = self.output_for(&source, folder);
            self.items.push(Item {
                source,
                output,
                config: config.clone(),
                options: options.clone(),
                status: Status::Pending,
            });
        }
        Ok(())
    }

    /// Where a job for `source` writes in `folder`: named after the source, as a video or a PNG
    /// as its kind asks, and numbered past any file that exists or another job will write.
    fn output_for(&self, source: &Path, folder: &Path) -> PathBuf {
        let stem = file_stem(source);
        let extension = if crtsim_media::MediaKind::of(source).batch_as_video() {
            "mkv"
        } else {
            "png"
        };
        (0..)
            .map(|n| match n {
                0 => folder.join(format!("{stem}-crt.{extension}")),
                n => folder.join(format!("{stem}-crt-{n}.{extension}")),
            })
            .find(|path| !path.exists() && !self.items.iter().any(|item| &item.output == path))
            .expect("some number is free")
    }

    pub fn running(&self) -> bool {
        self.running
    }

    /// Starts or pauses the queue. A paused queue finishes the job exporting now.
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// Whether one of the jobs is exporting now.
    pub fn exporting(&self) -> bool {
        self.active.is_some()
    }

    /// The next pending job, now marked running, unless the queue is paused. A job whose
    /// output has appeared since it was added fails instead. With no job left, the queue pauses.
    pub fn next(&mut self) -> Option<Item> {
        if !self.running {
            return None;
        }
        loop {
            let Some(index) = self
                .items
                .iter()
                .position(|item| item.status == Status::Pending)
            else {
                self.running = false;
                return None;
            };
            let item = &mut self.items[index];
            if item.output.exists() {
                item.status = Status::Failed(
                    "Destination already exists. Remove it or add a new job; batch exports \
                     never intentionally replace existing files."
                        .into(),
                );
                continue;
            }
            item.status = Status::Running;
            self.active = Some(index);
            return Some(item.clone());
        }
    }

    /// An export ended with `result`. Whether it was the queue's job. Cancelling pauses the
    /// queue rather than moving on, also when it was `asked_to_stop` too late to stop and the
    /// job finished anyway.
    pub fn finished(&mut self, result: &worker::Outcome<PathBuf>, asked_to_stop: bool) -> bool {
        let Some(index) = self.active.take() else {
            return false;
        };
        self.items[index].status = match result {
            Ok(_) => Status::Done,
            Err(Failure::Cancelled) => Status::Cancelled,
            Err(Failure::Failed(e)) => Status::Failed(e.clone()),
        };
        if asked_to_stop || matches!(result, Err(Failure::Cancelled)) {
            self.running = false;
        }
        true
    }

    /// Whether the job at `index` can be edited now: not while any job exports, since that
    /// would move the job being exported.
    pub fn can_edit(&self, index: usize) -> bool {
        self.active.is_none()
            && self
                .items
                .get(index)
                .is_some_and(|item| item.status != Status::Running)
    }

    pub fn edit(&mut self, index: usize, edit: Edit) {
        if !self.can_edit(index) {
            return;
        }
        match edit {
            Edit::Retry if self.items[index].status.retryable() => {
                self.items[index].status = Status::Pending;
            }
            Edit::MoveUp if index > 0 => self.items.swap(index, index - 1),
            Edit::Remove => {
                self.items.remove(index);
            }
            _ => {}
        }
    }
}

type Chosen = Option<(Vec<PathBuf>, PathBuf)>;

/// The file chooser open for new batch jobs, and the settings on screen when it opened, which
/// the jobs keep.
pub struct Picking {
    chosen: mpsc::Receiver<Chosen>,
    config: Config,
    options: Options,
}

impl App {
    /// Asks for the files to add to the queue, then the folder their exports go to.
    fn pick_batch_files(&mut self, ctx: &egui::Context) {
        self.stop_playback();
        self.dialog_open = true;
        let (send, chosen) = mpsc::channel();
        self.batch_picking = Some(Picking {
            chosen,
            config: self.config.clone(),
            options: self.video_options.clone(),
        });
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .add_filter("Images and videos", &files::media_extensions())
                .pick_files()
                .and_then(|files| {
                    rfd::FileDialog::new()
                        .set_title("Batch export destination")
                        .pick_folder()
                        .map(|folder| (files, folder))
                });
            let _ = send.send(result);
            ctx.request_repaint();
        });
    }

    /// Adds the jobs for the files chosen, once the chooser has closed.
    pub(crate) fn receive_batch_files(&mut self) {
        let Some(picking) = self.batch_picking.take() else {
            return;
        };
        match picking.chosen.try_recv() {
            Err(mpsc::TryRecvError::Empty) => self.batch_picking = Some(picking),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.dialog_open = false;
                self.error = Some("File chooser closed unexpectedly".into());
            }
            Ok(None) => self.dialog_open = false,
            Ok(Some((sources, folder))) => {
                self.dialog_open = false;
                let added = self
                    .queue
                    .add(sources, &folder, &picking.config, &picking.options);
                if let Err(e) = added {
                    self.error = Some(format!("{e:#}"));
                }
                self.show_queue = true;
                self.save_session();
            }
        }
    }

    /// Starts the queue's next job once nothing else needs the work thread or the preview.
    pub(crate) fn dispatch_queue(&mut self) {
        if !self.can_start_work()
            || self.schedule.rendering()
            || self.workflow.recovery_offered()
            || self.playback.is_some()
        {
            return;
        }
        let Some(job) = self.queue.next() else {
            return;
        };
        let status = format!("Batch export {}", job.source.display());
        let export = worker::Export::Batch {
            source: job.source,
            options: job.options,
            config: job.config,
        };
        self.start_export(export, job.output, status, None);
        self.save_session();
    }

    pub(crate) fn queue_finished(&mut self, result: &worker::Outcome<PathBuf>) {
        let asked_to_stop = self
            .work
            .cancel()
            .is_some_and(|cancel| cancel.load(Ordering::Relaxed));
        if self.queue.finished(result, asked_to_stop) {
            self.save_session();
        }
    }

    pub(crate) fn queue_window(&mut self, ctx: &egui::Context) {
        if !self.show_queue {
            return;
        }
        let mut edit = None;
        let mut window = self.window_state("Batch export queue");
        let open =
            chrome::tool_window(ctx, "Batch export queue", [650., 560.], &mut window, |ui| {
                ui.label(
                    "Each job keeps the settings used when it was added. Videos use MKV to \
                 preserve more tracks.",
                );
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.modal_open(), egui::Button::new("Add files…"))
                        .clicked()
                    {
                        self.pick_batch_files(ctx);
                    }
                    if ui
                        .add_enabled(!self.modal_open(), egui::Button::new("Video settings…"))
                        .clicked()
                    {
                        self.open_video_export(true);
                    }
                    let running = self.queue.running();
                    if ui
                        .button(if running {
                            "Pause after current"
                        } else {
                            "Start / resume"
                        })
                        .clicked()
                    {
                        self.stop_playback();
                        self.queue.set_running(!running);
                    }
                    if ui
                        .add_enabled(self.queue.exporting(), egui::Button::new("Cancel current"))
                        .clicked()
                    {
                        if let Some(cancel) = self.work.cancel() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        self.queue.set_running(false);
                    }
                });
                egui::ScrollArea::vertical()
                    .max_height(400.)
                    .show(ui, |ui| {
                        for (index, item) in self.queue.items().iter().enumerate() {
                            ui.push_id(index, |ui| {
                                ui.group(|ui| {
                                    ui.label(item.source.display().to_string());
                                    ui.small(format!("→ {}", item.output.display()));
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(match &item.status {
                                            Status::Pending => "Pending".into(),
                                            Status::Running => "Exporting…".into(),
                                            Status::Done => "Saved".into(),
                                            Status::Cancelled => "Cancelled".into(),
                                            Status::Failed(e) => format!("Failed: {e}"),
                                        });
                                        if !self.queue.can_edit(index) {
                                            return;
                                        }
                                        if item.status.retryable()
                                            && ui.small_button("Retry").clicked()
                                        {
                                            edit = Some((index, Edit::Retry));
                                        }
                                        if index > 0 && ui.small_button("Move up").clicked() {
                                            edit = Some((index, Edit::MoveUp));
                                        }
                                        if ui.small_button("Remove").clicked() {
                                            edit = Some((index, Edit::Remove));
                                        }
                                    });
                                });
                            });
                        }
                    });
            });
        self.store_window_state("Batch export queue", window);
        if let Some((index, change)) = edit {
            self.queue.edit(index, change);
        }
        self.show_queue = open;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outputs(queue: &Queue) -> Vec<String> {
        queue
            .items()
            .iter()
            .map(|item| file_name(&item.output))
            .collect()
    }

    fn statuses(queue: &Queue) -> Vec<Status> {
        queue
            .items()
            .iter()
            .map(|item| item.status.clone())
            .collect()
    }

    /// A running queue of a job for each source, exporting into `folder`.
    fn queue(folder: &Path, sources: &[&str]) -> Queue {
        let mut queue = Queue::default();
        let sources = sources.iter().map(PathBuf::from).collect();
        queue
            .add(sources, folder, &Config::general(), &Options::default())
            .unwrap();
        queue.set_running(true);
        queue
    }

    #[test]
    fn a_job_is_named_after_its_source_and_never_over_a_file_or_another_job() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("clip-crt.png"), b"taken").unwrap();
        let queue = queue(
            dir.path(),
            &["a/clip.png", "b/clip.png", "movie.MKV", "c.gif"],
        );
        assert_eq!(
            outputs(&queue),
            [
                "clip-crt-1.png",
                "clip-crt-2.png",
                "movie-crt.mkv",
                "c-crt.png"
            ]
        );
    }

    #[test]
    fn the_queue_stops_adding_once_it_is_full() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = Queue::default();
        let sources = (0..MAX_JOBS + 10)
            .map(|n| PathBuf::from(format!("{n}.png")))
            .collect();
        let added = queue.add(sources, dir.path(), &Config::general(), &Options::default());
        assert!(added.unwrap_err().to_string().contains("256"));
        assert_eq!(queue.items().len(), MAX_JOBS);
    }

    #[test]
    fn jobs_run_in_order_and_one_whose_output_appeared_fails_instead() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = queue(dir.path(), &["first.png", "second.png", "third.png"]);
        std::fs::write(dir.path().join("first-crt.png"), b"keep me").unwrap();
        let job = queue.next().unwrap();
        assert_eq!(file_name(&job.output), "second-crt.png");
        assert!(matches!(queue.items()[0].status, Status::Failed(_)));
        assert!(queue.exporting());
        assert!(queue.finished(&Ok(job.output), false));
        assert_eq!(file_name(&queue.next().unwrap().source), "third.png");
        assert!(queue.finished(&Err(Failure::Failed("full disk".into())), false));
        assert_eq!(queue.next().map(|job| job.source), None);
        assert!(!queue.running(), "a queue with nothing left pauses");
        assert_eq!(
            std::fs::read(dir.path().join("first-crt.png")).unwrap(),
            b"keep me"
        );
        assert!(
            !queue.finished(&Ok("elsewhere.png".into()), false),
            "not one of the queue's"
        );
    }

    #[test]
    fn cancelling_pauses_the_queue_even_when_it_came_too_late() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = queue(dir.path(), &["first.png", "second.png", "third.png"]);
        let job = queue.next().unwrap();
        queue.finished(&Ok(job.output), true);
        assert!(!queue.running() && queue.next().is_none());
        queue.set_running(true);
        queue.next().unwrap();
        queue.finished(&Err(Failure::Cancelled), false);
        assert!(!queue.running());
        assert_eq!(
            statuses(&queue),
            [Status::Done, Status::Cancelled, Status::Pending]
        );
    }

    #[test]
    fn jobs_are_edited_only_while_none_exports() {
        let dir = tempfile::tempdir().unwrap();
        let mut queue = queue(dir.path(), &["first.png", "second.png", "third.png"]);
        queue.next().unwrap();
        queue.edit(2, Edit::Remove);
        assert_eq!(queue.items().len(), 3, "refused while the first exports");
        queue.finished(&Err(Failure::Cancelled), false);
        queue.edit(0, Edit::Retry);
        queue.edit(2, Edit::MoveUp);
        queue.edit(0, Edit::Remove);
        assert_eq!(outputs(&queue), ["third-crt.png", "second-crt.png"]);
        queue.edit(0, Edit::Retry);
        assert_eq!(
            statuses(&queue),
            [Status::Pending, Status::Pending],
            "only a failed or cancelled job is retried"
        );
    }

    #[test]
    fn a_saved_job_is_checked_resolved_and_pending_again() {
        let root = std::env::temp_dir().join("projects");
        let elsewhere = std::env::temp_dir().join("output.png");
        let mut item = Item {
            source: "input.png".into(),
            output: elsewhere.clone(),
            config: Config::default(),
            options: Options::default(),
            status: Status::Running,
        };
        item.restore(&root).unwrap();
        assert_eq!(item.source, root.join("input.png"));
        assert_eq!(item.output, elsewhere);
        assert_eq!(item.status, Status::Pending);
        item.config.warmup = 1000;
        assert!(item.restore(&root).is_err());
    }

    #[test]
    fn a_queued_job_exports_with_its_own_settings_and_cancelling_it_pauses_the_queue() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &ctx,
            worker::Gpu::Own(wgpu::Backends::PRIMARY),
            None,
            Some(Smoke::new("unused-smoke.png".into())),
        );
        app.show_welcome = false;
        let (jobs, receive, _previews) = worker::Jobs::capture();
        app.jobs = jobs;
        let sources = vec!["first.png".into(), "second.png".into()];
        let captured = app.config.clone();
        app.queue
            .add(sources, dir.path(), &captured, &Options::default())
            .unwrap();
        app.config.bloom = 0.;
        app.queue.set_running(true);
        app.dispatch_queue();
        let Ok(Job::Export {
            export: worker::Export::Batch { config, .. },
            path,
            cancel,
        }) = receive.try_recv()
        else {
            panic!("Expected batch export");
        };
        assert_eq!(config, captured);
        // Asked while the file was being saved, after the last point the job checks.
        cancel.store(true, Ordering::Relaxed);
        app.queue_finished(&Ok(path));
        app.work = Work::Idle;
        assert!(!app.queue.running());
        app.dispatch_queue();
        assert!(
            receive.try_recv().is_err(),
            "the second job waits to be resumed"
        );
    }
}
