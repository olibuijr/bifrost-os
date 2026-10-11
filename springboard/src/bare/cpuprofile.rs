//! CPU power profiles: which governor and clock window each cluster runs in, how hard the
//! interaction boost pushes, and when the shell picks a profile by itself.
//!
//! Everything here is pure; `boost` turns a `Rule` into sysfs writes. Frequencies are never
//! hard-coded: a rule gives a percentage of each policy's own cpuinfo range, so it follows
//! whatever the hardware reports.

/// Stock governor, restored on clean exits and by tools/boot.sh.
pub const STOCK_GOVERNOR: &str = "schedutil";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    Performance,
    #[default]
    Balanced,
    Battery,
}

impl Profile {
    pub fn label(self) -> &'static str {
        match self {
            Profile::Performance => "Performance",
            Profile::Balanced => "Balanced",
            Profile::Battery => "Battery Saver",
        }
    }

    pub fn rule(self) -> Rule {
        match self {
            // the clocks stay up, so there is nothing left for the boost to do
            Profile::Performance => {
                let c = Cluster { governor: "performance", max_pct: 100, boost_pct: 100 };
                Rule { little: c, big: c, boost_hold_ms: 0.0, frame_ms: 0.0, wifi_power_save: false }
            }
            // today's behaviour: stock governor, full range, boost to the top
            Profile::Balanced => {
                let c = Cluster { governor: STOCK_GOVERNOR, max_pct: 100, boost_pct: 100 };
                Rule { little: c, big: c, boost_hold_ms: 350.0, frame_ms: 0.0, wifi_power_save: true }
            }
            // the big cores cost the most energy per frame, so they are capped harder; the boost
            // still helps the first frames but only to the middle of the range. The frame cap
            // stays with Low Power Mode (low_battery.rs), which Auto turns into this profile.
            Profile::Battery => Rule {
                little: Cluster { governor: STOCK_GOVERNOR, max_pct: 70, boost_pct: 50 },
                big: Cluster { governor: STOCK_GOVERNOR, max_pct: 60, boost_pct: 50 },
                boost_hold_ms: 300.0,
                frame_ms: 0.0,
                wifi_power_save: true,
            },
        }
    }
}

/// One cluster's settings; the minimum while idle is always the hardware minimum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cluster {
    pub governor: &'static str,
    /// scaling_max_freq as a percentage of the cpuinfo range
    pub max_pct: u8,
    /// scaling_min_freq while boosted, same scale (never above `max_pct`)
    pub boost_pct: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    pub little: Cluster,
    pub big: Cluster,
    /// how long the boost holds after the last touch or animated frame; 0 = no boost
    pub boost_hold_ms: f64,
    /// minimum time between presented frames; 0 = paced by the display only
    pub frame_ms: f64,
    /// Wi-Fi power save (nl80211); off keeps latency lowest, on saves battery
    pub wifi_power_save: bool,
}

impl Rule {
    pub fn cluster(&self, big: bool) -> Cluster {
        if big {
            self.big
        } else {
            self.little
        }
    }
}

/// The frequency `pct` percent of the way from `min` to `max` kHz.
pub fn freq_at(min: u32, max: u32, pct: u8) -> u32 {
    min + ((max.saturating_sub(min)) as u64 * pct.min(100) as u64 / 100) as u32
}

/// The Settings › Battery › Power Mode choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pref {
    #[default]
    Auto,
    Performance,
    Balanced,
    Battery,
}

impl Pref {
    /// The value stored under `power_profile` in settings.ini.
    pub fn key(self) -> &'static str {
        match self {
            Pref::Auto => "auto",
            Pref::Performance => "performance",
            Pref::Balanced => "balanced",
            Pref::Battery => "battery",
        }
    }

    pub fn parse(s: &str) -> Option<Pref> {
        [Pref::Auto, Pref::Performance, Pref::Balanced, Pref::Battery].into_iter().find(|p| p.key() == s)
    }
}

/// Charging at or above this level runs flat out; below it the battery is too empty to spend on it.
pub const PERFORMANCE_FROM: u8 = 20;
/// Unplugged at or below this level the Battery profile takes over.
pub const BATTERY_AT: u8 = 30;
/// A decision sticks until the charge moves this far past its threshold.
pub const HYSTERESIS: u8 = 3;

/// The stateless Automatic rule. Low Power Mode always means Battery; charging at 20 % or more
/// means Performance (the charger pays for it); unplugged it is Balanced above 30 % and Battery
/// at or below. `Auto` adds the hysteresis.
pub fn auto_profile(battery_pct: u8, charging: bool, low_power_mode: bool) -> Profile {
    if low_power_mode {
        Profile::Battery
    } else if charging {
        if battery_pct >= PERFORMANCE_FROM {
            Profile::Performance
        } else {
            Profile::Balanced
        }
    } else if battery_pct <= BATTERY_AT {
        Profile::Battery
    } else {
        Profile::Balanced
    }
}

