//! Reminders > Settings: the list new reminders go to, the time an all-day reminder alerts, and whether the Dock badge
//! counts the reminders that are due. The view choices that survive a restart (the sort order, the hidden smart lists
//! and the folded subtasks) are kept here too. They live in their own small file next to the reminders, so Undo and
//! Redo, which step through the reminders, never touch them.
use crate::model::{Smart, Sort, DEFAULT_ALERT_MIN};
use std::collections::BTreeSet;
use std::path::Path;

const HEADER: &str = "springboard-reminders-settings-v1";

/// The all-day alert times the settings offer, in minutes after midnight: every half hour from six in the morning to
/// eight in the evening, so 6:00, 6:30, 7:00 and so on up to 20:00.
pub const ALL_DAY_CHOICES: [u16; 29] = all_day_choices();

const fn all_day_choices() -> [u16; 29] {
    let mut out = [0u16; 29];
    let mut i = 0;
    while i < out.len() {
        out[i] = 6 * 60 + 30 * i as u16;
        i += 1;
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The list new reminders from a smart list go to; none is the first list.
    pub default_list: Option<u64>,
    /// When a reminder that has a day but no time alerts, in minutes after midnight.
    pub all_day_min: u16,
    /// Whether the Dock badge shows the number of reminders that are due.
    pub badge: bool,
    /// The order the open list is shown in (View > Sort By).
    pub sort: Sort,
    /// The smart list tiles hidden in the sidebar, as `Options::hidden_smart` bits.
    pub hidden_smart: u8,
    /// The reminders whose subtasks are folded shut.
    pub collapsed: BTreeSet<u64>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_list: None,
            all_day_min: DEFAULT_ALERT_MIN,
            badge: true,
            sort: Sort::Manual,
            hidden_smart: 0,
            collapsed: BTreeSet::new(),
        }
    }
}

impl Settings {
    /// Reads the file's text. A file with another header, or a line with a value out of range, leaves that setting at its
    /// default; unknown lines are skipped.
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return out;
        }
        for line in lines {
            let Some((key, value)) = line.split_once('\t') else { continue };
            match key {
                "default_list" => out.default_list = if value == "-" { None } else { value.parse().ok().or(out.default_list) },
                "all_day_alert" => {
                    if let Some(min) = value.parse::<u16>().ok().filter(|m| *m < 1440) {
                        out.all_day_min = min;
                    }
                }
                "badge" => {
                    if let Some(on) = match value {
                        "0" => Some(false),
                        "1" => Some(true),
                        _ => None,
                    } {
                        out.badge = on;
                    }
                }
                "sort" => out.sort = Sort::from_key(value).unwrap_or(out.sort),
                "hidden_smart" => {
                    if let Some(bits) = value.parse::<u8>().ok().filter(|b| usize::from(*b) < 1 << Smart::ALL.len()) {
                        out.hidden_smart = bits;
                    }
                }
                "collapsed" => out.collapsed = value.split(',').filter_map(|id| id.parse().ok()).collect(),
                _ => {}
            }
        }
        out
    }

    pub fn encode(&self) -> String {
        let collapsed: Vec<String> = self.collapsed.iter().map(u64::to_string).collect();
        format!(
            "{HEADER}\ndefault_list\t{}\nall_day_alert\t{}\nbadge\t{}\nsort\t{}\nhidden_smart\t{}\ncollapsed\t{}\n",
            self.default_list.map_or_else(|| "-".to_string(), |id| id.to_string()),
            self.all_day_min,
            u8::from(self.badge),
            self.sort.key(),
            self.hidden_smart,
            collapsed.join(","),
        )
    }
}
/// The all-day alert time after `current` among [`ALL_DAY_CHOICES`], from the last back to the first. A time that is
/// not a choice goes to the first.
pub fn next_all_day(current: u16) -> u16 {
    match ALL_DAY_CHOICES.iter().position(|m| *m == current) {
        Some(i) => ALL_DAY_CHOICES[(i + 1) % ALL_DAY_CHOICES.len()],
        None => ALL_DAY_CHOICES[0],
    }
}

