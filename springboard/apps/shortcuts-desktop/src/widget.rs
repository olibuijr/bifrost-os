//! Shortcuts: a grid of coloured tiles with a glyph each, a sidebar with the gallery and folders, a search field, and
//! an editor that shows a shortcut as a list of steps (text, variables, If and Repeat blocks, and the built-in actions
//! for Wi-Fi, Low Power Mode, Do Not Disturb, brightness, apps, screenshots and timers). The editor renames,
//! duplicates, reorders, files and exports a shortcut. One text field serves the search, the name and the text of a
//! step.

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXS, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CALLOUT_PT, MAC_CAPTION1_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT};
use crate::automation;
use springboard::shortcuts as model;
use bifrost_app::kit::ui::{
    Ui, Weight, CARD_BG, CARD_RADIUS_PT, CELL_RADIUS_PT, CONTROL_PRESSED, CONTENT_BG, LABEL, LABEL_2, LABEL_3, LINK,
    MAC_TITLE3_PT, PRESSED, ROW_PRESSED, SEPARATOR_C, SYSTEM_BLUE, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PINK,
    SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_TEAL, SYSTEM_YELLOW, TOAST_FILL, WHITE,
};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{search, Node, Rect as PtRect, Role, TextField};
use bifrost_app::layout::UNIFIED_BAR_PT;
use bifrost_app::{glyphs, widgets, KeyInput};
use bifrost_app::kit::{Event, Surface, TextCache};
use springboard::shortcuts::flow::{self, Step};
use springboard::shortcuts::{Action, Daily, Shortcut, FOLDERS, GLYPHS, KINDS, MAX_ACTIONS, MAX_SHORTCUTS};
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

/// Tile colours a shortcut can take, as the Shortcuts app offers them.
pub const PALETTE: [u32; 8] =
    [SYSTEM_RED, SYSTEM_ORANGE, SYSTEM_YELLOW, SYSTEM_GREEN, SYSTEM_TEAL, SYSTEM_BLUE, SYSTEM_PURPLE, SYSTEM_PINK];

type Rect = (f32, f32, f32, f32);

/// Sidebar width in points (Shortcuts' Library column).
const SIDEBAR_W: f32 = 200.0;
/// Grid tile: the narrowest a tile may get and its height, in points.
const TILE_MIN_W: f32 = 200.0;
const TILE_H: f32 = 100.0;
/// The glyph on the red Remove badge: white is text on the accent here.
const ON_BADGE: u32 = WHITE;
/// Sidebar row ids: All Shortcuts, the Gallery, and the folders (`ROW_FOLDER` plus the folder number).
const ROW_ALL: u32 = 100;
const ROW_GALLERY: u32 = 101;
const ROW_FOLDER: u32 = 110;
/// Toolbar item ids; they double as control ids.
const BAR_BACK: u32 = 1;
const BAR_TITLE: u32 = 2;
const BAR_ADD: u32 = 3;
const BAR_EDIT: u32 = 4;
const BAR_RUN: u32 = 5;
const BAR_SEARCH: u32 = 6;
/// The widest the toolbar title may get, in points.
const TITLE_MAX_W: f32 = 320.0;
/// The search field's width in the grid's toolbar, in points.
const SEARCH_W: f32 = 180.0;
/// How far from the right edge a row's value ends, in points: past the stepper, and past the move buttons on a step.
const VALUE_INSET: f32 = 96.0;
const STEP_VALUE_INSET: f32 = 172.0;
/// How far each level of a block indents its steps, in points.
const INDENT_PT: f32 = 16.0;
/// The width of a move button on a step row, in points.
const MOVE_W: f32 = 38.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Grid,
    Editor(usize),
    Picker(usize),
    Gallery,
}

/// Which shortcuts the grid lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Filter {
    All,
    /// a folder, by its number (1-based, as stored)
    Folder(usize),
}

/// What the text field is typing into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edit {
    None,
    /// the search field of the toolbar
    Search,
    /// the name of the shortcut in the editor
    Name,
    /// the text of step `k` of the shortcut in the editor
    Value(usize),
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
    /// moves step `k` up or down one place
    Up(usize),
    Down(usize),
    /// the text of step `k`: opens the field
    Value(usize),
    AddAction,
    Pick(usize),
    /// An intent in the Intents section of the action library: runs it on the shortcut in front.
    Intent(usize),
    Color(usize),
    Glyph(usize),
    Delete,
    Duplicate,
    /// the name row of the open shortcut: opens the field
    Name,
    /// the "Folder" row of the open shortcut: steps to the next folder
    Folder,
    /// "Export Shortcut..." in the editor
    Export,
    /// "Import from File..." in the gallery
    Import,
    /// a starter in the gallery: adds it
    Starter(usize),
    /// The follow-on button of the result panel: asks for the intent the result opens.
    ResultRun,
    /// The Done button of the result panel.
    ResultDismiss,
    /// The "Daily Reminder" row of the open shortcut: steps to the next time.
    Schedule,
    /// The "Open at Login" row of the grid: asks the shell to turn it on or off.
    LoginItem,
    /// A row of the sidebar.
    Side(u32),
    /// The search field of the toolbar, and its clear button.
    Search,
    SearchClear,
}

/// What a menu asks the window to do.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    New,
    Duplicate,
    Delete,
    Rename,
    Find,
    Gallery,
    /// runs the shortcut in the editor
    RunOpen,
    /// runs shortcut number `i` of the list
    Run(usize),
    Export,
    Import,
}

enum Look {
    Tile { name: String, color: u32, count: usize, glyph: usize },
    CreateTile,
    Header(String),
    Row { text: String, value: String, tint: u32, pos: u8, chevron: bool, centre: bool, indent: f32, inset: f32 },
    Minus,
    Stepper(bool),
    /// a move button of a step row: up when true
    Move(bool),
    Swatch { color: u32, on: bool },
    GlyphCell { glyph: usize, on: bool },
    /// A footer under a catalog row: the intent's description.
    Note(String),
}

struct Cell {
    target: Option<Target>,
    rect: Rect,
    look: Look,
}

/// The follow-on an intent result opens: the intent to run next and the parameters it gets.
#[derive(Clone, Debug)]
struct Follow {
    intent: String,
    title: String,
    params: Json,
}

/// The result panel of an intent run that carries a snippet (the plan steps), or of a run of a shortcut.
#[derive(Clone, Debug)]
struct ResultPanel {
    title: String,
    dialog: Option<String>,
    rows: Vec<(String, String)>,
    follow: Option<Follow>,
    until: f64,
}

pub struct Shortcuts {
    w: i32,
    h: i32,
    path: PathBuf,
    list: Vec<Shortcut>,
    view: View,
    filter: Filter,
    edit_mode: bool,
    scroll: f32,
    pressed: Option<Target>,
    down_y: f32,
    last_y: f32,
    dragging: bool,
    /// the shortcuts the last taps asked to run, oldest first
    run_requests: Vec<Shortcut>,
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
    /// the search text of the toolbar's field
    query: TextField,
    /// the text of the name or step being typed
    field: TextField,
    edit: Edit,
    /// an export the last tap asked for: the suggested file name and the file's text
    export_request: Option<(String, String)>,
    /// whether the last tap asked to import a file
    import_request: bool,
}

const ROW_POS_FIRST: u8 = 1;
const ROW_POS_LAST: u8 = 2;

