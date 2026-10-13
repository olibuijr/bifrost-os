//! Shortcut data and the built-in action library: a name, a tile colour and glyph, a folder, a list of actions, an
//! optional daily reminder, a menu key and an automation trigger, stored one shortcut per line
//! (`name TAB colour TAB action;action TAB HH:MM TAB folder TAB glyph TAB key TAB trigger`; trailing empty fields are
//! left out).
//! The Shortcuts app, its control socket and the `shortcuts` command share this model, so they read and write one file.
//! Values with a choice list (apps, messages, folders, scripts) are picked from that list, never typed, so a stored
//! value always has a known meaning. Free text (a Text action, a prompt, a URL, a file name, a variable name) is
//! escaped when stored, so it never contains the separators `;` or tab.
//!
//! Actions are a flat list. Control flow uses block markers: `If`, `Repeat` and `Choose from Menu` open a block, `Otherwise`
//! and `Menu Item` divide it and `End` closes it. `evaluate` runs the pure part of a list (text, variables, control flow)
//! and hands back the actions that need the shell.

use std::path::{Path, PathBuf};

/// The file the app saves the list to, inside the app's data directory.
pub const FILE: &str = "shortcuts.tsv";
/// The app's manifest id; its data directory is `appdata/<APP_ID>` under the shell's root.
pub const APP_ID: &str = "is.olibuijr.shortcuts.desktop";
pub const MAX_SHORTCUTS: usize = 64;
pub const MAX_ACTIONS: usize = 40;
pub const MAX_NAME: usize = 40;
/// The longest free text one action holds, in characters.
pub const MAX_TEXT: usize = 200;
/// The most steps one run performs, so a nested Repeat cannot run away.
pub const MAX_STEPS: usize = 2000;
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
/// The symbols a shortcut's tile can show, by name; the first is the default.
pub const GLYPHS: [&str; 10] =
    ["square.grid.2x2", "star", "house", "music.note", "photo", "envelope", "calendar", "flag", "globe", "clock"];
/// The key equivalents a shortcut can take in the app's Shortcuts menu, in the order the Details row offers them.
pub const HOTKEYS: [&str; 9] = ["cmd+1", "cmd+2", "cmd+3", "cmd+4", "cmd+5", "cmd+6", "cmd+7", "cmd+8", "cmd+9"];
/// The first line of an exported shortcut file.
pub const EXPORT_HEADER: &str = "BIFROST-SHORTCUT 1";
/// The extension of an exported shortcut file.
pub const EXPORT_EXT: &str = "shortcut";

/// What an `If` step tests, the previous step's result against the step's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cond {
    HasValue,
    IsEmpty,
    Equals,
    Contains,
}

impl Cond {
    pub const ALL: [Cond; 4] = [Cond::HasValue, Cond::IsEmpty, Cond::Equals, Cond::Contains];

    pub fn label(self) -> &'static str {
        match self {
            Cond::HasValue => "has any value",
            Cond::IsEmpty => "is empty",
            Cond::Equals => "is",
            Cond::Contains => "contains",
        }
    }

    /// Whether the condition compares with a text.
    pub fn needs_text(self) -> bool {
        matches!(self, Cond::Equals | Cond::Contains)
    }

    fn code(self) -> &'static str {
        match self {
            Cond::HasValue => "has",
            Cond::IsEmpty => "empty",
            Cond::Equals => "eq",
            Cond::Contains => "contains",
        }
    }

    fn from_code(c: &str) -> Option<Cond> {
        Cond::ALL.into_iter().find(|k| k.code() == c)
    }

    fn cycle(self, up: bool) -> Cond {
        let i = Cond::ALL.iter().position(|c| *c == self).unwrap_or(0);
        let n = Cond::ALL.len();
        Cond::ALL[if up { (i + 1) % n } else { (i + n - 1) % n }]
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
    /// a text, which may hold `{Variable}` tokens; it becomes the previous result
    Text(String),
    /// asks the person for a text with this prompt; the answer becomes the previous result
    AskInput(String),
    /// fetches this address (variables allowed)
    GetUrl(String),
    /// saves the previous result under this file name
    SaveFile(String),
    /// shows the previous result
    ShowResult,
    /// names the previous result, so later steps can use `{Name}`
    SetVariable(String),
    /// runs its block when the condition holds for the previous result (and the text, for the comparing conditions)
    If(Cond, String),
    /// divides an `If` block into its two branches
    Otherwise,
    /// runs its block this many times, 1..=20
    Repeat(u32),
    /// opens a menu with this prompt; its `MenuItem`s are the choices
    Menu(String),
    /// one choice of a menu, and the steps that run when it is chosen
    MenuItem(String),
    /// closes the block of the nearest `If`, `Repeat` or `Menu` before it
    End,
}

/// The kinds the "Add Action" list offers, in order.
pub const KINDS: [&str; 23] = [
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
    "Otherwise",
    "Repeat",
    "Choose from Menu",
    "Menu Item",
    "End",
];

/// How many of the `KINDS` are built-in system actions; the rest are text, input, web, file and scripting actions.
pub const SYSTEM_KINDS: usize = 11;

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

/// Escapes free text for storage: `%`, `;`, tab and newline become `%25`, `%3B`, `%09` and `%0A`.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '%' => out.push_str("%25"),
            ';' => out.push_str("%3B"),
            '\t' => out.push_str("%09"),
            '\n' => out.push_str("%0A"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The text `esc` made `s` from.
fn unesc(s: &str) -> String {
    s.replace("%3B", ";").replace("%09", "\t").replace("%0A", "\n").replace("%25", "%")
}

/// Free text as an action holds it: no control characters, at most `MAX_TEXT` characters.
fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect()
}

/// A file name: free text without path separators.
fn clean_file(s: &str) -> String {
    clean(s).chars().filter(|c| *c != '/' && *c != '\\').collect()
}

