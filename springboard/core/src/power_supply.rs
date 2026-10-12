//! Power supply model: the battery, mains and USB supplies that the kernel lists under the power-supply class,
//! each read into one record.
//!
//! Pure: `from_attributes` takes the attribute texts (`capacity`, `status`, `energy_now`, ...) as the caller read
//! them. `sources_in` reads one class directory and feeds them in. Units follow the kernel ABI: energy in
//! microwatt-hours, power in microwatts, charge in microampere-hours, current in microamperes. Time estimates are
//! whole minutes, rounded down. The fields mirror the power-source description keys of the official IOKit power
//! source API (current capacity, charging state, time to empty, time to full).
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The kernel's `type` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupplyKind {
    Battery,
    Mains,
    Usb,
    Other,
}

impl SupplyKind {
    pub fn parse(text: &str) -> SupplyKind {
        let t = text.trim();
        if t == "Battery" {
            SupplyKind::Battery
        } else if t == "Mains" {
            SupplyKind::Mains
        } else if t.starts_with("USB") {
            SupplyKind::Usb
        } else {
            SupplyKind::Other
        }
    }
}

/// The kernel's `status` attribute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

impl PowerState {
    pub fn parse(text: &str) -> PowerState {
        match text.trim() {
            "Charging" => PowerState::Charging,
            "Discharging" => PowerState::Discharging,
            "Full" => PowerState::Full,
            "Not charging" => PowerState::NotCharging,
            _ => PowerState::Unknown,
        }
    }
}

/// One supply. `percent` is 0 to 100 when the supply reports a level. The time fields are set only when the state
/// and the rate allow an estimate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerSource {
    pub name: String,
    pub kind: SupplyKind,
    pub percent: Option<u8>,
    pub state: PowerState,
    pub online: Option<bool>,
    pub minutes_to_empty: Option<u32>,
    pub minutes_to_full: Option<u32>,
}

/// Attribute files read for each supply. Missing files are simply absent from the map.
pub const ATTRIBUTES: &[&str] = &[
    "type",
    "status",
    "capacity",
    "online",
    "energy_now",
    "energy_full",
    "power_now",
    "charge_now",
    "charge_full",
    "current_now",
];

/// Whole minutes to move `amount` at `rate` per hour-unit: energy over power, or charge over current.
fn minutes(amount: u64, rate: u64) -> u32 {
    (amount.saturating_mul(60) / rate).min(u32::MAX as u64) as u32
}

/// Builds the record from attribute texts. Energy readings win over charge readings when both exist, because they
/// come with a rate in the same unit family. A missing capacity is worked out from the energy or charge pair.
pub fn from_attributes(name: &str, attrs: &BTreeMap<String, String>) -> PowerSource {
    let text = |k: &str| attrs.get(k).map(|v| v.trim());
    let num = |k: &str| text(k).and_then(|v| v.parse::<u64>().ok());
    let kind = text("type").map_or(SupplyKind::Other, SupplyKind::parse);
    let state = text("status").map_or(PowerState::Unknown, PowerState::parse);
    let online = num("online").map(|v| v != 0);
    // (now, rate, full) for the first pair the supply reports.
    let pair = match (num("energy_now"), num("power_now")) {
        (Some(now), Some(rate)) => Some((now, rate, num("energy_full"))),
        _ => match (num("charge_now"), num("current_now")) {
            (Some(now), Some(rate)) => Some((now, rate, num("charge_full"))),
            _ => None,
        },
    };
    let percent = match num("capacity") {
        Some(c) => Some(c.min(100) as u8),
        None => pair.and_then(|(now, _, full)| {
            full.filter(|f| *f > 0).map(|f| (now.saturating_mul(100) / f).min(100) as u8)
        }),
    };
    let (minutes_to_empty, minutes_to_full) = match pair {
        Some((now, rate, full)) if rate > 0 => {
            let empty = (state == PowerState::Discharging).then(|| minutes(now, rate));
            let to_full = match full {
                Some(f) if state == PowerState::Charging => Some(minutes(f.saturating_sub(now), rate)),
                _ => None,
            };
            (empty, to_full)
        }
        _ => (None, None),
    };
    PowerSource { name: name.to_owned(), kind, percent, state, online, minutes_to_empty, minutes_to_full }
}

