//! Keyboard layouts: Icelandic and English (US). The globe key cycles the
//! enabled layouts (then Emoji); a long press lists them. Settings > General >
//! Keyboard > Keyboards enables or removes them. The choice persists in
//! `kb_layouts.ini` next to the settings file.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering::Relaxed};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Icelandic,
    English,
}

impl Layout {
    pub const ALL: [Layout; 2] = [Layout::Icelandic, Layout::English];

    pub fn name(self) -> &'static str {
        match self {
            Layout::Icelandic => "Icelandic",
            Layout::English => "English (US)",
        }
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }

    fn id(self) -> &'static str {
        match self {
            Layout::Icelandic => "is",
            Layout::English => "en_US",
        }
    }
}

fn file() -> std::path::PathBuf {
    crate::shell::apphost::root().join("kb_layouts.ini")
}
/// Bit mask of enabled layouts (Icelandic alone by default) and the active one.
static ENABLED: AtomicU8 = AtomicU8::new(1);
static ACTIVE: AtomicU8 = AtomicU8::new(0);
static LOADED: std::sync::Once = std::sync::Once::new();
/// The keyboard app has no access to the shell's data: the shell owns the choice and sends it (`apply`).
static REMOTE: AtomicBool = AtomicBool::new(false);

/// For the keyboard process: never read or write `kb_layouts.ini`; the state comes from `apply`.
pub fn use_remote_state() {
    REMOTE.store(true, Relaxed);
    LOADED.call_once(|| {});
}

/// (enabled mask, active index): what the shell sends the keyboard process.
pub fn state() -> (u8, u8) {
    load();
    (ENABLED.load(Relaxed), ACTIVE.load(Relaxed))
}

/// The keyboard process takes the shell's choice. An empty or out-of-range state is ignored.
pub fn apply(mask: u8, active: u8) {
    let mask = mask & 0b11;
    if mask == 0 || active as usize >= Layout::ALL.len() || mask & (1 << active) == 0 {
        return;
    }
    ENABLED.store(mask, Relaxed);
    ACTIVE.store(active, Relaxed);
}


const ICELANDIC: [&[&str]; 3] = [
    &["q", "w", "e", "r", "t", "y", "u", "i", "o", "p", "ð"],
    &["a", "s", "d", "f", "g", "h", "j", "k", "l", "æ", "ö"],
    &["z", "x", "c", "v", "b", "n", "m", ",", ".", "þ"],
];
const ENGLISH: [&[&str]; 3] = [
    &["q", "w", "e", "r", "t", "y", "u", "i", "o", "p"],
    &["a", "s", "d", "f", "g", "h", "j", "k", "l"],
    &["z", "x", "c", "v", "b", "n", "m", ",", "."],
];

fn load() {
    LOADED.call_once(|| {
        if cfg!(test) {
            return;
        }
        let Ok(text) = std::fs::read_to_string(file()) else { return };
        let mut mask = 0u8;
        let mut active = None;
        for line in text.lines() {
            match line.split_once('=') {
                Some(("enabled", v)) => {
                    for l in Layout::ALL {
                        if v.split(',').any(|p| p.trim() == l.id()) {
                            mask |= l.bit();
                        }
                    }
                }
                Some(("active", v)) => active = Layout::ALL.into_iter().find(|l| l.id() == v.trim()),
                _ => {}
            }
        }
        if mask != 0 {
            ENABLED.store(mask, Relaxed);
            let a = active.filter(|a| mask & a.bit() != 0).unwrap_or_else(|| Layout::ALL.into_iter().find(|l| mask & l.bit() != 0).unwrap());
            ACTIVE.store(a as u8, Relaxed);
        }
    });
}

fn save() {
    if cfg!(test) || REMOTE.load(Relaxed) {
        return;
    }
    let list: Vec<&str> = enabled().iter().map(|l| l.id()).collect();
    let text = format!("enabled={}\nactive={}\n", list.join(","), active().id());
    let tmp = format!("{}.tmp", file().display());
    if let Some(dir) = file().parent().map(|p| p.to_path_buf()) {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, file())).is_err() {
        eprintln!("springboard: kb_layouts.ini not saved");
    }
}

pub fn active() -> Layout {
    load();
    Layout::ALL[(ACTIVE.load(Relaxed) as usize).min(1)]
}

pub fn enabled() -> Vec<Layout> {
    load();
    let m = ENABLED.load(Relaxed);
    Layout::ALL.into_iter().filter(|l| m & l.bit() != 0).collect()
}

pub fn is_enabled(l: Layout) -> bool {
    enabled().contains(&l)
}

/// Settings switch: adds or removes a layout (the last one cannot go).
/// Returns the new state of the switch.
pub fn toggle(l: Layout) -> bool {
    load();
    let m = ENABLED.load(Relaxed);
    let new = if m & l.bit() != 0 { m & !l.bit() } else { m | l.bit() };
    if new == 0 {
        return true;
    }
    ENABLED.store(new, Relaxed);
    if new & active().bit() == 0 {
        ACTIVE.store(Layout::ALL.into_iter().find(|x| new & x.bit() != 0).unwrap() as u8, Relaxed);
    }
    save();
    new & l.bit() != 0
}

pub fn set_active(l: Layout) {
    load();
    if is_enabled(l) {
        ACTIVE.store(l as u8, Relaxed);
        save();
    }
}

/// The layout after the active one, or `None` when the next stop is Emoji
/// (the globe cycles the layouts, then Emoji, then round again).
pub fn next(from_emoji: bool) -> Option<Layout> {
    let on = enabled();
    if from_emoji {
        return on.first().copied();
    }
    let i = on.iter().position(|l| *l == active()).unwrap_or(0);
    match on.get(i + 1) {
        Some(l) => Some(*l),
        // without an emoji font the globe skips the emoji page and wraps round
        None if !crate::emoji::available() => on.first().copied(),
        None => None,
    }
}

pub fn letters(l: Layout) -> [&'static [&'static str]; 3] {
    match l {
        Layout::Icelandic => ICELANDIC,
        Layout::English => ENGLISH,
    }
}

/// English has no ð, þ or æ among its accents.
pub fn accent_allowed(l: Layout, c: &str) -> bool {
    l == Layout::Icelandic || !matches!(c, "ð" | "þ" | "æ")
}

/// Rows of the long-press globe list: each enabled layout, then Emoji.
pub fn menu() -> Vec<String> {
    let mut v: Vec<String> = enabled().iter().map(|l| l.name().to_string()).collect();
    if crate::emoji::available() {
        v.push("Emoji".into());
    }
    v
}

pub fn by_name(n: &str) -> Option<Layout> {
    Layout::ALL.into_iter().find(|l| l.name() == n)
}

/// Settings > General > Keyboard value: "2" style count.
pub fn summary() -> String {
    enabled().len().to_string()
}
