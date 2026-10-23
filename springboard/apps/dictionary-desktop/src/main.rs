//! Dictionary: looks words up in the StarDict dictionaries installed in the app's `dictionaries` folder, and in the
//! system's `/usr/share/stardict` folder. Type a word to see its suggestions, Return to look it up, or Look Up
//! Clipboard. The `look-up` intent returns a definition to other callers.

use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::kit::ui::{Ui, Weight, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_SUBHEAD_PT, MATERIAL_SELECTION, ROW_PRESSED, SEGMENT_ON, SPACE_XS};
use bifrost_app::kit::{Damage, Scroller};
use bifrost_app::widgets;
use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::services::ServiceCall;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use bifrost_ui::stardict::{load_library, Hit, Library};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bifrost_dictionary::model::{self, History, Prefs};

const MENU_LOOK_UP_CLIPBOARD: u32 = 1;
const MENU_COPY_DEFINITION: u32 = 2;
const MENU_CLEAR: u32 = 3;
const MENU_FIND: u32 = 4;
const MENU_FIND_NEXT: u32 = 5;
const MENU_FIND_PREVIOUS: u32 = 6;
const MENU_SETTINGS: u32 = 7;
const MENU_BACK: u32 = 8;
const MENU_FORWARD: u32 = 9;
const MENU_NEXT_DICTIONARY: u32 = 10;
const MENU_PREVIOUS_DICTIONARY: u32 = 11;
const MENU_CLEAR_HISTORY: u32 = 12;
const MENU_BIGGER: u32 = 13;
const MENU_SMALLER: u32 = 14;
const MENU_ACTUAL_SIZE: u32 = 15;
const MENU_PRINT: u32 = 16;
const MENU_OPEN_FOLDER: u32 = 17;
const MENU_NEW_WINDOW: u32 = 18;

/// The size of a window opened with File > New Window, in points.
const NEW_WINDOW_PT: (f32, f32) = (480.0, 560.0);

/// Most suggestions listed under the search field.
const SUGGESTIONS: usize = 12;

/// Layout in points: the search field, the list below it and the footer line.
const FIELD_TOP: f32 = 12.0;
const FIELD_H: f32 = 32.0;
const PAD: f32 = 16.0;
const LIST_TOP: f32 = 56.0;
const FOOTER_H: f32 = 28.0;
/// Height of one suggestion row, in points.
const ROW_H: f32 = 28.0;
/// Line advance of wrapped text, as a multiple of its size.
const LEADING: f32 = 1.35;
/// Height of the scope bar under the search field, and the width of a toolbar button, in points.
const SCOPE_H: f32 = 28.0;
const BTN_W: f32 = 28.0;
/// Height of a row in the settings page.
const SETTINGS_ROW_H: f32 = 32.0;

/// What a clickable region does.
#[derive(Clone, Debug, PartialEq)]
enum Act {
    Back,
    Forward,
    Smaller,
    Bigger,
    Scope(Option<String>),
    Toggle(usize),
    Up(usize),
    Down(usize),
    Rename(usize),
    Done,
}

/// A clickable region of the last frame, in points.
struct Hot {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    act: Act,
}

/// Find in the definition on screen.
struct Find {
    query: String,
    /// Occurrences of the text, counted in the last frame.
    count: usize,
    /// Position in the matches of the highlighted one.
    cur: Option<usize>,
    /// Scroll the highlighted match into view in the next frame.
    scroll_to: bool,
}

/// A dictionary row of the settings page that is being dragged to another place.
struct Drag {
    from: usize,
    /// The pointer's vertical position, in points.
    y: f32,
}

/// The most words remembered as having (or not having) a definition before the memory is emptied.
const KNOWN_LIMIT: usize = 4096;

/// The lookup on screen: the word and what each dictionary says about it.
struct Shown {
    word: String,
    hits: Vec<Hit>,
}

/// One drawn line: its text, font size, weight, colour and the vertical space it takes, in points.
struct Line {
    text: String,
    pt: f32,
    weight: Weight,
    colour: u32,
    h: f32,
    /// A line of a definition: a tap on one of its words follows it as a cross-reference.
    body: bool,
}

struct DictionaryApp {
    library: Option<Library>,
    /// Where the app's own dictionaries are kept, shown in the empty state.
    folder: String,
    query: String,
    /// The lookup on screen. `None` while the query is being typed: the suggestions show instead.
    shown: Option<Shown>,
    suggestions: Vec<String>,
    scroll: Scroller,
    prefs: Prefs,
    prefs_path: PathBuf,
    history: History,
    /// The dictionary the scope bar shows; `None` is All.
    scope: Option<String>,
    find: Option<Find>,
    /// The settings page replaces the lookup while it is open.
    settings: bool,
    /// The dictionary being renamed in the settings page, with the text typed so far.
    rename: Option<(usize, String)>,
    hot: Vec<Hot>,
    /// The definition lines on screen in the last frame: top, height, text and size, in points.
    body_rows: Vec<(f32, f32, String, f32)>,
    /// The settings row being dragged, if any.
    drag: Option<Drag>,
    /// Top of the first dictionary row of the settings page in the last frame, in points.
    rows_top: f32,
    /// Whether a word has a definition in an enabled dictionary; links in definitions use it.
    known: HashMap<String, bool>,
    /// The windows opened with File > New Window, in the order they were opened. The main window is `self`.
    windows: Vec<ExtraWindow>,
}

/// A window opened with File > New Window. It is a whole lookup of its own (query, answer, history, scope, find), so
/// Back and Forward in one window never move another.
struct ExtraWindow {
    id: WindowId,
    app: DictionaryApp,
}

/// Pointer position in points: input is in pixels, and the layout is in points.
fn to_points(ctx: &Ctx, x: f32, y: f32) -> (f32, f32) {
    let scale = ctx.ui.pt(1.0).max(0.001);
    (x / scale, y / scale)
}

