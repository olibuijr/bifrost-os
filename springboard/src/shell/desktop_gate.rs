//! The start gate for work that is not needed for the first desktop frame: the login items and the windows restored
//! from the last session. The shell builds its first frame first; these open once that frame is committed, so a slow
//! app start never delays the desktop. A ceiling opens the gate anyway when the frame is never reported (the bare
//! display path has no Wayland commit to report it), so the login items still start.

/// How long after the shell's first tick the gate opens even without a presented frame, in milliseconds.
pub const CEILING_MS: f64 = 6000.0;

/// Waits for the first desktop frame, with a ceiling measured from the first tick it is asked on.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct DesktopGate {
    since: Option<f64>,
    open: bool,
}

impl DesktopGate {
    /// True once `presented` (the first desktop frame is on screen) or the ceiling has passed since the first call.
    /// `t` is the shell's clock in milliseconds; it only ever moves forward.
    pub fn is_open(&mut self, t: f64, presented: bool) -> bool {
        if self.open {
            return true;
        }
        let since = *self.since.get_or_insert(t);
        self.open = presented || t - since >= CEILING_MS;
        self.open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stays_closed_until_the_first_frame_is_presented() {
        let mut g = DesktopGate::default();
        assert!(!g.is_open(1000.0, false));
        assert!(!g.is_open(1500.0, false));
        assert!(g.is_open(1600.0, true));
    }

    #[test]
    fn opens_at_the_ceiling_when_no_frame_is_reported() {
        let mut g = DesktopGate::default();
        assert!(!g.is_open(500.0, false));
        assert!(!g.is_open(500.0 + CEILING_MS - 1.0, false));
        assert!(g.is_open(500.0 + CEILING_MS, false));
    }

    #[test]
    fn the_ceiling_counts_from_the_first_call_not_from_zero() {
        let mut g = DesktopGate::default();
        // the first call fixes the origin: a late first tick does not open the gate at once
        assert!(!g.is_open(90_000.0, false));
        assert!(!g.is_open(90_001.0, false));
    }

    #[test]
    fn once_open_it_stays_open() {
        let mut g = DesktopGate::default();
        assert!(g.is_open(0.0, true));
        assert!(g.is_open(1.0, false));
    }
}
