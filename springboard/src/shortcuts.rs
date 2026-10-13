//! Shortcut data and the built-in action library: a name, a tile colour, a list of actions and an optional daily
//! reminder, stored one shortcut per line (`name TAB colour TAB action;action [TAB HH:MM]`).
//! The Shortcuts app, its control socket and the `shortcuts` command share this model, so they read and write one file.
//! Values with a choice list (apps, messages, folders, scripts) are picked from that list, never typed, so a stored
//! value always has a known meaning and never contains the separators `;` or tab.

use std::path::{Path, PathBuf};

/// The file the app saves the list to, inside the app's data directory.
pub const FILE: &str = "shortcuts.tsv";
/// The app's manifest id; its data directory is `appdata/<APP_ID>` under the shell's root.
pub const APP_ID: &str = "is.olibuijr.shortcuts.desktop";
pub const MAX_SHORTCUTS: usize = 64;
pub const MAX_ACTIONS: usize = 24;
pub const MAX_NAME: usize = 40;
pub const APP_CHOICES: [&str; 10] =
    ["Notes", "Calendar", "Reminders", "Calculator", "Weather", "Camera", "Photos", "Music", "Clock", "Settings"];
/// The messages a "Show Notification" step can show.
pub const NOTE_CHOICES: [&str; 4] = ["Time for a break", "Dinner is ready", "Shortcut finished", "Stand up and stretch"];
/// The texts a "Copy Text" step can put on the clipboard.
pub const COPY_CHOICES: [&str; 4] = ["Thanks", "Be right back", "On my way", "Sent from BifrOSt"];
/// The folders in the home directory a "Open Folder" step can show.
pub const FOLDER_CHOICES: [&str; 5] = ["Desktop", "Documents", "Downloads", "Pictures", "Music"];
/// The commands a "Run Shell Script" step can run.
pub const SCRIPT_CHOICES: [&str; 4] = ["date", "uptime", "df -h /", "free -h"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Wifi(bool),
    LowPower(bool),
    Dnd(bool),
    /// percent, 0..=100
    Brightness(u8),
    OpenApp(String),
    Screenshot,
    /// minutes, 1..=180
    Timer(u32),
    /// a notification with one of `NOTE_CHOICES`
    Notify(String),
    /// copies one of `COPY_CHOICES` to the clipboard
    CopyText(String),
    /// shows one of `FOLDER_CHOICES` in the home directory
    OpenFolder(String),
    /// runs one of `SCRIPT_CHOICES` in a shell
    RunScript(String),
}

/// The kinds the "Add Action" list offers, in order.
pub const KINDS: [&str; 11] = [
    "Set Wi-Fi",
    "Set Low Power Mode",
    "Set Do Not Disturb",
    "Set Brightness",
    "Open App",
    "Take Screenshot",
    "Start Timer",
    "Show Notification",
    "Copy Text",
    "Open Folder",
    "Run Shell Script",
];

/// The next or previous of `choices` after `cur` (wrapping). A `cur` that is not one of them counts as the first choice.
fn cycle(choices: &[&str], cur: &str, up: bool) -> String {
    let i = choices.iter().position(|c| *c == cur).unwrap_or(0);
    let n = choices.len();
    choices[if up { (i + 1) % n } else { (i + n - 1) % n }].into()
}

/// `v` as an owned value when it is one of `choices`.
fn choice(choices: &[&str], v: &str) -> Option<String> {
    choices.contains(&v).then(|| v.to_string())
}

