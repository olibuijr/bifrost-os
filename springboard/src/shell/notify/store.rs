//! The notification store: the last `KEEP` notifications, newest last,
//! persisted as `notifications.ini` (one `[id]` section per notification,
//! written through a temporary file and a rename like `prefs.rs`).

use super::super::{AppId, apps};
use super::schedule::{format_minutes, parse_minutes, Schedule};
use crate::shell::prefs::write_atomic;
use std::path::PathBuf;

/// How many notifications survive; older ones are evicted.
pub const KEEP: usize = 50;

/// One notification, as an app posted it.
#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub app: AppId,
    pub title: String,
    pub body: String,
    /// Unix seconds when it was posted
    pub time: i64,
    /// assigned by the store on `push`; 0 before
    pub id: u64,
}

/// Which part of the file the parser is in.
#[derive(Clone, Copy, PartialEq)]
enum Section {
    Item,
    Mute,
    Dnd,
}

impl Notification {
    pub fn new(app: AppId, title: impl Into<String>, body: impl Into<String>, time: i64) -> Self {
        Notification { app, title: title.into(), body: body.into(), time, id: 0 }
    }
}

/// An app's notifications, newest first.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub app: AppId,
    pub items: Vec<Notification>,
}

impl Group {
    /// The newest notification of the group.
    pub fn latest(&self) -> &Notification {
        &self.items[0]
    }
}

/// The persistence key of an app: its bundle id, which `apps()` keeps unique.
fn app_key(app: AppId) -> &'static str {
    apps().iter().find(|a| a.id == app).map_or("", |a| a.icon)
}

fn app_from_key(key: &str) -> Option<AppId> {
    apps().iter().find(|a| a.icon == key).map(|a| a.id)
}

#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    /// oldest first
    items: Vec<Notification>,
    next_id: u64,
    /// apps whose notifications are stored and badged but never shown or woken for
    muted: Vec<AppId>,
    /// the Do Not Disturb schedule
    schedule: Schedule,
}

impl Default for Store {
    fn default() -> Self {
        Store { path: None, items: Vec::new(), next_id: 1, muted: Vec::new(), schedule: Schedule::default() }
    }
}

