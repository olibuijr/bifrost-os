//! The thermal warning: a notification when the computer gets too hot, once per rise.
//!
//! `bifrost_core::thermal` reads the kernel's thermal zones and rates them Nominal, Fair, Serious or Critical (the
//! `power` service hands the same state to apps). This module watches that state from the shell's tick and posts one
//! warning when it climbs to Serious and one more when it climbs to Critical. It speaks again only after the machine
//! has cooled to Fair or below, so a reading that wobbles around a limit never repeats the warning. The kernel keeps
//! its own protection (throttling and the emergency shutdown); this only tells the person while there is time to
//! save work and close what is running.
//!
//! The decisions are pure (`Watch::update`, `Monitor::poll`), so they are tested without a thermal zone.

use super::notify::Notification;
use super::services::power_service::THERMAL_CLASS;
use super::{AppId, Shell};
use bifrost_core::thermal::{self, ThermalState};
use std::path::Path;

/// How often the thermal zones are read, in ms. A zone changes over seconds, and each read lists a directory.
pub const CHECK_MS: f64 = 5000.0;

/// What a warning says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    pub title: &'static str,
    pub body: &'static str,
}

/// The warning for a state: none below Serious.
pub fn warning_for(state: ThermalState) -> Option<Warning> {
    match state {
        ThermalState::Serious => Some(Warning {
            title: "Computer Is Running Hot",
            body: "Performance may be reduced. Close the apps you are not using and move to a cooler place.",
        }),
        ThermalState::Critical => Some(Warning {
            title: "Computer Is Too Hot",
            body: "Save your work now. The system may shut down by itself to protect the hardware.",
        }),
        ThermalState::Nominal | ThermalState::Fair => None,
    }
}

/// Which warning has been given since the machine was last cool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    given: ThermalState,
}

impl Default for Watch {
    fn default() -> Self {
        Watch { given: ThermalState::Nominal }
    }
}

impl Watch {
    /// The warning to give for a new reading, if any. A rise to a state worse than every warning given since the last
    /// cool-down speaks; the same state, or a fall that is still Serious, stays quiet; Fair and Nominal re-arm it.
    pub fn update(&mut self, now: ThermalState) -> Option<Warning> {
        if now < ThermalState::Serious {
            self.given = ThermalState::Nominal;
            return None;
        }
        if now > self.given {
            self.given = now;
            return warning_for(now);
        }
        None
    }
}

/// The watch with its reading schedule.
#[derive(Clone, Debug, Default)]
pub struct Monitor {
    watch: Watch,
    /// when the zones were last read (ms); None before the first read
    checked: Option<f64>,
}

impl Monitor {
    /// Reads the state with `read` when `CHECK_MS` has passed since the last read (the first call reads at once) and
    /// returns the warning that reading earns.
    pub fn poll(&mut self, t: f64, read: impl FnOnce() -> ThermalState) -> Option<Warning> {
        if self.checked.is_some_and(|at| t - at < CHECK_MS) {
            return None;
        }
        self.checked = Some(t);
        self.watch.update(read())
    }
}

impl Shell {
    /// Posts the thermal warning as a notification from Settings when the zones climb to Serious or Critical.
    pub(super) fn thermal_tick(&mut self, t: f64) {
        // tests never read the build machine's own sensors: a hot host would post the warning into every shell test
        let class = if cfg!(test) { Path::new("/nonexistent/thermal") } else { Path::new(THERMAL_CLASS) };
        let Some(w) = self.thermal.poll(t, || thermal::worst_in(class)) else { return };
        let now = self.notify.now();
        self.notify(Notification::new(AppId::Settings, w.title, w.body, now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ThermalState::*;

    #[test]
    fn a_cool_machine_never_warns() {
        let mut w = Watch::default();
        for s in [Nominal, Fair, Nominal, Fair] {
            assert_eq!(w.update(s), None);
        }
    }

    #[test]
    fn serious_then_critical_each_speak_once() {
        let mut w = Watch::default();
        assert_eq!(w.update(Serious), warning_for(Serious));
        assert_eq!(w.update(Serious), None, "the same state is not repeated");
        assert_eq!(w.update(Critical), warning_for(Critical));
        assert_eq!(w.update(Critical), None);
        assert_eq!(w.update(Serious), None, "a fall that is still Serious stays quiet");
    }

    #[test]
    fn cooling_to_fair_re_arms_the_warning() {
        let mut w = Watch::default();
        assert!(w.update(Serious).is_some());
        assert_eq!(w.update(Fair), None);
        assert_eq!(w.update(Serious), warning_for(Serious), "it speaks again after a cool-down");
    }

    #[test]
    fn a_jump_straight_to_critical_gives_the_critical_warning() {
        let mut w = Watch::default();
        let given = w.update(Critical).expect("a warning");
        assert_eq!(given.title, "Computer Is Too Hot");
    }

    #[test]
    fn the_zones_are_read_every_check_interval_and_the_first_call_reads_at_once() {
        let mut m = Monitor::default();
        let mut reads = 0;
        let mut read = |state| {
            reads += 1;
            state
        };
        assert!(m.poll(0.0, || read(Serious)).is_some(), "the first call reads");
        assert!(m.poll(CHECK_MS - 1.0, || read(Critical)).is_none(), "too soon: no read");
        assert!(m.poll(CHECK_MS, || read(Critical)).is_some(), "due: the rise to Critical speaks");
        assert_eq!(reads, 2);
    }

    #[test]
    fn the_warnings_name_no_other_product_and_fit_a_banner() {
        for state in [Serious, Critical] {
            let w = warning_for(state).unwrap();
            assert!(w.title.len() <= 40 && w.body.len() <= 120, "{w:?}");
        }
        assert!(warning_for(Nominal).is_none() && warning_for(Fair).is_none());
    }
}
