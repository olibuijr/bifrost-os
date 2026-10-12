//! Power sources for apps: the battery, mains and USB supplies the system reports, read from the kernel's
//! power-supply class. The parsing and the time estimates live in `bifrost_core::power_supply`; this module chooses
//! the directory and gives apps the lookups they need.
pub use bifrost_core::power_supply::{PowerSource, PowerState, SupplyKind};
use bifrost_core::power_supply;
use std::path::Path;

/// The kernel's power-supply class directory.
pub const SUPPLY_CLASS: &str = "/sys/class/power_supply";

/// Every supply under `class_dir`, sorted by name. A missing directory gives an empty list.
pub fn current_in(class_dir: &Path) -> Vec<PowerSource> {
    power_supply::sources_in(class_dir)
}

/// Every supply the system lists, sorted by name.
pub fn current() -> Vec<PowerSource> {
    current_in(Path::new(SUPPLY_CLASS))
}

/// The first battery in `sources`, in the order given.
pub fn first_battery(sources: &[PowerSource]) -> Option<&PowerSource> {
    sources.iter().find(|s| s.kind == SupplyKind::Battery)
}

/// The first battery the system reports, if the machine has one.
pub fn battery() -> Option<PowerSource> {
    first_battery(&current()).cloned()
}

/// The time left for one supply: minutes to empty while discharging, minutes to full while charging, else None.
pub fn time_remaining(source: &PowerSource) -> Option<u32> {
    match source.state {
        PowerState::Discharging => source.minutes_to_empty,
        PowerState::Charging => source.minutes_to_full,
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn supply(name: &str, kind: &str, status: &str, energy_now: &str, power_now: &str) -> PowerSource {
        let mut attrs = BTreeMap::new();
        attrs.insert("type".to_owned(), kind.to_owned());
        attrs.insert("status".to_owned(), status.to_owned());
        attrs.insert("energy_now".to_owned(), energy_now.to_owned());
        attrs.insert("energy_full".to_owned(), "40000000".to_owned());
        attrs.insert("power_now".to_owned(), power_now.to_owned());
        power_supply::from_attributes(name, &attrs)
    }

    #[test]
    fn first_battery_skips_mains_and_keeps_order() {
        let list = vec![
            supply("AC", "Mains", "", "0", "0"),
            supply("BAT1", "Battery", "Discharging", "30000000", "15000000"),
            supply("BAT0", "Battery", "Discharging", "10000000", "15000000"),
        ];
        assert_eq!(first_battery(&list).map(|s| s.name.as_str()), Some("BAT1"));
    }

    #[test]
    fn mains_only_has_no_battery() {
        let list = vec![supply("AC", "Mains", "", "0", "0")];
        assert!(first_battery(&list).is_none());
        assert!(first_battery(&[]).is_none());
    }

    #[test]
    fn time_remaining_follows_the_state() {
        // Discharging: 30 Wh at 15 W is 120 minutes to empty.
        let out = supply("BAT0", "Battery", "Discharging", "30000000", "15000000");
        assert_eq!(time_remaining(&out), Some(120));
        // Charging: 20 Wh to go at 10 W is 120 minutes to full (the energy_full is 40 Wh).
        let mut charging = supply("BAT0", "Battery", "Charging", "20000000", "10000000");
        assert_eq!(time_remaining(&charging), Some(120));
        // Full: no estimate in either direction.
        charging.state = PowerState::Full;
        assert_eq!(time_remaining(&charging), None);
    }

    #[test]
    fn current_in_a_missing_directory_is_empty() {
        assert!(current_in(Path::new("/nonexistent/bifrost-supply-class")).is_empty());
    }

    #[test]
    fn battery_is_always_a_battery_kind() {
        // Reads the real class: on a machine without a battery this is None, on one with a battery it is Some.
        // Either way the returned supply, when present, is a battery.
        if let Some(b) = battery() {
            assert_eq!(b.kind, SupplyKind::Battery);
        }
    }
}
