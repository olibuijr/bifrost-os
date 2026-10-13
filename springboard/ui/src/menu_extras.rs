//! System menu extras of the menu bar: the fixed set of system status items, their order from the left, the ids Settings
//! and apps use for them, default visibility, and the clock options. Pure data shared by the shell and the SDK (re-exported
//! as `bifrost_app::status_item::system`); drawing and hit-testing stay in the shell's `desktop_bar`.
//!
//! The set is a `u16` with one bit per [`SystemExtra`] (bit = discriminant), so Settings stores it like the other menu bar
//! prefs. App menu extras (`NativeStatusItem`) are not part of this set; they sit left of these.

/// One system menu extra. Discriminant order is the order from the left of the menu bar ([`SystemExtra::ALL`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SystemExtra {
    /// Search
    Search = 0,
    /// the signed-in user and fast account switching
    User = 1,
    /// backup status
    Backup = 2,
    /// the Now Playing session of a media app
    NowPlaying = 3,
    /// the active keyboard source (shown only with two or more sources)
    InputSource = 4,
    /// output volume
    Sound = 5,
    Bluetooth = 6,
    /// Wi-Fi, or ethernet when a wired link is up
    WiFi = 7,
    /// Focus (Do Not Disturb and modes)
    Focus = 8,
    Battery = 9,
    /// the Control Center sheet
    ControlCenter = 10,
    /// the date and time; its options are [`ClockStyle`]
    Clock = 11,
}

impl SystemExtra {
    /// Every extra, left to right.
    pub const ALL: [SystemExtra; 12] = [
        SystemExtra::Search,
        SystemExtra::User,
        SystemExtra::Backup,
        SystemExtra::NowPlaying,
        SystemExtra::InputSource,
        SystemExtra::Sound,
        SystemExtra::Bluetooth,
        SystemExtra::WiFi,
        SystemExtra::Focus,
        SystemExtra::Battery,
        SystemExtra::ControlCenter,
        SystemExtra::Clock,
    ];

    /// The set with every extra that is on by default: all but User and Backup, which need an account or a backup
    /// destination before they have anything to show.
    pub const DEFAULT_BITS: u16 = Self::bits_of(&[
        SystemExtra::Search,
        SystemExtra::NowPlaying,
        SystemExtra::InputSource,
        SystemExtra::Sound,
        SystemExtra::Bluetooth,
        SystemExtra::WiFi,
        SystemExtra::Focus,
        SystemExtra::Battery,
        SystemExtra::ControlCenter,
        SystemExtra::Clock,
    ]);

    /// The bit of this extra in an extras set.
    pub const fn bit(self) -> u16 {
        1 << self as u16
    }

    /// The set made of `extras`.
    pub const fn bits_of(extras: &[SystemExtra]) -> u16 {
        let mut bits = 0u16;
        let mut i = 0;
        while i < extras.len() {
            bits |= extras[i].bit();
            i += 1;
        }
        bits
    }

    /// Stable identifier used in prefs and by apps (neutral names, no product names).
    pub const fn id(self) -> &'static str {
        match self {
            SystemExtra::Search => "search",
            SystemExtra::User => "user",
            SystemExtra::Backup => "backup",
            SystemExtra::NowPlaying => "now-playing",
            SystemExtra::InputSource => "input-source",
            SystemExtra::Sound => "sound",
            SystemExtra::Bluetooth => "bluetooth",
            SystemExtra::WiFi => "wifi",
            SystemExtra::Focus => "focus",
            SystemExtra::Battery => "battery",
            SystemExtra::ControlCenter => "control-center",
            SystemExtra::Clock => "clock",
        }
    }

    /// The extra with this [`id`](Self::id), if any.
    pub fn from_id(id: &str) -> Option<SystemExtra> {
        SystemExtra::ALL.iter().copied().find(|e| e.id() == id)
    }

    /// The row title in Settings > Menu Bar.
    pub const fn label(self) -> &'static str {
        match self {
            SystemExtra::Search => "Search",
            SystemExtra::User => "User",
            SystemExtra::Backup => "Backup",
            SystemExtra::NowPlaying => "Now Playing",
            SystemExtra::InputSource => "Input Source",
            SystemExtra::Sound => "Sound",
            SystemExtra::Bluetooth => "Bluetooth",
            SystemExtra::WiFi => "Wi-Fi",
            SystemExtra::Focus => "Focus",
            SystemExtra::Battery => "Battery",
            SystemExtra::ControlCenter => "Control Center",
            SystemExtra::Clock => "Clock",
        }
    }

    /// Whether this extra is in the set `bits`.
    pub const fn is_on(self, bits: u16) -> bool {
        bits & self.bit() != 0
    }

    /// The extras in `bits`, left to right.
    pub fn enabled(bits: u16) -> Vec<SystemExtra> {
        SystemExtra::ALL.iter().copied().filter(|e| e.is_on(bits)).collect()
    }
}