impl Action {
    pub fn of_kind(i: usize) -> Action {
        match i {
            0 => Action::Wifi(true),
            1 => Action::LowPower(true),
            2 => Action::Dnd(true),
            3 => Action::Brightness(50),
            4 => Action::OpenApp(APP_CHOICES[0].into()),
            5 => Action::Screenshot,
            6 => Action::Timer(5),
            7 => Action::Notify(NOTE_CHOICES[0].into()),
            8 => Action::CopyText(COPY_CHOICES[0].into()),
            9 => Action::OpenFolder(FOLDER_CHOICES[0].into()),
            10 => Action::RunScript(SCRIPT_CHOICES[0].into()),
            _ => Action::Timer(5),
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Action::Wifi(_) => KINDS[0],
            Action::LowPower(_) => KINDS[1],
            Action::Dnd(_) => KINDS[2],
            Action::Brightness(_) => KINDS[3],
            Action::OpenApp(_) => KINDS[4],
            Action::Screenshot => KINDS[5],
            Action::Timer(_) => KINDS[6],
            Action::Notify(_) => KINDS[7],
            Action::CopyText(_) => KINDS[8],
            Action::OpenFolder(_) => KINDS[9],
            Action::RunScript(_) => KINDS[10],
        }
    }

    /// The value shown right-aligned in the row.
    pub fn value(&self) -> String {
        let onoff = |b: &bool| if *b { "On" } else { "Off" }.to_string();
        match self {
            Action::Wifi(b) | Action::LowPower(b) | Action::Dnd(b) => onoff(b),
            Action::Brightness(p) => format!("{p}%"),
            Action::OpenApp(a) => a.clone(),
            Action::Screenshot => String::new(),
            Action::Timer(m) => format!("{m} min"),
            Action::Notify(m) | Action::CopyText(m) | Action::OpenFolder(m) | Action::RunScript(m) => m.clone(),
        }
    }

    /// The step as one line of a plan: its title, then its value when it has one.
    pub fn line(&self) -> String {
        let v = self.value();
        if v.is_empty() {
            self.title().to_string()
        } else {
            format!("{}: {v}", self.title())
        }
    }

    /// One step of the row's stepper (`up` = +); on/off values flip either way.
    pub fn step(&mut self, up: bool) {
        match self {
            Action::Wifi(b) | Action::LowPower(b) | Action::Dnd(b) => *b = !*b,
            Action::Brightness(p) => *p = if up { (*p + 10).min(100) } else { p.saturating_sub(10) },
            Action::OpenApp(a) => *a = cycle(&APP_CHOICES, a.as_str(), up),
            Action::Screenshot => {}
            Action::Timer(m) => *m = if up { (*m + 1).min(180) } else { (*m - 1).max(1) },
            Action::Notify(m) => *m = cycle(&NOTE_CHOICES, m.as_str(), up),
            Action::CopyText(m) => *m = cycle(&COPY_CHOICES, m.as_str(), up),
            Action::OpenFolder(m) => *m = cycle(&FOLDER_CHOICES, m.as_str(), up),
            Action::RunScript(m) => *m = cycle(&SCRIPT_CHOICES, m.as_str(), up),
        }
    }

    pub fn adjustable(&self) -> bool {
        !matches!(self, Action::Screenshot)
    }

    fn encode(&self) -> String {
        match self {
            Action::Wifi(b) => format!("wifi:{}", *b as u8),
            Action::LowPower(b) => format!("lowpower:{}", *b as u8),
            Action::Dnd(b) => format!("dnd:{}", *b as u8),
            Action::Brightness(p) => format!("brightness:{p}"),
            Action::OpenApp(a) => format!("app:{a}"),
            Action::Screenshot => "screenshot".into(),
            Action::Timer(m) => format!("timer:{m}"),
            Action::Notify(m) => format!("notify:{m}"),
            Action::CopyText(m) => format!("copy:{m}"),
            Action::OpenFolder(m) => format!("folder:{m}"),
            Action::RunScript(m) => format!("script:{m}"),
        }
    }