/// A variable name: free text without braces, at most 24 characters.
fn clean_var(s: &str) -> String {
    clean(s).chars().filter(|c| *c != '{' && *c != '}').take(24).collect()
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
            14 => Action::SaveFile("note.txt".into()),
            15 => Action::ShowResult,
            16 => Action::SetVariable("Name".into()),
            17 => Action::If(Cond::HasValue, String::new()),
            18 => Action::Otherwise,
            19 => Action::Repeat(3),
            20 => Action::Menu("Choose".into()),
            21 => Action::MenuItem("Option".into()),
            22 => Action::End,
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
            Action::Text(_) => KINDS[11],
            Action::AskInput(_) => KINDS[12],
            Action::GetUrl(_) => KINDS[13],
            Action::SaveFile(_) => KINDS[14],
            Action::ShowResult => KINDS[15],
            Action::SetVariable(_) => KINDS[16],
            Action::If(..) => KINDS[17],
            Action::Otherwise => KINDS[18],
            Action::Repeat(_) => KINDS[19],
            Action::Menu(_) => KINDS[20],
            Action::MenuItem(_) => KINDS[21],
            Action::End => KINDS[22],
        }
    }

    /// The value shown right-aligned in the row.
    pub fn value(&self) -> String {
        let onoff = |b: &bool| if *b { "On" } else { "Off" }.to_string();
        match self {
            Action::Wifi(b) | Action::LowPower(b) | Action::Dnd(b) => onoff(b),
            Action::Brightness(p) => format!("{p}%"),
            Action::OpenApp(a) => a.clone(),
            Action::Screenshot | Action::ShowResult | Action::Otherwise | Action::End => String::new(),
            Action::Timer(m) => format!("{m} min"),
            Action::Notify(m) | Action::CopyText(m) | Action::OpenFolder(m) | Action::RunScript(m) => m.clone(),
            Action::Text(t) | Action::AskInput(t) | Action::GetUrl(t) | Action::SaveFile(t) | Action::SetVariable(t) => t.clone(),
            Action::Menu(t) | Action::MenuItem(t) => t.clone(),
            Action::If(c, t) if c.needs_text() => format!("{} {t}", c.label()),
            Action::If(c, _) => c.label().to_string(),
            Action::Repeat(n) => format!("{n} times"),
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
            Action::If(c, _) => *c = c.cycle(up),
            Action::Repeat(n) => *n = if up { (*n + 1).min(20) } else { (*n - 1).max(1) },
            Action::Screenshot
            | Action::Text(_)
            | Action::AskInput(_)
            | Action::GetUrl(_)
            | Action::SaveFile(_)
            | Action::ShowResult
            | Action::SetVariable(_)
            | Action::Otherwise
            | Action::Menu(_)
            | Action::MenuItem(_)
            | Action::End => {}
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
                | Action::ShowResult
                | Action::SetVariable(_)
                | Action::Otherwise
                | Action::Menu(_)
                | Action::MenuItem(_)
                | Action::End
        )
    }

    /// The free text the row edits, when it has one (an `If` that compares edits its comparison text).
    pub fn text_value(&self) -> Option<&str> {
        match self {
            Action::Text(t)
            | Action::AskInput(t)
            | Action::GetUrl(t)
            | Action::SaveFile(t)
            | Action::SetVariable(t)
            | Action::Menu(t)
            | Action::MenuItem(t) => Some(t),
            Action::If(c, t) if c.needs_text() => Some(t),
            _ => None,
        }
    }

    /// Replaces the row's free text with `text`, cleaned for the kind. False when the kind has no free text.
    pub fn set_text(&mut self, text: &str) -> bool {
        match self {
            Action::Text(t) | Action::AskInput(t) | Action::GetUrl(t) | Action::Menu(t) | Action::MenuItem(t) => *t = clean(text),
            Action::If(c, t) if c.needs_text() => *t = clean(text),
            Action::SaveFile(t) => *t = clean_file(text),
            Action::SetVariable(t) => *t = clean_var(text),
            _ => return false,
        }
        true
    }

    /// Whether the step opens a block that an `End` closes.
    pub fn opens(&self) -> bool {
        matches!(self, Action::If(..) | Action::Repeat(_) | Action::Menu(_))
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
            Action::ShowResult => "show".into(),
            Action::SetVariable(t) => format!("setvar:{}", esc(t)),
            Action::If(c, t) => format!("if:{}|{}", c.code(), esc(t)),
            Action::Otherwise => "else".into(),
            Action::Repeat(n) => format!("repeat:{n}"),
            Action::Menu(t) => format!("menu:{}", esc(t)),
            Action::MenuItem(t) => format!("item:{}", esc(t)),
            Action::End => "end".into(),
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
            "text" => Action::Text(clean(&unesc(v))),
            "ask" => Action::AskInput(clean(&unesc(v))),
            "url" => Action::GetUrl(clean(&unesc(v))),
            "save" => Action::SaveFile(clean_file(&unesc(v))),
            "show" => Action::ShowResult,
            "setvar" => Action::SetVariable(clean_var(&unesc(v))),
            "if" => {
                let (code, text) = v.split_once('|')?;
                Action::If(Cond::from_code(code)?, clean(&unesc(text)))
            }
            "else" => Action::Otherwise,
            "repeat" => Action::Repeat(v.parse::<u32>().ok()?.clamp(1, 20)),
            "menu" => Action::Menu(clean(&unesc(v))),
            "item" => Action::MenuItem(clean(&unesc(v))),
            "end" => Action::End,
            _ => return None,
        })
    }
}

/// The index of the `End` that closes the block opened at `open` (the list's length when it is not closed).
pub fn block_end(acts: &[Action], open: usize) -> usize {
    let mut depth = 0i32;
    for (j, a) in acts.iter().enumerate().skip(open) {
        if a.opens() {
            depth += 1;
        } else if *a == Action::End {
            depth -= 1;
            if depth == 0 {
                return j;
            }
        }
    }
    acts.len()
}

/// The `Otherwise` and `Menu Item` rows that divide the block opened at `open` directly (not those of blocks inside it).
fn dividers(acts: &[Action], open: usize, end: usize) -> Vec<usize> {
    let mut depth = 0i32;
    let mut found = Vec::new();
    for j in open + 1..end.min(acts.len()) {
        let a = &acts[j];
        if a.opens() {
            depth += 1;
        } else if *a == Action::End {
            depth -= 1;
        } else if depth == 0 && matches!(a, Action::Otherwise | Action::MenuItem(_)) {
            found.push(j);
        }
    }
    found
}