/// Whether a step only closes or splits a block, and so has no remove or move buttons of its own.
fn is_marker(a: &Action) -> bool {
    matches!(a, Action::Otherwise | Action::EndIf | Action::EndRepeat)
}

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
            filter: Filter::All,
            edit_mode: false,
            scroll: 0.0,
            pressed: None,
            down_y: 0.0,
            last_y: 0.0,
            dragging: false,
            run_requests: Vec::new(),
            toast: None,
            now: 0.0,
            dirty: true,
            cache: TextCache::default(),
            login: None,
            login_request: None,
            intent_requests: Vec::new(),
            result: None,
            query: TextField::new(),
            field: TextField::new(),
            edit: Edit::None,
            export_request: None,
            import_request: false,
        }
    }

    /// The list as the window holds it now. Intents and scripts read it, so they never see a stale copy.
    pub fn list(&self) -> &[Shortcut] {
        &self.list
    }

    /// The shortcuts on screen as (index, in front). The grid shows the tiles that match the folder and the search and
    /// none is in front; an open editor or picker shows its one shortcut in front; the gallery shows none.
    pub fn on_screen(&self) -> Vec<(usize, bool)> {
        match self.view {
            View::Grid => self.shown().into_iter().map(|i| (i, false)).collect(),
            View::Editor(i) | View::Picker(i) => {
                if i < self.list.len() {
                    vec![(i, true)]
                } else {
                    Vec::new()
                }
            }
            View::Gallery => Vec::new(),
        }
    }

    /// The positions of the tiles the grid lists: those in the chosen folder that match the search.
    fn shown(&self) -> Vec<usize> {
        let folder = match self.filter {
            Filter::All => None,
            Filter::Folder(f) => Some(f),
        };
        model::visible(&self.list, folder, &self.query)
    }

    /// Runs `f` on a copy of the list and keeps the copy, and saves it, only when `f` changed it. Intents and
    /// scripts change the list through here, so the window and the saved file stay the same.
    pub fn edit_list<T>(&mut self, f: impl FnOnce(&mut Vec<Shortcut>) -> T) -> T {
        let mut next = self.list.clone();
        let out = f(&mut next);
        if next != self.list {
            self.list = next;
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

    /// The time of the last input or tick, in milliseconds.
    pub fn now(&self) -> f64 {
        self.now
    }

    /// The shortcuts the last taps asked to run, oldest first.
    pub fn take_run_requests(&mut self) -> Vec<Shortcut> {
        std::mem::take(&mut self.run_requests)
    }

    /// Shows how a run of shortcut `name` went: a panel with its steps, each with what happened, and whether the run
    /// finished or stopped.
    pub fn show_run(&mut self, name: &str, steps: &[Step], t: f64) {
        let stopped = steps.iter().position(|s| s.outcome.is_err());
        let dialog = match stopped {
            Some(k) => format!("Stopped at step {}", k + 1),
            None => "Finished".to_string(),
        };
        let rows = steps
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let text = match &s.outcome {
                    Ok(m) if m.is_empty() => s.line.clone(),
                    Ok(m) | Err(m) => format!("{} - {m}", s.line),
                };
                ((k + 1).to_string(), text)
            })
            .collect();
        self.toast = None;
        self.result = Some(ResultPanel { title: name.to_string(), dialog: Some(dialog), rows, follow: None, until: t + RESULT_MS });
        self.dirty = true;
    }

    /// The intents the last taps asked to run from the action library (or a result's follow-on), oldest first, with
    /// their parameters.
    pub fn take_intent_requests(&mut self) -> Vec<(String, Json)> {
        std::mem::take(&mut self.intent_requests)
    }

    /// The export the last tap or menu asked for, once: the suggested file name and the text to write.
    pub fn take_export_request(&mut self) -> Option<(String, String)> {
        self.export_request.take()
    }

    /// Whether the last tap or menu asked to import a file, once.
    pub fn take_import_request(&mut self) -> bool {
        std::mem::take(&mut self.import_request)
    }

    /// Adds the shortcuts of an export file to the list and says how it went.
    pub fn import_text(&mut self, text: &str, t: f64) {
        let mut next = self.list.clone();
        let msg = match model::import(&mut next, text) {
            Ok(n) => {
                self.list = next;
                self.save();
                if n == 1 { "Imported 1 shortcut".to_string() } else { format!("Imported {n} shortcuts") }
            }
            Err(why) => why,
        };
        self.say(&msg, t);
    }

    /// Shows `text` for a few seconds.
    pub fn say(&mut self, text: &str, t: f64) {
        self.result = None;
        self.toast = Some((text.to_string(), t + 3000.0));
        self.dirty = true;
    }

    /// Shows the outcome of an intent run from the action library: its dialog, or why it did not run.
    pub fn show_result(&mut self, text: &str, t: f64) {
        self.say(text, t);
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

    /// The sidebar row that is selected: the gallery, a folder, or All Shortcuts.
    fn selected_row(&self) -> u32 {
        match (self.view, self.filter) {
            (View::Gallery, _) => ROW_GALLERY,
            (_, Filter::Folder(f)) => ROW_FOLDER + f as u32,
            _ => ROW_ALL,
        }
    }

    fn sidebar(&self) -> SourceList {
        let in_folder = |f: usize| self.list.iter().filter(|s| s.folder == f).count() as u32;
        let mut items = vec![
            SidebarItem::header("Library"),
            SidebarItem::row(ROW_ALL, "All Shortcuts").with_symbol("square.grid.2x2").with_count(self.list.len() as u32),
            SidebarItem::row(ROW_GALLERY, "Gallery").with_symbol("star"),
            SidebarItem::header("Folders"),
        ];
        for (k, name) in FOLDERS.iter().enumerate() {
            items.push(SidebarItem::row(ROW_FOLDER + k as u32 + 1, name).with_symbol("folder").with_count(in_folder(k + 1)));
        }
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(Some(self.selected_row()));
        list
    }

    fn bar_bounds(&self, ui: &Ui) -> PtRect {
        let x = SIDEBAR_W + 12.0;
        let h = widgets::CAPSULE_PT;
        PtRect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.s - x - 16.0).max(0.0), h }
    }

    fn title(&self) -> String {
        match self.view {
            View::Grid => match self.filter {
                Filter::All => "All Shortcuts".into(),
                Filter::Folder(f) => model::folder_name(f).unwrap_or("Folder").to_string(),
            },
            View::Editor(i) => self.list.get(i).map(|s| s.name.clone()).unwrap_or_default(),
            View::Picker(_) => "Add Action".into(),
            View::Gallery => "Gallery".into(),
        }
    }

    /// The unified toolbar (`NativeToolbar`): back, title, then the view's commands.
    fn bar(&self, ui: &Ui) -> Toolbar {
        let search_room = if self.view == View::Grid { SEARCH_W + widgets::TOOLBAR_GAP_PT } else { 0.0 };
        let room = (self.bar_bounds(ui).w - 5.0 * widgets::CAPSULE_PT - 4.0 * widgets::TOOLBAR_GAP_PT - search_room).clamp(40.0, TITLE_MAX_W);
        let mut items = vec![];
        if self.view != View::Grid {
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
            View::Editor(_) => items.push(ToolbarItem::button(BAR_RUN, "Run")),
            View::Picker(_) | View::Gallery => {}
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
            _ => None,
        }
    }

    /// The toolbar buttons as controls (points), so the shell keeps them clickable in the drag band, and the search
    /// field as a text field.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar(ui);
        let placed = self.bar_layout(ui);
        let mut nodes: Vec<Node> = placed
            .visible
            .iter()
            .filter(|(id, _)| *id != BAR_TITLE && *id != BAR_SEARCH)
            .filter_map(|(id, r)| {
                let it = bar.items.iter().find(|i| i.id == *id)?;
                Some(Node { id: *id as u64, role: Role::Button, label: it.label.clone(), value: String::new(), bounds: *r, enabled: it.enabled })
            })
            .collect();
        if let Some(r) = placed.rect_of(BAR_SEARCH) {
            nodes.push(Node { id: BAR_SEARCH as u64, role: Role::TextField, label: "Search".into(), value: self.query.to_string(), bounds: r, enabled: true });
        }
        nodes
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = model::save(&self.path, &self.list) {
            eprintln!("springboard: shortcuts: save: {e}");
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row(&self, ui: &Ui, cells: &mut Vec<Cell>, y: &mut f32, text: &str, value: &str, tint: u32, pos: u8, target: Option<Target>) -> Rect {
        let (m, x0) = (ui.pt(SPACE_XL), Self::side(ui));
        let rect = (x0 + m, *y, self.w as f32 - x0 - 2.0 * m, ui.pt(44.0));
        cells.push(Cell {
            target,
            rect,
            look: Look::Row { text: text.into(), value: value.into(), tint, pos, chevron: false, centre: false, indent: 0.0, inset: VALUE_INSET },
        });
        *y += rect.3;
        rect
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
                self.row(ui, &mut cells, &mut y, "Open at Login", self.login_value(), LABEL.now(), ROW_POS_FIRST | ROW_POS_LAST, Some(Target::LoginItem));
                y += ui.pt(SPACE_L);
                let gap = ui.pt(SPACE_L);
                let cols = (((w - 2.0 * m0 + gap) / (ui.pt(TILE_MIN_W) + gap)).floor() as usize).max(2);
                let tw = (w - 2.0 * m0 - gap * (cols - 1) as f32) / cols as f32;
                let th = ui.pt(TILE_H);
                let shown = self.shown();
                // the Create tile follows the tiles, except while a search narrows them
                let total = shown.len() + usize::from(self.query.is_empty());
                for i in 0..total {
                    let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
                    let rect = (m + cx * (tw + gap), y + cy * (th + gap), tw, th);
                    if let Some(&idx) = shown.get(i) {
                        let s = &self.list[idx];
                        let look = Look::Tile { name: s.name.clone(), color: PALETTE[s.color % PALETTE.len()], count: s.actions.len(), glyph: s.glyph };
                        cells.push(Cell { target: Some(Target::Tile(idx)), rect, look });
                    } else {
                        cells.push(Cell { target: Some(Target::Create), rect, look: Look::CreateTile });
                    }
                }
                y += (total.div_ceil(cols)) as f32 * (th + gap);
                if total == 0 {
                    let note = (m + ui.pt(SPACE_L), y, w - 2.0 * ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
                    cells.push(Cell { target: None, rect: note, look: Look::Note(format!("No shortcuts match \"{}\".", &*self.query)) });
                    y += ui.pt(SPACE_XXXL);
                }
            }
            View::Editor(i) => {
                let Some(sc) = self.list.get(i) else { return (cells, y) };
                y += ui.pt(SPACE_L);
                self.row(ui, &mut cells, &mut y, "Name", &sc.name, LABEL.now(), ROW_POS_FIRST | ROW_POS_LAST, Some(Target::Name));
                y += ui.pt(35.0) - ui.pt(SPACE_S);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Actions".into()) });
                y += ui.pt(SPACE_XXXL);
                let n = sc.actions.len();
                let depths = flow::depths(&sc.actions);
                for (k, a) in sc.actions.iter().enumerate() {
                    let pos = (if k == 0 { ROW_POS_FIRST } else { 0 }) | if k + 1 == n { ROW_POS_LAST } else { 0 };
                    let target = a.text().map(|_| Target::Value(k));
                    let r = self.row(ui, &mut cells, &mut y, a.title(), &a.value(), LABEL.now(), pos, target);
                    if let Some(Cell { look: Look::Row { indent, inset, .. }, .. }) = cells.last_mut() {
                        *indent = INDENT_PT * depths[k] as f32;
                        *inset = STEP_VALUE_INSET;
                    }
                    // while its text is typed, the field covers the row and the buttons go
                    if self.edit == Edit::Value(k) {
                        continue;
                    }
                    let marker = is_marker(a);
                    if !marker {
                        cells.push(Cell { target: Some(Target::Remove(k)), rect: (r.0, r.1, ui.pt(44.0), r.3), look: Look::Minus });
                    }
                    if a.adjustable() {
                        let bw = ui.pt(44.0);
                        cells.push(Cell { target: Some(Target::Inc(k)), rect: (r.0 + r.2 - bw, r.1, bw, r.3), look: Look::Stepper(true) });
                        cells.push(Cell { target: Some(Target::Dec(k)), rect: (r.0 + r.2 - 2.0 * bw, r.1, bw, r.3), look: Look::Stepper(false) });
                    }
                    if !marker {
                        let (mw, bw) = (ui.pt(MOVE_W), ui.pt(44.0));
                        let base = r.0 + r.2 - 2.0 * bw - 2.0 * mw;
                        if k > 0 {
                            cells.push(Cell { target: Some(Target::Up(k)), rect: (base, r.1, mw, r.3), look: Look::Move(true) });
                        }
                        if k + 1 < n {
                            cells.push(Cell { target: Some(Target::Down(k)), rect: (base + mw, r.1, mw, r.3), look: Look::Move(false) });
                        }
                    }
                }
                if n < MAX_ACTIONS {
                    let pos = ROW_POS_LAST | if n == 0 { ROW_POS_FIRST } else { 0 };
                    self.row(ui, &mut cells, &mut y, "Add Action", "", SYSTEM_BLUE, pos, Some(Target::AddAction));
                }
                y += ui.pt(SPACE_L);
                let when = sc.schedule.map(|d| d.label()).unwrap_or_else(|| "Off".into());
                self.row(ui, &mut cells, &mut y, "Daily Reminder", &when, LABEL.now(), ROW_POS_FIRST, Some(Target::Schedule));
                let folder = model::folder_name(sc.folder).unwrap_or("None");
                self.row(ui, &mut cells, &mut y, "Folder", folder, LABEL.now(), ROW_POS_LAST, Some(Target::Folder));
                y += ui.pt(35.0) - ui.pt(SPACE_S);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Color".into()) });
                y += ui.pt(SPACE_XXXL);
                let sw = ui.pt(SPACE_XXXL);
                let step = (w - 2.0 * m0 - ui.pt(SPACE_XXL)) / PALETTE.len() as f32;
                let card = || Look::Row { text: String::new(), value: String::new(), tint: LABEL.now(), pos: ROW_POS_FIRST | ROW_POS_LAST, chevron: false, centre: false, indent: 0.0, inset: VALUE_INSET };
                cells.push(Cell { target: None, rect: (m, y, w - 2.0 * m0, ui.pt(56.0)), look: card() });
                for (c, color) in PALETTE.iter().enumerate() {
                    let cx = m + ui.pt(SPACE_M) + step * c as f32 + (step - sw) / 2.0;
                    cells.push(Cell { target: Some(Target::Color(c)), rect: (cx, y + ui.pt(SPACE_M), sw, sw), look: Look::Swatch { color: *color, on: sc.color % PALETTE.len() == c } });
                }
                y += ui.pt(56.0);
                y += ui.pt(35.0) - ui.pt(SPACE_S);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Glyph".into()) });
                y += ui.pt(SPACE_XXXL);
                cells.push(Cell { target: None, rect: (m, y, w - 2.0 * m0, ui.pt(56.0)), look: card() });
                let gstep = (w - 2.0 * m0 - ui.pt(SPACE_XXL)) / GLYPHS.len() as f32;
                let gw = sw.min(gstep);
                for g in 0..GLYPHS.len() {
                    let gx = m + ui.pt(SPACE_M) + gstep * g as f32 + (gstep - gw) / 2.0;
                    cells.push(Cell { target: Some(Target::Glyph(g)), rect: (gx, y + ui.pt(SPACE_M), gw, sw), look: Look::GlyphCell { glyph: g, on: sc.glyph == g } });
                }
                y += ui.pt(56.0) + ui.pt(35.0);
                self.row(ui, &mut cells, &mut y, "Duplicate Shortcut", "", SYSTEM_BLUE, ROW_POS_FIRST, Some(Target::Duplicate));
                self.row(ui, &mut cells, &mut y, "Export Shortcut...", "", SYSTEM_BLUE, ROW_POS_LAST, Some(Target::Export));
                y += ui.pt(35.0);
                let r = self.row(ui, &mut cells, &mut y, "Delete Shortcut", "", SYSTEM_RED, ROW_POS_FIRST | ROW_POS_LAST, Some(Target::Delete));
                if let Some(Cell { look: Look::Row { centre, .. }, .. }) = cells.iter_mut().rev().find(|c| c.rect == r) {
                    *centre = true;
                }
            }
            View::Picker(_) => {
                y += ui.pt(SPACE_L);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Built-in Actions".into()) });
                y += ui.pt(SPACE_XXXL);
                for (k, name) in KINDS.iter().enumerate() {
                    let pos = (if k == 0 { ROW_POS_FIRST } else { 0 }) | if k + 1 == KINDS.len() { ROW_POS_LAST } else { 0 };
                    let r = self.row(ui, &mut cells, &mut y, name, "", LABEL.now(), pos, Some(Target::Pick(k)));
                    if let Some(Cell { look: Look::Row { chevron, .. }, .. }) = cells.iter_mut().rev().find(|c| c.rect == r) {
                        *chevron = true;
                    }
                }
                // the intents this app declares, with the title and description from the catalog
                y += ui.pt(SPACE_L);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Intents".into()) });
                y += ui.pt(SPACE_XXXL);
                for (k, intent) in automation::declared().iter().enumerate() {
                    self.row(ui, &mut cells, &mut y, &intent.title, "", LABEL.now(), ROW_POS_FIRST | ROW_POS_LAST, Some(Target::Intent(k)));
                    let note = (m + ui.pt(SPACE_L), y, w - 2.0 * ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
                    cells.push(Cell { target: None, rect: note, look: Look::Note(intent.description.clone()) });
                    y += ui.pt(SPACE_XXXL);
                }
            }
            View::Gallery => {
                y += ui.pt(SPACE_L);
                cells.push(Cell { target: None, rect: (m + ui.pt(SPACE_L), y, w, ui.pt(SPACE_XXXL)), look: Look::Header("Starter Shortcuts".into()) });
                y += ui.pt(SPACE_XXXL);
                for (n, starter) in model::starters().iter().enumerate() {
                    self.row(ui, &mut cells, &mut y, &starter.shortcut.name, "Add", LABEL.now(), ROW_POS_FIRST | ROW_POS_LAST, Some(Target::Starter(n)));
                    let note = (m + ui.pt(SPACE_L), y, w - 2.0 * ui.pt(SPACE_L), ui.pt(SPACE_XXXL));
                    cells.push(Cell { target: None, rect: note, look: Look::Note(starter.summary.to_string()) });
                    y += ui.pt(SPACE_XXXL);
                }
                y += ui.pt(SPACE_L);
                self.row(ui, &mut cells, &mut y, "Import from File...", "", SYSTEM_BLUE, ROW_POS_FIRST | ROW_POS_LAST, Some(Target::Import));
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

    fn max_scroll(&self, ui: &Ui) -> f32 {
        let (_, bottom) = self.layout(ui);
        (bottom + ui.pt(SPACE_XXL) - self.h as f32).max(0.0)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if y < Self::top(ui) {
            let placed = self.bar_layout(ui);
            let id = placed.hit(x / ui.s, y / ui.s).filter(|id| self.bar(ui).items.iter().any(|i| i.id == *id && i.enabled))?;
            if id == BAR_SEARCH {
                let clear = placed.rect_of(id).is_some_and(|r| search::hits_clear(r, !self.query.is_empty(), x / ui.s, y / ui.s));
                return Some(if clear { Target::SearchClear } else { Target::Search });
            }
            return Self::bar_target(id);
        }
        if x < Self::side(ui) {
            let bounds = PtRect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.s };
            return self.sidebar().hit(bounds, -(UNIFIED_BAR_PT + 4.0), x / ui.s, y / ui.s).map(Target::Side);
        }
        // the result panel floats over the content: its buttons win, and the rest of it takes no tap
        if let Some(p) = &self.result {
            let (bx, by, bw, bh) = self.result_box(ui, p);
            if x >= bx && x < bx + bw && y >= by && y < by + bh {
                let (follow, done) = self.result_buttons(ui, p);
                let inside = |r: Rect| x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3;
                if follow.is_some_and(|r| inside(r)) {
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
            .find(|c| {
                let (cx, cy, cw, ch) = c.rect;
                x >= cx && x < cx + cw && y >= cy && y < cy + ch
            })
            .and_then(|c| c.target)
    }

    /// Asks for a run of shortcut `i`: the window cannot perform steps itself, so the app does, and shows the outcome.
    fn run_all(&mut self, i: usize) {
        let Some(s) = self.list.get(i) else { return };
        if s.actions.is_empty() {
            self.toast = Some((format!("{} has no actions", s.name), self.now + 2000.0));
            return;
        }
        self.run_requests.push(s.clone());
    }

    fn editing(&mut self) -> Option<&mut Shortcut> {
        match self.view {
            View::Editor(i) | View::Picker(i) => self.list.get_mut(i),
            View::Grid | View::Gallery => None,
        }
    }

    // ---- typing -------------------------------------------------------------------------------------------------

    /// Whether the app needs the keyboard: a field is being typed into.
    pub fn wants_keyboard(&self) -> bool {
        self.edit != Edit::None
    }

    fn field_mut(&mut self) -> &mut TextField {
        if self.edit == Edit::Search {
            &mut self.query
        } else {
            &mut self.field
        }
    }

    /// Starts typing into `edit`; the name and a step's text start with what they are now.
    fn begin(&mut self, edit: Edit) {
        let current = match (edit, self.view) {
            (Edit::Name, View::Editor(i)) => self.list.get(i).map(|s| s.name.clone()),
            (Edit::Value(k), View::Editor(i)) => self.list.get(i).and_then(|s| s.actions.get(k)).and_then(|a| a.text()).map(str::to_string),
            _ => None,
        };
        self.field.clear();
        if let Some(text) = current {
            self.field.push_str(&text);
        }
        self.edit = edit;
        self.dirty = true;
    }

    /// Leaves the field, keeping what was typed: a name or a step's text is saved when it is valid and otherwise the
    /// reason is shown; the search keeps its text and just loses the focus.
    fn commit_edit(&mut self) {
        let text = self.field.to_string();
        match (self.edit, self.view) {
            (Edit::Name, View::Editor(i)) => match model::rename(&mut self.list, i, &text) {
                Ok(()) => self.save(),
                Err(why) => self.toast = Some((why, self.now + 3000.0)),
            },
            (Edit::Value(k), View::Editor(_)) => {
                let ok = self.editing().and_then(|s| s.actions.get_mut(k)).is_some_and(|a| a.set_text(&text));
                if ok {
                    self.save();
                } else {
                    self.toast = Some(("That text cannot be used here.".into(), self.now + 3000.0));
                }
            }
            _ => {}
        }
        self.edit = Edit::None;
        self.field.clear();
        self.dirty = true;
    }

    /// Leaves the field, dropping what was typed; the search is emptied.
    fn cancel_edit(&mut self) {
        if self.edit == Edit::Search {
            self.query.clear();
        }
        self.edit = Edit::None;
        self.field.clear();
        self.dirty = true;
    }

    /// A key press. While a field is open Return keeps what was typed, Escape drops it, Backspace deletes and text is
    /// typed. True when the window needs a new frame.
    pub fn key(&mut self, key: KeyInput) -> bool {
        if self.edit == Edit::None {
            return false;
        }
        match key {
            KeyInput::Code(13) => self.commit_edit(),
            KeyInput::Code(27) => self.cancel_edit(),
            KeyInput::Code(8) => self.field_mut().pop(),
            KeyInput::Text(text) => self.field_mut().push_str(&text),
            KeyInput::Code(_) => return false,
        }
        self.dirty = true;
        true
    }

    /// Before a tap on `next` does its work: leaves the field unless the tap is on that same field.
    fn leave_edit_for(&mut self, next: Target) {
        let keeps = match (self.edit, next) {
            (Edit::Search, Target::Search | Target::SearchClear) => true,
            (Edit::Name, Target::Name) => true,
            (Edit::Value(a), Target::Value(b)) => a == b,
            _ => false,
        };
        if self.edit != Edit::None && !keeps {
            self.commit_edit();
        }
    }

    // ---- commands -----------------------------------------------------------------------------------------------

    /// Whether the editor of a shortcut is open, which the menus that act on one need.
    pub fn in_editor(&self) -> bool {
        matches!(self.view, View::Editor(_))
    }

    /// Does what a menu item asks.
    pub fn command(&mut self, c: Command) {
        match c {
            Command::New => self.activate(Target::Create),
            Command::Duplicate if self.in_editor() => self.activate(Target::Duplicate),
            Command::Delete if self.in_editor() => self.activate(Target::Delete),
            Command::Rename if self.in_editor() => {
                self.commit_edit();
                self.begin(Edit::Name);
            }
            Command::Find => {
                if self.view != View::Grid {
                    self.activate(Target::Side(ROW_ALL));
                }
                self.edit = Edit::Search;
                self.dirty = true;
            }
            Command::Gallery => self.activate(Target::Side(ROW_GALLERY)),
            Command::RunOpen if self.in_editor() => self.activate(Target::Run),
            Command::Run(i) => self.run_all(i),
            Command::Export => self.queue_export(),
            Command::Import => self.import_request = true,
            Command::Duplicate | Command::Delete | Command::Rename | Command::RunOpen => {}
        }
        self.dirty = true;
    }

    /// Queues an export: the open shortcut, or the whole list when none is open.
    fn queue_export(&mut self) {
        let (name, text) = match self.view {
            View::Editor(i) | View::Picker(i) if i < self.list.len() => {
                let s = &self.list[i];
                (format!("{} Shortcut.txt", model::clean_file(&s.name)), model::export(std::slice::from_ref(s)))
            }
            _ => ("Shortcuts.txt".to_string(), model::export(&self.list)),
        };
        self.export_request = Some((name, text));
    }

    fn activate(&mut self, t: Target) {
        match t {
            Target::Tile(i) if self.edit_mode => {
                self.view = View::Editor(i);
                self.scroll = 0.0;
            }
            Target::Tile(i) => self.run_all(i),
            Target::Create => {
                if self.list.len() < MAX_SHORTCUTS {
                    let name = (1..).map(|n| format!("Shortcut {n}")).find(|n| self.list.iter().all(|s| &s.name != n)).unwrap_or_default();
                    let color = self.list.len() % PALETTE.len();
                    let mut shortcut = Shortcut::new(&name, color, Vec::new());
                    if let Filter::Folder(f) = self.filter {
                        shortcut.folder = f;
                    }
                    self.list.push(shortcut);
                    self.view = View::Editor(self.list.len() - 1);
                    self.scroll = 0.0;
                    self.save();
                }
            }
            Target::Side(id) => {
                self.view = View::Grid;
                match id {
                    ROW_GALLERY => self.view = View::Gallery,
                    ROW_ALL => self.filter = Filter::All,
                    f if f > ROW_FOLDER && f <= ROW_FOLDER + FOLDERS.len() as u32 => self.filter = Filter::Folder((f - ROW_FOLDER) as usize),
                    _ => {}
                }
                self.scroll = 0.0;
            }
            Target::EditToggle => self.edit_mode = !self.edit_mode,
            Target::Back => {
                self.view = match self.view {
                    View::Picker(i) => View::Editor(i),
                    _ => View::Grid,
                };
                self.scroll = 0.0;
            }
            Target::Run => {
                if let View::Editor(i) = self.view {
                    self.run_all(i);
                }
            }
            Target::Remove(k) => {
                let removed = self.editing().map(|s| flow::remove_step(&mut s.actions, k)).unwrap_or(0);
                if removed > 0 {
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
                let up = matches!(t, Target::Up(_));
                let moved = self.editing().and_then(|s| model::move_action(&mut s.actions, k, up)).is_some();
                if moved {
                    self.save();
                }
            }
            Target::Value(k) => {
                if self.edit != Edit::Value(k) {
                    self.begin(Edit::Value(k));
                }
            }
            Target::Name => {
                if self.edit != Edit::Name {
                    self.begin(Edit::Name);
                }
            }
            Target::Search => {
                self.edit = Edit::Search;
            }
            Target::SearchClear => {
                self.query.clear();
                self.scroll = 0.0;
            }
            Target::AddAction => {
                if let View::Editor(i) = self.view {
                    self.view = View::Picker(i);
                    self.scroll = 0.0;
                }
            }
            Target::Pick(k) => {
                let added = Action::block(k);
                let full = match self.editing() {
                    Some(s) if s.actions.len() + added.len() > MAX_ACTIONS => true,
                    Some(s) => {
                        s.actions.extend(added);
                        false
                    }
                    None => false,
                };
                if full {
                    self.toast = Some((format!("A shortcut holds at most {MAX_ACTIONS} steps."), self.now + 3000.0));
                } else {
                    if let View::Picker(i) = self.view {
                        self.view = View::Editor(i);
                    }
                    self.save();
                }
            }
            Target::Color(c) => {
                if let Some(s) = self.editing() {
                    s.color = c;
                    self.save();
                }
            }
            Target::Glyph(g) => {
                if let Some(s) = self.editing() {
                    s.glyph = g.min(GLYPHS.len() - 1);
                    self.save();
                }
            }
            Target::Folder => {
                if let Some(s) = self.editing() {
                    s.folder = (s.folder + 1) % (FOLDERS.len() + 1);
                    self.save();
                }
            }
            Target::Duplicate => {
                if let View::Editor(i) = self.view {
                    match model::duplicate(&mut self.list, i) {
                        Some(copy) => {
                            self.view = View::Editor(copy);
                            self.scroll = 0.0;
                            self.save();
                            self.toast = Some(("Duplicated".into(), self.now + 2000.0));
                        }
                        None => self.toast = Some((format!("A list holds at most {MAX_SHORTCUTS} shortcuts."), self.now + 3000.0)),
                    }
                }
            }
            Target::Export => self.queue_export(),
            Target::Import => self.import_request = true,
            Target::Starter(n) => match model::add_starter(&mut self.list, n) {
                Some(added) => {
                    self.save();
                    self.toast = Some((format!("Added {}", self.list[added].name), self.now + 2000.0));
                }
                None => self.toast = Some((format!("A list holds at most {MAX_SHORTCUTS} shortcuts."), self.now + 3000.0)),
            },
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
            Target::LoginItem => self.login_request = Some(!self.login_on()),
            Target::Delete => {
                if let View::Editor(i) = self.view {
                    if i < self.list.len() {
                        self.list.remove(i);
                    }
                    self.view = View::Grid;
                    self.scroll = 0.0;
                    self.save();
                }
            }
        }
        self.dirty = true;
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) -> bool {
        self.now = t;
        match event {
            Event::Down(x, y) => {
                self.pressed = self.hit(ui, x, y);
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
                        self.leave_edit_for(p);
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
                Look::Tile { name, color, count, glyph } => {
                    s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), *color);
                    if pressed {
                        s.fill_rrect(x, y, cw, ch, ui.pt(CARD_RADIUS_PT), PRESSED);
                    }
                    draw_mark(s, ui, *glyph, x + ui.pt(SPACE_L), y + ui.pt(SPACE_L), ui.pt(SPACE_XXL), WHITE);
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
                Look::Row { text, value, tint, pos, chevron, centre, indent, inset } => {
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
                    let typing = match c.target {
                        Some(Target::Name) => self.edit == Edit::Name,
                        Some(Target::Value(k)) => self.edit == Edit::Value(k),
                        _ => false,
                    };
                    if typing {
                        let placeholder = if self.edit == Edit::Name { "Name" } else { "Text" };
                        let pad = ui.pt(SPACE_S);
                        let bounds = PtRect { x: (x + ui.pt(46.0)) / ui.s, y: (y + pad) / ui.s, w: ((cw - ui.pt(46.0) - pad) / ui.s).max(0.0), h: ((ch - 2.0 * pad) / ui.s).max(0.0) };
                        widgets::search_field(s, ui, &self.field, bounds, placeholder, true);
                    } else if *centre {
                        self.put(s, ui, text, MAC_BODY_PT, Weight::Regular, *tint, x + cw / 2.0, y, ch, 1);
                    } else {
                        let leading = *chevron
                            || text == "Add Action"
                            || matches!(
                                c.target,
                                Some(Target::Pick(_) | Target::Intent(_) | Target::Starter(_) | Target::Duplicate | Target::Export | Target::Import)
                            );
                        let tx = if leading { x + ui.pt(SPACE_L) } else { x + ui.pt(46.0 + *indent) };
                        self.put(s, ui, text, MAC_BODY_PT, Weight::Regular, *tint, tx, y, ch, 0);
                        if !value.is_empty() {
                            let right = x + cw - ui.pt(*inset);
                            let room = (right - tx - ui.measure(text, MAC_BODY_PT, Weight::Regular) - ui.pt(SPACE_L)).max(0.0);
                            let fitted = ui.fit(value, MAC_BODY_PT, room);
                            self.put(s, ui, &fitted, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), right, y, ch, 2);
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
                Look::Move(up) => {
                    let col = if pressed { LABEL_2.now() } else { LINK };
                    let (cx, cy, a, t) = (x + cw / 2.0, y + ch / 2.0, ui.pt(5.0), ui.pt(SPACE_XXS));
                    // a chevron pointing up or down, from two strokes
                    let (tip, base) = if *up { (cy - a / 2.0, cy + a / 2.0) } else { (cy + a / 2.0, cy - a / 2.0) };
                    glyphs::line(s, (cx - a, base), (cx, tip), t, col);
                    glyphs::line(s, (cx, tip), (cx + a, base), t, col);
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
                Look::GlyphCell { glyph, on } => {
                    if *on {
                        s.fill_rrect(x - 2.0, y - 2.0, cw + 4.0, ch + 4.0, ui.pt(CELL_RADIUS_PT), ROW_PRESSED.now());
                    }
                    let side = cw.min(ch) * 0.7;
                    draw_mark(s, ui, *glyph, x + (cw - side) / 2.0, y + (ch - side) / 2.0, side * 0.75, LABEL.now());
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
            widgets::search_field(s, ui, &self.query, r, "Search", self.edit == Edit::Search);
        }
        widgets::source_list(s, ui, PtRect { x: 0.0, y: 0.0, w: SIDEBAR_W, h: self.h as f32 / ui.s }, &self.sidebar(), -(UNIFIED_BAR_PT + 4.0), None);
        s.fill_rect(x0 as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        if let Some(p) = self.result.clone() {
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

/// Draws a shortcut's glyph in a box of side `g` pixels at (`x`, `y`): the plain Shortcuts mark (two overlapping
/// rounded squares) for glyph 0, otherwise the symbol with that number in `GLYPHS`.
fn draw_mark(s: &mut Surface, ui: &Ui, glyph: usize, x: f32, y: f32, g: f32, color: u32) {
    match GLYPHS.get(glyph) {
        Some(name) if !name.is_empty() => {
            let side = g * 1.4;
            let at = PtRect { x: x / ui.s, y: y / ui.s, w: side / ui.s, h: side / ui.s };
            glyphs::glyph(s, ui, name, at, color);
        }
        _ => {
            s.stroke_rrect(x, y, g, g, g * 0.28, ui.pt(SPACE_XXS), color);
            s.stroke_rrect(x + g * 0.4, y + g * 0.4, g, g, g * 0.28, ui.pt(SPACE_XXS), color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::Font;
    use springboard::shortcuts::Cond;

    fn ui() -> Ui {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    /// A window over the default list, saving into a directory of its own.
    fn window(tag: &str) -> Shortcuts {
        let dir = std::env::temp_dir().join(format!("shortcuts-widget-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Shortcuts::new(&ui(), 1200, 780, dir.join("shortcuts.tsv"))
    }

    fn names(w: &Shortcuts) -> Vec<String> {
        w.list().iter().map(|s| s.name.clone()).collect()
    }

    fn type_text(w: &mut Shortcuts, text: &str) {
        w.key(KeyInput::Text(text.to_string()));
    }

    #[test]
    fn every_glyph_name_has_a_drawing() {
        for name in &GLYPHS[1..] {
            assert!(glyphs::has(name), "{name}");
        }
    }

    #[test]
    fn a_name_is_typed_and_kept_with_return() {
        let mut w = window("rename");
        w.edit_mode = true;
        w.activate(Target::Tile(0));
        assert_eq!(w.view, View::Editor(0));
        w.activate(Target::Name);
        assert!(w.wants_keyboard());
        assert_eq!(&*w.field, "Bedtime", "the field starts with the name");
        for _ in 0.."Bedtime".len() {
            w.key(KeyInput::Code(8));
        }
        type_text(&mut w, "Wind Down");
        assert!(w.key(KeyInput::Code(13)));
        assert!(!w.wants_keyboard());
        assert_eq!(w.list()[0].name, "Wind Down");
        assert!(std::fs::read_to_string(&w.path).unwrap().starts_with("Wind Down\t"), "saved");
    }

    #[test]
    fn escape_drops_what_was_typed() {
        let mut w = window("escape");
        w.view = View::Editor(1);
        w.activate(Target::Name);
        type_text(&mut w, " and more");
        w.key(KeyInput::Code(27));
        assert_eq!(w.list()[1].name, "Save Battery");
        assert!(!w.wants_keyboard());
    }

    #[test]
    fn a_taken_name_is_refused_with_a_reason() {
        let mut w = window("taken");
        w.view = View::Editor(0);
        w.activate(Target::Name);
        for _ in 0.."Bedtime".len() {
            w.key(KeyInput::Code(8));
        }
        type_text(&mut w, "capture");
        w.key(KeyInput::Code(13));
        assert_eq!(w.list()[0].name, "Bedtime");
        assert!(w.toast.as_ref().unwrap().0.contains("already exists"));
    }

    #[test]
    fn a_step_text_is_edited_in_place() {
        let mut w = window("value");
        w.list[0].actions = vec![Action::Text("old".into())];
        w.view = View::Editor(0);
        w.activate(Target::Value(0));
        assert_eq!(&*w.field, "old");
        for _ in 0..3 {
            w.key(KeyInput::Code(8));
        }
        type_text(&mut w, "Hello, {Name}");
        w.key(KeyInput::Code(13));
        assert_eq!(w.list()[0].actions[0], Action::Text("Hello, {Name}".into()));
    }

    #[test]
    fn an_empty_step_text_is_refused_and_the_old_one_stays() {
        let mut w = window("empty-value");
        w.list[0].actions = vec![Action::Text("keep".into())];
        w.view = View::Editor(0);
        w.activate(Target::Value(0));
        for _ in 0..4 {
            w.key(KeyInput::Code(8));
        }
        w.key(KeyInput::Code(13));
        assert_eq!(w.list()[0].actions[0], Action::Text("keep".into()));
        assert!(w.toast.is_some());
    }

    #[test]
    fn tapping_elsewhere_keeps_what_was_typed() {
        let mut w = window("elsewhere");
        w.view = View::Editor(2);
        w.activate(Target::Name);
        w.key(KeyInput::Code(8));
        w.leave_edit_for(Target::AddAction);
        assert_eq!(w.list()[2].name, "Quick Time", "the name is kept as it stood");
        assert!(!w.wants_keyboard());
        w.activate(Target::Name);
        w.leave_edit_for(Target::Name);
        assert!(w.wants_keyboard(), "a tap on the same field keeps it open");
    }

    #[test]
    fn keys_do_nothing_without_a_field() {
        let mut w = window("nokeys");
        assert!(!w.key(KeyInput::Text("x".into())));
        assert!(!w.key(KeyInput::Code(13)));
    }

    #[test]
    fn a_search_narrows_the_grid_and_escape_clears_it() {
        let mut w = window("search");
        assert_eq!(w.shown(), vec![0, 1, 2, 3]);
        w.activate(Target::Search);
        assert!(w.wants_keyboard());
        type_text(&mut w, "BED");
        assert_eq!(w.shown(), vec![0]);
        assert_eq!(w.on_screen(), vec![(0, false)]);
        w.key(KeyInput::Code(13));
        assert!(!w.wants_keyboard());
        assert_eq!(w.shown(), vec![0], "return keeps the search");
        w.activate(Target::SearchClear);
        assert_eq!(w.shown().len(), 4);
        w.activate(Target::Search);
        type_text(&mut w, "zzz");
        assert!(w.shown().is_empty());
        w.key(KeyInput::Code(27));
        assert_eq!(w.shown().len(), 4, "escape empties the search");
    }

    #[test]
    fn a_search_that_finds_nothing_says_so_and_hides_the_create_tile() {
        let mut w = window("nothing");
        w.activate(Target::Search);
        type_text(&mut w, "zzz");
        let (cells, _) = w.layout(&ui());
        assert!(cells.iter().any(|c| matches!(&c.look, Look::Note(t) if t.contains("zzz"))));
        assert!(!cells.iter().any(|c| matches!(c.look, Look::CreateTile)));
    }

    #[test]
    fn the_sidebar_lists_the_gallery_and_the_folders() {
        let mut w = window("sidebar");
        w.list[1].folder = 3;
        let list = w.sidebar();
        let rows: Vec<String> = list
            .items
            .iter()
            .filter_map(|i| match i {
                SidebarItem::Row { label, count, .. } => Some(format!("{label}:{}", count.unwrap_or(0))),
                _ => None,
            })
            .collect();
        assert_eq!(rows, vec!["All Shortcuts:4", "Gallery:0", "Favorites:0", "Home:0", "Work:1"]);
        assert_eq!(list.selected(), Some(ROW_ALL));
    }

    #[test]
    fn a_folder_row_filters_the_grid_and_new_shortcuts_land_in_it() {
        let mut w = window("folder");
        w.list[1].folder = 3;
        w.list[3].folder = 3;
        w.activate(Target::Side(ROW_FOLDER + 3));
        assert_eq!(w.filter, Filter::Folder(3));
        assert_eq!(w.shown(), vec![1, 3]);
        assert_eq!(w.title(), "Work");
        assert_eq!(w.selected_row(), ROW_FOLDER + 3);
        w.activate(Target::Create);
        assert_eq!(w.list().last().unwrap().folder, 3);
        w.activate(Target::Side(ROW_ALL));
        assert_eq!(w.filter, Filter::All);
        assert_eq!(w.view, View::Grid);
        assert_eq!(w.shown().len(), 5);
    }

    #[test]
    fn the_folder_row_steps_through_the_folders_and_back_to_none() {
        let mut w = window("folder-row");
        w.view = View::Editor(0);
        let seen: Vec<usize> = (0..=FOLDERS.len())
            .map(|_| {
                w.activate(Target::Folder);
                w.list()[0].folder
            })
            .collect();
        assert_eq!(seen, vec![1, 2, 3, 0]);
    }

    #[test]
    fn the_sidebar_is_hit_by_row() {
        let w = window("side-hit");
        let ui = ui();
        // rows start under the toolbar band: header (30), All Shortcuts (32), Gallery (32)
        let y_all = ui.pt(UNIFIED_BAR_PT + 4.0 + 30.0 + 10.0);
        let y_gallery = ui.pt(UNIFIED_BAR_PT + 4.0 + 30.0 + 32.0 + 10.0);
        assert_eq!(w.hit(&ui, 40.0, y_all), Some(Target::Side(ROW_ALL)));
        assert_eq!(w.hit(&ui, 40.0, y_gallery), Some(Target::Side(ROW_GALLERY)));
    }

    #[test]
    fn the_gallery_adds_a_starter_under_a_free_name() {
        let mut w = window("gallery");
        w.activate(Target::Side(ROW_GALLERY));
        assert_eq!(w.view, View::Gallery);
        assert_eq!(w.title(), "Gallery");
        assert!(w.on_screen().is_empty());
        w.activate(Target::Starter(1));
        w.activate(Target::Starter(1));
        let list = names(&w);
        assert_eq!(&list[4..], ["Say Thanks", "Say Thanks 2"]);
        assert!(w.toast.as_ref().unwrap().0.starts_with("Added "));
        w.activate(Target::Back);
        assert_eq!(w.view, View::Grid);
    }

    #[test]
    fn the_gallery_lists_every_starter_and_an_import_row() {
        let mut w = window("gallery-layout");
        w.view = View::Gallery;
        let (cells, _) = w.layout(&ui());
        let starters = cells.iter().filter(|c| matches!(c.target, Some(Target::Starter(_)))).count();
        assert_eq!(starters, model::starters().len());
        assert!(cells.iter().any(|c| c.target == Some(Target::Import)));
        w.activate(Target::Import);
        assert!(w.take_import_request());
        assert!(!w.take_import_request(), "once");
    }

    #[test]
    fn a_shortcut_is_duplicated_and_the_editor_follows_the_copy() {
        let mut w = window("duplicate");
        w.view = View::Editor(1);
        w.activate(Target::Duplicate);
        assert_eq!(w.view, View::Editor(2));
        assert_eq!(names(&w)[1..4], ["Save Battery", "Save Battery Copy", "Quick Timer"]);
        assert_eq!(w.list()[2].actions, w.list()[1].actions);
    }

    #[test]
    fn steps_move_and_the_edge_buttons_are_not_offered() {
        let mut w = window("move");
        w.view = View::Editor(0);
        let before = w.list()[0].actions.clone();
        w.activate(Target::Down(0));
        assert_eq!(w.list()[0].actions[0], before[1]);
        assert_eq!(w.list()[0].actions[1], before[0]);
        w.activate(Target::Up(1));
        assert_eq!(w.list()[0].actions, before);
        let (cells, _) = w.layout(&ui());
        assert!(!cells.iter().any(|c| c.target == Some(Target::Up(0))), "the first step has no Up");
        assert!(!cells.iter().any(|c| c.target == Some(Target::Down(before.len() - 1))), "the last step has no Down");
        assert!(cells.iter().any(|c| c.target == Some(Target::Down(0))));
    }

    #[test]
    fn a_picked_block_adds_all_its_steps_and_removing_the_opener_takes_them_all() {
        let mut w = window("block");
        w.list[2].actions.clear();
        w.view = View::Picker(2);
        w.activate(Target::Pick(17));
        assert_eq!(w.view, View::Editor(2));
        assert_eq!(w.list()[2].actions, vec![Action::If(Cond::NotEmpty), Action::Otherwise, Action::EndIf]);
        w.activate(Target::Remove(0));
        assert!(w.list()[2].actions.is_empty());
    }

    #[test]
    fn a_block_that_does_not_fit_is_refused() {
        let mut w = window("full");
        w.list[0].actions = vec![Action::Screenshot; MAX_ACTIONS - 1];
        w.view = View::Picker(0);
        w.activate(Target::Pick(18));
        assert_eq!(w.list()[0].actions.len(), MAX_ACTIONS - 1);
        assert_eq!(w.view, View::Picker(0), "the list stays open");
        assert!(w.toast.as_ref().unwrap().0.contains("at most"));
        w.activate(Target::Pick(0));
        assert_eq!(w.list()[0].actions.len(), MAX_ACTIONS);
    }

    #[test]
    fn marker_rows_have_no_remove_or_move_buttons_and_blocks_indent() {
        let mut w = window("markers");
        w.list[0].actions = vec![Action::Repeat(2), Action::Dnd(true), Action::EndRepeat];
        w.view = View::Editor(0);
        let (cells, _) = w.layout(&ui());
        let has = |t: Target| cells.iter().any(|c| c.target == Some(t));
        assert!(has(Target::Remove(0)) && has(Target::Remove(1)));
        assert!(!has(Target::Remove(2)) && !has(Target::Up(2)) && !has(Target::Down(2)));
        let indents: Vec<f32> = cells
            .iter()
            .filter_map(|c| match &c.look {
                Look::Row { text, indent, .. } if matches!(text.as_str(), "Repeat" | "Set Do Not Disturb" | "End Repeat") => Some(*indent),
                _ => None,
            })
            .collect();
        assert_eq!(indents, vec![0.0, INDENT_PT, 0.0]);
    }

    #[test]
    fn only_steps_with_text_open_the_field() {
        let mut w = window("value-rows");
        w.list[0].actions = vec![Action::Text("a".into()), Action::Dnd(true), Action::If(Cond::Contains("x".into())), Action::If(Cond::Empty), Action::EndIf, Action::EndIf];
        w.view = View::Editor(0);
        let (cells, _) = w.layout(&ui());
        let value_rows: Vec<usize> = cells.iter().filter_map(|c| match c.target { Some(Target::Value(k)) => Some(k), _ => None }).collect();
        assert_eq!(value_rows, vec![0, 2]);
    }

    #[test]
    fn typing_in_a_step_hides_its_buttons() {
        let mut w = window("typing-row");
        w.list[0].actions = vec![Action::Text("a".into()), Action::Dnd(true)];
        w.view = View::Editor(0);
        w.activate(Target::Value(0));
        let (cells, _) = w.layout(&ui());
        assert!(!cells.iter().any(|c| c.target == Some(Target::Remove(0))));
        assert!(cells.iter().any(|c| c.target == Some(Target::Remove(1))));
    }

    #[test]
    fn the_glyph_is_chosen_from_the_row() {
        let mut w = window("glyph");
        w.view = View::Editor(0);
        w.activate(Target::Glyph(4));
        assert_eq!(w.list()[0].glyph, 4);
        w.activate(Target::Glyph(99));
        assert_eq!(w.list()[0].glyph, GLYPHS.len() - 1, "a number past the list is the last glyph");
        let (cells, _) = w.layout(&ui());
        let on: Vec<usize> = cells.iter().filter_map(|c| match c.look { Look::GlyphCell { glyph, on: true } => Some(glyph), _ => None }).collect();
        assert_eq!(on, vec![GLYPHS.len() - 1]);
        assert_eq!(cells.iter().filter(|c| matches!(c.look, Look::GlyphCell { .. })).count(), GLYPHS.len());
    }

    #[test]
    fn a_tile_carries_its_glyph() {
        let mut w = window("tile-glyph");
        w.list[2].glyph = 5;
        let (cells, _) = w.layout(&ui());
        let glyph = cells.iter().find_map(|c| match (&c.look, c.target) {
            (Look::Tile { glyph, .. }, Some(Target::Tile(2))) => Some(*glyph),
            _ => None,
        });
        assert_eq!(glyph, Some(5));
    }

    #[test]
    fn the_editor_offers_every_row_the_matrix_asks_for() {
        let mut w = window("editor-rows");
        w.view = View::Editor(0);
        let (cells, _) = w.layout(&ui());
        for t in [Target::Name, Target::Duplicate, Target::Export, Target::Folder, Target::Schedule, Target::Delete, Target::AddAction] {
            assert!(cells.iter().any(|c| c.target == Some(t)), "{t:?}");
        }
    }

    #[test]
    fn an_export_is_the_open_shortcut_or_the_whole_list() {
        let mut w = window("export");
        w.command(Command::Export);
        let (name, text) = w.take_export_request().unwrap();
        assert_eq!(name, "Shortcuts.txt");
        assert_eq!(model::parse(&text).len(), 4);
        w.view = View::Editor(1);
        w.activate(Target::Export);
        let (name, text) = w.take_export_request().unwrap();
        assert_eq!(name, "Save Battery Shortcut.txt");
        assert_eq!(model::parse(&text).len(), 1);
        assert!(w.take_export_request().is_none(), "once");
    }

    #[test]
    fn an_import_adds_shortcuts_and_reports_how_it_went() {
        let mut w = window("import");
        let text = model::export(&model::defaults()[..2]);
        w.import_text(&text, 0.0);
        assert_eq!(w.list().len(), 6);
        assert_eq!(w.toast.as_ref().unwrap().0, "Imported 2 shortcuts");
        assert_eq!(names(&w)[4], "Bedtime 2");
        w.import_text("not a file", 0.0);
        assert_eq!(w.list().len(), 6);
        assert!(w.toast.as_ref().unwrap().0.contains("not a Shortcuts file"));
    }

    #[test]
    fn menu_commands_reach_the_same_actions() {
        let mut w = window("menu");
        w.command(Command::New);
        assert_eq!(w.view, View::Editor(4));
        w.command(Command::Duplicate);
        assert_eq!(w.list().len(), 6);
        w.command(Command::Rename);
        assert!(w.wants_keyboard());
        w.key(KeyInput::Code(27));
        w.command(Command::Delete);
        assert_eq!(w.view, View::Grid);
        assert_eq!(w.list().len(), 5);
        w.command(Command::Delete);
        assert_eq!(w.list().len(), 5, "Delete needs an open shortcut");
        w.command(Command::Find);
        assert!(w.wants_keyboard());
        w.command(Command::Gallery);
        assert_eq!(w.view, View::Gallery);
        w.command(Command::Find);
        assert_eq!(w.view, View::Grid, "Find returns to the grid to search it");
        assert!(w.wants_keyboard());
        w.command(Command::Import);
        assert!(w.take_import_request());
    }

    #[test]
    fn a_run_asks_the_app_to_perform_the_steps_and_an_empty_one_says_so() {
        let mut w = window("run");
        w.command(Command::Run(0));
        let asked = w.take_run_requests();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].name, "Bedtime");
        w.list[1].actions.clear();
        w.command(Command::Run(1));
        assert!(w.take_run_requests().is_empty());
        assert!(w.toast.as_ref().unwrap().0.contains("has no actions"));
        w.command(Command::Run(99));
        assert!(w.take_run_requests().is_empty());
    }

    #[test]
    fn a_run_panel_lists_each_step_and_where_it_stopped() {
        let mut w = window("run-panel");
        let steps = vec![
            Step { line: "Text: Hi".into(), outcome: Ok("Hi".into()) },
            Step { line: "Take Screenshot".into(), outcome: Ok(String::new()) },
            Step { line: "Set Wi-Fi: On".into(), outcome: Err("needs the shell".into()) },
        ];
        w.show_run("Mix", &steps, 100.0);
        let p = w.result.clone().unwrap();
        assert_eq!(p.title, "Mix");
        assert_eq!(p.dialog.as_deref(), Some("Stopped at step 3"));
        assert_eq!(
            p.rows,
            vec![
                ("1".to_string(), "Text: Hi - Hi".to_string()),
                ("2".to_string(), "Take Screenshot".to_string()),
                ("3".to_string(), "Set Wi-Fi: On - needs the shell".to_string()),
            ]
        );
        w.show_run("Fine", &steps[..1], 100.0);
        assert_eq!(w.result.as_ref().unwrap().dialog.as_deref(), Some("Finished"));
    }

    #[test]
    fn the_toolbar_shows_the_search_field_only_in_the_grid() {
        let mut w = window("bar");
        let ui = ui();
        assert!(w.bar_layout(&ui).rect_of(BAR_SEARCH).is_some());
        assert!(w.controls(&ui).iter().any(|n| n.role == Role::TextField && n.label == "Search"));
        w.view = View::Gallery;
        assert!(w.bar_layout(&ui).rect_of(BAR_SEARCH).is_none());
        assert!(!w.controls(&ui).iter().any(|n| n.role == Role::TextField));
    }

    #[test]
    fn a_tap_on_the_search_field_focuses_it_and_the_clear_button_empties_it() {
        let mut w = window("search-hit");
        let ui = ui();
        let r = w.bar_layout(&ui).rect_of(BAR_SEARCH).unwrap();
        let (mid_y, left) = (ui.pt(r.y + r.h / 2.0), ui.pt(r.x + 30.0));
        assert_eq!(w.hit(&ui, left, mid_y), Some(Target::Search));
        w.activate(Target::Search);
        type_text(&mut w, "bed");
        let right = ui.pt(r.x + r.w - 10.0);
        assert_eq!(w.hit(&ui, right, mid_y), Some(Target::SearchClear), "the clear button shows once there is text");
    }
}
