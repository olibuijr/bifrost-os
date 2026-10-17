//! Slow Keys: a key counts only when it is held for the chosen acceptance delay.
//!
//! Pure state. The shell's input loop passes every key event of a frame through `filter`, which drops presses
//! that are not yet accepted and returns the releases that should still reach the key path. `due` then yields the
//! presses whose hold has reached the delay, and the caller queues them as key-down events. A press released
//! before the delay never types anything, which is what stops a brushed key from firing.

/// Acceptance delays offered in settings, as (label, milliseconds). The first entry is Off.
pub const DELAYS_MS: [(&str, u64); 4] = [("Off", 0), ("0.3 s", 300), ("0.5 s", 500), ("1 s", 1000)];

/// Hardware buttons are never slowed: the power and volume keys answer at once.
fn exempt(code: u16) -> bool {
    matches!(code, 114 | 115 | 116 | 143)
}

/// One key the user is holding down, not yet accepted.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pending {
    code: u16,
    since: f64,
}

/// The slow-keys state: the acceptance delay and the presses still being held.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SlowKeys {
    delay_ms: u64,
    pending: Vec<Pending>,
    /// keys whose press was accepted and whose release must reach the key path
    accepted: Vec<u16>,
}

/// One key event as the input loop sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Let the event through unchanged.
    Pass,
    /// Swallow it: the press is waiting for its delay, or the release belongs to a press that never counted.
    Drop,
}

impl SlowKeys {
    /// A model with the given acceptance delay; zero turns Slow Keys off.
    pub fn new(delay_ms: u64) -> Self {
        Self { delay_ms, pending: Vec::new(), accepted: Vec::new() }
    }

    pub fn enabled(&self) -> bool {
        self.delay_ms > 0
    }

    pub fn delay_ms(&self) -> u64 {
        self.delay_ms
    }

    /// Changes the delay. Holds in progress are dropped, so a new delay starts from a clean press.
    pub fn set_delay_ms(&mut self, delay_ms: u64) {
        self.delay_ms = delay_ms;
        self.pending.clear();
        self.accepted.clear();
    }

    /// A key went down at `t` (milliseconds). The press waits until `due` accepts it. A repeat of an accepted key passes.
    pub fn press(&mut self, code: u16, t: f64) -> Verdict {
        if !self.enabled() || exempt(code) || self.accepted.contains(&code) {
            return Verdict::Pass;
        }
        if !self.pending.iter().any(|p| p.code == code) {
            self.pending.push(Pending { code, since: t });
        }
        Verdict::Drop
    }

    /// A key went up. A release of an accepted press passes; a release of a press still waiting drops it.
    pub fn release(&mut self, code: u16) -> Verdict {
        if !self.enabled() || exempt(code) {
            return Verdict::Pass;
        }
        if let Some(i) = self.accepted.iter().position(|&c| c == code) {
            self.accepted.swap_remove(i);
            return Verdict::Pass;
        }
        self.pending.retain(|p| p.code != code);
        Verdict::Drop
    }

    /// The codes whose hold has reached the delay at `t`, oldest first. Each is accepted once; the caller queues a
    /// key-down for it, and its release then passes through `release`.
    pub fn due(&mut self, t: f64) -> Vec<u16> {
        if !self.enabled() {
            return Vec::new();
        }
        let delay = self.delay_ms as f64;
        let mut out = Vec::new();
        self.pending.retain(|p| {
            if t - p.since >= delay {
                out.push(p.code);
                false
            } else {
                true
            }
        });
        self.accepted.extend(out.iter().copied());
        out
    }

    /// The time the next pending press is accepted, for the loop's wake-up.
    pub fn deadline(&self) -> Option<f64> {
        if !self.enabled() {
            return None;
        }
        self.pending.iter().map(|p| p.since + self.delay_ms as f64).reduce(f64::min)
    }
}

/// Acceptance delay for a settings index, clamped to the last choice.
pub fn delay_for_index(index: usize) -> u64 {
    DELAYS_MS[index.min(DELAYS_MS.len() - 1)].1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_passes_everything() {
        let mut s = SlowKeys::new(0);
        assert_eq!(s.press(30, 0.0), Verdict::Pass);
        assert_eq!(s.release(30), Verdict::Pass);
        assert!(s.due(10_000.0).is_empty());
    }

    #[test]
    fn a_held_key_is_accepted_after_the_delay_and_its_release_passes() {
        let mut s = SlowKeys::new(500);
        assert_eq!(s.press(30, 0.0), Verdict::Drop);
        assert!(s.due(499.0).is_empty());
        assert_eq!(s.due(500.0), vec![30]);
        assert!(s.due(900.0).is_empty(), "accepted once");
        assert_eq!(s.release(30), Verdict::Pass);
    }

    #[test]
    fn a_brushed_key_released_early_never_types() {
        let mut s = SlowKeys::new(500);
        s.press(30, 0.0);
        assert_eq!(s.release(30), Verdict::Drop);
        assert!(s.due(1000.0).is_empty());
        assert_eq!(s.release(30), Verdict::Drop, "a stray release stays dropped");
    }

    #[test]
    fn exempt_hardware_keys_pass_at_once() {
        let mut s = SlowKeys::new(500);
        assert_eq!(s.press(116, 0.0), Verdict::Pass);
        assert_eq!(s.release(116), Verdict::Pass);
    }

    #[test]
    fn deadline_is_the_earliest_pending_press() {
        let mut s = SlowKeys::new(300);
        assert_eq!(s.deadline(), None);
        s.press(30, 100.0);
        s.press(31, 200.0);
        assert_eq!(s.deadline(), Some(400.0));
    }

    #[test]
    fn changing_the_delay_drops_holds_in_progress() {
        let mut s = SlowKeys::new(300);
        s.press(30, 0.0);
        s.set_delay_ms(1000);
        assert!(s.due(2000.0).is_empty());
    }
}