/// How many blocks step `k` is inside; the editor indents the row by this. Block markers sit one level out.
pub fn depth_of(acts: &[Action], k: usize) -> usize {
    let mut depth = 0usize;
    for a in acts.iter().take(k) {
        if a.opens() {
            depth += 1;
        } else if *a == Action::End {
            depth = depth.saturating_sub(1);
        }
    }
    match acts.get(k) {
        Some(Action::End) => depth.saturating_sub(1),
        Some(Action::Otherwise | Action::MenuItem(_)) => depth.saturating_sub(1),
        _ => depth,
    }
}

/// A piece of a text: literal text or a `{Variable}` token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    Text(String),
    Var(String),
}

/// Splits `text` into literal pieces and `{Name}` tokens. A brace that does not make a token stays literal.
pub fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut lit = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find(['{', '}']) {
            Some(close) if after.as_bytes()[close] == b'}' && close > 0 => {
                lit.push_str(&rest[..open]);
                if !lit.is_empty() {
                    out.push(Token::Text(std::mem::take(&mut lit)));
                }
                out.push(Token::Var(after[..close].to_string()));
                rest = &after[close + 1..];
            }
            _ => {
                lit.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    lit.push_str(rest);
    if !lit.is_empty() {
        out.push(Token::Text(lit));
    }
    out
}

/// The name of the previous step's result in a `{...}` token.
pub const PREVIOUS: &str = "Previous";

/// The values a run holds: the previous step's result and the named variables.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub previous: String,
    pub vars: Vec<(String, String)>,
}

impl Env {
    fn get(&self, name: &str) -> String {
        if name == PREVIOUS {
            return self.previous.clone();
        }
        self.vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone()).unwrap_or_default()
    }

    fn set(&mut self, name: &str, value: &str) {
        match self.vars.iter_mut().find(|(n, _)| n == name) {
            Some(slot) => slot.1 = value.to_string(),
            None => self.vars.push((name.to_string(), value.to_string())),
        }
    }

    /// `text` with each `{Name}` token replaced by its value (an unknown name is empty).
    pub fn expand(&self, text: &str) -> String {
        tokens(text)
            .into_iter()
            .map(|t| match t {
                Token::Text(s) => s,
                Token::Var(n) => self.get(&n),
            })
            .collect()
    }
}

/// The variable names a token at step `k` can use: the previous result, then each name set by a Set Variable before it.
pub fn variables_before(acts: &[Action], k: usize) -> Vec<String> {
    let mut names = vec![PREVIOUS.to_string()];
    for a in acts.iter().take(k) {
        if let Action::SetVariable(n) = a {
            if !n.is_empty() && !names.contains(n) {
                names.push(n.clone());
            }
        }
    }
    names
}

/// A question a run asks before it starts: an Ask for Input (no options) or a Choose from Menu (its choices).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    /// the index of the step that asks
    pub at: usize,
    pub text: String,
    pub options: Vec<String>,
}

/// The questions `acts` asks, in list order.
pub fn prompts(acts: &[Action]) -> Vec<Prompt> {
    let mut out = Vec::new();
    for (i, a) in acts.iter().enumerate() {
        match a {
            Action::AskInput(t) => out.push(Prompt { at: i, text: t.clone(), options: Vec::new() }),
            Action::Menu(t) => {
                let end = block_end(acts, i);
                let options = dividers(acts, i, end)
                    .into_iter()
                    .filter_map(|j| if let Action::MenuItem(m) = &acts[j] { Some(m.clone()) } else { None })
                    .collect();
                out.push(Prompt { at: i, text: t.clone(), options });
            }
            _ => {}
        }
    }
    out
}

/// What running a list did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Run {
    /// the actions that need the shell, in the order they would run, loops unrolled
    pub effects: Vec<Action>,
    /// what each Show Result showed
    pub results: Vec<String>,
    /// the files a Save File step asks to save: name and content
    pub saved: Vec<(String, String)>,
    /// the addresses a Get Contents of URL step asks for
    pub fetched: Vec<String>,
    /// the previous result when the list ended
    pub output: String,
    /// whether the run stopped at `MAX_STEPS`
    pub truncated: bool,
}

struct Eval<'a> {
    acts: &'a [Action],
    answers: &'a [(usize, String)],
    env: Env,
    run: Run,
    steps: usize,
}

impl Eval<'_> {
    fn answer(&self, at: usize) -> Option<&str> {
        self.answers.iter().find(|(i, _)| *i == at).map(|(_, a)| a.as_str())
    }

    fn exec(&mut self, mut i: usize, end: usize) {
        let acts = self.acts;
        while i < end.min(acts.len()) {
            if self.steps >= MAX_STEPS {
                self.run.truncated = true;
                return;
            }
            self.steps += 1;
            match &acts[i] {
                Action::Text(t) => self.env.previous = self.env.expand(t),
                Action::AskInput(_) => self.env.previous = self.answer(i).unwrap_or("").to_string(),
                Action::GetUrl(u) => {
                    let url = self.env.expand(u);
                    self.run.fetched.push(url);
                }
                Action::SaveFile(n) => {
                    let name = self.env.expand(n);
                    self.run.saved.push((name, self.env.previous.clone()));
                }
                Action::ShowResult => self.run.results.push(self.env.previous.clone()),
                Action::SetVariable(n) => {
                    let value = self.env.previous.clone();
                    self.env.set(n, &value);
                }
                Action::If(c, t) => {
                    let block_end = block_end(acts, i);
                    let otherwise = dividers(acts, i, block_end).into_iter().find(|j| acts[*j] == Action::Otherwise);
                    let operand = self.env.expand(t);
                    let prev = &self.env.previous;
                    let holds = match c {
                        Cond::HasValue => !prev.is_empty(),
                        Cond::IsEmpty => prev.is_empty(),
                        Cond::Equals => *prev == operand,
                        Cond::Contains => prev.contains(&operand),
                    };
                    if holds {
                        self.exec(i + 1, otherwise.unwrap_or(block_end));
                    } else if let Some(o) = otherwise {
                        self.exec(o + 1, block_end);
                    }
                    i = block_end + 1;
                    continue;
                }
                Action::Repeat(n) => {
                    let block_end = block_end(acts, i);
                    for _ in 0..*n {
                        if self.run.truncated {
                            break;
                        }
                        self.exec(i + 1, block_end);
                    }
                    i = block_end + 1;
                    continue;
                }
                Action::Menu(_) => {
                    let block_end = block_end(acts, i);
                    let items = dividers(acts, i, block_end);
                    let picked = self.answer(i).and_then(|a| items.iter().position(|j| matches!(&acts[*j], Action::MenuItem(m) if m == a)));
                    if let Some(p) = picked.or(if items.is_empty() { None } else { Some(0) }) {
                        let from = items[p];
                        let to = items.get(p + 1).copied().unwrap_or(block_end);
                        if let Action::MenuItem(m) = &acts[from] {
                            self.env.previous = m.clone();
                        }
                        self.exec(from + 1, to);
                    }
                    i = block_end + 1;
                    continue;
                }
                Action::Otherwise | Action::MenuItem(_) | Action::End => {}
                other => self.run.effects.push(other.clone()),
            }
            i += 1;
        }
    }
}

