//! Shortcut data and the built-in action library: a name, a tile colour, a list of actions, an optional daily
//! reminder, a tile glyph and a folder, stored one shortcut per line
//! (`name TAB colour TAB action;action [TAB HH:MM [TAB glyph TAB folder]]`).
//! The Shortcuts app, its control socket and the `shortcuts` command share this model, so they read and write one file.
//! Values with a choice list (apps, messages, folders, scripts) are picked from that list, never typed, so a stored
//! value always has a known meaning. Free text (a Text step, a prompt, a URL, a file name, a variable name) is escaped
//! when stored, so it never breaks the `;` and tab separators. Steps with variables and blocks run through `flow`.

use std::path::{Path, PathBuf};

pub mod flow;
#[cfg(test)]
mod parity_app_shortcuts;

/// The file the app saves the list to, inside the app's data directory.
pub const FILE: &str = "shortcuts.tsv";
/// The app's manifest id; its data directory is `appdata/<APP_ID>` under the shell's root.
pub const APP_ID: &str = "is.olibuijr.shortcuts.desktop";
pub const MAX_SHORTCUTS: usize = 64;
pub const MAX_ACTIONS: usize = 24;
pub const MAX_NAME: usize = 40;
/// The longest free text of a step (a Text step, a prompt, a result), in characters.
pub const MAX_TEXT: usize = 60;
/// The longest variable name, in characters.
pub const MAX_VAR: usize = 20;
/// The longest URL of a "Get Contents of URL" step, in characters.
pub const MAX_URL: usize = 120;
/// The most times a Repeat block runs.
pub const MAX_REPEAT: u32 = 20;
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
/// The glyphs a tile can show, by symbol name. Index 0 is the plain Shortcuts mark and has no symbol name.
pub const GLYPHS: [&str; 21] = [
    "", "star", "house", "clock", "music.note", "photo", "film", "envelope", "phone", "calendar", "globe", "bookmark", "flag",
    "tag", "pencil", "mic", "calculator", "cloud.sun", "doc", "person", "desktopcomputer",
];
/// The highest menu key a shortcut can take: the digits 1 to 9 with Control and Command.
pub const MAX_KEY: u8 = 9;
/// The folders the sidebar lists. A shortcut's `folder` is 0 for none, or the position here plus one.
pub const FOLDERS: [&str; 3] = ["Favorites", "Home", "Work"];

/// The name of folder number `i` (1-based, as stored), or `None` for 0 and numbers past the list.
pub fn folder_name(i: usize) -> Option<&'static str> {
    i.checked_sub(1).and_then(|k| FOLDERS.get(k)).copied()
}

/// A test on the previous step's output, for an `If` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    NotEmpty,
    Empty,
    /// the output contains this text, ignoring case
    Contains(String),
    /// the output is this text, ignoring case and surrounding spaces
    Equals(String),
}

impl Cond {
    fn kind_index(&self) -> usize {
        match self {
            Cond::NotEmpty => 0,
            Cond::Empty => 1,
            Cond::Contains(_) => 2,
            Cond::Equals(_) => 3,
        }
    }

    fn with_kind(i: usize, operand: &str) -> Cond {
        match i {
            0 => Cond::NotEmpty,
            1 => Cond::Empty,
            2 => Cond::Contains(operand.into()),
            _ => Cond::Equals(operand.into()),
        }
    }

    /// The text the test compares with; `None` for the tests that compare with nothing.
    pub fn text(&self) -> Option<&str> {
        match self {
            Cond::Contains(t) | Cond::Equals(t) => Some(t),
            Cond::NotEmpty | Cond::Empty => None,
        }
    }

    /// The test as the row shows it.
    pub fn label(&self) -> String {
        match self {
            Cond::NotEmpty => "has a value".into(),
            Cond::Empty => "is empty".into(),
            Cond::Contains(t) => format!("contains \"{t}\""),
            Cond::Equals(t) => format!("is \"{t}\""),
        }
    }

    /// Steps to the next kind of test (or the previous); the text carries over, or starts as "text".
    pub fn step(&mut self, up: bool) {
        let n = 4;
        let i = self.kind_index();
        let j = if up { (i + 1) % n } else { (i + n - 1) % n };
        let operand = self.text().unwrap_or("text").to_string();
        *self = Cond::with_kind(j, &operand);
    }

    /// Whether the test holds for `output`.
    pub fn holds(&self, output: &str) -> bool {
        match self {
            Cond::NotEmpty => !output.trim().is_empty(),
            Cond::Empty => output.trim().is_empty(),
            Cond::Contains(t) => output.to_lowercase().contains(&t.to_lowercase()),
            Cond::Equals(t) => output.trim().to_lowercase() == t.trim().to_lowercase(),
        }
    }

    /// The same test with its text passed through `f` (to fill in variables).
    pub fn map_text(&self, f: impl Fn(&str) -> String) -> Cond {
        match self {
            Cond::Contains(t) => Cond::Contains(f(t)),
            Cond::Equals(t) => Cond::Equals(f(t)),
            other => other.clone(),
        }
    }

    fn encode(&self) -> String {
        match self {
            Cond::NotEmpty => "filled".into(),
            Cond::Empty => "empty".into(),
            Cond::Contains(t) => format!("contains:{}", esc(t)),
            Cond::Equals(t) => format!("equals:{}", esc(t)),
        }
    }

    fn decode(v: &str) -> Option<Cond> {
        let (k, rest) = v.split_once(':').unwrap_or((v, ""));
        Some(match k {
            "filled" => Cond::NotEmpty,
            "empty" => Cond::Empty,
            "contains" => Cond::Contains(free(rest, clean_text)?),
            "equals" => Cond::Equals(free(rest, clean_text)?),
            _ => return None,
        })
    }
}

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
    /// outputs this text; `{Name}` stands for a variable and `{Output}` for the previous output
    Text(String),
    /// asks the user this question and outputs the answer
    AskInput(String),
    /// fetches this http or https URL and outputs what it returns
    GetUrl(String),
    /// saves the previous output to a file of this name in Documents
    SaveFile(String),
    /// shows this text as the result of the shortcut
    ShowResult(String),
    /// keeps the previous output under this variable name
    SetVariable(String),
    /// runs the steps up to Otherwise (or End If) when the test holds for the previous output
    If(Cond),
    Otherwise,
    EndIf,
    /// runs the steps up to End Repeat this many times
    Repeat(u32),
    /// runs the steps up to End Repeat once for each line of the previous output, with the line as `{Item}`
    RepeatEach,
    /// ends the shortcut here; the steps after it are not run
    Stop,
    EndRepeat,
}

