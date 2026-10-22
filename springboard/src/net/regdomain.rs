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

/// The country the domain should follow: the saved region when there is one, else the default the Language & Region
/// page shows (`bifrost_ui::region::DEFAULT_REGION`). A default install never saves `region=`, so without this
/// fallback the domain would stay on the world default for ever.
pub fn effective_country(settings_text: &str) -> String {
    bifrost_ui::region::Region::from_settings_text(settings_text).region_code()
}

/// How often the settings file is read.
const SETTINGS_POLL_SECS: u64 = 5;
/// While the domain is in force, `iw reg get` is checked every this many polls, so a driver that reloads and drops the
/// domain is put right again without spawning `iw` every few seconds.
const VERIFY_EVERY_POLLS: u32 = 12;
/// After a failed `iw reg set` (no wireless device yet, `iw` missing) the next try is this many polls later.
const RETRY_AFTER_POLLS: u32 = 6;

/// What one poll of the follower decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing to do this poll.
    Idle,
    /// Run `iw reg set` for this country.
    Apply(String),
}

/// The decision logic of the follower, apart from the clock, the file and `iw`, so it can be tested.
#[derive(Debug, Default)]
pub struct Follower {
    /// The country last set successfully.
    applied: Option<String>,
    /// Polls left until the next `iw reg get` check while the domain is in force.
    verify_in: u32,
    /// Polls left before a failed country is tried again.
    retry_in: u32,
    /// The country whose last attempt failed.
    failed: Option<String>,
}

impl Follower {
    /// One poll with the country the settings name now. `current` reads the domain back (`iw reg get`) and is only
    /// called when the periodic check is due. Applies at once on the first poll and whenever `wanted` changes.
    pub fn poll(&mut self, wanted: &str, current: impl FnOnce() -> Option<String>) -> Step {
        if !is_country_code(wanted) {
            return Step::Idle;
        }
        if self.failed.as_deref() == Some(wanted) {
            if self.retry_in > 0 {
                self.retry_in -= 1;
                return Step::Idle;
            }
            return Step::Apply(wanted.to_string());
        }
        if self.applied.as_deref() != Some(wanted) {
            return Step::Apply(wanted.to_string());
        }
        if self.verify_in > 0 {
            self.verify_in -= 1;
            return Step::Idle;
        }
        self.verify_in = VERIFY_EVERY_POLLS;
        if current().as_deref() == Some(wanted) {
            Step::Idle
        } else {
            Step::Apply(wanted.to_string())
        }
    }

    /// Records the result of the `iw reg set` a `Step::Apply` asked for.
    pub fn applied(&mut self, country: &str, ok: bool) {
        if ok {
            self.applied = Some(country.to_string());
            self.failed = None;
            self.verify_in = VERIFY_EVERY_POLLS;
        } else {
            self.applied = None;
            self.failed = Some(country.to_string());
            self.retry_in = RETRY_AFTER_POLLS;
        }
    }
}

/// Starts a thread that keeps the wireless regulatory domain on the effective country (`effective_country`): applied
/// at start and again each time the setting changes. It runs in `bifrost-privd`, which is root and started at boot,
/// so the domain is set on an idle system too. The shell saves the setting to `settings_path`; the file is read every
/// few seconds, which is cheap and needs no new message on a socket. A failed `iw` call is logged and retried later.
pub fn follow_region(settings_path: std::path::PathBuf) {
    crate::bare::sys::spawn_named("regdomain", move || {
        let mut follower = Follower::default();
        loop {
            let text = std::fs::read_to_string(&settings_path).unwrap_or_default();
            let wanted = effective_country(&text);
            if let Step::Apply(country) = follower.poll(&wanted, current) {
                let result = apply(&country);
                match &result {
                    Ok(()) => eprintln!("regdomain: wireless regulatory domain {country}"),
                    // only the first failure of a country is logged; the retries stay quiet
                    Err(e) if follower.failed.as_deref() != Some(country.as_str()) => eprintln!("regdomain: {e}"),
                    Err(_) => {}
                }
                follower.applied(&country, result.is_ok());
            }
            std::thread::sleep(std::time::Duration::from_secs(SETTINGS_POLL_SECS));
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
    fn a_fresh_install_follows_the_default_region_the_settings_page_shows() {
        assert_eq!(effective_country(""), bifrost_ui::region::DEFAULT_REGION, "no file at all");
        assert_eq!(effective_country("languages=en\ncurrency=ISK\n"), "IS", "no region key saved");
        assert_eq!(effective_country("region=garbage\n"), "IS", "a damaged value falls back to the default");
    }

    #[test]
    fn a_saved_region_wins_over_the_default() {
        assert_eq!(effective_country("region=dk\n"), "DK");
        assert_eq!(effective_country("# c\nlanguages=en\nregion = US\n"), "US");
    }

    #[test]
    fn the_first_poll_applies_and_an_unchanged_setting_then_idles() {
        let mut f = Follower::default();
        assert_eq!(f.poll("IS", || panic!("no read-back before the first apply")), Step::Apply("IS".into()));
        f.applied("IS", true);
        assert_eq!(f.poll("IS", || panic!("not due yet")), Step::Idle);
    }

    #[test]
    fn a_changed_region_applies_the_new_country_at_once() {
        let mut f = Follower::default();
        f.applied("IS", true);
        assert_eq!(f.poll("DK", || panic!("a change needs no read-back")), Step::Apply("DK".into()));
    }

    #[test]
    fn a_domain_that_was_dropped_is_set_again_at_the_periodic_check() {
        let mut f = Follower::default();
        f.applied("IS", true);
        for _ in 0..VERIFY_EVERY_POLLS {
            assert_eq!(f.poll("IS", || panic!("not due")), Step::Idle);
        }
        // due: the driver fell back to the world default
        assert_eq!(f.poll("IS", || None), Step::Apply("IS".into()));
        f.applied("IS", true);
        // due again later: the domain still holds, nothing to do
        for _ in 0..VERIFY_EVERY_POLLS {
            let _ = f.poll("IS", || panic!("not due"));
        }
        assert_eq!(f.poll("IS", || Some("IS".into())), Step::Idle);
    }

    #[test]
    fn a_failed_apply_is_retried_after_a_pause_not_every_poll() {
        let mut f = Follower::default();
        assert_eq!(f.poll("IS", || None), Step::Apply("IS".into()));
        f.applied("IS", false);
        for _ in 0..RETRY_AFTER_POLLS {
            assert_eq!(f.poll("IS", || None), Step::Idle);
        }
        assert_eq!(f.poll("IS", || None), Step::Apply("IS".into()));
        f.applied("IS", true);
        assert_eq!(f.poll("IS", || None), Step::Idle, "success clears the failure");
    }

    #[test]
    fn a_new_country_is_tried_at_once_even_while_the_old_one_is_failing() {
        let mut f = Follower::default();
        f.applied("IS", false);
        assert_eq!(f.poll("DK", || None), Step::Apply("DK".into()));
    }

    #[test]
    fn something_that_is_not_a_country_code_is_never_applied() {
        let mut f = Follower::default();
        assert_eq!(f.poll("", || None), Step::Idle);
        assert_eq!(f.poll("is", || None), Step::Idle);
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
    fn the_set_command_takes_a_country_code_only() {
        assert_eq!(set_args("DK").unwrap(), ["reg".to_string(), "set".to_string(), "DK".to_string()]);
        assert_eq!(set_args("dk"), None);
        assert_eq!(set_args("DK; rm -rf /"), None, "nothing but a code reaches the command line");
        assert!(apply("not a code").is_err(), "refused before iw runs");
    }
}