/// Runs the pure part of `acts` (text, variables, control flow) with the answers to its `prompts` (step index and
/// answer; a missing answer is empty, and a menu with no answer takes its first choice) and returns what is left for the
/// shell to do.
pub fn evaluate(acts: &[Action], answers: &[(usize, String)]) -> Run {
    let mut e = Eval { acts, answers, env: Env::default(), run: Run::default(), steps: 0 };
    e.exec(0, acts.len());
    e.run.output = e.env.previous;
    e.run
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

/// What starts a shortcut by itself, besides the daily reminder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    #[default]
    Off,
    /// when this app (one of `APP_CHOICES`) opens
    AppOpen(String),
    /// when the battery falls below this percent, 10..=90 in steps of 10
    BatteryBelow(u8),
}

impl Trigger {
    /// The trigger the row shows after this one when tapped: Off, then App Opens, then Battery, then Off.
    pub fn next_kind(&self) -> Trigger {
        match self {
            Trigger::Off => Trigger::AppOpen(APP_CHOICES[0].into()),
            Trigger::AppOpen(_) => Trigger::BatteryBelow(20),
            Trigger::BatteryBelow(_) => Trigger::Off,
        }
    }

    /// Steps the trigger's value: the app in the list, or the battery level by ten.
    pub fn step(&mut self, up: bool) {
        match self {
            Trigger::Off => {}
            Trigger::AppOpen(a) => *a = cycle(&APP_CHOICES, a.as_str(), up),
            Trigger::BatteryBelow(p) => *p = if up { (*p + 10).min(90) } else { p.saturating_sub(10).max(10) },
        }
    }

    pub fn label(&self) -> String {
        match self {
            Trigger::Off => "Off".into(),
            Trigger::AppOpen(_) => "When App Opens".into(),
            Trigger::BatteryBelow(_) => "When Battery Is Low".into(),
        }
    }

    pub fn value(&self) -> String {
        match self {
            Trigger::Off => String::new(),
            Trigger::AppOpen(a) => a.clone(),
            Trigger::BatteryBelow(p) => format!("Below {p}%"),
        }
    }

    /// Whether the condition holds now: `battery` is the charge percent when known, `opened` the app that just opened.
    pub fn holds(&self, battery: Option<u8>, opened: Option<&str>) -> bool {
        match self {
            Trigger::Off => false,
            Trigger::AppOpen(a) => opened == Some(a.as_str()),
            Trigger::BatteryBelow(p) => battery.is_some_and(|b| b < *p),
        }
    }

    fn encode(&self) -> String {
        match self {
            Trigger::Off => String::new(),
            Trigger::AppOpen(a) => format!("app:{a}"),
            Trigger::BatteryBelow(p) => format!("battery:{p}"),
        }
    }