impl DictionaryApp {
    fn new() -> DictionaryApp {
        DictionaryApp {
            library: None,
            folder: String::new(),
            query: String::new(),
            shown: None,
            suggestions: Vec::new(),
            scroll: Scroller::new(),
            prefs: Prefs::default(),
            prefs_path: PathBuf::new(),
            history: History::default(),
            scope: None,
            find: None,
            settings: false,
            rename: None,
            hot: Vec::new(),
            body_rows: Vec::new(),
            drag: None,
            rows_top: 0.0,
            known: HashMap::new(),
            windows: Vec::new(),
        }
    }

    /// Opens a window with an empty lookup. It loads the dictionaries and the stored choices the way the main window does.
    fn open_new_window(&mut self, ctx: &Ctx) -> bool {
        let (w, h) = NEW_WINDOW_PT;
        let Ok(id) = ctx.open_window(WindowSpec::new("Dictionary", w, h)) else { return false };
        let mut app = DictionaryApp::new();
        app.configure(ctx);
        self.windows.push(ExtraWindow { id, app });
        true
    }

    /// Hands an event to the extra window's own lookup. False when the window is not open.
    fn extra_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        match self.windows.iter_mut().find(|e| e.id == window) {
            Some(e) => e.app.event(ctx, ev),
            None => false,
        }
    }

    /// Drops the state of a window the shell or the app closed. Returns true when one was dropped.
    fn close_extra(&mut self, window: WindowId) -> bool {
        let before = self.windows.len();
        self.windows.retain(|e| e.id != window);
        self.windows.len() != before
    }

    /// True when an enabled dictionary defines `word`. The answer is remembered until the dictionary choices change.
    fn defines(&mut self, word: &str) -> bool {
        if let Some(known) = self.known.get(word) {
            return *known;
        }
        let found = self.library.as_ref().is_some_and(|l| l.dictionaries.iter().any(|d| self.prefs.is_enabled(&d.info.name) && d.find(word).is_some()));
        if self.known.len() >= KNOWN_LIMIT {
            self.known.clear();
        }
        self.known.insert(word.to_string(), found);
        found
    }

    /// True when `word` in a definition is a cross-reference: another word than the one on screen that has a definition.
    fn is_link(&mut self, word: &str) -> bool {
        if self.shown.as_ref().is_some_and(|s| s.word.eq_ignore_ascii_case(word)) {
            return false;
        }
        self.defines(word)
    }

    fn save_prefs(&self) {
        if !self.prefs_path.as_os_str().is_empty() {
            let _ = std::fs::write(&self.prefs_path, self.prefs.serialize());
        }
    }

    /// Applies one change to the stored choices. Every window has its own copy of them, so the file is the store: the
    /// copy is read back first, so a change made in another window survives this window's save.
    fn change_prefs(&mut self, change: impl FnOnce(&mut Prefs)) {
        if !self.prefs_path.as_os_str().is_empty() {
            if let Ok(text) = std::fs::read_to_string(&self.prefs_path) {
                self.prefs = Prefs::parse(&text);
            }
        }
        change(&mut self.prefs);
        self.save_prefs();
    }

    /// The hits of the shown lookup that the scope bar lets through.
    fn visible_hits(&self) -> Vec<Hit> {
        self.shown.as_ref().map(|s| model::in_scope(&s.hits, &self.scope)).unwrap_or_default()
    }

    fn scope_names(&self) -> Vec<String> {
        self.shown.as_ref().map(|s| model::scope_names(&s.hits)).unwrap_or_default()
    }

    fn show_scope_bar(&self) -> bool {
        self.scope_names().len() >= 2
    }

    /// Shows the same word again, for example after the dictionary choices changed.
    fn refresh(&mut self) -> bool {
        self.known.clear();
        match self.shown.as_ref().map(|s| s.word.clone()) {
            Some(w) => self.show(&w, false),
            None => true,
        }
    }

    fn go_back(&mut self) -> bool {
        match self.history.back() {
            Some(w) => self.show(&w, false),
            None => false,
        }
    }

    fn go_forward(&mut self) -> bool {
        match self.history.forward() {
            Some(w) => self.show(&w, false),
            None => false,
        }
    }

    fn step_scope(&mut self, delta: i32) -> bool {
        let names = self.scope_names();
        if names.len() < 2 {
            return false;
        }
        self.scope = model::cycle_scope(&self.scope, &names, delta);
        self.scroll.jump_top();
        true
    }

    fn step_find(&mut self, delta: i32) -> bool {
        let Some(f) = &mut self.find else { return false };
        let all: Vec<usize> = (0..f.count).collect();
        f.cur = model::step_match(&all, f.cur, delta);
        f.scroll_to = true;
        true
    }

    fn start_find(&mut self) -> bool {
        if self.shown.is_none() {
            return false;
        }
        self.find = Some(Find { query: String::new(), count: 0, cur: None, scroll_to: false });
        true
    }

    /// A click on one of the regions of the last frame.
    fn perform(&mut self, act: Act) -> bool {
        match act {
            Act::Back => self.go_back(),
            Act::Forward => self.go_forward(),
            Act::Smaller => {
                self.change_prefs(|p| p.smaller());
                true
            }
            Act::Bigger => {
                self.change_prefs(|p| p.bigger());
                true
            }
            Act::Scope(s) => {
                self.scope = s;
                self.scroll.jump_top();
                true
            }
            Act::Toggle(i) => {
                self.change_prefs(|p| p.toggle(i));
                self.refresh()
            }
            Act::Up(i) => {
                self.change_prefs(|p| p.move_dict(i, -1));
                self.refresh()
            }
            Act::Down(i) => {
                self.change_prefs(|p| p.move_dict(i, 1));
                self.refresh()
            }
            Act::Rename(i) => {
                let label = self.prefs.dicts.get(i).map(|d| d.label.clone()).unwrap_or_default();
                self.rename = Some((i, label));
                true
            }
            Act::Done => {
                self.settings = false;
                self.rename = None;
                self.drag = None;
                true
            }
        }
    }

    fn hit_test(&self, px: f32, py: f32) -> Option<Act> {
        self.hot.iter().find(|h| px >= h.x && px < h.x + h.w && py >= h.y && py < h.y + h.h).map(|h| h.act.clone())
    }

    /// The query changed: drop the lookup on screen and list the words that start with what is typed.
    fn edited(&mut self) -> bool {
        self.shown = None;
        self.find = None;
        self.suggestions = self.library.as_ref().map(|l| l.suggest(&self.query, SUGGESTIONS)).unwrap_or_default();
        self.scroll.jump_top();
        true
    }

    /// Looks `word` up and shows the answer. Returns true when the screen needs a frame.
    fn look_up(&mut self, word: &str) -> bool {
        self.show(word, true)
    }

    /// Looks `word` up with the stored dictionary choices. `record` adds it to the history; Back and Forward do not.
    fn show(&mut self, word: &str, record: bool) -> bool {
        let word = word.trim();
        if word.is_empty() {
            return false;
        }
        let raw = self.library.as_ref().map(|l| l.lookup(word)).unwrap_or_default();
        let hits = self.prefs.arrange(raw);
        if record {
            self.history.visit(word);
        }
        if let Some(s) = &self.scope {
            if !hits.iter().any(|h| &h.dictionary == s) {
                self.scope = None;
            }
        }
        self.query = word.to_string();
        self.suggestions.clear();
        self.shown = Some(Shown { word: word.to_string(), hits });
        if let Some(f) = &mut self.find {
            f.cur = None;
        }
        self.scroll.jump_top();
        true
    }

    /// Shows `word` in this window after another app asked for it (the look-up intent). Before the window has loaded its
    /// dictionaries there is nothing to show it with, so nothing changes. True when the screen needs a frame.
    fn show_from_intent(&mut self, word: &str) -> bool {
        if self.library.is_none() {
            return false;
        }
        self.settings = false;
        self.show(word, true)
    }

    /// The first line of the clipboard, looked up.
    fn look_up_clipboard(&mut self, ctx: &Ctx) -> bool {
        let text = ctx.pasteboard_string().unwrap_or_default();
        let word = text.lines().next().unwrap_or("").to_string();
        self.look_up(&word)
    }

    /// Copies the definitions on screen as plain text.
    fn copy_definition(&self, ctx: &Ctx) {
        if self.shown.is_none() {
            return;
        }
        ctx.set_pasteboard_string(&model::definition_text(&self.visible_hits()));
    }

    /// Sends the definitions on screen to the default printer on its own thread, so the window keeps drawing.
    fn print_definition(&self) -> bool {
        let Some(shown) = &self.shown else { return false };
        let hits = self.visible_hits();
        if hits.is_empty() {
            return false;
        }
        let body = model::definition_text(&hits);
        let job = bifrost_app::print::PrintOperation::text(shown.word.clone(), format!("{}\n\n{body}", shown.word));
        DispatchQueue::global(Qos::Utility).async_(move || {
            if let Err(e) = job.run() {
                eprintln!("Dictionary print: {e:?}");
            }
        });
        false
    }

    /// The text that explains an empty answer.
    fn no_answer(&self, word: &str) -> String {
        let Some(lib) = &self.library else { return String::new() };
        if lib.is_empty() {
            format!("No dictionary is installed yet. Copy StarDict dictionaries (.ifo, .idx and .dict files) into {}.", self.folder)
        } else if lib.contains_word(word) {
            format!("\u{201c}{word}\u{201d} is in the system word list, but no installed dictionary defines it.")
        } else {
            format!("No definition for \u{201c}{word}\u{201d}.")
        }
    }

    /// The lines of the list under the search field, in order, for a `width` points wide list.
    fn lines(&self, ui: &Ui, width: f32) -> Vec<Line> {
        let mut out = Vec::new();
        let size = self.prefs.scaled(MAC_BODY_PT);
        let body = |text: String, colour: u32| Line { text, pt: size, weight: Weight::Regular, colour, h: size * LEADING, body: false };
        let message = |out: &mut Vec<Line>, text: &str, colour: u32| {
            for line in ui.wrap(text, size, Weight::Regular, ui.pt(width - 2.0 * PAD)) {
                out.push(body(line, colour));
            }
        };
        if let Some(shown) = &self.shown {
            let hits = self.visible_hits();
            if hits.is_empty() {
                message(&mut out, &self.no_answer(&shown.word), LABEL_2.now());
            }
            for hit in &hits {
                out.push(Line { text: hit.dictionary.clone(), pt: MAC_SUBHEAD_PT, weight: Weight::Semibold, colour: LABEL_2.now(), h: 24.0, body: false });
                for paragraph in hit.text.split('\n') {
                    if paragraph.is_empty() {
                        out.push(Line { text: String::new(), pt: size, weight: Weight::Regular, colour: LABEL.now(), h: 8.0, body: false });
                        continue;
                    }
                    for line in ui.wrap(paragraph, size, Weight::Regular, ui.pt(width - 2.0 * PAD)) {
                        out.push(Line { body: true, ..body(line, LABEL.now()) });
                    }
                }
                out.push(Line { text: String::new(), pt: size, weight: Weight::Regular, colour: LABEL.now(), h: 18.0, body: false });
            }
        } else if !self.suggestions.is_empty() {
            for word in &self.suggestions {
                out.push(Line { text: word.clone(), pt: size, weight: Weight::Regular, colour: LABEL.now(), h: ROW_H, body: false });
            }
        } else if self.query.trim().is_empty() {
            message(&mut out, "Type a word to look it up.", LABEL_2.now());
        } else {
            message(&mut out, &format!("No words start with \u{201c}{}\u{201d}.", self.query.trim()), LABEL_2.now());
        }
        out
    }

    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let (px, py) = to_points(ctx, x, y);
        match kind {
            TouchKind::Down => {
                if self.settings && self.rename.is_none() && self.hit_test(px, py).is_none() {
                    if let Some(from) = model::row_at(py, self.rows_top, SETTINGS_ROW_H, self.prefs.dicts.len()) {
                        self.drag = Some(Drag { from, y: py });
                        return true;
                    }
                }
                self.scroll.down(py, t);
                false
            }
            TouchKind::Move => {
                if let Some(d) = &mut self.drag {
                    d.y = py;
                    return true;
                }
                self.scroll.moved(py, t);
                true
            }
            // Scroller::up reports a tap (no drag): a suggestion row looks its word up
            TouchKind::Up => {
                if let Some(d) = self.drag.take() {
                    let to = model::drop_slot(py, self.rows_top, SETTINGS_ROW_H, self.prefs.dicts.len());
                    if to != d.from {
                        self.change_prefs(|p| p.move_to(d.from, to));
                        self.refresh();
                    }
                    return true;
                }
                if self.scroll.up(py, t) {
                    if let Some(act) = self.hit_test(px, py) {
                        return self.perform(act);
                    }
                    if self.tap_definition(ctx, x, py) {
                        return true;
                    }
                    self.tap_row(py)
                } else {
                    true
                }
            }
            TouchKind::Cancel => {
                if self.drag.take().is_some() {
                    return true;
                }
                self.scroll.up(py, t);
                true
            }
        }
    }

    /// A tap on a word of a definition looks that word up when some dictionary knows it (a cross-reference).
    fn tap_definition(&mut self, ctx: &Ctx, x_px: f32, py: f32) -> bool {
        if self.settings || self.shown.is_none() {
            return false;
        }
        let Some((_, _, text, pt)) = self.body_rows.iter().find(|(y, h, _, _)| py >= *y && py < *y + *h).cloned() else { return false };
        let tap = x_px - ctx.ui.pt(PAD);
        for (start, end) in model::word_spans(&text) {
            let from = ctx.ui.measure(&text[..start], pt, Weight::Regular);
            let to = ctx.ui.measure(&text[..end], pt, Weight::Regular);
            if tap >= from && tap < to {
                let word = text[start..end].to_string();
                return self.is_link(&word) && self.look_up(&word);
            }
        }
        false
    }

    /// A tap at `py` (points) on the suggestion list looks that word up.
    fn tap_row(&mut self, py: f32) -> bool {
        if self.shown.is_some() || self.settings || py < LIST_TOP {
            return true;
        }
        let row = ((py - LIST_TOP + self.scroll.offset()) / ROW_H).floor();
        if row < 0.0 {
            return true;
        }
        match self.suggestions.get(row as usize).cloned() {
            Some(word) => self.look_up(&word),
            None => true,
        }
    }

    fn key(&mut self, key: &KeyInput) -> bool {
        if let Some((i, buf)) = &mut self.rename {
            match key {
                KeyInput::Text(text) => buf.extend(text.chars().filter(|c| !c.is_control())),
                KeyInput::Code(8) => {
                    buf.pop();
                }
                KeyInput::Code(13) => {
                    let (i, label) = (*i, buf.clone());
                    self.change_prefs(|p| p.set_label(i, &label));
                    self.rename = None;
                    return self.refresh();
                }
                KeyInput::Code(1 | 27) => self.rename = None,
                _ => return false,
            }
            return true;
        }
        if self.settings {
            return match key {
                KeyInput::Code(1 | 27) => {
                    self.settings = false;
                    self.drag = None;
                    true
                }
                _ => false,
            };
        }
        if let Some(f) = &mut self.find {
            match key {
                KeyInput::Text(text) => {
                    f.query.extend(text.chars().filter(|c| !c.is_control()));
                    f.cur = None;
                }
                KeyInput::Code(8) => {
                    f.query.pop();
                    f.cur = None;
                }
                KeyInput::Code(13) => return self.step_find(1),
                KeyInput::Code(1 | 27) => self.find = None,
                _ => return false,
            }
            return true;
        }
        match key {
            KeyInput::Text(text) => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    self.query.push(c);
                }
                self.edited()
            }
            KeyInput::Code(8) => {
                if self.query.pop().is_none() {
                    return false;
                }
                self.edited()
            }
            KeyInput::Code(13) => {
                let word = self.query.clone();
                self.look_up(&word)
            }
            KeyInput::Code(1 | 27) => {
                if self.query.is_empty() && self.shown.is_none() {
                    return false;
                }
                self.query.clear();
                self.edited()
            }
            _ => false,
        }
    }

    fn menu(&self) -> Vec<AppMenu> {
        let has_answer = self.shown.as_ref().is_some_and(|s| !s.hits.is_empty());
        let finding = self.find.as_ref().is_some_and(|f| f.count > 0);
        let dictionary = vec![
            AppMenuItem::new(MENU_LOOK_UP_CLIPBOARD, "Look Up Clipboard").key("cmd+l"),
            AppMenuItem::new(MENU_COPY_DEFINITION, "Copy Definition").enabled(has_answer),
            AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(has_answer && bifrost_app::print::available()),
            AppMenuItem::new(MENU_OPEN_FOLDER, "Open Dictionaries Folder"),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_FIND, "Find in Definition").key("cmd+f").enabled(self.shown.is_some() && !self.settings),
            AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(finding),
            AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(finding),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_CLEAR, "Clear Search").enabled(!self.query.is_empty() || self.shown.is_some()),
            AppMenuItem::new(MENU_SETTINGS, "Settings...").key("cmd+,").checked(self.settings),
        ];
        let multi = self.show_scope_bar();
        let go = vec![
            AppMenuItem::new(MENU_BACK, "Back").key("cmd+[").enabled(self.history.can_back()),
            AppMenuItem::new(MENU_FORWARD, "Forward").key("cmd+]").enabled(self.history.can_forward()),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_NEXT_DICTIONARY, "Select Next Dictionary").key("cmd+alt+right").enabled(multi),
            AppMenuItem::new(MENU_PREVIOUS_DICTIONARY, "Select Previous Dictionary").key("cmd+alt+left").enabled(multi),
            AppMenuItem::separator(),
            AppMenuItem::new(MENU_CLEAR_HISTORY, "Clear History").enabled(!self.history.is_empty()),
        ];
        let view = vec![
            AppMenuItem::new(MENU_BIGGER, "Bigger Text").key("cmd+=").enabled(self.prefs.step < model::MAX_STEP),
            AppMenuItem::new(MENU_SMALLER, "Smaller Text").key("cmd+-").enabled(self.prefs.step > model::MIN_STEP),
            AppMenuItem::new(MENU_ACTUAL_SIZE, "Actual Size").key("cmd+0").enabled(self.prefs.step != 0),
        ];
        let file = vec![AppMenuItem::new(MENU_NEW_WINDOW, "New Window").key("cmd+n")];
        vec![
            AppMenu { title: "File".into(), items: file },
            AppMenu { title: "Dictionary".into(), items: dictionary },
            AppMenu { title: "Go".into(), items: go },
            AppMenu { title: "View".into(), items: view },
        ]
    }
}

