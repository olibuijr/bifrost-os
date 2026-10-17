//! App run states kept apart (task 008, step 1): focus, on-screen visibility, occlusion and suspension. Pure model with
//! no clock or process of its own. The shell reports window events with a millisecond clock, and the app asks `tick`
//! what to do. An app that has focus, or that the user can see, keeps running, including visible unfocused windows.
//! An app that nothing shows is suspended only after `GRACE_MS`, so a window that is covered for a moment never
//! suspends. Suspension and resume are reported once per change.

/// Milliseconds an app may stay unseen before it is suspended.
pub const GRACE_MS: u64 = 2_000;

/// A report from the shell about one window-level fact of the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Focus(bool),
    /// At least one window of the app is shown (not minimized, not on another workspace).
    OnScreen(bool),
    /// Every shown window of the app is covered by windows of other apps.
    Covered(bool),
    /// The app holds a process assertion (for example audio playing or a running download). A held assertion keeps the
    /// app running even when nothing shows it, so the grace period never starts while it is held.
    Assertion(bool),
}

/// The app's run state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Run {
    Running,
    /// Unseen and unfocused, waiting out the grace period.
    Hidden,
    Suspended,
}

/// What the shell must do after a report or a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    None,
    Suspend,
    Resume,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occlusion {
    focused: bool,
    on_screen: bool,
    covered: bool,
    /// A process assertion is held (see `Event::Assertion`).
    asserted: bool,
    /// When the app first became unseen and unfocused; `None` while it is seen or focused.
    hidden_since: Option<u64>,
    suspended: bool,
}

impl Default for Occlusion {
    fn default() -> Self {
        Self::new()
    }
}

impl Occlusion {
    /// A new app is on screen, not covered and not focused.
    pub fn new() -> Self {
        Occlusion { focused: false, on_screen: true, covered: false, asserted: false, hidden_since: None, suspended: false }
    }

    /// Focus, a held process assertion, or a visible, uncovered window keeps the app running.
    pub fn wants_running(&self) -> bool {
        self.focused || self.asserted || (self.on_screen && !self.covered)
    }

    /// Whether the app holds a process assertion.
    pub fn is_asserted(&self) -> bool {
        self.asserted
    }

    pub fn run(&self) -> Run {
        if self.suspended {
            Run::Suspended
        } else if self.wants_running() {
            Run::Running
        } else {
            Run::Hidden
        }
    }

    /// Applies a shell report at `now_ms` and returns what to do.
    pub fn report(&mut self, event: Event, now_ms: u64) -> Transition {
        match event {
            Event::Focus(on) => self.focused = on,
            Event::OnScreen(on) => self.on_screen = on,
            Event::Covered(on) => self.covered = on,
            Event::Assertion(on) => self.asserted = on,
        }
        self.tick(now_ms)
    }

    /// Checks the grace period at `now_ms`. Returns `Suspend` once the app has been unseen for `GRACE_MS`, and
    /// `Resume` as soon as it is seen or focused again.
    pub fn tick(&mut self, now_ms: u64) -> Transition {
        if self.wants_running() {
            self.hidden_since = None;
            if self.suspended {
                self.suspended = false;
                return Transition::Resume;
            }
            return Transition::None;
        }
        let since = *self.hidden_since.get_or_insert(now_ms);
        if !self.suspended && now_ms.saturating_sub(since) >= GRACE_MS {
            self.suspended = true;
            return Transition::Suspend;
        }
        Transition::None
    }
}

/// Whether the window `[x, y, width, height]` is entirely covered by the windows in `above` (the windows of other apps
/// stacked over it). A window with no area is covered: nothing of it can be seen. Each window that lies over the
/// visible part cuts it into up to four pieces, and the window is covered when no piece is left.
pub fn is_covered(window: [i64; 4], above: &[[i64; 4]]) -> bool {
    let [_, _, w, h] = window;
    if w <= 0 || h <= 0 {
        return true;
    }
    let mut visible = vec![window];
    for &occluder in above {
        visible = visible.into_iter().flat_map(|piece| cut(piece, occluder)).collect();
        if visible.is_empty() {
            return true;
        }
    }
    visible.is_empty()
}

