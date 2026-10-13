//! The shell loop's next wake-up: the earliest of the deadlines that need a `tick`.
//!
//! `Shell::next_wake` folds every pending deadline (animations, caret blink, timers, polls) into one value. A
//! deadline is a monotonic time in ms; `None` means nothing is pending. An empty fold is no wake-up at all, so an
//! idle desktop sleeps until input arrives.

/// The earliest deadline added so far.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Wake(Option<f64>, Option<&'static std::panic::Location<'static>>);

impl Wake {
    /// Adds a deadline; `None` (nothing pending) changes nothing.
    #[track_caller]
    pub fn add(&mut self, at: Option<f64>) {
        if let Some(at) = at {
            if self.0.is_none_or(|due| at < due) {
                self.1 = Some(std::panic::Location::caller());
            }
            self.0 = Some(self.0.map_or(at, |due| due.min(at)));
        }
    }

    /// The call site that supplied the earliest deadline (diagnostics: `SPRINGBOARD_WAKE_TRACE`).
    pub fn source(&self) -> Option<&'static std::panic::Location<'static>> {
        self.1
    }

    /// The fold so far, in monotonic ms.
    pub fn get(self) -> Option<f64> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::Wake;

    #[test]
    fn nothing_pending_means_no_wake_up() {
        let mut w = Wake::default();
        w.add(None);
        w.add(None);
        assert_eq!(w.get(), None, "an idle shell has no deadline");
    }

    #[test]
    fn the_earliest_deadline_wins_in_any_order() {
        let mut a = Wake::default();
        a.add(Some(900.0));
        a.add(Some(120.0));
        a.add(Some(500.0));
        let mut b = Wake::default();
        b.add(Some(120.0));
        b.add(Some(500.0));
        b.add(Some(900.0));
        assert_eq!(a.get(), Some(120.0));
        assert_eq!(b.get(), Some(120.0));
    }

    #[test]
    fn a_past_deadline_stays_past_so_the_loop_ticks_at_once() {
        let mut w = Wake::default();
        w.add(Some(1000.0));
        w.add(Some(400.0));
        assert_eq!(w.get(), Some(400.0), "a deadline already due is not pushed later");
    }

    #[test]
    fn none_after_a_deadline_keeps_it() {
        let mut w = Wake::default();
        w.add(Some(42.0));
        w.add(None);
        assert_eq!(w.get(), Some(42.0));
    }

    #[test]
    fn an_animation_tick_pulls_a_later_timer_forward() {
        // the shell adds `Some(t)` while an animation runs: the timer far in the future must not win
        let t = 10_000.0;
        let mut w = Wake::default();
        w.add(Some(t + 60_000.0));
        w.add(Some(t));
        assert_eq!(w.get(), Some(t));
    }

    #[test]
    fn a_wake_inside_the_tolerance_window_is_reused() {
        // desktop_metrics::coalesce: a read that falls within a planned wake-up rides on it (no extra wake)
        let mut w = Wake::default();
        w.add(Some(2000.0));
        let coalesced = crate::shell::desktop_metrics::coalesce(1990.0, 200.0, w.get());
        assert_eq!(coalesced, 2000.0);
        w.add(Some(coalesced));
        assert_eq!(w.get(), Some(2000.0), "no second wake-up was added");
    }
}