/// Draws a flat button of `w` by `h` points at (`x`, `y`) with a centred label. A button with an action is clickable;
/// without one it is drawn dimmed. `strong` marks the selected segment.
#[allow(clippy::too_many_arguments)]
fn button(canvas: &mut dyn Canvas, ui: &Ui, hot: &mut Vec<Hot>, x: f32, y: f32, w: f32, h: f32, label: &str, act: Option<Act>, strong: bool) {
    let r = RectF::new(ui.pt(x), ui.pt(y), ui.pt(w), ui.pt(h));
    canvas.fill_rrect(r, ui.pt(6.0), if strong { SEGMENT_ON.now() } else { FIELD_BG.now() });
    let colour = if act.is_some() { LABEL.now() } else { LABEL_3.now() };
    let img = ui.text(&ui.fit(label, MAC_SUBHEAD_PT, r.w - ui.pt(SPACE_XS)), MAC_SUBHEAD_PT, Weight::Regular, colour);
    widgets::blit(canvas, &img, (r.x + (r.w - img.w as f32) / 2.0) as i32, (r.y + (r.h - img.h as f32) / 2.0) as i32);
    if let Some(act) = act {
        hot.push(Hot { x, y, w, h, act });
    }
}

/// Draws `text` in a settings row of `SETTINGS_ROW_H` points starting at `y`, vertically centred.
#[allow(clippy::too_many_arguments)]
fn text_at(canvas: &mut dyn Canvas, ui: &Ui, text: &str, pt: f32, weight: Weight, colour: u32, x: f32, y: f32, w: f32) {
    let img = ui.text(&ui.fit(text, pt, ui.pt(w)), pt, weight, colour);
    widgets::blit(canvas, &img, ui.pt(x) as i32, (ui.pt(y) + (ui.pt(SETTINGS_ROW_H) - img.h as f32) / 2.0) as i32);
}