/// The parts of `piece` that `over` does not cover: the bands above, below, left and right of the overlap.
fn cut(piece: [i64; 4], over: [i64; 4]) -> Vec<[i64; 4]> {
    let [px, py, pw, ph] = piece;
    let [ox, oy, ow, oh] = over;
    let (px1, py1) = (px + pw, py + ph);
    let (ox1, oy1) = (ox + ow, oy + oh);
    let (ix0, iy0, ix1, iy1) = (px.max(ox), py.max(oy), px1.min(ox1), py1.min(oy1));
    if ix1 <= ix0 || iy1 <= iy0 {
        return vec![piece];
    }
    let mut parts = Vec::new();
    if iy0 > py {
        parts.push([px, py, pw, iy0 - py]);
    }
    if iy1 < py1 {
        parts.push([px, iy1, pw, py1 - iy1]);
    }
    if ix0 > px {
        parts.push([px, iy0, ix0 - px, iy1 - iy0]);
    }
    if ix1 < px1 {
        parts.push([ix1, iy0, px1 - ix1, iy1 - iy0]);
    }
    parts
}

/// The signal that stops an app's processes when it is suspended (`SIGSTOP`, 19 on Linux for x86_64 and aarch64).
pub const SIGSTOP: i32 = 19;
/// The signal that resumes an app's processes (`SIGCONT`, 18 on Linux for x86_64 and aarch64).
pub const SIGCONT: i32 = 18;

