//! `pmset`: reads and sets the desktop's power settings. The Energy idle timeout (Settings > Energy, "Put the computer to
//! sleep after") is the shell's `system_sleep_minutes` pref, and `pmset sleep <minutes>` writes that same pref through the
//! shell's settings store, so the file format and the atomic write are the shell's own. Zero means never.
//!
//! Supported: `-g` (prints the settings), `-g batt` (the power source and the battery charge, state and time estimate,
//! from the kernel's power-supply class through `bifrost_core::power_supply`), `sleep <minutes>`, `lowpowermode 0|1`
//! (the shell's `low_power` pref, Settings > Battery > Low Power Mode), and the power source flags `-a -b -c -u`, which
//! are accepted and all mean the one setting the desktop keeps.
//! Refused with a message: `displaysleep` (the desktop has no display-off timer to set yet), `sleepnow`, `displaysleepnow`,
//! `touch`, `boot`, `schedule`, `repeat`, `relative`, `haltlevel`, `haltafter`, `haltremain`, and every other setting.
//! Gap: the shell reads `system_sleep_minutes` when it starts, so a change applies after the next start; the shell has no
//! live reload for it yet. `-g` also omits the assertion listing the man page describes.

use bifrost_core::power_supply::{PowerSource, PowerState, SupplyKind};
use springboard::shell::prefs::Prefs;
use std::path::PathBuf;

pub const USAGE: &str = "usage: pmset [-a | -b | -c | -u] sleep minutes | lowpowermode 0|1 | pmset -g [batt]";
/// The settings-store key of the Energy idle timeout, shared with the shell's Energy settings.
pub const SLEEP_KEY: &str = "system_sleep_minutes";
/// The settings-store key of Low Power Mode, shared with Settings > Battery.
pub const LOW_POWER_KEY: &str = "low_power";
/// The kernel's power-supply class directory.
pub const SUPPLY_CLASS: &str = "/sys/class/power_supply";

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Help,
    /// `-g`: print the current settings.
    Show,
    /// `-g batt`: print the power source and the battery.
    ShowBatt,
    /// `lowpowermode 0|1`.
    SetLowPower(bool),
    /// `sleep <minutes>`: set the idle timeout; zero means never.
    SetSleep(u32),
}

/// Reads the arguments. Power source flags may come first and are ignored. An unknown setting is refused by name.
pub fn parse(args: &[String]) -> Result<Action, String> {
    let rest: Vec<&str> = args.iter().map(String::as_str).filter(|a| !matches!(*a, "-a" | "-b" | "-c" | "-u")).collect();
    match rest.as_slice() {
        [] => Ok(Action::Help),
        ["-h" | "-help" | "--help"] => Ok(Action::Help),
        ["-g"] => Ok(Action::Show),
        ["-g", "batt" | "ps"] => Ok(Action::ShowBatt),
        ["lowpowermode", v] => match *v {
            "0" => Ok(Action::SetLowPower(false)),
            "1" => Ok(Action::SetLowPower(true)),
            other => Err(format!("lowpowermode needs 0 or 1, not {other}")),
        },
        ["sleep", minutes] => {
            let m: u32 = minutes.parse().map_err(|_| format!("sleep needs a number of minutes, not {minutes}"))?;
            Ok(Action::SetSleep(m))
        }
        ["displaysleep", ..] => Err("displaysleep is not applied by the desktop yet: it has no display-off timer setting".into()),
        ["sleepnow" | "displaysleepnow" | "touch" | "boot" | "schedule" | "repeat" | "relative" | "haltlevel" | "haltafter" | "haltremain", ..] => {
            Err(format!("{} is not supported yet", rest[0]))
        }
        [other, ..] => Err(format!("unknown setting {other}")),
    }
}

/// The settings file: the shell's `settings.ini` in its data directory, the same path the shell reads.
pub fn settings_path(root: bool, xdg_data_home: Option<&str>, home: Option<&str>) -> PathBuf {
    PathBuf::from(format!("{}/settings.ini", springboard::bare::platform::data_dir(root, xdg_data_home, home)))
}

