//! Display management as pure state: the criteria a playing app asks the display for (refresh rate and dynamic range), and
//! the choice of a display mode that meets them. The shell or the video path supplies the modes the panel reports and
//! performs the switch; this file decides which mode to use and tracks whether a switch is under way. Nothing is switched
//! here.

/// Rates closer than this, in hertz, count as the same rate. This keeps 23.976 Hz content on a 24 Hz mode.
pub const RATE_TOLERANCE_HZ: f32 = 0.05;

/// The dynamic range of the picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynamicRange {
    Sdr,
    Hdr,
}

/// A display mode the panel reports as supported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    pub refresh_rate: f32,
    pub dynamic_range: DynamicRange,
}

/// What the app asks for while its content plays: the frame rate of the content and its dynamic range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Criteria {
    pub refresh_rate: f32,
    pub dynamic_range: DynamicRange,
}

/// The answer to a request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// The current mode already meets the criteria. Nothing changes.
    Keep,
    /// The display must switch to this mode. Call `finish_switch` once the panel reports the new mode.
    Switch(Mode),
    /// No supported mode has the requested dynamic range, or the requested rate is not a usable number.
    Unsupported,
}

/// The display state of one app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DisplayManager {
    current: Option<Mode>,
    criteria: Option<Criteria>,
    pending: Option<Mode>,
}

impl DisplayManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The mode the panel is in, once known.
    pub fn current_mode(&self) -> Option<Mode> {
        self.current
    }

    /// The criteria the app last asked for, if it still holds them.
    pub fn criteria(&self) -> Option<Criteria> {
        self.criteria
    }

    /// True between a `Decision::Switch` and the matching `finish_switch`.
    pub fn is_switch_in_progress(&self) -> bool {
        self.pending.is_some()
    }

    /// Asks for a mode that meets `criteria`, given the modes the panel reports. The best mode has the requested dynamic
    /// range and the rate closest to the content rate; on a tie the higher rate wins, so content is never slowed down.
    /// A new request replaces a switch that is still pending.
    pub fn request(&mut self, criteria: Criteria, modes: &[Mode]) -> Decision {
        if !criteria.refresh_rate.is_finite() || criteria.refresh_rate <= 0.0 {
            return Decision::Unsupported;
        }
        self.criteria = Some(criteria);
        let Some(best) = best_mode(criteria, modes) else {
            return Decision::Unsupported;
        };
        if let Some(current) = self.current {
            if same_mode(current, best) {
                self.pending = None;
                return Decision::Keep;
            }
        }
        self.pending = Some(best);
        Decision::Switch(best)
    }

    /// Records that the panel now runs the mode that was requested. Returns the mode, or `None` when no switch was pending.
    pub fn finish_switch(&mut self) -> Option<Mode> {
        let mode = self.pending.take()?;
        self.current = Some(mode);
        Some(mode)
    }

    /// Drops the criteria when the app stops playing. The panel keeps its mode until a later request changes it.
    pub fn clear_criteria(&mut self) {
        self.criteria = None;
    }
}

/// True when two modes are the same dynamic range at the same rate, within the tolerance.
fn same_mode(a: Mode, b: Mode) -> bool {
    a.dynamic_range == b.dynamic_range && (a.refresh_rate - b.refresh_rate).abs() <= RATE_TOLERANCE_HZ
}

