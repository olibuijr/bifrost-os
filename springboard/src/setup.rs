//! The Setup Assistant: the first-run window after install (`apps/setup-assistant`, app id `APP_ID`). This file is its
//! model: the steps it walks, the choices it records, the state file the shell reads at start-up, and the read-only
//! probes it shows (the zone, the locale, whether a network interface is up). The install-time choices (language,
//! keyboard, account, look) come from `installer::plan::Config`; the assistant pre-fills from them and only asks what
//! the installer did not ask. Pure apart from the probes and the file helpers, so the model is tested on the host.
//!
//! The state file lives in the shell's data directory (`setup.ini`, see `state_path`). The window writes it when it
//! finishes (`done=1`); the shell copies the choices into `settings.ini` once (`applied=1` after that) through
//! `apply_pending`, because the shell owns `settings.ini` and an app must not write it.

use crate::installer::{keymaps, plan};
use crate::shell::prefs::Prefs;
use crate::shell::settings::{ACCENT_PREF, APPEARANCE_PREF};
use bifrost_render::appearance::{Accent, Mode};
use std::io;
use std::path::{Path, PathBuf};

/// The app id in the manifest (`apps/setup-assistant/manifest.toml`).
pub const APP_ID: &str = "is.olibuijr.setup.desktop";
/// The state file name inside the shell's data directory.
pub const FILE: &str = "setup.ini";
/// The settings.ini keys the shell reads for the choices that have a shell setting.
pub const ACCESSIBILITY_SHORTCUT_PREF: &str = "accessibility_shortcut";
pub const LOCATION_PREF: &str = "location_services";
pub const ANALYTICS_PREF: &str = "diagnostics_sharing";
/// Where the BifrOSt Account sign-in page lives (the web account page of bifrost-cloud, opened in the browser).
pub const ACCOUNT_URL: &str = "https://bifrost.olibuijr.com/account/";
/// The time zones offered on the Region page (IANA names). The current zone is shown first when it is not listed.
pub const TIMEZONES: [&str; 7] = ["Atlantic/Reykjavik", "Europe/London", "Europe/Berlin", "America/New_York", "America/Chicago", "America/Los_Angeles", "UTC"];

/// The screens of the assistant, in order. `Done` is the last screen; its button finishes the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Welcome,
    Language,
    Region,
    Keyboard,
    Network,
    Accessibility,
    Account,
    Appearance,
    Privacy,
    Cloud,
    Done,
}

impl Step {
    pub const ALL: [Step; 11] = [
        Step::Welcome,
        Step::Language,
        Step::Region,
        Step::Keyboard,
        Step::Network,
        Step::Accessibility,
        Step::Account,
        Step::Appearance,
        Step::Privacy,
        Step::Cloud,
        Step::Done,
    ];

    pub fn index(self) -> usize {
        Step::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The following screen; `Done` stays `Done`.
    pub fn next(self) -> Step {
        Step::ALL.get(self.index() + 1).copied().unwrap_or(Step::Done)
    }

    /// The previous screen; `Welcome` stays `Welcome`.
    pub fn back(self) -> Step {
        self.index().checked_sub(1).map_or(Step::Welcome, |i| Step::ALL[i])
    }

    pub fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome to BifrOSt",
            Step::Language => "Language",
            Step::Region => "Time Zone",
            Step::Keyboard => "Keyboard",
            Step::Network => "Network",
            Step::Accessibility => "Accessibility",
            Step::Account => "Your Account",
            Step::Appearance => "Appearance",
            Step::Privacy => "Privacy",
            Step::Cloud => "BifrOSt Account",
            Step::Done => "You're All Set",
        }
    }
}

/// Whether the user signed in to the BifrOSt Account or chose to do it later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cloud {
    Later,
    /// The account page was opened in the browser; the sign-in itself happens there.
    SignIn,
}

/// What the user chose. `Default` is what the window starts with when nothing was chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choices {
    /// IANA zone name; empty keeps the zone the system already has.
    pub timezone: String,
    /// Keyboard layout id as in `installer::keymaps` ("us", "is", ...).
    pub keyboard: String,
    pub accessibility_shortcut: bool,
    pub appearance: Mode,
    pub accent: Accent,
    pub location: bool,
    pub analytics: bool,
    pub cloud: Cloud,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            timezone: String::new(),
            keyboard: "us".into(),
            accessibility_shortcut: false,
            appearance: Mode::parse(plan::DEFAULT_APPEARANCE),
            accent: Accent::parse(plan::DEFAULT_ACCENT),
            location: false,
            analytics: false,
            cloud: Cloud::Later,
        }
    }
}

/// The state file: whether the run finished, whether the shell has copied the choices, and the choices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub done: bool,
    pub applied: bool,
    pub choices: Choices,
}

impl Default for State {
    fn default() -> Self {
        State { done: false, applied: false, choices: Choices::default() }
    }
}

