//! the terminal core chrome and panels, sharing the bifrost's settings, search and SSH models.
#[allow(unused_imports)]
use super::{
    clipboard,
    prefs::Prefs,
    termtouch,
    textfield::TextField,
    ui::{Ui, Weight, SECONDARY, SYSTEM_ORANGE, WHITE},
    hwkeys::TermKey,
    Event, Out, Shell,
};
#[allow(unused_imports)]
use crate::{
    gfx::{pack, Surface},
    scene,
    term::process,
};
#[allow(unused_imports)]
use termcore::{
    palette::THEMES,
    search::Direction,
    settings::Settings,
    sshconfig::{Config, Entry},
};
#[allow(unused_imports)]
use std::{fs, path::PathBuf};

pub mod layout;
mod command;
mod split;
mod control;
mod find;
mod groups;
mod help;
mod input;
mod launch;
mod links;
mod marks;
mod menus;
#[allow(dead_code)]
pub mod profile_table;
mod profiles;
mod selection;
mod session;
mod ssh;
mod startup;
mod tabs;
mod title;
mod view;
mod window;
#[cfg(test)]
mod tests;
pub const HEADER_PT: f32 = 40.0;
const BAR: u32 = pack(27, 27, 27, 255);
const ACTIVE: u32 = pack(17, 17, 17, 255);
const BODY: u32 = pack(20, 20, 20, 255);
const TRACK: u32 = pack(58, 58, 58, 255);
const BADGE: u32 = pack(38, 38, 38, 255);
const RED: u32 = pack(224, 90, 79, 255);
const ACCENT: u32 = pack(42, 166, 106, 255);
const ROW: f32 = 40.0;
/// Height of one line of the help page.
const HELP_PITCH: f32 = 30.0;
const MENU: [&str; 41] = [
    "Find in scrollback",
    "Smaller text",
    "Larger text",
    "Color theme",
    "Processes",
    "Reopen closed tab",
    "Duplicate tab",
    "Select all visible text",
    "Copy selection",
    "Paste",
    "Save scrollback to Files",
    "Clear scrollback",
    "Clear screen",
    "Reset terminal",
    "SSH connections",
    "Open link in selection",
    "Cursor style",
    "Blink cursor",
    "Secure keyboard entry",
    "New command",
    "Split pane",
    "Close split pane",
    "Save window group",
    "Open window group",
    "Edit tab title",
    "Close other tabs",
    "Move tab left",
    "Move tab right",
    "Mark line",
    "Previous mark",
    "Next mark",
    "Use selection for find",
    "Paste escaped text",
    "Paste selection",
    "Search the web for selection",
    "Open man page for selection",
    "Save selection to Files",
    "Allow mouse reporting",
    "Login shell",
    "Open window group at launch",
    "Terminal help",
];
/// More-menu rows added after the first 24; `terminal_menu_action` and the row details match on these.
pub(crate) mod item {
    pub const TITLE: usize = 24;
    pub const CLOSE_OTHERS: usize = 25;
    pub const MOVE_LEFT: usize = 26;
    pub const MOVE_RIGHT: usize = 27;
    pub const MARK: usize = 28;
    pub const PREVIOUS_MARK: usize = 29;
    pub const NEXT_MARK: usize = 30;
    pub const FIND_SELECTION: usize = 31;
    pub const PASTE_ESCAPED: usize = 32;
    pub const PASTE_SELECTION: usize = 33;
    pub const SEARCH_WEB: usize = 34;
    pub const MAN_PAGE: usize = 35;
    pub const SAVE_SELECTION: usize = 36;
    pub const MOUSE: usize = 37;
    pub const LOGIN_SHELL: usize = 38;
    pub const GROUP_LAUNCH: usize = 39;
    pub const HELP: usize = 40;
}
/// True for the Find line's "n of m" match count (secondary), false for "No matches" and warnings (red).
fn find_count(message: &str) -> bool {
    message.split_once(" of ").is_some_and(|(a, b)| a.parse::<usize>().is_ok() && b.parse::<usize>().is_ok())
}
/// The Term key equivalent shown beside a More-menu row.
fn menu_key(i: usize) -> Option<&'static str> {
    Some(match i {
        0 => "\u{2318}F",
        1 => "\u{2318}-",
        2 => "\u{2318}+",
        7 => "\u{2318}A",
        8 => "\u{2318}C",
        11 => "\u{2318}K",
        item::MARK => "\u{2318}U",
        item::PREVIOUS_MARK => "\u{2318}\u{2191}",
        item::NEXT_MARK => "\u{2318}\u{2193}",
        item::FIND_SELECTION => "\u{2318}E",
        _ => return None,
    })
}
const LABELS: [&str; 6] = ["Name", "Host", "User", "Port", "Identity file", "Jump host"];
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    None,
    Find,
    Connections,
    Form,
    Processes,
    More,
    /// New command: a line typed here runs in a new tab that closes when the command ends
    Command,
    /// Edit tab title: a name for the active tab
    Title,
    /// Login shell: the program new tabs run
    Shell,
    /// Terminal help: the keys and gestures
    Help,
}
pub struct TerminalUi {
    pub settings: Settings,
    panel: Panel,
    query: Option<TextField>,
    fields: Vec<TextField>,
    focused: usize,
    editing: Option<String>,
    config: Config,
    config_loaded: bool,
    pub message: String,
    scroll: f32,
    down: Option<(f32, f32)>,
    last_y: f32,
    dragged: bool,
    processes: Vec<process::Process>,
    cpu: Vec<f64>,
    sampled: f64,
    selected: Option<(u32, u64)>,
    cpu_sort: bool,
    telemetry_at: f64,
    app_memory: u64,
    system_used: Option<f32>,
    restore_keyboard: bool,
    pending_signal: Option<bool>,
    delete_confirm: bool,
    /// a tab whose job is running and whose close button was tapped once
    pending_close: Option<usize>,
    /// the profile's caret style and whether it blinks (More menu rows; saved in prefs)
    pub cursor: profile_table::CursorStyle,
    pub blink: bool,
    /// Secure Keyboard Entry: while the Terminal is in front, the shell's own shortcuts do not get its keys
    pub secure: bool,
    /// the command line of the New command panel
    command: Option<TextField>,
    /// the line of the Edit tab title and Login shell panels (one is open at a time)
    entry: Option<TextField>,
    /// programs that ask for mouse reports get them (More menu row; saved in prefs)
    pub mouse_reporting: bool,
    /// the shell new tabs run; None is the account's login shell
    pub login_shell: Option<String>,
    /// the saved window group opens when the Terminal starts
    pub group_launch: bool,
    /// Close other tabs named running programs and waits for a second choice
    confirm_others: bool,
}
impl Default for TerminalUi {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            panel: Panel::None,
            query: None,
            fields: vec![],
            focused: 0,
            editing: None,
            config: Config::parse(""),
            config_loaded: true,
            message: String::new(),
            scroll: 0.0,
            down: None,
            last_y: 0.0,
            dragged: false,
            processes: vec![],
            cpu: vec![],
            sampled: 0.0,
            selected: None,
            cpu_sort: false,
            telemetry_at: -2000.0,
            app_memory: 0,
            system_used: None,
            restore_keyboard: true,
            pending_signal: None,
            delete_confirm: false,
            pending_close: None,
            cursor: profile_table::CursorStyle::Block,
            blink: false,
            secure: false,
            command: None,
            entry: None,
            mouse_reporting: true,
            login_shell: None,
            group_launch: false,
            confirm_others: false,
        }
    }
}
impl TerminalUi {
    pub fn search_height(&self) -> f32 {
        if self.panel == Panel::Find {
            ROW
        } else {
            0.0
        }
    }
    pub fn load(&mut self, prefs: &Prefs) {
        self.settings = Settings::parse(&format!(
            "theme={}\nzoom={}\n",
            prefs.get("terminal.theme").unwrap_or("0"),
            prefs.get("terminal.zoom").unwrap_or("4")
        ));
        self.cursor = profile_table::CursorStyle::from_key(prefs.get("terminal.cursor"));
        self.blink = prefs.get("terminal.blink") == Some("1");
        self.secure = prefs.get("terminal.secure_input") == Some("1");
        self.mouse_reporting = prefs.get("terminal.mouse") != Some("0");
        self.login_shell = prefs.get(startup::SHELL_KEY).filter(|path| !path.is_empty()).map(str::to_string);
        self.group_launch = prefs.get(startup::LAUNCH_KEY) == Some("1");
    }
    pub fn next_wake(&self) -> Option<f64> {
        Some(if self.panel == Panel::Processes {
            (self.sampled + 2000.0).min(self.telemetry_at + 2000.0)
        } else {
            self.telemetry_at + 2000.0
        })
    }
    pub fn tick(&mut self, t: f64) -> bool {
        let mut changed = false;
        if t >= self.telemetry_at + 2000.0 {
            self.telemetry_at = t;
            let kb = |text: &str, key: &str| -> Option<u64> {
                text.lines()
                    .find_map(|l| l.strip_prefix(key)?.split_whitespace().next()?.parse().ok())
            };
            let rss = fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| kb(&s, "VmRSS:"))
                .unwrap_or(0)
                * 1024;
            let used = fs::read_to_string("/proc/meminfo").ok().and_then(|s| {
                let total = kb(&s, "MemTotal:")?;
                let free = kb(&s, "MemAvailable:")?;
                (total > 0).then_some((1.0 - free as f32 / total as f32).clamp(0.0, 1.0))
            });
            changed |= self.app_memory != rss || self.system_used != used;
            self.app_memory = rss;
            self.system_used = used;
        }
        if self.panel == Panel::Processes && t >= self.sampled + 2000.0 {
            self.sample(t);
            changed = true;
        }
        changed
    }
    fn sample(&mut self, t: f64) {
        let now = process::snapshot();
        let dt = (t - self.sampled) / 1000.0;
        self.cpu = now
            .iter()
            .map(|n| {
                self.processes
                    .iter()
                    .find(|p| p.info.pid == n.info.pid && p.start == n.start)
                    .filter(|_| dt > 0.0)
                    .map_or(0.0, |p| {
                        n.info.cpu_ns.saturating_sub(p.info.cpu_ns) as f64 / (dt * 1e9) * 100.0
                    })
            })
            .collect();
        self.processes = now;
        self.sampled = t;
    }
    fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.processes.len()).collect();
        if self.cpu_sort {
            order.sort_by(|&a, &b| self.cpu[b].total_cmp(&self.cpu[a]));
        }
        order
    }
    fn form(&mut self, ui: &Ui, entry: Entry, editing: bool) {
        self.editing = editing.then(|| entry.alias.clone());
        self.delete_confirm = false;
        let values = [
            entry.alias,
            entry.hostname,
            entry.user,
            entry.port,
            entry.identity_file,
            entry.proxy_jump,
        ];
        self.fields = values
            .iter()
            .zip(LABELS)
            .map(|(value, label)| {
                let mut field = TextField::new(ui, label, 14.0, false, 512);
                field.set_text(ui, value);
                field.focused = false;
                field
            })
            .collect();
        self.focused = 0;
        self.fields[0].focused = true;
        self.panel = Panel::Form;
        self.message.clear();
        self.scroll = 0.0;
    }
    fn entry(&self) -> Entry {
        let get = |i: usize| self.fields[i].text().trim().to_string();
        Entry {
            alias: get(0),
            hostname: get(1),
            user: get(2),
            port: get(3),
            identity_file: get(4),
            proxy_jump: get(5),
        }
    }
}
fn text(s: &mut Surface, ui: &Ui, value: &str, x: f32, y: f32, width: f32, color: u32) {
    let label = ui.fit(value, 13.0, width.max(0.0));
    let image = ui.text(&label, 13.0, Weight::Regular, color);
    s.blit(
        &image,
        x.round() as i32,
        (y + (ui.pt(ROW) - image.h as f32) / 2.0).round() as i32,
        255,
        255,
    );
}
fn button(s: &mut Surface, ui: &Ui, label: &str, rect: (f32, f32, f32, f32), selected: bool) {
    let (x, y, w, h) = rect;
    s.fill_rrect(x, y, w, h, 0.0, if selected { ACTIVE } else { BAR });
    if selected {
        s.fill_rrect(x, y + h - ui.pt(2.0), w, ui.pt(2.0), 0.0, ACCENT);
    }
    let img = ui.text(
        label,
        13.0,
        Weight::Medium,
        if selected { ACCENT } else { WHITE },
    );
    s.blit(
        &img,
        (x + (w - img.w as f32) / 2.0).round() as i32,
        (y + (h - img.h as f32) / 2.0).round() as i32,
        255,
        255,
    );
}