/// The kinds the "Add Action" list offers, in order.
pub const KINDS: [&str; 21] = [
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
    "Text",
    "Ask for Input",
    "Get Contents of URL",
    "Save File",
    "Show Result",
    "Set Variable",
    "If",
    "Repeat",
    "Repeat with Each",
    "Stop",
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

/// Escapes the characters that would break the line format: `%`, the separator `;` and line breaks.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            ';' => out.push_str("%3B"),
            '\t' => out.push_str("%09"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            c => out.push(c),
        }
    }
    out
}

/// The reverse of `esc`. A `%` that does not start a hex pair stays as it is.
fn unesc(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let byte = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(byte) = byte {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A function that cleans free text for one kind of step.
type Clean = fn(&str) -> String;

/// Unescapes `v` and cleans it with `clean`; `None` when nothing is left.
fn free(v: &str, clean: Clean) -> Option<String> {
    let c = clean(&unesc(v));
    (!c.is_empty()).then_some(c)
}

/// Free text: no control characters, at most `MAX_TEXT` characters, no spaces at the ends.
pub fn clean_text(s: &str) -> String {
    let kept: String = s.chars().filter(|c| !c.is_control()).collect();
    kept.trim().chars().take(MAX_TEXT).collect::<String>().trim_end().to_string()
}

/// A variable name: letters, digits, spaces and underscores, at most `MAX_VAR` characters, no spaces at the ends.
pub fn clean_var(s: &str) -> String {
    let kept: String = s.chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '_').collect();
    kept.trim().chars().take(MAX_VAR).collect::<String>().trim_end().to_string()
}

/// A file name for the Documents folder: no control characters, no path separators or colons, no leading dots, at most
/// `MAX_NAME` characters.
pub fn clean_file(s: &str) -> String {
    let kept: String = s.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':')).collect();
    kept.trim().trim_start_matches('.').chars().take(MAX_NAME).collect::<String>().trim_end().to_string()
}

/// A URL with a scheme: `https://` is added when the text has none. No spaces, at most `MAX_URL` characters.
pub fn clean_url(s: &str) -> String {
    let kept: String = s.chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    if kept.is_empty() {
        return kept;
    }
    let lower = kept.to_lowercase();
    let full = if lower.starts_with("http://") || lower.starts_with("https://") { kept } else { format!("https://{kept}") };
    full.chars().take(MAX_URL).collect()
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
            11 => Action::Text("Hello".into()),
            12 => Action::AskInput("What is your name?".into()),
            13 => Action::GetUrl("https://example.com".into()),
            14 => Action::SaveFile("Result.txt".into()),
            15 => Action::ShowResult("{Output}".into()),
            16 => Action::SetVariable("Name".into()),
            17 => Action::If(Cond::NotEmpty),
            18 => Action::Repeat(2),
            19 => Action::RepeatEach,
            20 => Action::Stop,
            _ => Action::Timer(5),
        }
    }

    /// The steps the "Add Action" list adds for kind `i`: one step, or a whole block (If with Otherwise and End If,
    /// Repeat and Repeat with Each with End Repeat) so the list never holds an opener without its end.
    pub fn block(i: usize) -> Vec<Action> {
        match i {
            17 => vec![Action::If(Cond::NotEmpty), Action::Otherwise, Action::EndIf],
            18 => vec![Action::Repeat(2), Action::EndRepeat],
            19 => vec![Action::RepeatEach, Action::EndRepeat],
            _ => vec![Action::of_kind(i)],
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
            Action::Text(_) => KINDS[11],
            Action::AskInput(_) => KINDS[12],
            Action::GetUrl(_) => KINDS[13],
            Action::SaveFile(_) => KINDS[14],
            Action::ShowResult(_) => KINDS[15],
            Action::SetVariable(_) => KINDS[16],
            Action::If(_) => KINDS[17],
            Action::Otherwise => "Otherwise",
            Action::EndIf => "End If",
            Action::Repeat(_) => KINDS[18],
            Action::RepeatEach => KINDS[19],
            Action::Stop => KINDS[20],
            Action::EndRepeat => "End Repeat",
        }
    }

    /// The value shown right-aligned in the row.
    pub fn value(&self) -> String {
        let onoff = |b: &bool| if *b { "On" } else { "Off" }.to_string();
        match self {
            Action::Wifi(b) | Action::LowPower(b) | Action::Dnd(b) => onoff(b),
            Action::Brightness(p) => format!("{p}%"),
            Action::OpenApp(a) => a.clone(),
            Action::Screenshot
            | Action::Otherwise
            | Action::EndIf
            | Action::EndRepeat
            | Action::RepeatEach
            | Action::Stop => String::new(),
            Action::Timer(m) => format!("{m} min"),
            Action::Notify(m)
            | Action::CopyText(m)
            | Action::OpenFolder(m)
            | Action::RunScript(m)
            | Action::Text(m)
            | Action::AskInput(m)
            | Action::GetUrl(m)
            | Action::SaveFile(m)
            | Action::ShowResult(m)
            | Action::SetVariable(m) => m.clone(),
            Action::If(c) => c.label(),
            Action::Repeat(n) => {
                if *n == 1 {
                    "1 time".to_string()
                } else {
                    format!("{n} times")
                }
            }
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
            Action::Timer(m) => *m = if up { (*m + 1).min(180) } else { (*m - 1).max(1) },
            Action::Notify(m) => *m = cycle(&NOTE_CHOICES, m.as_str(), up),
            Action::CopyText(m) => *m = cycle(&COPY_CHOICES, m.as_str(), up),
            Action::OpenFolder(m) => *m = cycle(&FOLDER_CHOICES, m.as_str(), up),
            Action::RunScript(m) => *m = cycle(&SCRIPT_CHOICES, m.as_str(), up),
            Action::If(c) => c.step(up),
            Action::Repeat(n) => *n = if up { (*n + 1).min(MAX_REPEAT) } else { n.saturating_sub(1).max(1) },
            Action::Screenshot
            | Action::Text(_)
            | Action::AskInput(_)
            | Action::GetUrl(_)
            | Action::SaveFile(_)
            | Action::ShowResult(_)
            | Action::SetVariable(_)
            | Action::Otherwise
            | Action::EndIf
            | Action::EndRepeat
            | Action::RepeatEach
            | Action::Stop => {}
        }
    }

    /// Whether the row has a stepper.
    pub fn adjustable(&self) -> bool {
        !matches!(
            self,
            Action::Screenshot
                | Action::Text(_)
                | Action::AskInput(_)
                | Action::GetUrl(_)
                | Action::SaveFile(_)
                | Action::ShowResult(_)
                | Action::SetVariable(_)
                | Action::Otherwise
                | Action::EndIf
                | Action::EndRepeat
                | Action::RepeatEach
                | Action::Stop
        )
    }

    /// The free text the user types into this step, if it has any (a Text step, a prompt, a URL, a file name, a result,
    /// a variable name, the text of an If test).
    pub fn text(&self) -> Option<&str> {
        match self {
            Action::Text(t)
            | Action::AskInput(t)
            | Action::GetUrl(t)
            | Action::SaveFile(t)
            | Action::ShowResult(t)
            | Action::SetVariable(t) => Some(t),
            Action::If(c) => c.text(),
            _ => None,
        }
    }

    /// Replaces the step's free text with `s`, cleaned for the kind of step (see `clean_text`, `clean_var`, `clean_file`,
    /// `clean_url`). False, and nothing changes, when the step has no free text or nothing is left after cleaning.
    pub fn set_text(&mut self, s: &str) -> bool {
        let (slot, clean): (&mut String, Clean) = match self {
            Action::Text(t) | Action::AskInput(t) | Action::ShowResult(t) => (t, clean_text as Clean),
            Action::GetUrl(t) => (t, clean_url as Clean),
            Action::SaveFile(t) => (t, clean_file as Clean),
            Action::SetVariable(t) => (t, clean_var as Clean),
            Action::If(Cond::Contains(t)) | Action::If(Cond::Equals(t)) => (t, clean_text as Clean),
            _ => return false,
        };
        let cleaned = clean(s);
        if cleaned.is_empty() {
            return false;
        }
        *slot = cleaned;
        true
    }

    /// The step with its free text passed through `f`, to fill in variables before it runs. Steps with fixed choices
    /// are returned as they are.
    pub fn map_text(&self, f: impl Fn(&str) -> String) -> Action {
        match self {
            Action::AskInput(t) => Action::AskInput(f(t)),
            Action::GetUrl(t) => Action::GetUrl(f(t)),
            Action::SaveFile(t) => Action::SaveFile(f(t)),
            Action::ShowResult(t) => Action::ShowResult(f(t)),
            other => other.clone(),
        }
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
            Action::Text(t) => format!("text:{}", esc(t)),
            Action::AskInput(t) => format!("ask:{}", esc(t)),
            Action::GetUrl(t) => format!("url:{}", esc(t)),
            Action::SaveFile(t) => format!("save:{}", esc(t)),
            Action::ShowResult(t) => format!("result:{}", esc(t)),
            Action::SetVariable(t) => format!("setvar:{}", esc(t)),
            Action::If(c) => format!("if:{}", c.encode()),
            Action::Otherwise => "else".into(),
            Action::EndIf => "endif".into(),
            Action::Repeat(n) => format!("repeat:{n}"),
            Action::RepeatEach => "repeatall".into(),
            Action::Stop => "stop".into(),
            Action::EndRepeat => "endrepeat".into(),
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
            "text" => Action::Text(free(v, clean_text)?),
            "ask" => Action::AskInput(free(v, clean_text)?),
            "url" => Action::GetUrl(free(v, clean_url)?),
            "save" => Action::SaveFile(free(v, clean_file)?),
            "result" => Action::ShowResult(free(v, clean_text)?),
            "setvar" => Action::SetVariable(free(v, clean_var)?),
            "if" => Action::If(Cond::decode(v)?),
            "else" => Action::Otherwise,
            "endif" => Action::EndIf,
            "repeat" => Action::Repeat(v.parse::<u32>().ok()?.clamp(1, MAX_REPEAT)),
            "repeatall" => Action::RepeatEach,
            "stop" => Action::Stop,
            "endrepeat" => Action::EndRepeat,
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

/// The lowest and highest charge, in percent, a battery automation can name.
pub const MIN_BATTERY: u8 = 5;
pub const MAX_BATTERY: u8 = 95;
/// The charge levels the editor's Automation row offers, in order, after the app openings.
pub const BATTERY_CHOICES: [u8; 4] = [10, 20, 30, 50];

/// A personal automation: a condition that runs its shortcut by itself, besides its menu key and daily reminder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Automation {
    /// runs when the app named `APP_CHOICES[i]` opens
    AppOpens(usize),
    /// runs each time the charge falls below this percent (`MIN_BATTERY` to `MAX_BATTERY`)
    BatteryBelow(u8),
}

