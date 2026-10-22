//! The life of one drag, as far as the interaction is concerned.
//!
//! The drag that is over the wheel has a life of its own, and several things
//! happen at once around it: the pointer comes and goes, the drag is ended
//! by Windows, a drop is handed over, and the work that was asked for runs
//! on a worker of its own.
//!
//! The rules here are the ones that have to survive all of that:
//!
//! 1. a drop that has been accepted is never taken back by a later event,
//! 2. a delayed event only ever acts on the drag it belongs to,
//! 3. and a drag that is over leaves nothing behind for the next one.
//!
//! Every delayed event carries the number of the drag it belongs to, and
//! nothing is done unless that drag is the one that is running. That is what
//! keeps a cancellation from one drag from touching another.

use std::time::{Duration, Instant};

/// How long the pointer may be outside the wheel before the interaction is
/// cancelled.
///
/// The pointer leaves the wheel for a moment all the time: the file is moved
/// a little, or the wheel is pushed out of the way. A short grace keeps that
/// from ending the interaction, and a pointer that does not come back ends
/// it.
pub const LEAVE_GRACE: Duration = Duration::from_millis(300);

/// What is going on with the drag that is over the wheel.
#[derive(Clone, Debug, Default)]
pub struct DragLifecycle {
    /// The drag this is about.
    session: u64,
    /// Whether the drop of this drag has been accepted.
    accepted: bool,
    /// When the pointer may be given up on, and the drag it belongs to.
    leaving: Option<(u64, Instant)>,
}

impl DragLifecycle {
    /// A lifecycle with no drag in it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The drag that is running.
    pub fn session(&self) -> u64 {
        self.session
    }

    /// A new drag begins.
    ///
    /// Everything the drag before left behind is dropped: the acceptance, and
    /// any leave that was still waiting.
    pub fn begin(&mut self, session: u64) {
        self.session = session;
        self.accepted = false;
        self.leaving = None;
    }

    /// The drop of the drag was accepted.
    ///
    /// The acceptance is the last word: a leave that is still waiting is
    /// dropped, and a leave that comes afterwards is ignored, so that a
    /// pointer that leaves the wheel a moment after the file was let go of
    /// cannot undo what was accepted.
    pub fn accept(&mut self) {
        self.accepted = true;
        self.leaving = None;
    }

    /// Whether the drop of the drag was accepted.
    pub fn is_accepted(&self) -> bool {
        self.accepted
    }

    /// The pointer left the wheel.
    ///
    /// The leave belongs to the drag that is running. A leave from another
    /// drag is ignored, and a leave after the drop was accepted is ignored.
    pub fn pointer_left(&mut self, session: u64, now: Instant) {
        if session != self.session || self.accepted {
            return;
        }
        self.leaving = Some((session, now + LEAVE_GRACE));
    }

    /// The pointer came back to the wheel.
    ///
    /// The leave that was waiting is dropped, so that the interaction goes on
    /// as if the pointer had never left.
    pub fn pointer_returned(&mut self) {
        self.leaving = None;
    }

    /// The interaction is over, whatever brought it to an end.
    ///
    /// Anything that was waiting is dropped. This may be called more than
    /// once for the same drag, and it may be called for a drag that has
    /// already been accepted: neither does any harm.
    pub fn cancel(&mut self) {
        self.leaving = None;
    }

    /// Whether the grace period is over.
    ///
    /// The number of the drag is given back, so that the caller can check
    /// that the drag is still the one that is running before it acts.
    pub fn leave_expired(&mut self, now: Instant) -> Option<u64> {
        let (session, deadline) = self.leaving?;
        if now < deadline {
            return None;
        }
        self.leaving = None;
        Some(session)
    }

    /// Whether a leave is waiting to be given up on.
    pub fn is_leaving(&self) -> bool {
        self.leaving.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_new_drag_leaves_nothing_behind() {
        let mut life = DragLifecycle::new();
        life.begin(1);
        life.accept();
        life.begin(2);
        assert!(!life.is_accepted());
        assert_eq!(life.session(), 2);
        assert!(!life.is_leaving());
    }

    #[test]
    fn a_leave_is_only_given_up_on_after_the_grace() {
        let mut life = DragLifecycle::new();
        life.begin(1);
        let start = now();
        life.pointer_left(1, start);
        assert!(life.is_leaving());
        // The pointer is still outside, but not for long enough yet.
        assert_eq!(life.leave_expired(start + Duration::from_millis(299)), None);
        assert!(life.is_leaving());
        // Now it has been long enough.
        assert_eq!(
            life.leave_expired(start + Duration::from_millis(300)),
            Some(1)
        );
        assert!(!life.is_leaving());
    }

    #[test]
    fn a_pointer_that_comes_back_keeps_the_interaction() {
        let mut life = DragLifecycle::new();
        life.begin(1);
        let start = now();
        life.pointer_left(1, start);
        life.pointer_returned();
        assert!(!life.is_leaving());
        assert_eq!(life.leave_expired(start + Duration::from_secs(1)), None);
    }

    #[test]
    fn an_accepted_drop_is_never_taken_back() {
        let mut life = DragLifecycle::new();
        life.begin(1);
        let start = now();
        life.pointer_left(1, start);
        // The file is let go of on a choice.
        life.accept();
        assert!(life.is_accepted());
        assert!(!life.is_leaving());
        // The pointer leaves the wheel afterwards: nothing may be given up on.
        life.pointer_left(1, start);
        assert!(!life.is_leaving());
        assert_eq!(life.leave_expired(start + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_leave_of_another_drag_is_ignored() {
        let mut life = DragLifecycle::new();
        life.begin(2);
        let start = now();
        // The leave belongs to the drag before.
        life.pointer_left(1, start);
        assert!(!life.is_leaving());
        assert_eq!(life.leave_expired(start + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_leave_that_belongs_to_an_older_drag_is_not_given_up_on() {
        let mut life = DragLifecycle::new();
        life.begin(1);
        let start = now();
        life.pointer_left(1, start);
        // Another drag begins before the grace is over. It leaves nothing of
        // the drag before behind, so the leave of the drag before cannot
        // come due and cannot touch the drag that is running.
        life.begin(2);
        assert!(!life.is_leaving());
        assert_eq!(life.leave_expired(start + Duration::from_secs(1)), None);
    }

    #[test]
    fn cancelling_twice_does_no_harm() {
        let mut life = DragLifecycle::new();
        life.begin(1);
        life.pointer_left(1, now());
        life.cancel();
        life.cancel();
        assert!(!life.is_leaving());
        assert!(!life.is_accepted());
    }

    #[test]
    fn the_grace_is_three_hundred_milliseconds() {
        assert_eq!(LEAVE_GRACE, Duration::from_millis(300));
    }
}
