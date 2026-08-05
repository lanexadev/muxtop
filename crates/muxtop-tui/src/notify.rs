// Notifications.
//
// muxtop 0.4 had a single status string and guessed its severity by looking for
// the substring "failed" — so a reworded error rendered on the success colour.
// Severity is now carried by the message, several messages can be in flight at
// once, and nothing is ever lost: everything lands in a log the user can reopen
// with Ctrl+L after the toast has gone.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::ui::sanitize::scrub_ctrl;
use crate::ui::theme::Level;

/// How long a toast stays on screen, by severity. An error the user did not
/// see is a bug report we never receive, so errors linger.
const TTL_INFO: Duration = Duration::from_secs(4);
const TTL_WARNING: Duration = Duration::from_secs(8);
const TTL_ERROR: Duration = Duration::from_secs(12);

/// Most toasts on screen at once. Beyond this the oldest is retired early —
/// it is still in the log.
const MAX_ACTIVE: usize = 3;

/// How many messages the log keeps.
const LOG_CAPACITY: usize = 50;

/// A single message.
#[derive(Debug, Clone)]
pub struct Toast {
    pub level: Level,
    pub text: String,
    pub at: Instant,
}

impl Toast {
    fn ttl(&self) -> Duration {
        match self.level {
            Level::Error => TTL_ERROR,
            Level::Warning => TTL_ERROR.min(TTL_WARNING),
            _ => TTL_INFO,
        }
    }

    fn is_expired(&self) -> bool {
        self.at.elapsed() >= self.ttl()
    }
}

/// The toast stack plus the message log behind it.
#[derive(Debug, Default)]
pub struct Notifier {
    active: Vec<Toast>,
    log: VecDeque<Toast>,
}

impl Notifier {
    pub fn new() -> Self {
        Self::default()
    }

    /// Post a message.
    ///
    /// Control characters are scrubbed here — once, for every current and
    /// future caller — because most messages interpolate a process or
    /// container name, and those are attacker-controlled by any local user
    /// able to spawn a process.
    pub fn push(&mut self, level: Level, text: impl Into<String>) {
        let toast = Toast {
            level,
            text: scrub_ctrl(&text.into()).into_owned(),
            at: Instant::now(),
        };
        self.log.push_back(toast.clone());
        while self.log.len() > LOG_CAPACITY {
            self.log.pop_front();
        }
        self.active.push(toast);
        while self.active.len() > MAX_ACTIVE {
            self.active.remove(0);
        }
    }

    pub fn info(&mut self, text: impl Into<String>) {
        self.push(Level::Info, text);
    }

    pub fn success(&mut self, text: impl Into<String>) {
        self.push(Level::Success, text);
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.push(Level::Warning, text);
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Level::Error, text);
    }

    /// Toasts currently on screen, oldest first.
    pub fn active(&self) -> &[Toast] {
        &self.active
    }

    /// The newest toast, which is what a one-line status bar shows.
    pub fn latest(&self) -> Option<&Toast> {
        self.active.last()
    }

    /// Everything posted this session, newest last.
    pub fn history(&self) -> &VecDeque<Toast> {
        &self.log
    }

    /// Drop expired toasts.
    ///
    /// Returns whether anything was dropped, so the event-driven render loop
    /// can schedule exactly one repaint when a toast disappears instead of
    /// polling at 60 Hz to notice the transition.
    pub fn expire(&mut self) -> bool {
        let before = self.active.len();
        self.active.retain(|t| !t.is_expired());
        before != self.active.len()
    }

    /// Time until the next toast expires, for the event loop's poll timeout.
    /// `None` when nothing is pending.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.active
            .iter()
            .map(|t| t.ttl().saturating_sub(t.at.elapsed()))
            .min()
    }

    /// Dismiss every visible toast (`Esc`). The log keeps them.
    pub fn dismiss_all(&mut self) {
        self.active.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Age every visible toast by `by`, so tests can reach expiry without
    /// sleeping. Test-only: nothing in the running application may rewrite
    /// when a message was posted.
    #[cfg(test)]
    pub fn backdate_for_test(&mut self, by: Duration) {
        for t in &mut self.active {
            t.at -= by;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_makes_a_toast_visible() {
        let mut n = Notifier::new();
        n.success("Container nginx stopped");
        assert_eq!(n.active().len(), 1);
        assert_eq!(n.latest().unwrap().level, Level::Success);
        assert_eq!(n.latest().unwrap().text, "Container nginx stopped");
    }

    #[test]
    fn severity_is_carried_not_guessed() {
        // Regression: 0.4 inferred severity from the substring "failed", so a
        // reworded error rendered green.
        let mut n = Notifier::new();
        n.error("Could not reach the daemon");
        assert_eq!(n.latest().unwrap().level, Level::Error);
        n.success("failed-over to the replica");
        assert_eq!(
            n.latest().unwrap().level,
            Level::Success,
            "wording must not override the declared severity"
        );
    }

    #[test]
    fn control_characters_are_scrubbed() {
        let mut n = Notifier::new();
        n.info("evil\x1b[31mprocess\x07");
        let text = &n.latest().unwrap().text;
        assert!(!text.contains('\x1b'), "escape sequence survived: {text:?}");
        assert!(!text.contains('\x07'));
    }

    #[test]
    fn stack_is_capped_but_the_log_is_not() {
        let mut n = Notifier::new();
        for i in 0..10 {
            n.info(format!("message {i}"));
        }
        assert_eq!(n.active().len(), MAX_ACTIVE);
        assert_eq!(n.history().len(), 10);
        // The newest survives on screen.
        assert_eq!(n.latest().unwrap().text, "message 9");
    }

    #[test]
    fn log_is_capped_at_capacity() {
        let mut n = Notifier::new();
        for i in 0..(LOG_CAPACITY + 20) {
            n.info(format!("m{i}"));
        }
        assert_eq!(n.history().len(), LOG_CAPACITY);
        assert_eq!(
            n.history().back().unwrap().text,
            format!("m{}", LOG_CAPACITY + 19)
        );
    }

    #[test]
    fn expire_drops_old_toasts_and_reports_the_change() {
        let mut n = Notifier::new();
        n.info("stale");
        // Backdate past the info TTL.
        n.active[0].at = Instant::now() - TTL_INFO - Duration::from_secs(1);
        assert!(n.expire(), "expiring a toast must report a change");
        assert!(n.is_empty());
        assert!(!n.expire(), "a second call must report no change");
    }

    #[test]
    fn errors_outlive_info_messages() {
        let mut n = Notifier::new();
        n.info("info");
        n.error("error");
        let age = TTL_INFO + Duration::from_secs(1);
        for t in &mut n.active {
            t.at = Instant::now() - age;
        }
        n.expire();
        assert_eq!(n.active().len(), 1);
        assert_eq!(n.active()[0].level, Level::Error);
    }

    #[test]
    fn dismiss_clears_the_screen_but_keeps_the_log() {
        let mut n = Notifier::new();
        n.error("boom");
        n.dismiss_all();
        assert!(n.is_empty());
        assert_eq!(n.history().len(), 1, "Ctrl+L must still find it");
    }

    #[test]
    fn next_deadline_is_none_when_idle() {
        let mut n = Notifier::new();
        assert_eq!(n.next_deadline(), None);
        n.info("x");
        assert!(n.next_deadline().is_some_and(|d| d <= TTL_INFO));
    }
}
