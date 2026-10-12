//! Shortcut data: a name, a tile colour and a list of built-in actions,
//! stored one shortcut per line (`name TAB colour TAB action;action`).

use std::path::Path;

pub const MAX_SHORTCUTS: usize = 64;
pub const MAX_ACTIONS: usize = 24;
pub const MAX_NAME: usize = 40;
pub const APP_CHOICES: [&str; 10] =
    ["Notes", "Calendar", "Reminders", "Calculator", "Weather", "Camera", "Photos", "Music", "Clock", "Settings"];

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
}

/// The kinds the "Add Action" list offers, in order.
pub const KINDS: [&str; 7] = [
    "Set Wi-Fi",
    "Set Low Power Mode",
    "Set Do Not Disturb",
    "Set Brightness",
    "Open App",
    "Take Screenshot",
    "Start Timer",
];

impl Action {
    pub fn of_kind(i: usize) -> Action {
        match i {
            0 => Action::Wifi(true),
            1 => Action::LowPower(true),
            2 => Action::Dnd(true),
            3 => Action::Brightness(50),
            4 => Action::OpenApp(APP_CHOICES[0].into()),
            5 => Action::Screenshot,
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
        }
    }

    /// One step of the row's stepper (`up` = +); on/off values flip either way.
    pub fn step(&mut self, up: bool) {
        match self {
            Action::Wifi(b) | Action::LowPower(b) | Action::Dnd(b) => *b = !*b,
            Action::Brightness(p) => *p = if up { (*p + 10).min(100) } else { p.saturating_sub(10) },
            Action::OpenApp(a) => {
                let i = APP_CHOICES.iter().position(|c| c == a).unwrap_or(0);
                let n = APP_CHOICES.len();
                *a = APP_CHOICES[if up { (i + 1) % n } else { (i + n - 1) % n }].into();
            }
            Action::Screenshot => {}
            Action::Timer(m) => *m = if up { (*m + 1).min(180) } else { (*m - 1).max(1) },
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
    springboard::shell::fsutil::write_atomic(path, serialize(list))
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

    #[test]
    fn round_trips() {
        let l = defaults();
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
        let l = parse("A\t2\twifi:1;bogus;app:Nope;timer:999\n\n\tx\n");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].actions, vec![Action::Wifi(true), Action::Timer(180)]);
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
    }

    #[test]
    fn rows_show_their_values() {
        assert_eq!(Action::Wifi(true).value(), "On");
        assert_eq!(Action::Dnd(false).value(), "Off");
        assert_eq!(Action::Brightness(50).value(), "50%");
        assert_eq!(Action::Timer(5).value(), "5 min");
        assert_eq!(Action::Screenshot.value(), "");
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
        let dir = std::env::temp_dir().join(format!("shortcuts-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shortcuts.tsv");
        assert!(load(&path).is_empty(), "no file yet");
        save(&path, &defaults()).unwrap();
        assert_eq!(load(&path), defaults());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
