//! The short message that says what came of an operation.
//!
//! The message is shown on the wheel's own window, so that it can be shown
//! after the wheel has gone: the work goes on while the wheel is away, and
//! what came of it is said when it is done.
//!
//! The message never names a file and never carries a path. The log has the
//! detail; this is what the person sees.

use std::time::{Duration, Instant};

use super::wheel::Notice;

/// How long a message is shown.
pub const NOTICE_TIME: Duration = Duration::from_secs(3);

/// How long a message takes to arrive, and to go away.
pub const NOTICE_FADE: Duration = Duration::from_millis(200);

/// A message about what came of an operation, and when it goes away.
#[derive(Clone, Debug)]
pub struct Showing {
    /// The words, and whether they are bad news.
    pub notice: Notice,
    /// When the message goes away.
    pub until: Instant,
}

impl Showing {
    /// A message that starts now.
    pub fn new(text: String, failed: bool, now: Instant) -> Self {
        Self {
            notice: Notice { text, failed },
            until: now + NOTICE_TIME,
        }
    }

    /// Whether the message is over.
    pub fn is_over(&self, now: Instant) -> bool {
        now >= self.until
    }

    /// How much of the message is showing.
    ///
    /// The message arrives over a moment and goes away over a moment, so
    /// that it does not blink in and out.
    pub fn appear(&self, now: Instant) -> f32 {
        let left = self.until.saturating_duration_since(now);
        let shown = NOTICE_TIME - left;
        let fade = NOTICE_FADE.as_secs_f32();
        if shown < NOTICE_FADE {
            shown.as_secs_f32() / fade
        } else {
            (left.as_secs_f32() / fade).min(1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn a_message_arrives_and_goes_away() {
        let start = now();
        let showing = Showing::new("1 file converted".to_string(), false, start);
        // It is not there at once, but it is there soon.
        assert!(showing.appear(start) < 1.0);
        assert!(showing.appear(start + Duration::from_millis(200)) > 0.9);
        // And it goes away over a moment.
        assert!(showing.appear(start + NOTICE_TIME) < 0.1);
        assert!(showing.is_over(start + NOTICE_TIME));
        assert!(!showing.is_over(start + NOTICE_TIME - Duration::from_millis(1)));
    }

    #[test]
    fn a_bad_message_is_marked_as_one() {
        let showing = Showing::new(
            "the file is already in that format".to_string(),
            true,
            now(),
        );
        assert!(showing.notice.failed);
        assert_eq!(showing.notice.text, "the file is already in that format");
    }

    #[test]
    fn a_good_message_is_not_marked_as_a_bad_one() {
        let showing = Showing::new("1 file converted".to_string(), false, now());
        assert!(!showing.notice.failed);
    }
}