    fn decode(s: &str) -> Option<Action> {
        let (k, v) = s.split_once(':').unwrap_or((s, ""));
        let flag = || match v {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        Some(match k {
            "wifi" => Action::Wifi(flag()?),
            "lowpower" => Action::LowPower(flag()?),
            "dnd" => Action::Dnd(flag()?),
            "brightness" => Action::Brightness(v.parse::<u8>().ok()?.min(100)),
            "app" if APP_CHOICES.contains(&v) => Action::OpenApp(v.into()),
            "screenshot" => Action::Screenshot,
            "timer" => Action::Timer(v.parse::<u32>().ok()?.clamp(1, 180)),
            "notify" => Action::Notify(choice(&NOTE_CHOICES, v)?),
            "copy" => Action::CopyText(choice(&COPY_CHOICES, v)?),
            "folder" => Action::OpenFolder(choice(&FOLDER_CHOICES, v)?),
            "script" => Action::RunScript(choice(&SCRIPT_CHOICES, v)?),
            _ => return None,
        })
    }
}

/// A daily time of day (local, 24-hour clock) at which a shortcut's reminder fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Daily {
    pub hour: u8,
    pub minute: u8,
}

/// The times the editor's "Daily Reminder" row offers, in order. Off comes before the first and after the last.
pub const TIMES: [Daily; 6] = [
    Daily { hour: 7, minute: 0 },
    Daily { hour: 9, minute: 0 },
    Daily { hour: 12, minute: 0 },
    Daily { hour: 18, minute: 0 },
    Daily { hour: 21, minute: 0 },
    Daily { hour: 22, minute: 30 },
];

impl Daily {
    /// `HH:MM`, as the row and the list file show it.
    pub fn label(self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// The next setting after `cur` in the row's order: Off (`None`), each time in `TIMES`, then Off again.
    pub fn after(cur: Option<Daily>) -> Option<Daily> {
        match cur {
            None => Some(TIMES[0]),
            Some(t) => TIMES.iter().position(|x| *x == t).and_then(|i| TIMES.get(i + 1)).copied(),
        }
    }

    fn parse(text: &str) -> Option<Daily> {
        let (h, m) = text.split_once(':')?;
        let (hour, minute) = (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?);
        (hour < 24 && minute < 60).then_some(Daily { hour, minute })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    /// index into the app's tile palette
    pub color: usize,
    pub actions: Vec<Action>,
    /// a daily reminder time; `None` for no reminder
    pub schedule: Option<Daily>,
}

pub fn serialize(list: &[Shortcut]) -> String {
    let mut out = String::new();
    for s in list {
        let name: String = s.name.chars().filter(|c| !c.is_control()).collect();
        let acts: Vec<String> = s.actions.iter().map(Action::encode).collect();
        out += &format!("{name}\t{}\t{}", s.color, acts.join(";"));
        if let Some(at) = s.schedule {
            out += &format!("\t{}", at.label());
        }
        out.push('\n');
    }
    out
}

pub fn parse(text: &str) -> Vec<Shortcut> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let name: String = f.next()?.chars().take(MAX_NAME).collect();
            let color = f.next()?.parse().ok()?;
            let actions = f.next().unwrap_or("").split(';').filter_map(Action::decode).take(MAX_ACTIONS).collect();
            let schedule = f.next().and_then(Daily::parse);
            (!name.trim().is_empty()).then_some(Shortcut { name, color, actions, schedule })
        })
        .take(MAX_SHORTCUTS)
        .collect()
}

pub fn load(path: &Path) -> Vec<Shortcut> {
    match std::fs::read_to_string(path) {
        Ok(t) => parse(&t),
        Err(_) => Vec::new(),
    }
}

/// Writes through a synced temp file and a rename, so a crash keeps the old list.
pub fn save(path: &Path, list: &[Shortcut]) -> std::io::Result<()> {
    crate::shell::fsutil::write_atomic(path, serialize(list))
}

/// The list in an app data directory: the saved file, or the first-launch shortcuts when there is none yet.
pub fn stored(data_dir: &Path) -> Vec<Shortcut> {
    let path = data_dir.join(FILE);
    if path.exists() {
        load(&path)
    } else {
        defaults()
    }
}