/// The value of `key` in a `key=value` text, trimmed.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix('=')).map(str::trim)
}

impl State {
    /// Reads the state text. Unknown or invalid values fall back to the defaults, so a damaged file asks again instead
    /// of breaking the desktop.
    pub fn parse(text: &str) -> State {
        let d = Choices::default();
        let flag = |key: &str, default: bool| field(text, key).map_or(default, |v| v == "1");
        let timezone = field(text, "timezone").filter(|z| plan::valid_timezone(z)).unwrap_or("").to_string();
        let keyboard = field(text, "keyboard").filter(|id| keymaps::layout_index(id).is_some()).unwrap_or(&d.keyboard).to_string();
        let appearance = field(text, "appearance").map_or(d.appearance, Mode::parse);
        let accent = field(text, "accent").map_or(d.accent, Accent::parse);
        let cloud = if field(text, "cloud") == Some("signin") { Cloud::SignIn } else { Cloud::Later };
        State {
            done: flag("done", false),
            applied: flag("applied", false),
            choices: Choices {
                timezone,
                keyboard,
                accessibility_shortcut: flag("accessibility_shortcut", d.accessibility_shortcut),
                appearance,
                accent,
                location: flag("location", d.location),
                analytics: flag("analytics", d.analytics),
                cloud,
            },
        }
    }

    /// The state as `key=value` lines, in the order the file is written.
    pub fn to_text(&self) -> String {
        let c = &self.choices;
        let b = |v: bool| if v { "1" } else { "0" };
        let cloud = match c.cloud {
            Cloud::Later => "later",
            Cloud::SignIn => "signin",
        };
        format!(
            "done={}\napplied={}\ntimezone={}\nkeyboard={}\naccessibility_shortcut={}\nappearance={}\naccent={}\nlocation={}\nanalytics={}\ncloud={cloud}\n",
            b(self.done),
            b(self.applied),
            c.timezone,
            c.keyboard,
            b(c.accessibility_shortcut),
            c.appearance.key(),
            c.accent.key(),
            b(c.location),
            b(c.analytics),
        )
    }

    /// Reads the file at `path`; a missing or unreadable file is the default state (not done).
    pub fn load(path: &Path) -> State {
        std::fs::read_to_string(path).map(|t| State::parse(&t)).unwrap_or_default()
    }

    /// Writes the file atomically (a temporary file, then a rename), creating the directory when needed.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("ini.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)
    }
}

/// The state file inside the shell's data directory `shell_dir`.
pub fn state_path(shell_dir: &Path) -> PathBuf {
    shell_dir.join(FILE)
}

/// The shell's data directory for this user, the same directory `bin/shell.rs` opens (root: `/var/lib/springboard`).
pub fn shell_data_dir() -> PathBuf {
    #[cfg(unix)]
    extern "C" {
        fn geteuid() -> u32;
    }
    // SAFETY: geteuid has no preconditions.
    #[cfg(unix)]
    let root = unsafe { geteuid() } == 0;
    #[cfg(not(unix))]
    let root = false;
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    PathBuf::from(crate::bare::platform::data_dir(root, xdg.as_deref(), home.as_deref()))
}

/// Whether the assistant should open: its state file is missing or says the run did not finish.
pub fn needs_window(shell_dir: &Path) -> bool {
    !State::load(&state_path(shell_dir)).done
}

/// The shell's start-up step: when the run finished and the choices are not in settings.ini yet, copies them there
/// and marks the state applied. Returns the choices it copied, or None when there was nothing to do.
pub fn apply_pending(shell_dir: &Path, prefs: &mut Prefs) -> io::Result<Option<Choices>> {
    let path = state_path(shell_dir);
    let mut state = State::load(&path);
    if !state.done || state.applied {
        return Ok(None);
    }
    let c = state.choices.clone();
    let shortcut = if c.accessibility_shortcut { "1" } else { "0" };
    let location = if c.location { "1" } else { "0" };
    let analytics = if c.analytics { "1" } else { "0" };
    prefs.set_many(&[
        (APPEARANCE_PREF, c.appearance.key()),
        (ACCENT_PREF, c.accent.key()),
        (ACCESSIBILITY_SHORTCUT_PREF, shortcut),
        (LOCATION_PREF, location),
        (ANALYTICS_PREF, analytics),
    ])?;
    state.applied = true;
    state.save(&path)?;
    Ok(Some(c))
}

/// The zone the system uses now: the target of `/etc/localtime` after `zoneinfo/`, or None.
pub fn current_timezone(localtime: &Path) -> Option<String> {
    let target = std::fs::read_link(localtime).ok()?;
    let text = target.to_string_lossy().into_owned();
    text.split_once("zoneinfo/").map(|(_, zone)| zone.to_string()).filter(|z| plan::valid_timezone(z))
}

