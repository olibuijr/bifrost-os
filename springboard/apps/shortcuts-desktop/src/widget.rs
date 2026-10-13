//! Shortcuts: a grid of coloured tiles that run actions (system actions, text, input, web, files and scripting),
//! a Gallery of starter shortcuts, folders in the sidebar, search, and an editor that shows a shortcut as a list of
//! actions with its details (name, folder, menu key, reminder, automation, colour and glyph).

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CALLOUT_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT};
use crate::automation;
use springboard::shortcuts as model;
use bifrost_app::kit::ui::{
    Ui, Weight, CARD_BG, CARD_RADIUS_PT, CELL_RADIUS_PT, CONTROL_PRESSED, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, LINK,
    MAC_TITLE3_PT, PRESSED, ROW_PRESSED, SEPARATOR_C, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PINK,
    SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, SYSTEM_YELLOW, TOAST_FILL, WHITE,
};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect as PtRect, Role, TextField};
use bifrost_app::glyphs::glyph;
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::widgets;
use bifrost_app::kit::{Event, Surface, TextCache};
use bifrost_app::KeyInput;
use springboard::shortcuts::{Action, Daily, Shortcut, Trigger, GLYPHS, KINDS, MAX_ACTIONS, MAX_SHORTCUTS, MAX_TEXT};
use bifrost_app::proto::LoginStatus as Status;
use bifrost_app::intents::Snippet;
use bifrost_core::json::Json;
use std::path::PathBuf;

/// The result panel's width (points) and the most step rows it shows before "and N more".
const RESULT_W: f32 = 360.0;
const RESULT_MAX_ROWS: usize = 8;
/// The result panel's corner radius (points): grouped cards are 13 pt (owner decision, 2026-10-09).
const RESULT_RADIUS_PT: f32 = 13.0;
/// How long a result panel stays when nobody dismisses it, in milliseconds.
const RESULT_MS: f64 = 8000.0;
/// The most option rows a menu question shows.
const ASK_MAX_OPTIONS: usize = 6;

/// Tile colours a shortcut can take, as the Shortcuts app offers them.
pub const PALETTE: [u32; 8] =
    [SYSTEM_RED, SYSTEM_ORANGE, SYSTEM_YELLOW, SYSTEM_GREEN, SYSTEM_TEAL, SYSTEM_BLUE, SYSTEM_PURPLE, SYSTEM_PINK];

/// The groups of the Add Action list: a header and the `KINDS` indexes under it.
const PICKER_GROUPS: [(&str, &[usize]); 4] = [
    ("Built-in Actions", &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10]),
    ("Text and Variables", &[11, 12, 16, 15]),
    ("Web and Files", &[13, 14]),
    ("Scripting", &[17, 18, 19, 20, 21, 22]),
];

type Rect = (f32, f32, f32, f32);

/// Sidebar width in points (Shortcuts' Library column).
const SIDEBAR_W: f32 = 200.0;
/// Grid tile: the narrowest a tile may get and its height, in points.
const TILE_MIN_W: f32 = 200.0;
const TILE_H: f32 = 100.0;
/// The glyph on the red Remove badge: white is text on the accent here.
const ON_BADGE: u32 = WHITE;
/// Sidebar row ids: All Shortcuts, Gallery, and the folders from `ROW_FOLDER` up.
const ROW_ALL: u32 = 100;
const ROW_GALLERY: u32 = 101;
const ROW_FOLDER: u32 = 200;
/// Toolbar item ids; they double as control ids.
const BAR_BACK: u32 = 1;
const BAR_TITLE: u32 = 2;
const BAR_ADD: u32 = 3;
const BAR_EDIT: u32 = 4;
const BAR_RUN: u32 = 5;
const BAR_SEARCH: u32 = 6;
/// The widest the toolbar title may get, in points, and the search field's width.
const TITLE_MAX_W: f32 = 320.0;
const SEARCH_W: f32 = 170.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Grid,
    Gallery,
    Editor(usize),
    Picker(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Tile(usize),
    Create,
    EditToggle,
    Back,
    Run,
    Remove(usize),
    Dec(usize),
    Inc(usize),
    /// Moves an action row up or down one place.
    Up(usize),
    Down(usize),
    AddAction,
    Pick(usize),
    /// An intent in the Intents section of the action library: runs it on the shortcut in front.
    Intent(usize),
    Color(usize),
    /// A symbol in the glyph picker.
    Glyph(usize),
    Delete,
    Duplicate,
    Export,
    /// The follow-on button of the result panel: asks for the intent the result opens.
    ResultRun,
    /// The Done button of the result panel.
    ResultDismiss,
    /// The "Daily Reminder" row of the open shortcut: steps to the next time.
    Schedule,
    /// The "Menu Key" row: steps to the next free key.
    Hotkey,
    /// The "Automation" row: steps to the next trigger kind; its value row has steppers.
    TrigKind,
    TrigDec,
    TrigInc,
    /// The "Open at Login" row of the grid: asks the shell to turn it on or off.
    LoginItem,
    /// A row of the sidebar (All Shortcuts, Gallery or a folder).
    Side(u32),
    /// The search field in the toolbar.
    Search,
    /// The name and folder rows of the open shortcut, and the text of an action row: start typing.
    EditName,
    EditFolder,
    EditText(usize),
    /// Inserts the next `{Variable}` token into the text being edited.
    InsertVar,
    /// A starter in the Gallery: adds a copy to the list.
    GalleryAdd(usize),
    /// A choice of the menu question, the Continue button and the Cancel button of the input panel.
    AskOption(usize),
    AskOk,
    AskCancel,
}

enum Look {
    Tile { name: String, color: u32, count: usize, glyph: &'static str },
    CreateTile,
    Header(String),
    Row { text: String, value: String, tint: u32, pos: u8, chevron: bool, centre: bool, lead: f32, inset: f32 },
    Minus,
    Stepper(bool),
    /// A symbol button drawn at the cell's centre (the reorder arrows).
    Symbol(&'static str),
    Swatch { color: u32, on: bool },
    GlyphChip { name: &'static str, on: bool },
    /// A footer under a catalog row: the intent's description.
    Note(String),
}

struct Cell {
    target: Option<Target>,
    rect: Rect,
    look: Look,
}

/// What a row of a card group looks like and does; `group` places a list of these.
struct RowSpec {
    text: String,
    value: String,
    tint: u32,
    target: Option<Target>,
    chevron: bool,
    centre: bool,
    /// the text's left offset and the value's right offset, in points
    lead_pt: f32,
    inset_pt: f32,
}

impl RowSpec {
    fn new(text: &str) -> Self {
        RowSpec { text: text.into(), value: String::new(), tint: LABEL.now(), target: None, chevron: false, centre: false, lead_pt: SPACE_L, inset_pt: SPACE_L }
    }

    fn value(mut self, v: impl Into<String>) -> Self {
        self.value = v.into();
        self
    }

    fn tint(mut self, c: u32) -> Self {
        self.tint = c;
        self
    }

    fn target(mut self, t: Target) -> Self {
        self.target = Some(t);
        self
    }

    fn chevron(mut self) -> Self {
        self.chevron = true;
        self
    }

    fn centre(mut self) -> Self {
        self.centre = true;
        self
    }

    fn lead(mut self, pt: f32) -> Self {
        self.lead_pt = pt;
        self
    }

    fn inset(mut self, pt: f32) -> Self {
        self.inset_pt = pt;
        self
    }
}

/// The follow-on an intent result opens: the intent to run next and the parameters it gets.
#[derive(Clone, Debug)]
struct Follow {
    intent: String,
    title: String,
    params: Json,
}

/// The result panel of an intent run that carries a snippet (the plan steps).
#[derive(Clone, Debug)]
struct ResultPanel {
    title: String,
    dialog: Option<String>,
    rows: Vec<(String, String)>,
    follow: Option<Follow>,
    until: f64,
}

/// The text being typed into a row of the editor.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Field {
    Name,
    Folder,
    /// the free text of action row `k`
    Text(usize),
}

#[derive(Clone, Debug)]
struct Editing {
    field: Field,
    shortcut: usize,
    buf: String,
}

/// The questions a run asks before it starts, and the answers so far.
#[derive(Clone, Debug)]
struct Asking {
    shortcut: usize,
    prompts: Vec<model::Prompt>,
    step: usize,
    answers: Vec<(usize, String)>,
    buf: String,
}

/// A file the app asks the shell's panels for: an import (open panel) or an export (save panel) with its content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileJob {
    Import,
    Export { name: String, text: String },
}

pub struct Shortcuts {
    w: i32,
    h: i32,
    path: PathBuf,
    list: Vec<Shortcut>,
    view: View,
    /// the folder the grid shows; `None` for all shortcuts
    folder: Option<String>,
    edit_mode: bool,
    scroll: f32,
    pressed: Option<Target>,
    down_y: f32,
    last_y: f32,
    dragging: bool,
    runs: Vec<Action>,
    toast: Option<(String, f64)>,
    now: f64,
    dirty: bool,
    cache: TextCache,
    /// the Login Items status the shell last reported; `None` until it answers
    login: Option<Status>,
    /// the toggle the last tap asked for: `Some(true)` to open at login, `Some(false)` to stop
    login_request: Option<bool>,
    /// the intents the last taps asked to run, oldest first, with their parameters
    intent_requests: Vec<(String, Json)>,
    /// the result panel of the last intent run that had a snippet, if it is still showing
    result: Option<ResultPanel>,
    /// the search field above the grid and whether it has the keyboard
    search: TextField,
    search_focus: bool,
    /// the row being typed into, if any
    edit: Option<Editing>,
    /// which variable the Insert Variable row offers next
    var_cursor: usize,
    /// the question panel of a run that asks for input
    asking: Option<Asking>,
    /// files the last run asked to save (name and content), and the panels the last taps asked for
    saves: Vec<(String, String)>,
    file_jobs: Vec<FileJob>,
    /// the names of shortcuts whose automation condition held at the last check, so each fires once per time it starts to hold
    fired: Vec<String>,
}

const ROW_POS_FIRST: u8 = 1;
const ROW_POS_LAST: u8 = 2;

impl Shortcuts {
    pub fn new(_ui: &Ui, w: i32, h: i32, path: PathBuf) -> Self {
        let mut list = model::load(&path);
        if list.is_empty() && !path.exists() {
            list = model::defaults();
        }
        Self {
            w,
            h,
            path,
            list,
            view: View::Grid,
            folder: None,
            edit_mode: false,
            scroll: 0.0,
            pressed: None,
            down_y: 0.0,
            last_y: 0.0,
            dragging: false,
            runs: Vec::new(),
            toast: None,
            now: 0.0,
            dirty: true,
            cache: TextCache::default(),
            login: None,
            login_request: None,
            intent_requests: Vec::new(),
            result: None,
            search: TextField::new(),
            search_focus: false,
            edit: None,
            var_cursor: 0,
            asking: None,
            saves: Vec::new(),
            file_jobs: Vec::new(),
            fired: Vec::new(),
        }
    }

    /// The list as the window holds it now. Intents and scripts read it, so they never see a stale copy.
    pub fn list(&self) -> &[Shortcut] {
        &self.list
    }

