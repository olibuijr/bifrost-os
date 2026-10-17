//! Focus modes: named Focus settings that decide which notifications show.
//! Do Not Disturb, Work, Personal and Sleep come built in; the person adds
//! more. A mode lists the people and apps that may interrupt, whether
//! time-sensitive notifications still show, and an optional schedule (a
//! window of minutes on chosen weekdays, which may cross midnight). One mode
//! is on by hand (`Modes::manual`) or by its schedule. Saved as
//! `focus_modes.ini` next to `notifications.ini`.
//!
//! Weekdays are numbered 0 (Monday) to 6 (Sunday); a schedule's `days` mask
//! has bit `d` set for weekday `d`. A window that crosses midnight belongs to
//! the weekday it starts on: Friday 22:00 to 07:00 covers Friday night and
//! the early hours of Saturday.

use super::schedule::{format_minutes, parse_minutes};
use crate::shell::prefs::write_atomic;
use std::path::Path;

/// The built-in Do Not Disturb mode's id.
pub const DND_ID: &str = "do-not-disturb";
/// The name Do Not Disturb shows, and the name apps store their Focus filter values under.
pub const DND_NAME: &str = "Do Not Disturb";
/// Most modes the person can keep.
pub const MAX_MODES: usize = 12;
/// Longest mode name kept, in characters.
const MAX_NAME: usize = 40;
/// Every weekday.
pub const ALL_DAYS: u8 = 0b111_1111;
const DAY_MIN: u16 = 1440;

/// The weekday (0 = Monday .. 6 = Sunday) of Unix `unix` at `offset` seconds from UTC.
pub fn weekday_of(unix: i64, offset: i32) -> u8 {
    // 1970-01-01 was a Thursday, weekday 3
    let days = (unix + offset as i64).div_euclid(86_400);
    (days + 3).rem_euclid(7) as u8
}

/// A daily window on the weekdays in `days`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModeSchedule {
    /// bit d set: the window opens on weekday d
    pub days: u8,
    /// local minute of the day the window opens
    pub start: u16,
    /// local minute of the day it closes
    pub end: u16,
}

impl ModeSchedule {
    /// Whether the window covers local `minute` on `weekday`. An empty window (start == end) covers nothing.
    pub fn covers(&self, weekday: u8, minute: u16) -> bool {
        if self.start == self.end {
            return false;
        }
        let (wd, m) = (weekday % 7, minute % DAY_MIN);
        let on = |d: u8| self.days & (1u8 << d) != 0;
        if self.start < self.end {
            on(wd) && m >= self.start && m < self.end
        } else if m >= self.start {
            on(wd)
        } else {
            m < self.end && on((wd + 6) % 7)
        }
    }

    /// `1111100 22:00-07:00`: seven 0/1 digits from Monday, then the window.
    fn text(&self) -> String {
        let days: String = (0u8..7).map(|d| if self.days & (1u8 << d) != 0 { '1' } else { '0' }).collect();
        format!("{days} {}-{}", format_minutes(self.start), format_minutes(self.end))
    }

    fn parse(text: &str) -> Option<ModeSchedule> {
        let (days, times) = text.trim().split_once(' ')?;
        if days.len() != 7 || !days.bytes().all(|b| b == b'0' || b == b'1') {
            return None;
        }
        let mut mask = 0u8;
        for (d, b) in days.bytes().enumerate() {
            if b == b'1' {
                mask |= 1u8 << d;
            }
        }
        let (a, b) = times.split_once('-')?;
        Some(ModeSchedule { days: mask, start: parse_minutes(a)?, end: parse_minutes(b)? })
    }
}

/// One Focus mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusMode {
    /// stable key, lowercase letters, digits and dashes
    pub id: String,
    /// the name the person sees, and the Focus name apps store filter values under
    pub name: String,
    /// senders that may interrupt (names or addresses, matched exactly)
    pub people: Vec<String>,
    /// app keys (bundle ids) whose notifications may interrupt
    pub apps: Vec<String>,
    /// whether time-sensitive notifications still show
    pub time_sensitive: bool,
    /// when the mode turns on by itself; None when it is only on by hand
    pub schedule: Option<ModeSchedule>,
}

impl FocusMode {
    fn new(id: &str, name: &str) -> FocusMode {
        FocusMode { id: id.into(), name: name.into(), people: Vec::new(), apps: Vec::new(), time_sensitive: false, schedule: None }
    }
}

/// What happens to one notification that arrives while a Focus is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// a banner, and a wake for a dark screen
    Show,
    /// stored in Notification Center and badged, with no banner or wake
    Quiet,
}