/// The system language from a locale.conf text (`LANG=is_IS.UTF-8`), without quotes, or None.
pub fn locale_from(conf: &str) -> Option<String> {
    field(conf, "LANG").map(|v| v.trim_matches('"').to_string()).filter(|v| !v.is_empty())
}

/// Whether any interface other than loopback is up, from `(name, operstate)` pairs.
pub fn online(states: &[(String, String)]) -> bool {
    states.iter().any(|(name, state)| name != "lo" && state == "up")
}

/// The `(name, operstate)` pairs under a sysfs class directory such as `/sys/class/net`.
pub fn interface_states(class_dir: &Path) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(class_dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let state = std::fs::read_to_string(e.path().join("operstate")).ok()?;
            Some((name, state.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_walk_in_order_and_stop_at_the_ends() {
        assert_eq!(Step::Welcome.next(), Step::Language);
        assert_eq!(Step::Cloud.next(), Step::Done);
        assert_eq!(Step::Done.next(), Step::Done);
        assert_eq!(Step::Welcome.back(), Step::Welcome);
        assert_eq!(Step::Privacy.back(), Step::Appearance);
        for (i, s) in Step::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
            assert!(!s.title().is_empty());
        }
    }

    #[test]
    fn default_choices_are_the_installer_look_and_the_privacy_off_position() {
        let c = Choices::default();
        assert_eq!((c.appearance, c.accent), (Mode::Dark, Accent::Blue));
        assert!(!c.location && !c.analytics && !c.accessibility_shortcut);
        assert_eq!(c.cloud, Cloud::Later);
        assert_eq!(c.keyboard, "us");
    }

    #[test]
    fn state_round_trips() {
        let s = State {
            done: true,
            applied: false,
            choices: Choices {
                timezone: "Atlantic/Reykjavik".into(),
                keyboard: "is".into(),
                accessibility_shortcut: true,
                appearance: Mode::Light,
                accent: Accent::Purple,
                location: true,
                analytics: false,
                cloud: Cloud::SignIn,
            },
        };
        assert_eq!(State::parse(&s.to_text()), s);
    }

    #[test]
    fn damaged_values_fall_back_to_the_defaults() {
        let s = State::parse("done=1\ntimezone=../../etc\nkeyboard=klingon\ncloud=maybe\nlocation=yes\n");
        assert!(s.done);
        assert_eq!(s.choices.timezone, "");
        assert_eq!(s.choices.keyboard, "us");
        assert_eq!(s.choices.cloud, Cloud::Later);
        assert!(!s.choices.location);
        assert!(!State::parse("").done);
    }

    #[test]
    fn the_shown_time_zones_are_all_valid_names() {
        assert!(TIMEZONES.iter().all(|z| plan::valid_timezone(z)));
    }

    #[test]
    fn zone_comes_from_the_localtime_link() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("localtime");
        std::os::unix::fs::symlink("/usr/share/zoneinfo/Atlantic/Reykjavik", &link).unwrap();
        assert_eq!(current_timezone(&link).as_deref(), Some("Atlantic/Reykjavik"));
        assert_eq!(current_timezone(&dir.path().join("missing")), None);
    }

    #[test]
    fn locale_reads_lang_without_quotes() {
        assert_eq!(locale_from("LANG=is_IS.UTF-8\n").as_deref(), Some("is_IS.UTF-8"));
        assert_eq!(locale_from("# none\nLANG=\"en_US.UTF-8\"\n").as_deref(), Some("en_US.UTF-8"));
        assert_eq!(locale_from("LANG=\n"), None);
    }

    #[test]
    fn online_ignores_loopback() {
        let pair = |n: &str, s: &str| (n.to_string(), s.to_string());
        assert!(!online(&[pair("lo", "up"), pair("wlan0", "down")]));
        assert!(online(&[pair("lo", "up"), pair("enp3s0", "up")]));
        assert!(!online(&[]));
    }

    #[test]
    fn the_shell_copies_the_choices_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        assert!(needs_window(dir.path()));
        let mut prefs = Prefs::memory();
        assert_eq!(apply_pending(dir.path(), &mut prefs).unwrap(), None);

        let done = State { done: true, applied: false, choices: Choices { appearance: Mode::Light, accent: Accent::Green, accessibility_shortcut: true, ..Choices::default() } };
        done.save(&path).unwrap();
        assert!(!needs_window(dir.path()));
        let copied = apply_pending(dir.path(), &mut prefs).unwrap().unwrap();
        assert_eq!(copied.appearance, Mode::Light);
        assert_eq!(prefs.get(APPEARANCE_PREF), Some("light"));
        assert_eq!(prefs.get(ACCENT_PREF), Some("green"));
        assert_eq!(prefs.get(ACCESSIBILITY_SHORTCUT_PREF), Some("1"));
        assert!(State::load(&path).applied);
        assert_eq!(apply_pending(dir.path(), &mut prefs).unwrap(), None);
    }
}