impl Store {
    /// Loads `path` (a missing or unreadable file is empty) and saves there from now on.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let mut store = Store::parse(&std::fs::read_to_string(&path).unwrap_or_default());
        store.path = Some(path);
        store
    }

    /// A store that never touches the disk.
    pub fn memory() -> Self {
        Store::default()
    }

    fn parse(text: &str) -> Store {
        fn finish(n: Option<Notification>, items: &mut Vec<Notification>) {
            if let Some(n) = n.filter(|n| n.id > 0) {
                items.push(n);
            }
        }
        let mut items: Vec<Notification> = Vec::new();
        let mut muted: Vec<AppId> = Vec::new();
        let mut schedule = Schedule::default();
        let mut current: Option<Notification> = None;
        let mut section = Section::Item;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(id) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                finish(current.take(), &mut items);
                section = match id {
                    "mute" => Section::Mute,
                    "dnd" => Section::Dnd,
                    _ => Section::Item,
                };
                if section == Section::Item {
                    current = id.parse().ok().map(|id| Notification { app: AppId::Settings, title: String::new(), body: String::new(), time: 0, id });
                }
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match section {
                Section::Mute => {
                    if let (true, Some(app)) = (k == "app", app_from_key(v)) {
                        if !muted.contains(&app) {
                            muted.push(app);
                        }
                    }
                }
                Section::Dnd => match k {
                    "on" => schedule.enabled = v == "1",
                    "from" => schedule.start = parse_minutes(v).unwrap_or(schedule.start),
                    "to" => schedule.end = parse_minutes(v).unwrap_or(schedule.end),
                    _ => {}
                },
                Section::Item => {
                    let Some(n) = current.as_mut() else { continue };
                    match k {
                        "app" => match app_from_key(v) {
                            Some(app) => n.app = app,
                            None => n.id = 0,
                        },
                        "time" => n.time = v.parse().unwrap_or(0),
                        "title" => n.title = v.to_string(),
                        "body" => n.body = v.to_string(),
                        _ => {}
                    }
                }
            }
        }
        finish(current.take(), &mut items);
        items.sort_by_key(|n| n.id);
        items.dedup_by_key(|n| n.id);
        if items.len() > KEEP {
            items.drain(..items.len() - KEEP);
        }
        let next_id = items.last().map_or(1, |n| n.id + 1);
        Store { path: None, items, next_id, muted, schedule }
    }

    fn serialize(&self) -> String {
        let clean = |s: &str| s.replace(['\n', '\r'], " ");
        let mut out = String::from("# springboard notifications\n");
        if self.schedule != Schedule::default() {
            let sc = &self.schedule;
            out.push_str(&format!("[dnd]\non={}\nfrom={}\nto={}\n", u8::from(sc.enabled), format_minutes(sc.start), format_minutes(sc.end)));
        }
        if !self.muted.is_empty() {
            out.push_str("[mute]\n");
            for &app in &self.muted {
                out.push_str(&format!("app={}\n", app_key(app)));
            }
        }
        for n in &self.items {
            out.push_str(&format!("[{}]\napp={}\ntime={}\ntitle={}\nbody={}\n", n.id, app_key(n.app), n.time, clean(&n.title), clean(&n.body)));
        }
        out
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Err(e) = write_atomic(path, &self.serialize()) {
            eprintln!("springboard: notifications.ini: {e}");
        }
    }

    /// Adds a notification (its id is assigned here), evicting the oldest
    /// past `KEEP`, and saves. Returns the id.
    pub fn push(&mut self, mut n: Notification) -> u64 {
        n.id = self.next_id;
        self.next_id += 1;
        self.items.push(n);
        if self.items.len() > KEEP {
            self.items.drain(..self.items.len() - KEEP);
        }
        self.save();
        self.items.last().map_or(0, |n| n.id)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether `app` is muted (stored and badged, never shown or woken for).
    pub fn is_muted(&self, app: AppId) -> bool {
        self.muted.contains(&app)
    }

    /// Mutes or unmutes `app` and saves.
    pub fn set_muted(&mut self, app: AppId, muted: bool) {
        if self.is_muted(app) == muted {
            return;
        }
        if muted {
            self.muted.push(app);
        } else {
            self.muted.retain(|&a| a != app);
        }
        self.save();
    }

    pub fn schedule(&self) -> Schedule {
        self.schedule
    }

    /// Replaces the Do Not Disturb schedule and saves.
    pub fn set_schedule(&mut self, schedule: Schedule) {
        if self.schedule != schedule {
            self.schedule = schedule;
            self.save();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }

    /// Every notification of `app`.
    pub fn count(&self, app: AppId) -> usize {
        self.items.iter().filter(|n| n.app == app).count()
    }

    /// Drops one notification; true if it existed.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|n| n.id != id);
        let changed = self.items.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// Drops every notification of `app`; true if there were any.
    pub fn clear_app(&mut self, app: AppId) -> bool {
        let before = self.items.len();
        self.items.retain(|n| n.app != app);
        let changed = self.items.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// Drops everything.
    pub fn clear(&mut self) {
        if !self.items.is_empty() {
            self.items.clear();
            self.save();
        }
    }

    /// Notifications grouped per app, newest first inside each group; the
    /// group with the newest notification comes first.
    pub fn groups(&self) -> Vec<Group> {
        let mut groups: Vec<Group> = Vec::new();
        for n in self.items.iter().rev() {
            match groups.iter_mut().find(|g| g.app == n.app) {
                Some(g) => g.items.push(n.clone()),
                None => groups.push(Group { app: n.app, items: vec![n.clone()] }),
            }
        }
        groups
    }
}

/// "now", "5m ago", "3h ago" or "2d ago": how long ago `time` was at `now`.
pub fn age_text(time: i64, now: i64) -> String {
    let d = (now - time).max(0);
    if d < 60 {
        "now".into()
    } else if d < 3600 {
        format!("{}m ago", d / 60)
    } else if d < 86_400 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}d ago", d / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notify")).join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn note(app: AppId, title: &str, time: i64) -> Notification {
        Notification::new(app, title, format!("body of {title}"), time)
    }

    #[test]
    fn push_assigns_increasing_ids_and_groups_newest_first() {
        let mut s = Store::memory();
        assert!(s.is_empty());
        let a = s.push(note(AppId::Files, "Timer", 100));
        let b = s.push(note(AppId::Settings, "Low Battery", 200));
        let c = s.push(note(AppId::Files, "Alarm", 300));
        assert_eq!((a, b, c), (1, 2, 3));
        assert_eq!(s.len(), 3);
        assert_eq!(s.count(AppId::Files), 2);
        assert_eq!(s.count(AppId::Terminal), 0);
        let g = s.groups();
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].app, AppId::Files, "the group with the newest notification comes first");
        assert_eq!(g[0].items.iter().map(|n| n.title.as_str()).collect::<Vec<_>>(), ["Alarm", "Timer"]);
        assert_eq!(g[0].latest().id, 3);
        assert_eq!(g[1].app, AppId::Settings);
        assert_eq!(s.get(2).unwrap().body, "body of Low Battery");
    }

    #[test]
    fn evicts_past_keep() {
        let mut s = Store::memory();
        for i in 0..KEEP as i64 + 7 {
            s.push(note(AppId::Deildu, &format!("n{i}"), i));
        }
        assert_eq!(s.len(), KEEP);
        assert_eq!(s.get(1), None, "the oldest went first");
        assert_eq!(s.groups()[0].items[0].title, format!("n{}", KEEP + 6));
        assert_eq!(s.groups()[0].items.last().unwrap().title, "n7");
    }

    #[test]
    fn remove_and_clear() {
        let mut s = Store::memory();
        s.push(note(AppId::Files, "a", 1));
        s.push(note(AppId::Terminal, "b", 2));
        s.push(note(AppId::Files, "c", 3));
        assert!(s.remove(2));
        assert!(!s.remove(2));
        assert_eq!(s.len(), 2);
        assert!(s.clear_app(AppId::Files));
        assert!(!s.clear_app(AppId::Files));
        assert!(s.is_empty());
        s.push(note(AppId::Files, "d", 4));
        assert_eq!(s.push(note(AppId::Files, "e", 5)), 5, "ids keep counting after removals");
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn every_registered_app_round_trips_through_notification_storage() {
        let mut store = Store::memory();
        for app in apps() {
            store.push(note(app.id, app.name, 1));
        }
        let again = Store::parse(&store.serialize());
        assert_eq!(again.len(), apps().len());
        for app in apps() {
            assert_eq!(again.count(app.id), 1, "{}", app.name);
        }
    }

    #[test]
    fn persists_and_loads_back_in_order() {
        let path = scratch("notifications.ini");
        let _ = std::fs::remove_file(&path);
        let mut s = Store::load(&path);
        assert!(s.is_empty());
        s.push(Notification::new(AppId::Files, "Timer", "Time's up\nnow", 1_700_000_000));
        s.push(Notification::new(AppId::Settings, "Wi-Fi", "Joined a=b", 1_700_000_060));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[1]\napp=bifrost.files\ntime=1700000000\ntitle=Timer\nbody=Time's up now\n"), "{text}");
        let again = Store::load(&path);
        assert_eq!(again.len(), 2);
        assert_eq!(again.get(2).unwrap().body, "Joined a=b", "only the first '=' splits a line");
        assert_eq!(again.get(2).unwrap().app, AppId::Settings);
        let mut again = again;
        assert_eq!(again.push(note(AppId::Deildu, "x", 1)), 3, "ids continue after a reload");
        assert!(again.remove(1));
        let reloaded = Store::load(&path);
        assert_eq!(reloaded.len(), 2);
        assert!(!path.with_file_name("notifications.ini.tmp").exists());
    }

    #[test]
    fn parse_skips_junk_and_unknown_apps_and_caps_at_keep() {
        let mut text = String::from("# header\n[x]\napp=bifrost.files\n[7]\napp=nope.app\ntitle=gone\n[3]\napp=is.olibuijr.deildu\ntitle=ok\nstray=line\n");
        let s = Store::parse(&text);
        assert_eq!(s.len(), 1);
        assert_eq!(s.get(3).unwrap().title, "ok");
        assert_eq!(s.next_id, 4);
        for i in 0..KEEP + 5 {
            text.push_str(&format!("[{}]\napp=is.olibuijr.term\ntitle=t{i}\n", 100 + i));
        }
        let s = Store::parse(&text);
        assert_eq!(s.len(), KEEP);
        assert_eq!(s.get(3), None, "the oldest are evicted on load too");
    }

    #[test]
    fn mutes_and_the_schedule_persist_with_the_notifications() {
        let path = scratch("settings-in-store.ini");
        let _ = std::fs::remove_file(&path);
        let mut s = Store::load(&path);
        assert!(!s.is_muted(AppId::Files) && s.schedule() == Schedule::default());
        s.push(note(AppId::Files, "a", 1));
        s.set_muted(AppId::Files, true);
        s.set_muted(AppId::Files, true);
        s.set_muted(AppId::Terminal, true);
        s.set_muted(AppId::Terminal, false);
        s.set_schedule(Schedule { enabled: true, start: 23 * 60, end: 6 * 60 + 30 });
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[dnd]\non=1\nfrom=23:00\nto=06:30\n"), "{text}");
        assert_eq!(text.matches("bifrost.files").count(), 2, "one mute line and one notification: {text}");
        let again = Store::load(&path);
        assert!(again.is_muted(AppId::Files) && !again.is_muted(AppId::Terminal));
        assert_eq!(again.schedule(), Schedule { enabled: true, start: 23 * 60, end: 6 * 60 + 30 });
        assert_eq!(again.len(), 1, "the settings sections never read as notifications");
        s.clear();
        let cleared = Store::load(&path);
        assert!(cleared.is_muted(AppId::Files), "clearing notifications keeps the mutes");
    }

    #[test]
    fn junk_in_the_settings_sections_is_ignored() {
        let s = Store::parse("[mute]\napp=nope\napp=bifrost.files\napp=bifrost.files\nx=1\n[dnd]\nfrom=99:00\nto=08:15\non=1\n[2]\napp=bifrost.files\n");
        assert_eq!(s.muted, vec![AppId::Files]);
        assert_eq!(s.schedule(), Schedule { enabled: true, start: 22 * 60, end: 8 * 60 + 15 });
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn age_text_buckets() {
        assert_eq!(age_text(1000, 1000), "now");
        assert_eq!(age_text(1000, 1059), "now");
        assert_eq!(age_text(1000, 1060), "1m ago");
        assert_eq!(age_text(1000, 1000 + 59 * 60), "59m ago");
        assert_eq!(age_text(1000, 1000 + 3600), "1h ago");
        assert_eq!(age_text(1000, 1000 + 86_399), "23h ago");
        assert_eq!(age_text(1000, 1000 + 2 * 86_400), "2d ago");
        assert_eq!(age_text(2000, 1000), "now", "a clock set back never shows a negative age");
    }
}
