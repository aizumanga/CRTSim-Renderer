//! The preview schedule: whether the preview on screen is out of date, and when to render the
//! next one. With live preview on, an edit is rendered at once, even while a slider is being
//! dragged, as an interactive preview; once it settles it is rendered again exactly. Gallery
//! auditions are rendered once they settle. One preview renders at a time, and a settled
//! preview that comes back for settings since replaced is never shown.
use std::time::Duration;
use web_time::Instant;

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

/// How a preview is rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// While an edit is under way: the CRT already on screen runs on with the settings as they
    /// are now, the way a game's picture follows its options menu, so the edit shows at once
    /// over the glow of the settings before.
    Interactive,
    /// Once a change has settled, or on Refresh: a still, as an export renders it.
    Settled,
}

/// What a pending change needs on this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Due {
    /// Nothing: no change is pending, or it has not settled and has been previewed.
    Nothing,
    /// The change has settled, so it becomes an undo step. With live preview off, the preview
    /// waits for Refresh.
    Commit,
    /// A preview of this kind. A settled one also makes the change an undo step.
    Preview(Kind),
}

pub struct Schedule {
    /// Counts the changes; a preview is of the revision it was asked for.
    revision: u64,
    /// Changed since the last settled preview was asked for: the change that asks the most, if
    /// any.
    pending: Option<Change>,
    changed_at: Instant,
    /// The revision the last preview of either kind was asked for.
    asked: u64,
    /// The kind of preview asked for that has not come back.
    rendering: Option<Kind>,
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
            asked: 0,
            rendering: None,
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
    /// so a slider being dragged becomes one undo step when it is let go; with live preview on,
    /// each new position is previewed interactively meanwhile.
    pub fn due(&self, now: Instant, pointer_down: bool) -> Due {
        match self.pending {
            Some(change) if now.duration_since(self.changed_at) >= SETTLE && !pointer_down => {
                if self.live || change == Change::Audition {
                    Due::Preview(Kind::Settled)
                } else {
                    Due::Commit
                }
            }
            Some(Change::Edit) if self.live && self.asked < self.revision => {
                Due::Preview(Kind::Interactive)
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
        waiting || self.rendering.is_some()
    }

    /// Asks for a preview of the current settings: the revision to render. A settled one takes
    /// the pending change, or is a Refresh when nothing is pending; an interactive one leaves
    /// the change pending until it settles. `None` while another preview is still rendering,
    /// which leaves the change pending for when it is back.
    pub fn take(&mut self, kind: Kind) -> Option<u64> {
        if self.rendering.is_some() {
            return None;
        }
        if kind == Kind::Settled {
            self.pending = None;
        }
        self.asked = self.revision;
        self.rendering = Some(kind);
        Some(self.revision)
    }

    /// A preview asked for with `take` came back, rendered or not. Whether to show it: a
    /// settled preview only if it is of the current revision, since one of an older revision
    /// would stand for settings since replaced; an interactive one always, since an edit under
    /// way has moved on by the time any comes back, and each is closer to it than the last.
    pub fn returned(&mut self, revision: u64) -> bool {
        let kind = self.rendering.take();
        revision == self.revision || kind == Some(Kind::Interactive)
    }

    /// The picture on screen now shows `revision`.
    pub fn show(&mut self, revision: u64) {
        self.shown = Some(revision);
    }

    /// The picture on screen now shows the current revision.
    pub fn show_current(&mut self) {
        self.show(self.revision);
    }

    /// Whether the picture on screen shows the current revision.
    pub fn is_current(&self) -> bool {
        self.shown == Some(self.revision)
    }

    pub fn rendering(&self) -> bool {
        self.rendering.is_some()
    }

    /// Whether a settled preview is rendering, which the interface shows as busy. Interactive
    /// previews come and go many times a second while a slider is dragged, so showing those
    /// would only flicker.
    pub fn rendering_settled(&self) -> bool {
        self.rendering == Some(Kind::Settled)
    }

    /// Whether the picture on screen is out of date with nothing on its way to replace it:
    /// not while an edit is being previewed interactively, which keeps it within a frame or
    /// two of the settings, nor while a preview renders.
    pub fn stale(&self) -> bool {
        !self.is_current()
            && self.rendering.is_none()
            && !(self.live && self.pending == Some(Change::Edit))
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
        self.rendering = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);
    const INTERACTIVE: Due = Due::Preview(Kind::Interactive);
    const SETTLED: Due = Due::Preview(Kind::Settled);

    #[test]
    fn an_edit_is_previewed_at_once_and_again_once_it_settles_and_only_with_live_preview() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        let edited = start + SETTLE * 2;
        schedule.changed(Change::Edit, edited);
        assert_eq!(schedule.due(edited, true), INTERACTIVE, "even mid-drag");
        let asked = schedule.take(Kind::Interactive).unwrap();
        assert_eq!(schedule.due(edited, true), Due::Nothing, "one at a time");
        assert!(schedule.returned(asked));
        schedule.show(asked);
        assert_eq!(
            schedule.due(edited + SETTLE - MS, false),
            Due::Nothing,
            "nothing new to show until it settles"
        );
        assert!(schedule.needs_frames(edited + SETTLE - MS));
        assert!(!schedule.stale());
        assert_eq!(
            schedule.due(edited + SETTLE, true),
            Due::Nothing,
            "a held pointer is still editing"
        );
        assert_eq!(schedule.due(edited + SETTLE, false), SETTLED);
        schedule.live = false;
        assert_eq!(schedule.due(edited + SETTLE, false), Due::Commit);
        assert!(
            !schedule.needs_frames(edited + SETTLE),
            "without live preview, a settled edit waits for Refresh without drawing"
        );
        schedule.changed(Change::Edit, edited);
        assert_eq!(
            schedule.due(edited, false),
            Due::Nothing,
            "nor is it previewed while under way"
        );
        assert!(schedule.stale());
    }

    #[test]
    fn an_interactive_preview_leaves_the_edit_to_settle_and_shows_even_when_overtaken() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        schedule.changed(Change::Edit, start);
        let first = schedule.take(Kind::Interactive).unwrap();
        schedule.changed(Change::Edit, start + MS);
        assert!(schedule.returned(first), "closer than what is on screen");
        schedule.show(first);
        assert!(!schedule.is_current());
        assert_eq!(schedule.due(start + MS, true), INTERACTIVE);
        let second = schedule.take(Kind::Interactive).unwrap();
        assert!(schedule.returned(second));
        schedule.show(second);
        assert!(schedule.is_current());
        assert_eq!(
            schedule.due(start + MS + SETTLE, false),
            SETTLED,
            "still exactly rendered once it settles"
        );
        let settled = schedule.take(Kind::Settled).unwrap();
        assert_eq!(settled, second);
        assert!(schedule.returned(settled));
        assert_eq!(schedule.due(start + SETTLE * 4, false), Due::Nothing);
    }

