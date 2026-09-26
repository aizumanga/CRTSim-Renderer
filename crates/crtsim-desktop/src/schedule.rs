//! The preview schedule: whether the preview on screen is out of date, and when to render the
//! next one. Edits and gallery auditions are rendered once they settle, one preview at a time,
//! and a preview that comes back for settings since replaced is never shown.
use std::time::{Duration, Instant};

/// How long a change must stay unchanged before it becomes an undo step and is previewed, so a
/// burst of edits, such as arrow-key steps, renders once.
const SETTLE: Duration = Duration::from_millis(180);

/// What changed what the preview should show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// The settings or the source. Previewed only with live preview on; otherwise it waits for
    /// Refresh.
    Edit,
    /// A look offered from a gallery, or taken back. Previewed whether or not live preview is
    /// on, since pointing at a look is asking to see it.
    Audition,
}

/// What a pending change needs on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// Nothing: no change is pending, or it has not settled.
    Nothing,
    /// The change has settled, so it becomes an undo step. With live preview off, the preview
    /// waits for Refresh.
    Commit,
    /// The change has settled: it becomes an undo step and is previewed.
    Preview,
}

pub struct Schedule {
    /// Counts the changes; a preview is of the revision it was asked for.
    revision: u64,
    /// Changed since the last preview was asked for: the change that asks the most, if any.
    pending: Option<Change>,
    changed_at: Instant,
    /// A preview has been asked for and has not come back.
    rendering: bool,
    /// The revision the picture on screen shows.
    shown: Option<u64>,
    /// Preview every edit once it settles, rather than on Refresh.
    pub live: bool,
}

impl Schedule {
    /// A schedule whose first preview, of the source as it opens, is already pending.
    pub fn new(now: Instant) -> Self {
        Self {
            revision: 0,
            pending: Some(Change::Edit),
            changed_at: now,
            rendering: false,
            shown: None,
            live: true,
        }
    }

    /// Records a change at `now`. The picture on screen is out of date from here on.
    pub fn changed(&mut self, change: Change, now: Instant) {
        self.revision += 1;
        self.pending = self.pending.max(Some(change));
        self.changed_at = now;
    }

    /// What the pending change needs at `now`. Nothing settles while the pointer is held down,
    /// so a slider being dragged renders when it is let go.
    pub fn due(&self, now: Instant, pointer_down: bool) -> Due {
        match self.pending {
            Some(change) if now.duration_since(self.changed_at) >= SETTLE && !pointer_down => {
                if self.live || change == Change::Audition {
                    Due::Preview
                } else {
                    Due::Commit
                }
            }
            _ => Due::Nothing,
        }
    }

    /// Whether frames must keep coming for the schedule to move on at `now`: a change is still
    /// settling or waiting to be previewed, or a preview is on its way.
    pub fn needs_frames(&self, now: Instant) -> bool {
        let waiting = self.pending.is_some_and(|change| {
            self.live || change == Change::Audition || now.duration_since(self.changed_at) < SETTLE
        });
        waiting || self.rendering
    }

    /// Asks for a preview of the pending change, or of the current settings when nothing is
    /// pending: the revision to render. `None` while another preview is still rendering, which
    /// leaves the change pending for when it is back.
    pub fn take(&mut self) -> Option<u64> {
        if self.rendering {
            return None;
        }
        self.pending = None;
        self.rendering = true;
        Some(self.revision)
    }

    /// A preview asked for with `take` came back, rendered or not. Whether it is of the current
    /// revision; one of an older revision must not be shown.
    pub fn returned(&mut self, revision: u64) -> bool {
        self.rendering = false;
        revision == self.revision
    }

    /// The picture on screen now shows the current revision.
    pub fn show_current(&mut self) {
        self.shown = Some(self.revision);
    }

    /// Whether the picture on screen shows the current revision.
    pub fn is_current(&self) -> bool {
        self.shown == Some(self.revision)
    }

    pub fn rendering(&self) -> bool {
        self.rendering
    }

    /// Forgets the pending change without previewing it, as playback does: it renders its own
    /// frames.
    pub fn drop_pending(&mut self) {
        self.pending = None;
    }

    /// Nothing more will be rendered: the renderer is gone. Drops the pending change and the
    /// preview on its way.
    pub fn stop(&mut self) {
        self.pending = None;
        self.rendering = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn an_edit_is_previewed_once_it_settles_and_only_with_live_preview() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        let edited = start + SETTLE * 2;
        schedule.changed(Change::Edit, edited);
        assert_eq!(schedule.due(edited + SETTLE - MS, false), Due::Nothing);
        assert!(schedule.needs_frames(edited + SETTLE - MS));
        assert_eq!(
            schedule.due(edited + SETTLE, true),
            Due::Nothing,
            "a held pointer is still editing"
        );
        assert_eq!(schedule.due(edited + SETTLE, false), Due::Preview);
        schedule.live = false;
        assert_eq!(schedule.due(edited + SETTLE, false), Due::Commit);
        assert!(
            !schedule.needs_frames(edited + SETTLE),
            "without live preview, a settled edit waits for Refresh without drawing"
        );
    }

    #[test]
    fn an_audition_is_previewed_even_without_live_preview_and_edits_do_not_undo_that() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        schedule.live = false;
        schedule.changed(Change::Audition, start);
        schedule.changed(Change::Edit, start);
        assert_eq!(schedule.due(start + SETTLE, false), Due::Preview);
        assert!(schedule.needs_frames(start + SETTLE));
        schedule.take();
        schedule.changed(Change::Edit, start);
        assert_eq!(
            schedule.due(start + SETTLE, false),
            Due::Commit,
            "once previewed, the audition asks for nothing more"
        );
    }

    #[test]
    fn one_preview_at_a_time_and_a_late_one_is_never_current() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        let first = schedule.take().unwrap();
        assert!(schedule.rendering() && schedule.needs_frames(start + SETTLE));
        assert_eq!(schedule.due(start + SETTLE, false), Due::Nothing);
        schedule.changed(Change::Edit, start);
        assert_eq!(schedule.take(), None, "the first is still rendering");
        assert!(!schedule.returned(first), "the settings changed since");
        assert!(!schedule.rendering() && !schedule.is_current());
        assert_eq!(
            schedule.due(start + SETTLE, false),
            Due::Preview,
            "the change waited for it"
        );
        let second = schedule.take().unwrap();
        assert!(schedule.returned(second));
        schedule.show_current();
        assert!(schedule.is_current());
        schedule.changed(Change::Edit, start);
        assert!(
            !schedule.is_current(),
            "out of date as soon as anything changes"
        );
    }

    #[test]
    fn dropping_the_pending_change_leaves_a_preview_on_its_way_and_stopping_does_not() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        schedule.take();
        schedule.changed(Change::Audition, start);
        schedule.drop_pending();
        assert_eq!(schedule.due(start + SETTLE, false), Due::Nothing);
        assert!(schedule.rendering());
        schedule.changed(Change::Edit, start);
        schedule.stop();
        assert_eq!(schedule.due(start + SETTLE, false), Due::Nothing);
        assert!(!schedule.rendering() && !schedule.needs_frames(start));
    }
}