/// `auto_profile` with +/- `HYSTERESIS` so a level wobbling around a threshold does not flap.
#[derive(Debug, Default)]
pub struct Auto {
    current: Profile,
}

impl Auto {
    pub fn update(&mut self, battery_pct: u8, charging: bool, low_power_mode: bool) -> Profile {
        // leaving a state needs the charge to cross its threshold by the margin
        let pct = match (self.current, charging) {
            (Profile::Battery, false) => battery_pct.saturating_sub(HYSTERESIS),
            (Profile::Performance, true) => battery_pct.saturating_add(HYSTERESIS).min(100),
            _ => battery_pct,
        };
        self.current = auto_profile(pct, charging, low_power_mode);
        self.current
    }
}

/// The profile in effect for a preference: Low Power Mode forces Battery over every choice
/// (it is a deliberate saving mode), otherwise a fixed choice wins and Automatic follows `auto`.
pub fn resolve(pref: Pref, auto: &mut Auto, battery_pct: u8, charging: bool, low_power_mode: bool) -> Profile {
    let automatic = auto.update(battery_pct, charging, low_power_mode);
    match pref {
        _ if low_power_mode => Profile::Battery,
        Pref::Auto => automatic,
        Pref::Performance => Profile::Performance,
        Pref::Balanced => Profile::Balanced,
        Pref::Battery => Profile::Battery,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_follow_the_profiles() {
        assert_eq!(Profile::default(), Profile::Balanced);
        assert_eq!(Profile::Balanced.rule().boost_hold_ms, 350.0);
        assert_eq!(Profile::Performance.rule().boost_hold_ms, 0.0);
        assert_eq!(Profile::Performance.rule().big.governor, "performance");
        assert!(!Profile::Performance.rule().wifi_power_save && Profile::Battery.rule().wifi_power_save);
        let b = Profile::Battery.rule();
        assert!(b.big.max_pct < b.little.max_pct && b.little.max_pct < 100);
        for p in [Profile::Performance, Profile::Balanced, Profile::Battery] {
            let r = p.rule();
            assert!(r.little.boost_pct <= r.little.max_pct && r.big.boost_pct <= r.big.max_pct);
        }
    }

    #[test]
    fn frequencies_scale_within_the_hardware_range() {
        assert_eq!(freq_at(600_000, 2_000_000, 0), 600_000);
        assert_eq!(freq_at(600_000, 2_000_000, 100), 2_000_000);
        assert_eq!(freq_at(1_000_000, 2_000_000, 60), 1_600_000);
        assert_eq!(freq_at(2_000_000, 1_000_000, 50), 2_000_000, "an inverted range never underflows");
    }

    #[test]
    fn automatic_thresholds() {
        assert_eq!(auto_profile(80, false, false), Profile::Balanced);
        assert_eq!(auto_profile(31, false, false), Profile::Balanced);
        assert_eq!(auto_profile(30, false, false), Profile::Battery);
        assert_eq!(auto_profile(80, true, false), Profile::Performance);
        assert_eq!(auto_profile(19, true, false), Profile::Balanced);
        assert_eq!(auto_profile(90, true, true), Profile::Battery);
    }

    #[test]
    fn automatic_does_not_flap_around_a_threshold() {
        let mut a = Auto::default();
        assert_eq!(a.update(31, false, false), Profile::Balanced);
        assert_eq!(a.update(30, false, false), Profile::Battery);
        assert_eq!(a.update(32, false, false), Profile::Battery);
        assert_eq!(a.update(33, false, false), Profile::Battery);
        assert_eq!(a.update(34, false, false), Profile::Balanced);
        let mut a = Auto::default();
        assert_eq!(a.update(20, true, false), Profile::Performance);
        assert_eq!(a.update(18, true, false), Profile::Performance);
        assert_eq!(a.update(16, true, false), Profile::Balanced);
        assert_eq!(a.update(19, true, false), Profile::Balanced);
    }

    #[test]
    fn preferences_resolve_with_low_power_mode_on_top() {
        let mut a = Auto::default();
        assert_eq!(resolve(Pref::Performance, &mut a, 10, false, false), Profile::Performance);
        assert_eq!(resolve(Pref::Auto, &mut a, 10, false, false), Profile::Battery);
        assert_eq!(resolve(Pref::Balanced, &mut a, 10, false, false), Profile::Balanced);
        assert_eq!(resolve(Pref::Performance, &mut a, 90, true, true), Profile::Battery);
        assert_eq!(resolve(Pref::Auto, &mut a, 90, true, false), Profile::Performance);
    }

    #[test]
    fn preference_keys_round_trip() {
        for p in [Pref::Auto, Pref::Performance, Pref::Balanced, Pref::Battery] {
            assert_eq!(Pref::parse(p.key()), Some(p));
        }
        assert_eq!(Pref::parse("turbo"), None);
    }
}