impl DictionaryApp {
    /// The settings page: which dictionaries are used, in what order and under what label, the text size and Done.
    fn draw_settings(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, width_pt: f32, top: f32, view_h: f32) {
        let ui = &ctx.ui;
        let (cw, _) = canvas.size();
        canvas.push_clip(RectF::new(0.0, ui.pt(top), cw as f32, ui.pt(view_h)), 0.0);
        let mut y = top;
        text_at(canvas, ui, "Dictionaries", MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), PAD, y, width_pt - 2.0 * PAD);
        y += SETTINGS_ROW_H;
        let count = self.prefs.dicts.len();
        self.rows_top = y;
        let rows_top = y;
        // while a row is dragged the others show where it would land
        let order = match &self.drag {
            Some(d) if d.from < count => model::preview_order(count, d.from, model::drop_slot(d.y, rows_top, SETTINGS_ROW_H, count)),
            _ => (0..count).collect(),
        };
        let dragged = self.drag.as_ref().map(|d| (d.from, d.y));
        for (slot, &i) in order.iter().enumerate() {
            let d = self.prefs.dicts[i].clone();
            let mut y = rows_top + slot as f32 * SETTINGS_ROW_H;
            if let Some((from, pointer)) = dragged {
                if from == i {
                    y = (pointer - SETTINGS_ROW_H / 2.0).clamp(rows_top, rows_top + (count.max(1) - 1) as f32 * SETTINGS_ROW_H);
                    canvas.fill_rect(RectF::new(0.0, ui.pt(y), cw as f32, ui.pt(SETTINGS_ROW_H)), ROW_PRESSED.now());
                }
            }
            let renaming = self.rename.as_ref().and_then(|(r, buf)| (*r == i).then(|| buf.clone()));
            button(canvas, ui, &mut self.hot, PAD, y + 4.0, 36.0, SETTINGS_ROW_H - 8.0, if d.enabled { "On" } else { "Off" }, Some(Act::Toggle(i)), d.enabled);
            let buttons_w = 3.0 * 56.0 + 2.0 * 4.0;
            let name_x = PAD + 36.0 + 12.0;
            let name_w = (width_pt - PAD - buttons_w - name_x - 8.0).max(40.0);
            match renaming {
                Some(buf) => {
                    let r = RectF::new(ui.pt(name_x), ui.pt(y + 3.0), ui.pt(name_w), ui.pt(SETTINGS_ROW_H - 6.0));
                    canvas.fill_rrect(r, ui.pt(6.0), FIELD_BG.now());
                    text_at(canvas, ui, &format!("{buf}|"), MAC_BODY_PT, Weight::Regular, LABEL.now(), name_x + 6.0, y, name_w - 12.0);
                }
                None => {
                    let colour = if d.enabled { LABEL.now() } else { LABEL_3.now() };
                    text_at(canvas, ui, d.shown_name(), MAC_BODY_PT, Weight::Regular, colour, name_x, y, name_w);
                }
            }
            let bx = width_pt - PAD - buttons_w;
            button(canvas, ui, &mut self.hot, bx, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "Up", (i > 0).then_some(Act::Up(i)), false);
            button(canvas, ui, &mut self.hot, bx + 60.0, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "Down", (i + 1 < count).then_some(Act::Down(i)), false);
            button(canvas, ui, &mut self.hot, bx + 120.0, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "Rename", Some(Act::Rename(i)), false);
        }
        y = rows_top + count as f32 * SETTINGS_ROW_H;
        if count == 0 {
            text_at(canvas, ui, "No dictionary is installed yet.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), PAD, y, width_pt - 2.0 * PAD);
            y += SETTINGS_ROW_H;
        }
        y += SETTINGS_ROW_H / 2.0;
        let size_text = format!("Text size: {}%", 100 + 10 * self.prefs.step);
        text_at(canvas, ui, &size_text, MAC_BODY_PT, Weight::Regular, LABEL.now(), PAD, y, width_pt - 2.0 * PAD - 120.0);
        let right = width_pt - PAD;
        button(canvas, ui, &mut self.hot, right - 56.0, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "A+", (self.prefs.step < model::MAX_STEP).then_some(Act::Bigger), false);
        button(canvas, ui, &mut self.hot, right - 116.0, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "A-", (self.prefs.step > model::MIN_STEP).then_some(Act::Smaller), false);
        y += SETTINGS_ROW_H * 1.5;
        button(canvas, ui, &mut self.hot, right - 80.0, y, 80.0, SETTINGS_ROW_H - 4.0, "Done", Some(Act::Done), true);
        canvas.pop_clip();
    }
}