    /// The shortcuts on screen as (index, in front). The grid shows every tile and none is in front; an open editor or
    /// picker shows its one shortcut in front.
    pub fn on_screen(&self) -> Vec<(usize, bool)> {
        match self.view {
            View::Grid => (0..self.list.len()).map(|i| (i, false)).collect(),
            View::Gallery => Vec::new(),
            View::Editor(i) | View::Picker(i) => {
                if i < self.list.len() {
                    vec![(i, true)]
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The shortcut open in the editor or the action picker, if any.
    pub fn open_shortcut(&self) -> Option<usize> {
        match self.view {
            View::Editor(i) | View::Picker(i) if i < self.list.len() => Some(i),
            _ => None,
        }
    }

    /// The time of the last input or tick, in milliseconds.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// Whether the list has room for another shortcut.
    pub fn has_room(&self) -> bool {
        self.list.len() < MAX_SHORTCUTS
    }

    /// Runs `f` on a copy of the list and keeps the copy, and saves it, only when `f` changed it. Intents and
    /// scripts change the list through here, so the window and the saved file stay the same.
    pub fn edit_list<T>(&mut self, f: impl FnOnce(&mut Vec<Shortcut>) -> T) -> T {
        let mut next = self.list.clone();
        let out = f(&mut next);
        if next != self.list {
            self.list = next;
            self.fix_selection();
            self.dirty = true;
            self.save();
        }
        out
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    /// An external host can report an unavailable run instead of showing success.
    pub fn show_run_unavailable(&mut self, t: f64) {
        self.toast = Some(("This shortcut cannot run on this device.".into(), t + 3000.0));
        self.dirty = true;
    }

    /// The actions the last tap asked the shell to run, oldest first.
    pub fn take_runs(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.runs)
    }

    /// The files the last run asked to save, as (name, content), oldest first.
    pub fn take_saves(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.saves)
    }

    /// The file panels the last taps or menu items asked for, oldest first.
    pub fn take_file_jobs(&mut self) -> Vec<FileJob> {
        std::mem::take(&mut self.file_jobs)
    }

    /// The intents the last taps asked to run from the action library (or a result's follow-on), oldest first, with
    /// their parameters.
    pub fn take_intent_requests(&mut self) -> Vec<(String, Json)> {
        std::mem::take(&mut self.intent_requests)
    }

    /// Shows the outcome of an intent run from the action library: its dialog, or why it did not run.
    pub fn show_result(&mut self, text: &str, t: f64) {
        self.result = None;
        self.toast = Some((text.to_string(), t + 3000.0));
        self.dirty = true;
    }

    /// Adds a line to the result panel when one is showing, otherwise shows it as a message.
    pub fn note(&mut self, label: &str, text: &str, t: f64) {
        match &mut self.result {
            Some(p) => p.rows.push((label.to_string(), text.to_string())),
            None => self.toast = Some((text.to_string(), t + 3000.0)),
        }
        self.dirty = true;
    }

    /// Shows a result that carries a snippet: a panel with the snippet's title, the dialog, its rows, a follow-on
    /// button when the result opens another intent, and Done. It stays until Done is tapped or it times out.
    pub fn show_snippet(&mut self, snippet: &Snippet, dialog: Option<&str>, opens: Option<(String, Json)>, t: f64) {
        let follow = opens.map(|(intent, params)| Follow { title: automation::title_of(&intent), intent, params });
        self.toast = None;
        self.result = Some(ResultPanel {
            title: snippet.title.clone(),
            dialog: dialog.map(str::to_string),
            rows: snippet.rows.iter().map(|r| (r.label.clone(), r.text.clone())).collect(),
            follow,
            until: t + RESULT_MS,
        });
        self.dirty = true;
    }

    /// The Login Items status the shell reported, or `None` when it has not answered.
    pub fn set_login(&mut self, status: Option<Status>) {
        self.login = status;
        self.dirty = true;
    }

    /// The login toggle the last tap asked for (`Some(true)` to open at login), once.
    pub fn take_login_request(&mut self) -> Option<bool> {
        self.login_request.take()
    }

    /// Whether the row reads On: the app opens at login, or the user still has to allow it in Settings.
    fn login_on(&self) -> bool {
        matches!(self.login, Some(Status::Enabled) | Some(Status::RequiresApproval))
    }

    fn login_value(&self) -> &'static str {
        match self.login {
            Some(Status::Enabled) => "On",
            Some(Status::RequiresApproval) => "Needs Approval",
            _ => "Off",
        }
    }

    fn top(ui: &Ui) -> f32 {
        ui.pt(UNIFIED_BAR_PT)
    }

    /// Left edge of the content column, in pixels.
    fn side(ui: &Ui) -> f32 {
        ui.pt(SIDEBAR_W)
    }

    /// The folders in use, as the sidebar lists them.
    fn folder_rows(&self) -> Vec<(String, usize)> {
        model::folders(&self.list)
    }

    fn sidebar(&self) -> SourceList {
        let folders = self.folder_rows();
        let mut items = vec![
            SidebarItem::header("Library"),
            SidebarItem::row(ROW_ALL, "All Shortcuts").with_symbol("square.grid.2x2").with_count(self.list.len() as u32),
            SidebarItem::row(ROW_GALLERY, "Gallery").with_symbol("star"),
        ];
        if !folders.is_empty() {
            items.push(SidebarItem::header("Folders"));
            for (k, (name, n)) in folders.iter().enumerate() {
                items.push(SidebarItem::row(ROW_FOLDER + k as u32, name).with_symbol("folder").with_count(*n as u32));
            }
        }
        let selected = match (&self.view, &self.folder) {
            (View::Gallery, _) => ROW_GALLERY,
            (_, Some(f)) => folders.iter().position(|(n, _)| n == f).map_or(ROW_ALL, |k| ROW_FOLDER + k as u32),
            _ => ROW_ALL,
        };
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(selected));
        list
    }

    fn sidebar_bounds(&self, ui: &Ui) -> PtRect {
        PtRect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.s }
    }