    #[test]
    fn an_audition_is_previewed_once_settled_even_without_live_preview_and_edits_do_not_undo_that()
    {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        schedule.changed(Change::Audition, start);
        assert_eq!(
            schedule.due(start, false),
            Due::Nothing,
            "pointing along a gallery previews where the pointer stops"
        );
        schedule.live = false;
        schedule.changed(Change::Edit, start);
        assert_eq!(schedule.due(start + SETTLE, false), SETTLED);
        assert!(schedule.needs_frames(start + SETTLE));
        schedule.take(Kind::Settled);
        schedule.changed(Change::Edit, start);
        assert_eq!(
            schedule.due(start + SETTLE, false),
            Due::Commit,
            "once previewed, the audition asks for nothing more"
        );
    }

    #[test]
    fn one_preview_at_a_time_and_a_late_settled_one_is_never_current() {
        let start = Instant::now();
        let mut schedule = Schedule::new(start);
        let first = schedule.take(Kind::Settled).unwrap();
        assert!(schedule.rendering() && schedule.needs_frames(start + SETTLE));
        assert_eq!(schedule.due(start + SETTLE, false), Due::Nothing);
        schedule.changed(Change::Edit, start);
        assert_eq!(
            schedule.take(Kind::Settled),
            None,
            "the first is still rendering"
        );
        assert_eq!(schedule.take(Kind::Interactive), None);
        assert!(!schedule.returned(first), "the settings changed since");
        assert!(!schedule.rendering() && !schedule.is_current());
        assert_eq!(
            schedule.due(start + SETTLE, false),
            SETTLED,
            "the change waited for it"
        );
        let second = schedule.take(Kind::Settled).unwrap();
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
        schedule.take(Kind::Settled);
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
