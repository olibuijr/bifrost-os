//! Reminders: lists with colours and icons, smart lists, subtasks, flags,
//! priority, notes/URL, due date and time with repeat rules, early alerts,
//! swipe actions with undo, drag to reorder, quick-add parsing, sort, group,
//! show/hide completed and search. State, input and persistence live here;
//! drawing is in `view`, every pure rule in `model`, `repeat`, `quick`.
//! The window is Golden Gate Golden Gate Reminders: a full-size window with the smart-list tiles and My Lists in a
//! sidebar, the open list (or a reminder's details) beside it, and the unified toolbar (`chrome`).
#[path = "view.rs"]
mod view;
#[path = "chrome.rs"]
mod chrome;
#[path = "completed.rs"]
mod completed;
#[path = "details.rs"]
mod details;
#[path = "purge.rs"]
mod purge;

use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL};
use springboard::layout::in_rect as within;
use crate::archive;
use bifrost_app::kit::ui::{home_zone, Ui};
use bifrost_app::kit::{Event, Image, TextCache};
use crate::{attach, format, model, organize, quick, repeat, settings, store, zones};
use springboard::deildu::scroll::Scroller;
use springboard::shell::clock::{
    tz::{civil_from_days, days_from_civil},
    Wall,
};
use springboard::shell::keyboard::Out;
use springboard::shell::textfield::TextField;
use bifrost_app::controls::undo::UndoManager;
use bifrost_app::controls::tips::{self, Datastore, Tip, TipSection, TipView, TipViewStyle};
use model::{Data, Group, ListDef, ListGroup, Options, Reminder, Row, Smart, Sort, View};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use store::local_day;
use purge::Purge;


const LONG_PRESS_MS: f64 = 400.0;
const UNDO_MS: f64 = 6000.0;
/// How many undoable changes the history keeps.
const UNDO_LIMIT: usize = 50;
const ITEM_H: f32 = 48.0;
/// Height in points of the first-use alert tip card in the details form.
const TIP_H: f32 = 64.0;
const HEAD_H: f32 = 40.0;
/// Width of one swipe button, in pt.
const BTN: f32 = 80.0;
/// Tops of the open list's rows (under the band and its large title) and of a details form, in points.
const ITEMS_TOP: f32 = 112.0;
const FORM_TOP: f32 = 64.0;
/// Points: the options popover's width and the tap zone at the Undo bar's right edge.
const OPTIONS_W_PT: f32 = 300.0;
const UNDO_HIT_PT: f32 = 120.0;
/// Content y (pt, below HOME_TOP) of the "My Lists" rows.
const MAX_SEARCH: usize = 100;
/// Pinned list tiles the sidebar shows beside the smart lists.
const MAX_PINNED: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    /// the open list beside the sidebar
    Items,
    Editor,
    ListEditor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Title,
    Notes,
    Url,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    /// shows or hides the sidebar
    Sidebar,
    New,
    NewList,
    Search,
    Options,
    Smart(Smart),
    /// Recently Deleted: opens the page of kept reminders.
    Deleted,
    /// Restores one kept reminder (by its store row) from the Recently Deleted page.
    Restore(u64),
    /// Selects one kept reminder (by its store row) on the Recently Deleted page (`purge.rs`).
    KeptRow(u64),
    List(u64),
    ListInfo(u64),
    /// The toolbar's "..." button: the open list's menu, shown at window pixel (x, y).
    ListMenu(f32, f32),
    /// A tag in the sidebar's Tags section, by its `model::tag_key`.
    Tag(u64),
    /// A custom smart list in the sidebar, by id.
    Custom(u64),
    /// Show List Info for the open custom smart list.
    SmartInfo(u64),
    /// A rule row of the smart list form.
    Rule(RuleRow),
    /// A row of the Settings form.
    Setting(SettingRow),
    /// The reminder form's Time Zone and Urgent rows.
    TimeZone,
    Urgent,
    /// The reminder form's picture rows: add one, the strip of thumbnails, remove the last.
    AddImage,
    Thumbs,
    RemoveImage,
    /// A section header row of the open list (its row index); choosing it edits the section.
    Section(usize),
    /// A list group's disclosure in the sidebar, by group id.
    ToggleGroup(u64),
    /// The list editor's Pin toggle and Group chooser.
    PinList,
    ListGroup,
    Row(usize),
    Circle(usize),
    Collapse(usize),
    SwipeFlag(usize),
    SwipeDelete(usize),
    Undo,
    OptSort(Sort),
    OptGroup(Group),
    OptCompleted,
    OptClearCompleted,
    /// None is "Automatic" (the first list).
    OptDefaultList(Option<u64>),
    OptExport,
    OptClose,
    Cancel,
    Save,
    Field(Field),
    Due,
    /// The first-use alert tip card (tapping it dismisses the tip).
    Tip,
    Calendar,
    Month(i32),
    Day(i64),
    Time,
    TimeStep,
    Hour(i32),
    Minute(i32),
    Repeat,
    Early,
    Flag,
    Priority,
    MoveList,
    Indent,
    Duplicate,
    Delete,
    Colors,
    Icons,
    Color(u8),
    Icon(u8),
    ListUp,
    ListDown,
    Scroll,
    /// The "i" of a list row (its row index): the reminder's Details popover (`details.rs`).
    Info(usize),
    /// A press outside the Details popover: it closes, keeping the edits.
    DetailsClose,
    /// The completed strip's Clear (`completed.rs`): asks first, then clears or not.
    ClearAsk,
    ClearConfirm,
    ClearCancel,
    /// The purge alert's delete button (`purge.rs`); its Cancel is `ClearCancel`.
    PurgeConfirm,
}

struct Editor {
    id: Option<u64>,
    draft: Reminder,
    month: i64,
    /// Pictures stored while this form has been open; they are deleted again if the form is cancelled.
    added: Vec<String>,
}

/// What the list editor form edits: a list, a section of a list (by list id) or a list group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditMode {
    List,
    Section(u64),
    Group,
    /// A custom smart list: a name, a colour and its rules.
    Smart,
    /// Reminders > Settings: no name, only the setting rows.
    Settings,
}

/// The rows of the Settings form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingRow {
    DefaultList,
    AllDayAlert,
    Badge,
}

impl SettingRow {
    const ALL: [Self; 3] = [Self::DefaultList, Self::AllDayAlert, Self::Badge];
}

/// The rows of the smart list form that choose a rule; each cycles through its choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleRow {
    Due,
    Time,
    Priority,
    Flagged,
    Tag,
    List,
}

impl RuleRow {
    const ALL: [Self; 6] = [Self::Due, Self::Time, Self::Priority, Self::Flagged, Self::Tag, Self::List];
    fn label(self) -> &'static str {
        match self {
            Self::Due => "Date",
            Self::Time => "Time of Day",
            Self::Priority => "Priority",
            Self::Flagged => "Flag",
            Self::Tag => "Tag",
            Self::List => "List",
        }
    }
}

/// The list editor's draft. `id` is the list, section or group being changed (`None` while creating one).
struct ListEdit {
    id: Option<u64>,
    color: u8,
    icon: u8,
    pinned: bool,
    group: Option<u64>,
    mode: EditMode,
    /// The smart list's rules while its form is open.
    rules: model::SmartRules,
    /// Delete was pressed once on a list's form: the next press deletes the list and its reminders (E2E W2-21).
    confirm_delete: bool,
}

/// The rows of a list's menu (the toolbar's "..." and a right click on a list in the sidebar).
pub const CTX_LIST_INFO: u32 = 1;
pub const CTX_PIN: u32 = 2;
pub const CTX_NEW_SECTION: u32 = 3;
pub const CTX_DELETE_LIST: u32 = 4;

struct UndoBar {
    label: String,
    until: f64,
}

/// The first-use tip for the alert controls in the details form.
struct AlertHint;

impl Tip for AlertHint {
    fn id(&self) -> &str {
        "is.olibuijr.reminders.tip.alerts"
    }

    fn title(&self) -> &str {
        "Get an alert"
    }

    fn message(&self) -> &str {
        "Set a due date and a time. The alert still shows when Reminders is closed."
    }
}

/// Seconds since the Unix epoch, for the tip's display gap.
fn now_seconds() -> tips::Seconds {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Puts removed reminders back (undo of a delete or a clear).
fn restore_items(removed: Vec<(usize, Reminder)>) -> impl Fn(&mut Data) + 'static {
    move |d| model::restore(&mut d.items, removed.clone())
}

/// Flips the flag of reminder `id`; undo and redo both do this.
fn flag_toggle(id: u64) -> impl Fn(&mut Data) + 'static {
    move |d| {
        if let Some(r) = d.items.iter_mut().find(|r| r.id == id) {
            r.flagged = !r.flagged;
        }
    }
}

/// Sets the data back to a saved state: the undo and redo of an edit that changed the data as a whole.
fn snapshot(state: Data) -> impl Fn(&mut Data) + 'static {
    move |d| *d = state.clone()
}

/// The undo name of a completion change, from the state after it.
fn completion_name(items: &[Reminder], id: u64) -> &'static str {
    if items.iter().any(|r| r.id == id && r.completed) {
        "Mark as Completed"
    } else {
        "Mark as Incomplete"
    }
}

struct Swipe {
    id: u64,
    /// current content offset in px (<= 0)
    x: f32,
    base: f32,
    open: bool,
}

struct Drag {
    id: u64,
    y: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Undecided,
    Vertical,
    Horizontal,
}

pub struct Reminders {
    /// the content column's width (the window less the sidebar) and its left edge, in px
    w: i32,
    x0: f32,
    full_w: i32,
    /// View > Hide Sidebar
    side_hidden: bool,
    h: i32,
    /// Safe inset at the top in points (from the shared `Layout`; 0 on the desktop).
    top_pt: f32,
    path: PathBuf,
    /// Reminders > Settings, kept in `settings.tsv` beside the reminders.
    settings: settings::Settings,
    settings_path: PathBuf,
    /// The folder beside the reminders file that holds the attached pictures.
    attach_dir: PathBuf,
    /// Thumbnails of attached pictures by file name (`None`: the file is missing or does not decode).
    thumbs: HashMap<String, Option<Image>>,
    /// The form asked for a picture; `main.rs` opens the open panel.
    image_request: bool,
    wall: Wall,
    data: Data,
    /// The Recently Deleted page: the kept reminders with their store rows, while the page is open.
    deleted: Option<Vec<archive::Kept>>,
    /// How many reminders the recently deleted store holds (the sidebar count).
    kept_count: usize,
    /// The lowest id a new reminder may take: past every kept reminder's id, so a new reminder never reuses the id of
    /// one in Recently Deleted (E2E N3). It only rises.
    id_floor: u64,
    /// The store row of the kept reminder selected on the Recently Deleted page, and the purge alert while it is up
    /// (`purge.rs`).
    kept_selected: Option<u64>,
    purge_ask: Option<Purge>,
    /// The menu the shell shows is the Recently Deleted menu (`purge.rs`), not a list's.
    menu_kept: bool,
    screen: Screen,
    view: View,
    opts: Options,
    options_open: bool,
    /// Desktop only: Export List saves a Markdown file under ~/Documents.
    file_export: bool,
    /// A confirmation, shown in place of the error line.
    notice: Option<String>,
    searching: bool,
    saved_query: String,
    rows: Vec<Row>,
    /// Row offsets in pt from the top of the list content.
    row_y: Vec<f32>,
    items_h: f32,
    collapsed: HashSet<u64>,
    /// List groups folded shut in the sidebar.
    collapsed_groups: HashSet<u64>,
    today: i64,
    editor: Option<Editor>,
    /// First-use tips (`TipView`); `None` keeps the tip hidden, as in tests.
    tips: Option<Datastore>,
    /// The tip's `mark_shown` was recorded for this session.
    tip_marked: bool,
    list_edit: Option<ListEdit>,
    /// A menu for the shell to show (window pixels and rows), and the list or smart list its choice acts on.
    menu_request: Option<(f32, f32, Vec<bifrost_proto::ContextItem>)>,
    menu_view: Option<View>,
    title: TextField,
    title_hint: &'static str,
    title_max: usize,
    notes: TextField,
    url: TextField,
    focus: Option<Field>,
    kb_on: bool,
    labels: TextCache,
    scroll: Scroller,
    pressed: Option<Target>,
    field_touch: Option<Field>,
    scrolling: bool,
    axis: Axis,
    down: (f32, f32),
    press_at: Option<f64>,
    swipe: Option<Swipe>,
    /// Row selected with the arrow keys (an item id).
    selected: Option<u64>,
    undo: Option<UndoBar>,
    /// Every undoable change (`NativeUndoManager`); the Undo bar offers the latest one.
    history: UndoManager<Data>,
    drag: Option<Drag>,
    /// The list's scroll offset while a Details popover uses the scroller for its form (`details.rs`).
    list_scroll: f32,
    /// The completed strip's Clear alert is up (`completed.rs`).
    clear_ask: bool,
    /// The Clear or purge alert's button that has the focus ring (Tab moves it, Space presses it); `None` until Tab.
    ask_focus: Option<u32>,
    dirty: bool,
    error: bool,
    load_error: bool,
    style: (u32, bool),
    gen: u32,
}

fn month_start(day: i64) -> i64 {
    let (year, month, _) = civil_from_days(day);
    days_from_civil(year, month, 1)
}
fn mk_field(ui: &Ui, hint: &str, max: usize, text: &str) -> TextField {
    let mut f = TextField::new(ui, hint, bifrost_app::kit::ui::MAC_BODY_PT, false, max);
    f.set_text(ui, text);
    f.focused = false;
    f
}
/// `YYYY-MM-DD` (or with `THH:MM` after it) as a local day since 1970-01-01 and the minute after midnight.
fn iso_due(text: &str) -> Option<(i64, Option<u16>)> {
    let (date, time) = match text.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (text, None),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    let minute = match time {
        Some(t) => Some(t.get(0..2)?.parse::<u16>().ok()? * 60 + t.get(3..5)?.parse::<u16>().ok()?),
        None => None,
    };
    Some((days_from_civil(year, month, day), minute))
}

fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Reminders {
    /// Attaches the first-use tip store (its state file lives in the app's data folder).
    pub fn with_tips(mut self, store: Datastore) -> Self {
        self.tips = Some(store);
        self
    }

    /// The alert tip's sections now. Empty once it was dismissed, or with no store.
    fn tip_sections(&self, now: tips::Seconds) -> Vec<TipSection> {
        match &self.tips {
            Some(store) => TipView::new(&AlertHint, TipViewStyle::Mini).sections(store, now),
            None => Vec::new(),
        }
    }

    /// A tap on the tip, or setting a due date, dismisses it for good.
    fn dismiss_tip(&mut self) {
        if let Some(store) = &mut self.tips {
            let _ = store.invalidate(AlertHint.id());
        }
    }

    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.full_w = w;
        self.w = w - self.x0 as i32;
        self.h = h;
    }

    /// The safe inset above the content, in points (`bifrost_app::layout::Layout::top`).
    pub fn set_top_inset(&mut self, pt: f32) {
        self.top_pt = pt;
    }

    pub fn new(ui: &Ui, w: i32, h: i32, path: PathBuf, wall: Wall) -> Self {
        let loaded = store::load_checked(&path);
        let load_error = loaded.is_err();
        let settings_path = path.with_file_name("settings.tsv");
        let attach_dir = path.with_file_name("attachments");
        let settings = settings::load(&settings_path);
        let opts = Options {
            default_list: settings.default_list,
            sort: settings.sort,
            hidden_smart: settings.hidden_smart,
            ..Options::default()
        };
        let collapsed: HashSet<u64> = settings.collapsed.iter().copied().collect();
        let kept_now = archive::kept(&archive::path_beside(&path)).unwrap_or_default();
        let kept_count = kept_now.len();
        let id_floor = kept_now.iter().map(|r| r.id.saturating_add(1)).max().unwrap_or(0);
        let mut app = Self {
            kept_count,
            id_floor,
            deleted: None,
            kept_selected: None,
            purge_ask: None,
            menu_kept: false,
            w,
            x0: 0.0,
            full_w: w,
            side_hidden: false,
            h,
            top_pt: 0.0,
            data: loaded.unwrap_or_default(),
            path,
            settings,
            settings_path,
            attach_dir,
            thumbs: HashMap::new(),
            image_request: false,
            wall,
            screen: Screen::Items,
            view: View::Smart(Smart::All),
            opts,
            options_open: false,
            file_export: false,
            notice: None,
            searching: false,
            saved_query: String::new(),
            rows: vec![],
            row_y: vec![],
            items_h: 0.0,
            collapsed,
            collapsed_groups: HashSet::new(),
            today: local_day(wall.unix, wall.offset),
            editor: None,
            tips: None,
            tip_marked: false,
            list_edit: None,
            menu_request: None,
            menu_view: None,
            title: mk_field(ui, "Reminder", model::MAX_TITLE, ""),
            title_hint: "Reminder",
            title_max: model::MAX_TITLE,
            notes: mk_field(ui, "Notes", model::MAX_NOTES, ""),
            url: mk_field(ui, "URL", model::MAX_URL, ""),
            focus: None,
            kb_on: false,
            labels: TextCache::default(),
            scroll: Scroller::new(),
            pressed: None,
            field_touch: None,
            scrolling: false,
            axis: Axis::Undecided,
            down: (0.0, 0.0),
            press_at: None,
            swipe: None,
            selected: None,
            undo: None,
            history: UndoManager::new(UNDO_LIMIT),
            drag: None,
            list_scroll: 0.0,
            clear_ask: false,
            ask_focus: None,
            dirty: true,
            error: load_error,
            load_error,
            style: (ui.text_scale.to_bits(), ui.bold),
            gen: 0,
        };
        app.layout_columns(ui);
        app.rebuild(ui);
        app
    }

    /// Bumped on every successful save; the alert scheduler watches it.
    /// Open reminders due today or earlier (the dock badge).
    pub fn due_count(&self) -> usize {
        model::due_count(&self.data.items, self.today)
    }

    /// Every reminder, for the Today widget timeline (`system.rs`).
    /// The lists in their sidebar order, for the scripting answers (`scripting`).
    pub fn lists(&self) -> &[model::ListDef] {
        &self.data.lists
    }

    pub fn items(&self) -> &[model::Reminder] {
        &self.data.items
    }

    /// The local day number the app counts due dates from (the badge and the Today widget use it).
    pub fn today(&self) -> i64 {
        self.today
    }

    /// Whether the list of reminders is on screen; the New, Find and Delete commands act only there.
    pub fn on_items(&self) -> bool {
        self.screen == Screen::Items
    }

    /// The arrow-selected reminder, if it still exists and is a row of the list on screen. A selection left over from
    /// another list, or hidden under the Recently Deleted page, is not one: the commands on the selection (Delete
    /// Reminder above all) never act on a reminder the user cannot see (E2E R2).
    pub fn selected_item(&self) -> Option<u64> {
        if self.deleted.is_some() {
            return None;
        }
        self.selected.filter(|id| self.row_shown(*id))
    }

    /// Whether reminder `id` is a row of the list on screen.
    fn row_shown(&self, id: u64) -> bool {
        self.rows.iter().any(|row| matches!(row, Row::Item { idx, .. } if self.data.items.get(*idx).is_some_and(|r| r.id == id)))
    }

    /// Selects the reminder `id` on the items screen (a Core Search result opened it); false when there is no such one.
    /// When the list on screen does not show it, its own list opens.
    pub fn show_item(&mut self, id: u64) -> bool {
        let Some(list) = self.data.items.iter().find(|r| r.id == id).map(|r| r.list) else {
            return false;
        };
        self.screen = Screen::Items;
        if self.deleted.is_some() || !self.row_shown(id) {
            self.view = View::List(list);
            self.deleted = None;
            self.searching = false;
            self.swipe = None;
            self.scroll.jump_top();
            self.refresh_rows();
            self.labels.clear();
        }
        self.selected = Some(id);
        self.dirty = true;
        true
    }

    /// File > New Reminder (Cmd+N), File > New List (Cmd+Shift+N), Edit > Find (Cmd+F): the toolbar and chord paths.
    pub fn new_reminder(&mut self, ui: &Ui, t: f64) {
        self.activate(ui, Target::New, t);
    }

    pub fn new_list(&mut self, ui: &Ui, t: f64) {
        self.activate(ui, Target::NewList, t);
    }

    pub fn find(&mut self, ui: &Ui, t: f64) {
        self.activate(ui, Target::Search, t);
    }

    /// Edit > Mark as Completed: the selected reminder, as its circle does. False when none is selected.
    pub fn toggle_selected(&mut self, ui: &Ui) -> bool {
        let Some(id) = self.selected_item() else { return false };
        self.toggle_item(ui, id);
        true
    }

    /// File > Delete Reminder (Cmd+Backspace): the selected reminder, as the chord deletes it. On the Recently Deleted
    /// page it asks before the selected kept reminder goes for good (`purge.rs`).
    pub fn delete_selected(&mut self, ui: &Ui, t: f64) -> bool {
        if self.deleted.is_some() {
            return self.purge_selected();
        }
        let Some(id) = self.selected_item() else { return false };
        if self.delete_item(id, t) {
            self.rebuild(ui);
        }
        true
    }


    // ---- menu commands --------------------------------------------------
    // Each acts on the arrow-selected reminder or on the open list, as its menu item says, and goes on the undo
    // history under the item's name. The menu enables them from the queries next to them.

    /// The open list's id: a list is on screen, not a smart list, a tag or a search.
    pub fn open_list(&self) -> Option<u64> {
        match self.view {
            View::List(id) if self.screen == Screen::Items && !self.searching => Some(id),
            _ => None,
        }
    }

    pub fn open_list_pinned(&self) -> bool {
        self.open_list().is_some_and(|id| self.data.lists.iter().any(|l| l.id == id && l.pinned))
    }

    /// Reminders > Indent (Cmd+]): the selected reminder becomes a subtask of the one above it.
    pub fn can_indent(&self) -> bool {
        self.on_items() && self.selected_item().is_some_and(|id| organize::can_indent(&self.data.items, id))
    }

    pub fn indent_selected(&mut self, ui: &Ui) -> bool {
        let Some(id) = self.selected_item().filter(|_| self.on_items()) else { return false };
        let mut next = self.data.clone();
        if !model::indent(&mut next.items, id) {
            return false;
        }
        // a subtask is shown in its parent's section
        let parent = next.items.iter().find(|r| r.id == id).and_then(|r| r.parent);
        let section = parent.and_then(|p| next.items.iter().find(|r| r.id == p)).and_then(|p| p.section);
        if let Some(r) = next.items.iter_mut().find(|r| r.id == id) {
            r.section = section;
        }
        let saved = self.commit_named(next, "Indent");
        if saved {
            self.rebuild(ui);
        }
        saved
    }

    /// Reminders > Outdent (Cmd+[): the selected subtask becomes a top-level reminder again.
    pub fn can_outdent(&self) -> bool {
        self.on_items() && self.selected_item().is_some_and(|id| self.data.items.iter().any(|r| r.id == id && r.parent.is_some()))
    }

    pub fn outdent_selected(&mut self, ui: &Ui) -> bool {
        let Some(id) = self.selected_item().filter(|_| self.on_items()) else { return false };
        let mut next = self.data.clone();
        if !organize::outdent(&mut next.items, id) {
            return false;
        }
        let saved = self.commit_named(next, "Outdent");
        if saved {
            self.rebuild(ui);
        }
        saved
    }

    /// Whether any reminder has subtasks (what View > Hide All Subtasks acts on).
    pub fn has_subtasks(&self) -> bool {
        self.data.items.iter().any(|r| r.parent.is_some())
    }

    /// Whether every reminder with subtasks is folded shut.
    pub fn subtasks_hidden(&self) -> bool {
        let parents = organize::parent_ids(&self.data.items);
        !parents.is_empty() && parents.iter().all(|p| self.collapsed.contains(p))
    }

    /// View > Hide All Subtasks / Show All Subtasks.
    pub fn toggle_all_subtasks(&mut self, ui: &Ui) {
        if self.subtasks_hidden() {
            self.collapsed.clear();
        } else {
            self.collapsed.extend(organize::parent_ids(&self.data.items));
        }
        self.save_collapsed();
        self.rebuild(ui);
    }

    /// Keeps the folded subtasks in the settings file, so they are still folded after a restart.
    fn save_collapsed(&mut self) {
        self.settings.collapsed = self.collapsed.iter().copied().collect();
        self.save_settings();
    }

    /// Reminders > Mark Due Date As: the selected open reminder gets the day, keeping its time.
    pub fn mark_selected_due(&mut self, ui: &Ui, mark: organize::MarkDue) -> bool {
        let Some(id) = self.selected_item().filter(|_| self.on_items()) else { return false };
        let mut next = self.data.clone();
        if !organize::mark_due(&mut next.items, id, organize::mark_day(mark, self.today)) {
            return false;
        }
        let saved = self.commit_named(next, &format!("Mark Due Date as {}", mark.label()));
        if saved {
            self.rebuild(ui);
        }
        saved
    }

    /// Whether Mark All Overdue as Today has anything to move.
    pub fn has_overdue(&self) -> bool {
        organize::has_overdue(&self.data.items, self.today)
    }

    /// Whether the selected reminder is marked urgent.
    pub fn selected_urgent(&self) -> bool {
        self.selected_item().is_some_and(|id| self.data.items.iter().any(|r| r.id == id && r.urgent))
    }

    /// Reminders > Mark as Urgent / Remove Urgent Mark: once an urgent reminder is due, its alert repeats every minute
    /// until it is completed (see `alerts`).
    pub fn toggle_urgent_selected(&mut self, ui: &Ui) -> bool {
        let Some(id) = self.selected_item().filter(|_| self.on_items()) else { return false };
        let mut next = self.data.clone();
        let Some(r) = next.items.iter_mut().find(|r| r.id == id) else { return false };
        r.urgent = !r.urgent;
        let name = if r.urgent { "Mark as Urgent" } else { "Remove Urgent Mark" };
        let saved = self.commit_named(next, name);
        if saved {
            self.rebuild(ui);
        }
        saved
    }

    /// Whether the details form asked for a picture since the last call; `main.rs` then opens the open panel.
    pub fn take_image_request(&mut self) -> bool {
        std::mem::take(&mut self.image_request)
    }

    /// The open panel answered: stores the picture and adds it to the reminder being edited. The form keeps it until
    /// Done; Cancel deletes it again.
    pub fn attach_image(&mut self, bytes: &[u8]) -> Result<(), String> {
        let Some(e) = &mut self.editor else { return Err("no reminder is open".into()) };
        if e.draft.attachments.len() >= attach::MAX_PER_REMINDER {
            return Err(format!("a reminder holds up to {} pictures", attach::MAX_PER_REMINDER));
        }
        let name = attach::store(&self.attach_dir, bytes, self.wall.unix).map_err(|_| "the picture is not a PNG or JPEG, or is too large".to_string())?;
        e.draft.attachments.push(name.clone());
        e.added.push(name);
        self.dirty = true;
        Ok(())
    }

    /// Deletes the pictures stored for a form that is cancelled.
    fn discard_new_pictures(&mut self) {
        if let Some(e) = &mut self.editor {
            for name in std::mem::take(&mut e.added) {
                attach::remove(&self.attach_dir, &name);
                self.thumbs.remove(&name);
            }
        }
    }

    /// Reminders > Mark All Overdue as Today: one undo step for every reminder moved.
    /// Whether the notes field of the open form has the focus, which is where Format > Bold and the list commands go.
    pub fn notes_focused(&self) -> bool {
        self.has_draft() && self.focus == Some(Field::Notes)
    }

    /// Format > Bold, Bullet List and Numbered List: the mark goes into the notes of the open form (`format`). The
    /// notes stay plain Markdown text, and the caret goes to the end of the text.
    pub fn format_notes(&mut self, ui: &Ui, style: format::Style) -> bool {
        if !self.notes_focused() {
            return false;
        }
        let (text, _) = format::apply(self.notes.text(), self.notes.selection(), style);
        self.notes.set_text(ui, &text);
        self.dirty = true;
        true
    }

    pub fn mark_overdue_today(&mut self, ui: &Ui) -> bool {
        let mut next = self.data.clone();
        if organize::overdue_to_today(&mut next.items, self.today) == 0 {
            return false;
        }
        let saved = self.commit_named(next, "Mark All Overdue as Today");
        if saved {
            self.rebuild(ui);
        }
        saved
    }

    /// View > Sort By.
    pub fn sort(&self) -> Sort {
        self.opts.sort
    }

    pub fn set_sort(&mut self, ui: &Ui, sort: Sort) {
        self.opts.sort = sort;
        self.settings.sort = sort;
        self.save_settings();
        self.rebuild(ui);
    }

    /// View > Show Smart List: whether the sidebar shows the smart list's tile.
    pub fn smart_shown(&self, smart: Smart) -> bool {
        self.opts.smart_shown(smart)
    }

    pub fn toggle_smart(&mut self, smart: Smart) {
        self.opts.toggle_smart(smart);
        self.settings.hidden_smart = self.opts.hidden_smart;
        self.save_settings();
    }

    /// Lists > Pin List / Unpin List: a pinned list has a tile beside the smart lists.
    pub fn toggle_pin(&mut self) -> bool {
        let Some(list) = self.open_list() else { return false };
        let pinned = self.data.lists.iter().filter(|l| l.pinned).count();
        let mut next = self.data.clone();
        let Some(l) = next.lists.iter_mut().find(|l| l.id == list) else { return false };
        if !l.pinned && pinned >= MAX_PINNED {
            self.notice = Some(format!("Up to {MAX_PINNED} lists can be pinned"));
            self.dirty = true;
            return false;
        }
        l.pinned = !l.pinned;
        let name = if l.pinned { "Pin List" } else { "Unpin List" };
        self.commit_named(next, name)
    }

    /// Lists > New Section: opens the name form for a section of the open list.
    pub fn new_section(&mut self, ui: &Ui, t: f64) -> bool {
        let Some(list) = self.open_list() else { return false };
        if self.data.lists.iter().any(|l| l.id == list && l.sections.len() >= model::MAX_SECTIONS) {
            self.notice = Some(format!("A list holds up to {} sections", model::MAX_SECTIONS));
            self.dirty = true;
            return false;
        }
        self.open_section_editor(ui, list, None, t);
        true
    }

    /// The sections of the open list, in order.
    pub fn open_list_sections(&self) -> Vec<(u64, String)> {
        let Some(list) = self.open_list() else { return vec![] };
        self.data.lists.iter().find(|l| l.id == list).map_or(vec![], |l| l.sections.iter().map(|s| (s.id, s.name.clone())).collect())
    }

    /// Whether the selected reminder is a top-level reminder of the open list, so Move to Section applies.
    pub fn can_move_to_section(&self) -> bool {
        let Some(list) = self.open_list() else { return false };
        self.selected_item().is_some_and(|id| self.data.items.iter().any(|r| r.id == id && r.list == list && r.parent.is_none()))
    }

    /// The section the selected reminder is in (none: no section).
    pub fn selected_section(&self) -> Option<u64> {
        let id = self.selected_item()?;
        self.data.items.iter().find(|r| r.id == id).and_then(|r| r.section)
    }

    /// Reminders > Move to Section: the selected reminder goes last in `section` (or out of any section).
    pub fn move_selected_to_section(&mut self, ui: &Ui, section: Option<u64>) -> bool {
        if !self.can_move_to_section() || section.is_some_and(|s| !self.open_list_sections().iter().any(|(id, _)| *id == s)) {
            return false;
        }
        let Some(id) = self.selected_item() else { return false };
        let mut next = self.data.clone();
        if !organize::place_in_section(&mut next.items, id, section, None) {
            return false;
        }
        let saved = self.commit_named(next, "Move to Section");
        if saved {
            self.rebuild(ui);
        }
        saved
    }

    /// Lists > Auto-Categorize: grocery sections for the open list's loose reminders.
    pub fn auto_categorize(&mut self, ui: &Ui) -> bool {
        let Some(list) = self.open_list() else { return false };
        let mut next = self.data.clone();
        let placed = organize::auto_categorize(&mut next, list);
        if placed == 0 {
            self.notice = Some("No reminders to categorize".to_string());
            self.dirty = true;
            return false;
        }
        let saved = self.commit_named(next, "Auto-Categorize");
        if saved {
            self.notice = Some(format!("Placed {placed} {} in sections", if placed == 1 { "reminder" } else { "reminders" }));
            self.rebuild(ui);
        }
        saved
    }

    /// File > New Smart List: opens the form for a custom smart list.
    pub fn new_smart_list(&mut self, ui: &Ui, t: f64) -> bool {
        if self.data.smarts.len() >= model::MAX_SMARTS {
            self.notice = Some(format!("Up to {} smart lists can be made", model::MAX_SMARTS));
            self.dirty = true;
            return false;
        }
        self.open_smart_editor(ui, None, t);
        true
    }

    /// Lists > New List Group: opens the name form for a group.
    pub fn new_group(&mut self, ui: &Ui, t: f64) {
        self.open_group_editor(ui, None, t);
    }

    /// Lists > Edit Group: opens the form for group `id`.
    pub fn edit_group(&mut self, ui: &Ui, id: u64, t: f64) {
        if self.data.groups.iter().any(|g| g.id == id) {
            self.open_group_editor(ui, Some(id), t);
        }
    }

    pub fn groups(&self) -> Vec<(u64, String)> {
        self.data.groups.iter().map(|g| (g.id, g.name.clone())).collect()
    }

    /// Lists > Save List as Template: the open list's reminders, without dates or completion, under the list's name.
    pub fn save_as_template(&mut self) -> bool {
        let Some(list) = self.open_list() else { return false };
        let Some(name) = self.data.lists.iter().find(|l| l.id == list).map(|l| l.name.clone()) else { return false };
        let mut next = self.data.clone();
        if organize::save_template(&mut next, list, &name).is_none() {
            self.notice = Some("Could not save the template".to_string());
            self.dirty = true;
            return false;
        }
        let saved = self.commit_named(next, "Save as Template");
        if saved {
            self.notice = Some(format!("Saved the template \u{201c}{name}\u{201d}"));
        }
        saved
    }

    pub fn templates(&self) -> Vec<(u64, String)> {
        self.data.templates.iter().map(|t| (t.id, t.name.clone())).collect()
    }

    /// Lists > New List from Template: a new list with the template's reminders, opened.
    pub fn new_list_from_template(&mut self, ui: &Ui, tid: u64) -> bool {
        if self.has_draft() {
            return false;
        }
        let mut next = self.data.clone();
        let Some(list) = organize::list_from_template(&mut next, tid, Some(self.wall.unix)) else { return false };
        let saved = self.commit_named(next, "New List from Template");
        if saved {
            self.open_view(ui, View::List(list));
        }
        saved
    }

    pub fn delete_template(&mut self, tid: u64) -> bool {
        let mut next = self.data.clone();
        organize::delete_template(&mut next, tid) && self.commit_named(next, "Delete Template")
    }

    /// File > Import List: `text` is a Markdown checklist (the Export List format); `fallback` names the list when the
    /// text has no heading. The new list opens.
    pub fn import_checklist(&mut self, ui: &Ui, text: &str, fallback: &str) -> bool {
        if self.has_draft() {
            return false;
        }
        let checklist = organize::parse_checklist(text);
        let mut next = self.data.clone();
        let Some(list) = organize::import_checklist(&mut next, &checklist, fallback, Some(self.wall.unix)) else {
            self.notice = Some("Nothing to import".to_string());
            self.dirty = true;
            return false;
        };
        let saved = self.commit_named(next, "Import List");
        if saved {
            self.open_view(ui, View::List(list));
            self.notice = Some(format!("Imported {} reminders", checklist.items.len()));
        }
        saved
    }

    /// Whether a list, or a smart list on screen (not a search), has something for File > Print List.
    pub fn can_print(&self) -> bool {
        self.open_list().is_some() || (self.screen == Screen::Items && !self.searching && matches!(self.view, View::Smart(_) | View::Custom(_)))
    }

    /// File > Print List: the list's name and its Markdown checklist, when a list or a smart list is on screen.
    /// A smart list prints its rows as one flat checklist under its own name.
    pub fn printable_list(&self) -> Option<(String, String)> {
        if let Some(list) = self.open_list() {
            let name = self.data.lists.iter().find(|l| l.id == list)?.name.clone();
            return Some((name, model::export_markdown(&self.data, list)));
        }
        if !self.can_print() {
            return None;
        }
        let name = match self.view {
            View::Smart(s) => s.label().to_string(),
            View::Custom(id) => self.data.smarts.iter().find(|s| s.id == id)?.name.clone(),
            _ => return None,
        };
        let items: Vec<&model::Reminder> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Item { idx, .. } => self.data.items.get(*idx),
                _ => None,
            })
            .collect();
        let text = model::flat_markdown(&name, &items);
        Some((name, text))
    }

    // ---- intents ---------------------------------------------------------

    /// The `create-list` intent: a new list with the next colour. Returns its id.
    pub fn create_list(&mut self, ui: &Ui, name: &str) -> Result<u64, String> {
        let name = organize::clean_name(name).ok_or_else(|| "the name is empty".to_string())?;
        let mut next = self.data.clone();
        let id = next.next_list_id().ok_or_else(|| "no list id is left".to_string())?;
        let color = (next.lists.len() % view::COLORS.len()) as u8;
        next.lists.push(ListDef::new(id, name, color, 0));
        if self.commit_named(next, "New List") {
            self.rebuild(ui);
            Ok(id)
        } else {
            Err("the reminders could not be saved".into())
        }
    }

    /// The Quick Reminder menu extra: `text` is read as the quick-add line ("pay rent every month !!") and becomes a
    /// reminder in the default list. Returns its id.
    pub fn quick_add(&mut self, ui: &Ui, text: &str, t: f64) -> Result<u64, String> {
        let line = clean(&text.chars().take(1000).collect::<String>());
        if line.is_empty() {
            return Err("there is no text".into());
        }
        let q = quick::parse(&line, self.today, self.now_min(t));
        let title: String = clean(&q.title).chars().take(model::MAX_TITLE).collect();
        if title.is_empty() {
            return Err("the title is empty".into());
        }
        let mut next = self.data.clone();
        let list = self.opts.default_list.filter(|id| next.lists.iter().any(|l| l.id == *id)).or_else(|| next.lists.first().map(|l| l.id));
        let Some(list) = list else { return Err("there is no list".into()) };
        let Some(id) = self.fresh_item_id(&next) else { return Err("no reminder id is left".into()) };
        let mut r = Reminder::new(id, title);
        r.list = list;
        r.due = q.due;
        r.time = q.time;
        r.repeat = q.repeat;
        r.priority = q.priority;
        r.created = Some(self.wall.now(t));
        next.items.push(r);
        if self.commit_named(next, "New Reminder") {
            self.rebuild(ui);
            Ok(id)
        } else {
            Err("the reminders could not be saved".into())
        }
    }

    /// The `open-list` intent: shows the list whose name matches without case. Returns the list's name.
    pub fn open_list_named(&mut self, ui: &Ui, name: &str) -> Result<String, String> {
        if self.has_draft() {
            return Err("finish the reminder that is open first".into());
        }
        let wanted = name.trim();
        let found = self.data.lists.iter().find(|l| l.name.eq_ignore_ascii_case(wanted)).map(|l| (l.id, l.name.clone()));
        let (id, name) = found.ok_or_else(|| format!("there is no list named \"{wanted}\""))?;
        self.open_view(ui, View::List(id));
        Ok(name)
    }

    /// The `open-smart-list` intent: shows the smart list called `name` (Today, Scheduled, Flagged, All, Completed).
    pub fn open_smart_named(&mut self, ui: &Ui, name: &str) -> Result<Smart, String> {
        if self.has_draft() {
            return Err("finish the reminder that is open first".into());
        }
        let smart = Smart::ALL.iter().copied().find(|s| s.label().eq_ignore_ascii_case(name.trim())).ok_or_else(|| format!("there is no smart list called \"{}\"", name.trim()))?;
        self.open_view(ui, View::Smart(smart));
        Ok(smart)
    }

    /// The `search-reminders` intent: runs the search and returns how many reminders match.
    pub fn search_for(&mut self, ui: &Ui, query: &str, t: f64) -> Result<usize, String> {
        if self.has_draft() {
            return Err("finish the reminder that is open first".into());
        }
        let query = clean(query);
        if query.is_empty() {
            return Err("the search is empty".into());
        }
        self.begin_search(ui, t);
        self.set_title_field(ui, "Search", MAX_SEARCH, &query);
        self.rebuild(ui);
        Ok(self.rows.iter().filter(|r| matches!(r, Row::Item { .. })).count())
    }

    /// The `add-reminder` intent: a reminder in the default list, with an optional due day (`YYYY-MM-DD`, or
    /// `YYYY-MM-DDTHH:MM` for a time). Returns the new reminder's id.
    pub fn add_reminder(&mut self, ui: &Ui, title: &str, due: Option<&str>) -> Result<u64, String> {
        self.add_reminder_in(ui, title, due, None)
    }

    /// The `create-list-item` intent: a reminder in the list called `list` (matched without case), with the same
    /// optional due day as `add-reminder`. Returns the new reminder's id.
    pub fn add_reminder_to_named(&mut self, ui: &Ui, title: &str, due: Option<&str>, list: &str) -> Result<u64, String> {
        let wanted = list.trim();
        let found = self.data.lists.iter().find(|l| l.name.eq_ignore_ascii_case(wanted)).map(|l| l.id);
        let id = found.ok_or_else(|| format!("there is no list named \"{wanted}\""))?;
        self.add_reminder_in(ui, title, due, Some(id))
    }

    /// Adds a reminder to `list`, or to the default list when `list` is `None`. The title and the due date are checked
    /// first, so a bad one never picks a list.
    fn add_reminder_in(&mut self, ui: &Ui, title: &str, due: Option<&str>, list: Option<u64>) -> Result<u64, String> {
        let title = clean(title);
        if title.is_empty() {
            return Err("the title is empty".into());
        }
        let (day, time) = match due {
            Some(text) => {
                let (day, time) = iso_due(text).ok_or_else(|| "the due date is not a date".to_string())?;
                (Some(day), time)
            }
            None => (None, None),
        };
        let mut next = self.data.clone();
        let list = match list {
            Some(id) => Some(id),
            None => self.opts.default_list.filter(|id| next.lists.iter().any(|l| l.id == *id)).or_else(|| next.lists.first().map(|l| l.id)),
        };
        let Some(list) = list else { return Err("there is no list".into()) };
        let Some(id) = self.fresh_item_id(&next) else { return Err("no reminder id is left".into()) };
        let mut r = Reminder::new(id, title);
        r.list = list;
        r.created = Some(self.wall.unix);
        r.due = day;
        r.time = time;
        next.items.push(r);
        if self.commit(next) {
            self.rebuild(ui);
            Ok(id)
        } else {
            Err("the reminders could not be saved".into())
        }
    }
    pub fn generation(&self) -> u32 {
        self.gen
    }
    pub fn set_wall(&mut self, wall: Wall) {
        self.wall = wall;
        let today = local_day(wall.unix, wall.offset);
        if today != self.today {
            self.today = today;
            self.dirty = true;
            self.labels.clear();
            self.refresh_rows();
        }
    }
    /// Whether an editor holds a draft that is not saved yet (a reminder or list being created or changed).
    pub fn has_draft(&self) -> bool {
        matches!(self.screen, Screen::Editor | Screen::ListEditor)
    }
    pub fn keyboard_shown(&self) -> bool {
        self.kb_on
    }
    // ---- geometry -------------------------------------------------------

    fn top_px(&self, ui: &Ui) -> f32 {
        match self.screen {
            Screen::Items => self.list_top_px(ui),
            // a reminder's details are in a popover over the list (`details.rs`)
            _ => match self.details_geometry(ui) {
                Some(g) => ui.pt(g.panel.y + details::DETAILS_PAD),
                None => ui.pt(self.top_pt + FORM_TOP),
            },
        }
    }
    /// Top of the Undo bar (the bottom of the list while there is none).
    fn bottom(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - if self.undo.is_some() { ui.pt(52.0) } else { 0.0 }
    }
    fn view_bottom(&self, ui: &Ui) -> f32 {
        match self.screen {
            Screen::Items => self.bottom(ui),
            _ => match self.details_geometry(ui) {
                Some(g) => ui.pt(g.panel.y + g.panel.h - details::DETAILS_PAD),
                None => self.h as f32 - home_zone(ui),
            },
        }
    }
    fn content_pt(&self) -> f32 {
        match self.screen {
            Screen::Items => self.items_h,
            _ => self.form_rows().1,
        }
    }
    fn fit_scroll(&mut self, ui: &Ui) {
        let viewport = self.view_bottom(ui) - self.top_px(ui);
        self.scroll.set_max(ui.pt(self.content_pt() + 16.0) - viewport);
    }
    fn row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let h = match self.rows.get(i) {
            Some(Row::Header(_) | Row::Section { .. }) => HEAD_H,
            _ => ITEM_H,
        };
        (
            ui.pt(SPACE_XL),
            self.list_top_px(ui) + ui.pt(self.row_y.get(i).copied().unwrap_or(0.0)) - self.list_offset(),
            self.w as f32 - ui.pt(SPACE_HUGE),
            ui.pt(h),
        )
    }
    fn item_id(&self, row: usize) -> Option<u64> {
        match self.rows.get(row)? {
            Row::Item { idx, .. } => self.data.items.get(*idx).map(|r| r.id),
            Row::Header(_) | Row::Section { .. } => None,
        }
    }
    fn form_rows(&self) -> (Vec<(Target, f32, f32)>, f32) {
        fn add(v: &mut Vec<(Target, f32, f32)>, y: &mut f32, t: Target, h: f32, gap: f32) {
            v.push((t, *y, h));
            *y += h + gap;
        }
        let mut v = vec![];
        let mut y = 0.0;
        match self.screen {
            Screen::Editor => {
                let Some(e) = &self.editor else { return (v, 0.0) };
                add(&mut v, &mut y, Target::Field(Field::Title), 56.0, 8.0);
                add(&mut v, &mut y, Target::Field(Field::Notes), 56.0, 8.0);
                add(&mut v, &mut y, Target::Field(Field::Url), 56.0, 20.0);
                add(&mut v, &mut y, Target::Due, 52.0, if e.draft.due.is_some() { 8.0 } else { 20.0 });
                if e.draft.due.is_none() && !self.tip_sections(now_seconds()).is_empty() {
                    add(&mut v, &mut y, Target::Tip, TIP_H, 20.0);
                }
                if e.draft.due.is_some() {
                    add(&mut v, &mut y, Target::Calendar, 294.0, 8.0);
                    add(&mut v, &mut y, Target::Time, 52.0, 8.0);
                    if e.draft.time.is_some() {
                        add(&mut v, &mut y, Target::TimeStep, 52.0, 8.0);
                    }
                    add(&mut v, &mut y, Target::Repeat, 52.0, 8.0);
                    add(&mut v, &mut y, Target::Early, 52.0, 8.0);
                    add(&mut v, &mut y, Target::TimeZone, 52.0, 8.0);
                    add(&mut v, &mut y, Target::Urgent, 52.0, 20.0);
                }
                add(&mut v, &mut y, Target::Flag, 52.0, 8.0);
                add(&mut v, &mut y, Target::Priority, 52.0, 8.0);
                add(&mut v, &mut y, Target::MoveList, 52.0, 8.0);
                add(&mut v, &mut y, Target::Indent, 52.0, 20.0);
                let pictures = e.draft.attachments.len();
                add(&mut v, &mut y, Target::AddImage, 52.0, if pictures == 0 { 20.0 } else { 8.0 });
                if pictures > 0 {
                    add(&mut v, &mut y, Target::Thumbs, 96.0, 8.0);
                    add(&mut v, &mut y, Target::RemoveImage, 52.0, 20.0);
                }
                if e.id.is_some() {
                    add(&mut v, &mut y, Target::Duplicate, 52.0, 8.0);
                    add(&mut v, &mut y, Target::Delete, 52.0, 8.0);
                }
            }
            Screen::ListEditor => {
                let Some(e) = &self.list_edit else { return (v, 0.0) };
                if e.mode == EditMode::Settings {
                    for (i, row) in SettingRow::ALL.iter().enumerate() {
                        add(&mut v, &mut y, Target::Setting(*row), 52.0, if i + 1 == SettingRow::ALL.len() { 20.0 } else { 8.0 });
                    }
                    return (v, y);
                }
                add(&mut v, &mut y, Target::Field(Field::Title), 56.0, 20.0);
                if e.mode == EditMode::Smart {
                    add(&mut v, &mut y, Target::Colors, 64.0, 20.0);
                    for (i, row) in RuleRow::ALL.iter().enumerate() {
                        add(&mut v, &mut y, Target::Rule(*row), 52.0, if i + 1 == RuleRow::ALL.len() { 20.0 } else { 8.0 });
                    }
                }
                if e.mode == EditMode::List {
                    add(&mut v, &mut y, Target::Colors, 64.0, 8.0);
                    add(&mut v, &mut y, Target::Icons, 64.0, 20.0);
                    let groups = !self.data.groups.is_empty();
                    add(&mut v, &mut y, Target::PinList, 52.0, if groups { 8.0 } else { 20.0 });
                    if groups {
                        add(&mut v, &mut y, Target::ListGroup, 52.0, 20.0);
                    }
                    if e.id.is_some() {
                        add(&mut v, &mut y, Target::ListUp, 52.0, 8.0);
                        add(&mut v, &mut y, Target::ListDown, 52.0, 8.0);
                    }
                }
                if e.id.is_some() && (e.mode != EditMode::List || self.data.lists.len() > 1) {
                    add(&mut v, &mut y, Target::Delete, 52.0, 8.0);
                }
            }
            _ => {}
        }
        (v, y)
    }
    fn form_rect(&self, ui: &Ui, y: f32, h: f32) -> (f32, f32, f32, f32) {
        if let Some(g) = self.details_geometry(ui) {
            let pad = details::DETAILS_PAD;
            return (ui.pt(g.panel.x + pad), self.top_px(ui) + ui.pt(y) - self.scroll.offset(), ui.pt(g.panel.w - 2.0 * pad), ui.pt(h));
        }
        (
            ui.pt(SPACE_XL),
            self.top_px(ui) + ui.pt(y) - self.scroll.offset(),
            self.w as f32 - ui.pt(SPACE_HUGE),
            ui.pt(h),
        )
    }
    fn field_frame(&self, ui: &Ui, f: Field) -> Option<(f32, f32, f32, f32)> {
        match self.screen {
            Screen::Editor | Screen::ListEditor => self
                .form_rows()
                .0
                .iter()
                .find(|(t, _, _)| *t == Target::Field(f))
                .map(|&(_, y, h)| {
                    // inset in its row: in the full form and in the Details popover alike
                    let r = self.form_rect(ui, y, h);
                    (r.0 + ui.pt(SPACE_L), r.1, r.2 - ui.pt(2.0 * SPACE_L), r.3)
                }),
            // the search field is in the toolbar (window coordinates there, content coordinates here)
            Screen::Items if self.searching && f == Field::Title => self.search_px(ui).map(|(x, y, w, h)| (x - self.x0, y, w, h)),
            _ => None,
        }
    }
    fn field(&self, f: Field) -> &TextField {
        match f {
            Field::Title => &self.title,
            Field::Notes => &self.notes,
            Field::Url => &self.url,
        }
    }
    fn field_mut(&mut self, f: Field) -> &mut TextField {
        match f {
            Field::Title => &mut self.title,
            Field::Notes => &mut self.notes,
            Field::Url => &mut self.url,
        }
    }
    pub(super) fn calendar_days(&self) -> (i64, u32, usize) {
        let month = self.editor.as_ref().map_or(self.today, |e| e.month);
        let (year, m, _) = civil_from_days(month);
        let next = if m == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, m + 1, 1) };
        (month, (next - month) as u32, (month + 3).rem_euclid(7) as usize) // Monday first
    }
    fn options_layout(&self, ui: &Ui) -> Vec<(Option<Target>, String, bool, (f32, f32, f32, f32))> {
        let pw = ui.pt(OPTIONS_W_PT);
        let x = self.w as f32 - ui.pt(SPACE_XL) - pw;
        let mut y = ui.pt(self.top_pt + 48.0);
        let mut out = vec![];
        let mut push = |t: Option<Target>, label: &str, on: bool, h: f32, y: &mut f32| {
            out.push((t, label.to_string(), on, (x, *y, pw, ui.pt(h))));
            *y += ui.pt(h);
        };
        push(None, "Sort By", false, 34.0, &mut y);
        for s in Sort::ALL {
            push(Some(Target::OptSort(s)), s.label(), self.opts.sort == s, 44.0, &mut y);
        }
        push(None, "Group By", false, 34.0, &mut y);
        for g in Group::ALL {
            push(Some(Target::OptGroup(g)), g.label(), self.opts.group == g, 44.0, &mut y);
        }
        push(Some(Target::OptCompleted), "Show Completed", self.opts.show_completed, 48.0, &mut y);
        if matches!(self.view, View::List(_)) {
            push(Some(Target::OptClearCompleted), "Clear Completed", false, 48.0, &mut y);
            if self.file_export {
                push(Some(Target::OptExport), "Export List", false, 48.0, &mut y);
            }
        }
        push(None, "New Reminders Go To", false, 34.0, &mut y);
        push(Some(Target::OptDefaultList(None)), "Automatic (first list)", self.opts.default_list.is_none(), 44.0, &mut y);
        for l in &self.data.lists {
            push(Some(Target::OptDefaultList(Some(l.id))), &l.name, self.opts.default_list == Some(l.id), 44.0, &mut y);
        }
        out
    }

    // ---- hit testing ----------------------------------------------------

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        // the Clear and purge alerts block the window; the Details popover is transient: a press outside it closes it
        if self.asking() {
            return self.ask_target(ui, x + self.x0, y);
        }
        if let Some(g) = self.details_geometry(ui) {
            let k = ui.pt(1.0);
            if !g.panel.contains(x / k, y / k) {
                return Some(Target::DetailsClose);
            }
        }
        if let Some(t) = self.bar_target(ui, x + self.x0, y) {
            return Some(t);
        }
        if x < 0.0 {
            return self.sidebar_target(ui, x + self.x0, y);
        }
        let top_bar = y < ui.pt(self.top_pt + bifrost_app::layout::UNIFIED_BAR_PT);
        let wpx = self.w as f32;
        match self.screen {
            Screen::Editor | Screen::ListEditor => {
                if y < self.top_px(ui) || y >= self.view_bottom(ui) {
                    return None;
                }
                for (target, ry, rh) in self.form_rows().0 {
                    let rect = self.form_rect(ui, ry, rh);
                    if within(x, y, rect) {
                        return Some(self.refine(ui, target, rect, x, y));
                    }
                }
                Some(Target::Scroll)
            }
            Screen::Items => {
                if self.options_open {
                    for (t, _, _, rect) in self.options_layout(ui) {
                        if t.is_some() && within(x, y, rect) {
                            return t;
                        }
                    }
                    return Some(Target::OptClose);
                }
                if top_bar {
                    return None;
                }
                // a kept row's Restore link restores it; the rest of the row selects it
                if let Some(kept) = &self.deleted {
                    for (i, k) in kept.iter().enumerate() {
                        let rect = self.deleted_row_rect(ui, i);
                        if within(x, y, rect) {
                            let link = x >= rect.0 + rect.2 - ui.pt(purge::RESTORE_W);
                            return Some(if link { Target::Restore(k.row) } else { Target::KeptRow(k.row) });
                        }
                    }
                    return None;
                }
                if let Some(t) = self.strip_target(ui, x, y) {
                    return Some(t);
                }
                let bar = self.bottom(ui);
                if self.undo.is_some() && y >= bar && y < bar + ui.pt(52.0) {
                    return if x > wpx - ui.pt(UNDO_HIT_PT) { Some(Target::Undo) } else { None };
                }
                if y < self.top_px(ui) || y >= bar {
                    return None;
                }
                for i in 0..self.rows.len() {
                    let rect = self.row_rect(ui, i);
                    if !within(x, y, rect) {
                        continue;
                    }
                    if matches!(self.rows.get(i), Some(Row::Section { .. })) {
                        return Some(Target::Section(i));
                    }
                    let Some(Row::Item { idx, depth }) = self.rows.get(i) else {
                        return Some(Target::Scroll);
                    };
                    let item = &self.data.items[*idx];
                    let right = rect.0 + rect.2;
                    if self.swipe.as_ref().is_some_and(|s| s.open && s.id == item.id) && x >= right - ui.pt(BTN) * 2.0 {
                        return Some(if x < right - ui.pt(BTN) { Target::SwipeFlag(i) } else { Target::SwipeDelete(i) });
                    }
                    if x < rect.0 + ui.pt(52.0 + 30.0 * *depth as f32) {
                        return Some(Target::Circle(i));
                    }
                    // the selected row's "i" at its trailing edge opens the Details popover (`details.rs`)
                    let info = self.info_shown(item.id);
                    if info && x >= right - ui.pt(details::INFO_W) {
                        return Some(Target::Info(i));
                    }
                    let room = if info { details::INFO_W } else { 0.0 };
                    if *depth == 0 && model::has_children(&self.data.items, item.id) && x > right - ui.pt(56.0 + room) {
                        return Some(Target::Collapse(i));
                    }
                    return Some(Target::Row(i));
                }
                Some(Target::Scroll)
            }
        }
    }
    fn refine(&self, ui: &Ui, target: Target, rect: (f32, f32, f32, f32), x: f32, y: f32) -> Target {
        let (rx, ry, rw, _) = rect;
        match target {
            Target::Calendar => {
                if y < ry + ui.pt(48.0) {
                    if x < rx + ui.pt(56.0) {
                        return Target::Month(-1);
                    }
                    if x >= rx + rw - ui.pt(56.0) {
                        return Target::Month(1);
                    }
                }
                let (first, count, start) = self.calendar_days();
                let row = ((y - ry - ui.pt(72.0)) / ui.pt(36.0)).floor() as i32;
                let col = ((x - rx) / (rw / 7.0)).floor() as i32;
                let day = row * 7 + col - start as i32;
                if row >= 0 && (0..7).contains(&col) && day >= 0 && day < count as i32 {
                    Target::Day(first + day as i64)
                } else {
                    Target::Calendar
                }
            }
            Target::TimeStep => {
                // Zones centred this far (pt) from the right edge: M> M< H> H<
                let from_right = (rx + rw - x) / ui.pt(1.0);
                for (centre, t) in [
                    (24.0, Target::Minute(5)),
                    (104.0, Target::Minute(-5)),
                    (156.0, Target::Hour(1)),
                    (236.0, Target::Hour(-1)),
                ] {
                    if (from_right - centre).abs() < 26.0 {
                        return t;
                    }
                }
                Target::TimeStep
            }
            Target::Colors => {
                let i = (((x - rx) / (rw / view::COLORS.len() as f32)).floor() as i32).clamp(0, view::COLORS.len() as i32 - 1);
                Target::Color(i as u8)
            }
            Target::Icons => {
                let i = (((x - rx) / (rw / view::ICONS.len() as f32)).floor() as i32).clamp(0, view::ICONS.len() as i32 - 1);
                Target::Icon(i as u8)
            }
            other => other,
        }
    }

    // ---- state helpers --------------------------------------------------

    fn refresh_rows(&mut self) {
        let query = if self.searching && self.screen == Screen::Items { self.title.text().to_string() } else { String::new() };
        self.rows = model::visible(&self.data, self.view, self.opts, self.today, &query, &self.collapsed);
        self.row_y.clear();
        let mut y = 0.0;
        for row in &self.rows {
            self.row_y.push(y);
            y += if matches!(row, Row::Header(_) | Row::Section { .. }) { HEAD_H } else { ITEM_H };
        }
        self.items_h = y;
    }
    fn rebuild(&mut self, ui: &Ui) {
        self.refresh_rows();
        self.labels.clear();
        self.fit_scroll(ui);
        self.dirty = true;
    }
    /// Publishes a mutation only after its atomic write succeeds. A failed
    /// editor save leaves the draft visible, even if the app goes to Home.
    fn commit(&mut self, next: Data) -> bool {
        if self.load_error {
            self.error = true;
            self.dirty = true;
            return false;
        }
        self.error = store::save(&self.path, &next).is_err();
        if !self.error {
            self.data = next;
            self.gen = self.gen.wrapping_add(1);
        }
        self.dirty = true;
        !self.error
    }
    /// `commit`, and the change goes on the undo history under `name` (Edit > Undo names it). A change that
    /// registers replaces the Undo bar, which always offers the latest change.
    fn commit_named(&mut self, next: Data, name: &str) -> bool {
        let before = self.data.clone();
        let changed = before != next;
        let after = next.clone();
        if !self.commit(next) {
            return false;
        }
        if changed {
            self.undo = None;
            self.history.register(name, snapshot(before), snapshot(after));
        }
        true
    }
    /// Edit > Undo and Redo (Cmd+Z, Cmd+Shift+Z): steps the history back or forward. Only on the list, since an open
    /// editor holds a draft of the data. False when there is no step that way.
    pub fn step_history(&mut self, ui: &Ui, redo: bool) -> bool {
        if self.screen != Screen::Items {
            return false;
        }
        let mut next = self.data.clone();
        let stepped = if redo { self.history.redo(&mut next) } else { self.history.undo(&mut next) };
        if stepped.is_none() {
            return false;
        }
        self.undo = None;
        if self.commit(next) {
            // the undo of a delete takes its reminders back out of Recently Deleted, the redo keeps them again (E2E N4)
            self.refresh_kept();
            self.rebuild(ui);
        }
        true
    }
    /// The Edit menu's Undo and Redo: enabled on the list when the history has a step that way.
    pub fn edit_commands(&self) -> bifrost_app::Commands {
        if self.screen != Screen::Items {
            return bifrost_app::Commands::default();
        }
        bifrost_app::Commands::default()
            .with(bifrost_app::Command::Undo, self.history.can_undo())
            .with(bifrost_app::Command::Redo, self.history.can_redo())
    }
    /// The Edit menu's titles: "Undo Mark as Completed" and so on, plain when there is nothing to name.
    pub fn edit_titles(&self) -> (String, String) {
        (self.history.undo_menu_item_title(), self.history.redo_menu_item_title())
    }
    fn set_focus(&mut self, f: Option<Field>, _t: f64) {
        self.focus = f;
        self.title.focused = f == Some(Field::Title);
        self.notes.focused = f == Some(Field::Notes);
        self.url.focused = f == Some(Field::Url);
        self.kb_on = f.is_some();
    }
    fn set_title_field(&mut self, ui: &Ui, hint: &'static str, max: usize, text: &str) {
        self.title = mk_field(ui, hint, max, text);
        self.title_hint = hint;
        self.title_max = max;
    }
    /// Reads the recently deleted store again: the sidebar count, the id floor and, while it is open, the page.
    fn refresh_kept(&mut self) {
        let kept = archive::kept_rows(&archive::path_beside(&self.path)).unwrap_or_default();
        self.kept_count = kept.len();
        if let Some(floor) = kept.iter().map(|k| k.reminder.id.saturating_add(1)).max() {
            self.id_floor = self.id_floor.max(floor);
        }
        if self.deleted.is_some() {
            self.deleted = Some(kept);
        }
    }

    /// The id for a new reminder in `data`: past every reminder on the list and every kept one.
    fn fresh_item_id(&self, data: &Data) -> Option<u64> {
        data.next_item_id().map(|id| id.max(self.id_floor))
    }

    /// Recently Deleted: shows the reminders kept in the archive store.
    fn open_deleted(&mut self, ui: &Ui) {
        self.deleted = Some(archive::kept_rows(&archive::path_beside(&self.path)).unwrap_or_default());
        self.kept_selected = None;
        self.selected = None;
        self.screen = Screen::Items;
        self.searching = false;
        self.swipe = None;
        self.scroll.jump_top();
        self.rebuild(ui);
    }

    /// Puts a kept reminder back into the reminders, in the same list when that list still exists (else the default
    /// list), and takes it out of the store only once the reminders file has been written. `row` is its store row.
    fn restore_deleted(&mut self, ui: &Ui, row: u64) {
        let path = archive::path_beside(&self.path);
        let Some(mut reminder) = self.deleted.as_ref().and_then(|kept| kept.iter().find(|k| k.row == row).map(|k| k.reminder.clone())) else {
            return;
        };
        let mut next = self.data.clone();
        if !next.lists.iter().any(|l| l.id == reminder.list) {
            reminder.list = model::DEFAULT_LIST;
        }
        if reminder.parent.is_some_and(|p| !next.items.iter().any(|r| r.id == p)) {
            reminder.parent = None;
        }
        if next.items.iter().any(|r| r.id == reminder.id) {
            let Some(fresh) = self.fresh_item_id(&next) else { return };
            reminder.id = fresh;
        }
        next.items.push(reminder);
        if !self.commit_named(next, "Restore Reminder") {
            return;
        }
        let _ = archive::take(&path, row);
        self.refresh_kept();
        self.rebuild(ui);
    }

    /// The rectangle of kept reminder `i` on the Recently Deleted page (the same geometry the page draws).
    fn deleted_row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        (ui.pt(SPACE_XL), self.top_px(ui) + ui.pt(ITEM_H) * i as f32, self.w as f32 - ui.pt(SPACE_HUGE), ui.pt(ITEM_H))
    }

    fn open_view(&mut self, ui: &Ui, view: View) {
        // a selection belongs to the list it was made in (E2E R2)
        if self.view != view || self.deleted.is_some() {
            self.selected = None;
        }
        self.view = view;
        self.deleted = None;
        self.screen = Screen::Items;
        self.searching = false;
        self.swipe = None;
        self.scroll.jump_top();
        self.rebuild(ui);
    }
    fn begin_search(&mut self, ui: &Ui, t: f64) {
        self.view = View::Smart(Smart::All);
        self.screen = Screen::Items;
        self.searching = true;
        self.swipe = None;
        self.set_title_field(ui, "Search", MAX_SEARCH, "");
        self.scroll.jump_top();
        self.rebuild(ui);
        self.set_focus(Some(Field::Title), t);
    }
    /// Leaves the search: the list shows All again.
    fn leave_items(&mut self, ui: &Ui, t: f64) {
        self.searching = false;
        self.view = View::Smart(Smart::All);
        self.set_focus(None, t);
        self.set_title_field(ui, "Reminder", model::MAX_TITLE, "");
        self.screen = Screen::Items;
        self.swipe = None;
        self.options_open = false;
        self.scroll.jump_top();
        self.rebuild(ui);
    }
    fn open_editor(&mut self, ui: &Ui, id: Option<u64>, t: f64) {
        let existing = id.and_then(|i| self.data.items.iter().find(|r| r.id == i)).cloned();
        let draft = existing.unwrap_or_else(|| {
            let mut r = Reminder::new(0, "");
            r.list = match self.view {
                View::List(l) => l,
                View::Smart(_) | View::Tag(_) | View::Custom(_) => self
                    .opts
                    .default_list
                    .filter(|id| self.data.lists.iter().any(|l| l.id == *id))
                    .or_else(|| self.data.lists.first().map(|l| l.id))
                    .unwrap_or(model::DEFAULT_LIST),
            };
            match self.view {
                View::Smart(Smart::Today) => r.due = Some(self.today),
                View::Smart(Smart::Flagged) => r.flagged = true,
                _ => {}
            }
            r
        });
        if self.searching {
            self.saved_query = self.title.text().to_string();
        }
        self.set_title_field(ui, "Reminder", model::MAX_TITLE, &draft.title);
        self.notes = mk_field(ui, "Notes", model::MAX_NOTES, &draft.notes);
        self.url = mk_field(ui, "URL", model::MAX_URL, &draft.url);
        let month = month_start(draft.due.unwrap_or(self.today));
        self.editor = Some(Editor { id: id.filter(|_| draft.id != 0), draft, month, added: vec![] });
        self.list_edit = None;
        // a saved reminder's details open in a popover over the list, which keeps its place underneath
        if self.screen == Screen::Items {
            self.list_scroll = self.scroll.offset();
        }
        self.screen = Screen::Editor;
        self.swipe = None;
        self.scroll.jump_top();
        self.set_focus(if id.is_none() { Some(Field::Title) } else { None }, t);
        self.dirty = true;
    }
    /// Another form is about to replace a reminder's Details: the reminder keeps its edits (saved when its title is
    /// not empty, as Done would) and its form closes, so Done on the new form saves that form and never the reminder
    /// (E2E W2-20: Add List while Details was open renamed the reminder to the list's name).
    fn close_item_editor(&mut self, ui: &Ui, t: f64) {
        if self.editor.is_none() {
            return;
        }
        self.save_editor(ui, t);
        if self.editor.is_some() {
            self.discard_new_pictures();
            self.close_form(ui, t);
        }
    }
    fn open_list_editor(&mut self, ui: &Ui, id: Option<u64>, t: f64) {
        self.close_item_editor(ui, t);
        let found = id.and_then(|i| self.data.lists.iter().find(|l| l.id == i)).cloned();
        let (name, color, icon) = found.as_ref().map_or((String::new(), (self.data.lists.len() % view::COLORS.len()) as u8, 0), |l| (l.name.clone(), l.color, l.icon));
        let (pinned, group) = found.as_ref().map_or((false, None), |l| (l.pinned, l.group));
        self.set_title_field(ui, "List Name", model::MAX_NAME, &name);
        self.list_edit = Some(ListEdit { id: found.map(|l| l.id), color, icon, pinned, group, mode: EditMode::List, rules: Default::default(), confirm_delete: false });
        self.screen = Screen::ListEditor;
        self.scroll.jump_top();
        self.set_focus(if id.is_none() { Some(Field::Title) } else { None }, t);
        self.dirty = true;
    }
    /// The list editor form for a section of `list` (a new one when `id` is `None`): a name and, for an existing
    /// section, Delete.
    fn open_section_editor(&mut self, ui: &Ui, list: u64, id: Option<u64>, t: f64) {
        self.close_item_editor(ui, t);
        let name = id
            .and_then(|sid| self.data.lists.iter().flat_map(|l| l.sections.iter()).find(|s| s.id == sid))
            .map_or(String::new(), |s| s.name.clone());
        self.set_title_field(ui, "Section Name", model::MAX_NAME, &name);
        self.list_edit = Some(ListEdit { id, color: 0, icon: 0, pinned: false, group: None, mode: EditMode::Section(list), rules: Default::default(), confirm_delete: false });
        self.screen = Screen::ListEditor;
        self.scroll.jump_top();
        self.set_focus(if id.is_none() { Some(Field::Title) } else { None }, t);
        self.dirty = true;
    }
    /// The list editor form for a list group (a new one when `id` is `None`).
    fn open_group_editor(&mut self, ui: &Ui, id: Option<u64>, t: f64) {
        self.close_item_editor(ui, t);
        let name = id.and_then(|gid| self.data.groups.iter().find(|g| g.id == gid)).map_or(String::new(), |g| g.name.clone());
        self.set_title_field(ui, "Group Name", model::MAX_NAME, &name);
        self.list_edit = Some(ListEdit { id, color: 0, icon: 0, pinned: false, group: None, mode: EditMode::Group, rules: Default::default(), confirm_delete: false });
        self.screen = Screen::ListEditor;
        self.scroll.jump_top();
        self.set_focus(if id.is_none() { Some(Field::Title) } else { None }, t);
        self.dirty = true;
    }
    /// Reminders > Settings: the form with the default list, the all-day alert time and the badge switch.
    pub fn open_settings(&mut self, t: f64) -> bool {
        if self.has_draft() {
            return false;
        }
        self.list_edit = Some(ListEdit { id: None, color: 0, icon: 0, pinned: false, group: None, mode: EditMode::Settings, rules: Default::default(), confirm_delete: false });
        self.screen = Screen::ListEditor;
        self.searching = false;
        self.options_open = false;
        self.scroll.jump_top();
        self.set_focus(None, t);
        self.dirty = true;
        true
    }

    /// Whether the Settings form is open.
    fn settings_open(&self) -> bool {
        self.list_edit.as_ref().is_some_and(|e| e.mode == EditMode::Settings)
    }

    /// Writes the settings and tells the alert planner and the widget that they changed.
    fn save_settings(&mut self) {
        if let Err(e) = settings::save(&self.settings_path, &self.settings) {
            eprintln!("springboard: reminders: settings: {e}");
            self.notice = Some("Could not save the settings".to_string());
        }
        self.gen = self.gen.wrapping_add(1);
        self.dirty = true;
    }

    /// The time a reminder with a day and no time alerts, in minutes after midnight (Settings).
    pub fn all_day_min(&self) -> u16 {
        self.settings.all_day_min
    }

    /// The number on the Dock badge: the due reminders, or nothing when Settings turned the badge off.
    pub fn badge_count(&self) -> usize {
        if self.settings.badge {
            self.due_count()
        } else {
            0
        }
    }

    /// The form for a custom smart list (a new one when `id` is `None`).
    fn open_smart_editor(&mut self, ui: &Ui, id: Option<u64>, t: f64) {
        self.close_item_editor(ui, t);
        let found = id.and_then(|sid| self.data.smarts.iter().find(|m| m.id == sid)).cloned();
        let color = found.as_ref().map_or((self.data.smarts.len() % view::COLORS.len()) as u8, |m| m.color);
        let name = found.as_ref().map_or(String::new(), |m| m.name.clone());
        let rules = found.as_ref().map_or_else(Default::default, |m| m.rules);
        self.set_title_field(ui, "Smart List Name", model::MAX_NAME, &name);
        self.list_edit = Some(ListEdit { id: found.map(|m| m.id), color, icon: 0, pinned: false, group: None, mode: EditMode::Smart, rules, confirm_delete: false });
        self.screen = Screen::ListEditor;
        self.scroll.jump_top();
        self.set_focus(if id.is_none() { Some(Field::Title) } else { None }, t);
        self.dirty = true;
    }
    /// The toolbar title of the list editor form.
    fn list_edit_title(&self) -> &'static str {
        match self.list_edit.as_ref().map(|e| (e.mode, e.id.is_some())) {
            Some((EditMode::List, true)) => "List Info",
            Some((EditMode::Section(_), true)) => "Section",
            Some((EditMode::Section(_), false)) => "New Section",
            Some((EditMode::Group, true)) => "Group",
            Some((EditMode::Group, false)) => "New Group",
            Some((EditMode::Smart, true)) => "Smart List",
            Some((EditMode::Smart, false)) => "New Smart List",
            Some((EditMode::Settings, _)) => "Settings",
            Some((EditMode::List, false)) | None => "New List",
        }
    }
    fn close_form(&mut self, ui: &Ui, t: f64) {
        let popover = self.details_popover();
        let back_to = Screen::Items;
        self.set_focus(None, t);
        self.editor = None;
        self.list_edit = None;
        self.screen = back_to;
        if self.searching && back_to == Screen::Items {
            let q = std::mem::take(&mut self.saved_query);
            self.set_title_field(ui, "Search", MAX_SEARCH, &q);
        } else {
            self.set_title_field(ui, "Reminder", model::MAX_TITLE, "");
        }
        self.scroll.jump_top();
        self.rebuild(ui);
        // the list under a Details popover comes back where it was
        if popover {
            self.scroll.jump_to(self.list_scroll);
        }
    }
    fn now_min(&self, t: f64) -> u16 {
        ((self.wall.now(t) + self.wall.offset as i64).rem_euclid(86_400) / 60) as u16
    }
    fn show_undo(&mut self, label: String, t: f64) {
        self.undo = Some(UndoBar { label, until: t + UNDO_MS });
    }
    fn delete_item(&mut self, id: u64, t: f64) -> bool {
        let title = self.data.items.iter().find(|r| r.id == id).map(|r| clean(&r.title));
        let mut next = self.data.clone();
        let removed = model::remove_with_children(&mut next.items, id);
        if removed.is_empty() || !self.commit(next) {
            return false;
        }
        // kept in the recently deleted store too, so the deletion survives a restart; a store that cannot be
        // written does not undo the deletion
        let kept_path = archive::path_beside(&self.path);
        let kept: Vec<Reminder> = removed.iter().map(|(_, r)| r.clone()).collect();
        let _ = archive::keep(&kept_path, &kept);
        self.refresh_kept();
        let label = format!("Deleted \"{}\"", title.unwrap_or_default());
        // the undo puts the reminders back and takes them out of Recently Deleted; the redo deletes and keeps them
        // again (E2E N4)
        let restore = restore_items(removed);
        let undo_path = kept_path.clone();
        self.history.register(
            &label,
            move |d| {
                restore(d);
                let _ = archive::take_back(&undo_path, &kept);
            },
            move |d| {
                let again: Vec<Reminder> = model::remove_with_children(&mut d.items, id).into_iter().map(|(_, r)| r).collect();
                let _ = archive::keep(&kept_path, &again);
            },
        );
        self.show_undo(label, t);
        true
    }
    /// Copies a reminder (title, notes, URL, priority, flag, due, repeat, alert) directly after the
    /// original, with the same list and parent. The copy starts incomplete and the editor closes.
    fn duplicate_item(&mut self, ui: &Ui, id: u64, t: f64) {
        let mut next = self.data.clone();
        let Some(at) = next.items.iter().position(|r| r.id == id) else { return };
        let Some(new_id) = self.fresh_item_id(&next) else { return };
        let mut copy = next.items[at].clone();
        copy.id = new_id;
        copy.completed = false;
        copy.created = Some(self.wall.unix);
        next.items.insert(at + 1, copy);
        if self.commit_named(next, "Duplicate Reminder") {
            self.close_form(ui, t);
        }
    }
    /// Removes every completed reminder in the open list. The undo bar restores them.
    fn clear_completed(&mut self, ui: &Ui, t: f64) {
        let View::List(list) = self.view else { return };
        let mut next = self.data.clone();
        let removed = model::remove_completed(&mut next.items, list);
        if removed.is_empty() {
            return;
        }
        let count = removed.iter().filter(|(_, r)| r.parent.is_none()).count();
        if self.commit(next) {
            let noun = if count == 1 { "reminder" } else { "reminders" };
            let label = format!("Cleared {count} completed {noun}");
            self.history.register(&label, restore_items(removed), move |d| {
                model::remove_completed(&mut d.items, list);
            });
            self.show_undo(label, t);
            self.rebuild(ui);
        }
    }
    /// Arrow keys move the row selection, Enter opens the selected reminder and Space toggles it.
    /// Only on the item list with no field focused.
    fn list_key(&mut self, ui: &Ui, out: Out, t: f64) {
        if self.screen != Screen::Items {
            return;
        }
        // on Recently Deleted the arrows move through the kept reminders, never the list hidden under the page
        if let Some(kept) = &self.deleted {
            let ids: Vec<u64> = kept.iter().map(|k| k.row).collect();
            let at = self.kept_selected.and_then(|id| ids.iter().position(|&i| i == id));
            match out {
                Out::Named("Down") => self.kept_selected = ids.get(at.map_or(0, |i| (i + 1).min(ids.len().saturating_sub(1)))).copied(),
                Out::Named("Up") => self.kept_selected = ids.get(at.map_or(0, |i| i.saturating_sub(1))).copied(),
                _ => {}
            }
            self.dirty = true;
            return;
        }
        let ids: Vec<u64> = self
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Item { idx, .. } => Some(self.data.items[*idx].id),
                Row::Header(_) | Row::Section { .. } => None,
            })
            .collect();
        let at = self.selected.and_then(|id| ids.iter().position(|&i| i == id));
        let chosen = self.selected.filter(|id| ids.contains(id));
        match out {
            Out::Named("Down") => {
                let next = at.map_or(0, |i| (i + 1).min(ids.len().saturating_sub(1)));
                self.selected = ids.get(next).copied();
            }
            Out::Named("Up") => {
                let next = at.map_or(0, |i| i.saturating_sub(1));
                self.selected = ids.get(next).copied();
            }
            Out::Enter => {
                if let Some(id) = chosen {
                    self.open_editor(ui, Some(id), t);
                }
            }
            Out::Text(ref s) if s == " " => {
                if let Some(id) = chosen {
                    self.toggle_item(ui, id);
                }
            }
            // Delete and Backspace delete the selected reminder, as Cmd+Backspace does (the undo bar offers it back)
            Out::Backspace | Out::Named("Delete") => {
                if let Some(id) = chosen {
                    if self.delete_item(id, t) {
                        self.rebuild(ui);
                    }
                }
            }
            _ => {}
        }
        self.dirty = true;
    }
    fn toggle_item(&mut self, ui: &Ui, id: u64) {
        let mut next = self.data.clone();
        if model::toggle_complete(&mut next.items, id, self.today).is_some() {
            let name = completion_name(&next.items, id);
            if self.commit_named(next, name) {
                self.rebuild(ui);
            }
        }
    }
    /// Swipe-right completion. A completed reminder stays completed; the circle is the way back.
    fn complete_swiped(&mut self, ui: &Ui, id: u64) {
        if self.data.items.iter().any(|r| r.id == id && r.completed) {
            return;
        }
        let mut next = self.data.clone();
        if model::toggle_complete(&mut next.items, id, self.today).is_some() {
            let name = completion_name(&next.items, id);
            if self.commit_named(next, name) {
                self.rebuild(ui);
            }
        }
    }
    fn toggle_flag(&mut self, id: u64) -> bool {
        let mut next = self.data.clone();
        let Some(r) = next.items.iter_mut().find(|r| r.id == id) else { return false };
        r.flagged = !r.flagged;
        self.commit(next)
    }
    /// The Undo bar's button: the same step as Edit > Undo.
    fn run_undo(&mut self, ui: &Ui) {
        if self.undo.is_some() {
            self.step_history(ui, false);
        }
    }
    fn save_editor(&mut self, ui: &Ui, t: f64) {
        let Some(e) = &self.editor else { return };
        let mut title = clean(self.title.text());
        if title.is_empty() {
            return;
        }
        let mut d = e.draft.clone();
        let id = e.id;
        d.notes = self.notes.text().trim().to_string();
        d.url = self.url.text().trim().to_string();
        if id.is_none() {
            let q = quick::parse(&title, self.today, self.now_min(t));
            title = q.title;
            d.due = d.due.or(q.due);
            d.time = d.time.or(q.time);
            d.repeat = d.repeat.or(q.repeat);
            d.priority = d.priority.max(q.priority);
            d.created = Some(self.wall.now(t));
        }
        d.title = title;
        let mut next = self.data.clone();
        match id {
            Some(id) => {
                let Some(pos) = next.items.iter().position(|r| r.id == id) else { return };
                let list_changed = next.items[pos].list != d.list;
                if model::has_children(&next.items, id) {
                    d.parent = None;
                    for kid in next.items.iter_mut().filter(|r| r.parent == Some(id)) {
                        kid.list = d.list;
                        if list_changed {
                            kid.section = None;
                        }
                    }
                }
                if list_changed {
                    d.parent = None;
                    d.section = None;
                }
                d.id = id;
                next.items[pos] = d;
            }
            None => {
                let Some(new_id) = self.fresh_item_id(&next) else { return };
                d.id = new_id;
                next.items.push(d);
            }
        }
        let name = if id.is_some() { "Edit Reminder" } else { "New Reminder" };
        let before: Vec<String> = id.and_then(|i| self.data.items.iter().find(|r| r.id == i)).map(|r| r.attachments.clone()).unwrap_or_default();
        if self.commit_named(next, name) {
            // a picture the reminder no longer lists is deleted unless another reminder (a duplicate) still uses it
            for gone in before {
                if !self.data.items.iter().any(|r| r.attachments.contains(&gone)) {
                    attach::remove(&self.attach_dir, &gone);
                }
            }
            if let Some(e) = &mut self.editor {
                e.added.clear();
            }
            self.close_form(ui, t);
        }
    }
    fn save_list(&mut self, ui: &Ui, t: f64) {
        let Some(e) = &self.list_edit else { return };
        if e.mode == EditMode::Settings {
            // each setting was saved as it changed; Done closes the form
            self.close_form(ui, t);
            return;
        }
        let Some(name) = organize::clean_name(self.title.text()) else { return };
        let (id, color, icon, pinned, group, mode, rules) = (e.id, e.color, e.icon, e.pinned, e.group, e.mode, e.rules);
        let label = match (mode, id.is_some()) {
            (EditMode::List, true) => "Edit List",
            (EditMode::List, false) => "New List",
            (EditMode::Section(_), true) => "Rename Section",
            (EditMode::Section(_), false) => "New Section",
            (EditMode::Group, true) => "Rename Group",
            (EditMode::Group, false) => "New List Group",
            (EditMode::Smart, true) => "Edit Smart List",
            (EditMode::Smart, false) => "New Smart List",
            (EditMode::Settings, _) => "Settings",
        };
        let mut next = self.data.clone();
        let mut opened = None;
        let done = match mode {
            EditMode::Settings => false,
            EditMode::Smart => match id {
                Some(id) => match next.smarts.iter_mut().find(|m| m.id == id) {
                    Some(m) => {
                        m.name = name;
                        m.color = color;
                        m.rules = rules;
                        true
                    }
                    None => false,
                },
                None => match next.next_smart_id() {
                    Some(sid) if next.smarts.len() < model::MAX_SMARTS => {
                        next.smarts.push(model::SmartDef { id: sid, name, color, rules });
                        opened = Some(sid);
                        true
                    }
                    _ => false,
                },
            },
            EditMode::List => {
                // The sidebar has room for a few pinned tiles; a list over the limit saves unpinned.
                let others = next.lists.iter().filter(|l| l.pinned && Some(l.id) != id).count();
                let pinned = pinned && others < MAX_PINNED;
                match id {
                    Some(id) => match next.lists.iter_mut().find(|l| l.id == id) {
                        Some(l) => {
                            l.name = name;
                            l.color = color;
                            l.icon = icon;
                            l.pinned = pinned;
                            l.group = group;
                            true
                        }
                        None => false,
                    },
                    None => match next.next_list_id() {
                        Some(new_id) => {
                            let mut l = ListDef::new(new_id, name, color, icon);
                            l.pinned = pinned;
                            l.group = group;
                            next.lists.push(l);
                            true
                        }
                        None => false,
                    },
                }
            }
            EditMode::Section(list) => match id {
                Some(id) => organize::rename_section(&mut next, id, &name),
                None => organize::add_section(&mut next, list, &name).is_some(),
            },
            EditMode::Group => match id {
                Some(id) => match next.groups.iter_mut().find(|g| g.id == id) {
                    Some(g) => {
                        g.name = name;
                        true
                    }
                    None => false,
                },
                None => match next.next_group_id() {
                    Some(gid) => {
                        next.groups.push(ListGroup { id: gid, name });
                        true
                    }
                    None => false,
                },
            },
        };
        if done && self.commit_named(next, label) {
            if let Some(sid) = opened {
                self.view = View::Custom(sid);
            }
            self.close_form(ui, t);
        }
    }
    fn finish_drag(&mut self, ui: &Ui) {
        let Some(drag) = self.drag.take() else { return };
        // Where the reminder was dropped: the section whose header was passed last (none above the first header),
        // and the first reminder of that section below the drop point.
        let mut section: Option<u64> = None;
        let mut before = None;
        for i in 0..self.rows.len() {
            let rect = self.row_rect(ui, i);
            let below = rect.1 + rect.3 / 2.0 > drag.y;
            match self.rows.get(i) {
                // dropped above a section header: the reminder ends the section before it
                Some(Row::Section { .. }) if below => break,
                Some(Row::Section { id, .. }) => section = Some(*id),
                Some(Row::Item { idx, depth: 0 }) => {
                    let id = self.data.items[*idx].id;
                    if id != drag.id && below {
                        before = Some(id);
                        break;
                    }
                }
                _ => {}
            }
        }
        let mut next = self.data.clone();
        if organize::place_in_section(&mut next.items, drag.id, section, before) && self.commit_named(next, "Move Reminder") {
            self.rebuild(ui);
        }
    }
    fn can_reorder(&self) -> bool {
        self.screen == Screen::Items
            && !self.searching
            && matches!(self.view, View::List(_))
            && self.opts.sort == Sort::Manual
            && self.opts.group == Group::None
    }

    /// Desktop chords (Cmd or Ctrl; evdev keys): N new reminder (Shift: new list) and F search on
    /// the list, Return saves the open editor, Z undoes the last delete. False when unused here.
    pub fn chord(&mut self, ui: &Ui, key: u32, mods: u32, t: f64) -> bool {
        use bifrost_proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
        if mods & (CHORD_CTRL | CHORD_META) == 0 || mods & CHORD_ALT != 0 || self.asking() {
            return false;
        }
        let shift = mods & CHORD_SHIFT != 0;
        if key == 14 {
            // Cmd+Backspace deletes the arrow-selected reminder on screen; on Recently Deleted it asks before the
            // selected kept reminder goes for good
            if self.screen != Screen::Items {
                return false;
            }
            if self.deleted.is_some() {
                return self.purge_selected();
            }
            let Some(id) = self.selected_item() else { return false };
            if self.delete_item(id, t) {
                self.rebuild(ui);
            }
            return true;
        }
        let target = match (self.screen, key) {
            (Screen::Items, 49) if shift => Target::NewList,
            (Screen::Items, 49) => Target::New,
            (Screen::Items, 33) if !shift => Target::Search,
            (Screen::Editor, 28) => Target::Save,
            (_, 44) if !shift && self.undo.is_some() => Target::Undo,
            _ => return false,
        };
        self.activate(ui, target, t);
        true
    }

    /// A key press for the Clear or purge alert (`ask_key`): `None` when no alert is up, so the key is not its.
    pub fn alert_key(&mut self, ui: &Ui, key: &bifrost_app::KeyInput, t: f64) -> Option<bool> {
        self.asking().then(|| self.ask_key(ui, key, t))
    }

    /// Escape: leaves a focused field first, then cancels the open editor.
    pub fn escape(&mut self, ui: &Ui, t: f64) -> bool {
        // Escape is the Clear and purge alerts' Cancel
        if self.asking() {
            self.activate(ui, Target::ClearCancel, t);
            return true;
        }
        // Escape closes the Details popover whatever field is focused in it, dropping its edits, as it closes a popover
        if self.details_popover() {
            self.activate(ui, Target::Cancel, t);
            return true;
        }
        if self.focus.is_some() {
            self.set_focus(None, t);
            return true;
        }
        if self.searching && self.screen == Screen::Items {
            self.leave_items(ui, t);
            return true;
        }
        match self.screen {
            Screen::Editor | Screen::ListEditor => {
                self.activate(ui, Target::Cancel, t);
                true
            }
            _ => false,
        }
    }

    /// Desktop builds turn on Export List.
    pub fn enable_file_export(&mut self) {
        self.file_export = true;
    }

    /// Writes the open list as `<name>.md` under ~/Documents/Reminders Export.
    /// The name and Markdown checklist of the list on screen, as Export List and Save List As write them.
    pub fn list_export(&self) -> Option<(String, String)> {
        let View::List(list) = self.view else { return None };
        let name = self.data.lists.iter().find(|l| l.id == list).map_or("Reminders".to_string(), |l| l.name.clone());
        Some((name, model::export_markdown(&self.data, list)))
    }

    fn export_list(&mut self) {
        let Some((name, text)) = self.list_export() else { return };
        let dir = PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Documents").join("Reminders Export");
        let written = std::fs::create_dir_all(&dir).and_then(|_| springboard::shell::fsutil::write_atomic(&dir.join(format!("{name}.md")), &text));
        self.notice = Some(match written {
            Ok(()) => format!("Exported to Documents/Reminders Export/{name}.md"),
            Err(e) => {
                eprintln!("springboard: reminders: {e}");
                "Could not export the list".to_string()
            }
        });
    }

    /// The rows of the menu of `view` (a list or a custom smart list); none for the built-in smart lists and tags.
    fn list_menu(&self, view: View) -> Option<Vec<bifrost_proto::ContextItem>> {
        use bifrost_proto::ContextItem as Item;
        match view {
            View::List(id) => {
                let pinned = self.data.lists.iter().any(|l| l.id == id && l.pinned);
                Some(vec![
                    Item::new(CTX_LIST_INFO, "Show List Info"),
                    Item::new(CTX_PIN, if pinned { "Unpin List" } else { "Pin List" }),
                    Item::new(CTX_NEW_SECTION, "New Section"),
                    Item::separator(),
                    Item::new(CTX_DELETE_LIST, "Delete List...").enabled(self.data.lists.len() > 1),
                ])
            }
            View::Custom(_) => Some(vec![Item::new(CTX_LIST_INFO, "Show List Info"), Item::separator(), Item::new(CTX_DELETE_LIST, "Delete Smart List")]),
            View::Smart(_) | View::Tag(_) => None,
        }
    }

    /// A right click at window pixel (x, y): the menu of the list or custom smart list under it in the sidebar, if any.
    pub fn context_menu_at(&mut self, ui: &Ui, x: f32, y: f32) -> Option<Vec<bifrost_proto::ContextItem>> {
        if self.has_draft() {
            return None;
        }
        self.layout_columns(ui);
        let view = match self.hit(ui, x - self.x0, y)? {
            // a kept reminder on Recently Deleted: the right click selects it, then its menu shows
            Target::KeptRow(id) | Target::Restore(id) => {
                self.kept_selected = Some(id);
                self.menu_view = None;
                self.menu_kept = true;
                self.dirty = true;
                return Some(self.kept_menu());
            }
            Target::List(id) | Target::ListInfo(id) => View::List(id),
            Target::Custom(id) | Target::SmartInfo(id) => View::Custom(id),
            _ => return None,
        };
        let items = self.list_menu(view)?;
        self.menu_view = Some(view);
        self.menu_kept = false;
        Some(items)
    }

    /// A menu the app asked for (the toolbar's "..."), for the shell to show; taken once.
    pub fn take_menu_request(&mut self) -> Option<(f32, f32, Vec<bifrost_proto::ContextItem>)> {
        self.menu_request.take()
    }

    /// List > Delete List...: the open list can go when another list is left to show. Like the sidebar menu's row.
    pub fn can_delete_list(&self) -> bool {
        self.open_list().is_some() && self.data.lists.len() > 1 && !self.has_draft()
    }

    /// List > Delete List...: opens the open list's form with Delete pressed once, so it asks before the list and its
    /// reminders go (the second press, in the form, deletes).
    pub fn delete_open_list(&mut self, ui: &Ui, t: f64) -> bool {
        let Some(id) = self.open_list().filter(|_| self.can_delete_list()) else { return false };
        self.activate(ui, Target::ListInfo(id), t);
        self.activate(ui, Target::Delete, t);
        self.dirty = true;
        true
    }

    /// Runs the row `id` chosen from a list's menu. True when something changed.
    pub fn context_action(&mut self, ui: &Ui, id: u32, t: f64) -> bool {
        if std::mem::take(&mut self.menu_kept) {
            let changed = self.deleted.is_some() && self.kept_action(ui, id);
            self.dirty = true;
            return changed;
        }
        let Some(view) = self.menu_view.take() else { return false };
        if self.has_draft() && id != CTX_LIST_INFO {
            return false;
        }
        match (id, view) {
            (CTX_LIST_INFO, View::List(l)) => self.activate(ui, Target::ListInfo(l), t),
            (CTX_LIST_INFO, View::Custom(c)) => self.activate(ui, Target::SmartInfo(c), t),
            (CTX_PIN, View::List(l)) => {
                self.open_view(ui, View::List(l));
                self.toggle_pin();
            }
            (CTX_NEW_SECTION, View::List(l)) => {
                self.open_view(ui, View::List(l));
                self.new_section(ui, t);
            }
            // the list's form opens with Delete pressed once: it asks before the list and its reminders go
            (CTX_DELETE_LIST, View::List(l)) => {
                self.activate(ui, Target::ListInfo(l), t);
                self.activate(ui, Target::Delete, t);
            }
            (CTX_DELETE_LIST, View::Custom(c)) => {
                self.activate(ui, Target::SmartInfo(c), t);
                self.activate(ui, Target::Delete, t);
            }
            _ => return false,
        }
        self.dirty = true;
        true
    }

    fn activate(&mut self, ui: &Ui, target: Target, t: f64) {
        if !matches!(target, Target::OptExport) {
            self.notice = None;
        }
        match target {
            Target::OptExport => self.export_list(),
            Target::OptDefaultList(id) => {
                self.opts.default_list = id;
                self.settings.default_list = id;
                self.save_settings();
            }
            Target::Setting(row) => {
                match row {
                    SettingRow::DefaultList => {
                        // Automatic, then each list in turn, then Automatic again
                        let ids: Vec<u64> = self.data.lists.iter().map(|l| l.id).collect();
                        let next = match self.settings.default_list {
                            None => ids.first().copied(),
                            Some(id) => ids.iter().position(|x| *x == id).and_then(|i| ids.get(i + 1)).copied(),
                        };
                        self.settings.default_list = next;
                        self.opts.default_list = next;
                    }
                    SettingRow::AllDayAlert => self.settings.all_day_min = settings::next_all_day(self.settings.all_day_min),
                    SettingRow::Badge => self.settings.badge = !self.settings.badge,
                }
                self.save_settings();
            }
            Target::Smart(s) => self.open_view(ui, View::Smart(s)),
            Target::Deleted => self.open_deleted(ui),
            Target::Restore(id) => self.restore_deleted(ui, id),
            Target::KeptRow(id) => self.kept_selected = Some(id),
            Target::PurgeConfirm => self.purge_confirmed(ui),
            Target::List(id) => self.open_view(ui, View::List(id)),
            Target::Tag(key) => self.open_view(ui, View::Tag(key)),
            Target::Custom(id) => self.open_view(ui, View::Custom(id)),
            Target::SmartInfo(id) => self.open_smart_editor(ui, Some(id), t),
            Target::Rule(row) => {
                let (items, lists) = (&self.data.items, &self.data.lists);
                if let Some(e) = &mut self.list_edit {
                    match row {
                        RuleRow::Due => e.rules.due = e.rules.due.next(),
                        RuleRow::Time => e.rules.time = e.rules.time.next(),
                        RuleRow::Priority => e.rules.priority = (e.rules.priority + 1) % 4,
                        RuleRow::Flagged => e.rules.flagged = e.rules.flagged.next(),
                        RuleRow::Tag => e.rules.tag = model::next_tag(e.rules.tag, items),
                        RuleRow::List => e.rules.list = model::next_list(e.rules.list, lists),
                    }
                }
            }
            Target::Section(row) => {
                if let (Some(Row::Section { id, .. }), View::List(list)) = (self.rows.get(row).cloned(), self.view) {
                    self.open_section_editor(ui, list, Some(id), t);
                }
            }
            Target::ToggleGroup(id) => {
                if !self.collapsed_groups.remove(&id) {
                    self.collapsed_groups.insert(id);
                }
            }
            Target::PinList => {
                if let Some(e) = &mut self.list_edit {
                    e.pinned = !e.pinned;
                }
            }
            Target::ListGroup => {
                let ids: Vec<u64> = self.data.groups.iter().map(|g| g.id).collect();
                if let Some(e) = &mut self.list_edit {
                    e.group = match e.group {
                        None => ids.first().copied(),
                        Some(g) => ids.iter().position(|x| *x == g).and_then(|i| ids.get(i + 1)).copied(),
                    };
                }
            }
            Target::ListInfo(id) => self.open_list_editor(ui, Some(id), t),
            Target::ListMenu(x, y) if self.deleted.is_some() => {
                self.menu_view = None;
                self.menu_kept = true;
                self.menu_request = Some((x, y, self.kept_menu()));
            }
            Target::ListMenu(x, y) => {
                if let Some(items) = self.list_menu(self.view) {
                    self.menu_view = Some(self.view);
                    self.menu_kept = false;
                    self.menu_request = Some((x, y, items));
                }
            }
            Target::NewList => self.open_list_editor(ui, None, t),
            Target::Search => self.begin_search(ui, t),
            Target::Sidebar => {
                self.side_hidden = !self.side_hidden;
                self.layout_columns(ui);
                self.rebuild(ui);
            }
            Target::Options => self.options_open = !self.options_open,
            Target::OptClose => self.options_open = false,
            Target::OptSort(s) => self.set_sort(ui, s),
            Target::OptGroup(g) => {
                self.opts.group = g;
                self.rebuild(ui);
            }
            Target::OptCompleted => {
                self.opts.show_completed = !self.opts.show_completed;
                self.rebuild(ui);
            }
            Target::New => self.open_editor(ui, None, t),
            Target::Info(row) => {
                if let Some(id) = self.item_id(row) {
                    self.selected = Some(id);
                    self.open_editor(ui, Some(id), t);
                }
            }
            Target::DetailsClose => self.close_details(ui, t),
            Target::ClearAsk => {
                self.clear_ask = self.completed_in_list() > 0;
                self.ask_focus = None;
            }
            Target::ClearConfirm => {
                self.clear_ask = false;
                self.clear_completed(ui, t);
            }
            Target::ClearCancel => {
                self.clear_ask = false;
                self.purge_ask = None;
            }
            Target::Row(row) => {
                if let Some(id) = self.item_id(row) {
                    self.open_editor(ui, Some(id), t);
                }
            }
            Target::Circle(row) => {
                if let Some(id) = self.item_id(row) {
                    self.toggle_item(ui, id);
                }
            }
            Target::Collapse(row) => {
                if let Some(id) = self.item_id(row) {
                    if !self.collapsed.remove(&id) {
                        self.collapsed.insert(id);
                    }
                    self.save_collapsed();
                    self.rebuild(ui);
                }
            }
            Target::SwipeFlag(row) => {
                if let Some(id) = self.item_id(row) {
                    self.swipe = None;
                    if self.toggle_flag(id) {
                        let (title, flagged) = self.data.items.iter().find(|r| r.id == id).map(|r| (clean(&r.title), r.flagged)).unwrap_or_default();
                        let verb = if flagged { "Flagged" } else { "Unflagged" };
                        let label = format!("{verb} \"{title}\"");
                        self.history.register(&label, flag_toggle(id), flag_toggle(id));
                        self.show_undo(label, t);
                        self.rebuild(ui);
                    }
                }
            }
            Target::SwipeDelete(row) => {
                if let Some(id) = self.item_id(row) {
                    self.swipe = None;
                    if self.delete_item(id, t) {
                        self.rebuild(ui);
                    }
                }
            }
            Target::Undo => self.run_undo(ui),
            Target::Field(f) => self.set_focus(Some(f), t),
            Target::Cancel => {
                self.discard_new_pictures();
                self.close_form(ui, t);
            }
            Target::Save => {
                if self.editor.is_some() {
                    self.save_editor(ui, t);
                } else {
                    self.save_list(ui, t);
                }
            }
            Target::Tip => self.dismiss_tip(),
            Target::Due => {
                // Setting a date is what the tip asks for, so it does not show again.
                self.dismiss_tip();
                let today = self.today;
                if let Some(e) = &mut self.editor {
                    if e.draft.due.is_some() {
                        e.draft.due = None;
                        e.draft.time = None;
                        e.draft.repeat = None;
                        e.draft.early = None;
                        e.draft.tz = None;
                        e.draft.urgent = false;
                    } else {
                        e.draft.due = Some(today);
                    }
                    e.month = month_start(e.draft.due.unwrap_or(today));
                }
                self.set_focus(None, t);
            }
            Target::Month(delta) => {
                if let Some(e) = &mut self.editor {
                    let (year, month, _) = civil_from_days(e.month);
                    let value = year * 12 + month as i64 - 1 + delta as i64;
                    if (12..120_000).contains(&value) {
                        e.month = days_from_civil(value.div_euclid(12), value.rem_euclid(12) as u32 + 1, 1);
                    }
                }
            }
            Target::Day(day) => {
                if let Some(e) = &mut self.editor {
                    e.draft.due = Some(day);
                }
            }
            Target::Time => {
                if let Some(e) = &mut self.editor {
                    e.draft.time = if e.draft.time.is_some() { None } else { Some(model::DEFAULT_ALERT_MIN) };
                }
                self.set_focus(None, t);
            }
            Target::Hour(d) | Target::Minute(d) => {
                if let Some(e) = &mut self.editor {
                    if let Some(time) = e.draft.time {
                        let step = if matches!(target, Target::Hour(_)) { 60 * d } else { d };
                        e.draft.time = Some((time as i32 + step).rem_euclid(1440) as u16);
                    }
                }
            }
            Target::Repeat => {
                if let Some(e) = &mut self.editor {
                    e.draft.repeat = repeat::Repeat::cycle(e.draft.repeat);
                }
            }
            Target::Early => {
                if let Some(e) = &mut self.editor {
                    e.draft.early = model::cycle_early(e.draft.early);
                }
            }
            Target::TimeZone => {
                if let Some(e) = &mut self.editor {
                    e.draft.tz = zones::next_choice(e.draft.tz.as_deref());
                }
            }
            Target::AddImage => {
                if self.editor.as_ref().is_some_and(|e| e.draft.attachments.len() >= attach::MAX_PER_REMINDER) {
                    self.notice = Some(format!("A reminder holds up to {} pictures", attach::MAX_PER_REMINDER));
                } else {
                    self.image_request = true;
                }
            }
            Target::RemoveImage => {
                if let Some(e) = &mut self.editor {
                    e.draft.attachments.pop();
                }
            }
            Target::Urgent => {
                if let Some(e) = &mut self.editor {
                    e.draft.urgent = !e.draft.urgent;
                }
            }
            Target::Flag => {
                if let Some(e) = &mut self.editor {
                    e.draft.flagged = !e.draft.flagged;
                }
            }
            Target::Priority => {
                if let Some(e) = &mut self.editor {
                    e.draft.priority = (e.draft.priority + 1) % 4;
                }
            }
            Target::MoveList => {
                let ids: Vec<u64> = self.data.lists.iter().map(|l| l.id).collect();
                if let Some(e) = &mut self.editor {
                    let at = ids.iter().position(|&i| i == e.draft.list).unwrap_or(0);
                    e.draft.list = ids[(at + 1) % ids.len()];
                    e.draft.parent = None;
                }
            }
            Target::Indent => {
                let items = &self.data.items;
                if let Some(e) = &mut self.editor {
                    if e.draft.parent.is_some() {
                        e.draft.parent = None;
                    } else if let Some(id) = e.id {
                        let mut copy = items.clone();
                        if let Some(r) = copy.iter_mut().find(|r| r.id == id) {
                            r.list = e.draft.list;
                            r.parent = None;
                        }
                        if model::indent(&mut copy, id) {
                            e.draft.parent = copy.iter().find(|r| r.id == id).and_then(|r| r.parent);
                        }
                    } else {
                        e.draft.parent = items.iter().rev().find(|r| r.list == e.draft.list && r.parent.is_none()).map(|r| r.id);
                    }
                }
            }
            Target::Duplicate => {
                if let Some(id) = self.editor.as_ref().and_then(|e| e.id) {
                    self.duplicate_item(ui, id, t);
                }
            }
            Target::OptClearCompleted => {
                self.options_open = false;
                self.clear_completed(ui, t);
            }
            Target::Delete => match self.editor.as_ref().map(|e| e.id) {
                Some(Some(id)) => {
                    if self.delete_item(id, t) {
                        self.close_form(ui, t);
                    }
                }
                Some(None) => {}
                None => {
                    // a list goes with its reminders, so the first press only asks (E2E W2-21)
                    if let Some(e) = self.list_edit.as_mut().filter(|e| e.mode == EditMode::List && e.id.is_some() && !e.confirm_delete) {
                        e.confirm_delete = true;
                        let list = e.id;
                        let n = self.data.items.iter().filter(|r| Some(r.list) == list).count();
                        self.notice = Some(delete_list_question(n));
                        self.dirty = true;
                        return;
                    }
                    let edit = self.list_edit.as_ref().and_then(|e| e.id.map(|id| (id, e.mode)));
                    if let Some((id, mode)) = edit {
                        let mut next = self.data.clone();
                        let (done, name) = match mode {
                            EditMode::List => (next.delete_list(id), "Delete List"),
                            EditMode::Section(_) => (organize::delete_section(&mut next, id), "Delete Section"),
                            EditMode::Group => (next.delete_group(id), "Delete Group"),
                            EditMode::Smart => {
                                let before = next.smarts.len();
                                next.smarts.retain(|m| m.id != id);
                                (next.smarts.len() != before, "Delete Smart List")
                            }
                            EditMode::Settings => (false, "Settings"),
                        };
                        if done && self.commit_named(next, name) {
                            if (mode == EditMode::List && self.view == View::List(id)) || (mode == EditMode::Smart && self.view == View::Custom(id)) {
                                self.view = View::Smart(Smart::All);
                            }
                            self.close_form(ui, t);
                        }
                    }
                }
            },
            Target::Colors | Target::Icons | Target::Calendar | Target::TimeStep | Target::Thumbs | Target::Scroll => {}
            Target::Color(c) => {
                if let Some(e) = &mut self.list_edit {
                    e.color = c;
                }
            }
            Target::Icon(i) => {
                if let Some(e) = &mut self.list_edit {
                    e.icon = i;
                }
            }
            Target::ListUp | Target::ListDown => {
                if let Some(id) = self.list_edit.as_ref().and_then(|e| e.id) {
                    let mut next = self.data.clone();
                    if next.move_list(id, if target == Target::ListUp { -1 } else { 1 }) {
                        self.commit(next);
                    }
                }
            }
        }
        self.dirty = true;
    }

    // ---- input ----------------------------------------------------------

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        // Return is the Clear alert's default button; other keys wait for the alert. The purge alert's delete button
        // is destructive, so Return does not answer it.
        if self.asking() {
            if matches!(out, Out::Enter) && self.clear_ask {
                self.activate(ui, Target::ClearConfirm, t);
            }
            return;
        }
        if self.focus.is_none() {
            self.list_key(ui, out, t);
            return;
        }
        let Some(f) = self.focus else { return };
        // the forward Delete key has no meaning in a field here (Backspace edits it)
        if matches!(out, Out::Named("Delete")) {
            return;
        }
        // Return in a form's name field is Done, as on the stock app (E2E W2-21: New Reminder and New List)
        if matches!(out, Out::Enter) && f == Field::Title && matches!(self.screen, Screen::Editor | Screen::ListEditor) {
            // a new reminder's field stays open after Return so several can be typed in a row, as on the stock app;
            // Done (the button) and Escape still close the form
            let next_list = self.editor.as_ref().filter(|e| e.id.is_none()).map(|e| e.draft.list);
            self.set_focus(None, t);
            self.activate(ui, Target::Save, t);
            if let (Some(list), true) = (next_list, self.editor.is_none() && self.screen == Screen::Items) {
                self.open_editor(ui, None, t);
                if let Some(e) = &mut self.editor {
                    e.draft.list = list;
                }
            }
            return;
        }
        if matches!(out, Out::Hide | Out::Enter) {
            self.set_focus(None, t);
        } else {
            self.field_mut(f).edit(ui, &out, t);
            if self.searching && self.screen == Screen::Items {
                self.rebuild(ui);
                self.scroll.jump_top();
            }
        }
        self.dirty = true;
    }
    fn scrollable(target: Target) -> bool {
        !matches!(
            target,
            Target::Sidebar
                | Target::Smart(_)
                | Target::List(_)
                | Target::Tag(_)
                | Target::ToggleGroup(_)
                | Target::Custom(_)
                | Target::SmartInfo(_)
                | Target::ListInfo(_)
                | Target::New
                | Target::NewList
                | Target::ListMenu(..)
                | Target::Search
                | Target::Options
                | Target::Undo
                | Target::Cancel
                | Target::Save
                | Target::OptSort(_)
                | Target::OptGroup(_)
                | Target::OptCompleted
                | Target::OptClearCompleted
                | Target::OptDefaultList(_)
                | Target::OptExport
                | Target::OptClose
                | Target::Field(_)
                | Target::Info(_)
                | Target::DetailsClose
                | Target::ClearAsk
                | Target::ClearConfirm
                | Target::ClearCancel
                | Target::PurgeConfirm
        )
    }
    fn swipe_to(&mut self, ui: &Ui, x: f32) {
        let max = ui.pt(BTN) * 2.0;
        if let Some(sw) = &mut self.swipe {
            sw.x = (sw.base + (x - self.down.0)).clamp(-max, max);
        }
    }
    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        // the content column's coordinates: the sidebar is left of it (its targets come back from `hit` at x < 0)
        self.layout_columns(ui);
        let event = match event {
            Event::Down(x, y) => Event::Down(x - self.x0, y),
            Event::Move(x, y) => Event::Move(x - self.x0, y),
            Event::Up(x, y) => Event::Up(x - self.x0, y),
            other => other,
        };
        let (x, y) = match event {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        self.dirty = true;
        if let Some(f) = self.field_touch {
            if let Some(frame) = self.field_frame(ui, f) {
                let w = self.w as f32;
                self.field_mut(f).touch(ui, event, frame, w, t);
            }
            if matches!(event, Event::Up(..)) {
                self.field_touch = None;
            }
            return;
        }
        match event {
            Event::Down(..) => {
                self.fit_scroll(ui);
                // Selection handles and the editing menu extend beyond the
                // field frame, so the focused field sees the touch first.
                if let Some(f) = self.focus {
                    if let Some(frame) = self.field_frame(ui, f) {
                        let w = self.w as f32;
                        if self.field_mut(f).touch(ui, event, frame, w, t) {
                            self.field_touch = Some(f);
                            self.pressed = None;
                            return;
                        }
                    }
                }
                self.down = (x, y);
                self.axis = Axis::Undecided;
                self.pressed = self.hit(ui, x, y);
                match self.pressed {
                    Some(Target::Field(f)) => {
                        self.set_focus(Some(f), t);
                        if let Some(frame) = self.field_frame(ui, f) {
                            let w = self.w as f32;
                            if self.field_mut(f).touch(ui, event, frame, w, t) {
                                self.field_touch = Some(f);
                            }
                        }
                        self.pressed = None;
                    }
                    Some(target) if Self::scrollable(target) => {
                        self.scrolling = true;
                        self.scroll.down(y, t);
                        if matches!(target, Target::Row(_)) {
                            self.press_at = Some(t);
                        }
                    }
                    _ => {}
                }
            }
            Event::Move(..) => {
                if self.drag.is_some() {
                    if let Some(d) = &mut self.drag {
                        d.y = y;
                    }
                } else if self.axis == Axis::Horizontal {
                    self.swipe_to(ui, x);
                } else if self.scrolling {
                    let (dx, dy) = (x - self.down.0, y - self.down.1);
                    let swipeable = self.screen == Screen::Items && matches!(self.pressed, Some(Target::Row(_) | Target::Circle(_)));
                    if swipeable && self.axis == Axis::Undecided && dx.abs() > ui.pt(SPACE_M) && dx.abs() > dy.abs() * 1.2 {
                        let row = match self.pressed {
                            Some(Target::Row(r) | Target::Circle(r)) => r,
                            _ => 0,
                        };
                        self.axis = Axis::Horizontal;
                        self.scrolling = false;
                        self.scroll.up(y, t);
                        self.press_at = None;
                        self.pressed = None;
                        if let Some(id) = self.item_id(row) {
                            let base = match &self.swipe {
                                Some(s) if s.open && s.id == id => -ui.pt(BTN) * 2.0,
                                _ => 0.0,
                            };
                            self.swipe = Some(Swipe { id, x: base, base, open: false });
                            self.swipe_to(ui, x);
                        }
                    } else {
                        self.scroll.moved(y, t);
                        if self.scroll.dragging() {
                            self.pressed = None;
                            self.axis = Axis::Vertical;
                        }
                        if dx.abs().max(dy.abs()) > ui.pt(SPACE_S) {
                            self.press_at = None;
                        }
                    }
                }
            }
            Event::Up(..) => {
                self.press_at = None;
                if self.drag.is_some() {
                    self.finish_drag(ui);
                    self.pressed = None;
                    return;
                }
                if self.axis == Axis::Horizontal {
                    self.axis = Axis::Undecided;
                    let max = ui.pt(BTN) * 2.0;
                    // Swiped right past the threshold: complete the reminder.
                    if let Some(id) = self.swipe.as_ref().filter(|s| s.x > ui.pt(BTN)).map(|s| s.id) {
                        self.swipe = None;
                        self.pressed = None;
                        self.complete_swiped(ui, id);
                        return;
                    }
                    if let Some(sw) = &mut self.swipe {
                        sw.open = sw.x < -ui.pt(BTN);
                        sw.x = if sw.open { -max } else { 0.0 };
                    }
                    if self.swipe.as_ref().is_some_and(|s| !s.open) {
                        self.swipe = None;
                    }
                    self.pressed = None;
                    return;
                }
                self.axis = Axis::Undecided;
                let tap = if std::mem::take(&mut self.scrolling) { self.scroll.up(y, t) } else { true };
                let pressed = self.pressed.take();
                if tap {
                    if let Some(target) = pressed.filter(|&p| self.hit(ui, x, y) == Some(p)) {
                        let own_button = matches!(target, Target::SwipeFlag(_) | Target::SwipeDelete(_));
                        if self.swipe.as_ref().is_some_and(|s| s.open) && !own_button {
                            self.swipe = None;
                            return;
                        }
                        // a click selects a reminder; a click on the selected one opens its Details (E2E W2-21)
                        if let Target::Row(row) = target {
                            let id = self.item_id(row);
                            if self.screen == Screen::Items && id.is_some() && self.selected != id {
                                self.selected = id;
                                return;
                            }
                        }
                        self.activate(ui, target, t);
                    }
                }
            }
            _ => {}
        }
    }
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        let style = (ui.text_scale.to_bits(), ui.bold);
        if style != self.style {
            self.style = style;
            self.labels.clear();
            let (hint, max) = (self.title_hint, self.title_max);
            let (title, notes, url) = (self.title.text().to_string(), self.notes.text().to_string(), self.url.text().to_string());
            self.title = mk_field(ui, hint, max, &title);
            self.notes = mk_field(ui, "Notes", model::MAX_NOTES, &notes);
            self.url = mk_field(ui, "URL", model::MAX_URL, &url);
            let focus = self.focus;
            self.title.focused = focus == Some(Field::Title);
            self.notes.focused = focus == Some(Field::Notes);
            self.url.focused = focus == Some(Field::Url);
            self.dirty = true;
        }
        let today = local_day(self.wall.now(t), self.wall.offset);
        if today != self.today {
            self.today = today;
            self.rebuild(ui);
        }
        if self.undo.as_ref().is_some_and(|u| t >= u.until) {
            self.undo = None;
            self.dirty = true;
        }
        if let (Some(p), Some(Target::Row(row))) = (self.press_at, self.pressed) {
            if t - p >= LONG_PRESS_MS && self.can_reorder() {
                if let Some(Row::Item { idx, depth: 0 }) = self.rows.get(row) {
                    let id = self.data.items[*idx].id;
                    self.drag = Some(Drag { id, y: self.down.1 });
                    self.press_at = None;
                    self.pressed = None;
                    self.scrolling = false;
                    self.scroll.up(self.down.1, t);
                    self.swipe = None;
                    self.dirty = true;
                }
            }
        }
        self.fit_scroll(ui);
        let fields = self.title.tick(ui, t) | self.notes.tick(ui, t) | self.url.tick(ui, t);
        self.dirty |= self.scroll.tick(t) || fields;
        if self.focus.is_some() {
            self.dirty = true;
        }
        self.dirty
    }
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let now = self.wall.now(t);
        let day = local_day(now, self.wall.offset);
        let midnight = (day + 1) * 86_400 - self.wall.offset as i64;
        let mut due = Some(self.wall.at + (midnight - self.wall.unix) as f64 * 1e3);
        for wake in [
            self.title.next_wake(t),
            self.notes.next_wake(t),
            self.url.next_wake(t),
            self.undo.as_ref().map(|u| u.until),
            self.press_at.filter(|_| self.pressed.is_some()).map(|p| p + LONG_PRESS_MS),
            self.scroll.animating().then_some(t),
        ] {
            if let Some(wake) = wake {
                due = Some(due.map_or(wake, |old| old.min(wake)));
            }
        }
        due
    }
}


#[cfg(test)]
#[path = "actions_tests.rs"]
mod actions_tests;

/// What the list's Delete button reads once pressed; the caption names it, so the two agree.
pub(crate) const CONFIRM_DELETE_LIST: &str = "Confirm Delete List";

/// What a first press on a list's Delete says: the list goes with its reminders.
pub(crate) fn delete_list_question(reminders: usize) -> String {
    match reminders {
        0 => format!("Delete this list? Press {CONFIRM_DELETE_LIST} to delete it."),
        1 => format!("Delete this list and its reminder? Press {CONFIRM_DELETE_LIST} to delete both."),
        n => format!("Delete this list and its {n} reminders? Press {CONFIRM_DELETE_LIST} to delete them all."),
    }
}