    fn decode(s: &str) -> Trigger {
        match s.split_once(':') {
            Some(("app", a)) if APP_CHOICES.contains(&a) => Trigger::AppOpen(a.into()),
            Some(("battery", p)) => match p.parse::<u8>() {
                Ok(p) if (10..=90).contains(&p) => Trigger::BatteryBelow(p),
                _ => Trigger::Off,
            },
            _ => Trigger::Off,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcut {
    pub name: String,
    /// index into the app's tile palette
    pub color: usize,
    pub actions: Vec<Action>,
    /// a daily reminder time; `None` for no reminder
    pub schedule: Option<Daily>,
    /// the folder it is filed in; empty for none
    pub folder: String,
    /// index into `GLYPHS`
    pub glyph: usize,
    /// a key equivalent from `HOTKEYS` that runs it from the app's Shortcuts menu; empty for none
    pub hotkey: String,
    pub trigger: Trigger,
}

impl Shortcut {
    /// A new shortcut with no actions and the defaults for everything else.
    pub fn new(name: &str, color: usize) -> Shortcut {
        Shortcut { name: name.into(), color, ..Shortcut::default() }
    }

    /// Adds the action of `kind` at the end. A block opener brings its closing `End` (a menu also two items).
    /// False, with nothing added, when the list has no room for it.
    pub fn add_kind(&mut self, kind: usize) -> bool {
        let first = Action::of_kind(kind);
        let mut add = vec![first.clone()];
        match first {
            Action::Menu(_) => add.extend([Action::MenuItem("Option 1".into()), Action::MenuItem("Option 2".into()), Action::End]),
            ref a if a.opens() => add.push(Action::End),
            _ => {}
        }
        if self.actions.len() + add.len() > MAX_ACTIONS {
            return false;
        }
        self.actions.extend(add);
        true
    }

    /// Removes step `k`. A block opener takes its `End` and its dividers with it; the steps inside stay.
    pub fn remove_action(&mut self, k: usize) {
        if k >= self.actions.len() {
            return;
        }
        let mut gone = vec![k];
        if self.actions[k].opens() {
            let end = block_end(&self.actions, k);
            gone.extend(dividers(&self.actions, k, end));
            if end < self.actions.len() {
                gone.push(end);
            }
        }
        gone.sort_unstable_by(|a, b| b.cmp(a));
        for j in gone {
            self.actions.remove(j);
        }
    }

    /// Moves step `k` one place up or down; the index it has now, or `None` when it cannot move that way.
    pub fn move_action(&mut self, k: usize, up: bool) -> Option<usize> {
        let to = if up { k.checked_sub(1)? } else { k + 1 };
        if k >= self.actions.len() || to >= self.actions.len() {
            return None;
        }
        self.actions.swap(k, to);
        Some(to)
    }

    /// Sets the name, trimmed and cut to `MAX_NAME` characters, without control characters; false for an empty name.
    pub fn set_name(&mut self, name: &str) -> bool {
        let n: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
        let n = n.trim().to_string();
        if n.is_empty() {
            return false;
        }
        self.name = n;
        true
    }

    /// Files it in `folder` (empty for none), cleaned like a name.
    pub fn set_folder(&mut self, folder: &str) {
        let f: String = folder.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
        self.folder = f.trim().to_string();
    }
}

/// `base`, or `base 2`, `base 3` ... the first that no shortcut in `list` has as its name.
pub fn unique_name(list: &[Shortcut], base: &str) -> String {
    if list.iter().all(|s| s.name != base) {
        return base.to_string();
    }
    let cut: String = base.chars().take(MAX_NAME - 4).collect();
    (2..).map(|n| format!("{cut} {n}")).find(|n| list.iter().all(|s| &s.name != n)).unwrap_or(cut)
}

/// Adds a copy of shortcut `i` after it, named "<name> copy", without its menu key. The copy's index, or `None` when
/// `i` is out of range or the list is full.
pub fn duplicate(list: &mut Vec<Shortcut>, i: usize) -> Option<usize> {
    if i >= list.len() || list.len() >= MAX_SHORTCUTS {
        return None;
    }
    let mut copy = list[i].clone();
    let stem: String = list[i].name.chars().take(MAX_NAME - 5).collect();
    copy.name = unique_name(list, &format!("{stem} copy"));
    copy.hotkey.clear();
    list.insert(i + 1, copy);
    Some(i + 1)
}

/// The key after `list[i]`'s in the order Off, then each of `HOTKEYS` no other shortcut uses, then Off again.
pub fn next_hotkey(list: &[Shortcut], i: usize) -> String {
    let Some(s) = list.get(i) else { return String::new() };
    let taken = |k: &str| list.iter().enumerate().any(|(j, o)| j != i && o.hotkey == k);
    let mut order: Vec<&str> = vec![""];
    order.extend(HOTKEYS.iter().copied().filter(|k| !taken(k) || *k == s.hotkey));
    let at = order.iter().position(|k| *k == s.hotkey).unwrap_or(0);
    order[(at + 1) % order.len()].to_string()
}

/// The folders in use with how many shortcuts each holds, sorted by name.
pub fn folders(list: &[Shortcut]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    for s in list.iter().filter(|s| !s.folder.is_empty()) {
        match out.iter_mut().find(|(f, _)| *f == s.folder) {
            Some(slot) => slot.1 += 1,
            None => out.push((s.folder.clone(), 1)),
        }
    }
    out.sort();
    out
}

/// The indexes of the shortcuts the grid shows: those in `folder` (all when `None`) whose name or folder holds `query`
/// (ignoring case; an empty query matches all).
pub fn visible(list: &[Shortcut], query: &str, folder: Option<&str>) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    list.iter()
        .enumerate()
        .filter(|(_, s)| folder.map_or(true, |f| s.folder == f))
        .filter(|(_, s)| q.is_empty() || s.name.to_lowercase().contains(&q) || s.folder.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// One shortcut as an exported file: the header line, then its line of the list file.
pub fn export_text(s: &Shortcut) -> String {
    format!("{EXPORT_HEADER}\n{}", serialize(std::slice::from_ref(s)))
}

/// The shortcuts in an exported file. Its hotkey is dropped (the importing list decides its own keys).
pub fn import_text(text: &str) -> Result<Vec<Shortcut>, String> {
    let body = text.strip_prefix(EXPORT_HEADER).and_then(|r| r.strip_prefix('\n')).ok_or("not a shortcut file")?;
    let mut list = parse(body);
    if list.is_empty() {
        return Err("the file holds no shortcut".into());
    }
    for s in &mut list {
        s.hotkey.clear();
    }
    Ok(list)
}

/// The starter shortcuts the Gallery offers.
pub fn gallery() -> Vec<Shortcut> {
    let mk = |name: &str, color, glyph, actions| Shortcut { name: name.into(), color, glyph, actions, ..Shortcut::default() };
    vec![
        mk("Morning Routine", 1, 9, vec![Action::Brightness(70), Action::Dnd(false), Action::OpenApp("Calendar".into())]),
        mk("Focus Hour", 6, 7, vec![Action::Dnd(true), Action::Notify("Stand up and stretch".into()), Action::Timer(50)]),
        mk("Say Hello", 3, 5, vec![
            Action::AskInput("What is your name?".into()),
            Action::SetVariable("Name".into()),
            Action::Text("Hello, {Name}!".into()),
            Action::ShowResult,
        ]),
        mk("Pick a Greeting", 4, 5, vec![
            Action::Menu("Greeting".into()),
            Action::MenuItem("Hello".into()),
            Action::Text("Hello there".into()),
            Action::MenuItem("Goodbye".into()),
            Action::Text("See you soon".into()),
            Action::End,
            Action::ShowResult,
        ]),
        mk("Check a Page", 5, 8, vec![
            Action::GetUrl("https://example.com".into()),
            Action::SaveFile("page.html".into()),
            Action::Notify("Shortcut finished".into()),
        ]),
        mk("Battery Care", 3, 1, vec![
            Action::Text("low".into()),
            Action::If(Cond::Equals, "low".into()),
            Action::LowPower(true),
            Action::Brightness(30),
            Action::Otherwise,
            Action::LowPower(false),
            Action::End,
        ]),
    ]
}

pub fn serialize(list: &[Shortcut]) -> String {
    let mut out = String::new();
    for s in list {
        let name: String = s.name.chars().filter(|c| !c.is_control()).collect();
        let acts: Vec<String> = s.actions.iter().map(Action::encode).collect();
        out += &format!("{name}\t{}\t{}", s.color, acts.join(";"));
        let folder: String = s.folder.chars().filter(|c| !c.is_control()).collect();
        let tail = [
            s.schedule.map(Daily::label).unwrap_or_default(),
            folder,
            if s.glyph == 0 { String::new() } else { s.glyph.to_string() },
            s.hotkey.clone(),
            s.trigger.encode(),
        ];
        let used = tail.iter().rposition(|f| !f.is_empty()).map_or(0, |p| p + 1);
        for f in tail.iter().take(used) {
            out.push('\t');
            out.push_str(f);
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
            let folder: String = f.next().unwrap_or("").chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
            let glyph = f.next().and_then(|g| g.parse::<usize>().ok()).filter(|g| *g < GLYPHS.len()).unwrap_or(0);
            let hotkey = f.next().filter(|k| HOTKEYS.contains(k)).unwrap_or("").to_string();
            let trigger = f.next().map(Trigger::decode).unwrap_or_default();
            (!name.trim().is_empty()).then_some(Shortcut { name, color, actions, schedule, folder, glyph, hotkey, trigger })
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
    let mk = |name: &str, color, actions| Shortcut { name: name.into(), color, actions, ..Shortcut::default() };
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
        let l = vec![Shortcut { name: "All".into(), color: 2, actions: every_kind(), ..Shortcut::default() }];
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

    fn acts(list: &[Action]) -> Vec<Action> {
        list.to_vec()
    }

    #[test]
    fn the_new_kinds_round_trip_with_awkward_text() {
        let a = vec![
            Action::Text("a;b%c\td".into()),
            Action::AskInput("Name?".into()),
            Action::GetUrl("https://example.com/?a=1;b=2".into()),
            Action::SaveFile("x.txt".into()),
            Action::ShowResult,
            Action::SetVariable("Who".into()),
            Action::If(Cond::Contains, "x:y|z".into()),
            Action::Otherwise,
            Action::Repeat(4),
            Action::Menu("M".into()),
            Action::MenuItem("One".into()),
            Action::End,
            Action::End,
        ];
        let l = vec![Shortcut { name: "N".into(), color: 1, actions: a, ..Shortcut::default() }];
        assert_eq!(parse(&serialize(&l)), l);
    }

    #[test]
    fn text_is_cleaned_to_its_kind() {
        let mut f = Action::SaveFile(String::new());
        assert!(f.set_text("a/b\\c.txt"));
        assert_eq!(f, Action::SaveFile("abc.txt".into()));
        let mut v = Action::SetVariable(String::new());
        v.set_text("{x}y");
        assert_eq!(v, Action::SetVariable("xy".into()));
        let mut t = Action::Text(String::new());
        t.set_text(&"a".repeat(500));
        assert_eq!(t.text_value().map(str::len), Some(MAX_TEXT));
        assert!(!Action::Wifi(true).set_text("x"));
        assert_eq!(Action::If(Cond::IsEmpty, String::new()).text_value(), None);
        assert_eq!(Action::If(Cond::Equals, "a".into()).text_value(), Some("a"));
    }

    #[test]
    fn tokens_split_text_and_variables() {
        assert_eq!(tokens("Hi {Name}!"), vec![Token::Text("Hi ".into()), Token::Var("Name".into()), Token::Text("!".into())]);
        assert_eq!(tokens("{a}{b}"), vec![Token::Var("a".into()), Token::Var("b".into())]);
        assert_eq!(tokens("a {} b { c {d"), vec![Token::Text("a {} b { c {d".into())]);
        assert_eq!(tokens(""), Vec::<Token>::new());
    }

    #[test]
    fn a_variable_carries_a_result_to_a_later_step() {
        let a = acts(&[
            Action::Text("Ada".into()),
            Action::SetVariable("Name".into()),
            Action::Text("Hello, {Name}! ({Previous}) {Missing}".into()),
            Action::ShowResult,
        ]);
        let r = evaluate(&a, &[]);
        assert_eq!(r.results, vec!["Hello, Ada! (Ada) ".to_string()]);
        assert_eq!(r.output, "Hello, Ada! (Ada) ");
        assert_eq!(variables_before(&a, 3), vec!["Previous".to_string(), "Name".to_string()]);
        assert_eq!(variables_before(&a, 1), vec!["Previous".to_string()]);
    }

    #[test]
    fn an_answer_becomes_the_previous_result() {
        let a = acts(&[Action::AskInput("Q".into()), Action::ShowResult]);
        assert_eq!(prompts(&a), vec![Prompt { at: 0, text: "Q".into(), options: vec![] }]);
        assert_eq!(evaluate(&a, &[(0, "yes".into())]).results, vec!["yes".to_string()]);
        assert_eq!(evaluate(&a, &[]).results, vec![String::new()]);
    }

    #[test]
    fn if_runs_one_branch() {
        let a = acts(&[
            Action::Text("low".into()),
            Action::If(Cond::Equals, "low".into()),
            Action::LowPower(true),
            Action::Otherwise,
            Action::LowPower(false),
            Action::End,
            Action::Screenshot,
        ]);
        assert_eq!(evaluate(&a, &[]).effects, vec![Action::LowPower(true), Action::Screenshot]);
        let mut b = a.clone();
        b[0] = Action::Text("high".into());
        assert_eq!(evaluate(&b, &[]).effects, vec![Action::LowPower(false), Action::Screenshot]);
        let mut c = a.clone();
        c[1] = Action::If(Cond::Contains, "ow".into());
        assert_eq!(evaluate(&c, &[]).effects[0], Action::LowPower(true));
        c[1] = Action::If(Cond::IsEmpty, String::new());
        assert_eq!(evaluate(&c, &[]).effects[0], Action::LowPower(false));
        c[1] = Action::If(Cond::HasValue, String::new());
        assert_eq!(evaluate(&c, &[]).effects[0], Action::LowPower(true));
    }

    #[test]
    fn repeat_unrolls_and_nests() {
        let a = acts(&[Action::Repeat(2), Action::Repeat(3), Action::Screenshot, Action::End, Action::Timer(1), Action::End]);
        let r = evaluate(&a, &[]);
        assert_eq!(r.effects.iter().filter(|e| **e == Action::Screenshot).count(), 6);
        assert_eq!(r.effects.iter().filter(|e| **e == Action::Timer(1)).count(), 2);
        assert!(!r.truncated);
    }

    #[test]
    fn a_runaway_repeat_stops() {
        let a = acts(&[Action::Repeat(20), Action::Repeat(20), Action::Repeat(20), Action::Screenshot, Action::End, Action::End, Action::End]);
        let r = evaluate(&a, &[]);
        assert!(r.truncated);
        assert!(r.effects.len() <= MAX_STEPS);
    }

    #[test]
    fn a_menu_runs_the_chosen_item_or_the_first() {
        let a = acts(&[
            Action::Menu("Pick".into()),
            Action::MenuItem("A".into()),
            Action::Text("first".into()),
            Action::MenuItem("B".into()),
            Action::Text("second".into()),
            Action::End,
            Action::ShowResult,
        ]);
        assert_eq!(prompts(&a), vec![Prompt { at: 0, text: "Pick".into(), options: vec!["A".into(), "B".into()] }]);
        assert_eq!(evaluate(&a, &[(0, "B".into())]).results, vec!["second".to_string()]);
        assert_eq!(evaluate(&a, &[(0, "nope".into())]).results, vec!["first".to_string()]);
        assert_eq!(evaluate(&a, &[]).results, vec!["first".to_string()]);
    }

    #[test]
    fn web_and_file_steps_are_reported_not_performed() {
        let a = acts(&[Action::Text("n".into()), Action::GetUrl("https://x/{Previous}".into()), Action::SaveFile("{Previous}.txt".into())]);
        let r = evaluate(&a, &[]);
        assert_eq!(r.fetched, vec!["https://x/n".to_string()]);
        assert_eq!(r.saved, vec![("n.txt".to_string(), "n".to_string())]);
        assert!(r.effects.is_empty());
    }

    #[test]
    fn an_unclosed_block_still_runs_to_the_end_of_the_list() {
        let a = acts(&[Action::Repeat(2), Action::Screenshot]);
        assert_eq!(evaluate(&a, &[]).effects.len(), 2);
        assert_eq!(block_end(&a, 0), 2);
    }

    #[test]
    fn blocks_come_and_go_with_their_end() {
        let mut s = Shortcut::new("S", 0);
        assert!(s.add_kind(17));
        assert_eq!(s.actions, vec![Action::If(Cond::HasValue, String::new()), Action::End]);
        assert!(s.add_kind(20));
        assert_eq!(s.actions.len(), 6, "a menu brings two items and its end");
        s.remove_action(2);
        assert_eq!(s.actions, vec![Action::If(Cond::HasValue, String::new()), Action::End]);
        s.remove_action(0);
        assert!(s.actions.is_empty());
        for _ in 0..30 {
            s.add_kind(17);
        }
        assert!(s.actions.len() <= MAX_ACTIONS);
    }

    #[test]
    fn rows_move_one_place_and_stop_at_the_ends() {
        let mut s = Shortcut::new("S", 0);
        s.actions = vec![Action::Screenshot, Action::Timer(2), Action::Wifi(true)];
        assert_eq!(s.move_action(0, true), None);
        assert_eq!(s.move_action(2, false), None);
        assert_eq!(s.move_action(1, true), Some(0));
        assert_eq!(s.actions, vec![Action::Timer(2), Action::Screenshot, Action::Wifi(true)]);
        assert_eq!(s.move_action(1, false), Some(2));
        assert_eq!(s.actions[2], Action::Screenshot);
        assert_eq!(s.move_action(9, true), None);
    }

    #[test]
    fn rows_inside_blocks_are_indented() {
        let a = acts(&[
            Action::If(Cond::HasValue, String::new()),
            Action::Screenshot,
            Action::Otherwise,
            Action::Timer(1),
            Action::End,
            Action::Wifi(true),
        ]);
        let d: Vec<usize> = (0..a.len()).map(|k| depth_of(&a, k)).collect();
        assert_eq!(d, vec![0, 1, 0, 1, 0, 0]);
    }

    #[test]
    fn the_condition_and_repeat_count_step() {
        let mut i = Action::If(Cond::HasValue, String::new());
        i.step(true);
        assert_eq!(i, Action::If(Cond::IsEmpty, String::new()));
        i.step(false);
        i.step(false);
        assert_eq!(i, Action::If(Cond::Contains, String::new()));
        let mut r = Action::Repeat(20);
        r.step(true);
        assert_eq!(r, Action::Repeat(20));
        let mut r = Action::Repeat(1);
        r.step(false);
        assert_eq!(r, Action::Repeat(1));
        assert!(i.adjustable() && r.adjustable() && !Action::Text(String::new()).adjustable());
    }

    #[test]
    fn extra_fields_round_trip_and_old_lines_default() {
        let l = vec![Shortcut {
            name: "X".into(),
            color: 2,
            actions: vec![Action::Screenshot],
            schedule: None,
            folder: "Work".into(),
            glyph: 3,
            hotkey: "cmd+2".into(),
            trigger: Trigger::BatteryBelow(30),
        }];
        let text = serialize(&l);
        assert_eq!(text, "X\t2\tscreenshot\t\tWork\t3\tcmd+2\tbattery:30\n");
        assert_eq!(parse(&text), l);
        let old = parse("A\t2\twifi:1\t07:00\n");
        assert_eq!((old[0].folder.as_str(), old[0].glyph, old[0].hotkey.as_str(), old[0].trigger.clone()), ("", 0, "", Trigger::Off));
    }

    #[test]
    fn bad_extra_fields_are_dropped_not_the_shortcut() {
        let l = parse("A\t1\t\t\tF\t99\tcmd+q\tbattery:5\n");
        assert_eq!(l.len(), 1);
        assert_eq!((l[0].glyph, l[0].hotkey.as_str(), l[0].trigger.clone()), (0, "", Trigger::Off));
        assert_eq!(parse("A\t1\t\t\t\t\t\tapp:Nope\n")[0].trigger, Trigger::Off);
        assert_eq!(parse("A\t1\t\t\t\t\t\tapp:Music\n")[0].trigger, Trigger::AppOpen("Music".into()));
    }

    #[test]
    fn triggers_step_and_hold() {
        let t = Trigger::Off.next_kind();
        assert_eq!(t, Trigger::AppOpen("Notes".into()));
        assert_eq!(t.next_kind(), Trigger::BatteryBelow(20));
        assert_eq!(t.next_kind().next_kind(), Trigger::Off);
        let mut b = Trigger::BatteryBelow(90);
        b.step(true);
        assert_eq!(b, Trigger::BatteryBelow(90));
        let mut b = Trigger::BatteryBelow(10);
        b.step(false);
        assert_eq!(b, Trigger::BatteryBelow(10));
        assert!(Trigger::BatteryBelow(20).holds(Some(19), None));
        assert!(!Trigger::BatteryBelow(20).holds(Some(20), None));
        assert!(!Trigger::BatteryBelow(20).holds(None, None));
        assert!(Trigger::AppOpen("Music".into()).holds(None, Some("Music")));
        assert!(!Trigger::AppOpen("Music".into()).holds(None, Some("Notes")));
        assert!(!Trigger::Off.holds(Some(1), Some("Music")));
    }

    #[test]
    fn names_stay_unique_and_copies_keep_everything_but_the_key() {
        let mut l = defaults();
        assert_eq!(unique_name(&l, "Fresh"), "Fresh");
        assert_eq!(unique_name(&l, "Bedtime"), "Bedtime 2");
        l[0].hotkey = "cmd+1".into();
        l[0].folder = "Night".into();
        assert_eq!(duplicate(&mut l, 0), Some(1));
        assert_eq!(l[1].name, "Bedtime copy");
        assert_eq!((l[1].hotkey.as_str(), l[1].folder.as_str()), ("", "Night"));
        assert_eq!(l[1].actions, l[0].actions);
        assert_eq!(duplicate(&mut l, 0), Some(1));
        assert_eq!(l[1].name, "Bedtime copy 2");
        assert_eq!(duplicate(&mut l, 99), None);
        let long = "x".repeat(MAX_NAME);
        let mut one = vec![Shortcut::new(&long, 0)];
        duplicate(&mut one, 0);
        assert!(one[1].name.chars().count() <= MAX_NAME);
    }

    #[test]
    fn set_name_and_folder_clean_their_input() {
        let mut s = Shortcut::new("A", 0);
        assert!(!s.set_name("  \t "));
        assert_eq!(s.name, "A");
        assert!(s.set_name("  New\tname "));
        assert_eq!(s.name, "Newname");
        s.set_folder(&" Work\n ".repeat(30));
        assert!(s.folder.chars().count() <= MAX_NAME && !s.folder.contains('\n'));
        s.set_folder("");
        assert_eq!(s.folder, "");
    }

    #[test]
    fn hotkeys_cycle_through_the_free_keys() {
        let mut l = vec![Shortcut::new("A", 0), Shortcut::new("B", 0)];
        assert_eq!(next_hotkey(&l, 0), "cmd+1");
        l[0].hotkey = "cmd+1".into();
        assert_eq!(next_hotkey(&l, 1), "cmd+2", "a key another shortcut has is skipped");
        assert_eq!(next_hotkey(&l, 0), "cmd+2");
        l[0].hotkey = HOTKEYS[HOTKEYS.len() - 1].into();
        assert_eq!(next_hotkey(&l, 0), "", "after the last key comes Off");
        assert_eq!(next_hotkey(&l, 7), "");
    }

    #[test]
    fn folders_count_and_search_filters() {
        let mut l = defaults();
        l[0].folder = "Night".into();
        l[1].folder = "Power".into();
        l[2].folder = "Power".into();
        assert_eq!(folders(&l), vec![("Night".to_string(), 1), ("Power".to_string(), 2)]);
        assert_eq!(visible(&l, "", None), vec![0, 1, 2, 3]);
        assert_eq!(visible(&l, "", Some("Power")), vec![1, 2]);
        assert_eq!(visible(&l, "TIMER", None), vec![2]);
        assert_eq!(visible(&l, "pow", None), vec![1, 2], "the folder name matches too");
        assert_eq!(visible(&l, "timer", Some("Night")), Vec::<usize>::new());
        assert_eq!(visible(&l, "  cap ", None), vec![3]);
    }

    #[test]
    fn an_exported_file_imports_again_without_its_key() {
        let mut s = defaults()[0].clone();
        s.hotkey = "cmd+3".into();
        s.folder = "Night".into();
        let text = export_text(&s);
        assert!(text.starts_with("BIFROST-SHORTCUT 1\n"));
        let back = import_text(&text).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].hotkey, "");
        assert_eq!(back[0].actions, s.actions);
        assert_eq!(back[0].folder, "Night");
        assert!(import_text("hello").is_err());
        assert!(import_text("BIFROST-SHORTCUT 1\n\n").is_err());
    }

    #[test]
    fn every_gallery_shortcut_is_valid_and_runs() {
        let g = gallery();
        assert!(g.len() >= 5);
        for s in &g {
            assert_eq!(parse(&serialize(std::slice::from_ref(s))), vec![s.clone()], "{}", s.name);
            assert!(s.glyph < GLYPHS.len());
            assert!(s.actions.len() <= MAX_ACTIONS);
            let opens = s.actions.iter().filter(|a| a.opens()).count();
            let ends = s.actions.iter().filter(|a| **a == Action::End).count();
            assert_eq!(opens, ends, "{} closes every block", s.name);
        }
        let hello = g.iter().find(|s| s.name == "Say Hello").unwrap();
        assert_eq!(evaluate(&hello.actions, &[(0, "Ada".into())]).results, vec!["Hello, Ada!".to_string()]);
    }

    #[test]
    fn the_system_kinds_come_first() {
        assert_eq!(SYSTEM_KINDS, 11);
        assert_eq!(KINDS[SYSTEM_KINDS], "Text");
        assert_eq!(KINDS.len(), 23);
    }
}