/// The Shortcuts app's data directory on this shell, where its list is saved.
pub fn app_data_dir() -> PathBuf {
    crate::shell::apphost::root().join("appdata").join(APP_ID)
}

/// The shortcuts a first launch starts with.
pub fn defaults() -> Vec<Shortcut> {
    let mk = |name: &str, color, actions| Shortcut { name: name.into(), color, actions, schedule: None };
    vec![
        mk("Bedtime", 6, vec![Action::Dnd(true), Action::Brightness(10), Action::Wifi(false)]),
        mk("Save Battery", 3, vec![Action::LowPower(true), Action::Brightness(30)]),
        mk("Quick Timer", 1, vec![Action::Timer(10)]),
        mk("Capture", 4, vec![Action::Screenshot, Action::OpenApp("Photos".into())]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("shortcuts-model-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// One of each kind, with a non-default value where the kind has one.
    fn every_kind() -> Vec<Action> {
        vec![
            Action::Wifi(true),
            Action::LowPower(false),
            Action::Dnd(true),
            Action::Brightness(70),
            Action::OpenApp("Music".into()),
            Action::Screenshot,
            Action::Timer(25),
            Action::Notify("Dinner is ready".into()),
            Action::CopyText("On my way".into()),
            Action::OpenFolder("Downloads".into()),
            Action::RunScript("df -h /".into()),
        ]
    }

    #[test]
    fn round_trips() {
        let l = defaults();
        assert_eq!(parse(&serialize(&l)), l);
    }

    #[test]
    fn every_kind_round_trips_through_the_file() {
        let l = vec![Shortcut { name: "All".into(), color: 2, actions: every_kind(), schedule: None }];
        assert_eq!(parse(&serialize(&l)), l);
    }

    #[test]
    fn a_reminder_time_round_trips_and_old_lines_have_none() {
        let mut l = defaults();
        l[1].schedule = Some(Daily { hour: 7, minute: 5 });
        let text = serialize(&l);
        assert!(text.lines().nth(1).unwrap().ends_with("\t07:05"));
        assert_eq!(parse(&text), l);
        assert_eq!(parse("A\t2\twifi:1\n")[0].schedule, None);
    }

    #[test]
    fn a_bad_reminder_time_is_dropped_not_the_shortcut() {
        let l = parse("A\t2\t\t25:00\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].schedule, None);
    }

    #[test]
    fn the_reminder_row_steps_through_its_times_then_off() {
        assert_eq!(Daily::after(None), Some(TIMES[0]));
        assert_eq!(Daily::after(Some(TIMES[0])), Some(TIMES[1]));
        assert_eq!(Daily::after(Some(TIMES[TIMES.len() - 1])), None);
        assert_eq!(TIMES[5].label(), "22:30");
    }

    #[test]
    fn junk_is_dropped() {
        let l = parse("A\t2\twifi:1;bogus;app:Nope;timer:999;notify:Hi;folder:Desktop\n\n\tx\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].actions, vec![Action::Wifi(true), Action::Timer(180), Action::OpenFolder("Desktop".into())]);
    }

    #[test]
    fn a_value_outside_its_choices_is_dropped() {
        let l = parse("A\t2\tnotify:Not a choice;copy:;script:rm -rf /;folder:Trash\n");
        assert!(l[0].actions.is_empty());
    }

    #[test]
    fn steps_clamp() {
        let mut a = Action::Brightness(100);
        a.step(true);
        assert_eq!(a, Action::Brightness(100));
        let mut t = Action::Timer(1);
        t.step(false);
        assert_eq!(t, Action::Timer(1));
    }

    #[test]
    fn every_add_menu_kind_builds_its_own_action() {
        for (i, title) in KINDS.iter().enumerate() {
            assert_eq!(Action::of_kind(i).title(), *title);
        }
        assert!(!Action::Screenshot.adjustable());
        assert!(Action::Timer(5).adjustable());
        assert!(Action::RunScript(SCRIPT_CHOICES[0].into()).adjustable());
    }

    #[test]
    fn rows_show_their_values() {
        assert_eq!(Action::Wifi(true).value(), "On");
        assert_eq!(Action::Dnd(false).value(), "Off");
        assert_eq!(Action::Brightness(50).value(), "50%");
        assert_eq!(Action::Timer(5).value(), "5 min");
        assert_eq!(Action::Screenshot.value(), "");
        assert_eq!(Action::CopyText("Thanks".into()).value(), "Thanks");
    }

    #[test]
    fn a_plan_line_is_the_title_and_then_the_value() {
        assert_eq!(Action::Dnd(true).line(), "Set Do Not Disturb: On");
        assert_eq!(Action::Screenshot.line(), "Take Screenshot");
        assert_eq!(Action::OpenFolder("Documents".into()).line(), "Open Folder: Documents");
        assert_eq!(Action::RunScript("uptime".into()).line(), "Run Shell Script: uptime");
    }

    #[test]
    fn app_choice_steps_wrap_around_the_list() {
        let mut a = Action::OpenApp("Settings".into());
        a.step(true);
        assert_eq!(a, Action::OpenApp("Notes".into()));
        a.step(false);
        assert_eq!(a, Action::OpenApp("Settings".into()));
    }

    #[test]
    fn the_new_kinds_step_through_their_choices_and_wrap() {
        let mut n = Action::Notify(NOTE_CHOICES[NOTE_CHOICES.len() - 1].into());
        n.step(true);
        assert_eq!(n, Action::Notify(NOTE_CHOICES[0].into()));
        n.step(false);
        assert_eq!(n, Action::Notify(NOTE_CHOICES[NOTE_CHOICES.len() - 1].into()));
        let mut f = Action::OpenFolder(FOLDER_CHOICES[0].into());
        f.step(false);
        assert_eq!(f, Action::OpenFolder(FOLDER_CHOICES[FOLDER_CHOICES.len() - 1].into()));
        let mut s = Action::RunScript(SCRIPT_CHOICES[1].into());
        s.step(true);
        assert_eq!(s, Action::RunScript(SCRIPT_CHOICES[2].into()));
        let mut c = Action::CopyText("Not a preset".into());
        c.step(true);
        assert_eq!(c, Action::CopyText(COPY_CHOICES[1].into()), "an unknown value counts as the first choice, so up goes to the second");
    }

    #[test]
    fn choices_never_hold_a_separator() {
        for list in [&NOTE_CHOICES[..], &COPY_CHOICES[..], &FOLDER_CHOICES[..], &SCRIPT_CHOICES[..]] {
            for c in list {
                assert!(!c.contains(';') && !c.contains('\t') && !c.contains('\n'), "{c}");
            }
        }
    }

    #[test]
    fn brightness_steps_by_ten_within_bounds() {
        let mut b = Action::Brightness(45);
        b.step(true);
        assert_eq!(b, Action::Brightness(55));
        let mut low = Action::Brightness(5);
        low.step(false);
        assert_eq!(low, Action::Brightness(0));
    }

    #[test]
    fn list_survives_a_save_and_load_on_disk() {
        let d = dir("disk");
        let path = d.join("shortcuts.tsv");
        assert!(load(&path).is_empty(), "no file yet");
        save(&path, &defaults()).unwrap();
        assert_eq!(load(&path), defaults());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn stored_is_the_defaults_until_a_list_is_saved() {
        let d = dir("stored");
        assert_eq!(stored(&d), defaults());
        save(&d.join(FILE), &[]).unwrap();
        assert!(stored(&d).is_empty(), "a saved empty list stays empty");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_app_data_directory_is_named_by_its_app_id() {
        assert!(app_data_dir().ends_with(format!("appdata/{APP_ID}")));
    }
}