impl Automation {
    /// The form stored in the list file: `open:N` for `APP_CHOICES[N]`, or `battery:P`.
    fn encode(self) -> String {
        match self {
            Automation::AppOpens(i) => format!("open:{i}"),
            Automation::BatteryBelow(p) => format!("battery:{p}"),
        }
    }

    /// The automation a stored form names; `None` for an unknown kind or a value out of range.
    fn decode(v: &str) -> Option<Automation> {
        let (kind, n) = v.split_once(':')?;
        let n: usize = n.parse().ok()?;
        match kind {
            "open" => (n < APP_CHOICES.len()).then_some(Automation::AppOpens(n)),
            "battery" => u8::try_from(n).ok().filter(|p| (MIN_BATTERY..=MAX_BATTERY).contains(p)).map(Automation::BatteryBelow),
            _ => None,
        }
    }
}

/// Something that can start an automation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event<'a> {
    /// the app with this name opened
    AppOpened(&'a str),
    /// the battery charge changed from `before` (`None` for the first reading) to `now`, in percent
    BatteryLevel { before: Option<u8>, now: u8 },
}

/// Whether a change in the charge crosses below `level`: the charge was at or above it and is now below it. The first
/// reading (`before` of `None`) crosses nothing.
pub fn battery_fell_below(before: Option<u8>, now: u8, level: u8) -> bool {
    before.is_some_and(|b| b >= level) && now < level
}

/// The positions in `list`, in list order, of the automations that `event` starts.
pub fn due(list: &[Shortcut], event: Event) -> Vec<usize> {
    list.iter()
        .enumerate()
        .filter(|(_, s)| match (s.automation, event) {
            (Some(Automation::AppOpens(i)), Event::AppOpened(app)) => APP_CHOICES.get(i) == Some(&app),
            (Some(Automation::BatteryBelow(level)), Event::BatteryLevel { before, now }) => battery_fell_below(before, now, level),
            _ => false,
        })
        .map(|(i, _)| i)
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    /// index into the app's tile palette
    pub color: usize,
    pub actions: Vec<Action>,
    /// a daily reminder time; `None` for no reminder
    pub schedule: Option<Daily>,
    /// index into `GLYPHS`; 0 is the plain Shortcuts mark
    pub glyph: usize,
    /// 0 for no folder, or the position in `FOLDERS` plus one
    pub folder: usize,
    /// the digit (1 to `MAX_KEY`) that runs the shortcut from the Shortcuts menu with Control and Command; 0 for none
    pub key: u8,
    /// a personal automation that runs the shortcut when its event happens; `None` for none
    pub automation: Option<Automation>,
}

impl Shortcut {
    /// A shortcut with no reminder, the plain glyph, no folder, no menu key and no automation.
    pub fn new(name: &str, color: usize, actions: Vec<Action>) -> Shortcut {
        Shortcut { name: name.into(), color, actions, schedule: None, glyph: 0, folder: 0, key: 0, automation: None }
    }
}