/// Whether the process runs as root: the owner of its `/proc` entry is its effective user.
pub fn is_root() -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid() == 0).unwrap_or(false)
}

/// The text `-g` prints for the idle timeout stored in `prefs`. A missing or malformed value reads as never.
pub fn show_text(prefs: &Prefs) -> String {
    let minutes: u32 = prefs.get_parsed(SLEEP_KEY).unwrap_or(0);
    let shown = if minutes == 0 { "never".to_string() } else { minutes.to_string() };
    let low = prefs.get(LOW_POWER_KEY).is_some_and(|v| matches!(v, "1" | "true" | "on"));
    format!("System-wide power settings:\n sleep        {shown}\n lowpowermode {}\n", u8::from(low))
}

/// Stores Low Power Mode as the shell's boolean pref ("1" or "0").
pub fn set_low_power(prefs: &mut Prefs, on: bool) -> std::io::Result<()> {
    prefs.set(LOW_POWER_KEY, if on { "1" } else { "0" })
}

/// `H:MM` for a whole number of minutes.
fn clock(minutes: u32) -> String {
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// The text of `-g batt`: the power source in use, then one line per battery with charge, state and time estimate.
/// The source is AC when a mains or USB supply is online, battery power otherwise.
pub fn batt_text(sources: &[PowerSource]) -> String {
    let on_ac = sources.iter().any(|s| matches!(s.kind, SupplyKind::Mains | SupplyKind::Usb) && s.online == Some(true));
    let mut out = format!("Now drawing from '{}'\n", if on_ac { "AC Power" } else { "Battery Power" });
    for s in sources.iter().filter(|s| s.kind == SupplyKind::Battery) {
        let pct = s.percent.map_or("?".to_string(), |p| p.to_string());
        let (state, time) = match s.state {
            PowerState::Charging => ("charging", s.minutes_to_full),
            PowerState::Discharging => ("discharging", s.minutes_to_empty),
            PowerState::Full => ("charged", None),
            PowerState::NotCharging => ("AC attached; not charging", None),
            PowerState::Unknown => ("unknown", None),
        };
        let estimate = match (s.state, time) {
            (_, Some(m)) => format!("{} remaining", clock(m)),
            (PowerState::Charging | PowerState::Discharging, None) => "(no estimate)".to_string(),
            _ => String::new(),
        };
        let tail = if estimate.is_empty() { String::new() } else { format!("; {estimate}") };
        out.push_str(&format!(" -{}\t{pct}%; {state}{tail} present: true\n", s.name));
    }
    out
}

/// Stores the idle timeout. Zero stores "0", which the shell reads as never.
pub fn set_sleep(prefs: &mut Prefs, minutes: u32) -> std::io::Result<()> {
    prefs.set(SLEEP_KEY, &minutes.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sleep_and_show_parse_with_source_flags_ignored() {
        assert_eq!(parse(&args(&["-g"])), Ok(Action::Show));
        assert_eq!(parse(&args(&["-a", "sleep", "15"])), Ok(Action::SetSleep(15)));
        assert_eq!(parse(&args(&["-c", "sleep", "0"])), Ok(Action::SetSleep(0)));
        assert_eq!(parse(&args(&[])), Ok(Action::Help));
        assert_eq!(parse(&args(&["-h"])), Ok(Action::Help));
    }

    #[test]
    fn display_sleep_and_other_settings_are_refused_by_name() {
        assert!(parse(&args(&["displaysleep", "10"])).unwrap_err().contains("displaysleep"));
        assert!(parse(&args(&["sleepnow"])).unwrap_err().contains("sleepnow is not supported"));
        assert!(parse(&args(&["womp", "1"])).unwrap_err().contains("unknown setting womp"));
        assert!(parse(&args(&["sleep", "soon"])).unwrap_err().contains("number of minutes"));
    }

    fn battery(state: PowerState, pct: u8, empty: Option<u32>, full: Option<u32>) -> PowerSource {
        PowerSource { name: "BAT0".into(), kind: SupplyKind::Battery, percent: Some(pct), state, online: None, minutes_to_empty: empty, minutes_to_full: full }
    }

    fn mains(online: bool) -> PowerSource {
        PowerSource { name: "AC".into(), kind: SupplyKind::Mains, percent: None, state: PowerState::Unknown, online: Some(online), minutes_to_empty: None, minutes_to_full: None }
    }

    #[test]
    fn batt_and_low_power_mode_parse() {
        assert_eq!(parse(&args(&["-g", "batt"])), Ok(Action::ShowBatt));
        assert_eq!(parse(&args(&["lowpowermode", "1"])), Ok(Action::SetLowPower(true)));
        assert_eq!(parse(&args(&["-a", "lowpowermode", "0"])), Ok(Action::SetLowPower(false)));
        assert!(parse(&args(&["lowpowermode", "2"])).unwrap_err().contains("0 or 1"));
    }

    #[test]
    fn batt_text_names_the_source_charge_state_and_estimate() {
        let on_battery = batt_text(&[mains(false), battery(PowerState::Discharging, 85, Some(192), None)]);
        assert!(on_battery.starts_with("Now drawing from 'Battery Power'\n"), "{on_battery}");
        assert!(on_battery.contains(" -BAT0\t85%; discharging; 3:12 remaining present: true"), "{on_battery}");
        let charging = batt_text(&[mains(true), battery(PowerState::Charging, 40, None, Some(45))]);
        assert!(charging.starts_with("Now drawing from 'AC Power'"), "{charging}");
        assert!(charging.contains("40%; charging; 0:45 remaining"), "{charging}");
        let full = batt_text(&[mains(true), battery(PowerState::Full, 100, None, None)]);
        assert!(full.contains("100%; charged present: true"), "{full}");
        let unknown = batt_text(&[battery(PowerState::Discharging, 7, None, None)]);
        assert!(unknown.contains("discharging; (no estimate)"), "{unknown}");
        assert_eq!(batt_text(&[]), "Now drawing from 'Battery Power'\n");
    }

    #[test]
    fn low_power_mode_is_the_settings_pref_and_shows_in_the_summary() {
        let mut p = Prefs::memory();
        assert!(show_text(&p).contains("lowpowermode 0"));
        set_low_power(&mut p, true).unwrap();
        assert_eq!(p.get(LOW_POWER_KEY), Some("1"));
        assert!(show_text(&p).contains("lowpowermode 1"));
        set_low_power(&mut p, false).unwrap();
        assert!(show_text(&p).contains("lowpowermode 0"));
    }

    #[test]
    fn the_settings_file_is_the_shells() {
        assert_eq!(settings_path(true, None, None), PathBuf::from("/var/lib/springboard/settings.ini"));
        assert_eq!(settings_path(false, Some("/data"), None), PathBuf::from("/data/springboard/settings.ini"));
        assert_eq!(settings_path(false, None, Some("/home/me")), PathBuf::from("/home/me/.local/share/springboard/settings.ini"));
    }

    #[test]
    fn show_reads_never_for_unset_or_zero() {
        let mut p = Prefs::memory();
        assert!(show_text(&p).contains("sleep        never"));
        p.set(SLEEP_KEY, "15").unwrap();
        assert!(show_text(&p).contains("sleep        15"));
        p.set(SLEEP_KEY, "0").unwrap();
        assert!(show_text(&p).contains("never"));
    }

    #[test]
    fn set_sleep_writes_the_shells_key_and_the_shell_reads_it_back() {
        let dir = std::env::temp_dir().join(format!("bifrost-pmset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.ini");
        let mut p = Prefs::load(&file);
        set_sleep(&mut p, 30).unwrap();
        let back = Prefs::load(&file);
        assert_eq!(back.get(SLEEP_KEY), Some("30"));
        assert_eq!(springboard::bare::sleep::system_sleep_after(back.get(SLEEP_KEY)), Some(1_800_000.0));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