impl App for DictionaryApp {
    fn id(&self) -> &str {
        "is.olibuijr.dictionary.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination();
        if self.library.is_none() {
            self.folder = ctx.data_dir.join("dictionaries").display().to_string();
            let library = load_library(&ctx.data_dir);
            self.prefs_path = ctx.data_dir.join("prefs.txt");
            self.prefs = std::fs::read_to_string(&self.prefs_path).map(|t| Prefs::parse(&t)).unwrap_or_default();
            let names: Vec<String> = library.dictionaries.iter().map(|d| d.info.name.clone()).collect();
            if self.prefs.sync(&names) {
                self.save_prefs();
            }
            self.library = Some(library);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, t, .. } => self.touch(ctx, kind, x, y, t),
            Input::Key(key) => self.key(&key),
            Input::Open(url) => match model::word_from_url(&url) {
                Some(word) => {
                    self.settings = false;
                    self.look_up(&word)
                }
                None => false,
            },
            Input::Command(Command::Copy) => {
                self.copy_definition(ctx);
                false
            }
            Input::Command(Command::Paste) => {
                let text = ctx.pasteboard_string().unwrap_or_default();
                let line = text.lines().next().unwrap_or("").to_string();
                self.query.push_str(&line);
                self.edited()
            }
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        self.menu()
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW_WINDOW => self.open_new_window(ctx),
            MENU_LOOK_UP_CLIPBOARD => self.look_up_clipboard(ctx),
            MENU_COPY_DEFINITION => {
                self.copy_definition(ctx);
                false
            }
            MENU_PRINT => self.print_definition(),
            MENU_OPEN_FOLDER => {
                let _ = std::fs::create_dir_all(&self.folder);
                ctx.open_url(&model::folder_url(&self.folder));
                false
            }
            MENU_CLEAR => {
                self.query.clear();
                self.edited()
            }
            MENU_FIND => self.start_find(),
            MENU_FIND_NEXT => self.step_find(1),
            MENU_FIND_PREVIOUS => self.step_find(-1),
            MENU_SETTINGS => {
                self.settings = !self.settings;
                self.rename = None;
                self.drag = None;
                true
            }
            MENU_BACK => self.go_back(),
            MENU_FORWARD => self.go_forward(),
            MENU_NEXT_DICTIONARY => self.step_scope(1),
            MENU_PREVIOUS_DICTIONARY => self.step_scope(-1),
            MENU_CLEAR_HISTORY => {
                self.history.clear();
                true
            }
            MENU_BIGGER => self.perform(Act::Bigger),
            MENU_SMALLER => self.perform(Act::Smaller),
            MENU_ACTUAL_SIZE => {
                self.prefs.actual_size();
                self.save_prefs();
                true
            }
            _ => false,
        }
    }

    fn rpc_methods(&self) -> Vec<&'static str> {
        vec!["service.look_up"]
    }

    /// Services menu: "Look Up in Dictionary" shows the selected text in this window, with the history, scope bar and
    /// find, where the look-up popover only shows the first definition.
    fn rpc(&mut self, ctx: Option<&Ctx>, _data_dir: &Path, method: &str, params: &Json) -> Result<Json, RpcError> {
        if method != "service.look_up" {
            return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}")));
        }
        let call = ServiceCall::from_params(params).ok_or_else(|| RpcError::new(INVALID_PARAMS, "the service call has no text"))?;
        let text = call.text.ok_or_else(|| RpcError::new(INVALID_PARAMS, "the service needs text"))?;
        if ctx.is_none() {
            return Err(RpcError::new(INTERNAL_ERROR, "Dictionary is not running yet"));
        }
        let word = model::service_word(&text).ok_or_else(|| RpcError::new(INVALID_PARAMS, "the selection has no word to look up"))?;
        if !self.show_from_intent(&word) {
            return Err(RpcError::new(INTERNAL_ERROR, "Dictionary has not loaded its dictionaries yet"));
        }
        Ok(Json::Obj(Vec::new()))
    }

    fn intents(&self) -> Vec<Intent> {
        vec![Intent::new("look-up", "Look Up", "Looks a word up in the installed dictionaries and returns its definition.").param(Param::new("word", "Word", ParamType::String))]
    }

    fn perform_intent(&mut self, ctx: Option<&Ctx>, data_dir: &Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
        if id != "look-up" {
            return Err(RpcError::new(INVALID_PARAMS, format!("no intent {id}")));
        }
        let word = intents::str_param(params, "word")?.trim().to_string();
        let hits = load_library(data_dir).lookup(&word);
        let Some(first) = hits.first() else {
            return Err(intents::failed(format!("no definition for {word}")));
        };
        let text = first.text.clone();
        // a running window shows the word too, so Look Up from another app (the Services menu, the look-up popover's
        // Open in Dictionary row) lands in the app's own window with its history, scope bar and find
        if ctx.is_some() {
            self.show_from_intent(&word);
        }
        Ok(IntentResult::value(Json::Str(text.clone())).with_dialog(&format!("{word}: {text}")))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let mut redraw = false;
        if self.scroll.animating() {
            self.scroll.tick(t);
            redraw = true;
        }
        for e in &mut self.windows {
            redraw |= e.app.tick(ctx, t);
        }
        redraw
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let own = self.scroll.animating().then_some(t + 16.0);
        self.windows.iter().filter_map(|e| e.app.next_wake(t)).chain(own).reduce(f64::min)
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        // the keyboard, the menus and the clipboard belong to the main window; a window takes its touches
        let Some(e) = self.windows.iter_mut().find(|e| e.id == window) else { return false };
        match ev {
            Input::Touch { .. } => e.app.event(ctx, ev),
            _ => false,
        }
    }

    fn draw_window(&mut self, ctx: &Ctx, window: WindowId, s: &mut bifrost_app::kit::Surface, t: f64) -> Damage {
        if window.is_main() {
            return self.draw(ctx, s, t);
        }
        match self.windows.iter_mut().find(|e| e.id == window) {
            Some(e) => e.app.draw(ctx, s, t),
            None => Damage::new(s.w, s.h),
        }
    }

    fn window_closed(&mut self, _ctx: &Ctx, window: WindowId) -> bool {
        self.close_extra(window)
    }

    /// An extra window is a whole lookup of its own, so its keys and Copy go to it; Cmd+W closes it. The main window
    /// takes the rest as before (`default_window_input`).
    fn window_input(&mut self, ctx: &Ctx, window: WindowId, input: bifrost_app::proto::WindowInput) -> bool {
        use bifrost_app::proto::WindowInput;
        if window.is_main() {
            return bifrost_app::app_windows::default_window_input(self, ctx, window, input);
        }
        match input {
            WindowInput::Key(k) if bifrost_app::app_windows::is_close_window_key(&k) => {
                ctx.close_window(window).is_ok() && self.close_extra(window)
            }
            WindowInput::Key(k) => self.extra_event(ctx, window, Input::Key(k)),
            WindowInput::Command(c) => self.extra_event(ctx, window, Input::Command(c)),
            // the wheel and hover of the window under the pointer take the SDK route (`App::window_pointer`)
            pointer @ WindowInput::Pointer(_) => bifrost_app::app_windows::default_window_input(self, ctx, window, pointer),
            _ => false,
        }
    }

    fn draws_canvas(&self) -> bool {
        true
    }

    fn draw_canvas(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, _t: f64) -> Damage {
        let (cw, ch) = canvas.size();
        let ui = &ctx.ui;
        let scale = ui.pt(1.0).max(0.001);
        let width_pt = cw as f32 / scale;
        let height_pt = ch as f32 / scale;
        self.hot.clear();
        self.body_rows.clear();

        canvas.fill_rect(RectF::new(0.0, 0.0, cw as f32, ch as f32), CONTENT_BG.now());

        // the toolbar: Back and Forward, the search field, and the text size buttons
        let gap = 4.0;
        let can_back = self.history.can_back();
        let can_forward = self.history.can_forward();
        let can_smaller = self.prefs.step > model::MIN_STEP;
        let can_bigger = self.prefs.step < model::MAX_STEP;
        button(canvas, ui, &mut self.hot, PAD, FIELD_TOP, BTN_W, FIELD_H, "<", can_back.then_some(Act::Back), false);
        button(canvas, ui, &mut self.hot, PAD + BTN_W + gap, FIELD_TOP, BTN_W, FIELD_H, ">", can_forward.then_some(Act::Forward), false);
        let right = width_pt - PAD;
        button(canvas, ui, &mut self.hot, right - BTN_W, FIELD_TOP, BTN_W, FIELD_H, "A+", can_bigger.then_some(Act::Bigger), false);
        button(canvas, ui, &mut self.hot, right - 2.0 * BTN_W - gap, FIELD_TOP, BTN_W, FIELD_H, "A-", can_smaller.then_some(Act::Smaller), false);
        let field_x = PAD + 2.0 * (BTN_W + gap);
        let field_w = (right - 2.0 * (BTN_W + gap) - field_x).max(40.0);
        let field = RectF::new(ui.pt(field_x), ui.pt(FIELD_TOP), ui.pt(field_w), ui.pt(FIELD_H));
        canvas.fill_rrect(field, ui.pt(6.0), FIELD_BG.now());
        let (text, colour) = if self.query.is_empty() { ("Search Dictionary", LABEL_2.now()) } else { (self.query.as_str(), LABEL.now()) };
        let fitted = ui.fit(text, MAC_BODY_PT, field.w - ui.pt(PAD));
        let img = ui.text(&fitted, MAC_BODY_PT, Weight::Regular, colour);
        widgets::blit(canvas, &img, (field.x + ui.pt(PAD / 2.0)) as i32, (field.y + (field.h - img.h as f32) / 2.0) as i32);

        let mut top = LIST_TOP;
        // the scope bar: All and each dictionary that answered
        if !self.settings && self.show_scope_bar() {
            let mut x = PAD;
            let mut entries: Vec<(String, Option<String>)> = vec![("All".to_string(), None)];
            entries.extend(self.scope_names().into_iter().map(|n| (n.clone(), Some(n))));
            for (label, scope) in entries {
                let w = ui.measure(&label, MAC_SUBHEAD_PT, Weight::Regular) / scale + 16.0;
                if x + w > width_pt - PAD {
                    break;
                }
                let on = self.scope == scope;
                button(canvas, ui, &mut self.hot, x, top, w, SCOPE_H - 4.0, &label, Some(Act::Scope(scope)), on);
                x += w + gap;
            }
            top += SCOPE_H;
        }
        // the find bar
        if !self.settings {
            if let Some(f) = &self.find {
                let status = if f.query.trim().is_empty() {
                    "Type to find in the definition".to_string()
                } else if f.count == 0 {
                    "No matches".to_string()
                } else {
                    format!("{} of {}", f.cur.map_or(0, |c| c + 1), f.count)
                };
                let text = format!("Find: {}  \u{b7}  {status}", f.query);
                let img = ui.text(&ui.fit(&text, MAC_BODY_PT, ui.pt(width_pt - 2.0 * PAD)), MAC_BODY_PT, Weight::Regular, LABEL.now());
                canvas.fill_rect(RectF::new(0.0, ui.pt(top), cw as f32, ui.pt(SCOPE_H)), ROW_PRESSED.now());
                widgets::blit(canvas, &img, ui.pt(PAD) as i32, (ui.pt(top) + (ui.pt(SCOPE_H) - img.h as f32) / 2.0) as i32);
                top += SCOPE_H;
            }
        }
        let view_h = (height_pt - top - FOOTER_H).max(0.0);

        if self.settings {
            self.draw_settings(ctx, canvas, width_pt, top, view_h);
        } else {
            // the list, clipped under the field and above the footer
            let lines = self.lines(ui, width_pt);
            let content_h: f32 = lines.iter().map(|l| l.h).sum();
            self.scroll.set_max(content_h - view_h);
            let mut ys = Vec::with_capacity(lines.len());
            let mut acc = 0.0;
            for l in &lines {
                ys.push(acc);
                acc += l.h;
            }
            let matches: Vec<model::Match> = match &self.find {
                Some(f) => {
                    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
                    model::find_matches(&texts, &f.query)
                }
                None => Vec::new(),
            };
            let mut current = None;
            let mut scroll_target = None;
            if let Some(f) = &mut self.find {
                f.count = matches.len();
                if f.cur.is_some_and(|c| c >= matches.len()) {
                    f.cur = None;
                }
                current = f.cur.map(|c| matches[c]);
                if f.scroll_to {
                    f.scroll_to = false;
                    scroll_target = current.map(|m| m.line);
                }
            }
            if let Some(i) = scroll_target {
                self.scroll.jump_to((ys[i] - 8.0).max(0.0));
            }
            let offset = self.scroll.offset();
            let clip = RectF::new(0.0, ui.pt(top), cw as f32, ui.pt(view_h));
            canvas.push_clip(clip, 0.0);
            for (i, line) in lines.iter().enumerate() {
                let y = top + ys[i] - offset;
                if y + line.h > top && y < top + view_h && !line.text.is_empty() {
                    let left = ui.pt(PAD);
                    // the found words, the current one stronger
                    for m in matches.iter().filter(|m| m.line == i) {
                        let from = ui.measure(&line.text[..m.start], line.pt, line.weight);
                        let to = ui.measure(&line.text[..m.end], line.pt, line.weight);
                        let tint = if current == Some(*m) { MATERIAL_SELECTION.now() } else { ROW_PRESSED.now() };
                        canvas.fill_rrect(RectF::new(left + from, ui.pt(y), to - from, ui.pt(line.h)), ui.pt(3.0), tint);
                    }
                    // a definition line draws the words that have a definition of their own as links
                    let runs = if line.body {
                        model::link_runs(&line.text, |w| self.is_link(w))
                    } else {
                        vec![model::Run { start: 0, end: line.text.len(), link: false }]
                    };
                    for run in runs {
                        let piece = &line.text[run.start..run.end];
                        let img = ui.text(piece, line.pt, line.weight, if run.link { LINK } else { line.colour });
                        let x = left + if run.start == 0 { 0.0 } else { ui.measure(&line.text[..run.start], line.pt, line.weight) };
                        let ty = ui.pt(y) + (ui.pt(line.h) - img.h as f32) / 2.0;
                        widgets::blit(canvas, &img, x as i32, ty as i32);
                    }
                    if line.body {
                        self.body_rows.push((y, line.h, line.text.clone(), line.pt));
                    }
                }
            }
            canvas.pop_clip();
        }

        // the footer: what is installed, and any dictionary that could not be opened
        let footer = match &self.library {
            Some(lib) if !lib.is_empty() => {
                let names = lib.dictionaries.len();
                let mut text = format!("{names} dictionar{} \u{b7} {} words", if names == 1 { "y" } else { "ies" }, lib.word_count());
                if !lib.problems.is_empty() {
                    text.push_str(&format!(" \u{b7} {} could not be opened", lib.problems.len()));
                }
                text
            }
            _ => "No dictionaries installed".to_string(),
        };
        let img = ui.text(&ui.fit(&footer, MAC_FOOTNOTE_PT, ui.pt(width_pt - 2.0 * PAD)), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
        widgets::blit(canvas, &img, ui.pt(PAD) as i32, (ui.pt(height_pt - FOOTER_H) + (ui.pt(FOOTER_H) - img.h as f32) / 2.0) as i32);

        let mut d = Damage::new(cw as usize, ch as usize);
        d.all();
        d
    }

    /// The shared-memory path: the same routine on the CPU `Surface`, which implements `Canvas`.
    fn draw(&mut self, ctx: &Ctx, s: &mut bifrost_app::kit::Surface, t: f64) -> Damage {
        self.draw_canvas(ctx, s, t)
    }
}