pub fn serialize(list: &[Shortcut]) -> String {
    let mut out = String::new();
    for s in list {
        let name: String = s.name.chars().filter(|c| !c.is_control()).collect();
        let acts: Vec<String> = s.actions.iter().map(Action::encode).collect();
        out += &format!("{name}\t{}\t{}", s.color, acts.join(";"));
        let extra = s.glyph != 0 || s.folder != 0 || s.key != 0 || s.automation.is_some();
        if s.schedule.is_some() || extra {
            out += &format!("\t{}", s.schedule.map(Daily::label).unwrap_or_default());
        }
        if extra {
            out += &format!("\t{}\t{}", s.glyph, s.folder);
        }
        // the key is written as 0 when only an automation follows it, so the automation keeps its place
        if s.key != 0 || s.automation.is_some() {
            out += &format!("\t{}", s.key);
        }
        if let Some(automation) = s.automation {
            out += &format!("\t{}", automation.encode());
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
            let glyph = f.next().and_then(|g| g.parse::<usize>().ok()).filter(|g| *g < GLYPHS.len()).unwrap_or(0);
            let folder = f.next().and_then(|g| g.parse::<usize>().ok()).filter(|g| *g <= FOLDERS.len()).unwrap_or(0);
            let key = f.next().and_then(|k| k.parse::<u8>().ok()).filter(|k| (1..=MAX_KEY).contains(k)).unwrap_or(0);
            let automation = f.next().and_then(Automation::decode);
            (!name.trim().is_empty()).then_some(Shortcut { name, color, actions, schedule, glyph, folder, key, automation })
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

/// The shortcuts a first launch starts with. The first four hold the menu keys 1 to 4.
pub fn defaults() -> Vec<Shortcut> {
    let mut list = vec![
        Shortcut::new("Bedtime", 6, vec![Action::Dnd(true), Action::Brightness(10), Action::Wifi(false)]),
        Shortcut::new("Save Battery", 3, vec![Action::LowPower(true), Action::Brightness(30)]),
        Shortcut::new("Quick Timer", 1, vec![Action::Timer(10)]),
        Shortcut::new("Capture", 4, vec![Action::Screenshot, Action::OpenApp("Photos".into())]),
    ];
    for (i, s) in list.iter_mut().enumerate() {
        s.key = i as u8 + 1;
    }
    list
}

/// Gives shortcut `i` the menu key `key` (0 for none, or 1 to `MAX_KEY`). A key another shortcut holds moves to this one,
/// so no two shortcuts share a key. Fails, and changes nothing, for a key out of range or a shortcut that is not there.
pub fn set_key(list: &mut [Shortcut], i: usize, key: u8) -> Result<(), String> {
    if i >= list.len() {
        return Err("There is no such shortcut.".into());
    }
    if key > MAX_KEY {
        return Err(format!("A menu key is 1 to {MAX_KEY}."));
    }
    if key != 0 {
        for (k, s) in list.iter_mut().enumerate() {
            if k != i && s.key == key {
                s.key = 0;
            }
        }
    }
    list[i].key = key;
    Ok(())
}

/// The next menu key after `cur` in the row's order: none, then 1 to `MAX_KEY`, then none again.
pub fn next_key(cur: u8) -> u8 {
    if cur >= MAX_KEY {
        0
    } else {
        cur + 1
    }
}

/// The automation after `cur` in the order the editor's Automation row steps through: none, each app in
/// `APP_CHOICES`, each level in `BATTERY_CHOICES`, then none again. `None` for a value the row never offers.
pub fn next_automation(cur: Option<Automation>) -> Option<Automation> {
    let order: Vec<Automation> =
        (0..APP_CHOICES.len()).map(Automation::AppOpens).chain(BATTERY_CHOICES.iter().map(|&p| Automation::BatteryBelow(p))).collect();
    match cur {
        None => order.first().copied(),
        Some(a) => order.iter().position(|&x| x == a).and_then(|k| order.get(k + 1).copied()),
    }
}

/// The text the Automation row shows for `cur`: "Off", "When Notes Opens" or "Battery Below 20%".
pub fn automation_label(cur: Option<Automation>) -> String {
    match cur {
        None => "Off".into(),
        Some(Automation::AppOpens(i)) => match APP_CHOICES.get(i) {
            Some(app) => format!("When {app} Opens"),
            None => "Off".into(),
        },
        Some(Automation::BatteryBelow(p)) => format!("Battery Below {p}%"),
    }
}

/// The position of the shortcut before (`next` false) or after (`next` true) `i` in a list of `len`, or `None` at an
/// end or for a position outside the list.
pub fn neighbour(len: usize, i: usize, next: bool) -> Option<usize> {
    if i >= len {
        return None;
    }
    if next {
        (i + 1 < len).then_some(i + 1)
    } else {
        i.checked_sub(1)
    }
}

/// The names of the variables a shortcut's steps can use, in order: `Output` (the previous output) first, then each name a
/// Set Variable step sets, once, so a picker can offer them.
pub fn variable_names(actions: &[Action]) -> Vec<String> {
    let mut names = vec!["Output".to_string()];
    for a in actions {
        if let Action::SetVariable(name) = a {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
    }
    names
}

/// A name for the list: control characters removed, no spaces at the ends, at most `MAX_NAME` characters.
pub fn clean_name(s: &str) -> String {
    let kept: String = s.chars().filter(|c| !c.is_control()).collect();
    kept.trim().chars().take(MAX_NAME).collect::<String>().trim_end().to_string()
}

/// `wanted` as a name no shortcut in `list` has (ignoring case): itself when free, otherwise with " 2", " 3" and so on.
pub fn unique_name(list: &[Shortcut], wanted: &str) -> String {
    let cleaned = clean_name(wanted);
    let base = if cleaned.is_empty() { "Shortcut".to_string() } else { cleaned };
    let taken = |n: &str| list.iter().any(|s| s.name.to_lowercase() == n.to_lowercase());
    if !taken(&base) {
        return base;
    }
    (2usize..)
        .map(|k| {
            let suffix = format!(" {k}");
            let room = MAX_NAME.saturating_sub(suffix.chars().count());
            let head: String = base.chars().take(room).collect();
            format!("{}{suffix}", head.trim_end())
        })
        .find(|n| !taken(n))
        .unwrap_or_default()
}

/// Gives shortcut `i` the name `wanted`, cleaned. Fails, and changes nothing, for an empty name or one another
/// shortcut already has.
pub fn rename(list: &mut [Shortcut], i: usize, wanted: &str) -> Result<(), String> {
    if i >= list.len() {
        return Err("There is no such shortcut.".into());
    }
    let name = clean_name(wanted);
    if name.is_empty() {
        return Err("A name needs at least one character.".into());
    }
    let lower = name.to_lowercase();
    if list.iter().enumerate().any(|(k, s)| k != i && s.name.to_lowercase() == lower) {
        return Err(format!("A shortcut named {name} already exists."));
    }
    list[i].name = name;
    Ok(())
}

/// Adds a copy of shortcut `i` right after it, named "<name> Copy" (made unique) and without the daily reminder, the
/// menu key or the automation, so the copy does not notify a second time, take a key from the original or run twice.
/// Returns the copy's position, or `None` when `i` is out of range or the list is full.
pub fn duplicate(list: &mut Vec<Shortcut>, i: usize) -> Option<usize> {
    if i >= list.len() || list.len() >= MAX_SHORTCUTS {
        return None;
    }
    let mut copy = list[i].clone();
    copy.name = unique_name(list, &format!("{} Copy", copy.name));
    copy.schedule = None;
    copy.key = 0;
    copy.automation = None;
    list.insert(i + 1, copy);
    Some(i + 1)
}

/// Moves step `k` one place up or down. Returns its new position, or `None` at either end of the list.
pub fn move_action(actions: &mut [Action], k: usize, up: bool) -> Option<usize> {
    let to = if up { k.checked_sub(1)? } else { k + 1 };
    if k >= actions.len() || to >= actions.len() {
        return None;
    }
    actions.swap(k, to);
    Some(to)
}

/// Takes step `from` out and puts it back at index `to`, the index it lands on once taken out (a drag onto the row of step
/// `to`). Only a plain step moves: an opener, an Otherwise or an end keeps its place, so the blocks keep their shape.
/// Returns the new position, or `None` when nothing moves.
pub fn move_step(actions: &mut Vec<Action>, from: usize, to: usize) -> Option<usize> {
    let plain = !matches!(
        actions.get(from)?,
        Action::If(_) | Action::Repeat(_) | Action::RepeatEach | Action::Otherwise | Action::EndIf | Action::EndRepeat
    );
    if !plain || to >= actions.len() || to == from {
        return None;
    }
    let step = actions.remove(from);
    actions.insert(to, step);
    Some(to)
}

/// Whether `s` matches a search: the text is part of its name, of one of its steps or of its folder, ignoring case.
/// An empty search matches everything.
pub fn matches(s: &Shortcut, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || s.name.to_lowercase().contains(&q)
        || s.actions.iter().any(|a| a.line().to_lowercase().contains(&q))
        || folder_name(s.folder).is_some_and(|f| f.to_lowercase().contains(&q))
}

/// The positions of the shortcuts to show: those in `folder` (`None` for all) that match `query`, in list order.
pub fn visible(list: &[Shortcut], folder: Option<usize>, query: &str) -> Vec<usize> {
    list.iter()
        .enumerate()
        .filter(|(_, s)| folder.map_or(true, |f| s.folder == f) && matches(s, query))
        .map(|(i, _)| i)
        .collect()
}

/// A ready-made shortcut from the starter gallery, with a line saying what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Starter {
    pub summary: &'static str,
    /// the gallery heading the starter is listed under: Focus, Everyday or Tools
    pub category: &'static str,
    pub shortcut: Shortcut,
}

/// The gallery headings, in the order the gallery shows them.
pub const STARTER_CATEGORIES: [&str; 3] = ["Focus", "Everyday", "Tools"];

/// The starter gallery: shortcuts to add as they are and change afterwards, grouped by category in `STARTER_CATEGORIES`.
pub fn starters() -> Vec<Starter> {
    let mk = |summary, category, name: &str, color, glyph, folder, actions| {
        let mut shortcut = Shortcut::new(name, color, actions);
        shortcut.glyph = glyph;
        shortcut.folder = folder;
        Starter { summary, category, shortcut }
    };
    vec![
        mk(
            "Silence notifications, dim the screen and time an hour.",
            "Focus",
            "Focus Hour",
            6,
            3,
            3,
            vec![Action::Dnd(true), Action::Brightness(40), Action::Timer(60)],
        ),
        mk(
            "Put a short thank-you on the clipboard.",
            "Everyday",
            "Say Thanks",
            3,
            1,
            1,
            vec![Action::CopyText("Thanks".into())],
        ),
        mk("Show the Downloads folder.", "Everyday", "Open Downloads", 5, 5, 1, vec![Action::OpenFolder("Downloads".into())]),
        mk(
            "Show how much room is left on the disk.",
            "Tools",
            "Disk Report",
            4,
            0,
            3,
            vec![Action::RunScript("df -h /".into()), Action::ShowResult("{Output}".into())],
        ),
        mk(
            "Ask a name and greet it.",
            "Everyday",
            "Greeting",
            1,
            7,
            0,
            vec![
                Action::AskInput("What is your name?".into()),
                Action::SetVariable("Name".into()),
                Action::Text("Hello, {Name}!".into()),
                Action::ShowResult("{Output}".into()),
            ],
        ),
        mk(
            "Remind to stand up three times.",
            "Focus",
            "Stretch Reminder",
            2,
            11,
            1,
            vec![Action::Repeat(3), Action::Notify("Stand up and stretch".into()), Action::EndRepeat],
        ),
        mk(
            "Fetch a page and report whether it changed.",
            "Tools",
            "Check the Site",
            0,
            10,
            3,
            vec![
                Action::GetUrl("https://example.com".into()),
                Action::If(Cond::Contains("Example".into())),
                Action::ShowResult("The site is up".into()),
                Action::Otherwise,
                Action::ShowResult("The site changed".into()),
                Action::EndIf,
            ],
        ),
        mk(
            "Turn off Wi-Fi and dim the screen for the night.",
            "Focus",
            "Wind Down",
            7,
            2,
            2,
            vec![Action::Dnd(true), Action::Brightness(10), Action::Wifi(false)],
        ),
        mk(
            "Set a lunch timer and remind when it is over.",
            "Everyday",
            "Lunch Break",
            2,
            3,
            0,
            vec![Action::Timer(45), Action::Notify("Time for a break".into())],
        ),
        mk(
            "Save power and show the battery and memory left.",
            "Tools",
            "Battery Check",
            3,
            9,
            3,
            vec![Action::LowPower(true), Action::RunScript("free -h".into()), Action::ShowResult("{Output}".into())],
        ),
        mk("Copy a sign-off for a message.", "Everyday", "Sign Off", 2, 7, 1, vec![Action::CopyText("On my way".into())]),
        mk(
            "Show the date and time from the shell.",
            "Tools",
            "Date Check",
            4,
            3,
            3,
            vec![Action::RunScript("date".into()), Action::ShowResult("{Output}".into())],
        ),
        mk(
            "Dim the screen and start a twenty-minute rest timer.",
            "Focus",
            "Eye Rest",
            8,
            11,
            2,
            vec![Action::Brightness(30), Action::Timer(20)],
        ),
        mk(
            "Turn on Do Not Disturb and open Music.",
            "Everyday",
            "Listen Mode",
            5,
            4,
            0,
            vec![Action::Dnd(true), Action::OpenApp("Music".into())],
        ),
    ]
}

/// Adds starter `n` to `list` under a name no other shortcut has. Returns its position, or `None` for an unknown starter
/// or a full list.
pub fn add_starter(list: &mut Vec<Shortcut>, n: usize) -> Option<usize> {
    if list.len() >= MAX_SHORTCUTS {
        return None;
    }
    let mut shortcut = starters().into_iter().nth(n)?.shortcut;
    shortcut.name = unique_name(list, &shortcut.name);
    list.push(shortcut);
    Some(list.len() - 1)
}

/// The first line of an exported file.
pub const EXPORT_HEADER: &str = "#bifrost-shortcuts 1";

/// `list` as the text of an export file: the header, then the shortcuts in the saved format.
pub fn export(list: &[Shortcut]) -> String {
    format!("{EXPORT_HEADER}\n{}", serialize(list))
}

/// Adds the shortcuts of an export file to `list` under names no other shortcut has, without their reminders, menu
/// keys or automations. Returns how many were added. Fails for a file that is not an export, one with no shortcuts, or a
/// full list.
pub fn import(list: &mut Vec<Shortcut>, text: &str) -> Result<usize, String> {
    if !text.starts_with(EXPORT_HEADER) {
        return Err("This is not a Shortcuts file.".into());
    }
    let incoming = parse(text);
    if incoming.is_empty() {
        return Err("The file has no shortcuts.".into());
    }
    let mut added = 0;
    for mut s in incoming {
        if list.len() >= MAX_SHORTCUTS {
            break;
        }
        s.name = unique_name(list, &s.name);
        s.schedule = None;
        s.key = 0;
        s.automation = None;
        list.push(s);
        added += 1;
    }
    if added == 0 {
        return Err("The list is full.".into());
    }
    Ok(added)
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
            Action::Text("Hello, {Name}!".into()),
            Action::AskInput("Which one?".into()),
            Action::GetUrl("https://example.com/a".into()),
            Action::SaveFile("Notes.txt".into()),
            Action::ShowResult("{Output}".into()),
            Action::SetVariable("Name".into()),
            Action::If(Cond::Contains("yes".into())),
            Action::Otherwise,
            Action::EndIf,
            Action::Repeat(4),
            Action::RepeatEach,
            Action::Stop,
            Action::EndRepeat,
        ]
    }

    fn named(name: &str) -> Shortcut {
        Shortcut::new(name, 0, Vec::new())
    }

    #[test]
    fn round_trips() {
        let l = defaults();
        assert_eq!(parse(&serialize(&l)), l);
    }

    #[test]
    fn every_kind_round_trips_through_the_file() {
        let l = vec![Shortcut::new("All", 2, every_kind())];
        assert_eq!(parse(&serialize(&l)), l);
    }

    #[test]
    fn free_text_with_separators_round_trips() {
        let a = Action::Text("a;b%c".into());
        let l = vec![Shortcut::new("X", 0, vec![a.clone(), Action::EndIf])];
        let text = serialize(&l);
        assert!(text.contains("text:a%3Bb%25c"), "{text}");
        assert_eq!(parse(&text)[0].actions[0], a);
        let c = Action::If(Cond::Equals("x;y".into()));
        assert_eq!(Action::decode(&c.encode()), Some(c));
    }

    #[test]
    fn a_reminder_time_round_trips_and_old_lines_have_none() {
        let mut l = defaults();
        l[1].schedule = Some(Daily { hour: 7, minute: 5 });
        let text = serialize(&l);
        assert!(text.lines().nth(1).unwrap().ends_with("\t07:05\t0\t0\t2"), "the menu key follows the reminder: {text}");
        assert_eq!(parse(&text), l);
        assert_eq!(parse("A\t2\twifi:1\n")[0].schedule, None);
    }

    #[test]
    fn a_glyph_and_a_folder_round_trip_with_and_without_a_reminder() {
        let mut l = defaults();
        l[0].glyph = 3;
        l[0].folder = 2;
        l[1].glyph = 12;
        l[1].schedule = Some(Daily { hour: 9, minute: 0 });
        let text = serialize(&l);
        assert!(text.lines().next().unwrap().ends_with("\t\t3\t2\t1"), "no reminder leaves its field empty: {text}");
        assert!(text.lines().nth(1).unwrap().ends_with("\t09:00\t12\t0\t2"));
        assert_eq!(parse(&text), l);
        let plain = serialize(&defaults());
        assert!(!plain.lines().next().unwrap().ends_with("\t0\t0"), "a menu key alone adds no glyph or folder as zeros at the end");
    }

    #[test]
    fn a_glyph_or_folder_outside_its_list_is_dropped() {
        let l = parse("A\t2\t\t\t99\t9\n");
        assert_eq!((l[0].glyph, l[0].folder), (0, 0));
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
    fn free_text_is_cleaned_when_read() {
        let l = parse("A\t2\ttext:;url:example.com;save:../etc/x;setvar:a-b!c;repeat:99;if:bogus\n");
        assert_eq!(
            l[0].actions,
            vec![
                Action::GetUrl("https://example.com".into()),
                Action::SaveFile("etcx".into()),
                Action::SetVariable("abc".into()),
                Action::Repeat(MAX_REPEAT),
            ],
            "an empty text and an unknown test are dropped"
        );
    }

    #[test]
    fn steps_clamp() {
        let mut a = Action::Brightness(100);
        a.step(true);
        assert_eq!(a, Action::Brightness(100));
        let mut t = Action::Timer(1);
        t.step(false);
        assert_eq!(t, Action::Timer(1));
        let mut r = Action::Repeat(1);
        r.step(false);
        assert_eq!(r, Action::Repeat(1));
        let mut top = Action::Repeat(MAX_REPEAT);
        top.step(true);
        assert_eq!(top, Action::Repeat(MAX_REPEAT));
    }

    #[test]
    fn every_add_menu_kind_builds_its_own_action() {
        for (i, title) in KINDS.iter().enumerate() {
            assert_eq!(Action::of_kind(i).title(), *title);
        }
        assert!(!Action::Screenshot.adjustable());
        assert!(Action::Timer(5).adjustable());
        assert!(Action::RunScript(SCRIPT_CHOICES[0].into()).adjustable());
        assert!(Action::If(Cond::Empty).adjustable() && Action::Repeat(2).adjustable());
        assert!(!Action::Text("x".into()).adjustable() && !Action::EndIf.adjustable());
    }

    #[test]
    fn the_add_menu_adds_whole_blocks() {
        assert_eq!(Action::block(0), vec![Action::Wifi(true)]);
        assert_eq!(Action::block(17), vec![Action::If(Cond::NotEmpty), Action::Otherwise, Action::EndIf]);
        assert_eq!(Action::block(18), vec![Action::Repeat(2), Action::EndRepeat]);
    }

    #[test]
    fn rows_show_their_values() {
        assert_eq!(Action::Wifi(true).value(), "On");
        assert_eq!(Action::Dnd(false).value(), "Off");
        assert_eq!(Action::Brightness(50).value(), "50%");
        assert_eq!(Action::Timer(5).value(), "5 min");
        assert_eq!(Action::Screenshot.value(), "");
        assert_eq!(Action::CopyText("Thanks".into()).value(), "Thanks");
        assert_eq!(Action::Repeat(1).value(), "1 time");
        assert_eq!(Action::Repeat(3).value(), "3 times");
        assert_eq!(Action::If(Cond::Contains("a".into())).value(), "contains \"a\"");
        assert_eq!(Action::EndIf.value(), "");
    }

    #[test]
    fn a_plan_line_is_the_title_and_then_the_value() {
        assert_eq!(Action::Dnd(true).line(), "Set Do Not Disturb: On");
        assert_eq!(Action::Screenshot.line(), "Take Screenshot");
        assert_eq!(Action::OpenFolder("Documents".into()).line(), "Open Folder: Documents");
        assert_eq!(Action::RunScript("uptime".into()).line(), "Run Shell Script: uptime");
        assert_eq!(Action::Text("Hi".into()).line(), "Text: Hi");
        assert_eq!(Action::EndRepeat.line(), "End Repeat");
        assert_eq!(Action::If(Cond::Empty).line(), "If: is empty");
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
    fn an_if_test_steps_through_its_kinds_and_keeps_its_text() {
        let mut c = Cond::NotEmpty;
        c.step(true);
        assert_eq!(c, Cond::Empty);
        c.step(true);
        assert_eq!(c, Cond::Contains("text".into()), "a test with text starts from \"text\"");
        c = Cond::Contains("ok".into());
        c.step(true);
        assert_eq!(c, Cond::Equals("ok".into()), "the text carries over");
        c.step(true);
        assert_eq!(c, Cond::NotEmpty, "wraps");
        c.step(false);
        assert_eq!(c, Cond::Equals("text".into()));
    }

    #[test]
    fn an_if_test_holds_by_its_kind() {
        assert!(Cond::NotEmpty.holds("x") && !Cond::NotEmpty.holds("  "));
        assert!(Cond::Empty.holds("") && Cond::Empty.holds(" \n") && !Cond::Empty.holds("a"));
        assert!(Cond::Contains("PASS".into()).holds("all pass here"));
        assert!(!Cond::Contains("fail".into()).holds("all pass here"));
        assert!(Cond::Equals("Yes".into()).holds(" yes\n"));
        assert!(!Cond::Equals("yes".into()).holds("yes please"));
    }

    #[test]
    fn free_text_is_edited_and_cleaned_per_kind() {
        let mut t = Action::Text("old".into());
        assert!(t.set_text("  new\ttext "));
        assert_eq!(t, Action::Text("newtext".into()));
        assert!(!t.set_text("   "), "an empty text is refused");
        assert_eq!(t, Action::Text("newtext".into()));
        let mut u = Action::GetUrl("https://a.example".into());
        assert!(u.set_text("b.example/x y"));
        assert_eq!(u, Action::GetUrl("https://b.example/xy".into()));
        let mut f = Action::SaveFile("a.txt".into());
        assert!(f.set_text("../b/c.txt"));
        assert_eq!(f, Action::SaveFile("bc.txt".into()), "separators and leading dots go");
        let mut v = Action::SetVariable("Name".into());
        assert!(v.set_text("my var!"));
        assert_eq!(v, Action::SetVariable("my var".into()));
        let mut i = Action::If(Cond::Contains("a".into()));
        assert!(i.set_text("bee"));
        assert_eq!(i, Action::If(Cond::Contains("bee".into())));
        let mut none = Action::If(Cond::Empty);
        assert!(!none.set_text("x"), "a test without text takes none");
        assert!(!Action::Timer(5).set_text("x"));
        assert_eq!(Action::Timer(5).text(), None);
        assert_eq!(Action::If(Cond::Equals("q".into())).text(), Some("q"));
    }

    #[test]
    fn free_text_is_capped() {
        let long = "x".repeat(200);
        let mut t = Action::Text("a".into());
        t.set_text(&long);
        assert_eq!(t.text().unwrap().chars().count(), MAX_TEXT);
        assert_eq!(clean_var(&long).chars().count(), MAX_VAR);
        assert!(clean_url(&format!("example.com/{long}")).chars().count() <= MAX_URL);
        assert_eq!(clean_file(&long).chars().count(), MAX_NAME);
    }

    #[test]
    fn text_passes_through_a_function_only_where_variables_apply() {
        let up = |s: &str| s.to_uppercase();
        assert_eq!(Action::ShowResult("a".into()).map_text(up), Action::ShowResult("A".into()));
        assert_eq!(Action::Text("a".into()).map_text(up), Action::Text("a".into()), "Text is expanded by the run itself");
        assert_eq!(Action::Timer(5).map_text(up), Action::Timer(5));
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

    #[test]
    fn a_name_is_cleaned_and_made_unique() {
        let list = vec![named("Bedtime"), named("Bedtime 2")];
        assert_eq!(unique_name(&list, "Morning"), "Morning");
        assert_eq!(unique_name(&list, "  bedtime "), "bedtime 3", "case does not make a name new");
        assert_eq!(unique_name(&list, ""), "Shortcut");
        assert_eq!(clean_name("a\tb\n"), "ab");
        let long = "n".repeat(60);
        assert_eq!(clean_name(&long).chars().count(), MAX_NAME);
        let full = vec![named(&"n".repeat(MAX_NAME))];
        let next = unique_name(&full, &"n".repeat(MAX_NAME));
        assert_eq!(next.chars().count(), MAX_NAME, "the number fits inside the length limit");
        assert!(next.ends_with(" 2"));
    }

    #[test]
    fn renaming_refuses_an_empty_or_taken_name() {
        let mut list = vec![named("One"), named("Two")];
        assert_eq!(rename(&mut list, 0, "  Uno  "), Ok(()));
        assert_eq!(list[0].name, "Uno");
        assert!(rename(&mut list, 0, "  ").is_err());
        assert!(rename(&mut list, 0, "two").unwrap_err().contains("already exists"));
        assert_eq!(list[0].name, "Uno", "a refused rename changes nothing");
        assert_eq!(rename(&mut list, 1, "TWO"), Ok(()), "a shortcut may change the case of its own name");
        assert!(rename(&mut list, 5, "x").is_err());
    }

    #[test]
    fn a_copy_follows_its_original_without_the_reminder() {
        let mut list = defaults();
        list[1].schedule = Some(TIMES[0]);
        list[1].glyph = 4;
        list[1].folder = 1;
        let at = duplicate(&mut list, 1).unwrap();
        assert_eq!(at, 2);
        assert_eq!(list.len(), 5);
        assert_eq!(list[2].name, "Save Battery Copy");
        assert_eq!(list[2].actions, list[1].actions);
        assert_eq!((list[2].glyph, list[2].folder, list[2].color), (4, 1, 3));
        assert_eq!(list[2].schedule, None);
        assert_eq!(list[1].schedule, Some(TIMES[0]), "the original keeps its reminder");
        duplicate(&mut list, 1).unwrap();
        assert_eq!(list[2].name, "Save Battery Copy 2", "a second copy gets a number");
    }

    #[test]
    fn a_full_list_or_a_bad_position_makes_no_copy() {
        let mut list: Vec<Shortcut> = (0..MAX_SHORTCUTS).map(|n| named(&format!("S{n}"))).collect();
        assert_eq!(duplicate(&mut list, 0), None);
        let mut few = defaults();
        assert_eq!(duplicate(&mut few, 9), None);
        assert_eq!(few.len(), 4);
    }

    #[test]
    fn a_step_moves_up_and_down_and_stops_at_the_ends() {
        let mut a = vec![Action::Wifi(true), Action::Screenshot, Action::Timer(5)];
        assert_eq!(move_action(&mut a, 1, true), Some(0));
        assert_eq!(a, vec![Action::Screenshot, Action::Wifi(true), Action::Timer(5)]);
        assert_eq!(move_action(&mut a, 0, true), None);
        assert_eq!(move_action(&mut a, 2, false), None);
        assert_eq!(move_action(&mut a, 1, false), Some(2));
        assert_eq!(a, vec![Action::Screenshot, Action::Timer(5), Action::Wifi(true)]);
        assert_eq!(move_action(&mut a, 7, true), None);
    }

    #[test]
    fn a_search_matches_names_steps_and_folders() {
        let mut s = Shortcut::new("Morning Routine", 0, vec![Action::OpenApp("Weather".into())]);
        s.folder = 3;
        assert!(matches(&s, ""));
        assert!(matches(&s, "  morn "));
        assert!(matches(&s, "WEATHER"), "a step matches");
        assert!(matches(&s, "work"), "the folder matches");
        assert!(!matches(&s, "evening"));
    }

    #[test]
    fn the_visible_list_filters_by_folder_and_search() {
        let mut list = defaults();
        list[0].folder = 2;
        list[2].folder = 2;
        assert_eq!(visible(&list, None, ""), vec![0, 1, 2, 3]);
        assert_eq!(visible(&list, Some(2), ""), vec![0, 2]);
        assert_eq!(visible(&list, Some(2), "timer"), vec![2]);
        assert_eq!(visible(&list, Some(1), ""), Vec::<usize>::new());
        assert_eq!(visible(&list, None, "battery"), vec![1]);
    }

    #[test]
    fn folder_numbers_name_the_folders() {
        assert_eq!(folder_name(0), None);
        assert_eq!(folder_name(1), Some("Favorites"));
        assert_eq!(folder_name(FOLDERS.len()), Some(FOLDERS[FOLDERS.len() - 1]));
        assert_eq!(folder_name(FOLDERS.len() + 1), None);
    }

    #[test]
    fn every_starter_is_valid_and_survives_the_file() {
        let all = starters();
        assert!(all.len() >= 5);
        let mut names = std::collections::HashSet::new();
        for s in &all {
            assert!(names.insert(s.shortcut.name.to_lowercase()), "{} twice", s.shortcut.name);
            assert!(s.shortcut.name.chars().count() <= MAX_NAME);
            assert!(!s.summary.is_empty());
            assert!(!s.shortcut.actions.is_empty() && s.shortcut.actions.len() <= MAX_ACTIONS);
            assert!(s.shortcut.glyph < GLYPHS.len() && s.shortcut.folder <= FOLDERS.len());
            let back = parse(&serialize(std::slice::from_ref(&s.shortcut)));
            assert_eq!(back, vec![s.shortcut.clone()], "{} changes when saved", s.shortcut.name);
        }
    }

    #[test]
    fn every_starter_has_balanced_blocks() {
        for s in starters() {
            let mut open = 0i32;
            for a in &s.shortcut.actions {
                match a {
                    Action::If(_) | Action::Repeat(_) | Action::RepeatEach => open += 1,
                    Action::EndIf | Action::EndRepeat => open -= 1,
                    _ => {}
                }
                assert!(open >= 0, "{}", s.shortcut.name);
            }
            assert_eq!(open, 0, "{}", s.shortcut.name);
        }
    }

    #[test]
    fn adding_a_starter_uses_a_free_name() {
        let mut list = vec![named("Say Thanks")];
        let at = add_starter(&mut list, 1).unwrap();
        assert_eq!(list[at].name, "Say Thanks 2");
        assert_eq!(list[at].actions, vec![Action::CopyText("Thanks".into())]);
        assert_eq!(add_starter(&mut list, 99), None);
        let mut full: Vec<Shortcut> = (0..MAX_SHORTCUTS).map(|n| named(&format!("S{n}"))).collect();
        assert_eq!(add_starter(&mut full, 0), None);
    }

    #[test]
    fn an_export_imports_into_another_list() {
        let mut src = defaults();
        src[0].schedule = Some(TIMES[1]);
        src[1].glyph = 2;
        let text = export(&src);
        assert!(text.starts_with(EXPORT_HEADER));
        let mut dst = vec![named("Bedtime")];
        assert_eq!(import(&mut dst, &text), Ok(4));
        assert_eq!(dst.len(), 5);
        assert_eq!(dst[1].name, "Bedtime 2", "a name that is taken gets a number");
        assert_eq!(dst[1].schedule, None, "a reminder does not travel with the file");
        assert_eq!(dst[2].glyph, 2);
        assert_eq!(dst[2].actions, src[1].actions);
    }

    #[test]
    fn a_file_that_is_not_an_export_is_refused() {
        let mut list = defaults();
        assert!(import(&mut list, "hello\n").is_err());
        assert!(import(&mut list, &serialize(&defaults())).is_err(), "the saved list has no header");
        assert!(import(&mut list, &format!("{EXPORT_HEADER}\n")).unwrap_err().contains("no shortcuts"));
        assert_eq!(list, defaults());
    }

    #[test]
    fn an_import_stops_at_a_full_list() {
        let mut list: Vec<Shortcut> = (0..MAX_SHORTCUTS - 1).map(|n| named(&format!("S{n}"))).collect();
        assert_eq!(import(&mut list, &export(&defaults())), Ok(1));
        assert_eq!(list.len(), MAX_SHORTCUTS);
        assert!(import(&mut list, &export(&defaults())).unwrap_err().contains("full"));
    }

    #[test]
    fn the_glyph_list_starts_with_the_plain_mark() {
        assert_eq!(GLYPHS[0], "");
        assert!(GLYPHS[1..].iter().all(|g| !g.is_empty()));
    }
}