/// Every supply under `class_dir` (the kernel's power-supply class), sorted by name. A missing directory gives an
/// empty list. A directory with none of the attribute files is skipped.
pub fn sources_in(class_dir: &Path) -> Vec<PowerSource> {
    let Ok(dir) = fs::read_dir(class_dir) else { return Vec::new() };
    let mut out: Vec<PowerSource> = dir
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let mut attrs = BTreeMap::new();
            for key in ATTRIBUTES {
                if let Ok(t) = fs::read_to_string(path.join(key)) {
                    attrs.insert((*key).to_owned(), t);
                }
            }
            if attrs.is_empty() {
                return None;
            }
            Some(from_attributes(&name, &attrs))
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    #[test]
    fn discharging_battery_reports_level_and_time_to_empty() {
        // 30 Wh left at 15 W: 120 minutes.
        let s = from_attributes(
            "BAT0",
            &attrs(&[("type", "Battery\n"), ("status", "Discharging"), ("capacity", "73"), ("energy_now", "30000000"), ("power_now", "15000000")]),
        );
        assert_eq!(s.kind, SupplyKind::Battery);
        assert_eq!(s.percent, Some(73));
        assert_eq!(s.state, PowerState::Discharging);
        assert_eq!(s.minutes_to_empty, Some(120));
        assert_eq!(s.minutes_to_full, None);
    }

    #[test]
    fn charging_battery_without_capacity_is_worked_out_from_energy() {
        // 20 of 40 Wh at 10 W charging: 50 percent, 20 Wh to go at 10 W is 120 minutes.
        let s = from_attributes(
            "BAT0",
            &attrs(&[("type", "Battery"), ("status", "Charging"), ("energy_now", "20000000"), ("energy_full", "40000000"), ("power_now", "10000000")]),
        );
        assert_eq!(s.percent, Some(50));
        assert_eq!(s.state, PowerState::Charging);
        assert_eq!(s.minutes_to_full, Some(120));
        assert_eq!(s.minutes_to_empty, None);
    }

    #[test]
    fn charge_pair_is_used_when_energy_is_missing() {
        // 3 Ah of 4 Ah at 1 A discharging: 75 percent, 3 hours.
        let s = from_attributes(
            "BAT1",
            &attrs(&[("type", "Battery"), ("status", "Discharging"), ("charge_now", "3000000"), ("charge_full", "4000000"), ("current_now", "1000000")]),
        );
        assert_eq!(s.percent, Some(75));
        assert_eq!(s.minutes_to_empty, Some(180));
    }

    #[test]
    fn mains_and_usb_supplies_have_no_level() {
        let m = from_attributes("AC", &attrs(&[("type", "Mains"), ("online", "1")]));
        assert_eq!(m.kind, SupplyKind::Mains);
        assert_eq!(m.online, Some(true));
        assert_eq!(m.percent, None);
        assert_eq!(m.state, PowerState::Unknown);
        let u = from_attributes("ucsi", &attrs(&[("type", "USB_PD"), ("online", "0")]));
        assert_eq!(u.kind, SupplyKind::Usb);
        assert_eq!(u.online, Some(false));
    }

    #[test]
    fn level_is_clamped_and_full_or_idle_states_give_no_estimate() {
        let s = from_attributes("BAT0", &attrs(&[("type", "Battery"), ("status", "Full"), ("capacity", "130"), ("energy_now", "40000000"), ("energy_full", "40000000"), ("power_now", "0")]));
        assert_eq!(s.percent, Some(100));
        assert_eq!(s.state, PowerState::Full);
        assert_eq!(s.minutes_to_empty, None);
        assert_eq!(s.minutes_to_full, None);
        let n = from_attributes("BAT0", &attrs(&[("status", "Not charging")]));
        assert_eq!(n.state, PowerState::NotCharging);
        assert_eq!(n.kind, SupplyKind::Other);
    }

    #[test]
    fn discharging_without_a_rate_gives_no_estimate() {
        let s = from_attributes("BAT0", &attrs(&[("type", "Battery"), ("status", "Discharging"), ("energy_now", "5000000")]));
        assert_eq!(s.minutes_to_empty, None);
        assert_eq!(s.percent, None);
    }

    #[test]
    fn missing_class_directory_gives_no_supplies() {
        assert!(sources_in(Path::new("/nonexistent/bifrost-power-supply")).is_empty());
    }
}