/// Decides delivery for a notification from app key `app` (and sender `from`, empty when unknown) while `mode` is
/// on. With no mode on, everything shows.
pub fn decide(mode: Option<&FocusMode>, app: &str, from: &str, time_sensitive: bool) -> Delivery {
    let Some(mode) = mode else { return Delivery::Show };
    let app_allowed = mode.apps.iter().any(|a| a == app);
    let person_allowed = !from.is_empty() && mode.people.iter().any(|p| p == from);
    let urgent_allowed = time_sensitive && mode.time_sensitive;
    if app_allowed || person_allowed || urgent_allowed { Delivery::Show } else { Delivery::Quiet }
}

/// All Focus modes and the one on by hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Modes {
    pub list: Vec<FocusMode>,
    /// id of the mode switched on by hand, if any
    pub manual: Option<String>,
}

impl Default for Modes {
    fn default() -> Self {
        Modes::builtin()
    }
}

impl Modes {
    /// Do Not Disturb, Work, Personal and Sleep, none scheduled and none on.
    pub fn builtin() -> Modes {
        Modes {
            list: vec![FocusMode::new(DND_ID, DND_NAME), FocusMode::new("work", "Work"), FocusMode::new("personal", "Personal"), FocusMode::new("sleep", "Sleep")],
            manual: None,
        }
    }

    pub fn get(&self, id: &str) -> Option<&FocusMode> {
        self.list.iter().find(|m| m.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut FocusMode> {
        self.list.iter_mut().find(|m| m.id == id)
    }

    /// The mode switched on by hand.
    pub fn manual_mode(&self) -> Option<&FocusMode> {
        self.manual.as_deref().and_then(|id| self.get(id))
    }

    /// The first mode whose schedule covers local `minute` on `weekday`.
    pub fn scheduled(&self, weekday: u8, minute: u16) -> Option<&FocusMode> {
        self.list.iter().find(|m| m.schedule.as_ref().is_some_and(|s| s.covers(weekday, minute)))
    }

    /// Switches a mode on by hand (`Some(id)`) or off (`None`). An unknown id is refused and returns false.
    pub fn set_manual(&mut self, id: Option<&str>) -> bool {
        match id {
            Some(id) if self.get(id).is_none() => false,
            _ => {
                self.manual = id.map(str::to_string);
                true
            }
        }
    }

    /// Adds a mode named `name` and returns its id, or None when the name is empty or the list is full.
    pub fn add(&mut self, name: &str) -> Option<String> {
        let name = clean(name);
        if name.is_empty() || name.chars().count() > MAX_NAME || self.list.len() >= MAX_MODES {
            return None;
        }
        let base = slug(&name);
        let mut id = base.clone();
        let mut n = 2;
        while self.get(&id).is_some() {
            id = format!("{base}-{n}");
            n += 1;
        }
        self.list.push(FocusMode::new(&id, &name));
        Some(id)
    }

    /// Removes a custom mode. Do Not Disturb stays; removing the mode on by hand turns Focus off.
    pub fn remove(&mut self, id: &str) -> bool {
        if id == DND_ID || self.get(id).is_none() {
            return false;
        }
        self.list.retain(|m| m.id != id);
        if self.manual.as_deref() == Some(id) {
            self.manual = None;
        }
        true
    }

    /// The modes as `focus_modes.ini`: an optional `[manual]` section, then one `[mode id]` section per mode.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        if let Some(id) = &self.manual {
            out.push_str(&format!("[manual]\nid={id}\n"));
        }
        for m in &self.list {
            out.push_str(&format!(
                "[mode {}]\nname={}\npeople={}\napps={}\ntime_sensitive={}\n",
                m.id,
                clean(&m.name),
                m.people.iter().map(|p| clean(p)).collect::<Vec<_>>().join(";"),
                m.apps.join(";"),
                u8::from(m.time_sensitive)
            ));
            if let Some(s) = &m.schedule {
                out.push_str(&format!("schedule={}\n", s.text()));
            }
        }
        out
    }

    /// Reads the file text. Junk is skipped; an empty result is the built-in set, and Do Not Disturb always exists.
    pub fn parse(text: &str) -> Modes {
        let mut list: Vec<FocusMode> = Vec::new();
        let mut manual: Option<String> = None;
        let mut current: Option<FocusMode> = None;
        let mut in_manual = false;
        let finish = |m: Option<FocusMode>, list: &mut Vec<FocusMode>| {
            if let Some(m) = m {
                if list.len() < MAX_MODES && list.iter().all(|x| x.id != m.id) {
                    list.push(m);
                }
            }
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(head) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                finish(current.take(), &mut list);
                in_manual = head == "manual";
                if let Some(id) = head.strip_prefix("mode ") {
                    if valid_id(id) {
                        current = Some(FocusMode::new(id, ""));
                    }
                }
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            if in_manual {
                if key == "id" && valid_id(value) {
                    manual = Some(value.to_string());
                }
            } else if let Some(m) = current.as_mut() {
                match key {
                    "name" => m.name = clean(value),
                    "people" => m.people = split_list(value),
                    "apps" => m.apps = split_list(value),
                    "time_sensitive" => m.time_sensitive = value == "1",
                    "schedule" => m.schedule = ModeSchedule::parse(value),
                    _ => {}
                }
            }
        }
        finish(current.take(), &mut list);
        for m in list.iter_mut().filter(|m| m.name.is_empty()) {
            m.name = m.id.clone();
        }
        if list.is_empty() {
            return Modes::builtin();
        }
        if !list.iter().any(|m| m.id == DND_ID) {
            list.insert(0, FocusMode::new(DND_ID, DND_NAME));
        }
        let manual = manual.filter(|id| list.iter().any(|m| &m.id == id));
        Modes { list, manual }
    }