/// The signal the shell sends an app's process group for a transition, or `None` when nothing is sent. The shell
/// sends it only when the Settings toggle that allows suspension is on.
pub fn signal_for(transition: Transition) -> Option<i32> {
    match transition {
        Transition::None => None,
        Transition::Suspend => Some(SIGSTOP),
        Transition::Resume => Some(SIGCONT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suspension_stops_the_app_and_a_resume_continues_it() {
        assert_eq!(signal_for(Transition::Suspend), Some(SIGSTOP));
        assert_eq!(signal_for(Transition::Resume), Some(SIGCONT));
        assert_eq!(signal_for(Transition::None), None);
        assert_eq!((SIGSTOP, SIGCONT), (19, 18));
    }

    #[test]
    fn a_window_under_one_larger_window_is_covered() {
        assert!(is_covered([10, 10, 100, 50], &[[0, 0, 500, 500]]));
        assert!(is_covered([10, 10, 100, 50], &[[10, 10, 100, 50]]), "the same rectangle");
    }

    #[test]
    fn a_window_partly_showing_is_not_covered() {
        assert!(!is_covered([0, 0, 100, 100], &[[50, 0, 100, 100]]));
        assert!(!is_covered([0, 0, 100, 100], &[]), "nothing over it");
        assert!(!is_covered([0, 0, 100, 100], &[[200, 200, 10, 10]]), "an occluder that misses it");
    }

    #[test]
    fn two_windows_together_can_cover_one() {
        let left = [0, 0, 50, 100];
        let right = [50, 0, 50, 100];
        assert!(is_covered([10, 10, 80, 80], &[left, right]));
        // a gap of one pixel at the seam leaves the window showing
        assert!(!is_covered([10, 10, 80, 80], &[[0, 0, 49, 100], [50, 0, 50, 100]]));
    }

    #[test]
    fn a_window_with_no_area_is_covered() {
        assert!(is_covered([5, 5, 0, 10], &[]));
    }

    #[test]
    fn a_covering_fact_suspends_an_app_after_the_grace_period() {
        let mut o = Occlusion::new();
        let covered = is_covered([0, 0, 100, 100], &[[-10, -10, 200, 200]]);
        assert_eq!(o.report(Event::Covered(covered), 0), Transition::None);
        assert_eq!(o.tick(GRACE_MS), Transition::Suspend);
    }

    #[test]
    fn visible_unfocused_app_keeps_running() {
        let mut o = Occlusion::new();
        assert_eq!(o.report(Event::Focus(false), 0), Transition::None);
        assert_eq!(o.tick(10 * GRACE_MS), Transition::None);
        assert_eq!(o.run(), Run::Running);
    }

    #[test]
    fn covered_app_is_suspended_only_after_the_grace_period() {
        let mut o = Occlusion::new();
        assert_eq!(o.report(Event::Covered(true), 100), Transition::None);
        assert_eq!(o.tick(100 + GRACE_MS - 1), Transition::None);
        assert_eq!(o.run(), Run::Hidden);
        assert_eq!(o.tick(100 + GRACE_MS), Transition::Suspend);
        assert_eq!(o.run(), Run::Suspended);
        assert_eq!(o.tick(100 + 5 * GRACE_MS), Transition::None, "a suspension is reported once");
    }

    #[test]
    fn briefly_covered_app_never_suspends() {
        let mut o = Occlusion::new();
        o.report(Event::Covered(true), 0);
        assert_eq!(o.report(Event::Covered(false), GRACE_MS / 2), Transition::None);
        assert_eq!(o.tick(10 * GRACE_MS), Transition::None);
        assert_eq!(o.run(), Run::Running);
    }

    #[test]
    fn focus_resumes_a_suspended_app_at_once() {
        let mut o = Occlusion::new();
        o.report(Event::OnScreen(false), 0);
        assert_eq!(o.tick(GRACE_MS), Transition::Suspend);
        assert_eq!(o.report(Event::Focus(true), 5 * GRACE_MS), Transition::Resume);
        assert_eq!(o.run(), Run::Running);
        // Focus lost while still unseen starts a new grace period from that moment.
        assert_eq!(o.report(Event::Focus(false), 6 * GRACE_MS), Transition::None);
        assert_eq!(o.tick(7 * GRACE_MS - 1), Transition::None);
        assert_eq!(o.tick(7 * GRACE_MS), Transition::Suspend);
    }

    #[test]
    fn minimized_app_is_suspended_and_resumes_when_shown_again() {
        let mut o = Occlusion::new();
        assert_eq!(o.report(Event::OnScreen(false), 0), Transition::None);
        assert_eq!(o.tick(GRACE_MS), Transition::Suspend);
        assert_eq!(o.report(Event::OnScreen(true), GRACE_MS + 1), Transition::Resume);
        assert_eq!(o.run(), Run::Running);
    }

    #[test]
    fn a_held_assertion_keeps_an_unseen_app_running() {
        let mut o = Occlusion::new();
        assert_eq!(o.report(Event::Assertion(true), 0), Transition::None);
        assert!(o.is_asserted());
        o.report(Event::OnScreen(false), 1);
        assert_eq!(o.tick(10 * GRACE_MS), Transition::None, "audio playing is not stopped");
        assert_eq!(o.run(), Run::Running);
    }

    #[test]
    fn releasing_an_assertion_starts_the_grace_period_from_that_moment() {
        let mut o = Occlusion::new();
        o.report(Event::Assertion(true), 0);
        o.report(Event::OnScreen(false), 0);
        assert_eq!(o.report(Event::Assertion(false), 5 * GRACE_MS), Transition::None);
        assert_eq!(o.tick(6 * GRACE_MS - 1), Transition::None);
        assert_eq!(o.tick(6 * GRACE_MS), Transition::Suspend);
    }

    #[test]
    fn taking_an_assertion_resumes_a_suspended_app() {
        let mut o = Occlusion::new();
        o.report(Event::OnScreen(false), 0);
        assert_eq!(o.tick(GRACE_MS), Transition::Suspend);
        assert_eq!(o.report(Event::Assertion(true), 3 * GRACE_MS), Transition::Resume);
        assert_eq!(o.run(), Run::Running);
    }

    #[test]
    fn focus_alone_keeps_a_covered_app_running() {
        let mut o = Occlusion::new();
        o.report(Event::Covered(true), 0);
        o.report(Event::Focus(true), 1);
        assert_eq!(o.tick(100 * GRACE_MS), Transition::None);
        assert_eq!(o.run(), Run::Running);
    }
}
