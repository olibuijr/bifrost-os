//! Rain notifications: which places have them switched on, and when one is due. The Locations page has a
//! Notifications switch per place; when a refresh shows rain starting within the hour at such a place, the app posts
//! one notification for that spell of rain. The pure parts live here so they can be checked without a shell.

use crate::insights;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// The file in the app's data directory that lists the place keys with notifications on, one per line.
const FILE: &str = "notifications.txt";

/// The place keys that have notifications on.
pub fn load(dir: &Path) -> HashSet<String> {
    std::fs::read_to_string(dir.join(FILE)).unwrap_or_default().lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

/// Saves the place keys that have notifications on, in a stable order.
pub fn save(dir: &Path, keys: &HashSet<String>) {
    let mut list: Vec<&String> = keys.iter().collect();
    list.sort();
    let text: String = list.iter().map(|k| format!("{k}\n")).collect();
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join(format!("{FILE}.tmp"));
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(tmp, dir.join(FILE));
    }
}

/// One notification: a title and a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// The chance of precipitation tomorrow, in percent, from which a place with notifications on is told.
pub const TOMORROW_CHANCE: u8 = 60;

/// Remembers the places already notified about a spell of rain, so a refresh every fifteen minutes does not repeat it,
/// and the day each place was told about rain tomorrow.
#[derive(Default)]
pub struct RainWatch {
    notified: HashSet<String>,
    tomorrow: HashMap<String, String>,
}

impl RainWatch {
    /// Looks at tomorrow's chance of precipitation at the place `key` called `name`, on the local date `today` of the
    /// current conditions. A notice comes back once per day when notifications are on and the chance is at least
    /// `TOMORROW_CHANCE` percent. Turning notifications off forgets the place.
    pub fn check_tomorrow(&mut self, key: &str, name: &str, today: &str, chance: u8, enabled: bool) -> Option<Notice> {
        if !enabled {
            self.tomorrow.remove(key);
            return None;
        }
        if chance < TOMORROW_CHANCE || self.tomorrow.get(key).is_some_and(|day| day == today) {
            return None;
        }
        self.tomorrow.insert(key.to_string(), today.to_string());
        Some(Notice {
            title: format!("Rain likely tomorrow in {name}"),
            body: format!("The chance of precipitation tomorrow is {chance} percent."),
        })
    }

    /// Looks at a fresh minute forecast for the place `key` called `name`. A notice comes back when notifications
    /// are on, it is dry now and rain starts within the hour, and this spell has not been announced yet. A dry hour
    /// ends the spell, so the next rain is announced again; turning notifications off forgets the place.
    pub fn check(&mut self, key: &str, name: &str, minutes: &[f64], enabled: bool) -> Option<Notice> {
        if !enabled {
            self.notified.remove(key);
            return None;
        }
        match insights::rain_due_in(minutes) {
            Some(due) => self.notified.insert(key.to_string()).then(|| Notice {
                title: format!("Rain soon in {name}"),
                body: format!("Rain is expected to start in about {due} minutes."),
            }),
            None => {
                if !minutes.iter().any(|m| *m >= insights::WET_STEP_MM) {
                    self.notified.remove(key);
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRY: [f64; 4] = [0.0, 0.0, 0.0, 0.0];
    const SOON: [f64; 4] = [0.0, 0.0, 0.5, 0.5];
    const WET: [f64; 4] = [0.5, 0.5, 0.0, 0.0];

    #[test]
    fn rain_starting_within_the_hour_is_announced_once_per_spell() {
        let mut w = RainWatch::default();
        assert_eq!(w.check("a", "Akureyri", &DRY, true), None);
        let n = w.check("a", "Akureyri", &SOON, true).unwrap();
        assert_eq!(n.title, "Rain soon in Akureyri");
        assert_eq!(n.body, "Rain is expected to start in about 30 minutes.");
        assert_eq!(w.check("a", "Akureyri", &SOON, true), None, "the next refresh does not repeat it");
        assert_eq!(w.check("a", "Akureyri", &WET, true), None, "it is raining now: already announced");
        assert_eq!(w.check("a", "Akureyri", &SOON, true), None, "the rain has not stopped in between");
        assert_eq!(w.check("a", "Akureyri", &DRY, true), None, "a dry hour ends the spell");
        assert!(w.check("a", "Akureyri", &SOON, true).is_some(), "the next rain is announced again");
    }

    #[test]
    fn a_place_with_notifications_off_announces_nothing_and_is_forgotten() {
        let mut w = RainWatch::default();
        assert_eq!(w.check("a", "A", &SOON, false), None);
        assert!(w.check("a", "A", &SOON, true).is_some());
        assert_eq!(w.check("a", "A", &SOON, false), None);
        assert!(w.check("a", "A", &SOON, true).is_some(), "switching off and on again starts afresh");
    }

    #[test]
    fn rain_tomorrow_is_announced_once_a_day_at_the_threshold() {
        let mut w = RainWatch::default();
        assert_eq!(w.check_tomorrow("a", "Akureyri", "2026-10-04", TOMORROW_CHANCE - 1, true), None, "below the threshold");
        let n = w.check_tomorrow("a", "Akureyri", "2026-10-04", 60, true).unwrap();
        assert_eq!(n.title, "Rain likely tomorrow in Akureyri");
        assert_eq!(n.body, "The chance of precipitation tomorrow is 60 percent.");
        assert_eq!(w.check_tomorrow("a", "Akureyri", "2026-10-04", 85, true), None, "the next refresh the same day does not repeat it");
        assert!(w.check_tomorrow("a", "Akureyri", "2026-10-05", 85, true).is_some(), "the next day it is announced again");
    }

    #[test]
    fn rain_tomorrow_is_silent_with_notifications_off_and_forgets_the_place() {
        let mut w = RainWatch::default();
        assert!(w.check_tomorrow("a", "A", "2026-10-04", 90, true).is_some());
        assert_eq!(w.check_tomorrow("a", "A", "2026-10-04", 90, false), None);
        assert!(w.check_tomorrow("a", "A", "2026-10-04", 90, true).is_some(), "switching on again announces it again the same day");
    }

    #[test]
    fn places_are_watched_separately() {
        let mut w = RainWatch::default();
        assert!(w.check("a", "A", &SOON, true).is_some());
        assert!(w.check("b", "B", &SOON, true).is_some());
    }

    #[test]
    fn rain_already_falling_when_first_seen_is_not_announced() {
        let mut w = RainWatch::default();
        assert_eq!(w.check("a", "A", &WET, true), None);
        assert_eq!(w.check("a", "A", &[], true), None, "no minute data says nothing");
    }

    #[test]
    fn the_switches_are_saved_and_loaded() {
        let dir = std::env::temp_dir().join(format!("weather-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load(&dir).is_empty(), "a first run has none on");
        let keys: HashSet<String> = ["64.14_-21.94", "65.68_-18.13"].iter().map(|s| s.to_string()).collect();
        save(&dir, &keys);
        assert_eq!(load(&dir), keys);
        save(&dir, &HashSet::new());
        assert!(load(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
