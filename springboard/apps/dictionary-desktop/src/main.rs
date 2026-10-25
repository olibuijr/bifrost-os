//! Dictionary: looks words up in the StarDict dictionaries installed in the app's `dictionaries` folder, and in the
//! system's `/usr/share/stardict` folder. Type a word to see its suggestions, Return to look it up, or Look Up
//! Clipboard. The `look-up` intent returns a definition to other callers.

use bifrost_app::intents::{self, Intent, IntentResult, Param, ParamType};
use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::kit::canvas::{Canvas, RectF};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Rect, TextField};
use bifrost_app::kit::ui::{
    Ui, Weight, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, LINK, MAC_BODY_PT, MAC_HEADLINE_PT, MAC_SUBHEAD_PT, MAC_TITLE1_PT, MATERIAL_SELECTION,
    ROW_PRESSED, SEGMENT_ON, SEPARATOR_C, SPACE_XS,
};
use bifrost_app::kit::{Damage, Scroller};
use bifrost_app::widgets;
use bifrost_app::app_windows::{WindowId, WindowSpec};
use bifrost_app::services::ServiceCall;
use bifrost_app::{App, AppMenu, AppMenuItem, Command, ContextItem, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use bifrost_ui::stardict::{load_library, Hit, Library};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bifrost_dictionary::model::{self, layout, History, Prefs};

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

/// The size of a window opened with File > New Window, in points: wide enough for the sidebar
/// (`layout::SIDEBAR_MIN_WINDOW_PT`).
const NEW_WINDOW_PT: (f32, f32) = (720.0, 560.0);

/// Most matching headwords listed in the sidebar.
const SUGGESTIONS: usize = 60;

/// Layout in points (the window regions are `model::layout`). Side padding of the content column.
const PAD: f32 = layout::PAD;
/// Height of a sidebar row and of a match row in a window too narrow for the sidebar.
const ROW_H: f32 = 28.0;
/// Height of a sidebar section header (unused: the sidebar has no headers, but `SourceList` needs one).
const SIDEBAR_HEADER_H: f32 = 22.0;
/// The senses of a part of speech are indented under its label, as in the reference.
const SENSE_INDENT: f32 = 12.0;
/// Line advance of wrapped text, as a multiple of its size.
const LEADING: f32 = 1.35;
/// Height of the scope bar and the find bar under the top bar, in points.
const SCOPE_H: f32 = 28.0;
/// Height of a row in the settings page.
const SETTINGS_ROW_H: f32 = 32.0;

/// Toolbar item ids.
const TB_BACK: u32 = 1;
const TB_FORWARD: u32 = 2;
const TB_TEXT_SIZE: u32 = 3;

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
    /// The toolbar's text size pull-down; its menu opens at this point under the button.
    TextSize(f32, f32),
    /// The search field's clear button.
    ClearSearch,
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
    /// Points from the content column's padding to the text.
    indent: f32,
    /// The cross-references on the line: a byte range of `text` and the term a tap on it looks up. Only the terms a
    /// cross-reference line names (`model::cross_references`) that some dictionary defines are links.
    links: Vec<(usize, usize, String)>,
    /// Secondary text drawn after `text` in grey: the headword's pronunciation.
    trail: Option<String>,
}

impl Line {
    fn plain(text: String, pt: f32, weight: Weight, colour: u32, h: f32) -> Line {
        Line { text, pt, weight, colour, h, indent: 0.0, links: Vec::new(), trail: None }
    }
}

/// A line drawn with links in the last frame, for taps: its top, height and left edge in points, its text, size and links.
struct LinkRow {
    y: f32,
    h: f32,
    x: f32,
    text: String,
    pt: f32,
    links: Vec<(usize, usize, String)>,
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
    /// The sidebar's own scroll, under the traffic lights' band: its rows scroll by wheel and drag when they overflow.
    side_scroll: Scroller,
    /// The touch that is down started in the sidebar, so it drags `side_scroll` and not the content.
    side_touch: bool,
    /// The next frame scrolls the sidebar so its selected row is in view (set when the word on screen changes).
    reveal_selection: bool,
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
    /// The lines with links on screen in the last frame.
    link_rows: Vec<LinkRow>,
    /// The sidebar's bounds in the last frame (zero wide when the window is too narrow for one), in points.
    sidebar: Rect,
    /// Top of the match rows in the content column in the last frame, when the window has no sidebar and lists them there.
    match_rows_top: Option<f32>,
    /// The toolbar offers the text size pull-down. Only the main window: a context menu opens in it.
    text_size_menu: bool,
    /// The settings row being dragged, if any.
    drag: Option<Drag>,
    /// Top of the first dictionary row of the settings page in the last frame, in points.
    rows_top: f32,
    /// Whether a word has a definition in an enabled dictionary; links in definitions use it.
    known: HashMap<String, bool>,
    /// The windows opened with File > New Window, in the order they were opened. The main window is `self`.
    windows: Vec<ExtraWindow>,
    /// A word asked for before the dictionaries were loaded (a look-up intent, a Services call or a launch argument
    /// on a cold launch: the control socket answers before the first Configure). `configure` shows it.
    pending_word: Option<String>,
}

