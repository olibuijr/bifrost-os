//! Time zones of single reminders. A reminder may name a zone ("Europe/London", or a POSIX rule such as
//! "EST5EDT,M3.2.0,M11.1.0"); its due time is then read on that zone's clock, with that zone's daylight saving, instead
//! of the clock of the device. Zones are loaded from the system's zone database and kept for the next lookup.
use springboard::bare::timezone::{self, Zone};
use std::sync::{Arc, Mutex};

/// The zones the reminder form offers one after the other, after "Same as Device".
pub const CHOICES: [&str; 12] = [
    "UTC",
    "Atlantic/Reykjavik",
    "Europe/London",
    "Europe/Paris",
    "America/New_York",
    "America/Chicago",
    "America/Denver",
    "America/Los_Angeles",
    "Asia/Dubai",
    "Asia/Kolkata",
    "Asia/Tokyo",
    "Australia/Sydney",
];

/// The longest zone name the store keeps.
pub const MAX_NAME: usize = 64;

/// Zones looked up so far (a name that does not resolve is kept as `None`, so it is not searched for again).
static CACHE: Mutex<Vec<(String, Option<Arc<Zone>>)>> = Mutex::new(Vec::new());
const CACHE_MAX: usize = 64;

/// Whether `name` is shaped like a zone name or rule: letters, digits and `/ _ + - , . :`, at most [`MAX_NAME`] long.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= MAX_NAME && name.chars().all(|c| c.is_ascii_alphanumeric() || "/_+-,.:".contains(c))
}

/// The zone called `name`: a database name, or else a POSIX rule. `None` when it is neither.
pub fn resolve(name: &str) -> Option<Arc<Zone>> {
    if !valid_name(name) {
        return None;
    }
    let mut cache = CACHE.lock().ok()?;
    if let Some((_, zone)) = cache.iter().find(|(n, _)| n == name) {
        return zone.clone();
    }
    let zone = timezone::load(name).or_else(|| Zone::from_posix_rule(name)).map(Arc::new);
    if cache.len() < CACHE_MAX {
        cache.push((name.to_string(), zone.clone()));
    }
    zone
}

/// The zone after `current` in [`CHOICES`]; none after the last. Anything that is not a choice goes to the first.
pub fn next_choice(current: Option<&str>) -> Option<String> {
    match current {
        None => Some(CHOICES[0].to_string()),
        Some(name) => match CHOICES.iter().position(|c| *c == name) {
            Some(i) => CHOICES.get(i + 1).map(|c| (*c).to_string()),
            None => Some(CHOICES[0].to_string()),
        },
    }
}

/// The place in a zone name: "London" for "Europe/London", "New York" for "America/New_York".
pub fn short_name(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// New York as a POSIX rule, so the tests do not depend on the zone database of the machine.
    const NEW_YORK: &str = "EST5EDT,M3.2.0,M11.1.0";

    #[test]
    fn a_posix_rule_names_a_zone_with_daylight_saving() {
        let zone = resolve(NEW_YORK).expect("a rule resolves");
        let summer = springboard::shell::clock::tz::days_from_civil(2026, 7, 1) * 86_400 + 12 * 3600;
        let winter = springboard::shell::clock::tz::days_from_civil(2026, 1, 15) * 86_400 + 12 * 3600;
        assert_eq!(zone.at(summer).0, -4 * 3600);
        assert_eq!(zone.at(winter).0, -5 * 3600);
        assert!(resolve(NEW_YORK).is_some(), "the second lookup comes from the cache");
    }

    #[test]
    fn names_that_are_not_zones_do_not_resolve() {
        assert!(resolve("").is_none());
        assert!(resolve("../etc/passwd").is_none());
        assert!(resolve("Not/AZone").is_none());
        assert!(resolve("Not/AZone").is_none(), "and the miss is remembered");
        assert!(resolve(&"x".repeat(MAX_NAME + 1)).is_none());
    }

    #[test]
    fn only_zone_shaped_names_are_valid() {
        assert!(valid_name("Europe/London"));
        assert!(valid_name(NEW_YORK));
        assert!(valid_name("Etc/GMT+5"));
        for bad in ["", " ", "a b", "a\tb", "a\nb", "a;b", "é"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert!(!valid_name(&"x".repeat(MAX_NAME + 1)));
    }

    #[test]
    fn the_choices_step_through_the_list_and_back_to_the_device_zone() {
        assert_eq!(next_choice(None).as_deref(), Some("UTC"));
        assert_eq!(next_choice(Some("UTC")).as_deref(), Some("Atlantic/Reykjavik"));
        assert_eq!(next_choice(Some("Australia/Sydney")), None, "after the last is the device zone again");
        assert_eq!(next_choice(Some("Asia/Seoul")).as_deref(), Some("UTC"), "a zone outside the list goes to the first");
        let mut zone = None;
        for _ in 0..CHOICES.len() {
            zone = next_choice(zone.as_deref());
            assert!(zone.is_some());
        }
        assert_eq!(next_choice(zone.as_deref()), None);
    }

    #[test]
    fn the_place_is_the_last_part_of_the_name() {
        assert_eq!(short_name("Europe/London"), "London");
        assert_eq!(short_name("America/New_York"), "New York");
        assert_eq!(short_name("UTC"), "UTC");
    }
}