    fn bar_bounds(&self, ui: &Ui) -> PtRect {
        let x = SIDEBAR_W + 12.0;
        let h = widgets::CAPSULE_PT;
        PtRect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.s - x - 16.0).max(0.0), h }
    }

    fn title(&self) -> String {
        match self.view {
            View::Grid => self.folder.clone().unwrap_or_else(|| "All Shortcuts".into()),
            View::Gallery => "Gallery".into(),
            View::Editor(i) => self.list.get(i).map(|s| s.name.clone()).unwrap_or_default(),
            View::Picker(_) => "Add Action".into(),
        }
    }

    /// The unified toolbar (`NativeToolbar`): back, title, then the view's commands.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let search = if self.view == View::Grid { SEARCH_W + widgets::TOOLBAR_GAP_PT } else { 0.0 };
        let room = (self.bar_bounds(ui).w - 5.0 * widgets::CAPSULE_PT - 4.0 * widgets::TOOLBAR_GAP_PT - search).clamp(40.0, TITLE_MAX_W);
        let mut items = vec![];
        if matches!(self.view, View::Editor(_) | View::Picker(_)) {
            items.push(ToolbarItem::icon(BAR_BACK, "Back", "chevron.left").in_group(1));
        }
        items.push(ToolbarItem::custom(BAR_TITLE, "Title", room));
        items.push(ToolbarItem::flexible_space());
        match self.view {
            View::Grid => {
                items.push(ToolbarItem::custom(BAR_SEARCH, "Search", SEARCH_W));
                items.push(ToolbarItem::icon(BAR_ADD, "New Shortcut", "plus").in_group(2).enabled(self.list.len() < MAX_SHORTCUTS));
                items.push(ToolbarItem::button(BAR_EDIT, if self.edit_mode { "Done" } else { "Edit" }).selected(self.edit_mode));
            }
            View::Gallery => {}
            View::Editor(_) => items.push(ToolbarItem::button(BAR_RUN, "Run")),
            View::Picker(_) => {}
        }
        Toolbar::new(items)
    }

    fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        self.bar(ui).layout_with(self.bar_bounds(ui), |_| widgets::CAPSULE_PT, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    fn bar_target(id: u32) -> Option<Target> {
        match id {
            BAR_BACK => Some(Target::Back),
            BAR_ADD => Some(Target::Create),
            BAR_EDIT => Some(Target::EditToggle),
            BAR_RUN => Some(Target::Run),
            BAR_SEARCH => Some(Target::Search),
            _ => None,
        }
    }

    /// The toolbar buttons as controls (points), so the shell keeps them clickable in the drag band.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar(ui);
        self.bar_layout(ui)
            .visible
            .iter()
            .filter(|(id, _)| *id != BAR_TITLE)
            .filter_map(|(id, r)| {
                let it = bar.items.iter().find(|i| i.id == *id)?;
                let (role, value) = if *id == BAR_SEARCH { (Role::TextField, self.search.to_string()) } else { (Role::Button, String::new()) };
                Some(Node { id: *id as u64, role, label: it.label.clone(), value, bounds: *r, enabled: it.enabled })
            })
            .collect()
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = model::save(&self.path, &self.list) {
            eprintln!("springboard: shortcuts: save: {e}");
        }
    }

    /// Drops the folder selection when its last shortcut left it, and the open shortcut when it is gone.
    fn fix_selection(&mut self) {
        if let Some(f) = &self.folder {
            if !self.list.iter().any(|s| &s.folder == f) {
                self.folder = None;
            }
        }
        match self.view {
            View::Editor(i) | View::Picker(i) if i >= self.list.len() => self.view = View::Grid,
            _ => {}
        }
    }

    /// The label of a menu key in a row: `Cmd+1`, or None.
    fn hotkey_label(key: &str) -> String {
        if key.is_empty() {
            "None".into()
        } else {
            key.replacen("cmd+", "Cmd+", 1)
        }
    }

    /// Places a group of rows as one card and returns their rectangles.
    fn group(&self, ui: &Ui, cells: &mut Vec<Cell>, y: &mut f32, specs: Vec<RowSpec>) -> Vec<Rect> {
        let (m, x0) = (ui.pt(SPACE_XL), Self::side(ui));
        let n = specs.len();
        let mut out = Vec::with_capacity(n);
        for (k, sp) in specs.into_iter().enumerate() {
            let pos = (if k == 0 { ROW_POS_FIRST } else { 0 }) | if k + 1 == n { ROW_POS_LAST } else { 0 };
            let rect = (x0 + m, *y, self.w as f32 - x0 - 2.0 * m, ui.pt(44.0));
            cells.push(Cell {
                target: sp.target,
                rect,
                look: Look::Row {
                    text: sp.text,
                    value: sp.value,
                    tint: sp.tint,
                    pos,
                    chevron: sp.chevron,
                    centre: sp.centre,
                    lead: ui.pt(sp.lead_pt),
                    inset: ui.pt(sp.inset_pt),
                },
            });
            *y += rect.3;
            out.push(rect);
        }
        out
    }

    /// A small grey section header above a group.
    fn head(&self, ui: &Ui, cells: &mut Vec<Cell>, y: &mut f32, text: &str) {
        let (m, x0) = (ui.pt(SPACE_XL), Self::side(ui));
        let w = self.w as f32 - x0;
        cells.push(Cell { target: None, rect: (x0 + m + ui.pt(SPACE_L), *y, w, ui.pt(SPACE_XXXL)), look: Look::Header(text.into()) });
        *y += ui.pt(SPACE_XXXL);
    }

    /// A footnote under a group.
    fn foot(&self, ui: &Ui, cells: &mut Vec<Cell>, y: &mut f32, text: &str) {
        let (m, x0) = (ui.pt(SPACE_XL), Self::side(ui));
        let w = self.w as f32 - x0;
        cells.push(Cell { target: None, rect: (x0 + m + ui.pt(SPACE_L), *y, w - 2.0 * (m + ui.pt(SPACE_L)), ui.pt(SPACE_XXXL)), look: Look::Note(text.into()) });
        *y += ui.pt(SPACE_XXXL);
    }

    /// The text of a row while it is being typed into: the typed text and a caret.
    fn typing(&self, field: &Field, shortcut: usize) -> Option<String> {
        self.edit.as_ref().filter(|e| &e.field == field && e.shortcut == shortcut).map(|e| format!("{}|", e.buf))
    }

    /// Every drawn and tappable thing in screen coordinates, plus the content bottom.
    fn layout(&self, ui: &Ui) -> (Vec<Cell>, f32) {
        let (m, top) = (ui.pt(SPACE_XL), Self::top(ui));
        let x0 = Self::side(ui);
        let w = self.w as f32 - x0;
        let m0 = m;
        let m = x0 + m0;
        let mut cells = Vec::new();
        let mut y = top - self.scroll;
        match self.view {
            View::Grid => {
                y += ui.pt(SPACE_XL);
                self.group(ui, &mut cells, &mut y, vec![RowSpec::new("Open at Login").value(self.login_value()).target(Target::LoginItem).lead(46.0)]);
                y += ui.pt(SPACE_L);
                let gap = ui.pt(SPACE_L);
                let cols = (((w - 2.0 * m0 + gap) / (ui.pt(TILE_MIN_W) + gap)).floor() as usize).max(2);
                let tw = (w - 2.0 * m0 - gap * (cols - 1) as f32) / cols as f32;
                let th = ui.pt(TILE_H);
                let shown = model::visible(&self.list, &self.search, self.folder.as_deref());
                let searching = !self.search.is_empty();
                if shown.is_empty() && searching {
                    cells.push(Cell { target: None, rect: (m, y, w - 2.0 * m0, ui.pt(SPACE_XXXL)), look: Look::Note("No shortcuts match the search.".into()) });
                    y += ui.pt(SPACE_XXXL);
                }
                let total = shown.len() + usize::from(!searching);
                for k in 0..total {
                    let (cx, cy) = ((k % cols) as f32, (k / cols) as f32);
                    let rect = (m + cx * (tw + gap), y + cy * (th + gap), tw, th);
                    if let Some(&i) = shown.get(k) {
                        let s = &self.list[i];
                        let look = Look::Tile {
                            name: s.name.clone(),
                            color: PALETTE[s.color % PALETTE.len()],
                            count: s.actions.len(),
                            glyph: GLYPHS[s.glyph % GLYPHS.len()],
                        };
                        cells.push(Cell { target: Some(Target::Tile(i)), rect, look });
                    } else {
                        cells.push(Cell { target: Some(Target::Create), rect, look: Look::CreateTile });
                    }
                }
                y += (total.div_ceil(cols)) as f32 * (th + gap);
            }
            View::Gallery => {
                y += ui.pt(SPACE_L);
                self.head(ui, &mut cells, &mut y, "Starter Shortcuts");
                for (k, s) in model::gallery().iter().enumerate() {
                    let n = s.actions.len();
                    let value = format!("{n} {}", if n == 1 { "action" } else { "actions" });
                    self.group(ui, &mut cells, &mut y, vec![RowSpec::new(&s.name).value(value).target(Target::GalleryAdd(k))]);
                    let steps: Vec<&str> = s.actions.iter().filter(|a| !matches!(a, Action::End | Action::Otherwise | Action::MenuItem(_))).take(4).map(Action::title).collect();
                    self.foot(ui, &mut cells, &mut y, &steps.join(", "));
                }
            }
            View::Editor(i) => {
                let Some(sc) = self.list.get(i) else { return (cells, y) };
                y += ui.pt(SPACE_L);
                self.head(ui, &mut cells, &mut y, "Actions");
                if let Some(Editing { field: Field::Text(k), .. }) = &self.edit {
                    let names = model::variables_before(&sc.actions, *k);
                    let next = names[self.var_cursor % names.len()].clone();
                    self.group(ui, &mut cells, &mut y, vec![RowSpec::new("Insert Variable").value(format!("{{{next}}}")).tint(SYSTEM_BLUE).target(Target::InsertVar)]);
                    y += ui.pt(SPACE_L);
                }
                let n = sc.actions.len();
                let mut specs = Vec::with_capacity(n + 1);
                for (k, a) in sc.actions.iter().enumerate() {
                    let depth = model::depth_of(&sc.actions, k);
                    let value = match (a.text_value(), self.typing(&Field::Text(k), i)) {
                        (Some(_), Some(typed)) => typed,
                        (_, _) => a.value(),
                    };
                    let inset = if a.adjustable() { 2.0 * 32.0 + 2.0 * 44.0 + SPACE_S } else { 2.0 * 32.0 + SPACE_S };
                    let mut spec = RowSpec::new(a.title()).value(value).lead(46.0 + 16.0 * depth as f32).inset(inset);
                    if a.text_value().is_some() {
                        spec = spec.target(Target::EditText(k));
                    }
                    specs.push(spec);
                }
                if n < MAX_ACTIONS {
                    specs.push(RowSpec::new("Add Action").tint(SYSTEM_BLUE).target(Target::AddAction));
                }
                let rects = self.group(ui, &mut cells, &mut y, specs);
                for (k, a) in sc.actions.iter().enumerate() {
                    let r = rects[k];
                    cells.push(Cell { target: Some(Target::Remove(k)), rect: (r.0, r.1, ui.pt(44.0), r.3), look: Look::Minus });
                    let bw2 = ui.pt(32.0);
                    cells.push(Cell { target: Some(Target::Down(k)), rect: (r.0 + r.2 - bw2, r.1, bw2, r.3), look: Look::Symbol("chevron.down") });
                    cells.push(Cell { target: Some(Target::Up(k)), rect: (r.0 + r.2 - 2.0 * bw2, r.1, bw2, r.3), look: Look::Symbol("chevron.up") });
                    if a.adjustable() {
                        let bw = ui.pt(44.0);
                        cells.push(Cell { target: Some(Target::Inc(k)), rect: (r.0 + r.2 - 2.0 * bw2 - bw, r.1, bw, r.3), look: Look::Stepper(true) });
                        cells.push(Cell { target: Some(Target::Dec(k)), rect: (r.0 + r.2 - 2.0 * bw2 - 2.0 * bw, r.1, bw, r.3), look: Look::Stepper(false) });
                    }
                }
                y += ui.pt(SPACE_L);
                self.head(ui, &mut cells, &mut y, "Details");
                let name = self.typing(&Field::Name, i).unwrap_or_else(|| sc.name.clone());
                let folder = self.typing(&Field::Folder, i).unwrap_or_else(|| if sc.folder.is_empty() { "None".into() } else { sc.folder.clone() });
                let when = sc.schedule.map(|d| d.label()).unwrap_or_else(|| "Off".into());
                let mut details = vec![
                    RowSpec::new("Name").value(name).target(Target::EditName),
                    RowSpec::new("Folder").value(folder).target(Target::EditFolder),
                    RowSpec::new("Menu Key").value(Self::hotkey_label(&sc.hotkey)).target(Target::Hotkey),
                    RowSpec::new("Daily Reminder").value(when).target(Target::Schedule),
                    RowSpec::new("Automation").value(sc.trigger.label()).target(Target::TrigKind),
                ];
                if sc.trigger != Trigger::Off {
                    details.push(RowSpec::new("Trigger").value(sc.trigger.value()).inset(2.0 * 44.0 + SPACE_S));
                }
                let rects = self.group(ui, &mut cells, &mut y, details);
                if sc.trigger != Trigger::Off {
                    let r = rects[rects.len() - 1];
                    let bw = ui.pt(44.0);
                    cells.push(Cell { target: Some(Target::TrigInc), rect: (r.0 + r.2 - bw, r.1, bw, r.3), look: Look::Stepper(true) });
                    cells.push(Cell { target: Some(Target::TrigDec), rect: (r.0 + r.2 - 2.0 * bw, r.1, bw, r.3), look: Look::Stepper(false) });
                }
                self.foot(ui, &mut cells, &mut y, "The menu key runs the shortcut from the Shortcuts menu.");
                y += ui.pt(SPACE_S);
                self.head(ui, &mut cells, &mut y, "Color");
                let sw = ui.pt(SPACE_XXXL);
                let step = (w - 2.0 * m0 - ui.pt(SPACE_XXL)) / PALETTE.len() as f32;
                cells.push(Cell { target: None, rect: (m, y, w - 2.0 * m0, ui.pt(56.0)), look: Look::Row { text: String::new(), value: String::new(), tint: LABEL.now(), pos: ROW_POS_FIRST | ROW_POS_LAST, chevron: false, centre: false, lead: 0.0, inset: 0.0 } });
                for (c, color) in PALETTE.iter().enumerate() {
                    let cx = m + ui.pt(SPACE_M) + step * c as f32 + (step - sw) / 2.0;
                    cells.push(Cell { target: Some(Target::Color(c)), rect: (cx, y + ui.pt(SPACE_M), sw, sw), look: Look::Swatch { color: *color, on: sc.color % PALETTE.len() == c } });
                }
                y += ui.pt(56.0) + ui.pt(SPACE_S);
                self.head(ui, &mut cells, &mut y, "Glyph");
                cells.push(Cell { target: None, rect: (m, y, w - 2.0 * m0, ui.pt(56.0)), look: Look::Row { text: String::new(), value: String::new(), tint: LABEL.now(), pos: ROW_POS_FIRST | ROW_POS_LAST, chevron: false, centre: false, lead: 0.0, inset: 0.0 } });
                let gstep = (w - 2.0 * m0 - ui.pt(SPACE_XXL)) / GLYPHS.len() as f32;
                for (g, name) in GLYPHS.iter().enumerate() {
                    let cx = m + ui.pt(SPACE_M) + gstep * g as f32 + (gstep - sw) / 2.0;
                    cells.push(Cell { target: Some(Target::Glyph(g)), rect: (cx, y + ui.pt(SPACE_M), sw, sw), look: Look::GlyphChip { name, on: sc.glyph % GLYPHS.len() == g } });
                }
                y += ui.pt(56.0) + ui.pt(35.0);
                self.group(ui, &mut cells, &mut y, vec![
                    RowSpec::new("Duplicate Shortcut").tint(SYSTEM_BLUE).centre().target(Target::Duplicate),
                    RowSpec::new("Export Shortcut...").tint(SYSTEM_BLUE).centre().target(Target::Export),
                ]);
                y += ui.pt(SPACE_XL);
                self.group(ui, &mut cells, &mut y, vec![RowSpec::new("Delete Shortcut").tint(SYSTEM_RED).centre().target(Target::Delete)]);
            }
            View::Picker(_) => {
                y += ui.pt(SPACE_L);
                for (title, kinds) in PICKER_GROUPS.iter() {
                    self.head(ui, &mut cells, &mut y, title);
                    let specs = kinds.iter().map(|k| RowSpec::new(KINDS[*k]).chevron().target(Target::Pick(*k))).collect();
                    self.group(ui, &mut cells, &mut y, specs);
                    y += ui.pt(SPACE_L);
                }
                // the intents this app declares, with the title and description from the catalog
                self.head(ui, &mut cells, &mut y, "Intents");
                for (k, intent) in automation::declared().iter().enumerate() {
                    self.group(ui, &mut cells, &mut y, vec![RowSpec::new(&intent.title).target(Target::Intent(k))]);
                    self.foot(ui, &mut cells, &mut y, &intent.description);
                }
            }
        }
        (cells, y + self.scroll)
    }

    /// The result panel's box in pixels: a title, the dialog (when there is one), the step rows, an "and N more" line
    /// when the rows are capped, and the button row.
    fn result_box(&self, ui: &Ui, p: &ResultPanel) -> Rect {
        let x0 = Self::side(ui);
        let pw = ui.pt(RESULT_W).min(self.w as f32 - x0 - 2.0 * ui.pt(SPACE_L));
        let shown = p.rows.len().min(RESULT_MAX_ROWS);
        let more = usize::from(p.rows.len() > shown);
        let lines = 2 + usize::from(p.dialog.is_some()) + shown + more;
        let ph = ui.pt(SPACE_L) * 2.0 + ui.pt(SPACE_XXXL) * lines as f32;
        let x = x0 + (self.w as f32 - x0 - pw) / 2.0;
        let y = self.h as f32 - ph - ui.pt(SPACE_XXL);
        (x, y, pw, ph)
    }

    /// The follow-on button's rect (only when the result opens an intent) and the Done button's rect, in pixels.
    fn result_buttons(&self, ui: &Ui, p: &ResultPanel) -> (Option<Rect>, Rect) {
        let (x, y, w, h) = self.result_box(ui, p);
        let (pad, row_h) = (ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
        let by = y + h - pad - row_h;
        let follow = p.follow.as_ref().map(|_| (x + pad, by, w / 2.0 - pad, row_h));
        let done = (x + w / 2.0, by, w / 2.0 - pad, row_h);
        (follow, done)
    }

    /// The question panel's box in pixels: the prompt, then the field or the menu's choices, then the buttons.
    fn ask_box(&self, ui: &Ui, a: &Asking) -> Rect {
        let x0 = Self::side(ui);
        let pw = ui.pt(RESULT_W).min(self.w as f32 - x0 - 2.0 * ui.pt(SPACE_L));
        let body = a.prompts.get(a.step).map_or(1, |p| if p.options.is_empty() { 1 } else { p.options.len().min(ASK_MAX_OPTIONS) });
        let ph = ui.pt(SPACE_L) * 2.0 + ui.pt(SPACE_XXXL) * (2 + body) as f32;
        let x = x0 + (self.w as f32 - x0 - pw) / 2.0;
        let y = self.h as f32 - ph - ui.pt(SPACE_XXL);
        (x, y, pw, ph)
    }

    /// The Cancel and Continue buttons of the question panel in pixels (Continue only for a text question), and the
    /// rect of choice `k`.
    fn ask_buttons(&self, ui: &Ui, a: &Asking) -> (Rect, Option<Rect>) {
        let (x, y, w, h) = self.ask_box(ui, a);
        let (pad, row_h) = (ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
        let by = y + h - pad - row_h;
        let text = a.prompts.get(a.step).is_some_and(|p| p.options.is_empty());
        ((x + pad, by, w / 2.0 - pad, row_h), text.then_some((x + w / 2.0, by, w / 2.0 - pad, row_h)))
    }

    fn ask_option(&self, ui: &Ui, a: &Asking, k: usize) -> Rect {
        let (x, y, w, _) = self.ask_box(ui, a);
        let (pad, row_h) = (ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
        (x + pad, y + pad + row_h * (1 + k) as f32, w - 2.0 * pad, row_h)
    }

    fn max_scroll(&self, ui: &Ui) -> f32 {
        let (_, bottom) = self.layout(ui);
        (bottom + ui.pt(SPACE_XXL) - self.h as f32).max(0.0)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let inside = |r: Rect| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
        // the question panel is modal: only its buttons and choices take a tap
        if let Some(a) = &self.asking {
            let (cancel, ok) = self.ask_buttons(ui, a);
            if inside(cancel) {
                return Some(Target::AskCancel);
            }
            if ok.is_some_and(inside) {
                return Some(Target::AskOk);
            }
            if let Some(p) = a.prompts.get(a.step) {
                for k in 0..p.options.len().min(ASK_MAX_OPTIONS) {
                    if inside(self.ask_option(ui, a, k)) {
                        return Some(Target::AskOption(k));
                    }
                }
            }
            return None;
        }
        if y < Self::top(ui) {
            return self.bar_layout(ui).hit(x / ui.s, y / ui.s).filter(|id| {
                self.bar(ui).items.iter().any(|i| i.id == *id && i.enabled)
            }).and_then(Self::bar_target);
        }
        if x < Self::side(ui) {
            return self.sidebar().hit(self.sidebar_bounds(ui), -(UNIFIED_BAR_PT + 4.0), x / ui.s, y / ui.s).map(Target::Side);
        }
        // the result panel floats over the content: its buttons win, and the rest of it takes no tap
        if let Some(p) = &self.result {
            let (bx, by, bw, bh) = self.result_box(ui, p);
            if x >= bx && x < bx + bw && y >= by && y < by + bh {
                let (follow, done) = self.result_buttons(ui, p);
                if follow.is_some_and(inside) {
                    return Some(Target::ResultRun);
                }
                return inside(done).then_some(Target::ResultDismiss);
            }
        }
        let (cells, _) = self.layout(ui);
        cells
            .iter()
            .rev()
            .filter(|c| c.target.is_some())
            .find(|c| inside(c.rect))
            .and_then(|c| c.target)
    }

    /// Starts shortcut `i`: asks the questions its actions have, then runs it.
    fn start_run(&mut self, i: usize) {
        let Some(s) = self.list.get(i) else { return };
        let prompts = model::prompts(&s.actions);
        if prompts.is_empty() {
            self.finish_run(i, &[]);
        } else {
            self.search_focus = false;
            self.asking = Some(Asking { shortcut: i, prompts, step: 0, answers: Vec::new(), buf: String::new() });
        }
    }

    /// Runs shortcut `i` from the Shortcuts menu or an automation.
    pub fn run_index(&mut self, i: usize) {
        self.start_run(i);
        self.dirty = true;
    }

    /// Takes the answer to the question on screen and moves on; the last answer runs the shortcut.
    fn answer(&mut self, text: String) {
        let Some(mut a) = self.asking.take() else { return };
        if let Some(p) = a.prompts.get(a.step) {
            a.answers.push((p.at, text));
        }
        a.step += 1;
        a.buf.clear();
        if a.step >= a.prompts.len() {
            self.finish_run(a.shortcut, &a.answers);
        } else {
            self.asking = Some(a);
        }
    }

    /// Runs shortcut `i` with the answers it asked for. The text, variable and scripting steps run here; the steps that
    /// need the shell are queued in `runs`, files to save in `saves`. A Show Result step opens the result panel.
    fn finish_run(&mut self, i: usize, answers: &[(usize, String)]) {
        let Some(s) = self.list.get(i) else { return };
        let run = model::evaluate(&s.actions, answers);
        let name = s.name.clone();
        let empty = s.actions.is_empty();
        self.runs.extend(run.effects.iter().cloned());
        self.saves.extend(run.saved.iter().cloned());
        let mut rows: Vec<(String, String)> = run.results.iter().enumerate().map(|(k, r)| ((k + 1).to_string(), r.clone())).collect();
        for u in &run.fetched {
            rows.push((String::new(), format!("Get {u}: web requests are not available on this device yet")));
        }
        if !rows.is_empty() {
            self.toast = None;
            self.result = Some(ResultPanel { title: name, dialog: None, rows, follow: None, until: self.now + RESULT_MS });
        } else {
            let msg = if empty {
                format!("{name} has no actions")
            } else if run.truncated {
                format!("{name} stopped after too many steps")
            } else {
                format!("Ran {name}")
            };
            self.toast = Some((msg, self.now + 2000.0));
        }
    }

    fn editing(&mut self) -> Option<&mut Shortcut> {
        match self.view {
            View::Editor(i) | View::Picker(i) => self.list.get_mut(i),
            View::Grid | View::Gallery => None,
        }
    }

    /// Starts typing into `field` of the open shortcut, beginning with its current text.
    fn begin_edit(&mut self, field: Field) {
        let Some(i) = self.open_shortcut() else { return };
        let s = &self.list[i];
        let buf = match &field {
            Field::Name => s.name.clone(),
            Field::Folder => s.folder.clone(),
            Field::Text(k) => s.actions.get(*k).and_then(|a| a.text_value()).unwrap_or("").to_string(),
        };
        self.search_focus = false;
        self.var_cursor = 0;
        self.edit = Some(Editing { field, shortcut: i, buf });
    }

    /// Applies the typed text to its row and stops typing.
    fn commit_edit(&mut self) {
        let Some(e) = self.edit.take() else { return };
        let Some(s) = self.list.get_mut(e.shortcut) else { return };
        match e.field {
            Field::Name => {
                s.set_name(&e.buf);
            }
            Field::Folder => s.set_folder(&e.buf),
            Field::Text(k) => {
                if let Some(a) = s.actions.get_mut(k) {
                    a.set_text(&e.buf);
                }
            }
        }
        self.fix_selection();
        self.save();
        self.dirty = true;
    }

    /// Whether a text field has the keyboard: the search field, a row being typed into, or a question.
    pub fn wants_keyboard(&self) -> bool {
        self.search_focus || self.edit.is_some() || self.asking.as_ref().is_some_and(|a| a.prompts.get(a.step).is_some_and(|p| p.options.is_empty()))
    }

    /// The text field the keyboard goes to, as an editable string.
    fn typed_text(&mut self) -> Option<&mut String> {
        if let Some(a) = self.asking.as_mut() {
            return a.prompts.get(a.step).is_some_and(|p| p.options.is_empty()).then_some(&mut a.buf);
        }
        self.edit.as_mut().map(|e| &mut e.buf)
    }

    /// Typed text from the keyboard or a paste: into the question, the row being typed into, or the search field.
    pub fn type_text(&mut self, text: &str) -> bool {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        if clean.is_empty() {
            return false;
        }
        if let Some(buf) = self.typed_text() {
            buf.push_str(&clean);
            while buf.chars().count() > MAX_TEXT {
                buf.pop();
            }
        } else if self.search_focus && self.asking.is_none() {
            self.search.push_str(&clean);
            self.scroll = 0.0;
        } else {
            return false;
        }
        self.dirty = true;
        true
    }

    /// Pastes the first line of the clipboard where `type_text` would type.
    pub fn paste(&mut self, text: &str) -> bool {
        self.type_text(text.lines().next().unwrap_or(""))
    }

    /// A key code (Backspace 8, Return 13, Escape 27) for the field that has the keyboard.
    fn key_code(&mut self, code: u32) -> bool {
        match code {
            8 => {
                if let Some(buf) = self.typed_text() {
                    buf.pop();
                } else if self.search_focus && self.asking.is_none() {
                    self.search.pop();
                } else {
                    return false;
                }
            }
            13 => {
                if self.asking.is_some() {
                    let text = self.asking.as_ref().map(|a| a.buf.clone()).unwrap_or_default();
                    if self.asking.as_ref().is_some_and(|a| a.prompts.get(a.step).is_some_and(|p| p.options.is_empty())) {
                        self.answer(text);
                    } else {
                        return false;
                    }
                } else if self.edit.is_some() {
                    self.commit_edit();
                } else if self.search_focus {
                    self.search_focus = false;
                } else {
                    return false;
                }
            }
            27 => {
                if self.asking.is_some() {
                    self.asking = None;
                } else if self.edit.is_some() {
                    self.edit = None;
                } else if self.search_focus {
                    self.search_focus = false;
                    self.search.clear();
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    /// A key from the shell: text goes to the field that has the keyboard, and Backspace, Return and Escape edit it.
    pub fn key(&mut self, key: &KeyInput) -> bool {
        match key {
            KeyInput::Text(text) => self.type_text(text),
            KeyInput::Code(code) => self.key_code(*code),
        }
    }

    /// Puts the keyboard in the search field of the grid (the Find menu item).
    pub fn focus_search(&mut self) {
        self.commit_edit();
        self.view = View::Grid;
        self.search_focus = true;
        self.dirty = true;
    }

    /// Shows the Gallery of starter shortcuts.
    pub fn show_gallery(&mut self) {
        self.commit_edit();
        self.view = View::Gallery;
        self.scroll = 0.0;
        self.dirty = true;
    }

    /// Shows the grid of every shortcut.
    pub fn show_all(&mut self) {
        self.commit_edit();
        self.view = View::Grid;
        self.folder = None;
        self.scroll = 0.0;
        self.dirty = true;
    }

    /// Adds an empty shortcut (in the folder on screen) and opens it in the editor.
    pub fn new_shortcut(&mut self) {
        if self.list.len() >= MAX_SHORTCUTS {
            return;
        }
        self.commit_edit();
        let name = (1..).map(|n| format!("Shortcut {n}")).find(|n| self.list.iter().all(|s| &s.name != n)).unwrap_or_default();
        let mut s = Shortcut::new(&name, self.list.len() % PALETTE.len());
        if let Some(f) = &self.folder {
            s.folder = f.clone();
        }
        self.list.push(s);
        self.view = View::Editor(self.list.len() - 1);
        self.scroll = 0.0;
        self.save();
        self.dirty = true;
    }

    /// Copies the open shortcut and opens the copy.
    pub fn duplicate_open(&mut self) {
        self.commit_edit();
        let Some(i) = self.open_shortcut() else { return };
        if let Some(copy) = model::duplicate(&mut self.list, i) {
            self.view = View::Editor(copy);
            self.scroll = 0.0;
            self.save();
            self.dirty = true;
        }
    }

    /// Deletes the open shortcut and goes back to the grid.
    pub fn delete_open(&mut self) {
        self.commit_edit();
        let Some(i) = self.open_shortcut() else { return };
        self.list.remove(i);
        self.view = View::Grid;
        self.scroll = 0.0;
        self.fix_selection();
        self.save();
        self.dirty = true;
    }

    /// Runs the open shortcut.
    pub fn run_open(&mut self) {
        self.commit_edit();
        if let Some(i) = self.open_shortcut() {
            self.start_run(i);
            self.dirty = true;
        }
    }

    /// Asks for the open panel to import a shortcut file.
    pub fn request_import(&mut self) {
        self.file_jobs.push(FileJob::Import);
    }

    /// Asks for the save panel to export the open shortcut as a file.
    pub fn request_export(&mut self) {
        self.commit_edit();
        if let Some(i) = self.open_shortcut() {
            let s = &self.list[i];
            self.file_jobs.push(FileJob::Export { name: format!("{}.{}", s.name, model::EXPORT_EXT), text: model::export_text(s) });
        }
    }

    /// Adds the shortcuts in an imported file (with names made unique) and says how many; a bad file says why.
    pub fn import(&mut self, text: &str) -> usize {
        let t = self.now;
        match model::import_text(text) {
            Ok(found) => {
                let mut added = 0;
                for mut s in found {
                    if self.list.len() >= MAX_SHORTCUTS {
                        break;
                    }
                    s.name = model::unique_name(&self.list, &s.name);
                    self.list.push(s);
                    added += 1;
                }
                self.save();
                self.toast = Some((format!("Imported {added} {}", if added == 1 { "shortcut" } else { "shortcuts" }), t + 3000.0));
                self.dirty = true;
                added
            }
            Err(e) => {
                self.toast = Some((format!("Could not import: {e}"), t + 3000.0));
                self.dirty = true;
                0
            }
        }
    }

    /// Says the export finished.
    pub fn exported(&mut self) {
        self.toast = Some(("Exported the shortcut.".into(), self.now + 3000.0));
        self.dirty = true;
    }

    /// Says a panel or a file operation failed.
    pub fn failed(&mut self, text: &str) {
        self.toast = Some((text.to_string(), self.now + 3000.0));
        self.dirty = true;
    }

    /// Whether any shortcut has a battery automation, so the app knows to read the charge.
    pub fn has_battery_trigger(&self) -> bool {
        self.list.iter().any(|s| matches!(s.trigger, Trigger::BatteryBelow(_)))
    }

    /// Checks the automations against the battery charge `battery` and starts each shortcut whose condition has just
    /// begun to hold (it starts again only after the condition stopped holding). The names started, in list order.
    pub fn check_triggers(&mut self, battery: Option<u8>) -> Vec<String> {
        let mut started = Vec::new();
        for i in 0..self.list.len() {
            let (name, holds) = (self.list[i].name.clone(), self.list[i].trigger.holds(battery, None));
            let was = self.fired.contains(&name);
            if holds && !was {
                self.fired.push(name.clone());
                self.start_run(i);
                started.push(name);
            } else if !holds && was {
                self.fired.retain(|n| n != &name);
            }
        }
        if !started.is_empty() {
            self.dirty = true;
        }
        started
    }

    fn open_editor(&mut self, i: usize) {
        self.view = View::Editor(i);
        self.scroll = 0.0;
    }

    fn activate(&mut self, t: Target) {
        // tapping anything but the text being typed into finishes it, and the search field gives up the keyboard
        if !matches!(t, Target::EditName | Target::EditFolder | Target::EditText(_) | Target::InsertVar) {
            self.commit_edit();
        }
        if t != Target::Search {
            self.search_focus = false;
        }
        match t {
            Target::Tile(i) if self.edit_mode => self.open_editor(i),
            Target::Tile(i) => self.start_run(i),
            Target::Create => self.new_shortcut(),
            Target::Side(id) => {
                self.scroll = 0.0;
                match id {
                    ROW_GALLERY => self.view = View::Gallery,
                    ROW_ALL => {
                        self.view = View::Grid;
                        self.folder = None;
                    }
                    id if id >= ROW_FOLDER => {
                        let folders = self.folder_rows();
                        if let Some((name, _)) = folders.get((id - ROW_FOLDER) as usize) {
                            self.folder = Some(name.clone());
                            self.view = View::Grid;
                        }
                    }
                    _ => {}
                }
            }
            Target::Search => {
                self.search_focus = true;
                self.edit = None;
            }
            Target::EditToggle => self.edit_mode = !self.edit_mode,
            Target::Back => {
                self.view = match self.view {
                    View::Picker(i) => View::Editor(i),
                    _ => View::Grid,
                };
                self.scroll = 0.0;
            }
            Target::Run => self.run_open(),
            Target::Remove(k) => {
                if let Some(s) = self.editing().filter(|s| k < s.actions.len()) {
                    s.remove_action(k);
                    self.save();
                }
            }
            Target::Dec(k) | Target::Inc(k) => {
                if let Some(a) = self.editing().and_then(|s| s.actions.get_mut(k)) {
                    a.step(matches!(t, Target::Inc(_)));
                    self.save();
                }
            }
            Target::Up(k) | Target::Down(k) => {
                if let Some(s) = self.editing() {
                    if s.move_action(k, matches!(t, Target::Up(_))).is_some() {
                        self.save();
                    }
                }
            }
            Target::AddAction => {
                if let View::Editor(i) = self.view {
                    self.view = View::Picker(i);
                    self.scroll = 0.0;
                }
            }
            Target::Pick(k) => {
                let added = self.editing().map(|s| s.add_kind(k));
                if let View::Picker(i) = self.view {
                    self.view = View::Editor(i);
                }
                if added == Some(false) {
                    self.toast = Some(("There is no room for that action.".into(), self.now + 3000.0));
                }
                self.save();
            }
            Target::Color(c) => {
                if let Some(s) = self.editing() {
                    s.color = c;
                    self.save();
                }
            }
            Target::Glyph(g) => {
                if let Some(s) = self.editing() {
                    s.glyph = g % GLYPHS.len();
                    self.save();
                }
            }
            Target::Intent(k) => {
                if let Some(intent) = automation::declared().get(k) {
                    self.intent_requests.push((intent.id.clone(), Json::Obj(Vec::new())));
                }
            }
            Target::ResultRun => {
                // the follow-on runs the intent the result opened, with the parameters it named
                if let Some(follow) = self.result.take().and_then(|p| p.follow) {
                    self.intent_requests.push((follow.intent, follow.params));
                }
            }
            Target::ResultDismiss => self.result = None,
            Target::Schedule => {
                if let Some(s) = self.editing() {
                    s.schedule = Daily::after(s.schedule);
                    self.save();
                }
            }
            Target::Hotkey => {
                if let Some(i) = self.open_shortcut() {
                    let next = model::next_hotkey(&self.list, i);
                    self.list[i].hotkey = next;
                    self.save();
                }
            }
            Target::TrigKind => {
                if let Some(s) = self.editing() {
                    s.trigger = s.trigger.next_kind();
                    self.save();
                }
            }
            Target::TrigDec | Target::TrigInc => {
                if let Some(s) = self.editing() {
                    s.trigger.step(matches!(t, Target::TrigInc));
                    self.save();
                }
            }
            Target::EditName => self.begin_edit(Field::Name),
            Target::EditFolder => self.begin_edit(Field::Folder),
            Target::EditText(k) => self.begin_edit(Field::Text(k)),
            Target::InsertVar => {
                if let Some(Editing { field: Field::Text(k), shortcut, .. }) = self.edit.clone() {
                    if let Some(s) = self.list.get(shortcut) {
                        let names = model::variables_before(&s.actions, k);
                        let token = format!("{{{}}}", names[self.var_cursor % names.len()]);
                        self.var_cursor += 1;
                        self.type_text(&token);
                    }
                }
            }
            Target::GalleryAdd(k) => {
                let starters = model::gallery();
                if let Some(mut s) = starters.into_iter().nth(k) {
                    if self.list.len() < MAX_SHORTCUTS {
                        s.name = model::unique_name(&self.list, &s.name);
                        let name = s.name.clone();
                        self.list.push(s);
                        self.save();
                        self.toast = Some((format!("Added {name}"), self.now + 2500.0));
                    } else {
                        self.toast = Some(("The list is full.".into(), self.now + 2500.0));
                    }
                }
            }
            Target::AskOption(k) => {
                let choice = self.asking.as_ref().and_then(|a| a.prompts.get(a.step)).and_then(|p| p.options.get(k)).cloned();
                if let Some(c) = choice {
                    self.answer(c);
                }
            }
            Target::AskOk => {
                let text = self.asking.as_ref().map(|a| a.buf.clone()).unwrap_or_default();
                self.answer(text);
            }
            Target::AskCancel => self.asking = None,
            Target::LoginItem => self.login_request = Some(!self.login_on()),
            Target::Duplicate => self.duplicate_open(),
            Target::Export => self.request_export(),
            Target::Delete => self.delete_open(),
        }
        self.fix_selection();
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) -> bool {
        self.now = t;
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
                if self.pressed.is_none() && self.asking.is_none() {
                    self.commit_edit();
                    self.search_focus = false;
                }
                self.down_y = y;
                self.last_y = y;
                self.dragging = false;
                self.dirty = true;
            }
            Event::Move(_, y) => {
                if !self.dragging && (y - self.down_y).abs() > ui.pt(SPACE_M) {
                    self.dragging = true;
                    self.pressed = None;
                }
                if self.dragging {
                    let max = self.max_scroll(ui);
                    self.scroll = (self.scroll - (y - self.last_y)).clamp(0.0, max);
                    self.dirty = true;
                }
                self.last_y = y;
            }
            Event::Up(x, y) => {
                if let Some(p) = self.pressed.take() {
                    if !self.dragging && self.hit(ui, x, y) == Some(p) {
                        self.activate(p);
                    }
                }
                self.dragging = false;
                self.dirty = true;
            }
            _ => {}
        }
        true
    }

    pub fn tick(&mut self, _ui: &Ui, t: f64) -> bool {
        self.now = t;
        if matches!(&self.toast, Some((_, until)) if t >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        if matches!(&self.result, Some(p) if t >= p.until) {
            self.result = None;
            self.dirty = true;
        }
        self.dirty
    }

    pub fn next_wake(&self, _t: f64) -> Option<f64> {
        let toast = self.toast.as_ref().map(|(_, until)| *until);
        let panel = self.result.as_ref().map(|p| p.until);
        match (toast, panel) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn put(&mut self, s: &mut Surface, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32, h: f32, align: i8) {
        let img = self.cache.get(ui, text, pt, weight, color);
        let ix = match align {
            0 => x,
            1 => x - img.w as f32 / 2.0,
            _ => x - img.w as f32,
        };
        s.blit(img, ix as i32, (y + (h - img.h as f32) / 2.0) as i32, 255, 255);
    }

    /// Draws the result panel: the title, the dialog, the step rows, then the follow-on button and Done.
    fn draw_result(&mut self, s: &mut Surface, ui: &Ui, p: &ResultPanel) {
        let (x, y, w, h) = self.result_box(ui, p);
        s.fill_rrect(x, y, w, h, ui.pt(RESULT_RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, w, h, ui.pt(RESULT_RADIUS_PT), 1.0, SEPARATOR_C.now());
        let (pad, row_h) = (ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
        let inner = w - 2.0 * pad;
        let mut ty = y + pad;
        let title = ui.fit(&p.title, MAC_TITLE3_PT, inner);
        self.put(s, ui, &title, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), x + pad, ty, row_h, 0);
        ty += row_h;
        if let Some(d) = &p.dialog {
            let text = ui.fit(d, MAC_FOOTNOTE_PT, inner);
            self.put(s, ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + pad, ty, row_h, 0);
            ty += row_h;
        }
        // the step number in a narrow column, the step text beside it
        let col = ui.pt(SPACE_XXXL);
        for (label, text) in p.rows.iter().take(RESULT_MAX_ROWS) {
            self.put(s, ui, label, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + pad, ty, row_h, 0);
            let fitted = ui.fit(text, MAC_BODY_PT, inner - col);
            self.put(s, ui, &fitted, MAC_BODY_PT, Weight::Regular, LABEL.now(), x + pad + col, ty, row_h, 0);
            ty += row_h;
        }
        if p.rows.len() > RESULT_MAX_ROWS {
            let more = format!("and {} more", p.rows.len() - RESULT_MAX_ROWS);
            self.put(s, ui, &more, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x + pad, ty, row_h, 0);
        }
        let (follow, done) = self.result_buttons(ui, p);
        if let (Some((fx, fy, fw, fh)), Some(f)) = (follow, &p.follow) {
            let col = if self.pressed == Some(Target::ResultRun) { LABEL_2.now() } else { LINK };
            let text = ui.fit(&f.title, MAC_BODY_PT, fw);
            self.put(s, ui, &text, MAC_BODY_PT, Weight::Semibold, col, fx, fy, fh, 0);
        }
        let (dx, dy, dw, dh) = done;
        let col = if self.pressed == Some(Target::ResultDismiss) { LABEL_2.now() } else { LINK };
        self.put(s, ui, "Done", MAC_BODY_PT, Weight::Semibold, col, dx + dw, dy, dh, 2);
    }

    /// Draws the question panel: the prompt, the typed answer or the menu's choices, then Cancel (and Continue).
    fn draw_ask(&mut self, s: &mut Surface, ui: &Ui, a: &Asking) {
        let Some(p) = a.prompts.get(a.step) else { return };
        let (x, y, w, h) = self.ask_box(ui, a);
        s.fill_rrect(x, y, w, h, ui.pt(RESULT_RADIUS_PT), CARD_BG.now());
        s.stroke_rrect(x, y, w, h, ui.pt(RESULT_RADIUS_PT), 1.0, SEPARATOR_C.now());
        let (pad, row_h) = (ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
        let inner = w - 2.0 * pad;
        let title = ui.fit(&p.text, MAC_TITLE3_PT, inner);
        self.put(s, ui, &title, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), x + pad, y + pad, row_h, 0);
        if p.options.is_empty() {
            let (fx, fy, fw, fh) = self.ask_option(ui, a, 0);
            s.fill_rrect(fx, fy, fw, fh, ui.pt(6.0), FIELD_BG.now());
            let typed = ui.fit(&format!("{}|", a.buf), MAC_BODY_PT, fw - 2.0 * ui.pt(SPACE_S));
            self.put(s, ui, &typed, MAC_BODY_PT, Weight::Regular, LABEL.now(), fx + ui.pt(SPACE_S), fy, fh, 0);
        } else {
            for (k, option) in p.options.iter().take(ASK_MAX_OPTIONS).enumerate() {
                let (ox, oy, ow, oh) = self.ask_option(ui, a, k);
                if self.pressed == Some(Target::AskOption(k)) {
                    s.fill_rrect(ox, oy, ow, oh, ui.pt(6.0), ROW_PRESSED.now());
                }
                let text = ui.fit(option, MAC_BODY_PT, ow - 2.0 * ui.pt(SPACE_S));
                self.put(s, ui, &text, MAC_BODY_PT, Weight::Regular, LINK, ox + ui.pt(SPACE_S), oy, oh, 0);
            }
        }
        let (cancel, ok) = self.ask_buttons(ui, a);
        let col = if self.pressed == Some(Target::AskCancel) { LABEL_2.now() } else { LINK };
        self.put(s, ui, "Cancel", MAC_BODY_PT, Weight::Regular, col, cancel.0, cancel.1, cancel.3, 0);
        if let Some((ox, oy, ow, oh)) = ok {
            let col = if self.pressed == Some(Target::AskOk) { LABEL_2.now() } else { LINK };
            self.put(s, ui, "Continue", MAC_BODY_PT, Weight::Semibold, col, ox + ow, oy, oh, 2);
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        if self.cache.len() > 128 {
            self.cache.clear();
        }
        self.scroll = self.scroll.clamp(0.0, self.max_scroll(ui));
        let (cells, _) = self.layout(ui);
        let (top, x0) = (Self::top(ui), Self::side(ui));
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
        let r = ui.pt(CELL_RADIUS_PT);
        for c in cells.iter() {
            let (x, y, cw, ch) = c.rect;
            if y + ch < top || y > self.h as f32 {
                continue;
            }
            let pressed = c.target.is_some() && c.target == self.pressed;
            match &c.look {
                Look::Header(t) => self.put(s, ui, t, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y, ch, 0),
                Look::Tile { name, color, count, glyph: symbol } => {
                    s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), *color);
                    if pressed {
                        s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), PRESSED);
                    }
                    let (gx, gy, g) = (x + ui.pt(SPACE_L), y + ui.pt(SPACE_L), ui.pt(SPACE_XXL));
                    glyph(s, ui, symbol, PtRect { x: gx / ui.s, y: gy / ui.s, w: g / ui.s, h: g / ui.s }, WHITE);
                    if self.edit_mode {
                        self.put(s, ui, "Edit", MAC_FOOTNOTE_PT, Weight::Semibold, WHITE, x + cw - ui.pt(SPACE_L), y + ui.pt(SPACE_M), ui.pt(SPACE_XL), 2);
                    } else if *count > 0 {
                        let n = format!("{count} {}", if *count == 1 { "action" } else { "actions" });
                        self.put(s, ui, &n, MAC_CAPTION1_PT, Weight::Regular, WHITE, x + cw - ui.pt(SPACE_L), y + ui.pt(SPACE_M), ui.pt(SPACE_XL), 2);
                    }
                    let fitted = ui.fit(name, MAC_BODY_PT, cw - ui.pt(28.0));
                    self.put(s, ui, &fitted, MAC_BODY_PT, Weight::Semibold, WHITE, x + ui.pt(SPACE_L), y + ch - ui.pt(SPACE_HUGE), ui.pt(28.0), 0);
                }
                Look::CreateTile => {
                    s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), if pressed { CONTROL_PRESSED.now() } else { CARD_BG.now() });
                    self.put(s, ui, "+", MAC_TITLE1_PT, Weight::Regular, SYSTEM_BLUE, x + ui.pt(SPACE_L), y + ui.pt(SPACE_S), ui.pt(SPACE_HUGE), 0);
                    self.put(s, ui, "Create Shortcut", MAC_BODY_PT, Weight::Semibold, LABEL.now(), x + ui.pt(SPACE_L), y + ch - ui.pt(SPACE_HUGE), ui.pt(28.0), 0);
                }
                Look::Row { text, value, tint, pos, chevron, centre, lead, inset } => {
                    let fill = if pressed { ROW_PRESSED.now() } else { CARD_BG.now() };
                    if pos & ROW_POS_FIRST != 0 && pos & ROW_POS_LAST != 0 {
                        s.fill_rrect(x, y, cw, ch, r, fill);
                    } else if pos & ROW_POS_FIRST != 0 {
                        s.fill_rrect(x, y, cw, ch + r, r, fill);
                    } else if pos & ROW_POS_LAST != 0 {
                        s.fill_rrect(x, y - r, cw, ch + r, r, fill);
                    } else {
                        s.fill_rect(x as usize, y.max(0.0) as usize, cw as usize, ch as usize, fill);
                    }
                    if pos & ROW_POS_LAST == 0 {
                        let sx = x + ui.pt(46.0);
                        s.fill_rect(sx as usize, (y + ch - 1.0).max(0.0) as usize, (x + cw - sx) as usize, 1, SEPARATOR_C.now());
                    }
                    if *centre {
                        self.put(s, ui, text, MAC_BODY_PT, Weight::Regular, *tint, x + cw / 2.0, y, ch, 1);
                    } else if !text.is_empty() {
                        let title = ui.fit(text, MAC_BODY_PT, (cw - lead - inset).max(ui.pt(40.0)));
                        let title_w = ui.measure(&title, MAC_BODY_PT, Weight::Regular);
                        self.put(s, ui, &title, MAC_BODY_PT, Weight::Regular, *tint, x + lead, y, ch, 0);
                        if !value.is_empty() {
                            let room = (cw - inset - lead - title_w - ui.pt(SPACE_L)).max(ui.pt(40.0));
                            let v = ui.fit(value, MAC_BODY_PT, room);
                            self.put(s, ui, &v, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), x + cw - inset, y, ch, 2);
                        }
                    }
                    if *chevron {
                        self.put(s, ui, ">", MAC_BODY_PT, Weight::Semibold, LABEL_3.now(), x + cw - ui.pt(SPACE_L), y, ch, 2);
                    }
                }
                Look::Minus => {
                    let (d, cx, cy) = (ui.pt(SPACE_XXL), x + ui.pt(SPACE_XXL), y + ch / 2.0);
                    s.fill_rrect(cx - d / 2.0, cy - d / 2.0, d, d, d / 2.0, SYSTEM_RED);
                    let bar = ui.pt(SPACE_XXS);
                    s.fill_rrect(cx - d * 0.28, cy - bar / 2.0, d * 0.56, bar, bar / 2.0, ON_BADGE);
                }
                Look::Stepper(up) => {
                    let col = if pressed { LABEL_2.now() } else { LINK };
                    self.put(s, ui, if *up { "+" } else { "\u{2212}" }, MAC_TITLE1_PT, Weight::Regular, col, x + cw / 2.0, y, ch, 1);
                }
                Look::Symbol(name) => {
                    let col = if pressed { LABEL_2.now() } else { LABEL_3.now() };
                    let g = ui.pt(12.0);
                    glyph(s, ui, name, PtRect { x: (x + (cw - g) / 2.0) / ui.s, y: (y + (ch - g) / 2.0) / ui.s, w: g / ui.s, h: g / ui.s }, col);
                }
                Look::Note(text) => {
                    let fitted = ui.fit(text, MAC_FOOTNOTE_PT, cw);
                    self.put(s, ui, &fitted, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now(), x, y, ch, 0);
                }
                Look::Swatch { color, on } => {
                    if *on {
                        s.stroke_rrect(x - 4.0, y - 4.0, cw + 8.0, ch + 8.0, (cw + 8.0) / 2.0, ui.pt(SPACE_XXS), LABEL.now());
                    }
                    s.fill_rrect(x, y, cw, ch, cw / 2.0, *color);
                }
                Look::GlyphChip { name, on } => {
                    if *on {
                        s.fill_rrect(x - 2.0, y - 2.0, cw + 4.0, ch + 4.0, ui.pt(6.0), CONTROL_PRESSED.now());
                    }
                    let g = cw * 0.7;
                    glyph(s, ui, name, PtRect { x: (x + (cw - g) / 2.0) / ui.s, y: (y + (ch - g) / 2.0) / ui.s, w: g / ui.s, h: g / ui.s }, LABEL.now());
                }
            }
        }
        // the unified toolbar over the content, then the sidebar beside it
        let wp = self.w as f32 / ui.s;
        widgets::toolbar_band(s, ui, PtRect { x: SIDEBAR_W, y: 0.0, w: wp - SIDEBAR_W, h: UNIFIED_BAR_PT });
        let pressed = self.bar(ui).items.iter().map(|i| i.id).find(|id| Self::bar_target(*id).is_some() && Self::bar_target(*id) == self.pressed);
        let placed = widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(ui), pressed);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let fitted = ui.fit(&self.title(), MAC_TITLE3_PT, ui.pt(r.w));
            self.put(s, ui, &fitted, MAC_TITLE3_PT, Weight::Bold, LABEL.now(), ui.pt(r.x), ui.pt(r.y), ui.pt(r.h), 0);
        }
        if let Some(r) = placed.rect_of(BAR_SEARCH) {
            widgets::search_field(s, ui, &self.search, r, "Search", self.search_focus);
        }
        widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), -(UNIFIED_BAR_PT + 4.0), None);
        s.fill_rect(x0 as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        if let Some(a) = self.asking.clone() {
            self.draw_ask(s, ui, &a);
        } else if let Some(p) = self.result.clone() {
            self.draw_result(s, ui, &p);
        } else if let Some((msg, _)) = self.toast.clone() {
            let img = self.cache.get(ui, &msg, MAC_CALLOUT_PT, Weight::Regular, WHITE);
            let pw = img.w as f32 + ui.pt(SPACE_XXXL);
            let ph = img.h as f32 + ui.pt(SPACE_L);
            let x = x0 + (self.w as f32 - x0 - pw) / 2.0;
            let y = self.h as f32 - ph - ui.pt(SPACE_XXL);
            s.fill_rrect(x, y, pw, ph, 1.0e4, TOAST_FILL);
            s.blit(img, (x + ui.pt(SPACE_L)) as i32, (y + ui.pt(SPACE_S)) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::Font;

    fn ui() -> Ui {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn window(tag: &str) -> (Shortcuts, Ui) {
        let dir = std::env::temp_dir().join(format!("shortcuts-widget-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ui = ui();
        (Shortcuts::new(&ui, 1200, 780, dir.join("shortcuts.tsv")), ui)
    }

    /// The centre of the first cell with `target`, in pixels.
    fn centre(w: &Shortcuts, ui: &Ui, target: Target) -> Option<(f32, f32)> {
        let mid = |r: Rect| (r.0 + r.2 / 2.0, r.1 + r.3 / 2.0);
        if let Some(a) = &w.asking {
            let (cancel, ok) = w.ask_buttons(ui, a);
            match target {
                Target::AskCancel => return Some(mid(cancel)),
                Target::AskOk => return ok.map(mid),
                Target::AskOption(k) => return Some(mid(w.ask_option(ui, a, k))),
                _ => {}
            }
        }
        if let Some(id) = (1..=6).find(|id| Shortcuts::bar_target(*id) == Some(target)) {
            let r = w.bar_layout(ui).rect_of(id)?;
            return Some(((r.x + r.w / 2.0) * ui.s, (r.y + r.h / 2.0) * ui.s));
        }
        let (cells, _) = w.layout(ui);
        cells.iter().find(|c| c.target == Some(target)).map(|c| (c.rect.0 + c.rect.2 / 2.0, c.rect.1 + c.rect.3 / 2.0))
    }

    /// Taps `target` where it is drawn; false when it is not on screen.
    fn tap(w: &mut Shortcuts, ui: &Ui, target: Target) -> bool {
        let Some((x, y)) = centre(w, ui, target) else { return false };
        w.touch(ui, Event::Down(x, y), 1.0);
        w.touch(ui, Event::Up(x, y), 2.0);
        true
    }

    fn type_in(w: &mut Shortcuts, text: &str) {
        assert!(w.key(&KeyInput::Text(text.into())));
    }

    #[test]
    fn a_tap_on_create_opens_a_new_shortcut_in_the_editor() {
        let (mut w, ui) = window("create");
        let before = w.list().len();
        assert!(tap(&mut w, &ui, Target::Create));
        assert_eq!(w.list().len(), before + 1);
        assert_eq!(w.open_shortcut(), Some(before));
        assert_eq!(w.list()[before].name, "Shortcut 1");
    }

    #[test]
    fn the_name_is_renamed_by_typing_and_return() {
        let (mut w, ui) = window("rename");
        w.edit_mode = true;
        w.activate(Target::Tile(0));
        assert_eq!(w.open_shortcut(), Some(0));
        assert!(tap(&mut w, &ui, Target::EditName));
        assert!(w.wants_keyboard());
        for _ in 0..20 {
            w.key(&KeyInput::Code(8));
        }
        type_in(&mut w, "Night");
        assert!(w.key(&KeyInput::Code(13)));
        assert_eq!(w.list()[0].name, "Night");
        assert!(!w.wants_keyboard());
        assert_eq!(model::load(&w.path)[0].name, "Night", "saved");
    }

    #[test]
    fn escape_drops_what_was_typed_and_an_empty_name_is_kept_as_it_was() {
        let (mut w, _ui) = window("escape");
        w.open_editor(1);
        let old = w.list()[1].name.clone();
        w.activate(Target::EditName);
        type_in(&mut w, "zzz");
        w.key(&KeyInput::Code(27));
        assert_eq!(w.list()[1].name, old);
        w.activate(Target::EditName);
        for _ in 0..60 {
            w.key(&KeyInput::Code(8));
        }
        w.key(&KeyInput::Code(13));
        assert_eq!(w.list()[1].name, old, "an empty name is refused");
    }

    #[test]
    fn tapping_elsewhere_commits_the_typed_text() {
        let (mut w, _ui) = window("commit");
        w.open_editor(0);
        w.activate(Target::EditFolder);
        type_in(&mut w, "Night");
        w.activate(Target::Color(2));
        assert_eq!(w.list()[0].folder, "Night");
        assert_eq!(w.folder_rows(), vec![("Night".to_string(), 1)]);
    }

    #[test]
    fn the_sidebar_lists_folders_and_filters_the_grid() {
        let (mut w, ui) = window("folders");
        w.list[0].folder = "Night".into();
        w.list[1].folder = "Power".into();
        assert!(w.sidebar().items.len() >= 6);
        w.activate(Target::Side(ROW_FOLDER));
        assert_eq!(w.folder.as_deref(), Some("Night"));
        let (cells, _) = w.layout(&ui);
        let tiles = cells.iter().filter(|c| matches!(c.look, Look::Tile { .. })).count();
        assert_eq!(tiles, 1);
        assert_eq!(w.title(), "Night");
        w.activate(Target::Side(ROW_ALL));
        assert_eq!(w.folder, None);
        // the last shortcut leaving a folder drops the selection
        w.activate(Target::Side(ROW_FOLDER));
        w.list[0].folder.clear();
        w.fix_selection();
        assert_eq!(w.folder, None);
    }

    #[test]
    fn a_new_shortcut_joins_the_folder_on_screen() {
        let (mut w, _ui) = window("newin");
        w.list[2].folder = "Work".into();
        w.folder = Some("Work".into());
        w.new_shortcut();
        let i = w.open_shortcut().unwrap();
        assert_eq!(w.list()[i].folder, "Work");
    }

    #[test]
    fn typing_in_the_search_field_filters_the_tiles() {
        let (mut w, ui) = window("search");
        assert!(tap(&mut w, &ui, Target::Search));
        assert!(w.wants_keyboard());
        type_in(&mut w, "timer");
        let (cells, _) = w.layout(&ui);
        let tiles: Vec<usize> = cells.iter().filter_map(|c| if let Some(Target::Tile(i)) = c.target { Some(i) } else { None }).collect();
        assert_eq!(tiles, vec![2]);
        assert!(!cells.iter().any(|c| matches!(c.look, Look::CreateTile)), "no create tile while searching");
        type_in(&mut w, "zzz");
        let (cells, _) = w.layout(&ui);
        assert!(cells.iter().any(|c| matches!(&c.look, Look::Note(t) if t.contains("match"))));
        w.key(&KeyInput::Code(27));
        assert!(!w.wants_keyboard());
        assert!(w.search.is_empty(), "Escape clears the search");
    }

    #[test]
    fn the_search_field_is_a_control_and_find_focuses_it() {
        let (mut w, ui) = window("find");
        let nodes = w.controls(&ui);
        assert!(nodes.iter().any(|n| n.role == Role::TextField && n.label == "Search"));
        w.open_editor(0);
        w.focus_search();
        assert_eq!(w.view, View::Grid);
        assert!(w.wants_keyboard());
    }

    #[test]
    fn actions_are_added_moved_and_removed_from_the_editor() {
        let (mut w, ui) = window("actions");
        w.open_editor(2);
        let before = w.list()[2].actions.len();
        assert!(tap(&mut w, &ui, Target::AddAction));
        assert!(matches!(w.view, View::Picker(2)));
        assert!(tap(&mut w, &ui, Target::Pick(11)));
        assert_eq!(w.list()[2].actions[before], Action::Text("Hello".into()));
        assert!(tap(&mut w, &ui, Target::Up(before)));
        assert_eq!(w.list()[2].actions[before - 1], Action::Text("Hello".into()));
        assert!(tap(&mut w, &ui, Target::Down(before - 1)));
        assert_eq!(w.list()[2].actions[before], Action::Text("Hello".into()));
        assert!(tap(&mut w, &ui, Target::Remove(before)));
        assert_eq!(w.list()[2].actions.len(), before);
    }

    #[test]
    fn a_block_comes_with_its_end_and_leaves_with_it() {
        let (mut w, _ui) = window("block");
        w.open_editor(2);
        let before = w.list()[2].actions.len();
        w.activate(Target::AddAction);
        w.activate(Target::Pick(17));
        assert_eq!(w.list()[2].actions.len(), before + 2);
        assert_eq!(w.list()[2].actions[before + 1], Action::End);
        w.activate(Target::Remove(before));
        assert_eq!(w.list()[2].actions.len(), before);
    }

    #[test]
    fn the_text_of_an_action_is_typed_into_its_row_with_a_variable() {
        let (mut w, ui) = window("text");
        w.open_editor(2);
        w.list[2].actions = vec![Action::Text("a".into()), Action::SetVariable("Who".into()), Action::Text("b".into())];
        assert!(tap(&mut w, &ui, Target::EditText(2)));
        for _ in 0..5 {
            w.key(&KeyInput::Code(8));
        }
        type_in(&mut w, "Hi ");
        assert!(tap(&mut w, &ui, Target::InsertVar));
        assert!(tap(&mut w, &ui, Target::InsertVar));
        w.key(&KeyInput::Code(13));
        assert_eq!(w.list()[2].actions[2], Action::Text("Hi {Previous}{Who}".into()));
    }

    #[test]
    fn a_paste_goes_into_the_field_with_the_keyboard() {
        let (mut w, _ui) = window("paste");
        assert!(!w.paste("nothing has the keyboard"));
        w.open_editor(0);
        w.activate(Target::EditFolder);
        assert!(w.paste("Pasted\nsecond line"));
        w.key(&KeyInput::Code(13));
        assert_eq!(w.list()[0].folder, "Pasted");
    }

    #[test]
    fn details_rows_step_the_key_reminder_trigger_and_glyph() {
        let (mut w, ui) = window("details");
        w.open_editor(0);
        assert!(tap(&mut w, &ui, Target::Hotkey));
        assert_eq!(w.list()[0].hotkey, "cmd+1");
        assert!(tap(&mut w, &ui, Target::Schedule));
        assert!(w.list()[0].schedule.is_some());
        assert!(tap(&mut w, &ui, Target::TrigKind));
        assert_eq!(w.list()[0].trigger, Trigger::AppOpen("Notes".into()));
        assert!(tap(&mut w, &ui, Target::TrigInc));
        assert_eq!(w.list()[0].trigger, Trigger::AppOpen("Calendar".into()));
        assert!(tap(&mut w, &ui, Target::TrigKind));
        assert_eq!(w.list()[0].trigger, Trigger::BatteryBelow(20));
        assert!(tap(&mut w, &ui, Target::Glyph(3)));
        assert_eq!(w.list()[0].glyph, 3);
        assert_eq!(model::load(&w.path)[0].glyph, 3);
    }

    #[test]
    fn duplicate_copies_the_open_shortcut_and_opens_the_copy() {
        let (mut w, ui) = window("dup");
        w.open_editor(0);
        let n = w.list().len();
        assert!(tap(&mut w, &ui, Target::Duplicate));
        assert_eq!(w.list().len(), n + 1);
        assert_eq!(w.open_shortcut(), Some(1));
        assert_eq!(w.list()[1].name, "Bedtime copy");
        assert_eq!(w.list()[1].actions, w.list()[0].actions);
    }

    #[test]
    fn delete_removes_the_open_shortcut_and_returns_to_the_grid() {
        let (mut w, ui) = window("del");
        w.open_editor(0);
        let n = w.list().len();
        assert!(tap(&mut w, &ui, Target::Delete));
        assert_eq!(w.list().len(), n - 1);
        assert_eq!(w.view, View::Grid);
    }

    #[test]
    fn running_a_text_shortcut_shows_its_result() {
        let (mut w, _ui) = window("run");
        w.list[0].actions = vec![Action::Text("Hello".into()), Action::ShowResult];
        w.activate(Target::Tile(0));
        let p = w.result.clone().expect("a result panel");
        assert_eq!(p.title, "Bedtime");
        assert_eq!(p.rows, vec![("1".to_string(), "Hello".to_string())]);
        assert!(w.take_runs().is_empty());
    }

    #[test]
    fn a_shortcut_with_system_steps_queues_them_for_the_shell() {
        let (mut w, _ui) = window("effects");
        w.list[0].actions = vec![Action::Dnd(true), Action::Repeat(2), Action::Screenshot, Action::End];
        w.activate(Target::Tile(0));
        assert_eq!(w.take_runs(), vec![Action::Dnd(true), Action::Screenshot, Action::Screenshot]);
        assert!(w.result.is_none());
        assert!(w.toast.as_ref().is_some_and(|(m, _)| m.starts_with("Ran")));
    }

    #[test]
    fn ask_for_input_waits_for_the_answer_before_running() {
        let (mut w, ui) = window("ask");
        w.list[0].actions = vec![Action::AskInput("Name?".into()), Action::SetVariable("N".into()), Action::Text("Hi {N}".into()), Action::ShowResult];
        w.activate(Target::Tile(0));
        assert!(w.asking.is_some() && w.wants_keyboard());
        assert!(w.result.is_none());
        type_in(&mut w, "Ada");
        assert!(tap(&mut w, &ui, Target::AskOk));
        assert!(w.asking.is_none());
        assert_eq!(w.result.as_ref().unwrap().rows[0].1, "Hi Ada");
    }

    #[test]
    fn return_answers_and_cancel_stops_the_run() {
        let (mut w, ui) = window("askkeys");
        w.list[0].actions = vec![Action::AskInput("Q".into()), Action::ShowResult];
        w.activate(Target::Tile(0));
        type_in(&mut w, "yes");
        w.key(&KeyInput::Code(13));
        assert_eq!(w.result.as_ref().unwrap().rows[0].1, "yes");
        w.result = None;
        w.activate(Target::Tile(0));
        assert!(tap(&mut w, &ui, Target::AskCancel));
        assert!(w.asking.is_none() && w.result.is_none());
    }

    #[test]
    fn a_menu_question_lists_its_choices_and_runs_the_chosen_branch() {
        let (mut w, ui) = window("menu");
        w.list[0].actions = vec![
            Action::Menu("Pick".into()),
            Action::MenuItem("A".into()),
            Action::Text("first".into()),
            Action::MenuItem("B".into()),
            Action::Text("second".into()),
            Action::End,
            Action::ShowResult,
        ];
        w.activate(Target::Tile(0));
        assert!(!w.wants_keyboard(), "a menu question takes taps, not typing");
        assert!(tap(&mut w, &ui, Target::AskOption(1)));
        assert_eq!(w.result.as_ref().unwrap().rows[0].1, "second");
    }

    #[test]
    fn a_modal_question_ignores_taps_elsewhere() {
        let (mut w, ui) = window("modal");
        w.list[0].actions = vec![Action::AskInput("Q".into())];
        w.activate(Target::Tile(0));
        assert_eq!(w.hit(&ui, 600.0, 200.0), None);
    }

    #[test]
    fn saved_files_and_web_requests_are_reported() {
        let (mut w, _ui) = window("files");
        w.list[0].actions = vec![Action::Text("data".into()), Action::SaveFile("out.txt".into()), Action::GetUrl("https://example.com".into())];
        w.activate(Target::Tile(0));
        assert_eq!(w.take_saves(), vec![("out.txt".to_string(), "data".to_string())]);
        assert!(w.result.as_ref().unwrap().rows[0].1.contains("https://example.com"));
        assert!(w.take_saves().is_empty());
    }

    #[test]
    fn the_gallery_adds_a_copy_of_a_starter() {
        let (mut w, ui) = window("gallery");
        w.activate(Target::Side(ROW_GALLERY));
        assert_eq!(w.view, View::Gallery);
        assert_eq!(w.title(), "Gallery");
        let n = w.list().len();
        assert!(tap(&mut w, &ui, Target::GalleryAdd(0)));
        assert_eq!(w.list().len(), n + 1);
        assert_eq!(w.list()[n].name, "Morning Routine");
        assert!(tap(&mut w, &ui, Target::GalleryAdd(0)));
        assert_eq!(w.list()[n + 1].name, "Morning Routine 2");
    }

    #[test]
    fn export_asks_for_a_save_panel_and_import_adds_the_shortcut() {
        let (mut w, _ui) = window("files2");
        w.open_editor(1);
        w.activate(Target::Export);
        let jobs = w.take_file_jobs();
        assert_eq!(jobs.len(), 1);
        let FileJob::Export { name, text } = &jobs[0] else { panic!("export job") };
        assert_eq!(name, "Save Battery.shortcut");
        let n = w.list().len();
        assert_eq!(w.import(text), 1);
        assert_eq!(w.list().len(), n + 1);
        assert_eq!(w.list()[n].name, "Save Battery 2");
        assert_eq!(w.import("junk"), 0);
        assert!(w.toast.as_ref().is_some_and(|(m, _)| m.starts_with("Could not import")));
        w.request_import();
        assert_eq!(w.take_file_jobs(), vec![FileJob::Import]);
    }

    #[test]
    fn a_battery_automation_runs_once_while_the_condition_holds() {
        let (mut w, _ui) = window("battery");
        w.list[1].trigger = Trigger::BatteryBelow(30);
        assert!(w.has_battery_trigger());
        assert_eq!(w.check_triggers(Some(80)), Vec::<String>::new());
        assert_eq!(w.check_triggers(Some(25)), vec!["Save Battery".to_string()]);
        assert_eq!(w.take_runs().len(), 2, "its two system steps are queued");
        assert_eq!(w.check_triggers(Some(20)), Vec::<String>::new(), "still low: no second run");
        assert_eq!(w.check_triggers(Some(60)), Vec::<String>::new());
        assert_eq!(w.check_triggers(Some(10)), vec!["Save Battery".to_string()], "it runs again after the charge recovered");
        assert_eq!(w.check_triggers(None), Vec::<String>::new());
    }

    #[test]
    fn the_reorder_arrows_and_steppers_are_laid_out_on_every_action_row() {
        let (mut w, ui) = window("rows");
        w.open_editor(0);
        let (cells, _) = w.layout(&ui);
        let n = w.list()[0].actions.len();
        for k in 0..n {
            assert!(cells.iter().any(|c| c.target == Some(Target::Up(k))));
            assert!(cells.iter().any(|c| c.target == Some(Target::Down(k))));
            assert!(cells.iter().any(|c| c.target == Some(Target::Remove(k))));
        }
    }

    #[test]
    fn run_from_the_menu_runs_the_shortcut_by_index() {
        let (mut w, _ui) = window("menurun");
        w.run_index(2);
        assert_eq!(w.take_runs(), vec![Action::Timer(10)]);
    }

    #[test]
    fn the_toolbar_has_search_and_add_in_the_grid_only() {
        let (mut w, ui) = window("bar");
        assert!(w.bar_layout(&ui).rect_of(BAR_SEARCH).is_some());
        assert!(w.bar_layout(&ui).rect_of(BAR_ADD).is_some());
        w.show_gallery();
        assert!(w.bar_layout(&ui).rect_of(BAR_SEARCH).is_none());
        w.open_editor(0);
        assert!(w.bar_layout(&ui).rect_of(BAR_RUN).is_some());
    }
}