/// The saved settings, or the defaults when the file is missing or unreadable.
pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path).map(|text| Settings::parse(&text)).unwrap_or_default()
}

pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    springboard::shell::fsutil::write_atomic(path, settings.encode())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("target/test-reminders-settings-{}-{name}/settings.tsv", std::process::id()));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        path
    }

    #[test]
    fn the_defaults_are_the_first_list_nine_in_the_morning_and_a_badge() {
        let s = Settings::default();
        assert_eq!((s.default_list, s.all_day_min, s.badge), (None, 9 * 60, true));
    }

    #[test]
    fn settings_round_trip_through_their_text() {
        let s = Settings { default_list: Some(7), all_day_min: 8 * 60 + 30, badge: false, ..Settings::default() };
        assert_eq!(Settings::parse(&s.encode()), s);
        assert_eq!(Settings::parse(&Settings::default().encode()), Settings::default());
    }

    #[test]
    fn view_choices_survive_a_restart() {
        let mut s = Settings::default();
        s.sort = Sort::Due;
        s.hidden_smart = 0b10010;
        s.collapsed = [4, 9].into();
        let text = s.encode();
        assert!(text.contains("\nsort\tdue\n") && text.contains("\nhidden_smart\t18\n") && text.contains("\ncollapsed\t4,9\n"));
        assert_eq!(Settings::parse(&text), s);
    }

    #[test]
    fn a_bad_view_choice_is_ignored() {
        let s = Settings::parse(&format!("{HEADER}\nsort\tbogus\nhidden_smart\t64\ncollapsed\t3,x,5\n"));
        assert_eq!(s.sort, Sort::Manual);
        assert_eq!(s.hidden_smart, 0, "bit 6 names no smart list");
        assert_eq!(s.collapsed, [3, 5].into(), "an id that is not a number is skipped");
    }

    #[test]
    fn a_missing_header_or_a_bad_value_leaves_the_default() {
        assert_eq!(Settings::parse(""), Settings::default());
        assert_eq!(Settings::parse("other-format\nbadge\t0\n"), Settings::default(), "a file of another format is ignored whole");
        let s = Settings::parse(&format!("{HEADER}\ndefault_list\tx\nall_day_alert\t1440\nbadge\t2\nunknown\t1\nno tab here\n"));
        assert_eq!(s, Settings::default());
        let s = Settings::parse(&format!("{HEADER}\nall_day_alert\t0\nbadge\t0\ndefault_list\t-\n"));
        assert_eq!((s.all_day_min, s.badge, s.default_list), (0, false, None), "midnight is a valid time");
    }

    #[test]
    fn the_all_day_choices_are_every_half_hour_from_six_to_eight() {
        assert_eq!(ALL_DAY_CHOICES.len(), 29);
        assert_eq!((ALL_DAY_CHOICES[0], ALL_DAY_CHOICES[1], ALL_DAY_CHOICES[28]), (6 * 60, 6 * 60 + 30, 20 * 60));
        assert!(ALL_DAY_CHOICES.windows(2).all(|w| w[1] - w[0] == 30), "no gap between two choices");
    }

    #[test]
    fn the_all_day_time_steps_through_the_choices_and_wraps() {
        assert_eq!(next_all_day(9 * 60), 9 * 60 + 30, "the steps are half hours");
        assert_eq!(next_all_day(12 * 60 + 30), 13 * 60);
        assert_eq!(next_all_day(20 * 60), 6 * 60, "the last choice wraps to the first");
        assert_eq!(next_all_day(5 * 60 + 30), 6 * 60, "a time before the first choice goes to the first");
        let mut t = ALL_DAY_CHOICES[0];
        for _ in 0..ALL_DAY_CHOICES.len() {
            t = next_all_day(t);
        }
        assert_eq!(t, ALL_DAY_CHOICES[0], "a full turn is back at the start");
    }

    #[test]
    fn settings_are_saved_and_loaded_from_a_file() {
        let path = scratch("file");
        assert_eq!(load(&path), Settings::default(), "no file yet");
        let s = Settings { default_list: Some(3), all_day_min: 7 * 60, badge: false, ..Settings::default() };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