/// A window opened with File > New Window. It is a whole lookup of its own (query, answer, history, scope, find), so
/// Back and Forward in one window never move another.
struct ExtraWindow {
    id: WindowId,
    app: DictionaryApp,
}

/// The part of the sidebar its rows are drawn in: below the `bar_h` band of the traffic lights, in points.
fn sidebar_rows(sidebar: Rect, bar_h: f32) -> Rect {
    let band = bar_h.clamp(0.0, sidebar.h.max(0.0));
    Rect { x: sidebar.x, y: sidebar.y + band, w: sidebar.w, h: sidebar.h - band }
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
            side_scroll: Scroller::new(),
            side_touch: false,
            reveal_selection: false,
            prefs: Prefs::default(),
            prefs_path: PathBuf::new(),
            history: History::default(),
            scope: None,
            find: None,
            settings: false,
            rename: None,
            hot: Vec::new(),
            link_rows: Vec::new(),
            sidebar: Rect::default(),
            match_rows_top: None,
            text_size_menu: true,
            drag: None,
            rows_top: 0.0,
            known: HashMap::new(),
            windows: Vec::new(),
            pending_word: None,
        }
    }

    /// Opens a window with an empty lookup. It loads the dictionaries and the stored choices the way the main window does.
    fn open_new_window(&mut self, ctx: &Ctx) -> bool {
        let (w, h) = NEW_WINDOW_PT;
        let Ok(id) = ctx.open_window(WindowSpec::new("Dictionary", w, h)) else { return false };
        let mut app = DictionaryApp::new();
        app.text_size_menu = false;
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
            // opened by `touch`, which has the context the menu needs
            Act::TextSize(..) => false,
            Act::ClearSearch => {
                self.query.clear();
                self.edited()
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
        self.side_scroll.jump_top();
        self.preview_first();
        true
    }

    /// While a word is typed, the first match is selected in the sidebar and its entry shown, as in the reference, so
    /// nothing waits for Return. The field keeps what was typed, and the history records the word only once it is
    /// chosen (Return or a tap). A window without a sidebar lists the matches instead.
    fn preview_first(&mut self) {
        if self.sidebar.w <= 0.0 || self.query.trim().is_empty() {
            return;
        }
        let Some(word) = self.suggestions.first().cloned() else { return };
        let raw = self.library.as_ref().map(|l| l.lookup(&word)).unwrap_or_default();
        let hits = self.prefs.arrange(raw);
        if let Some(s) = &self.scope {
            if !hits.iter().any(|h| &h.dictionary == s) {
                self.scope = None;
            }
        }
        self.shown = Some(Shown { word, hits });
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
        // the sidebar keeps the search's matches when the word is one of them (a tap on a row, Back to a listed word);
        // a word from elsewhere (a link, another app, the history) lists the headwords that match it, selected
        if !self.suggestions.iter().any(|w| w.eq_ignore_ascii_case(word)) {
            self.suggestions = self.library.as_ref().map(|l| l.suggest(word, SUGGESTIONS)).unwrap_or_default();
            self.side_scroll.jump_top();
        }
        self.reveal_selection = true;
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

    /// Shows `word` now when the dictionaries are loaded, else keeps it for `configure` to show once they are, so a
    /// look-up that starts the app (Spotlight's dictionary hit, Open in Dictionary) lands on its word. True when the
    /// screen needs a frame.
    fn show_or_queue(&mut self, word: &str) -> bool {
        if self.library.is_some() {
            return self.show_from_intent(word);
        }
        let word = word.trim();
        if !word.is_empty() {
            self.pending_word = Some(word.to_string());
        }
        false
    }

    /// Shows the word asked for before the dictionaries were loaded (cold launch from Spotlight or another app), once
    /// they are. True when it was shown.
    fn show_pending(&mut self) -> bool {
        if self.library.is_none() {
            return false;
        }
        match self.pending_word.take() {
            Some(word) => self.show_from_intent(&word),
            None => false,
        }
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

    /// The sidebar: the headwords that match the query, the one on screen selected.
    fn source_list(&self) -> SourceList {
        let rows = self.suggestions.iter().enumerate().map(|(i, w)| SidebarItem::row(i as u32, w)).collect();
        let mut list = SourceList::new(rows, ROW_H, SIDEBAR_HEADER_H);
        let on = self.shown.as_ref().and_then(|s| self.suggestions.iter().position(|w| w.eq_ignore_ascii_case(&s.word)));
        list.set_selected(on.map(|i| i as u32));
        list
    }

    /// The lines of the content column, in order, for a column `width` points wide. `list_matches` lists the matching
    /// headwords as rows (a window too narrow for the sidebar).
    fn lines(&mut self, ui: &Ui, width: f32, list_matches: bool) -> Vec<Line> {
        let mut out = Vec::new();
        let size = self.prefs.scaled(MAC_BODY_PT);
        let wrap_px = |indent: f32| ui.pt((width - 2.0 * PAD - indent).max(PAD));
        let message = |out: &mut Vec<Line>, text: &str, colour: u32| {
            for line in ui.wrap(text, size, Weight::Regular, wrap_px(0.0)) {
                out.push(Line::plain(line, size, Weight::Regular, colour, size * LEADING));
            }
        };
        let Some(word) = self.shown.as_ref().map(|s| s.word.clone()) else {
            if list_matches && !self.suggestions.is_empty() {
                for w in &self.suggestions {
                    out.push(Line::plain(w.clone(), size, Weight::Regular, LABEL.now(), ROW_H));
                }
            } else if self.query.trim().is_empty() {
                message(&mut out, "Type a word to look it up.", LABEL_2.now());
            } else if self.suggestions.is_empty() {
                message(&mut out, &format!("No words start with \u{201c}{}\u{201d}.", self.query.trim()), LABEL_2.now());
            } else {
                message(&mut out, "Choose a word in the sidebar, or press Return to look it up.", LABEL_2.now());
            }
            return out;
        };
        let hits = self.visible_hits();
        if hits.is_empty() {
            message(&mut out, &self.no_answer(&word), LABEL_2.now());
            return out;
        }
        // the headword, with the first pronunciation a dictionary gives
        let head_pt = self.prefs.scaled(MAC_TITLE1_PT);
        let pron = hits.iter().find_map(|h| model::pronunciation(&h.text).0);
        out.push(Line { trail: pron.map(|p| format!("| {p} |")), ..Line::plain(word.clone(), head_pt, Weight::Bold, LABEL.now(), head_pt * LEADING + 8.0) });
        let several = hits.len() > 1;
        for hit in &hits {
            if several {
                out.push(Line::plain(hit.dictionary.clone(), MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), 24.0));
            }
            let (_, text) = model::pronunciation(&hit.text);
            for paragraph in text.split('\n') {
                let para = model::normalize(paragraph);
                if para.is_empty() {
                    out.push(Line::plain(String::new(), size, Weight::Regular, LABEL.now(), 8.0));
                    continue;
                }
                if model::is_part_of_speech(&para) {
                    out.push(Line::plain(para, size, Weight::Regular, LABEL_2.now(), size * LEADING));
                    continue;
                }
                // links only for the terms of a cross-reference line that a dictionary defines
                let refs: Vec<(usize, usize)> = model::cross_references(&para).into_iter().filter(|&(s, e)| self.is_link(&para[s..e])).collect();
                let wrapped = ui.wrap(&para, size, Weight::Regular, wrap_px(SENSE_INDENT));
                let starts = model::line_starts(&para, &wrapped);
                for (line, start) in wrapped.into_iter().zip(starts) {
                    let links = model::clip_links(start, line.len(), &refs).into_iter().map(|(a, b, i)| (a, b, para[refs[i].0..refs[i].1].to_string())).collect();
                    out.push(Line { indent: SENSE_INDENT, links, ..Line::plain(line, size, Weight::Regular, LABEL.now(), size * LEADING) });
                }
            }
            out.push(Line::plain(String::new(), size, Weight::Regular, LABEL.now(), 18.0));
        }
        out
    }

    fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let (px, py) = to_points(ctx, x, y);
        match kind {
            TouchKind::Down => {
                if self.settings && self.rename.is_none() && px >= self.sidebar.w && self.hit_test(px, py).is_none() {
                    if let Some(from) = model::row_at(py, self.rows_top, SETTINGS_ROW_H, self.prefs.dicts.len()) {
                        self.drag = Some(Drag { from, y: py });
                        return true;
                    }
                }
                // a touch (or the wheel's emulated drag) in the sidebar scrolls the sidebar, anywhere else the content
                self.side_touch = self.sidebar.w > 0.0 && self.sidebar.contains(px, py);
                self.active_scroll().down(py, t);
                false
            }
            TouchKind::Move => {
                if let Some(d) = &mut self.drag {
                    d.y = py;
                    return true;
                }
                self.active_scroll().moved(py, t);
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
                if self.active_scroll().up(py, t) {
                    if let Some(act) = self.hit_test(px, py) {
                        if let Act::TextSize(ax, ay) = act {
                            self.open_text_size_menu(ctx, ax, ay);
                            return false;
                        }
                        return self.perform(act);
                    }
                    if self.tap_sidebar(px, py) {
                        return true;
                    }
                    if self.tap_definition(ctx, px, py) {
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
                self.active_scroll().up(py, t);
                true
            }
        }
    }

    /// The scroller the touch that is down drags: the sidebar's when it started there, else the content's.
    fn active_scroll(&mut self) -> &mut Scroller {
        if self.side_touch { &mut self.side_scroll } else { &mut self.scroll }
    }

    /// The scroll offset the sidebar's rows are drawn and hit with: its own scroll, rows starting under the bar band.
    fn sidebar_scroll(&self) -> f32 {
        self.side_scroll.offset() - bifrost_app::layout::UNIFIED_BAR_PT
    }

    /// The toolbar's text size pull-down (`NSMenuToolbarItem`): its menu opens under the button, at (`x`, `y`) points, and
    /// the choice comes back as `Input::ContextAction` with the View menu's ids.
    fn open_text_size_menu(&self, ctx: &Ctx, x: f32, y: f32) {
        let s = ctx.ui.pt(1.0);
        ctx.context_menu(
            x * s,
            y * s,
            vec![
                ContextItem::new(MENU_BIGGER, "Bigger Text").key("cmd+=").enabled(self.prefs.step < model::MAX_STEP),
                ContextItem::new(MENU_SMALLER, "Smaller Text").key("cmd+-").enabled(self.prefs.step > model::MIN_STEP),
                ContextItem::new(MENU_ACTUAL_SIZE, "Actual Size").key("cmd+0").enabled(self.prefs.step != 0),
            ],
        );
    }

    /// A tap on a sidebar row (points) looks its headword up.
    fn tap_sidebar(&mut self, px: f32, py: f32) -> bool {
        if self.sidebar.w <= 0.0 || !self.sidebar.contains(px, py) {
            return false;
        }
        // the band under the traffic lights is not a row, even with rows scrolled under it
        if py < self.sidebar.y + bifrost_app::layout::UNIFIED_BAR_PT {
            return true;
        }
        match self.source_list().hit(self.sidebar, self.sidebar_scroll(), px, py) {
            Some(i) => match self.suggestions.get(i as usize).cloned() {
                Some(word) => {
                    self.settings = false;
                    self.look_up(&word)
                }
                None => true,
            },
            None => true,
        }
    }

    /// A tap on a cross-reference in a definition (points) looks the term up.
    fn tap_definition(&mut self, ctx: &Ctx, px: f32, py: f32) -> bool {
        if self.settings || self.shown.is_none() {
            return false;
        }
        let Some(row) = self.link_rows.iter().find(|r| py >= r.y && py < r.y + r.h) else { return false };
        let tap = ctx.ui.pt(px - row.x);
        let target = row.links.iter().find_map(|(a, b, term)| {
            let from = ctx.ui.measure(&row.text[..*a], row.pt, Weight::Regular);
            let to = ctx.ui.measure(&row.text[..*b], row.pt, Weight::Regular);
            (tap >= from && tap < to).then(|| term.clone())
        });
        match target {
            Some(term) => self.look_up(&term),
            None => false,
        }
    }

    /// A tap at `py` (points) on the match rows of a window without a sidebar looks that word up.
    fn tap_row(&mut self, py: f32) -> bool {
        let Some(top) = self.match_rows_top else { return true };
        if self.shown.is_some() || self.settings || py < top {
            return true;
        }
        let row = ((py - top + self.scroll.offset()) / ROW_H).floor();
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
            // Return chooses the word on screen (the first match while typing), else looks the typed text up
            KeyInput::Code(13) => {
                let word = self.shown.as_ref().map(|s| s.word.clone()).unwrap_or_else(|| self.query.clone());
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
    /// `left` and `width_pt` are the content column's left edge and width, in points.
    fn draw_settings(&mut self, ctx: &Ctx, canvas: &mut dyn Canvas, left: f32, width_pt: f32, top: f32, view_h: f32) {
        let ui = &ctx.ui;
        canvas.push_clip(RectF::new(ui.pt(left), ui.pt(top), ui.pt(width_pt), ui.pt(view_h)), 0.0);
        let mut y = top;
        let pad = left + PAD;
        let right = left + width_pt - PAD;
        text_at(canvas, ui, "Dictionaries", MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now(), pad, y, width_pt - 2.0 * PAD);
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
                    canvas.fill_rect(RectF::new(ui.pt(left), ui.pt(y), ui.pt(width_pt), ui.pt(SETTINGS_ROW_H)), ROW_PRESSED.now());
                }
            }
            let renaming = self.rename.as_ref().and_then(|(r, buf)| (*r == i).then(|| buf.clone()));
            button(canvas, ui, &mut self.hot, pad, y + 4.0, 36.0, SETTINGS_ROW_H - 8.0, if d.enabled { "On" } else { "Off" }, Some(Act::Toggle(i)), d.enabled);
            let buttons_w = 3.0 * 56.0 + 2.0 * 4.0;
            let name_x = pad + 36.0 + 12.0;
            let name_w = (right - buttons_w - name_x - 8.0).max(40.0);
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
            let bx = right - buttons_w;
            button(canvas, ui, &mut self.hot, bx, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "Up", (i > 0).then_some(Act::Up(i)), false);
            button(canvas, ui, &mut self.hot, bx + 60.0, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "Down", (i + 1 < count).then_some(Act::Down(i)), false);
            button(canvas, ui, &mut self.hot, bx + 120.0, y + 4.0, 56.0, SETTINGS_ROW_H - 8.0, "Rename", Some(Act::Rename(i)), false);
        }
        y = rows_top + count as f32 * SETTINGS_ROW_H;
        if count == 0 {
            text_at(canvas, ui, "No dictionary is installed yet.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), pad, y, width_pt - 2.0 * PAD);
            y += SETTINGS_ROW_H;
        }
        y += SETTINGS_ROW_H / 2.0;
        let size_text = format!("Text size: {}%", 100 + 10 * self.prefs.step);
        text_at(canvas, ui, &size_text, MAC_BODY_PT, Weight::Regular, LABEL.now(), pad, y, width_pt - 2.0 * PAD - 120.0);
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
        self.show_pending();
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
            // the toolbar's text size pull-down
            Input::ContextAction(id @ (MENU_BIGGER | MENU_SMALLER | MENU_ACTUAL_SIZE)) => self.menu_action(ctx, id),
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
        let word = model::service_word(&text).ok_or_else(|| RpcError::new(INVALID_PARAMS, "the selection has no word to look up"))?;
        // before the first Configure (the service started the app) the word waits for the window
        if ctx.is_none() || self.library.is_none() {
            self.pending_word = Some(word);
        } else {
            self.show_from_intent(&word);
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
        // Open in Dictionary row) lands in the app's own window with its history, scope bar and find. On a cold launch
        // the intent arrives before the first Configure (no context, no dictionaries yet): the word waits for it.
        if ctx.is_some() {
            self.show_or_queue(&word);
        } else {
            self.pending_word = Some(word.clone());
        }
        Ok(IntentResult::value(Json::Str(text.clone())).with_dialog(&format!("{word}: {text}")))
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let mut redraw = false;
        if self.scroll.animating() {
            self.scroll.tick(t);
            redraw = true;
        }
        if self.side_scroll.animating() {
            self.side_scroll.tick(t);
            redraw = true;
        }
        for e in &mut self.windows {
            redraw |= e.app.tick(ctx, t);
        }
        redraw
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        let own = (self.scroll.animating() || self.side_scroll.animating()).then_some(t + 16.0);
        self.windows.iter().filter_map(|e| e.app.next_wake(t)).chain(own).reduce(f64::min)
    }

    fn window_event(&mut self, ctx: &Ctx, window: WindowId, ev: Input) -> bool {
        if window.is_main() {
            return self.event(ctx, ev);
        }
        // the keyboard, the menus and the clipboard belong to the main window; a window takes its touches
        let Some(e) = self.windows.iter_mut().find(|e| e.id == window) else { return false };
        match ev {
            // an extra window's touches arrive in window points (`WindowTouch`); `event` takes px, as the main window's
            Input::Touch { .. } => e.app.event(ctx, bifrost_app::app_windows::window_touch_in_px(ev, ctx.ui.pt(1.0))),
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
        self.link_rows.clear();
        self.match_rows_top = None;
        let bar_h = bifrost_app::layout::UNIFIED_BAR_PT;
        let regions = layout::regions(width_pt, height_pt, bar_h, widgets::toolbar_lead_pt());
        self.sidebar = regions.sidebar;
        let content = regions.content;

        canvas.fill_rect(RectF::new(0.0, 0.0, cw as f32, ch as f32), CONTENT_BG.now());

        // the sidebar: the matching headwords under the traffic lights, the one on screen selected
        if regions.sidebar.w > 0.0 {
            let list = self.source_list();
            // the rows scroll in the column under the bar band; a new word on screen brings its row into view
            let rows_h = (regions.sidebar.h - bar_h).max(0.0);
            self.side_scroll.set_max(list.content_height() - rows_h);
            if std::mem::take(&mut self.reveal_selection) {
                let to = list.scroll_to_selected(rows_h, self.side_scroll.offset());
                if to != self.side_scroll.offset() {
                    self.side_scroll.jump_to(to);
                }
            }
            // the band under the traffic lights keeps the sidebar's material, but the rows are clipped below it, so a
            // scrolled row never draws under the traffic lights (the material is written, not blended, so the second
            // pass over the rows' area leaves it as the first)
            widgets::source_list(canvas, ui, regions.sidebar, &SourceList::new(Vec::new(), ROW_H, SIDEBAR_HEADER_H), 0.0, None);
            let rows = sidebar_rows(regions.sidebar, bar_h);
            canvas.push_clip(RectF::new(ui.pt(rows.x), ui.pt(rows.y), ui.pt(rows.w), ui.pt(rows.h)), 0.0);
            widgets::source_list(canvas, ui, regions.sidebar, &list, self.sidebar_scroll(), None);
            canvas.pop_clip();
            canvas.fill_rect(RectF::new(ui.pt(regions.sidebar.w) - 1.0, 0.0, 1.0, ch as f32), SEPARATOR_C.now());
        }

        // the title and what was found, as the reference's "Dictionary / 33 found"
        let subtitle = match &self.library {
            Some(lib) if !lib.problems.is_empty() => format!("{} could not be opened", lib.problems.len()),
            Some(lib) if lib.is_empty() => "No dictionaries installed".to_string(),
            _ if !self.suggestions.is_empty() => format!("{} found", self.suggestions.len()),
            // short enough for the title column: "151,378 words" (the dictionaries are listed in Settings)
            Some(lib) => model::idle_subtitle(lib.word_count()),
            None => String::new(),
        };
        let t = regions.title;
        if t.w > 0.0 {
            let title = ui.text(&ui.fit("Dictionary", MAC_HEADLINE_PT, ui.pt(t.w)), MAC_HEADLINE_PT, Weight::Semibold, LABEL.now());
            let sub = ui.text(&ui.fit(&subtitle, MAC_SUBHEAD_PT, ui.pt(t.w)), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
            let top = ui.pt(t.y) + (ui.pt(t.h) - (title.h + sub.h) as f32) / 2.0;
            widgets::blit(canvas, &title, ui.pt(t.x) as i32, top as i32);
            widgets::blit(canvas, &sub, ui.pt(t.x) as i32, (top + title.h as f32) as i32);
        }

        // the toolbar: Back and Forward in one capsule, then the text size pull-down, right-aligned before the search field
        let mut items = vec![
            ToolbarItem::flexible_space(),
            ToolbarItem::icon(TB_BACK, "Back", "chevron.left").in_group(1).enabled(self.history.can_back()),
            ToolbarItem::icon(TB_FORWARD, "Forward", "chevron.right").in_group(1).enabled(self.history.can_forward()),
        ];
        if self.text_size_menu {
            items.push(ToolbarItem::menu(TB_TEXT_SIZE, "Text Size", Some("textformat")));
        }
        let bar = Toolbar::new(items);
        if regions.tools.w > 0.0 {
            let placed = widgets::toolbar(canvas, ui, regions.tools, &bar, None);
            for (id, r) in &placed.visible {
                let act = match *id {
                    TB_BACK if self.history.can_back() => Act::Back,
                    TB_FORWARD if self.history.can_forward() => Act::Forward,
                    TB_TEXT_SIZE => {
                        let (ax, ay) = placed.menu_anchor(TB_TEXT_SIZE, 4.0).unwrap_or((r.x, r.y + r.h));
                        Act::TextSize(ax, ay)
                    }
                    _ => continue,
                };
                self.hot.push(Hot { x: r.x, y: r.y, w: r.w, h: r.h, act });
            }
        }

        // the search field: typing always goes to it while no find or settings page has the keys
        let s = regions.search;
        if s.w > 0.0 {
            let mut field = TextField::new();
            field.push_str(&self.query);
            widgets::search_field(canvas, ui, &field, s, "Search", !self.settings && self.find.is_none());
            if let Some(c) = bifrost_app::controls::search::layout(s, !self.query.is_empty()).clear {
                self.hot.push(Hot { x: c.x, y: c.y, w: c.w, h: c.h, act: Act::ClearSearch });
            }
        }

        let left = content.x + PAD;
        let mut top = content.y;
        let gap = 4.0;
        // the scope bar: All and each dictionary that answered
        if !self.settings && self.show_scope_bar() {
            let mut x = left;
            let mut entries: Vec<(String, Option<String>)> = vec![("All".to_string(), None)];
            entries.extend(self.scope_names().into_iter().map(|n| (n.clone(), Some(n))));
            for (label, scope) in entries {
                let w = ui.measure(&label, MAC_SUBHEAD_PT, Weight::Regular) / scale + 16.0;
                if x + w > content.x + content.w - PAD {
                    break;
                }
                let on = self.scope == scope;
                button(canvas, ui, &mut self.hot, x, top + 2.0, w, SCOPE_H - 4.0, &label, Some(Act::Scope(scope)), on);
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
                let img = ui.text(&ui.fit(&text, MAC_BODY_PT, ui.pt(content.w - 2.0 * PAD)), MAC_BODY_PT, Weight::Regular, LABEL.now());
                canvas.fill_rect(RectF::new(ui.pt(content.x), ui.pt(top), ui.pt(content.w), ui.pt(SCOPE_H)), ROW_PRESSED.now());
                widgets::blit(canvas, &img, ui.pt(left) as i32, (ui.pt(top) + (ui.pt(SCOPE_H) - img.h as f32) / 2.0) as i32);
                top += SCOPE_H;
            }
        }
        // a hairline under the bars, as under the reference's scope bar
        canvas.fill_rect(RectF::new(ui.pt(content.x), ui.pt(top) - 1.0, ui.pt(content.w), 1.0), SEPARATOR_C.now());
        top += PAD;
        let view_h = (content.y + content.h - top).max(0.0);

        if self.settings {
            self.draw_settings(ctx, canvas, content.x, content.w, top, view_h);
        } else {
            // the entry (or the matches, in a window too narrow for the sidebar), clipped to the content column
            let list_matches = regions.sidebar.w <= 0.0;
            let lines = self.lines(ui, content.w, list_matches);
            if list_matches && self.shown.is_none() && !self.suggestions.is_empty() {
                self.match_rows_top = Some(top);
            }
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
            let clip = RectF::new(ui.pt(content.x), ui.pt(top), ui.pt(content.w), ui.pt(view_h));
            canvas.push_clip(clip, 0.0);
            for (i, line) in lines.iter().enumerate() {
                let y = top + ys[i] - offset;
                if y + line.h > top && y < top + view_h && !line.text.is_empty() {
                    let x_pt = left + line.indent;
                    let x0 = ui.pt(x_pt);
                    // the found words, the current one stronger
                    for m in matches.iter().filter(|m| m.line == i) {
                        let from = ui.measure(&line.text[..m.start], line.pt, line.weight);
                        let to = ui.measure(&line.text[..m.end], line.pt, line.weight);
                        let tint = if current == Some(*m) { MATERIAL_SELECTION.now() } else { ROW_PRESSED.now() };
                        canvas.fill_rrect(RectF::new(x0 + from, ui.pt(y), to - from, ui.pt(line.h)), ui.pt(3.0), tint);
                    }
                    // plain text, with the cross-references of the line as links
                    let spans: Vec<(usize, usize)> = line.links.iter().map(|(a, b, _)| (*a, *b)).collect();
                    for run in model::runs(line.text.len(), &spans) {
                        let piece = &line.text[run.start..run.end];
                        let img = ui.text(piece, line.pt, line.weight, if run.link { LINK } else { line.colour });
                        let x = x0 + if run.start == 0 { 0.0 } else { ui.measure(&line.text[..run.start], line.pt, line.weight) };
                        let ty = ui.pt(y) + (ui.pt(line.h) - img.h as f32) / 2.0;
                        widgets::blit(canvas, &img, x as i32, ty as i32);
                    }
                    if let Some(trail) = &line.trail {
                        let img = ui.text(trail, line.pt, Weight::Regular, LABEL_2.now());
                        let x = x0 + ui.measure(&line.text, line.pt, line.weight) + ui.pt(PAD);
                        let ty = ui.pt(y) + (ui.pt(line.h) - img.h as f32) / 2.0;
                        widgets::blit(canvas, &img, x as i32, ty as i32);
                    }
                    if !line.links.is_empty() {
                        self.link_rows.push(LinkRow { y, h: line.h, x: x_pt, text: line.text.clone(), pt: line.pt, links: line.links.clone() });
                    }
                }
            }
            canvas.pop_clip();
        }

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
    let mut app = DictionaryApp::new();
    // `bifrost-dictionary <word>` or a `dict:` URL: the first frame shows that word
    app.pending_word = model::launch_word(std::env::args().skip(1));
    bifrost_app::run(app);
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
    fn the_sidebar_lists_the_matches_with_the_word_on_screen_selected() {
        let mut app = DictionaryApp::new();
        app.suggestions = vec!["computer".into(), "computer-aided".into()];
        assert_eq!(app.source_list().selected(), None, "nothing is selected while the query is typed");
        app.shown = Some(Shown { word: "Computer".into(), hits: Vec::new() });
        let list = app.source_list();
        assert_eq!(list.items.len(), 2);
        assert_eq!(list.selected(), Some(0));
        app.shown = Some(Shown { word: "abacus".into(), hits: Vec::new() });
        assert_eq!(app.source_list().selected(), None, "a word that is not in the list selects no row");
    }

    #[test]
    fn a_tap_on_a_sidebar_row_looks_its_word_up_and_the_content_column_is_left_alone() {
        let mut app = DictionaryApp::new();
        app.suggestions = vec!["computer".into(), "computer-aided".into()];
        app.sidebar = Rect { x: 0.0, y: 0.0, w: 200.0, h: 600.0 };
        assert!(!app.tap_sidebar(300.0, 70.0), "a tap right of the sidebar is not the sidebar's");
        assert!(app.shown.is_none());
        // the rows start under the traffic lights' band
        assert!(app.tap_sidebar(50.0, bifrost_app::layout::UNIFIED_BAR_PT + ROW_H * 0.5));
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("computer"));
        assert_eq!(app.history.current(), Some("computer"));
        app.sidebar = Rect::default();
        assert!(!app.tap_sidebar(50.0, 70.0), "a window without a sidebar takes no sidebar taps");
    }

    #[test]
    fn a_tap_on_a_sidebar_row_keeps_the_searchs_matches() {
        let mut app = DictionaryApp::new();
        let matches: Vec<String> = vec!["comp".into(), "computer".into(), "computer-aided".into()];
        app.suggestions = matches.clone();
        app.sidebar = Rect { x: 0.0, y: 0.0, w: 200.0, h: 600.0 };
        assert!(app.tap_sidebar(50.0, bifrost_app::layout::UNIFIED_BAR_PT + ROW_H * 1.5));
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("computer"));
        assert_eq!(app.suggestions, matches, "the list is the search's, not the matches of the chosen word");
        assert!(app.reveal_selection, "the selected row is brought into view on the next frame");
        assert_eq!(app.source_list().selected(), Some(1));
        // a word that is not listed (a link, another app) lists its own matches
        app.show("abacus", true);
        assert!(app.suggestions.is_empty(), "no library: nothing matches");
    }

    #[test]
    fn a_scrolled_sidebar_hits_the_row_under_the_pointer_and_not_the_bar_band() {
        let mut app = DictionaryApp::new();
        app.suggestions = (0..40).map(|i| format!("word{i}")).collect();
        app.sidebar = Rect { x: 0.0, y: 0.0, w: 200.0, h: 300.0 };
        let list = app.source_list();
        let rows_h = app.sidebar.h - bifrost_app::layout::UNIFIED_BAR_PT;
        app.side_scroll.set_max(list.content_height() - rows_h);
        assert!(app.side_scroll.max() > 0.0, "40 rows overflow the column");
        app.side_scroll.jump_to(ROW_H * 10.0);
        assert!(app.tap_sidebar(50.0, bifrost_app::layout::UNIFIED_BAR_PT + ROW_H * 0.5));
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("word10"));
        // the band under the traffic lights takes the tap without choosing the row scrolled beneath it
        assert!(app.tap_sidebar(50.0, bifrost_app::layout::UNIFIED_BAR_PT * 0.5));
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("word10"));
        // a selection below the column scrolls into view
        app.shown = Some(Shown { word: "word30".into(), hits: Vec::new() });
        let to = app.source_list().scroll_to_selected(rows_h, app.side_scroll.offset());
        assert!(to + rows_h >= ROW_H * 31.0 && to <= ROW_H * 30.0);
    }

    /// E2E9 2: a look-up that starts the app arrives before the dictionaries are loaded; the word waits and is shown then.
    #[test]
    fn a_word_asked_for_before_the_dictionaries_load_is_shown_once_they_are() {
        let mut app = DictionaryApp::new();
        assert!(!app.show_or_queue("  terminal "));
        assert!(app.shown.is_none());
        assert_eq!(app.pending_word.as_deref(), Some("terminal"));
        assert!(!app.show_pending(), "still nothing to show it with");
        assert_eq!(app.pending_word.as_deref(), Some("terminal"), "the word keeps waiting");
        let dir = std::env::temp_dir().join(format!("bifrost-dictionary-pending-{}", std::process::id()));
        app.library = Some(load_library(&dir));
        app.settings = true;
        assert!(app.show_pending());
        assert!(!app.settings);
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("terminal"));
        assert_eq!(app.query, "terminal", "the search field shows the word");
        assert!(app.pending_word.is_none() && !app.show_pending(), "shown once");
        assert!(!app.show_or_queue("   ") && app.pending_word.is_none(), "a blank word is not kept");
        // a running app shows a later word at once
        assert!(app.show_or_queue("orange"));
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("orange"));
    }

    /// E2E9 10: typing selects the first match and shows its entry; Return chooses it.
    #[test]
    fn typing_selects_and_shows_the_first_match() {
        let mut app = DictionaryApp::new();
        app.sidebar = Rect { x: 0.0, y: 0.0, w: 200.0, h: 600.0 };
        app.query = "comp".into();
        app.suggestions = vec!["comp".into(), "compact".into(), "computer".into()];
        app.preview_first();
        assert_eq!(app.shown.as_ref().map(|s| s.word.as_str()), Some("comp"));
        assert_eq!(app.source_list().selected(), Some(0), "the first row is selected");
        assert_eq!(app.query, "comp", "the field keeps what was typed");
        assert_eq!(app.history.current(), None, "typing records nothing");
        app.suggestions = vec!["compact".into(), "computer".into()];
        app.query = "compa".into();
        app.preview_first();
        assert!(app.key(&KeyInput::Code(13)));
        assert_eq!(app.history.current(), Some("compact"), "Return chooses the word on screen");
        // a blank query, no matches or no sidebar show nothing
        let mut narrow = DictionaryApp::new();
        narrow.query = "comp".into();
        narrow.suggestions = vec!["comp".into()];
        narrow.preview_first();
        assert!(narrow.shown.is_none(), "a window without a sidebar lists the matches instead");
        narrow.sidebar = Rect { x: 0.0, y: 0.0, w: 200.0, h: 600.0 };
        narrow.query = "  ".into();
        narrow.preview_first();
        assert!(narrow.shown.is_none());
        narrow.query = "zzz".into();
        narrow.suggestions.clear();
        narrow.preview_first();
        assert!(narrow.shown.is_none());
    }

    /// E2E9 8: the sidebar's rows are clipped below the traffic lights' band.
    #[test]
    fn the_sidebar_rows_are_drawn_below_the_traffic_lights_band() {
        let bar = bifrost_app::layout::UNIFIED_BAR_PT;
        let side = Rect { x: 0.0, y: 0.0, w: 200.0, h: 600.0 };
        let rows = sidebar_rows(side, bar);
        assert_eq!((rows.x, rows.y, rows.w), (0.0, bar, 200.0));
        assert!((rows.y + rows.h - 600.0).abs() < 0.01, "down to the bottom of the column");
        let short = sidebar_rows(Rect { x: 0.0, y: 0.0, w: 200.0, h: bar / 2.0 }, bar);
        assert!(short.h >= 0.0, "a column shorter than the band has no rows area");
    }

    #[test]
    fn the_search_fields_clear_button_empties_the_query() {
        let mut app = DictionaryApp::new();
        app.query = "comp".into();
        assert!(app.perform(Act::ClearSearch));
        assert!(app.query.is_empty() && app.shown.is_none());
        assert!(!app.perform(Act::TextSize(10.0, 20.0)), "the text size pull-down opens from `touch`, with the context");
    }

    #[test]
    fn only_the_main_window_offers_the_text_size_pull_down() {
        assert!(DictionaryApp::new().text_size_menu);
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
