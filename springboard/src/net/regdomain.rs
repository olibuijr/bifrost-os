//! The wireless regulatory domain: the country whose radio rules the Wi-Fi chip follows (which channels and power
//! levels are allowed). Set from the Language & Region country with `iw reg set`, and read back with `iw reg get`.
//! The parsing and the argument checks are pure; `current` and `apply` run `iw`.

use std::process::Command;

/// The country code `iw` uses for the world-safe default, which is not a country.
const WORLD: &str = "00";

/// Whether `code` is a two-letter ISO 3166 country code (upper-case ASCII letters), the form `iw reg set` takes.
pub fn is_country_code(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase())
}

/// The country named by the output of `iw reg get`, from its first `country XX:` line. The world default (`00`) and
/// output with no country line give `None`.
pub fn parse_get(output: &str) -> Option<String> {
    let rest = output.lines().find_map(|line| line.trim_start().strip_prefix("country "))?;
    let code = rest.split(':').next()?.trim();
    (is_country_code(code) && code != WORLD).then(|| code.to_string())
}

/// The arguments for `iw` that set `country`, or `None` when it is not a country code.
pub fn set_args(country: &str) -> Option<[String; 3]> {
    is_country_code(country).then(|| ["reg".to_string(), "set".to_string(), country.to_string()])
}

/// The country `iw reg get` reports now, or `None` when `iw` is missing or the domain is the world default.
pub fn current() -> Option<String> {
    let out = Command::new("iw").args(["reg", "get"]).output().ok()?;
    parse_get(&String::from_utf8_lossy(&out.stdout))
}