/// Settings > Clock options of the menu bar clock (the reference clock menu's switches).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockStyle {
    /// show the date beside the time
    pub show_date: bool,
    /// show the weekday in the date
    pub show_day: bool,
    /// 24-hour time instead of 12-hour with an AM/PM marker
    pub use_24h: bool,
    /// show seconds
    pub show_seconds: bool,
    /// flash the colon between hour and minute
    pub flash_separator: bool,
}

impl Default for ClockStyle {
    fn default() -> Self {
        // the shell's current clock: date, weekday and 24-hour time without seconds
        ClockStyle { show_date: true, show_day: true, use_24h: true, show_seconds: false, flash_separator: false }
    }
}

impl ClockStyle {
    /// The time part as a pattern of `HH`, `hh`, `mm`, `ss`, `a` (AM/PM marker) and the literal separator `:`.
    pub fn time_pattern(&self) -> &'static str {
        match (self.use_24h, self.show_seconds) {
            (true, false) => "HH:mm",
            (true, true) => "HH:mm:ss",
            (false, false) => "h:mm a",
            (false, true) => "h:mm:ss a",
        }
    }

    /// The date part as a pattern of `EEE` (weekday), `d` (day) and `MMM` (month), or `None` when the date is hidden.
    pub fn date_pattern(&self) -> Option<&'static str> {
        match (self.show_date, self.show_day) {
            (false, _) => None,
            (true, true) => Some("EEE d MMM"),
            (true, false) => Some("d MMM"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_round_trip() {
        for (i, e) in SystemExtra::ALL.iter().enumerate() {
            assert_eq!(*e as usize, i, "discriminant follows ALL order");
            assert_eq!(SystemExtra::from_id(e.id()), Some(*e));
            for other in &SystemExtra::ALL[i + 1..] {
                assert_ne!(e.id(), other.id());
            }
        }
        assert_eq!(SystemExtra::from_id("nope"), None);
    }

    #[test]
    fn bits_are_distinct_and_fit_the_set() {
        let mut seen = 0u16;
        for e in SystemExtra::ALL {
            assert_eq!(seen & e.bit(), 0);
            seen |= e.bit();
        }
        assert_eq!(seen, (1u16 << SystemExtra::ALL.len()) - 1);
    }

    #[test]
    fn defaults_leave_out_user_and_backup() {
        assert!(!SystemExtra::User.is_on(SystemExtra::DEFAULT_BITS));
        assert!(!SystemExtra::Backup.is_on(SystemExtra::DEFAULT_BITS));
        assert!(SystemExtra::Clock.is_on(SystemExtra::DEFAULT_BITS));
        assert_eq!(SystemExtra::enabled(SystemExtra::DEFAULT_BITS).len(), SystemExtra::ALL.len() - 2);
    }

    #[test]
    fn enabled_keeps_left_to_right_order() {
        let bits = SystemExtra::bits_of(&[SystemExtra::Clock, SystemExtra::Search, SystemExtra::Battery]);
        assert_eq!(SystemExtra::enabled(bits), vec![SystemExtra::Search, SystemExtra::Battery, SystemExtra::Clock]);
        assert!(SystemExtra::enabled(0).is_empty());
    }

    #[test]
    fn clock_patterns_follow_options() {
        let d = ClockStyle::default();
        assert_eq!(d.time_pattern(), "HH:mm");
        assert_eq!(d.date_pattern(), Some("EEE d MMM"));
        let twelve = ClockStyle { use_24h: false, show_seconds: true, show_day: false, ..d };
        assert_eq!(twelve.time_pattern(), "h:mm:ss a");
        assert_eq!(twelve.date_pattern(), Some("d MMM"));
        let hidden = ClockStyle { show_date: false, ..d };
        assert_eq!(hidden.date_pattern(), None);
    }
}