fn main() {
    bifrost_app::run(DictionaryApp::new());
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn each_window_keeps_its_own_history() {
        let mut main = DictionaryApp::new();
        let mut other = DictionaryApp::new();
        main.look_up("cat");
        main.look_up("dog");
        assert!(main.history.can_back());
        assert!(!other.history.can_back());
        other.look_up("ox");
        assert!(!other.history.can_back(), "one look-up in a window leaves nothing to go back to");
        assert!(main.history.can_back());
    }

    #[test]
    fn the_look_up_intent_shows_its_word_in_a_window_that_has_its_dictionaries() {
        let mut app = DictionaryApp::new();
        app.settings = true;
        assert!(!app.show_from_intent("cat"), "a window that has not loaded its dictionaries stays as it is");
        assert!(app.shown.is_none() && app.settings);
        let dir = std::env::temp_dir().join(format!("bifrost-dictionary-intent-{}", std::process::id()));
        app.library = Some(load_library(&dir));
        assert!(app.show_from_intent("  cat "));
        assert!(!app.settings, "the settings page gives way to the answer");
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("cat"));
        assert_eq!(app.history.current(), Some("cat"));
        assert!(!app.show_from_intent("   "), "a blank word shows nothing");
    }

    #[test]
    fn a_closed_window_is_dropped_once_and_unknown_ids_change_nothing() {
        let mut app = DictionaryApp::new();
        app.windows.push(ExtraWindow { id: WindowId(1), app: DictionaryApp::new() });
        app.windows.push(ExtraWindow { id: WindowId(2), app: DictionaryApp::new() });
        assert!(!app.close_extra(WindowId(9)));
        assert_eq!(app.windows.len(), 2);
        assert!(app.close_extra(WindowId(1)));
        assert!(!app.close_extra(WindowId(1)), "a window is removed once");
        assert_eq!(app.windows.iter().map(|e| e.id).collect::<Vec<_>>(), vec![WindowId(2)]);
    }
}