/// Sets the wireless regulatory domain to `country`. Refuses anything that is not a country code before running `iw`.
pub fn apply(country: &str) -> Result<(), String> {
    let args = set_args(country).ok_or_else(|| format!("{country:?} is not a country code"))?;
    let status = Command::new("iw").args(&args).status().map_err(|e| format!("iw: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("iw reg set {country} failed: {status}"))
    }
}

/// The country the Language & Region setting names (`region=` in `settings.ini`), upper-cased, or `None` when it
/// is unset or is not a country code.
pub fn from_region_pref(region: Option<&str>) -> Option<String> {
    let code = region?.trim().to_ascii_uppercase();
    is_country_code(&code).then_some(code)
}

/// How many times a country that `iw reg get` does not show is set again before the follower gives up until the
/// setting changes (a chip with its own regulatory rules keeps reporting its own country).
const MAX_RETRIES: u8 = 3;

/// Decides when the regulatory domain is set: at the first pass, on each change of the setting, and again when the
/// kernel reports another country than the setting names (the driver or an access point can move it back).
#[derive(Debug, Default)]
pub struct Follower {
    target: Option<String>,
    retries: u8,
}

impl Follower {
    /// The country to apply now, given the country the setting names and the country the kernel reports.
    pub fn step(&mut self, desired: Option<&str>, current: Option<&str>) -> Option<String> {
        let desired = desired?;
        if self.target.as_deref() != Some(desired) {
            self.target = Some(desired.to_string());
            self.retries = 0;
            return Some(desired.to_string());
        }
        if current == Some(desired) {
            self.retries = 0;
            return None;
        }
        if self.retries < MAX_RETRIES {
            self.retries += 1;
            return Some(desired.to_string());
        }
        None
    }
}

/// Starts a thread that applies the region's country at start and again each time the setting changes, and sets it
/// again when the kernel has moved to another country. The shell saves the setting to `settings_path`; the file is
/// read every few seconds, which is cheap and needs no new message on the network socket. A failed `iw` call is
/// logged and tried again on the following passes, up to `MAX_RETRIES` times for one setting.
pub fn follow_region(settings_path: std::path::PathBuf) {
    crate::bare::sys::spawn_named("regdomain", move || {
        let mut follower = Follower::default();
        loop {
            let prefs = crate::shell::prefs::Prefs::load(&settings_path);
            let desired = from_region_pref(prefs.get("region"));
            if let Some(country) = follower.step(desired.as_deref(), current().as_deref()) {
                match apply(&country) {
                    Ok(()) => eprintln!("netd: wireless regulatory domain {country}"),
                    Err(e) => eprintln!("netd: {e}"),
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_region_setting_names_the_country_in_upper_case() {
        assert_eq!(from_region_pref(Some("is")).as_deref(), Some("IS"));
        assert_eq!(from_region_pref(Some(" DK ")).as_deref(), Some("DK"));
        assert_eq!(from_region_pref(Some("Iceland")), None, "a name is not a code");
        assert_eq!(from_region_pref(Some("")), None);
        assert_eq!(from_region_pref(None), None, "no region saved, no domain set");
    }

    #[test]
    fn a_country_code_is_two_upper_case_letters() {
        assert!(is_country_code("IS"));
        assert!(!is_country_code("is"));
        assert!(!is_country_code("ISL"));
        assert!(!is_country_code("I1"));
        assert!(!is_country_code(""));
    }

    #[test]
    fn the_country_is_read_from_the_first_country_line() {
        let out = "global\ncountry IS: DFS-ETSI\n\t(2400 - 2483 @ 40), (N/A, 20), (N/A)\n";
        assert_eq!(parse_get(out).as_deref(), Some("IS"));
        assert_eq!(parse_get("country US: DFS-FCC\n"), Some("US".to_string()));
    }

    #[test]
    fn the_world_default_and_missing_lines_are_no_country() {
        assert_eq!(parse_get("global\ncountry 00: DFS-UNSET\n"), None);
        assert_eq!(parse_get("nl80211 not found\n"), None);
        assert_eq!(parse_get(""), None);
    }

    #[test]
    fn the_follower_applies_at_the_first_pass_and_on_each_change() {
        let mut f = Follower::default();
        assert_eq!(f.step(None, None), None, "no region saved, nothing to set");
        assert_eq!(f.step(Some("IS"), None).as_deref(), Some("IS"), "first pass sets the country");
        assert_eq!(f.step(Some("IS"), Some("IS")), None, "kernel already reports it");
        assert_eq!(f.step(Some("DK"), Some("IS")).as_deref(), Some("DK"), "a changed setting is applied at once");
        assert_eq!(f.step(Some("DK"), Some("DK")), None);
    }

    #[test]
    fn the_follower_sets_the_country_again_when_the_kernel_moved_away() {
        let mut f = Follower::default();
        assert!(f.step(Some("IS"), None).is_some());
        assert_eq!(f.step(Some("IS"), Some("US")).as_deref(), Some("IS"), "the driver reset the domain");
        assert_eq!(f.step(Some("IS"), None).as_deref(), Some("IS"), "the world default is not the country either");
    }

    #[test]
    fn the_follower_gives_up_after_the_retries_until_the_setting_changes() {
        let mut f = Follower::default();
        assert!(f.step(Some("IS"), Some("US")).is_some(), "first pass");
        for _ in 0..MAX_RETRIES {
            assert!(f.step(Some("IS"), Some("US")).is_some());
        }
        assert_eq!(f.step(Some("IS"), Some("US")), None, "a chip with its own rules is left alone");
        assert_eq!(f.step(Some("DK"), Some("US")).as_deref(), Some("DK"), "a new setting starts again");
    }

    #[test]
    fn a_match_resets_the_retries() {
        let mut f = Follower::default();
        f.step(Some("IS"), None);
        f.step(Some("IS"), Some("US"));
        assert_eq!(f.step(Some("IS"), Some("IS")), None);
        for _ in 0..MAX_RETRIES {
            assert!(f.step(Some("IS"), Some("US")).is_some(), "the count started over");
        }
    }

    #[test]
    fn the_set_command_takes_a_country_code_only() {
        assert_eq!(set_args("DK").unwrap(), ["reg".to_string(), "set".to_string(), "DK".to_string()]);
        assert_eq!(set_args("dk"), None);
        assert_eq!(set_args("DK; rm -rf /"), None, "nothing but a code reaches the command line");
        assert!(apply("not a code").is_err(), "refused before iw runs");
    }
}