/// The supported mode that best meets `criteria`, or `None` when none has the requested dynamic range.
fn best_mode(criteria: Criteria, modes: &[Mode]) -> Option<Mode> {
    let mut best: Option<Mode> = None;
    for &mode in modes {
        if mode.dynamic_range != criteria.dynamic_range || !mode.refresh_rate.is_finite() || mode.refresh_rate <= 0.0 {
            continue;
        }
        best = Some(match best {
            None => mode,
            Some(prev) => {
                let gap = (mode.refresh_rate - criteria.refresh_rate).abs();
                let prev_gap = (prev.refresh_rate - criteria.refresh_rate).abs();
                let same_gap = (gap - prev_gap).abs() <= RATE_TOLERANCE_HZ;
                if gap < prev_gap && !same_gap || same_gap && mode.refresh_rate > prev.refresh_rate {
                    mode
                } else {
                    prev
                }
            }
        });
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sdr(rate: f32) -> Mode {
        Mode { refresh_rate: rate, dynamic_range: DynamicRange::Sdr }
    }

    fn hdr(rate: f32) -> Mode {
        Mode { refresh_rate: rate, dynamic_range: DynamicRange::Hdr }
    }

    fn want(rate: f32, dynamic_range: DynamicRange) -> Criteria {
        Criteria { refresh_rate: rate, dynamic_range }
    }

    #[test]
    fn a_request_with_no_usable_rate_is_unsupported() {
        let mut m = DisplayManager::new();
        let modes = [sdr(60.0)];
        assert_eq!(m.request(want(0.0, DynamicRange::Sdr), &modes), Decision::Unsupported);
        assert_eq!(m.request(want(f32::NAN, DynamicRange::Sdr), &modes), Decision::Unsupported);
        assert!(m.criteria().is_none());
    }

    #[test]
    fn no_mode_with_the_requested_dynamic_range_is_unsupported() {
        let mut m = DisplayManager::new();
        assert_eq!(m.request(want(24.0, DynamicRange::Hdr), &[sdr(60.0)]), Decision::Unsupported);
        assert!(!m.is_switch_in_progress());
    }

    #[test]
    fn the_mode_closest_to_the_content_rate_is_chosen() {
        let mut m = DisplayManager::new();
        let modes = [sdr(60.0), sdr(48.0), sdr(120.0)];
        assert_eq!(m.request(want(24.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(48.0)));
    }

    #[test]
    fn a_tie_goes_to_the_higher_rate() {
        let mut m = DisplayManager::new();
        let modes = [sdr(50.0), sdr(70.0)];
        assert_eq!(m.request(want(60.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(70.0)));
    }

    #[test]
    fn a_film_rate_is_satisfied_by_the_matching_mode_without_switching_back() {
        let mut m = DisplayManager::new();
        let modes = [sdr(24.0), sdr(60.0)];
        assert_eq!(m.request(want(24.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(24.0)));
        m.finish_switch();
        assert_eq!(m.request(want(24.0, DynamicRange::Sdr), &modes), Decision::Keep);
        assert!(!m.is_switch_in_progress());
    }

    #[test]
    fn a_switch_is_pending_until_it_finishes() {
        let mut m = DisplayManager::new();
        let modes = [sdr(60.0)];
        assert!(m.finish_switch().is_none());
        assert_eq!(m.request(want(60.0, DynamicRange::Sdr), &modes), Decision::Switch(sdr(60.0)));
        assert!(m.is_switch_in_progress());
        assert_eq!(m.current_mode(), None);
        assert_eq!(m.finish_switch(), Some(sdr(60.0)));
        assert!(!m.is_switch_in_progress());
        assert_eq!(m.current_mode(), Some(sdr(60.0)));
    }

    #[test]
    fn a_dynamic_range_change_needs_a_switch_even_at_the_same_rate() {
        let mut m = DisplayManager::new();
        let modes = [sdr(60.0), hdr(60.0)];
        m.request(want(60.0, DynamicRange::Sdr), &modes);
        m.finish_switch();
        assert_eq!(m.request(want(60.0, DynamicRange::Hdr), &modes), Decision::Switch(hdr(60.0)));
    }

    #[test]
    fn clearing_the_criteria_keeps_the_current_mode() {
        let mut m = DisplayManager::new();
        let modes = [sdr(48.0)];
        m.request(want(24.0, DynamicRange::Sdr), &modes);
        m.finish_switch();
        m.clear_criteria();
        assert!(m.criteria().is_none());
        assert_eq!(m.current_mode(), Some(sdr(48.0)));
    }
}