    /// Loads `path`; a missing or unreadable file gives the built-in modes.
    pub fn load(path: &Path) -> Modes {
        match std::fs::read_to_string(path) {
            Ok(text) => Modes::parse(&text),
            Err(_) => Modes::builtin(),
        }
    }

    /// Saves to `path` through a temporary file and a rename.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        write_atomic(path, self.serialize())
    }
}

/// Text with the characters that separate fields in the file taken out.
fn clean(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, '\n' | '\r' | ';')).collect::<String>().trim().to_string()
}

fn split_list(value: &str) -> Vec<String> {
    value.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A mode id from a name: lowercase letters and digits, words joined by dashes.
fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { "mode".to_string() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weekend_sleep() -> ModeSchedule {
        // Friday 22:00 to 07:00 on the following morning, nothing else
        ModeSchedule { days: 1 << 4, start: 22 * 60, end: 7 * 60 }
    }

    #[test]
    fn a_window_that_crosses_midnight_belongs_to_the_day_it_starts() {
        let s = weekend_sleep();
        assert!(s.covers(4, 23 * 60), "Friday night");
        assert!(s.covers(5, 6 * 60), "Saturday early hours, the window opened on Friday");
        assert!(!s.covers(5, 7 * 60), "the end is exclusive");
        assert!(!s.covers(6, 6 * 60), "Sunday morning: Saturday is not in the window");
        assert!(!s.covers(3, 23 * 60), "Thursday night is not in the window");
        assert!(!s.covers(0, 6 * 60), "Monday early hours: Sunday is not in the window");
    }

    #[test]
    fn a_daytime_window_on_weekdays_only() {
        let work = ModeSchedule { days: 0b1_1111, start: 9 * 60, end: 17 * 60 };
        assert!(work.covers(0, 9 * 60), "Monday 09:00 starts it");
        assert!(work.covers(4, 16 * 60 + 59));
        assert!(!work.covers(4, 17 * 60));
        assert!(!work.covers(5, 10 * 60), "Saturday is off");
        assert!(!work.covers(0, 8 * 60));
    }

    #[test]
    fn an_empty_window_covers_nothing_and_weekdays_wrap() {
        assert!(!ModeSchedule { days: ALL_DAYS, start: 600, end: 600 }.covers(2, 600));
        assert!(ModeSchedule { days: ALL_DAYS, start: 1400, end: 60 }.covers(6, 1439), "a Sunday night window runs on into Monday");
        assert!(ModeSchedule { days: ALL_DAYS, start: 1400, end: 60 }.covers(0, 30));
    }

    #[test]
    fn weekdays_follow_the_unix_epoch_and_local_offset() {
        assert_eq!(weekday_of(0, 0), 3, "1970-01-01 was a Thursday");
        assert_eq!(weekday_of(4 * 86_400, 0), 0, "1970-01-05 was a Monday");
        assert_eq!(weekday_of(4 * 86_400 - 60, 0), 6, "one minute before Monday is Sunday");
        assert_eq!(weekday_of(4 * 86_400 - 3600, 3600), 0, "23:00 UTC Sunday is Monday midnight at +1");
        assert_eq!(weekday_of(-3600, 0), 2, "the evening before the epoch was a Wednesday");
    }

    #[test]
    fn a_manual_mode_overrides_the_schedule_and_unknown_ids_are_refused() {
        let mut modes = Modes::builtin();
        modes.get_mut("work").unwrap().schedule = Some(ModeSchedule { days: ALL_DAYS, start: 9 * 60, end: 17 * 60 });
        assert_eq!(modes.scheduled(1, 10 * 60).map(|m| m.id.as_str()), Some("work"));
        assert!(!modes.set_manual(Some("nope")));
        assert!(modes.set_manual(Some(DND_ID)));
        assert_eq!(modes.manual_mode().map(|m| m.id.as_str()), Some(DND_ID));
        assert!(modes.set_manual(None));
        assert!(modes.manual_mode().is_none());
    }

    #[test]
    fn delivery_follows_the_mode_apps_people_and_time_sensitivity() {
        assert_eq!(decide(None, "bifrost.mail", "", false), Delivery::Show, "no Focus shows everything");
        let mut mode = FocusMode::new("work", "Work");
        mode.apps = vec!["bifrost.files".into()];
        mode.people = vec!["Anna Jensen".into()];
        assert_eq!(decide(Some(&mode), "bifrost.files", "", false), Delivery::Show);
        assert_eq!(decide(Some(&mode), "bifrost.mail", "", false), Delivery::Quiet);
        assert_eq!(decide(Some(&mode), "bifrost.mail", "Anna Jensen", false), Delivery::Show);
        assert_eq!(decide(Some(&mode), "bifrost.mail", "anna jensen", false), Delivery::Quiet, "names match exactly");
        assert_eq!(decide(Some(&mode), "bifrost.mail", "", true), Delivery::Quiet, "time-sensitive waits for the mode to allow it");
        mode.time_sensitive = true;
        assert_eq!(decide(Some(&mode), "bifrost.mail", "", true), Delivery::Show);
        assert_eq!(decide(Some(&mode), "bifrost.mail", "", false), Delivery::Quiet);
    }

    #[test]
    fn custom_modes_get_unique_slugs_and_do_not_disturb_stays() {
        let mut modes = Modes::builtin();
        assert_eq!(modes.add("Deep  Work!").as_deref(), Some("deep-work"));
        assert_eq!(modes.add("Deep Work").as_deref(), Some("deep-work-2"));
        assert_eq!(modes.add("   "), None, "an empty name is refused");
        assert_eq!(modes.add(&"x".repeat(41)), None, "a long name is refused");
        assert!(!modes.remove(DND_ID), "Do Not Disturb cannot be removed");
        modes.set_manual(Some("deep-work"));
        assert!(modes.remove("deep-work"));
        assert_eq!(modes.manual, None, "removing the mode on by hand turns Focus off");
        while modes.list.len() < MAX_MODES {
            modes.add("More").unwrap();
        }
        assert_eq!(modes.add("One too many"), None);
    }

    #[test]
    fn modes_round_trip_through_the_file() {
        let mut modes = Modes::builtin();
        let id = modes.add("Study; night").unwrap();
        {
            let work = modes.get_mut("work").unwrap();
            work.apps = vec!["bifrost.files".into(), "bifrost.terminal".into()];
            work.people = vec!["Anna Jensen".into()];
            work.time_sensitive = true;
            work.schedule = Some(ModeSchedule { days: 0b1_1111, start: 9 * 60, end: 17 * 60 });
        }
        modes.get_mut(&id).unwrap().schedule = Some(weekend_sleep());
        modes.set_manual(Some("work"));
        let again = Modes::parse(&modes.serialize());
        assert_eq!(again.manual.as_deref(), Some("work"));
        assert_eq!(again.get(&id).unwrap().name, "Study night", "the separator is taken out of the name");
        assert_eq!(again.get(&id).unwrap().schedule, Some(weekend_sleep()));
        assert_eq!(again.get("work"), modes.get("work"));
        assert_eq!(again.list.len(), modes.list.len());
    }

    #[test]
    fn junk_is_skipped_and_do_not_disturb_always_comes_back() {
        let parsed = Modes::parse("[mode work]\nname=Office\nschedule=111 09:00-17:00\nschedule=1111111 09:00-17:00\napps=a;;b\n[mode bad id]\nname=no\n[mode work]\nname=twice\n[manual]\nid=ghost\n[unknown]\nx=1\n");
        assert_eq!(parsed.list.len(), 2, "{:?}", parsed.list);
        assert_eq!(parsed.list[0].id, DND_ID, "Do Not Disturb is put back first");
        let work = parsed.get("work").unwrap();
        assert_eq!(work.name, "Office");
        assert_eq!(work.schedule, Some(ModeSchedule { days: ALL_DAYS, start: 9 * 60, end: 17 * 60 }));
        assert_eq!(work.apps, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(parsed.manual, None, "a manual mode that does not exist is dropped");
        assert_eq!(Modes::parse(""), Modes::builtin());
    }

    #[test]
    fn modes_persist_through_a_file() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/test-focus-modes").join(std::process::id().to_string());
        let path = dir.join("focus_modes.ini");
        let _ = std::fs::remove_file(&path);
        assert_eq!(Modes::load(&path), Modes::builtin(), "a missing file gives the built-in modes");
        let mut modes = Modes::builtin();
        modes.get_mut("personal").unwrap().apps = vec!["bifrost.music".into()];
        modes.set_manual(Some("personal"));
        modes.save(&path).unwrap();
        let again = Modes::load(&path);
        assert_eq!(again.manual.as_deref(), Some("personal"));
        assert_eq!(again.get("personal").unwrap().apps, vec!["bifrost.music".to_string()]);
    }
}
