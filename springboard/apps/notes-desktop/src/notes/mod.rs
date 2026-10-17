//! Notes, Golden Gate Golden Gate style: a full-size window with a source-list
//! sidebar of folders, the note list column (title, date and snippet rows,
//! swipe-to-delete) and the editor beside it under one unified toolbar
//! (`chrome.rs`), undo/redo and a debounced autosave to `<dir>/<id>.md`.
//! Colours follow Light and Dark (`Pair` tokens). Pin/unpin controls
//! group matching notes under Pinned while preserving their Markdown files.
//! Notes render lightweight Markdown (headings, bold, italic, code, lists and
//! tap-to-toggle checklists), live in folders (subdirectories) with a
//! Recently Deleted bin, sort by edit/creation date or title, search all
//! notes with highlighted, navigable matches, show attached images inline
//! and export as `.md` or text.

pub mod attach;
pub mod attachments;
pub mod folders;
pub mod note_url;
pub mod settings;
mod chrome;
mod settings_pane;
pub mod icon;
pub mod preview;
pub mod rpc;
pub mod sketch;
pub mod smart;
pub mod import;
pub mod math;
pub mod outline;
pub mod table;
pub mod fold;
pub mod gallery;
pub mod note_windows;

mod parity;
mod html;
mod pdf;
mod picker;
pub mod store;
mod lists;
mod tags;
mod grouping;
mod lock;
pub mod system;

pub use icon::icon;

use bifrost_app::layout::top_inset;

/// Unified Logging: subsystem `is.olibuijr.notes` (manifest `[[log.subsystem]]`), categories `save` and `sync`.
fn notes_log(category: &str) -> &'static bifrost_app::log::Logger {
    static L: std::sync::Mutex<Vec<(String, &'static bifrost_app::log::Logger)>> = std::sync::Mutex::new(Vec::new());
    let mut all = L.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, l)) = all.iter().find(|(c, _)| c == category) {
        return l;
    }
    let l: &'static bifrost_app::log::Logger = Box::leak(Box::new(bifrost_app::log::Logger::new("is.olibuijr.notes", category)));
    all.push((category.to_string(), l));
    l
}
use bifrost_app::kit::ui::{SPACE_HUGE, SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL};
use bifrost_app::kit::ui::{MAC_FOOTNOTE_PT, MAC_LARGE_TITLE_PT, MAC_TITLE1_PT, MAC_TITLE3_PT};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_CAPTION1_PT, MAC_SUBHEAD_PT};
use springboard::layout::in_rect as within;
use bifrost_app::kit::TextCache;
use springboard::shell::md;
use springboard::shell::clock::Wall;
use springboard::shell::keyboard::{self, Keyboard, Out};
use springboard::shell::textarea::TextArea;
use springboard::shell::textfield::TextField;
use springboard::shell::ui::{centred, home_zone, Ui, Weight, WHITE};
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, DESTRUCTIVE, FIELD_BG, LABEL, LABEL_2, LABEL_3, LINK, SEPARATOR_C};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::kit::Event;
use springboard::deildu::scroll::Scroller;
use springboard::gfx::{pack, Image, Surface};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use store::{Autosave, Note};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use std::sync::{Arc, Mutex};

/// A background result waiting for the main queue: `None` until the worker has finished.
type ImageSlot = Arc<Mutex<Option<HashMap<String, Image>>>>;
/// Gallery thumbnails decoded off the UI thread: (note id, picture name, picture or None when it did not decode).
type ThumbSlot = Arc<Mutex<Vec<(String, String, Option<Image>)>>>;

/// A disclosure triangle in a box `size` wide at (`x`, `y`): pointing right when `folded`, down when open. The kit
/// has no disclosure control, so it is drawn as stripes the way the Mail app draws its triangles.
fn disclosure(s: &mut Surface, x: f32, y: f32, size: f32, folded: bool, color: u32) {
    let n = size.max(2.0) as usize;
    let (x, y) = (x.max(0.0) as usize, y.max(0.0) as usize);
    for i in 0..n / 2 + 1 {
        let len = n.saturating_sub(2 * i);
        if len == 0 {
            break;
        }
        if folded {
            s.fill_rect(x + i, y + i, 1, len, color);
        } else {
            s.fill_rect(x + i, y + i, len, 1, color);
        }
    }
}
use bifrost_app::kit::ui::{SPACE_XS};

/// Picker sheet header height and thumbnail decode size (pt).
const PICK_HEAD_PT: f32 = 52.0;
const PICK_THUMB_PT: f32 = 200.0;
/// Sketch pad header height and the width of its Cancel, Undo and Done zones (pt).
const SKETCH_HEAD_PT: f32 = 52.0;
const SKETCH_ZONE_PT: f32 = 90.0;

/// The Notes tint (systemYellow in dark mode).
pub const YELLOW: u32 = springboard::shell::ui::SYSTEM_YELLOW;
const RED: u32 = springboard::shell::ui::SYSTEM_RED;

/// The ink of the note text: white on the dark note background, the appearance's label colour otherwise.
pub fn ink(dark_background: bool) -> u32 {
    if dark_background { LABEL.dark } else { LABEL.now() }
}

/// An editor for the note text at `w` px, with the Settings text size and ink (Markdown, links detected).
fn new_editor(ui: &Ui, w: f32, settings: &settings::Settings) -> TextArea {
    let mut e = TextArea::new(ui, w, settings.text_size.body_pt(), TITLE_PT, ink(settings.dark_background), YELLOW);
    e.set_detect_links(true);
    e.set_markdown(ui, true);
    e
}
/// Two taps on one row within this many ms open the note in a window of its own.
const DOUBLE_TAP_MS: f64 = 400.0;
/// How long the Undo bar stays after a delete.
const UNDO_TOAST_MS: f64 = 5000.0;

/// A note just moved to Recently Deleted: where it came from, so undo can put it back.
struct UndoToast {
    id: String,
    from: Option<String>,
    until: f64,
}

const MARGIN_PT: f32 = 20.0;
/// The top of the content under the unified toolbar band (list rows and the editor).
fn nav_bottom_pt() -> f32 { top_inset() + bifrost_app::layout::UNIFIED_BAR_PT }
fn list_top_pt() -> f32 { nav_bottom_pt() + SPACE_S }
const ROW_PT: f32 = 56.0;
/// the revealed Delete button
const DELETE_PT: f32 = 90.0;
/// a swipe this far across the row deletes on release
const FULL_SWIPE: f32 = 0.45;
const SLOP_PT: f32 = 8.0;
/// the undo/redo/format row above the keyboard
const SHORTCUT_PT: f32 = 44.0;
/// width of the undo and redo slots in the shortcut row
const UNDO_W_PT: f32 = 70.0;
const TEXT_INSET_PT: f32 = 20.0;
/// The narrowest a tag can sit beside a note's title in the list.
const TAG_MIN_PT: f32 = 120.0;
const PIN_W_PT: f32 = 72.0;
const SECTION_PT: f32 = 32.0;
const SECTION_GAP_PT: f32 = 12.0;
/// The disclosure chevron beside a heading, and the gap to the text.
const CHEVRON_PT: f32 = 10.0;
const CHEVRON_GAP_PT: f32 = 6.0;
/// A gallery card's height, and the part of it the picture takes.
const GALLERY_CARD_PT: f32 = 196.0;
const GALLERY_THUMB_PT: f32 = 110.0;
const DATE_TOP_PT: f32 = 12.0;
const TEXT_TOP_PT: f32 = 36.0;
const TITLE_PT: f32 = MAC_LARGE_TITLE_PT;
const MENU_W_PT: f32 = 240.0;
/// How many tags the list menu lists (the sidebar lists them all).
const MENU_TAGS: usize = 12;
/// How many notes the Recent Notes menu lists.
pub const RECENT_MENU: usize = 5;
/// evdev Up and Down arrows (the move-item chords) and Tab (Shift+Tab outdents).
const KEY_UP: u32 = 103;
const KEY_DOWN: u32 = 108;
const KEY_TAB: u32 = 15;
/// evdev K (Cmd+K adds a link).
const KEY_K: u32 = 37;
const MENU_ROW_PT: f32 = bifrost_app::widgets::MENU_ROW_PT;
const MENU_PAD_PT: f32 = bifrost_app::widgets::MENU_PAD_PT;
/// width of one formatting button in the shortcut row
const FMT_W_PT: f32 = 52.0;
/// The paragraph and list styles of the Format menu, in menu order.
const PARAGRAPH_STYLES: [(&str, md::Block); 10] = [
    ("Title", md::Block::Title),
    ("Heading", md::Block::Heading),
    ("Subheading", md::Block::Subheading),
    ("Body", md::Block::Body),
    ("Monostyled", md::Block::Mono),
    ("Block Quote", md::Block::Quote),
    ("Bulleted List", md::Block::Bullet),
    ("Dashed List", md::Block::Dashed),
    ("Numbered List", md::Block::Numbered(1)),
    ("Checklist", md::Block::Check(false)),
];
/// The character styles of the Format menu, in menu order.
const CHARACTER_STYLES: [(&str, Fmt); 6] = [
    ("Bold", Fmt::Bold),
    ("Italic", Fmt::Italic),
    ("Underline", Fmt::Underline),
    ("Strikethrough", Fmt::Strike),
    ("Highlight", Fmt::Highlight),
    ("Inline Code", Fmt::Code),
];
/// the formatting buttons in the shortcut row, left to right
const FMT_BUTTONS: [(&str, Fmt); 6] = [("B", Fmt::Bold), ("I", Fmt::Italic), ("Code", Fmt::Code), ("List", Fmt::Bullet), ("1.", Fmt::Number), ("[ ]", Fmt::Check)];

/// Which notes the list shows.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    All,
    Folder(String),
    Trash,
    /// A smart folder: the notes its predicate picks out (outside the bin).
    Smart(smart::Smart),
    /// The notes that carry a `#tag` (lowercase, without the `#`), outside the bin.
    Tag(String),
}

/// An editing command on the open note that the Format menu offers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Format {
    /// Bold, italic, underline, strikethrough, highlight or inline code on the selection.
    Char(Fmt),
    /// One more level on the list lines of the selection (Tab).
    Indent,
    /// One level less (Shift+Tab).
    Outdent,
    /// The list item with the items under it passes its neighbour (Ctrl+Cmd+Up).
    MoveUp,
    MoveDown,
    /// Tick or untick every checklist item of the note.
    CheckAll,
    UncheckAll,
    /// An operation on the table under the caret.
    Table(table::Op),
    /// Ask for a web address and make the selection a link (Cmd+K).
    AddLink,
    /// Answer every line that ends with `=` after a calculation.
    CalculateMath,
    /// Fold or unfold the section under the caret (Format > Collapse / Expand Section).
    CollapseSection,
    ExpandSection,
    /// Fold or unfold every section of the note.
    CollapseAll,
    ExpandAll,
}

/// A formatting button: inline emphasis or a list style for the paragraphs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fmt {
    Bold,
    Italic,
    Code,
    Strike,
    Underline,
    Highlight,
    Bullet,
    Number,
    Check,
}

/// What a menu row does.
#[derive(Clone, Debug, PartialEq)]
enum Act {
    View(View),
    NewFolder,
    RenameFolder,
    DeleteFolder,
    Sort(store::SortKey),
    ToggleCheckedBottom,
    SortCheckedNow,
    ShareMd,
    ShareText,
    SharePdf,
    ShareHtml,
    InsertTable,
    /// move the open note to a folder ("" is no folder)
    MoveTo(String),
    Style(md::Block),
    DeleteNote,
    Duplicate,
    ClearChecked,
    Recover,
    EraseNote,
    AttachPhoto,
    InsertSketch,
    ChoosePhoto,
    /// pin or unpin the open note
    TogglePin,
    /// Find > Match Whole Word
    ToggleWholeWord,
    /// Check Spelling and Links in the open note
    CheckSpelling,
    /// Proofread the selection, or the whole note: the menu lists the suggestions
    Proofread,
    /// accept suggestion n of the proofreading menu
    ProofreadAccept(usize),
    /// accept every suggestion left
    ProofreadAll,
    /// close the proofreading menu
    ProofreadDone,
    /// Find > Replace: opens the replace card for the find text
    Replace,
    /// Sort > Newest First (`false`) or Oldest First (`true`)
    SortOrder(bool),
    /// the Format menu from the editor menu
    OpenFormat,
    /// a Format menu command
    Do(Format),
    /// Math Results on or off
    ToggleMath,
    /// New Quick Note reopens the last quick note, or not
    ToggleResumeQuick,
    /// the list sections by date on or off
    ToggleGroupDate,
}

/// Find > Replace: the replacement text being typed in the replace card over the editor.
struct Replacing {
    field: TextField,
}

/// The replace card's height and its rows (pt): the label and buttons row, then the field.
const REPLACE_CARD_PT: f32 = 80.0;
const REPLACE_BAR_PT: f32 = 32.0;
const REPLACE_FIELD_PT: f32 = 36.0;
/// Width of the Replace and Replace All buttons' hit zones (pt).
const REPLACE_ONE_PT: f32 = 84.0;
const REPLACE_ALL_PT: f32 = 104.0;

/// A popover list of actions (folders and sorting in the list, note actions
/// in the editor). `bool` marks the current choice.
struct Menu {
    items: Vec<(String, Act, bool)>,
    x: f32,
    y: f32,
}

/// The one-line prompt over the list: a folder name (`old` is the folder being renamed) or, for Add Link, a web
/// address (`link` is the editor range the link wraps).
struct Naming {
    old: Option<String>,
    field: TextField,
    link: Option<Range<usize>>,
}

/// Something a touch can land on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    Search,
    Row(usize),
    /// the revealed Delete button of a row
    Delete(usize),
    /// Pin/unpin a shown row, without opening its editor.
    Pin(usize),
    /// the list header's More button: opens the folders and sort menu
    Title,
    /// the editor's actions menu
    EditorMore,
    /// New Folder (sidebar toolbar)
    FolderNew,
    /// Show or hide the sidebar
    SidebarToggle,
    /// a sidebar row (`chrome::sidebar` id)
    Sidebar(u32),
    /// a toolbar button without a target of its own (`chrome::B_*`)
    Bar(u32),
    /// previous / next search match in the open note
    FindPrev,
    FindNext,
    /// a formatting button in the shortcut row
    Fmt(Fmt),
    /// Pin/unpin the open note.
    EditorPin,
    /// Share the open note.
    EditorShare,
    Compose,
    /// empty list space (scrolls)
    List,
    Back,
    Done,
    Undo,
    UndoDelete,
    Redo,
    /// cycles the paragraph style marker
    Format,
    Text,
    /// the disclosure chevron of the heading on this paragraph (folds or unfolds its section)
    Fold(usize),
    Keyboard,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Screen {
    /// no note open: the editor column is empty
    List,
    /// index into `notes`
    Editor(usize),
}

/// A touch that started on a list row: it becomes a vertical scroll or a
/// horizontal swipe once it leaves the slop.
#[derive(Clone, Copy, Debug)]
struct RowTouch {
    row: usize,
    x0: f32,
    y0: f32,
    /// None until the direction is known
    horizontal: Option<bool>,
    /// starting reveal (negative when the row was already open)
    dx0: f32,
}

struct Row {
    title: Image,
    date: Image,
    snippet: Image,
    /// the texts, for the white-on-accent selected row
    title_text: String,
    date_text: String,
    snippet_text: String,
}

pub struct Notes {
    w: i32,
    h: i32,
    dir: PathBuf,
    wall: Wall,
    notes: Vec<Note>,
    pinned: HashSet<String>,
    pin_error: Option<String>,
    /// where each note lives: "" is the root, else a folder or the bin
    loc: HashMap<String, String>,
    folders: Vec<String>,
    /// View > Hide Sidebar (the sidebar also folds away in a narrow window)
    sidebar_hidden: bool,
    /// the editor's wrap width in px, as last laid out
    editor_w: f32,
    /// `LABEL` when the rows and the editor text were last coloured (Light/Dark)
    appearance: u32,
    view: View,
    prefs: store::Prefs,
    /// the Notes settings pane's values (text size, new note start, dark background)
    note_settings: settings::Settings,
    /// Notes > Settings is open over the window
    settings_open: bool,
    /// the collapsed headings of every note (Format > Collapse Section)
    folds: fold::Folds,
    /// gallery thumbnails by note id: the picture's name and the decoded picture (None when it did not decode)
    thumbs: HashMap<String, (String, Option<Image>)>,
    thumb_pending: HashSet<String>,
    thumb_slot: ThumbSlot,
    menu: Option<Menu>,
    /// the Photos picker sheet
    picker: Option<picker::Picker>,
    /// the sketch pad over the open note (Insert Sketch)
    sketch: Option<sketch::Pad>,
    pick_drag: Option<f32>,
    pick_moved: bool,
    naming: Option<Naming>,
    /// the replace card over the open note (Find > Replace)
    replacing: Option<Replacing>,
    /// the time of the last list rebuild (frame clock, ms): smart folders measure "this week" from it
    now_t: f64,
    /// a transient error line (folder or move failures)
    notice: Option<String>,
    /// The last delete, for a short Undo bar.
    undo_toast: Option<UndoToast>,
    /// search matches in the open note, the current one, and the query
    hl: Vec<Range<usize>>,
    hl_cur: usize,
    hl_query: String,
    /// Proofread: the suggestions left, while the person goes through them (see `bifrost_ui::proofread_panel`)
    proofread: Option<bifrost_app::controls::proofread_panel::Panel>,
    /// Find > Match Whole Word: a search hit must not touch a letter or digit.
    whole_word: bool,
    rows: Vec<Row>,
    /// every `#tag` of the notes outside the bin with the number of notes that carry it, by name
    tag_list: Vec<(String, u32)>,
    /// indices into `notes` shown by the list (the search result)
    shown: Vec<usize>,
    shown_pinned: usize,
    /// the list's section headers as `(label, index into shown of its first row)`: Pinned, Notes, or the date groups
    sections: Vec<(String, usize)>,
    screen: Screen,
    list_scroll: Scroller,
    scroll: Scroller,
    search: TextField,
    searching: bool,
    editor: TextArea,
    /// the images of the open note being decoded off the UI thread: the worker's result lands here on the main queue
    images_slot: Option<ImageSlot>,
    date: Option<Image>,
    keyboard: Keyboard,
    kb_touch: bool,
    /// a physical keyboard is attached (desktop): the docked one stays hidden
    hw_kb: bool,
    autosave: Autosave,
    /// Id of the note the pending autosave belongs to, so the write still
    /// lands when the screen has changed (or the list reordered) by then.
    unsaved: Option<String>,
    pressed: Option<Target>,
    share_req: Option<springboard::shell::share::Item>,
    /// a link the user tapped in the note, for the app to open
    link_req: Option<String>,
    /// the last row tapped and when (ms), for a double-click that opens a note in its own window
    row_tap: Option<(String, f64)>,
    /// a note the user asked to open in a window of its own (set by a double-click or the Note menu)
    window_req: Option<String>,
    /// the note windows open now (Note > Open in Separate Window)
    pub windows: note_windows::NoteWindows,
    row_touch: Option<RowTouch>,
    /// (row, reveal px <= 0) of the row sliding or resting open
    reveal: Option<(usize, f32)>,
    /// Row selected with the arrow keys, by note id.
    list_selected: Option<String>,
    scrolling: bool,
    labels: TextCache,
    dirty: bool,
}

/// Every note on disk (root, folders and the bin after purging the expired),
/// where each lives, and the folder names.
fn load_everything(dir: &Path, now: i64) -> (Vec<Note>, HashMap<String, String>, Vec<String>) {
    folders::purge(dir, now);
    let (mut notes, mut loc) = (vec![], HashMap::new());
    for (folder, note) in folders::load_all(dir) {
        if let Some(f) = folder {
            loc.insert(note.id.clone(), f);
        }
        notes.push(note);
    }
    for note in store::load(&dir.join(folders::TRASH)) {
        loc.insert(note.id.clone(), folders::TRASH.to_string());
        notes.push(note);
    }
    (notes, loc, folders::list(dir))
}

impl Notes {
    /// Lays the app out for a pane of `w` x `h` px (Split View, Slide Over).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
    }

    /// With a physical keyboard (desktop) the docked keyboard is never shown.
    pub fn set_hardware_keyboard(&mut self, on: bool) {
        self.hw_kb = on;
        if on {
            self.keyboard.set_shown(false, 0.0);
        }
    }

    /// Loads the notes in `dir`; `wall` anchors dates to the frame clock.
    pub fn new(ui: &Ui, w: i32, h: i32, dir: PathBuf, wall: Wall) -> Self {
        let (notes, loc, folders) = load_everything(&dir, wall.now(0.0));
        let pinned = store::load_pins(&dir, &notes);
        let prefs = store::load_prefs(&dir);
        let folds = fold::load(&dir);
        let note_settings = settings::load(&dir);
        let mut n = Notes {
            w,
            h,
            note_settings,
            settings_open: false,
            notes,
            pinned,
            pin_error: None,
            loc,
            folders,
            sidebar_hidden: false,
            editor_w: 0.0,
            appearance: LABEL.now(),
            view: View::All,
            prefs,
            folds,
            thumbs: HashMap::new(),
            thumb_pending: HashSet::new(),
            thumb_slot: ThumbSlot::default(),
            menu: None,
            picker: None,
            sketch: None,
            pick_drag: None,
            pick_moved: false,
            naming: None,
            replacing: None,
            now_t: 0.0,
            notice: None,
            undo_toast: None,
            hl: vec![],
            hl_cur: 0,
            hl_query: String::new(),
            proofread: None,
            whole_word: false,
            dir,
            wall,
            rows: vec![],
            tag_list: vec![],
            shown: vec![],
            shown_pinned: 0,
            sections: vec![],
            screen: Screen::List,
            list_scroll: Scroller::new(),
            scroll: Scroller::new(),
            search: TextField::new(ui, "Search", MAC_BODY_PT, false, 100),
            searching: false,
            editor: {
                let mut e = TextArea::new(ui, w as f32 - 2.0 * ui.pt(TEXT_INSET_PT), note_settings.text_size.body_pt(), TITLE_PT, ink(note_settings.dark_background), YELLOW);
                e.set_detect_links(true);
                e
            },
            images_slot: None,
            date: None,
            keyboard: Keyboard::new_prose(w as f32, ui.s),
            kb_touch: false,
            hw_kb: false,
            autosave: Autosave::default(),
            unsaved: None,
            pressed: None,
            share_req: None,
            link_req: None,
            row_tap: None,
            window_req: None,
            windows: note_windows::NoteWindows::new(),
            row_touch: None,
            reveal: None,
            list_selected: None,
            scrolling: false,
            labels: TextCache::default(),
            dirty: true,
        };
        n.search.focused = false;
        n.editor.set_markdown(ui, true);
        n.sync_width(ui);
        n.rebuild(ui, 0.0);
        n
    }

    /// Re-anchors wall-clock time (see `Shell::set_wall`).
    pub fn set_wall(&mut self, wall: Wall) {
        self.wall = wall;
    }

    /// The list's section headers for the rows shown: Pinned first, then either date groups (Group by Date, a date
    /// sort, no search, not in the bin) or a plain Notes header when something is pinned.
    fn compute_sections(&self, now: i64, searching: bool) -> Vec<(String, usize)> {
        let (n, pins) = (self.shown.len(), self.shown_pinned);
        let mut out = vec![];
        if pins > 0 {
            out.push(("Pinned".to_string(), 0));
        }
        let by_date = self.prefs.group_by_date && self.prefs.sort != store::SortKey::Title && !searching && self.view != View::Trash;
        if by_date {
            let mut last: Option<String> = None;
            for k in pins..n {
                let note = &self.notes[self.shown[k]];
                let stamp = if self.prefs.sort == store::SortKey::DateCreated { store::created(note) } else { note.modified };
                let group = store::date_group(stamp, now, self.wall.offset);
                if last.as_ref() != Some(&group) {
                    out.push((group.clone(), k));
                    last = Some(group);
                }
            }
        } else if pins > 0 && pins < n {
            out.push(("Notes".to_string(), pins));
        }
        out
    }

    /// Height in points of the section header number `k` (the first has no gap above it).
    fn header_pt(k: usize) -> f32 {
        if k == 0 { SECTION_PT } else { SECTION_PT + SECTION_GAP_PT }
    }

    /// Points of section headers above shown row `i`.
    fn section_offset(&self, i: usize) -> f32 {
        self.sections.iter().enumerate().filter(|(_, (_, start))| *start <= i).map(|(k, _)| Self::header_pt(k)).sum()
    }

    /// Points of all section headers.
    fn sections_total(&self) -> f32 {
        (0..self.sections.len()).map(Self::header_pt).sum()
    }

    /// Rows `[shown index]` as `(note id, title)`, for tests and inspection.
    pub fn listed(&self) -> Vec<(&str, &str)> {
        self.shown.iter().map(|&i| (self.notes[i].id.as_str(), self.notes[i].title())).collect()
    }

    /// The notes Search may find: every note outside Recently Deleted, with its folder ("" for the root).
    pub fn indexable(&self) -> Vec<(&Note, &str)> {
        self.notes
            .iter()
            .filter_map(|n| {
                let folder = self.loc.get(&n.id).map_or("", String::as_str);
                (folder != folders::TRASH).then_some((n, folder))
            })
            .collect()
    }

    /// The note open in the editor.
    pub fn editing(&self) -> Option<&Note> {
        match self.screen {
            Screen::Editor(i) => self.notes.get(i),
            Screen::List => None,
        }
    }

    pub fn keyboard_shown(&self) -> bool {
        self.keyboard.shown()
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.contains(id)
    }

    /// Persists a pin change before changing the visible grouping. Existing
    /// note text and timestamps stay untouched. False means no such note or
    /// persistence failed; the old pin state remains visible.
    pub fn set_pinned(&mut self, ui: &Ui, id: &str, pinned: bool, t: f64) -> bool {
        if !self.notes.iter().any(|note| note.id == id) { return false; }
        if let Err(error) = store::set_pinned(&self.dir, id, pinned) {
            self.pin_error = Some(format!("Unable to update pin: {error}"));
            self.dirty = true;
            return false;
        }
        if pinned { self.pinned.insert(id.to_string()); } else { self.pinned.remove(id); }
        self.pin_error = None;
        // rebuild re-finds the open note after sorting, so the editor keeps addressing it
        self.rebuild(ui, t);
        self.dirty = true;
        true
    }

    /// key=value lines for remote inspection.
    pub fn describe(&self) -> String {
        let screen = match self.screen {
            Screen::List => "list".to_string(),
            Screen::Editor(i) => format!("editor:{}", self.notes[i].id),
        };
        let view = match &self.view {
            View::All => "all".to_string(),
            View::Folder(f) => format!("folder:{f}"),
            View::Trash => "trash".to_string(),
            View::Smart(s) => format!("smart:{}", s.key()),
            View::Tag(t) => format!("tag:{t}"),
        };
        format!("notes_screen={screen}\nnotes_count={}\nnotes_shown={}\nnotes_keyboard={}\nnotes_pinned={}\nnotes_pin_error={}\nnotes_view={view}\nnotes_folders={}\nnotes_sort={}\n", self.notes.len(), self.shown.len(), self.keyboard.shown(), self.notes.iter().filter(|n| self.is_pinned(&n.id)).count(), self.pin_error.is_some(), self.folders.len(), self.prefs.sort.label())
    }

    // --- folders and locations ---

    /// Where a note lives: "" (root), a folder name or the bin's directory.
    fn loc_of(&self, id: &str) -> &str {
        self.loc.get(id).map_or("", String::as_str)
    }

    /// The directory holding a note's file.
    fn note_dir(&self, id: &str) -> PathBuf {
        match self.loc_of(id) {
            "" => self.dir.clone(),
            loc => self.dir.join(loc),
        }
    }

    /// The folder to pass to `folders::*` for a note (None is the root).
    fn folder_arg(&self, id: &str) -> Option<String> {
        Some(self.loc_of(id).to_string()).filter(|l| !l.is_empty())
    }

    /// Whether note `i` belongs in the current view; a search looks through
    /// every folder but never the bin (unless the bin is open).
    fn visible(&self, i: usize, searching: bool, cutoff: i64) -> bool {
        let loc = self.loc_of(&self.notes[i].id);
        match &self.view {
            View::Trash => loc == folders::TRASH,
            _ if loc == folders::TRASH => false,
            View::All => true,
            View::Folder(f) => searching || loc == f,
            View::Smart(s) => s.matches(&self.notes[i], self.is_pinned(&self.notes[i].id), cutoff),
            View::Tag(t) => store::has_tag(&self.notes[i].text, t),
        }
    }

    /// The Unix time before which a note is not "Edited This Week": a week before the last list rebuild.
    fn smart_cutoff(&self) -> i64 {
        self.wall.now(self.now_t) - smart::WEEK_SECS
    }

    pub fn view(&self) -> &View {
        &self.view
    }

    pub fn folder_names(&self) -> &[String] {
        &self.folders
    }

    pub fn prefs(&self) -> store::Prefs {
        self.prefs
    }

    /// The folder (or bin) a note is in; None for the root.
    pub fn folder_of(&self, id: &str) -> Option<&str> {
        Some(self.loc_of(id)).filter(|l| !l.is_empty())
    }

    /// Re-reads every note from disk (after folder changes).
    fn reload(&mut self, ui: &Ui, t: f64) {
        let (notes, loc, folders) = load_everything(&self.dir, self.wall.now(t));
        self.pinned = store::load_pins(&self.dir, &notes);
        self.notes = notes;
        self.loc = loc;
        self.folders = folders;
        if let View::Folder(f) = &self.view {
            if !self.folders.contains(f) {
                self.view = View::All;
            }
        }
        self.rebuild(ui, t);
    }

    fn view_title(&self) -> String {
        match &self.view {
            View::All => "Notes".to_string(),
            View::Folder(f) => f.clone(),
            View::Trash => "Recently Deleted".to_string(),
            View::Smart(s) => s.label().to_string(),
            View::Tag(t) => format!("#{t}"),
        }
    }

    // --- list ---

    /// Sorts, filters and re-renders the list rows at time `t`.
    fn rebuild(&mut self, ui: &Ui, t: f64) {
        self.now_t = t;
        // sorting moves notes: the open one is found again by id
        let open = self.editing().map(|n| n.id.clone());
        store::sort_ordered(&mut self.notes, self.prefs.sort, self.prefs.oldest_first, &self.pinned);
        let mut tag_counts: HashMap<String, u32> = HashMap::new();
        for n in self.notes.iter().filter(|n| self.loc_of(&n.id) != folders::TRASH) {
            for tag in store::tags(&n.text) {
                *tag_counts.entry(tag).or_insert(0) += 1;
            }
        }
        let mut tag_list: Vec<(String, u32)> = tag_counts.into_iter().collect();
        tag_list.sort();
        self.tag_list = tag_list;
        if let Some(id) = open {
            if let Some(i) = self.notes.iter().position(|n| n.id == id) {
                self.screen = Screen::Editor(i);
            }
        }
        let query = self.search.text().to_string();
        let searching = !query.trim().is_empty();
        let ranked = store::search_ranked(&self.notes, &query, &self.pinned);
        let cutoff = self.smart_cutoff();
        self.shown = ranked.into_iter().filter(|&i| self.visible(i, searching, cutoff)).collect();
        self.shown_pinned = self.shown.iter().filter(|&&i| self.is_pinned(&self.notes[i].id)).count();
        let now = self.wall.now(t);
        self.sections = self.compute_sections(now, searching);
        let text_w = self.list_px(ui).1 - 2.0 * ui.pt(SPACE_S + SPACE_L);
        self.appearance = LABEL.now();
        self.rows = self
            .notes
            .iter()
            .map(|n| {
                let date = ui.text(&store::list_date(n.modified, now, self.wall.offset), MAC_SUBHEAD_PT, Weight::Regular, LABEL.now());
                let snippet_w = text_w - date.w as f32 - ui.pt(SPACE_S);
                // Recently Deleted rows say when the note goes for good
                let snippet = if self.loc_of(&n.id) == folders::TRASH { store::trash_caption(n.modified, now) } else { n.row_snippet() };
                Row {
                    title: ui.text(&ui.fit(n.title(), MAC_BODY_PT, text_w), MAC_BODY_PT, Weight::Bold, LABEL.now()),
                    date,
                    snippet: ui.text(&ui.fit(&snippet, MAC_SUBHEAD_PT, snippet_w), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now()),
                    title_text: ui.fit(n.title(), MAC_BODY_PT, text_w),
                    date_text: store::list_date(n.modified, now, self.wall.offset),
                    snippet_text: ui.fit(&snippet, MAC_SUBHEAD_PT, snippet_w),
                }
            })
            .collect();
        self.reveal = None;
        self.fit_scroll(ui);
        self.dirty = true;
    }

    fn kb_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - self.keyboard.h()
    }

    /// Bottom of the scrolling content: the keyboard (with its shortcut
    /// row in the editor), the find bar, else the window's bottom.
    fn content_bottom(&self, ui: &Ui) -> f32 {
        if self.keyboard.shown() {
            self.kb_top(ui) - if !matches!(self.screen, Screen::Editor(_)) { 0.0 } else { ui.pt(SHORTCUT_PT) }
        } else if self.find_strip() {
            self.find_top(ui)
        } else {
            self.h as f32 - home_zone(ui)
        }
    }

    /// The find bar (previous/next match) shows under the text while the
    /// keyboard is hidden and the open note has matches.
    fn find_strip(&self) -> bool {
        self.screen != Screen::List && !self.keyboard.shown() && !self.hl.is_empty()
    }

    fn find_top(&self, ui: &Ui) -> f32 {
        self.h as f32 - home_zone(ui) - ui.pt(SHORTCUT_PT)
    }

    /// A toolbar item's pixel rect (empty when it is not on the bar).
    fn bar_px(&self, ui: &Ui, id: u32) -> (f32, f32, f32, f32) {
        self.bar_rect(ui, id).map_or((0.0, 0.0, 0.0, 0.0), |r| chrome::px(ui, r))
    }

    /// The list header's More button: folders, sorting.
    fn title_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_px(ui, chrome::B_LIST_MORE)
    }

    fn editor_more_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_px(ui, chrome::B_MORE)
    }

    fn search_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_px(ui, chrome::B_SEARCH)
    }

    /// Screen rect of shown row `i` at the current scroll.
    fn row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        if self.prefs.layout == gallery::Layout::Gallery {
            let (cells, _, _) = self.gallery_grid(ui);
            let (x, y, w, h) = cells.get(i).copied().unwrap_or((0.0, 0.0, 0.0, 0.0));
            return (x, y - self.list_scroll.offset(), w, h);
        }
        let (lx, lw) = self.list_px(ui);
        let m = ui.pt(SPACE_S);
        let section = self.section_offset(i);
        (lx + m, ui.pt(list_top_pt() + section + ROW_PT * i as f32) - self.list_scroll.offset(), lw - 2.0 * m, ui.pt(ROW_PT))
    }

    #[cfg(test)]
    fn shown_pins(&self) -> usize {
        self.shown_pinned
    }

    #[cfg(test)]
    fn compose_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_px(ui, chrome::B_COMPOSE)
    }

    fn list_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let bottom = self.content_bottom(ui);
        if y < ui.pt(nav_bottom_pt()) || y >= bottom {
            return None;
        }
        if self.prefs.layout == gallery::Layout::Gallery {
            let (cells, _, _) = self.gallery_grid(ui);
            let off = self.list_scroll.offset();
            return Some(cells.iter().position(|&(rx, ry, rw, rh)| within(x, y, (rx, ry - off, rw, rh))).map_or(Target::List, Target::Row));
        }
        for i in 0..self.shown.len() {
            let (rx, ry, rw, rh) = self.row_rect(ui, i);
            if within(x, y, (rx, ry, rw, rh)) {
                if let Some((r, dx)) = self.reveal.filter(|&(r, _)| r == i) {
                    if dx > 0.0 && x < rx + dx {
                        return Some(Target::Pin(i));
                    }
                    return Some(if x >= rx + rw + dx { Target::Delete(r) } else { Target::Row(i) });
                }
                return Some(Target::Row(i));
            }
        }
        Some(Target::List)
    }

    // --- editor ---

    /// Done (hides the on-screen keyboard), at the top right of the editor while it is up.
    fn done_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (ex, ew) = self.ed_px(ui);
        (ex + ew - ui.pt(MARGIN_PT + 80.0), ui.pt(nav_bottom_pt()), ui.pt(80.0 + MARGIN_PT), ui.pt(SHORTCUT_PT))
    }

    #[cfg(test)]
    fn editor_share_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_px(ui, chrome::B_SHARE)
    }

    /// The note the user just chose to share, if any.
    pub fn take_share(&mut self) -> Option<springboard::shell::share::Item> {
        self.share_req.take()
    }

    /// The link the user tapped in a note, if any: the app opens it through the shell.
    pub fn take_link(&mut self) -> Option<String> {
        self.link_req.take()
    }

    /// The note the user asked to open in a window of its own, if any (`note_windows`).
    pub fn take_window_request(&mut self) -> Option<String> {
        self.window_req.take()
    }

    /// Note > Open in Separate Window: asks for the open note in a window of its own.
    pub fn request_window(&mut self) {
        self.window_req = self.editing().map(|n| n.id.clone());
    }

    /// The title of a note, for the title of its window.
    pub fn note_title(&self, id: &str) -> Option<String> {
        self.notes.iter().find(|n| n.id == id).map(|n| n.title().to_string())
    }

    /// Draws one of the note windows (`note_windows::draw`). The main window is drawn by `draw`.
    pub fn draw_window(&mut self, s: &mut Surface, ui: &Ui, window: bifrost_app::app_windows::WindowId) {
        let text = self.windows.note_of(window).map(str::to_owned);
        let text = text.as_deref().and_then(|id| self.notes.iter().find(|n| n.id == id)).map(|n| n.text.as_str());
        note_windows::draw(window, &mut self.windows, s, ui, text, self.note_settings);
    }

    /// The shortcut row above the keyboard while it is shown.
    fn shortcut_top(&self, ui: &Ui) -> Option<f32> {
        self.keyboard.shown().then(|| self.kb_top(ui) - ui.pt(SHORTCUT_PT))
    }

    fn shortcut_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let top = self.shortcut_top(ui)?;
        if y < top || y >= top + ui.pt(SHORTCUT_PT) {
            return None;
        }
        let m = ui.pt(MARGIN_PT);
        if x < m + ui.pt(UNDO_W_PT) {
            Some(Target::Undo)
        } else if x < m + ui.pt(2.0 * UNDO_W_PT) {
            Some(Target::Redo)
        } else if x >= self.w as f32 - m - ui.pt(60.0) {
            Some(Target::Format)
        } else {
            let slot = ((x - m - ui.pt(2.0 * UNDO_W_PT)) / ui.pt(FMT_W_PT)) as usize;
            let fits = m + ui.pt(2.0 * UNDO_W_PT) + ui.pt(FMT_W_PT) * (slot + 1) as f32 <= self.w as f32 - m - ui.pt(60.0);
            match FMT_BUTTONS.get(slot) {
                Some(&(_, f)) if fits => Some(Target::Fmt(f)),
                _ => Some(Target::Keyboard),
            }
        }
    }

    /// Where the text origin sits on screen at the current scroll.
    fn text_origin(&self, ui: &Ui) -> (f32, f32) {
        (self.ed_px(ui).0 + ui.pt(TEXT_INSET_PT), ui.pt(nav_bottom_pt() + TEXT_TOP_PT) - self.scroll.offset())
    }

    fn editor_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.find_strip() && y >= self.find_top(ui) && y < self.find_top(ui) + ui.pt(SHORTCUT_PT) {
            let m = self.ed_px(ui).0 + ui.pt(MARGIN_PT);
            return Some(if x < m + ui.pt(UNDO_W_PT) { Target::FindPrev } else if x < m + ui.pt(2.0 * UNDO_W_PT) { Target::FindNext } else { Target::Keyboard });
        }
        if self.keyboard.shown() && within(x, y, self.done_rect(ui)) {
            return Some(Target::Done);
        }
        if y < ui.pt(nav_bottom_pt()) || y >= self.content_bottom(ui) {
            return None;
        }
        if let Some((line, _, _)) = self.fold_chevrons(ui).into_iter().find(|(_, r, _)| within(x, y, *r)) {
            return Some(Target::Fold(line));
        }
        Some(Target::Text)
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if self.undo_toast.is_some() && self.pin_error.is_none() && self.notice.is_none() && !self.keyboard.shown() && within(x, y, self.toast_rect(ui)) {
            return Some(Target::UndoDelete);
        }
        if self.keyboard.shown() && y >= self.kb_top(ui) {
            return Some(Target::Keyboard);
        }
        if let Some(t) = self.shortcut_hit(ui, x, y) {
            return Some(t);
        }
        if let Some(id) = self.bar_hit(ui, x, y) {
            return Some(match id {
                chrome::B_NEW_FOLDER => Target::FolderNew,
                chrome::B_SIDEBAR => Target::SidebarToggle,
                chrome::B_LIST_MORE => Target::Title,
                chrome::B_COMPOSE => Target::Compose,
                chrome::B_STYLE => Target::Format,
                chrome::B_CHECK => Target::Fmt(Fmt::Check),
                chrome::B_BULLET => Target::Fmt(Fmt::Bullet),
                chrome::B_SHARE => Target::EditorShare,
                chrome::B_MORE => Target::EditorMore,
                chrome::B_SEARCH => Target::Search,
                other => Target::Bar(other),
            });
        }
        if y < ui.pt(nav_bottom_pt()) {
            return None;
        }
        if x < self.list_px(ui).0 {
            return self.sidebar_hit(ui, x, y).map(Target::Sidebar);
        }
        if x < self.ed_px(ui).0 {
            return self.list_hit(ui, x, y);
        }
        match self.screen {
            Screen::List => None,
            Screen::Editor(_) => self.editor_hit(ui, x, y),
        }
    }

    /// Keeps the scroll range in step with the content.
    fn fit_scroll(&mut self, ui: &Ui) {
        let bottom = self.content_bottom(ui);
        let section = self.sections_total();
        let list_h = if self.prefs.layout == gallery::Layout::Gallery {
            self.gallery_grid(ui).2 + ui.pt(SPACE_L)
        } else {
            ui.pt(list_top_pt() + section) + ui.pt(ROW_PT) * self.shown.len() as f32 + ui.pt(SPACE_L)
        };
        let text_h = ui.pt(nav_bottom_pt() + TEXT_TOP_PT) + self.editor.height() + ui.pt(60.0);
        self.list_scroll.set_max(list_h - bottom);
        self.scroll.set_max(text_h - bottom);
    }

    /// Scrolls so the caret is inside the editor's viewport.
    fn show_caret(&mut self, ui: &Ui) {
        self.fit_scroll(ui);
        let (_, cy, _, ch) = self.editor.caret_rect(ui);
        let top = ui.pt(nav_bottom_pt() + TEXT_TOP_PT) + cy;
        let (view_top, view_bottom) = (ui.pt(nav_bottom_pt()), self.content_bottom(ui));
        let pad = ui.pt(SPACE_M);
        let off = self.scroll.offset();
        if top + ch + pad - off > view_bottom {
            self.scroll.jump_to(top + ch + pad - view_bottom);
        } else if top - pad - off < view_top {
            // the first line brings the date above it back too
            self.scroll.jump_to(if self.editor.caret_line() == 0 { 0.0 } else { top - pad - view_top });
        }
    }

    /// What a relaunch restores (state restoration): the list's view and the id of the open note.
    pub fn restorable(&self) -> (View, Option<String>) {
        (self.view.clone(), self.editing().map(|n| n.id.clone()))
    }

    /// Puts back what `restorable` returned. A folder that was deleted since falls back to All Notes; a note that is
    /// gone leaves the list showing. True when a frame is needed.
    pub fn restore(&mut self, ui: &Ui, view: View, open: Option<&str>, t: f64) -> bool {
        let view = match view {
            View::Folder(f) if !self.folders.iter().any(|x| *x == f) => View::All,
            v => v,
        };
        if view != self.view {
            self.view = view;
            self.list_scroll.jump_top();
            self.reload(ui, t);
        }
        if let Some(id) = open {
            self.show_note(ui, id, t);
        }
        self.dirty = true;
        true
    }

    /// Opens a note by id (Search); false when it no longer exists.
    pub fn show_note(&mut self, ui: &Ui, id: &str, t: f64) -> bool {
        let Some(i) = self.notes.iter().position(|n| n.id == id) else { return false };
        self.open_note(ui, i, false, t);
        true
    }

    /// Index into `shown` of the arrow-selected row.
    fn selected_index(&self) -> Option<usize> {
        let id = self.list_selected.as_ref()?;
        self.shown.iter().position(|&i| &self.notes[i].id == id)
    }

    /// Moves the row selection by `step`; with none selected, down starts at the top and up at the bottom.
    fn move_selection(&mut self, step: i32) {
        let n = self.shown.len();
        if n == 0 {
            self.list_selected = None;
            return;
        }
        let next = match self.selected_index() {
            None if step > 0 => 0,
            None => n - 1,
            Some(i) => (i as i32 + step).clamp(0, n as i32 - 1) as usize,
        };
        self.list_selected = Some(self.notes[self.shown[next]].id.clone());
        self.dirty = true;
    }

    /// Release of a rightward swipe: a full swipe toggles the pin, a half swipe rests with Pin revealed.
    fn release_lead(&mut self, ui: &Ui, row: usize, dx: f32, rw: f32, t: f64) {
        if dx >= rw * FULL_SWIPE {
            self.reveal = None;
            if let Some(&i) = self.shown.get(row) {
                let id = self.notes[i].id.clone();
                self.set_pinned(ui, &id, !self.is_pinned(&id), t);
            }
        } else if dx >= ui.pt(PIN_W_PT) / 2.0 {
            self.reveal = Some((row, ui.pt(PIN_W_PT)));
        } else {
            self.reveal = None;
        }
    }

    fn open_note(&mut self, ui: &Ui, i: usize, focus: bool, t: f64) {
        // the note open beside the list is saved (or dropped when empty) first
        if let Screen::Editor(j) = self.screen {
            if j == i {
                self.editor.focused |= focus;
                return;
            }
            let id = self.notes[i].id.clone();
            self.close_note(ui, t);
            let Some(k) = self.notes.iter().position(|n| n.id == id) else { return };
            return self.open_note(ui, k, focus, t);
        }
        self.screen = Screen::Editor(i);
        self.hl_query = self.search.text().trim().to_string();
        self.hl_cur = 0;
        self.menu = None;
        self.editor.set_text(ui, &self.notes[i].text);
        self.load_images(ui);
        self.editor.focused = focus;
        self.refresh_date(ui);
        self.refresh_highlights(ui, false);
        self.scroll.jump_top();
        self.keyboard.set_shown(focus && !self.hw_kb, t);
        self.searching = false;
        self.search.focused = false;
        self.fit_scroll(ui);
        self.scroll_to_match(ui);
        self.dirty = true;
    }

    /// Decodes the open note's attached images (bounded) for inline display.
    fn load_images(&mut self, ui: &Ui) {
        if let Screen::Editor(i) = self.screen {
            let width = self.editor_w.max(1.0) as usize;
            let (dir, text) = (self.note_dir(&self.notes[i].id), self.notes[i].text.clone());
            if !text.lines().any(|l| md::image_ref(l).is_some()) {
                self.images_slot = None;
                self.editor.set_images(ui, HashMap::new());
                return;
            }
            // Decoding up to 12 MP pictures takes long enough to freeze the UI: the note shows
            // its `![image](..)` lines meanwhile and `tick` swaps the pictures in. A newer load
            // replaces the slot, so a stale result is dropped.
            let slot: ImageSlot = Arc::default();
            let sink = slot.clone();
            DispatchQueue::global(Qos::Utility).async_(move || {
                let images = attach::load_images(&dir, &text, width);
                DispatchQueue::main().async_(move || *sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(images));
            });
            self.images_slot = Some(slot);
        }
    }

    /// Takes finished image decodes; true when the text was laid out again.
    fn poll_images(&mut self, ui: &Ui) -> bool {
        let Some(taken) = self.images_slot.as_ref().map(|slot| slot.lock().unwrap_or_else(|e| e.into_inner()).take()) else { return false };
        match taken {
            Some(images) => {
                self.images_slot = None;
                self.editor.set_images(ui, images);
                self.fit_scroll(ui);
                true
            }
            None => false,
        }
    }

    /// The date line above the text, with the word count.
    fn refresh_date(&mut self, ui: &Ui) {
        if let Screen::Editor(i) = self.screen {
            let words = md::word_count(&self.notes[i].text);
            let mut label = format!("{} - {} word{}", store::long_date(self.notes[i].modified, self.wall.offset), words, if words == 1 { "" } else { "s" });
            // The note's language, when its text has letters to judge by (Edit > Language is detected, not set).
            if let Some(name) = parity::language_label(&self.notes[i].text) {
                label.push_str(" - ");
                label.push_str(name);
            }
            self.date = Some(ui.text(&label, MAC_CAPTION1_PT, Weight::Regular, LABEL_2.now()));
        }
    }

    /// Recomputes the search matches in the open note.
    fn refresh_highlights(&mut self, ui: &Ui, jump: bool) {
        self.hl = if self.hl_query.is_empty() { vec![] } else { parity::find_ranges(self.editor.text(), &self.hl_query, self.whole_word) };
        if self.hl_cur >= self.hl.len() {
            self.hl_cur = 0;
        }
        self.editor.set_highlights(self.hl.clone(), (!self.hl.is_empty()).then_some(self.hl_cur));
        if jump {
            self.scroll_to_match(ui);
        }
    }

    /// Check Spelling and Links (editor menu): flags the misspelled words, links, dates and phone numbers of the
    /// open note, and steps through them with the find arrows. The next edit ends the check, as a search does.
    fn check_note(&mut self, ui: &Ui) {
        if !self.is_editing() {
            return;
        }
        self.hl_query.clear();
        self.hl = parity::check_ranges(self.editor.text());
        self.hl_cur = 0;
        self.editor.set_highlights(self.hl.clone(), (!self.hl.is_empty()).then_some(0));
        self.scroll_to_match(ui);
        self.dirty = true;
    }

    /// Proofread (editor menu): the writing assistant reads the selection, or the whole note, and the menu lists what it
    /// would change. Nothing changes until a row is chosen.
    fn proofread_note(&mut self, ui: &Ui) {
        use bifrost_app::controls::{proofread::Options, proofread_panel::Panel};
        if !self.is_editing() {
            return;
        }
        let text = self.editor.text().to_string();
        let byte_of = |n: usize| text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
        let selection = self.editor.selection().map_or(0..0, |r| byte_of(r.start)..byte_of(r.end));
        match Panel::open(&text, selection, Options { english: parity::note_is_english(&text) }) {
            Ok(panel) => {
                self.proofread = Some(panel);
                self.show_proofread(ui);
            }
            Err(e) => eprintln!("notes: proofread: {e:?}"),
        }
    }

    /// The proofreading menu: one row per suggestion, Accept All when there are several.
    fn show_proofread(&mut self, ui: &Ui) {
        let items = {
            let Some(panel) = &self.proofread else { return };
            let text = self.editor.text();
            let mut items: Vec<(String, Act, bool)> = (0..panel.len()).filter_map(|i| panel.brief(text, i).map(|label| (label, Act::ProofreadAccept(i), false))).collect();
            if items.is_empty() {
                items.push(("No Suggestions".to_string(), Act::ProofreadDone, false));
            } else if items.len() > 1 {
                items.push(("Accept All".to_string(), Act::ProofreadAll, false));
            }
            items
        };
        let (mx, my, _, mh) = self.editor_more_rect(ui);
        self.show_menu(ui, items, mx, my + mh + ui.pt(SPACE_XS));
    }

    /// Accepts suggestion `index` of the proofreading menu, or every one left (`None`), in the note's own editor so Undo
    /// works. The menu comes back with what is left.
    fn proofread_accept(&mut self, ui: &Ui, index: Option<usize>, t: f64) {
        let Some(mut panel) = self.proofread.take() else { return };
        let mut text = self.editor.text().to_string();
        let steps = match index {
            Some(i) => panel.accept_at(&text, i).map(|(step, _)| vec![step]),
            None => panel.accept_all(&text),
        };
        let Ok(steps) = steps else { return };
        for step in steps {
            let chars = text[..step.range.start].chars().count()..text[..step.range.end].chars().count();
            self.editor.replace_range(ui, chars, &step.replacement, t, false);
            text = self.editor.text().to_string();
        }
        self.changed(ui, t);
        if !panel.is_empty() {
            self.proofread = Some(panel);
            self.show_proofread(ui);
        }
    }

    /// Scrolls the current match into view.
    fn scroll_to_match(&mut self, ui: &Ui) {
        if let Some(r) = self.hl.get(self.hl_cur).cloned() {
            self.fit_scroll(ui);
            let y = self.editor.y_of(r.start) + ui.pt(TEXT_TOP_PT) - ui.pt(60.0);
            self.scroll.jump_to(y.max(0.0));
        }
    }

    /// Steps to the next (`1`) or previous (`-1`) match.
    pub fn step_match(&mut self, ui: &Ui, delta: i32) {
        self.hl_cur = store::step_match(self.hl_cur, self.hl.len(), delta);
        self.editor.set_highlights(self.hl.clone(), (!self.hl.is_empty()).then_some(self.hl_cur));
        self.scroll_to_match(ui);
        self.dirty = true;
    }

    /// (current match, total) in the open note.
    pub fn match_state(&self) -> (usize, usize) {
        (if self.hl.is_empty() { 0 } else { self.hl_cur + 1 }, self.hl.len())
    }

    fn new_note(&mut self, ui: &Ui, t: f64) {
        if matches!(self.screen, Screen::Editor(_)) {
            self.close_note(ui, t);
        }
        let id = self.fresh_id(t);
        let text = self.note_settings.new_note_start.prefix().to_string();
        self.notes.insert(0, Note { id: id.clone(), text, modified: self.wall.now(t) });
        if let View::Folder(f) = &self.view {
            self.loc.insert(id.clone(), f.clone());
        }
        self.search.set_text(ui, "");
        self.rebuild(ui, t);
        let i = self.notes.iter().position(|note| note.id == id).unwrap();
        self.open_note(ui, i, true, t);
    }

    /// A new note holding `text` (File > Import: a text file chosen in the open panel). It is saved at once and opens
    /// in the editor, in the folder shown, like a composed note.
    pub fn import_note(&mut self, ui: &Ui, text: &str, t: f64) -> String {
        if matches!(self.screen, Screen::Editor(_)) {
            self.close_note(ui, t);
        }
        let id = self.fresh_id(t);
        self.notes.insert(0, Note { id: id.clone(), text: text.to_string(), modified: self.wall.now(t) });
        if let View::Folder(f) = &self.view {
            self.loc.insert(id.clone(), f.clone());
        }
        self.search.set_text(ui, "");
        self.unsaved = Some(id.clone());
        self.autosave.touch(t);
        self.save(t, true);
        self.rebuild(ui, t);
        if let Some(i) = self.notes.iter().position(|note| note.id == id) {
            self.open_note(ui, i, false, t);
        }
        id
    }

    /// New Note (File menu, Cmd+N): the same path as the Compose button.
    pub fn compose(&mut self, ui: &Ui, t: f64) {
        self.activate(ui, Target::Compose, 0.0, 0.0, t);
    }

    /// Find (Edit menu, Cmd+F): the same path as the search field.
    pub fn find(&mut self, ui: &Ui, t: f64) {
        self.activate(ui, Target::Search, 0.0, 0.0, t);
    }

    /// Delete Note (Note menu, Cmd+Backspace): the open note, or else the arrow-selected row on the list.
    /// True while a note is open in the editor (Edit > Undo and Redo apply).
    pub fn is_editing(&self) -> bool {
        matches!(self.screen, Screen::Editor(_))
    }

    /// Runs a Format menu command on the open note; false when no note is open.
    pub fn format(&mut self, ui: &Ui, f: Format, t: f64) -> bool {
        if !self.is_editing() {
            return false;
        }
        self.do_format(ui, f, t);
        self.dirty = true;
        true
    }

    /// Gives the paragraphs of the selection (or the caret's) a paragraph style; false when no note is open.
    pub fn style(&mut self, ui: &Ui, block: md::Block, t: f64) -> bool {
        if !self.is_editing() {
            return false;
        }
        let sel = self.editor.selection().unwrap_or(self.editor.caret()..self.editor.caret());
        self.restyle(ui, sel, block, t);
        true
    }

    /// The table commands that apply at the caret (empty when the caret is not in a table or no note is open).
    pub fn table_ops(&self) -> Vec<table::Op> {
        if self.is_editing() { table::available(self.editor.text(), self.editor.caret()) } else { vec![] }
    }

    /// Whether the open note has a checklist (Mark All and Unmark All apply).
    pub fn has_checklist(&self) -> bool {
        self.editing().is_some_and(|n| n.checklist().1 > 0)
    }

    /// Edit > Paste as Markdown: the pasteboard text goes in as it is, so Markdown marks become formatting.
    pub fn paste_markdown(&mut self, ui: &Ui, t: f64) -> bool {
        if !self.is_editing() {
            return false;
        }
        let Some(text) = springboard::shell::clipboard::text() else { return false };
        self.paste_text(ui, &text, t);
        true
    }

    /// Edit > Paste and Match Style: the pasteboard text with its Markdown marks removed, so it takes the style of
    /// the paragraph it lands in.
    pub fn paste_match_style(&mut self, ui: &Ui, t: f64) -> bool {
        if !self.is_editing() {
            return false;
        }
        let Some(text) = springboard::shell::clipboard::text() else { return false };
        self.paste_text(ui, &md::to_plain(&text), t);
        true
    }

    fn paste_text(&mut self, ui: &Ui, text: &str, t: f64) {
        self.editor.replace_selection(ui, text, t);
        self.changed(ui, t);
    }

    /// Math Results on or off (the Note menu).
    pub fn math_results(&self) -> bool {
        self.prefs.math_results
    }

    pub fn toggle_math_results(&mut self) {
        self.prefs.math_results = !self.prefs.math_results;
        self.save_prefs();
        self.dirty = true;
    }

    /// The newest notes outside Recently Deleted as `(id, title)`, for the Recent Notes menu.
    pub fn recent_notes(&self) -> Vec<(String, String)> {
        let mut live: Vec<&Note> = self.notes.iter().filter(|n| self.loc_of(&n.id) != folders::TRASH).collect();
        live.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
        live.into_iter().take(RECENT_MENU).map(|n| (n.id.clone(), n.title().to_string())).collect()
    }

    /// Opens the note that `query` names: its id, else the newest note with that title (any case), else the
    /// newest whose title contains it. The note's title, or `None` when nothing matches.
    pub fn open_named(&mut self, ui: &Ui, query: &str, t: f64) -> Option<String> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return None;
        }
        let newest = |keep: &dyn Fn(&str) -> bool| {
            self.notes
                .iter()
                .enumerate()
                .filter(|(_, n)| self.loc_of(&n.id) != folders::TRASH && keep(&n.title().to_lowercase()))
                .max_by_key(|(_, n)| n.modified)
                .map(|(i, _)| i)
        };
        let by_id = self.notes.iter().position(|n| n.id == query.trim() && self.loc_of(&n.id) != folders::TRASH);
        let i = by_id.or_else(|| newest(&|title| title == q)).or_else(|| newest(&|title| title.contains(&q)))?;
        let title = self.notes[i].title().to_string();
        self.open_note(ui, i, false, t);
        Some(title)
    }

    /// Shows a folder in the list (the Open Folder intent); false when there is no such folder.
    pub fn open_folder(&mut self, ui: &Ui, name: &str, t: f64) -> bool {
        let Some(folder) = self.folders.iter().find(|f| f.eq_ignore_ascii_case(name.trim())).cloned() else { return false };
        self.show_view(ui, View::Folder(folder), t);
        self.dirty = true;
        true
    }

    /// New Quick Note from the menu bar. With `resume` (or the "resumes last" setting) and a quick note still on
    /// disk, that note opens again; otherwise a new note starts and is recorded as the quick note. `resume` alone
    /// never starts a note.
    pub fn quick_note(&mut self, ui: &Ui, resume: bool, t: f64) {
        if resume || self.prefs.resume_quick {
            if let Some(id) = store::load_quick(&self.dir) {
                if self.notes.iter().any(|n| n.id == id && self.loc_of(&n.id) != folders::TRASH) {
                    self.show_note(ui, &id, t);
                    return;
                }
            }
        }
        if resume {
            return;
        }
        self.compose(ui, t);
        if let Some(id) = self.editing().map(|n| n.id.clone()) {
            if let Err(error) = store::save_quick(&self.dir, &id) {
                self.notice = Some(format!("Unable to record the quick note: {error}"));
            }
        }
    }

    /// The selected Markdown of the open note (Edit > Copy), or `None` when no note is open or nothing is selected.
    /// The selection is a char range, as the text area keeps it.
    pub fn selected_markdown(&self) -> Option<String> {
        if !self.is_editing() {
            return None;
        }
        let r = self.editor.selection()?;
        if r.end <= r.start {
            return None;
        }
        Some(self.editor.text().chars().skip(r.start).take(r.end - r.start).collect())
    }

    /// The open note's text for printing: what the editor holds, or `None` when no note is open.
    pub fn printable_text(&self) -> Option<String> {
        if !self.is_editing() {
            return None;
        }
        Some(self.editor.text().to_string())
    }

    /// The text Edit > Speech speaks: the selection of the open note, else the whole note. `None` when no note is open
    /// or the note is blank.
    pub fn speakable_text(&self) -> Option<String> {
        let text = match self.selected_markdown() {
            Some(selected) => selected,
            None => self.printable_text()?,
        };
        Some(text).filter(|t| !t.trim().is_empty())
    }

    /// Whether Edit > Undo and Redo have something to do in the open note.
    pub fn can_undo_redo(&self) -> (bool, bool) {
        (self.is_editing() && self.editor.can_undo(), self.is_editing() && self.editor.can_redo())
    }

    /// The Edit menu titles of the open note's undo history ("Undo Typing").
    pub fn undo_titles(&self) -> (String, String) {
        if self.is_editing() { self.editor.undo_titles() } else { Default::default() }
    }

    /// A new note id from the wall clock: milliseconds since the epoch, with the sub-second part of `t` as the tie-break.
    fn fresh_id(&self, t: f64) -> String {
        let ms = std::time::Duration::from_secs(1).as_millis() as f64;
        store::new_id((self.wall.now(t) as f64 * ms + (t % ms)) as i64, &self.notes)
    }

    pub fn can_delete(&self) -> bool {
        match self.screen {
            Screen::Editor(_) => true,
            Screen::List => self.selected_index().is_some(),
        }
    }

    /// Runs Delete Note; false when there is no note to delete.
    pub fn delete_current(&mut self, ui: &Ui, t: f64) -> bool {
        match self.screen {
            Screen::Editor(_) => self.run(ui, Act::DeleteNote, t),
            Screen::List => match self.selected_index() {
                Some(row) => self.delete_row(ui, row, t),
                None => return false,
            },
        }
        self.rebuild(ui, t);
        true
    }

    /// Writes the open note now (when `now`) or once the debounce lapses.
    fn save(&mut self, t: f64, now: bool) {
        if !self.autosave.fire(t, now) {
            return;
        }
        let Some(id) = self.unsaved.take() else { return };
        if let Some(note) = self.notes.iter().find(|n| n.id == id) {
            notes_log("save").debug(format!("saving note {}", bifrost_app::log::private(&id)));
            if let Err(e) = store::save(&self.note_dir(&note.id), note) {
                notes_log("save").error(format!("saving {}: {e}", bifrost_app::log::private(&id)));
            }
        }
    }

    /// Writes any unsaved edit now (the app is being closed for good).
    pub fn flush(&mut self) {
        self.save(0.0, true);
    }

    /// The editor changed: copy its text to the note and arm the autosave.
    fn changed(&mut self, ui: &Ui, t: f64) {
        if let Screen::Editor(i) = self.screen {
            self.notes[i].text = self.editor.text().to_string();
            self.notes[i].modified = self.wall.now(t);
            self.unsaved = Some(self.notes[i].id.clone());
            self.autosave.touch(t);
        }
        self.refresh_date(ui);
        self.refresh_highlights(ui, false);
        self.show_caret(ui);
        self.dirty = true;
    }

    /// Leaves the editor, saving (or dropping an empty note).
    fn close_note(&mut self, ui: &Ui, t: f64) {
        if let Screen::Editor(i) = self.screen {
            self.save(t, true);
            if self.notes[i].text.trim().is_empty() {
                self.erase(i);
            }
        }
        self.hl.clear();
        self.hl_query.clear();
        self.editor.set_highlights(vec![], None);
        self.menu = None;
        self.screen = Screen::List;
        self.keyboard.set_shown(false, t);
        self.editor.focused = false;
        self.rebuild(ui, t);
    }

    /// Permanently removes note `i` (files, attachments and pin).
    fn erase(&mut self, i: usize) {
        let id = self.notes[i].id.clone();
        folders::erase(&self.dir, &id, self.folder_arg(&id).as_deref());
        if let Err(error) = store::set_pinned(&self.dir, &id, false) {
            eprintln!("springboard: notes: removing pin marker: {error}");
        }
        self.pinned.remove(&id);
        self.loc.remove(&id);
        self.notes.remove(i);
        // the editor holds an index into `notes`
        match self.screen {
            Screen::Editor(j) if j == i => {
                self.screen = Screen::List;
                self.editor.focused = false;
            }
            Screen::Editor(j) if j > i => self.screen = Screen::Editor(j - 1),
            _ => {}
        }
    }

    /// Moves note `i` to Recently Deleted; false (with a notice) on failure.
    fn trash_note(&mut self, i: usize, t: f64) -> bool {
        let id = self.notes[i].id.clone();
        let from = self.folder_arg(&id);
        if let Err(error) = folders::trash(&self.dir, &id, from.as_deref(), self.wall.now(t)) {
            self.notice = Some(format!("Unable to delete note: {error}"));
            return false;
        }
        if let Err(error) = store::set_pinned(&self.dir, &id, false) {
            eprintln!("springboard: notes: removing pin marker: {error}");
        }
        self.pinned.remove(&id);
        self.loc.insert(id.clone(), folders::TRASH.to_string());
        self.undo_toast = Some(UndoToast { id, from, until: t + UNDO_TOAST_MS });
        true
    }

    /// Undo after a delete: the note goes back to the folder it came from.
    fn undo_delete(&mut self, ui: &Ui, t: f64) {
        let Some(toast) = self.undo_toast.take() else { return };
        self.dirty = true;
        match folders::move_note(&self.dir, &toast.id, Some(folders::TRASH), toast.from.as_deref()) {
            Ok(()) => {
                match toast.from {
                    Some(folder) => self.loc.insert(toast.id, folder),
                    None => self.loc.remove(&toast.id),
                };
                self.rebuild(ui, t);
            }
            Err(error) => self.notice = Some(format!("Unable to undo delete: {error}")),
        }
    }

    /// The Undo bar: the bottom notice slot, above the keyboard.
    fn toast_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let y = self.content_bottom(ui) - ui.pt(SPACE_HUGE);
        let (lx, lw) = self.list_px(ui);
        (lx + ui.pt(SPACE_M), y, lw - ui.pt(SPACE_XL), ui.pt(36.0))
    }

    /// Swipe-delete: into Recently Deleted, or for good from inside it.
    fn delete_row(&mut self, ui: &Ui, row: usize, t: f64) {
        if let Some(&i) = self.shown.get(row) {
            if self.loc_of(&self.notes[i].id) == folders::TRASH {
                self.erase(i);
            } else {
                self.trash_note(i, t);
            }
            self.rebuild(ui, t);
        }
    }

    /// Copies the open note (text and attached images) into the same folder
    /// and opens the copy.
    fn duplicate_open_note(&mut self, ui: &Ui, t: f64) {
        let Screen::Editor(i) = self.screen else { return };
        self.save(t, true);
        let src = self.notes[i].id.clone();
        let folder = self.folder_arg(&src);
        let id = self.fresh_id(t);
        let text = match folders::duplicate(&self.dir, &src, &id, folder.as_deref(), &self.notes[i].text) {
            Ok(text) => text,
            Err(error) => {
                self.notice = Some(format!("Unable to duplicate note: {error}"));
                return;
            }
        };
        if let Some(f) = &folder {
            self.loc.insert(id.clone(), f.clone());
        }
        let note = Note { id: id.clone(), text, modified: self.wall.now(t) };
        if let Err(error) = store::save(&self.note_dir(&id), &note) {
            self.notice = Some(format!("Unable to duplicate note: {error}"));
            return;
        }
        self.close_note(ui, t);
        self.notes.insert(0, note);
        self.search.set_text(ui, "");
        self.rebuild(ui, t);
        let i = self.notes.iter().position(|n| n.id == id).unwrap();
        self.open_note(ui, i, false, t);
    }

    /// Removes every ticked checklist item from the open note, as one undo step.
    fn clear_checked(&mut self, ui: &Ui, t: f64) {
        let old = self.editor.text().to_string();
        let kept: Vec<&str> = old.split('\n').filter(|l| !matches!(md::classify(l).block, md::Block::Check(true))).collect();
        let new = kept.join("\n");
        if new == old {
            return;
        }
        let caret = self.editor.caret().min(new.chars().count());
        self.rewrite(ui, &new, caret, t);
    }

    /// Runs a Format menu command on the open note.
    fn do_format(&mut self, ui: &Ui, f: Format, t: f64) {
        if !matches!(self.screen, Screen::Editor(_)) {
            return;
        }
        match f {
            Format::Char(fmt) => self.apply_fmt(ui, fmt, t),
            Format::Indent => self.shift_indent(ui, true, t),
            Format::Outdent => self.shift_indent(ui, false, t),
            Format::MoveUp => self.move_items(ui, true, t),
            Format::MoveDown => self.move_items(ui, false, t),
            Format::CheckAll => self.set_checks(ui, true, t),
            Format::UncheckAll => self.set_checks(ui, false, t),
            Format::Table(op) => self.edit_table(ui, op, t),
            Format::CalculateMath => self.calculate(ui, t),
            Format::CollapseSection | Format::ExpandSection => self.fold_section(ui, matches!(f, Format::CollapseSection)),
            Format::CollapseAll | Format::ExpandAll => self.fold_all(ui, matches!(f, Format::CollapseAll)),
            Format::AddLink => self.start_link(ui, t),
        }
    }

    /// Ticks or unticks every checklist item of the open note, as one undo step.
    fn set_checks(&mut self, ui: &Ui, done: bool, t: f64) {
        let mut new = md::set_all_checks(self.editor.text(), done);
        // Move Checked to Bottom applies to Mark All too, as it does to a single tick
        if self.prefs.checked_to_bottom {
            new = md::checked_to_bottom(&new);
        }
        let caret = self.editor.caret().min(new.chars().count());
        self.rewrite(ui, &new, caret, t);
    }

    /// Tab and Shift+Tab (and the Format menu): one level more or less on the list lines of the selection or
    /// the caret's paragraph. Other lines are left alone.
    fn shift_indent(&mut self, ui: &Ui, more: bool, t: f64) {
        let caret = self.editor.caret();
        let sel = self.editor.selection().unwrap_or(caret..caret);
        let a = self.editor.paragraph_of(sel.start).start;
        let b = self.editor.paragraph_of(sel.end).end;
        let part: String = self.editor.text().chars().skip(a).take(b - a).collect();
        let Some(new) = (if more { outline::indent(&part) } else { outline::outdent(&part) }) else { return };
        let (old_len, new_len) = (part.chars().count(), new.chars().count());
        self.editor.replace_range(ui, a..b, &new, t, false);
        let target = if part.contains('\n') { a + new_len } else { (caret + new_len).saturating_sub(old_len).max(a) };
        self.editor.set_caret(target);
        self.changed(ui, t);
    }

    /// Ctrl+Cmd+Up and Down (and the Format menu): the list item under the caret, with the items nested under
    /// it, passes its neighbour.
    fn move_items(&mut self, ui: &Ui, up: bool, t: f64) {
        let text = self.editor.text().to_string();
        let caret = self.editor.caret();
        let sel = self.editor.selection().unwrap_or(caret..caret);
        let line_of = |c: usize| text.chars().take(c).filter(|&ch| ch == '\n').count();
        let (first, last) = (line_of(self.editor.paragraph_of(sel.start).start), line_of(self.editor.paragraph_of(sel.end).start));
        let Some(moved) = outline::move_lines(&text, first, last, up) else { return };
        let target = (caret as isize + moved.shift).max(0) as usize;
        self.rewrite(ui, &moved.text, target, t);
    }

    /// A table command from the Format menu, applied to the table under the caret.
    fn edit_table(&mut self, ui: &Ui, op: table::Op, t: f64) {
        let text = self.editor.text().to_string();
        let Some(edit) = table::apply(&text, self.editor.caret(), op) else { return };
        self.editor.replace_range(ui, edit.range, &edit.with, t, false);
        self.editor.set_caret(edit.caret);
        self.changed(ui, t);
    }

    /// Calculate Math Results: every line that ends with `=` after a calculation gets its answer.
    fn calculate(&mut self, ui: &Ui, t: f64) {
        let old = self.editor.text().to_string();
        let new = math::complete_text(&old);
        if new == old {
            return;
        }
        let caret = self.editor.caret();
        let para_end = self.editor.paragraph_of(caret).end;
        let head: String = old.chars().take(para_end).collect();
        let grown = math::complete_text(&head).chars().count().saturating_sub(head.chars().count());
        let target = (caret + grown).min(new.chars().count());
        self.rewrite(ui, &new, target, t);
    }

    /// With Math Results on, typing `=` at the end of a calculation writes the answer after it.
    fn answer_math(&mut self, ui: &Ui, t: f64) {
        let caret = self.editor.caret();
        let para = self.editor.paragraph_of(caret);
        if caret != para.end {
            return;
        }
        let line: String = self.editor.text().chars().skip(para.start).take(para.len()).collect();
        if let Some(answer) = math::result_for_line(&line) {
            self.editor.replace_range(ui, caret..caret, &format!(" {answer}"), t, false);
        }
    }

    /// Cycles the current paragraph through body, heading ("## ") and
    /// title ("# ") markers.
    fn cycle_format(&mut self, ui: &Ui, t: f64) {
        let text: Vec<char> = self.editor.text().chars().collect();
        let caret = self.editor.caret();
        let start = text[..caret].iter().rposition(|&c| c == '\n').map_or(0, |p| p + 1);
        let para: String = text[start..].iter().take_while(|&&c| c != '\n').collect();
        let (old, new) = if para.starts_with("# ") {
            (2, "")
        } else if para.starts_with("## ") {
            (3, "# ")
        } else {
            (0, "## ")
        };
        self.editor.replace_range(ui, start..start + old, new, t, false);
        self.changed(ui, t);
    }

    /// Replaces the editor text with `new` as one small undo step, keeping the
    /// caret near `caret`.
    fn rewrite(&mut self, ui: &Ui, new: &str, caret: usize, t: f64) {
        let (range, with) = md::diff_range(self.editor.text(), new);
        if range.is_empty() && with.is_empty() {
            return;
        }
        self.editor.replace_range(ui, range, &with, t, false);
        self.editor.set_caret(caret);
        self.changed(ui, t);
    }

    /// Ticks or unticks the checklist item of the paragraph starting at
    /// `para_start`; ticked items sink to the bottom when that option is on.
    fn toggle_checkbox(&mut self, ui: &Ui, para_start: usize, t: f64) {
        let para = self.editor.paragraph_of(para_start);
        let old = self.editor.text().to_string();
        let line: String = old.chars().skip(para.start).take(para.len()).collect();
        let Some(flipped) = md::toggle_check(&line) else { return };
        let mut new: String = old.chars().take(para.start).chain(flipped.chars()).chain(old.chars().skip(para.end)).collect();
        if self.prefs.checked_to_bottom && md::classify(&flipped).block == md::Block::Check(true) {
            new = md::checked_to_bottom(&new);
        }
        let caret = self.editor.caret();
        self.rewrite(ui, &new, caret, t);
    }

    /// Applies a formatting button to the selection (or the caret's paragraph).
    fn apply_fmt(&mut self, ui: &Ui, f: Fmt, t: f64) {
        let text = self.editor.text().to_string();
        let sel = self.editor.selection().unwrap_or(self.editor.caret()..self.editor.caret());
        match f {
            Fmt::Bold | Fmt::Italic | Fmt::Code | Fmt::Strike | Fmt::Underline | Fmt::Highlight => {
                let marker = match f {
                    Fmt::Bold => "**",
                    Fmt::Italic => "*",
                    Fmt::Strike => "~~",
                    Fmt::Underline => "++",
                    Fmt::Highlight => "==",
                    _ => "`",
                };
                let r = md::toggle_wrap(&text, sel, marker);
                self.editor.replace_range(ui, r.range, &r.with, t, false);
                self.editor.select(r.select);
            }
            Fmt::Bullet | Fmt::Number | Fmt::Check => {
                let block = match f {
                    Fmt::Bullet => md::Block::Bullet,
                    Fmt::Number => md::Block::Numbered(1),
                    _ => md::Block::Check(false),
                };
                self.restyle(ui, sel, block, t);
                return;
            }
        }
        self.changed(ui, t);
    }

    /// Gives every paragraph touched by `sel` the paragraph style `block`.
    fn restyle(&mut self, ui: &Ui, sel: Range<usize>, block: md::Block, t: f64) {
        let text = self.editor.text().to_string();
        let a = self.editor.paragraph_of(sel.start).start;
        let b = self.editor.paragraph_of(sel.end).end;
        let part: String = text.chars().skip(a).take(b - a).collect();
        let new = md::set_block_lines(&part, block);
        let end = a + new.chars().count();
        self.editor.replace_range(ui, a..b, &new, t, false);
        self.editor.set_caret(end);
        self.changed(ui, t);
    }

    // --- menus ---

    fn menu_rect(&self, ui: &Ui, menu: &Menu) -> (f32, f32, f32, f32) {
        (menu.x, menu.y, ui.pt(MENU_W_PT), ui.pt(MENU_ROW_PT) * menu.items.len() as f32 + ui.pt(2.0 * MENU_PAD_PT))
    }

    /// Places a menu of `items` with its top-left near (x, y), kept on screen.
    fn show_menu(&mut self, ui: &Ui, items: Vec<(String, Act, bool)>, x: f32, y: f32) {
        let (w, h) = (ui.pt(MENU_W_PT), ui.pt(MENU_ROW_PT) * items.len() as f32 + ui.pt(2.0 * MENU_PAD_PT));
        let x = x.min(self.w as f32 - w - ui.pt(SPACE_S)).max(ui.pt(SPACE_S));
        let y = y.min(self.h as f32 - home_zone(ui) - h - ui.pt(SPACE_S)).max(ui.pt(top_inset()));
        self.menu = Some(Menu { items, x, y });
    }

    fn open_list_menu(&mut self, ui: &Ui) {
        let count = |loc: Option<&str>| self.notes.iter().filter(|n| match loc {
            None => self.loc_of(&n.id) != folders::TRASH,
            Some(l) => self.loc_of(&n.id) == l,
        }).count();
        let mut items = vec![(format!("All Notes\t{}", count(None)), Act::View(View::All), self.view == View::All)];
        for f in &self.folders {
            items.push((format!("{f}\t{}", count(Some(f))), Act::View(View::Folder(f.clone())), self.view == View::Folder(f.clone())));
        }
        items.push((format!("Recently Deleted\t{}", count(Some(folders::TRASH))), Act::View(View::Trash), self.view == View::Trash));
        for s in smart::Smart::ALL {
            let view = View::Smart(s);
            let current = self.view == view;
            items.push((format!("{}\t{}", s.label(), self.count_in(&view)), Act::View(view), current));
        }
        for (tag, n) in self.tag_list.iter().take(MENU_TAGS) {
            let view = View::Tag(tag.clone());
            let current = self.view == view;
            items.push((format!("#{tag}\t{n}"), Act::View(view), current));
        }
        items.push(("New Folder".to_string(), Act::NewFolder, false));
        if matches!(self.view, View::Folder(_)) {
            items.push(("Rename Folder".to_string(), Act::RenameFolder, false));
            items.push(("Delete Folder".to_string(), Act::DeleteFolder, false));
        }
        for k in store::SortKey::ALL {
            items.push((format!("Sort by {}", k.label()), Act::Sort(k), self.prefs.sort == k));
        }
        items.push(("Group by Date".to_string(), Act::ToggleGroupDate, self.prefs.group_by_date));
        items.push(("Newest First".to_string(), Act::SortOrder(false), !self.prefs.oldest_first));
        items.push(("Oldest First".to_string(), Act::SortOrder(true), self.prefs.oldest_first));
        items.push(("Move Checked to Bottom".to_string(), Act::ToggleCheckedBottom, self.prefs.checked_to_bottom));
        items.push(("New Quick Note Resumes Last".to_string(), Act::ToggleResumeQuick, self.prefs.resume_quick));
        let (mx, my, mw, mh) = self.title_rect(ui);
        self.show_menu(ui, items, mx + mw - ui.pt(MENU_W_PT), my + mh + ui.pt(SPACE_XS));
    }

    fn open_editor_menu(&mut self, ui: &Ui) {
        let Screen::Editor(i) = self.screen else { return };
        let id = self.notes[i].id.clone();
        let here = self.loc_of(&id).to_string();
        let pin = if self.is_pinned(&id) { "Unpin Note" } else { "Pin Note" };
        let mut items = vec![
            (pin.to_string(), Act::TogglePin, false),
            ("Share as Markdown (.md)".to_string(), Act::ShareMd, false),
            ("Share as Text (.txt)".to_string(), Act::ShareText, false),
            ("Share as PDF (.pdf)".to_string(), Act::SharePdf, false),
            ("Share as HTML (.html)".to_string(), Act::ShareHtml, false),
            ("Format...".to_string(), Act::OpenFormat, false),
            ("Title".to_string(), Act::Style(md::Block::Title), false),
            ("Heading".to_string(), Act::Style(md::Block::Heading), false),
            ("Subheading".to_string(), Act::Style(md::Block::Subheading), false),
            ("Body".to_string(), Act::Style(md::Block::Body), false),
            ("Move Checked to Bottom".to_string(), Act::SortCheckedNow, false),
            ("Photo from Library".to_string(), Act::ChoosePhoto, false),
            ("Attach Latest Photo".to_string(), Act::AttachPhoto, false),
            ("Insert Sketch".to_string(), Act::InsertSketch, false),
            ("Insert Table".to_string(), Act::InsertTable, false),
            ("Check Spelling and Links".to_string(), Act::CheckSpelling, false),
            ("Proofread".to_string(), Act::Proofread, false),
            ("Match Whole Word in Find".to_string(), Act::ToggleWholeWord, self.whole_word),
            ("Replace in Note".to_string(), Act::Replace, false),
        ];
        if here != folders::TRASH {
            if !here.is_empty() {
                items.push(("Move to All Notes".to_string(), Act::MoveTo(String::new()), false));
            }
            for f in self.folders.iter().filter(|f| **f != here) {
                items.push((format!("Move to {f}"), Act::MoveTo(f.clone()), false));
            }
            items.push(("Duplicate Note".to_string(), Act::Duplicate, false));
            if self.notes[i].checklist().0 > 0 {
                items.push(("Clear Completed".to_string(), Act::ClearChecked, false));
            }
            items.push(("Delete Note".to_string(), Act::DeleteNote, false));
        } else {
            items.push(("Recover Note".to_string(), Act::Recover, false));
            items.push(("Delete Permanently".to_string(), Act::EraseNote, false));
        }
        let (mx, my, _, mh) = self.editor_more_rect(ui);
        self.show_menu(ui, items, mx, my + mh + ui.pt(SPACE_XS));
    }

    /// The Format menu of the open note: paragraph and list styles (a check marks the caret's), character styles,
    /// indentation, the checklist commands, the table commands when the caret is in a table, and Math Results.
    fn open_format_menu(&mut self, ui: &Ui) {
        let Screen::Editor(i) = self.screen else { return };
        let text = self.editor.text().to_string();
        let caret = self.editor.caret();
        let para = self.editor.paragraph_of(caret);
        let line: String = text.chars().skip(para.start).take(para.len()).collect();
        let current = md::classify(&line).block;
        let same = |b: md::Block| std::mem::discriminant(&b) == std::mem::discriminant(&current);
        let mut items: Vec<(String, Act, bool)> = vec![];
        for (label, block) in PARAGRAPH_STYLES {
            items.push((label.to_string(), Act::Style(block), same(block)));
        }
        for (label, f) in CHARACTER_STYLES {
            items.push((label.to_string(), Act::Do(Format::Char(f)), false));
        }
        items.push(("Increase Indent".to_string(), Act::Do(Format::Indent), false));
        items.push(("Decrease Indent".to_string(), Act::Do(Format::Outdent), false));
        items.push(("Move Item Up".to_string(), Act::Do(Format::MoveUp), false));
        items.push(("Move Item Down".to_string(), Act::Do(Format::MoveDown), false));
        if self.notes[i].checklist().1 > 0 {
            items.push(("Mark All".to_string(), Act::Do(Format::CheckAll), false));
            items.push(("Unmark All".to_string(), Act::Do(Format::UncheckAll), false));
        }
        for op in table::available(&text, caret) {
            items.push((op.label().to_string(), Act::Do(Format::Table(op)), false));
        }
        items.push(("Add Link...".to_string(), Act::Do(Format::AddLink), false));
        items.push(("Calculate Math Results".to_string(), Act::Do(Format::CalculateMath), false));
        items.push(("Math Results".to_string(), Act::ToggleMath, self.prefs.math_results));
        let (mx, my, _, mh) = self.editor_more_rect(ui);
        self.show_menu(ui, items, mx, my + mh + ui.pt(SPACE_XS));
    }

    fn menu_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Act> {
        let menu = self.menu.as_ref()?;
        let (mx, my, mw, mh) = self.menu_rect(ui, menu);
        if !within(x, y, (mx, my, mw, mh)) {
            return None;
        }
        let row = ((y - my - ui.pt(MENU_PAD_PT)).max(0.0) / ui.pt(MENU_ROW_PT)) as usize;
        menu.items.get(row).map(|(_, act, _)| act.clone())
    }

    fn start_naming(&mut self, ui: &Ui, old: Option<String>, t: f64) {
        let mut field = TextField::new(ui, "Folder Name", MAC_BODY_PT, false, 40);
        field.set_text(ui, &old.clone().unwrap_or_else(|| folders::unique_name(&self.folders, "New Folder")));
        field.focused = true;
        self.naming = Some(Naming { old, field, link: None });
        self.searching = false;
        self.search.focused = false;
        self.keyboard.set_shown(!self.hw_kb, t);
    }

    /// Add Link (Cmd+K): asks for a web address and wraps the selection (or the caret) as `[text](address)`.
    fn start_link(&mut self, ui: &Ui, t: f64) {
        if !self.is_editing() {
            return;
        }
        let caret = self.editor.caret();
        let sel = self.editor.selection().unwrap_or(caret..caret);
        let mut field = TextField::new(ui, "Web Address", MAC_BODY_PT, false, 400);
        field.set_text(ui, "https://");
        field.focused = true;
        self.naming = Some(Naming { old: None, field, link: Some(sel) });
        self.searching = false;
        self.search.focused = false;
        self.keyboard.set_shown(!self.hw_kb, t);
    }

    /// The address was entered: the link replaces the range, or the note says the address is empty.
    fn finish_link(&mut self, ui: &Ui, sel: Range<usize>, url: &str, t: f64) {
        if !self.is_editing() {
            return;
        }
        let text = self.editor.text().to_string();
        match md::link_markdown(&text, sel, url) {
            Some(link) => {
                self.notice = None;
                self.editor.replace_range(ui, link.range, &link.with, t, false);
                self.editor.select(link.select);
                self.changed(ui, t);
            }
            None => self.notice = Some("Enter a web address for the link".to_string()),
        }
    }

    fn cancel_naming(&mut self, t: f64) {
        self.naming = None;
        self.keyboard.set_shown(false, t);
        self.dirty = true;
    }

    fn commit_name(&mut self, ui: &Ui, t: f64) {
        let Some(naming) = self.naming.take() else { return };
        self.keyboard.set_shown(false, t);
        let name = naming.field.text().to_string();
        if let Some(range) = naming.link.clone() {
            self.finish_link(ui, range, &name, t);
            return;
        }
        let result = match &naming.old {
            None => folders::create(&self.dir, &name),
            Some(old) => folders::rename(&self.dir, old, &name),
        };
        match result {
            Ok(name) => {
                self.notice = None;
                if naming.old.as_ref().is_none_or(|o| self.view == View::Folder(o.clone())) {
                    self.view = View::Folder(name);
                }
                self.reload(ui, t);
            }
            Err(error) => self.notice = Some(format!("Unable to save folder: {error}")),
        }
    }

    // --- find and replace ---

    /// Find > Replace: opens the replace card over the open note. The find text is the search field's text as the
    /// note last found it, so without find text there is nothing to replace and the note says so.
    fn start_replacing(&mut self, ui: &Ui, t: f64) {
        if !self.is_editing() || self.hl_query.is_empty() {
            self.notice = Some("Search for text first, then replace it".to_string());
            self.dirty = true;
            return;
        }
        let mut field = TextField::new(ui, "Replacement", MAC_BODY_PT, false, 100);
        field.focused = true;
        self.replacing = Some(Replacing { field });
        self.searching = false;
        self.search.focused = false;
        self.keyboard.set_shown(!self.hw_kb, t);
        self.dirty = true;
    }

    fn cancel_replacing(&mut self, t: f64) {
        self.replacing = None;
        self.keyboard.set_shown(false, t);
        self.dirty = true;
    }

    /// The replacement text the card holds.
    fn replacement(&self) -> String {
        self.replacing.as_ref().map_or_else(String::new, |r| r.field.text().to_string())
    }

    /// Replace (the card's left button): the selected match becomes the replacement, and the find bar moves on to
    /// the next match.
    fn replace_one(&mut self, ui: &Ui, t: f64) {
        if self.hl_query.is_empty() {
            return;
        }
        let Some(r) = self.hl.get(self.hl_cur).cloned() else { return };
        let with = self.replacement();
        self.editor.replace_range(ui, r.clone(), &with, t, false);
        self.editor.set_caret(r.start + with.chars().count());
        self.changed(ui, t);
        self.scroll_to_match(ui);
        self.dirty = true;
    }

    /// Replace All (the card's right button, or Enter): every match of the find text in the open note becomes the
    /// replacement, as one undo step. The card closes.
    fn replace_all(&mut self, ui: &Ui, t: f64) {
        if !self.is_editing() || self.hl_query.is_empty() {
            self.cancel_replacing(t);
            return;
        }
        let text = self.editor.text().to_string();
        let ranges = parity::find_ranges(&text, &self.hl_query, self.whole_word);
        if !ranges.is_empty() {
            let caret = self.editor.caret();
            let new = parity::replace_ranges(&text, &ranges, &self.replacement());
            let end = self.editor.len();
            let n = new.chars().count();
            self.editor.replace_range(ui, 0..end, &new, t, false);
            self.editor.set_caret(caret.min(n));
        }
        self.cancel_replacing(t);
        self.changed(ui, t);
    }

    /// The replace card's rectangle (px): over the editor column, below the toolbar band.
    fn replacing_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (ex, ew) = self.ed_px(ui);
        let pad = ui.pt(SPACE_S);
        (ex + pad, ui.pt(nav_bottom_pt()) + pad, ew - 2.0 * pad, ui.pt(REPLACE_CARD_PT))
    }

    /// The card's buttons (px): Replace, then Replace All at the right edge of the label row.
    fn replace_buttons(&self, ui: &Ui) -> ((f32, f32, f32, f32), (f32, f32, f32, f32)) {
        let (x, y, w, _) = self.replacing_rect(ui);
        let (one_w, all_w, h) = (ui.pt(REPLACE_ONE_PT), ui.pt(REPLACE_ALL_PT), ui.pt(REPLACE_BAR_PT));
        let right = x + w - ui.pt(SPACE_S);
        ((right - all_w - one_w, y, one_w, h), (right - all_w, y, all_w, h))
    }

    fn draw_replacing(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(text) = self.replacing.as_ref().map(|r| r.field.text().to_string()) else { return };
        let (x, y, w, h) = self.replacing_rect(ui);
        let (one, all) = self.replace_buttons(ui);
        s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        s.stroke_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), 1.0, SEPARATOR_C.now());
        let label = self.labels.get(ui, "Replace With", MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now());
        s.blit(label, (x + ui.pt(SPACE_S)) as i32, (y + ui.pt(REPLACE_BAR_PT) / 2.0 - label.h as f32 / 2.0) as i32, 255, 255);
        let one_label = self.labels.get(ui, "Replace", MAC_FOOTNOTE_PT, Weight::Semibold, LINK);
        s.blit(one_label, (one.0 + one.2 / 2.0 - one_label.w as f32 / 2.0) as i32, (one.1 + one.3 / 2.0 - one_label.h as f32 / 2.0) as i32, 255, 255);
        let all_label = self.labels.get(ui, "Replace All", MAC_FOOTNOTE_PT, Weight::Semibold, LINK);
        s.blit(all_label, (all.0 + all.2 / 2.0 - all_label.w as f32 / 2.0) as i32, (all.1 + all.3 / 2.0 - all_label.h as f32 / 2.0) as i32, 255, 255);
        let mut field = bifrost_app::controls::TextField::new();
        field.push_str(&text);
        let r = chrome::pt_rect(ui, (x + ui.pt(SPACE_S), y + ui.pt(REPLACE_BAR_PT), w - 2.0 * ui.pt(SPACE_S), ui.pt(REPLACE_FIELD_PT)));
        bifrost_app::widgets::text_field(s, ui, &field, r, "Replacement", true);
    }

    /// Runs a menu action.
    fn run(&mut self, ui: &Ui, act: Act, t: f64) {
        self.dirty = true;
        match act {
            Act::View(v) => {
                self.view = v;
                self.list_scroll.jump_top();
                self.reload(ui, t);
            }
            Act::NewFolder => self.start_naming(ui, None, t),
            Act::RenameFolder => {
                if let View::Folder(f) = self.view.clone() {
                    self.start_naming(ui, Some(f), t);
                }
            }
            Act::DeleteFolder => {
                if let View::Folder(f) = self.view.clone() {
                    match folders::delete_folder(&self.dir, &f, self.wall.now(t)) {
                        Ok(()) => {
                            self.view = View::All;
                            self.reload(ui, t);
                        }
                        Err(error) => self.notice = Some(format!("Unable to delete folder: {error}")),
                    }
                }
            }
            Act::Sort(k) => {
                self.prefs.sort = k;
                self.save_prefs();
                self.rebuild(ui, t);
            }
            Act::ToggleCheckedBottom => {
                self.prefs.checked_to_bottom = !self.prefs.checked_to_bottom;
                self.save_prefs();
            }
            Act::SortCheckedNow => {
                let new = md::checked_to_bottom(self.editor.text());
                let caret = self.editor.caret();
                self.rewrite(ui, &new, caret, t);
            }
            Act::Style(block) => {
                let sel = self.editor.selection().unwrap_or(self.editor.caret()..self.editor.caret());
                self.restyle(ui, sel, block, t);
            }
            Act::ShareMd => {
                if let Screen::Editor(i) = self.screen {
                    self.save(t, true);
                    match store::export_md(&self.dir, &self.notes[i]) {
                        Ok(path) => self.share_req = Some(springboard::shell::share::Item::File(path)),
                        Err(error) => self.notice = Some(format!("Unable to export note: {error}")),
                    }
                }
            }
            Act::ShareText => {
                if let Screen::Editor(i) = self.screen {
                    let note = &self.notes[i];
                    self.share_req = Some(springboard::shell::share::Item::Text { title: note.title().to_string(), body: md::to_plain(&note.text) });
                }
            }
            Act::SharePdf => {
                if let Screen::Editor(i) = self.screen {
                    self.save(t, true);
                    let note = &self.notes[i];
                    let bytes = pdf::render(note.title(), &md::to_plain(&note.text));
                    match store::export_file(&self.dir, note, "pdf", &bytes) {
                        Ok(path) => self.share_req = Some(springboard::shell::share::Item::File(path)),
                        Err(error) => self.notice = Some(format!("Unable to export note: {error}")),
                    }
                }
            }
            Act::ShareHtml => {
                if let Screen::Editor(i) = self.screen {
                    self.save(t, true);
                    let note = &self.notes[i];
                    let page = html::render(note.title(), &note.text);
                    match store::export_file(&self.dir, note, "html", page.as_bytes()) {
                        Ok(path) => self.share_req = Some(springboard::shell::share::Item::File(path)),
                        Err(error) => self.notice = Some(format!("Unable to export note: {error}")),
                    }
                }
            }
            Act::MoveTo(dest) => self.move_open_note(&dest, t),
            Act::Duplicate => self.duplicate_open_note(ui, t),
            Act::ClearChecked => self.clear_checked(ui, t),
            Act::DeleteNote => {
                if let Screen::Editor(i) = self.screen {
                    self.save(t, true);
                    if self.trash_note(i, t) {
                        self.close_note(ui, t);
                    }
                }
            }
            Act::Recover => {
                if let Screen::Editor(i) = self.screen {
                    self.save(t, true);
                    let id = self.notes[i].id.clone();
                    match folders::restore(&self.dir, &id) {
                        Ok(()) => {
                            self.loc.remove(&id);
                            self.close_note(ui, t);
                        }
                        Err(error) => self.notice = Some(format!("Unable to recover note: {error}")),
                    }
                }
            }
            Act::EraseNote => {
                if let Screen::Editor(i) = self.screen {
                    self.autosave.fire(t, true);
                    self.unsaved = None;
                    self.screen = Screen::List;
                    self.keyboard.set_shown(false, t);
                    self.editor.focused = false;
                    self.erase(i);
                    self.hl.clear();
                    self.rebuild(ui, t);
                }
            }
            Act::AttachPhoto => self.attach_latest_photo(ui, t),
            Act::InsertSketch => self.open_sketch(),
            Act::InsertTable => self.insert_table(ui, t),
            Act::TogglePin => self.activate(ui, Target::EditorPin, 0.0, 0.0, t),
            Act::ToggleWholeWord => {
                self.whole_word = !self.whole_word;
                self.refresh_highlights(ui, true);
                self.dirty = true;
            }
            Act::CheckSpelling => self.check_note(ui),
            Act::Proofread => self.proofread_note(ui),
            Act::ProofreadAccept(i) => self.proofread_accept(ui, Some(i), t),
            Act::ProofreadAll => self.proofread_accept(ui, None, t),
            Act::ProofreadDone => self.proofread = None,
            Act::Replace => self.start_replacing(ui, t),
            Act::SortOrder(oldest) => {
                self.prefs.oldest_first = oldest;
                self.save_prefs();
                self.rebuild(ui, t);
            }
            Act::OpenFormat => self.open_format_menu(ui),
            Act::Do(f) => self.do_format(ui, f, t),
            Act::ToggleMath => {
                self.prefs.math_results = !self.prefs.math_results;
                self.save_prefs();
            }
            Act::ToggleGroupDate => {
                self.prefs.group_by_date = !self.prefs.group_by_date;
                self.save_prefs();
                self.rebuild(ui, t);
            }
            Act::ToggleResumeQuick => {
                self.prefs.resume_quick = !self.prefs.resume_quick;
                self.save_prefs();
            }
            Act::ChoosePhoto => self.picker = Some(picker::Picker::open(ui.pt(PICK_THUMB_PT) as usize)),
        }
    }

    /// Folds (`collapse`) or unfolds the heading the caret is under. Only the sidecar of folds changes, never the text.
    fn fold_section(&mut self, ui: &Ui, collapse: bool) {
        let Some(id) = self.editing().map(|note| note.id.clone()) else { return };
        let text = self.editor.text().to_string();
        let line = self.caret_para();
        let keys = self.folds.entry(id).or_default();
        let changed = if collapse { fold::collapse(keys, &text, line) } else { fold::expand(keys, &text, line) };
        if changed {
            self.save_folds();
            self.apply_folds(ui);
        }
    }

    /// Folds every heading of the open note, or unfolds them all.
    fn fold_all(&mut self, ui: &Ui, collapse: bool) {
        let Some(id) = self.editing().map(|note| note.id.clone()) else { return };
        let text = self.editor.text().to_string();
        let keys = self.folds.entry(id).or_default();
        let changed = if collapse { fold::collapse_all(keys, &text) } else { fold::expand_all(keys) };
        if changed {
            self.save_folds();
            self.apply_folds(ui);
        }
    }

    /// The logical (paragraph) line the caret is on; the editor's own line numbers count wrapped lines.
    fn caret_para(&self) -> usize {
        self.editor.text().chars().take(self.editor.caret()).filter(|&c| c == '\n').count()
    }

    /// Hands the open note's folded sections to the editor, which hides their paragraphs.
    fn apply_folds(&mut self, ui: &Ui) {
        let ranges = self.editing().and_then(|note| self.folds.get(&note.id)).map_or_else(Vec::new, |keys| fold::hidden(self.editor.text(), keys));
        self.editor.set_hidden(ui, ranges);
    }

    /// The disclosure chevron of each heading in the open note: its paragraph, its hit box on screen and whether its
    /// section is folded. A hidden heading has no height and gets no chevron.
    fn fold_chevrons(&self, ui: &Ui) -> Vec<(usize, (f32, f32, f32, f32), bool)> {
        let (ox, oy) = self.text_origin(ui);
        let size = ui.pt(CHEVRON_PT);
        let gap = ui.pt(CHEVRON_GAP_PT);
        let keys = self.editing().and_then(|note| self.folds.get(&note.id));
        let mut out = vec![];
        let mut start = 0;
        for (line, para) in self.editor.text().split('\n').enumerate() {
            if fold::level_of(md::classify(para).block).is_some() {
                let (ly, lh) = self.editor.line_span(start);
                if lh > 0.0 {
                    let x = ox - size - gap;
                    let folded = keys.is_some_and(|k| k.contains(para.trim_end()));
                    out.push((line, (x - gap, oy + ly, size + 2.0 * gap, lh), folded));
                }
            }
            start += para.chars().count() + 1;
        }
        out
    }

    /// Folds or unfolds the section of the heading on paragraph `line` (a tap on its chevron). A caret inside a section
    /// that folds moves up to its heading, so it is never left on a hidden line.
    fn toggle_fold(&mut self, ui: &Ui, line: usize) {
        let Some(id) = self.editing().map(|note| note.id.clone()) else { return };
        let text = self.editor.text().to_string();
        let Some(head) = fold::heading_above(&text, line) else { return };
        let caret = self.caret_para();
        let keys = self.folds.entry(id).or_default();
        let collapsed = if keys.remove(&head.key) {
            false
        } else {
            keys.insert(head.key.clone());
            true
        };
        self.save_folds();
        if collapsed {
            let heads = fold::headings(&text);
            if let Some(i) = heads.iter().position(|h| h.line == head.line) {
                if fold::section(&heads, i, text.lines().count()).contains(&caret) {
                    let start: usize = text.split('\n').take(head.line).map(|p| p.chars().count() + 1).sum();
                    self.editor.set_caret(start);
                }
            }
        }
        self.apply_folds(ui);
    }


    fn save_folds(&mut self) {
        // only notes still on disk keep their folds (a deleted note's entry is dropped when the sidecar is saved)
        let live: HashSet<&str> = self.notes.iter().map(|n| n.id.as_str()).collect();
        fold::prune(&mut self.folds, &live);
        if let Err(error) = fold::save(&self.dir, &self.folds) {
            self.notice = Some(format!("Unable to save sections: {error}"));
        }
    }

    /// View > as List / as Gallery: the choice is saved in Prefs and the list starts again from its top.
    pub fn set_layout(&mut self, ui: &Ui, layout: gallery::Layout) {
        if self.prefs.layout != layout {
            self.prefs.layout = layout;
            self.save_prefs();
            self.list_scroll.jump_top();
            self.fit_scroll(ui);
            self.dirty = true;
        }
    }

    /// The list toolbar's toggle: the same choice as View > as List / as Gallery.
    fn toggle_layout(&mut self, ui: &Ui, _t: f64) {
        let next = if self.prefs.layout == gallery::Layout::Gallery { gallery::Layout::List } else { gallery::Layout::Gallery };
        self.set_layout(ui, next);
    }

    /// The gallery's columns as `(count, card width, gap)` in px, from the width of the list column.
    fn gallery_cols(&self, ui: &Ui) -> (usize, f32, f32) {
        let (_, lw) = self.list_px(ui);
        let width = lw - 2.0 * ui.pt(SPACE_S);
        let gap = ui.pt(gallery::CARD_GAP_PT);
        let cols = gallery::columns(width, ui.pt(gallery::CARD_MIN_PT), gap);
        (cols, gallery::card_width(width, cols, gap), gap)
    }

    /// The gallery's cards as `(x, y, w, h)` in list space (scroll not applied), the section headers as
    /// `(index into sections, y)`, and the height of the whole grid. A section starts on a new row.
    fn gallery_grid(&self, ui: &Ui) -> (Vec<(f32, f32, f32, f32)>, Vec<(usize, f32)>, f32) {
        let (cols, cw, gap) = self.gallery_cols(ui);
        let (lx, _) = self.list_px(ui);
        let m = ui.pt(SPACE_S);
        let pitch = ui.pt(GALLERY_CARD_PT) + gap;
        let ch = ui.pt(GALLERY_CARD_PT);
        let mut y = ui.pt(list_top_pt());
        let (mut col, mut cells, mut heads) = (0usize, Vec::with_capacity(self.shown.len()), vec![]);
        for i in 0..self.shown.len() {
            if let Some(k) = self.sections.iter().position(|(_, start)| *start == i) {
                if col != 0 {
                    y += pitch;
                    col = 0;
                }
                heads.push((k, y));
                y += ui.pt(Self::header_pt(k));
            }
            cells.push((lx + m + col as f32 * (cw + gap), y, cw, ch));
            col += 1;
            if col == cols {
                col = 0;
                y += pitch;
            }
        }
        if col != 0 {
            y += pitch;
        }
        (cells, heads, y)
    }

    /// The first picture a note shows, by file name.
    fn first_picture(text: &str) -> Option<&str> {
        text.lines().find_map(md::image_ref)
    }

    /// Asks for the first picture of each shown note that the gallery does not have yet; the decode runs off the UI
    /// thread and `poll_thumbs` takes the result.
    fn request_thumbs(&mut self, ui: &Ui) {
        let (_, cw, _) = self.gallery_cols(ui);
        let max_w = cw.max(1.0) as usize;
        let mut jobs = vec![];
        for &i in &self.shown {
            let note = &self.notes[i];
            let Some(name) = Self::first_picture(&note.text) else { continue };
            let have = self.thumbs.get(&note.id).is_some_and(|(kept, _)| kept == name);
            if have || self.thumb_pending.contains(&note.id) {
                continue;
            }
            jobs.push((note.id.clone(), name.to_string(), self.note_dir(&note.id)));
        }
        if jobs.is_empty() {
            return;
        }
        for (id, _, _) in &jobs {
            self.thumb_pending.insert(id.clone());
        }
        let sink = self.thumb_slot.clone();
        DispatchQueue::global(Qos::Utility).async_(move || {
            let done: Vec<(String, String, Option<Image>)> = jobs
                .into_iter()
                .map(|(id, name, dir)| {
                    let img = std::fs::read(dir.join(&name)).ok().and_then(|bytes| attach::decode_fitted(&bytes, max_w));
                    (id, name, img)
                })
                .collect();
            DispatchQueue::main().async_(move || sink.lock().unwrap_or_else(|e| e.into_inner()).extend(done));
        });
    }

    /// Takes finished thumbnail decodes; true when there were any.
    fn poll_thumbs(&mut self) -> bool {
        let done: Vec<(String, String, Option<Image>)> = std::mem::take(&mut *self.thumb_slot.lock().unwrap_or_else(|e| e.into_inner()));
        let any = !done.is_empty();
        for (id, name, img) in done {
            self.thumb_pending.remove(&id);
            self.thumbs.insert(id, (name, img));
        }
        any
    }


    fn save_prefs(&mut self) {
        if let Err(error) = store::save_prefs(&self.dir, self.prefs) {
            self.notice = Some(format!("Unable to save settings: {error}"));
        }
    }

    /// Moves the open note into `dest` ("" is no folder), saving it first.
    fn move_open_note(&mut self, dest: &str, t: f64) {
        let Screen::Editor(i) = self.screen else { return };
        self.save(t, true);
        let id = self.notes[i].id.clone();
        let to = Some(dest.to_string()).filter(|d| !d.is_empty());
        match folders::move_note(&self.dir, &id, self.folder_arg(&id).as_deref(), to.as_deref()) {
            Ok(()) => {
                if dest.is_empty() {
                    self.loc.remove(&id);
                } else {
                    self.loc.insert(id, dest.to_string());
                }
                self.notice = None;
            }
            Err(error) => self.notice = Some(format!("Unable to move note: {error}")),
        }
        self.dirty = true;
    }

    /// Stores the newest photo beside the open note and inserts it as an
    /// image paragraph at the caret (one undo step).
    fn attach_latest_photo(&mut self, ui: &Ui, t: f64) {
        let Some(path) = attach::latest_image(&springboard::shell::photos::library::dirs()) else {
            self.notice = Some("No photos to attach".to_string());
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                if let Err(error) = self.attach_image(ui, &bytes, t) {
                    self.notice = Some(format!("Unable to attach image: {error}"));
                }
            }
            Err(error) => self.notice = Some(format!("Unable to read photo: {error}")),
        }
    }

    /// Attaches PNG or JPEG `bytes` to the open note (bounded size) and
    /// inserts the image paragraph at the caret.
    pub fn attach_image(&mut self, ui: &Ui, bytes: &[u8], t: f64) -> std::io::Result<()> {
        let Screen::Editor(i) = self.screen else { return Err(std::io::Error::other("No open note")) };
        let id = self.notes[i].id.clone();
        let name = attach::import(&self.note_dir(&id), &id, bytes)?;
        self.insert_image(ui, &name, t);
        Ok(())
    }

    /// Inserts the image paragraph for file `name` at the caret (the note is then saved as changed).
    fn insert_image(&mut self, ui: &Ui, name: &str, t: f64) {
        let para = self.editor.paragraph_of(self.editor.caret());
        let at_start = self.editor.caret() == para.start;
        let line = format!("{}{}\n", if at_start { "" } else { "\n" }, md::image_line(name));
        let caret = self.editor.caret();
        self.editor.replace_range(ui, caret..caret, &line, t, false);
        self.load_images(ui);
        self.changed(ui, t);
    }

    /// Insert Table (editor menu): a header row, its rule and one empty row, as a Markdown pipe table, on
    /// their own paragraph at the caret. Cells are typed in place; the table is saved as plain text.
    fn insert_table(&mut self, ui: &Ui, t: f64) {
        let para = self.editor.paragraph_of(self.editor.caret());
        let at_start = self.editor.caret() == para.start;
        let table = "| Column | Column |\n| --- | --- |\n|  |  |\n";
        let line = format!("{}{}\n", if at_start { "" } else { "\n" }, table);
        let caret = self.editor.caret();
        self.editor.replace_range(ui, caret..caret, &line, t, false);
        self.changed(ui, t);
    }

    /// Insert Sketch (editor menu): opens an empty sketch pad over the window.
    fn open_sketch(&mut self) {
        if matches!(self.screen, Screen::Editor(_)) {
            self.sketch = Some(sketch::Pad::new());
        }
    }

    /// Done on the sketch pad: saves the drawing beside the note and inserts its picture at the caret. An empty pad
    /// just closes.
    fn finish_sketch(&mut self, ui: &Ui, t: f64) {
        let Some(pad) = self.sketch.take() else { return };
        let drawing = pad.into_drawing();
        if drawing.is_empty() {
            return;
        }
        let Screen::Editor(i) = self.screen else { return };
        let id = self.notes[i].id.clone();
        match sketch::save(&self.note_dir(&id), &id, &drawing) {
            Ok(name) => self.insert_image(ui, &name, t),
            Err(error) => self.notice = Some(format!("Unable to save sketch: {error}")),
        }
    }

    /// Touches on the sketch pad. Below the header a stroke follows the finger. In the header, Cancel closes the pad
    /// (left zone), Undo removes the newest stroke (middle zone) and Done saves it (right zone).
    fn sketch_touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (head, zone, width, w) = (ui.pt(SKETCH_HEAD_PT), ui.pt(SKETCH_ZONE_PT), ui.pt(sketch::LINE_PT), self.w as f32);
        let Some(pad) = self.sketch.as_mut() else { return };
        let x = match ev {
            Event::Down(x, y) => {
                if y >= head {
                    pad.begin(x, y, width);
                }
                return;
            }
            Event::Move(x, y) => {
                pad.extend(x, y);
                return;
            }
            Event::Up(x, y) => {
                if pad.is_drawing() {
                    pad.end();
                    return;
                }
                if y >= head {
                    return;
                }
                x
            }
            _ => return,
        };
        if x < zone {
            self.sketch = None;
        } else if x >= w - zone {
            self.finish_sketch(ui, t);
        } else if x >= w - 2.0 * zone {
            if let Some(pad) = self.sketch.as_mut() {
                pad.undo();
            }
        }
    }

    /// The sketch pad: the whole window in the content colour, a rule under the header, the strokes and the header
    /// controls (Cancel, Sketch, Undo, Done).
    fn draw_sketch(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(pad) = &self.sketch else { return };
        let (head, zone) = (ui.pt(SKETCH_HEAD_PT), ui.pt(SKETCH_ZONE_PT));
        let w = self.w as f32;
        s.fill_rect(0, 0, self.w as usize, self.h as usize, CONTENT_BG.now());
        s.fill_rect(0, head as usize, self.w as usize, 1, SEPARATOR_C.now());
        for stroke in pad.all() {
            for (cx, cy, r, color) in sketch::stroke_dabs(stroke, &|x: f32, y: f32| (x, y), 1.0) {
                s.fill_rect((cx - r).max(0) as usize, (cy - r).max(0) as usize, (2 * r + 1) as usize, (2 * r + 1) as usize, color);
            }
        }
        let title = self.labels.get(ui, "Sketch", MAC_BODY_PT, Weight::Semibold, LABEL.now());
        centred(s, title, self.w / 2, (head / 2.0 - title.h as f32 / 2.0) as i32);
        let cancel = self.labels.get(ui, "Cancel", MAC_BODY_PT, Weight::Regular, LINK);
        centred(s, cancel, (zone / 2.0) as i32, (head / 2.0) as i32);
        let undo = self.labels.get(ui, "Undo", MAC_BODY_PT, Weight::Regular, LINK);
        centred(s, undo, (w - 1.5 * zone) as i32, (head / 2.0) as i32);
        let done = self.labels.get(ui, "Done", MAC_BODY_PT, Weight::Semibold, LINK);
        centred(s, done, (w - zone / 2.0) as i32, (head / 2.0) as i32);
    }

    /// Sheet rect: the bottom 62% of the screen.
    fn picker_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let h = self.h as f32 * 0.62;
        let _ = ui;
        (0.0, self.h as f32 - h, self.w as f32, h)
    }

    fn picker_touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        let (sx, sy, sw, sh) = self.picker_rect(ui);
        let head = ui.pt(PICK_HEAD_PT);
        let gap = ui.pt(picker::GAP_PT);
        let Some(p) = &mut self.picker else { return };
        match ev {
            Event::Move(_, y) => {
                if let Some(y0) = self.pick_drag.replace(y) {
                    let (_, total) = p.metrics(sw, gap);
                    p.scroll = (p.scroll + y0 - y).clamp(0.0, (total - (sh - head)).max(0.0));
                    if (y0 - y).abs() > 0.5 {
                        self.pick_moved = true;
                    }
                }
            }
            Event::Down(_, y) => {
                self.pick_drag = Some(y);
                self.pick_moved = false;
            }
            Event::Up(x, y) => {
                self.pick_drag = None;
                if std::mem::take(&mut self.pick_moved) {
                    return;
                }
                if y < sy || (y < sy + head && x < sx + ui.pt(90.0)) {
                    self.picker = None;
                } else if y >= sy + head {
                    if let Some(i) = p.hit(x - sx, y - sy - head, sw, gap) {
                        let path = p.photos[i].clone();
                        self.picker = None;
                        match std::fs::read(&path) {
                            Ok(bytes) => {
                                if let Err(error) = self.attach_image(ui, &bytes, t) {
                                    self.notice = Some(format!("Unable to attach image: {error}"));
                                }
                            }
                            Err(error) => self.notice = Some(format!("Unable to read photo: {error}")),
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn draw_picker(&mut self, s: &mut Surface, ui: &Ui) {
        let (sx, sy, sw, sh) = self.picker_rect(ui);
        let head = ui.pt(PICK_HEAD_PT);
        let gap = ui.pt(picker::GAP_PT);
        let Some(p) = &self.picker else { return };
        // compliance-allow: COL-01, COL-03 the picker's dimming scrim: ui.rs has no translucent-black token yet
        s.fill_rrect(0.0, 0.0, self.w as f32, sy, 0.0, pack(0, 0, 0, 120));
        s.fill_rrect(sx, sy, sw, sh + ui.pt(SPACE_XL), ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let title = self.labels.get(ui, "Photos", MAC_BODY_PT, Weight::Semibold, LABEL.now());
        centred(s, title, self.w / 2, (sy + head / 2.0 - title.h as f32 / 2.0) as i32);
        let cancel = self.labels.get(ui, "Cancel", MAC_BODY_PT, Weight::Regular, LINK);
        s.blit(cancel, (sx + ui.pt(SPACE_L)) as i32, (sy + head / 2.0 - cancel.h as f32 / 2.0) as i32, 255, 255);
        let (side, _) = p.metrics(sw, gap);
        let (top, bottom) = (sy + head, sy + sh);
        for i in 0..p.photos.len() {
            let (col, row) = (i % picker::COLS, i / picker::COLS);
            let y = top + row as f32 * (side + gap) - p.scroll;
            if y + side < top || y > bottom {
                continue;
            }
            let x = sx + col as f32 * (side + gap);
            s.fill_rect(x as usize, y.max(top) as usize, side as usize, ((y + side).min(bottom) - y.max(top)).max(0.0) as usize, FIELD_BG.now());
            if let Some(img) = p.thumbs.get(&i) {
                if y >= top && y + side <= bottom {
                    s.blit(img, (x + (side - img.w as f32) / 2.0) as i32, (y + (side - img.h as f32) / 2.0) as i32, 255, 255);
                }
            }
        }
        if p.photos.is_empty() {
            let none = self.labels.get(ui, "No Photos", MAC_BODY_PT, Weight::Regular, LABEL_2.now());
            centred(s, none, self.w / 2, (sy + sh / 2.0) as i32);
        }
    }

    /// Typed input for the search field or the editor.
    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        if self.sketch.is_some() {
            // the sketch pad takes no typing; Hide closes it
            if matches!(out, Out::Hide) {
                self.sketch = None;
            }
            return;
        }
        // the search field and the folder-name prompt take the keys over the note beside them
        let list_keys = self.screen == Screen::List || self.search.focused || self.naming.is_some() || self.replacing.is_some();
        match list_keys {
            true => {
                if self.replacing.is_some() {
                    match out {
                        Out::Enter => self.replace_all(ui, t),
                        Out::Hide => self.cancel_replacing(t),
                        _ => {
                            if let Some(r) = &mut self.replacing {
                                r.field.edit(ui, &out, t);
                            }
                        }
                    }
                    return;
                }
                if self.naming.is_some() {
                    match out {
                        Out::Enter => self.commit_name(ui, t),
                        Out::Hide => self.cancel_naming(t),
                        _ => {
                            if let Some(n) = &mut self.naming {
                                n.field.edit(ui, &out, t);
                            }
                        }
                    }
                    return;
                }
                if !self.search.focused || self.screen == Screen::List {
                    match out {
                        Out::Named("Down") => return self.move_selection(1),
                        Out::Named("Up") => return self.move_selection(-1),
                        Out::Enter if !self.search.focused => {
                            if let Some(i) = self.selected_index() {
                                self.open_note(ui, self.shown[i], false, t);
                                return;
                            }
                        }
                        _ => {}
                    }
                }
                if out == Out::Hide || out == Out::Enter {
                    self.keyboard.set_shown(false, t);
                    self.search.focused = false;
                    return;
                }
                self.search.edit(ui, &out, t);
                self.rebuild(ui, t);
            }
            false => {
                if out == Out::Hide {
                    self.keyboard.set_shown(false, t);
                    self.editor.focused = false;
                    self.save(t, true);
                    return;
                }
                if out == Out::Named("Tab") {
                    // Tab on a list line indents it; elsewhere it does nothing
                    self.shift_indent(ui, true, t);
                    return;
                }
                if self.editor.edit(ui, &out, t) {
                    if self.prefs.math_results && matches!(&out, Out::Text(s) if s == "=") {
                        self.answer_math(ui, t);
                    }
                    self.changed(ui, t);
                } else {
                    self.show_caret(ui);
                }
            }
        }
    }

    /// Desktop chords (Cmd or Ctrl; evdev keys): N new note and F search in the list, Z undo and
    /// Shift+Z redo in the editor. False when the chord has no meaning on this screen.
    pub fn chord(&mut self, ui: &Ui, key: u32, mods: u32, t: f64) -> bool {
        use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
        if key == KEY_TAB && mods == CHORD_SHIFT && matches!(self.screen, Screen::Editor(_)) {
            // Shift+Tab takes a level off the list lines
            self.shift_indent(ui, false, t);
            return true;
        }
        if mods & (CHORD_CTRL | CHORD_META) == 0 || mods & CHORD_ALT != 0 {
            return false;
        }
        let shift = mods & CHORD_SHIFT != 0;
        if matches!(self.screen, Screen::Editor(_)) && mods & CHORD_CTRL != 0 && mods & CHORD_META != 0 && (key == KEY_UP || key == KEY_DOWN) {
            // Ctrl+Cmd+Up and Down move the list item
            self.move_items(ui, key == KEY_UP, t);
            return true;
        }
        if key == KEY_K && !shift && matches!(self.screen, Screen::Editor(_)) {
            // Cmd+K asks for a web address
            self.start_link(ui, t);
            return true;
        }
        if key == 14 && !shift && self.screen == Screen::List {
            // Cmd+Backspace deletes the arrow-selected note
            let Some(i) = self.selected_index() else { return false };
            self.delete_row(ui, i, t);
            return true;
        }
        let target = match (self.screen, key, shift) {
            (_, 49, false) => Target::Compose,
            (_, 33, false) => Target::Search,
            (Screen::Editor(_), 44, false) => Target::Undo,
            (Screen::Editor(_), 44, true) => Target::Redo,
            (Screen::Editor(_), 48, false) => Target::Fmt(Fmt::Bold),
            (Screen::Editor(_), 23, false) => Target::Fmt(Fmt::Italic),
            (Screen::Editor(_), 22, false) => Target::Fmt(Fmt::Underline),
            (Screen::Editor(_), 45, true) => Target::Fmt(Fmt::Strike),
            (Screen::Editor(_), 35, true) => Target::Fmt(Fmt::Highlight),
            _ => return false,
        };
        self.activate(ui, target, 0.0, 0.0, t);
        true
    }

    /// Escape: cancels a name prompt, closes the open note, or leaves the search field.
    pub fn escape(&mut self, ui: &Ui, t: f64) -> bool {
        if self.replacing.is_some() {
            self.cancel_replacing(t);
            return true;
        }
        if self.naming.is_some() {
            self.cancel_naming(t);
            return true;
        }
        if self.search.focused {
            self.search.focused = false;
            self.searching = false;
            self.keyboard.set_shown(false, t);
            self.dirty = true;
            return true;
        }
        match self.screen {
            Screen::Editor(_) => {
                self.close_note(ui, t);
                true
            }
            Screen::List => false,
        }
    }

    fn activate(&mut self, ui: &Ui, target: Target, x: f32, y: f32, t: f64) {
        self.dirty = true;
        match target {
            Target::Search => {
                let (fx, fy, fw, fh) = self.search_rect(ui);
                let clear = bifrost_app::controls::search::hits_clear(chrome::pt_rect(ui, (fx, fy, fw, fh)), !self.search.is_empty(), x / ui.pt(1.0), y / ui.pt(1.0));
                if clear {
                    self.search.set_text(ui, "");
                    self.rebuild(ui, t);
                }
                self.searching = true;
                self.search.focused = true;
                self.editor.focused = false;
                self.keyboard.set_shown(!self.hw_kb, t);
                self.search.tap(ui, x - fx - ui.pt(36.0), t);
                self.reveal = None;
            }
            Target::Row(i) => {
                if self.reveal.take().is_none() {
                    self.search.focused = false;
                    let id = self.notes[self.shown[i]].id.clone();
                    // a second tap on the same row within DOUBLE_TAP_MS opens the note in a window of its own
                    let double = matches!(self.row_tap.take(), Some((ref prev, t0)) if *prev == id && t - t0 < DOUBLE_TAP_MS);
                    self.row_tap = (!double).then(|| (id.clone(), t));
                    self.open_note(ui, self.shown[i], false, t);
                    if double {
                        self.window_req = Some(id);
                    }
                }
            }
            Target::Delete(i) => self.delete_row(ui, i, t),
            Target::Pin(row) => {
                self.reveal = None;
                if let Some(&i) = self.shown.get(row) {
                    let id = self.notes[i].id.clone();
                    self.set_pinned(ui, &id, !self.is_pinned(&id), t);
                }
            }
            Target::EditorPin => {
                if let Screen::Editor(i) = self.screen {
                    let id = self.notes[i].id.clone();
                    self.set_pinned(ui, &id, !self.is_pinned(&id), t);
                }
            }
            Target::EditorShare => {
                if let Screen::Editor(i) = self.screen {
                    self.share_req = Some(springboard::shell::share::Item::Text { title: self.notes[i].title().to_string(), body: self.notes[i].text.clone() });
                }
            }
            Target::Title => self.open_list_menu(ui),
            Target::FolderNew => self.start_naming(ui, None, t),
            Target::SidebarToggle => self.toggle_sidebar(ui),
            Target::Sidebar(id) => {
                if let Some(view) = self.row_view(id) {
                    self.show_view(ui, view, t);
                }
            }
            Target::Bar(chrome::B_PHOTO) => self.picker = Some(picker::Picker::open(ui.pt(PICK_THUMB_PT) as usize)),
            Target::Bar(chrome::B_LAYOUT) => self.toggle_layout(ui, t),
            Target::Fold(line) => self.toggle_fold(ui, line),
            Target::Bar(_) => {}
            Target::EditorMore => self.open_editor_menu(ui),
            Target::FindPrev => self.step_match(ui, -1),
            Target::FindNext => self.step_match(ui, 1),
            Target::Fmt(f) => self.apply_fmt(ui, f, t),
            Target::Compose => self.new_note(ui, t),
            Target::List => self.reveal = None,
            Target::Back => self.close_note(ui, t),
            Target::Done => self.key(ui, Out::Hide, t),
            Target::UndoDelete => self.undo_delete(ui, t),
            Target::Undo => {
                if self.editor.undo(ui, t) {
                    self.changed(ui, t);
                }
            }
            Target::Redo => {
                if self.editor.redo(ui, t) {
                    self.changed(ui, t);
                }
            }
            Target::Format => self.cycle_format(ui, t),
            Target::Text => {
                let (ox, oy) = self.text_origin(ui);
                // a tap on a link opens it; the caret stays where it was
                if let Some(url) = self.editor.link_at(x - ox, y - oy) {
                    self.link_req = Some(url);
                    return;
                }
                if let Some(para) = self.editor.checkbox_at(x - ox, y - oy) {
                    self.toggle_checkbox(ui, para, t);
                    return;
                }
                self.editor.tap(x - ox, y - oy, t);
                self.editor.focused = true;
                self.search.focused = false;
                self.keyboard.set_shown(!self.hw_kb, t);
                self.show_caret(ui);
            }
            Target::Keyboard => {}
        }
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.dirty = true;
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => return,
        };
        self.sync_width(ui);
        let kb_top = self.kb_top(ui);
        let in_list = !matches!(self.screen, Screen::Editor(_)) || x < self.ed_px(ui).0;
        if self.sketch.is_some() {
            self.sketch_touch(ui, ev, t);
            return;
        }
        if self.picker.is_some() {
            self.picker_touch(ui, ev, t);
            return;
        }
        if self.settings_open {
            // the Settings pane takes the taps: a Done or an outside tap closes it, a row changes its value
            if let Event::Up(..) = ev {
                self.settings_tap(ui, x, y, t);
            }
            return;
        }
        if self.menu.is_some() {
            // a menu is modal: a tap picks a row, a tap elsewhere closes it
            if let Event::Up(..) = ev {
                let act = self.menu_hit(ui, x, y);
                self.menu = None;
                if let Some(act) = act {
                    self.run(ui, act, t);
                }
            }
            return;
        }
        if self.replacing.is_some() && !(self.keyboard.shown() && y >= kb_top) && !self.kb_touch {
            // the replace card takes its own taps: Replace, Replace All, or a tap outside it closes it
            if let Event::Up(..) = ev {
                let (one, all) = self.replace_buttons(ui);
                if within(x, y, all) {
                    self.replace_all(ui, t);
                } else if within(x, y, one) {
                    self.replace_one(ui, t);
                } else if !within(x, y, self.replacing_rect(ui)) {
                    self.cancel_replacing(t);
                }
            }
            return;
        }
        if self.naming.is_some() && !(self.keyboard.shown() && y >= kb_top) && !self.kb_touch {
            if let Event::Up(..) = ev {
                self.cancel_naming(t);
            }
            return;
        }
        match ev {
            Event::Down(..) => {
                self.fit_scroll(ui);
                self.pressed = self.hit(ui, x, y);
                match self.pressed {
                    Some(Target::Keyboard) => {
                        self.kb_touch = true;
                        self.keyboard.down(x, y - kb_top, t);
                    }
                    Some(Target::Row(i) | Target::Pin(i)) => {
                        let dx0 = self.reveal.filter(|&(r, _)| r == i).map_or(0.0, |(_, dx)| dx);
                        self.row_touch = Some(RowTouch { row: i, x0: x, y0: y, horizontal: None, dx0 });
                        self.list_scroll.down(y, t);
                    }
                    Some(Target::List) => {
                        self.list_scroll.down(y, t);
                        self.scrolling = true;
                    }
                    Some(Target::Text) => {
                        self.scroll.down(y, t);
                        self.scrolling = true;
                    }
                    _ => {}
                }
            }
            Event::Move(..) => {
                if self.kb_touch {
                    self.keyboard.moved(x, y - kb_top);
                } else if let Some(mut rt) = self.row_touch {
                    let (dx, dy) = (x - rt.x0, y - rt.y0);
                    if rt.horizontal.is_none() && dx.hypot(dy) > ui.pt(SLOP_PT) {
                        // cards have no swipe actions: a drag in the gallery scrolls
                        rt.horizontal = Some(dx.abs() > dy.abs() && self.prefs.layout != gallery::Layout::Gallery);
                        if rt.horizontal == Some(true) {
                            self.pressed = None;
                        } else {
                            self.scrolling = true;
                            self.pressed = None;
                        }
                    }
                    match rt.horizontal {
                        Some(true) => {
                            let (_, _, rw, _) = self.row_rect(ui, rt.row);
                            self.reveal = Some((rt.row, (rt.dx0 + dx).clamp(-rw, rw)));
                        }
                        Some(false) => self.list_scroll.moved(y, t),
                        None => {}
                    }
                    self.row_touch = Some(rt);
                } else if self.scrolling {
                    let s = if in_list { &mut self.list_scroll } else { &mut self.scroll };
                    s.moved(y, t);
                    if s.dragging() {
                        self.pressed = None;
                    }
                }
            }
            Event::Up(..) | Event::Power | Event::Wake | Event::VolumeUp | Event::VolumeDown | Event::Screenshot => {
                let pressed = self.pressed.take();
                if self.kb_touch {
                    self.kb_touch = false;
                    if let Some(out) = self.keyboard.up(x, y - kb_top, t) {
                        self.key(ui, out, t);
                    }
                    return;
                }
                if let Some(rt) = self.row_touch.take() {
                    match rt.horizontal {
                        Some(true) => {
                            let (_, _, rw, _) = self.row_rect(ui, rt.row);
                            let dx = self.reveal.map_or(0.0, |(_, dx)| dx);
                            if dx > 0.0 {
                                self.release_lead(ui, rt.row, dx, rw, t);
                            } else if -dx >= rw * FULL_SWIPE {
                                self.delete_row(ui, rt.row, t);
                            } else if -dx >= ui.pt(DELETE_PT) / 2.0 {
                                self.reveal = Some((rt.row, -ui.pt(DELETE_PT)));
                            } else {
                                self.reveal = None;
                            }
                            return;
                        }
                        Some(false) => {
                            self.scrolling = false;
                            self.list_scroll.up(y, t);
                            return;
                        }
                        None => {}
                    }
                }
                let tap = if std::mem::take(&mut self.scrolling) { if in_list { self.list_scroll.up(y, t) } else { self.scroll.up(y, t) } } else { true };
                if tap {
                    if let Some(p) = pressed.filter(|&p| Some(p) == self.hit(ui, x, y)) {
                        self.activate(ui, p, x, y, t);
                    }
                }
            }
        }
    }

    /// Held keys, scrolling, the keyboard slide and the autosave timer;
    /// true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        if self.undo_toast.as_ref().is_some_and(|u| t >= u.until) {
            self.undo_toast = None;
            self.dirty = true;
        }
        for out in self.keyboard.tick(t) {
            self.key(ui, out, t);
        }
        if self.autosave.pending() {
            self.save(t, false);
        }
        if self.poll_images(ui) {
            self.dirty = true;
        }
        if self.poll_thumbs() {
            self.dirty = true;
        }
        if let Some(p) = &mut self.picker {
            self.dirty |= p.poll() | p.loading();
        }
        self.fit_scroll(ui);
        let moving = self.list_scroll.tick(t) | self.scroll.tick(t);
        self.dirty |= moving || self.keyboard.animating();
        self.dirty
    }

    /// When the next frame is due on its own: caret blink, keyboard slide,
    /// scrolling or the autosave.
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due: Option<f64> = self.autosave.due();
        if self.keyboard.animating() || self.list_scroll.animating() || self.scroll.animating() || self.reveal.is_some() {
            due = Some(t);
        }
        if let Some(toast) = &self.undo_toast {
            due = Some(due.map_or(toast.until, |d| d.min(toast.until)));
        }
        let blink = match self.screen {
            _ if self.naming.is_some() => self.naming.as_ref().and_then(|n| n.field.next_blink(t)),
            _ if self.replacing.is_some() => self.replacing.as_ref().and_then(|r| r.field.next_blink(t)),
            _ if self.search.focused => self.search.next_blink(t),
            Screen::Editor(_) if self.keyboard.shown() => self.editor.next_blink(t),
            _ => None,
        };
        if let Some(b) = blink {
            due = Some(due.map_or(b, |d| d.min(b)));
        }
        due
    }

    // --- drawing ---

    /// After a Light/Dark switch: the cached rows and the editor's text colour are rendered again.
    fn follow_appearance(&mut self, ui: &Ui, t: f64) {
        if LABEL.now() == self.appearance || bifrost_app::kit::appearance::fading() {
            return;
        }
        self.remake_editor(ui, t);
    }

    /// The editor laid out again with the current text size and ink (Settings changes and Light/Dark switches). The
    /// text, caret, focus and highlights are kept.
    fn remake_editor(&mut self, ui: &Ui, t: f64) {
        let (text, caret, focused) = (self.editor.text().to_string(), self.editor.caret(), self.editor.focused);
        let mut e = new_editor(ui, self.editor_w.max(1.0), &self.note_settings);
        e.set_text(ui, &text);
        e.set_caret(caret);
        e.focused = focused;
        self.editor = e;
        self.editor.set_highlights(self.hl.clone(), (!self.hl.is_empty()).then_some(self.hl_cur));
        self.load_images(ui);
        self.refresh_date(ui);
        self.rebuild(ui, t);
    }

    fn draw_list(&mut self, s: &mut Surface, ui: &Ui) {
        if self.prefs.layout == gallery::Layout::Gallery {
            return self.draw_gallery(s, ui);
        }
        let (lx, lw) = self.list_px(ui);
        let bottom = self.content_bottom(ui);
        let rh = ui.pt(ROW_PT);
        let n = self.shown.len();
        let sections = self.sections.clone();
        let open = self.editing().map(|note| note.id.clone());
        let selected = self.selected_index();
        let rects: Vec<_> = (0..n).map(|i| self.row_rect(ui, i)).collect();
        let places: Vec<String> = (0..n).map(|i| self.loc_of(&self.notes[self.shown[i]].id).to_string()).collect();
        let pinned: Vec<bool> = (0..n).map(|i| self.is_pinned(&self.notes[self.shown[i]].id)).collect();
        let mut content = s.rows(ui.pt(nav_bottom_pt()) as usize, bottom as usize);
        for (label, row) in &sections {
            if *row < n {
                let (rx, ry, ..) = rects[*row];
                let img = self.labels.get(ui, label, MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now());
                content.blit(img, (rx + ui.pt(SPACE_S)) as i32, (ry - ui.pt(SPACE_XXL)) as i32, 255, 255);
            }
        }
        for i in 0..n {
            let (rx, ry, rw, _) = rects[i];
            if ry + rh < content.y0 as f32 || ry > content.h as f32 {
                continue;
            }
            let dx = self.reveal.filter(|&(r, _)| r == i).map_or(0.0, |(_, dx)| dx);
            let is_open = open.as_deref() == Some(self.notes[self.shown[i]].id.as_str());
            if dx > 0.0 {
                // leading edge: Pin / Unpin on yellow, the row slides right over it
                content.fill_rrect(rx, ry, dx, rh, ui.pt(SPACE_S), YELLOW);
                let label = self.labels.get(ui, if pinned[i] { "Unpin" } else { "Pin" }, MAC_BODY_PT, Weight::Regular, WHITE);
                let lx = (rx + dx / 2.0 - label.w as f32 / 2.0).max(rx + ui.pt(SPACE_S));
                content.blit(label, lx as i32, (ry + rh / 2.0 - label.h as f32 / 2.0) as i32, 255, 255);
                content.fill_rrect(rx + dx, ry, rw - dx, rh, 0.0, CONTENT_BG.now());
            }
            if dx < 0.0 {
                content.fill_rrect(rx, ry, rw, rh, ui.pt(m::SELECTION_RADIUS), DESTRUCTIVE);
                let label = self.labels.get(ui, "Delete", MAC_BODY_PT, Weight::Regular, WHITE);
                let bx = rx + rw + dx;
                let lx = (bx + (-dx).min(ui.pt(DELETE_PT)) / 2.0 - label.w as f32 / 2.0).max(bx + ui.pt(SPACE_S));
                content.blit(label, lx as i32, (ry + rh / 2.0 - label.h as f32 / 2.0) as i32, 255, 255);
                content.fill_rrect(rx, ry, rw + dx, rh, 0.0, CONTENT_BG.now());
            }
            // the open note: the accent plate with white text (Golden Gate Notes); the arrow-key row: a grey plate
            if is_open && dx == 0.0 {
                content.fill_rrect(rx, ry, rw, rh, ui.pt(m::SELECTION_RADIUS), YELLOW);
            } else if selected == Some(i) && dx == 0.0 {
                content.fill_rrect(rx, ry, rw, rh, ui.pt(m::SELECTION_RADIUS), FIELD_BG.now());
            }
            let tx = rx + ui.pt(SPACE_L) + dx;
            let row = &self.rows[self.shown[i]];
            let (title, date, snippet) = if is_open && dx == 0.0 {
                let (a, b, c) = (row.title_text.clone(), row.date_text.clone(), row.snippet_text.clone());
                (
                    self.labels.get(ui, &a, MAC_BODY_PT, Weight::Bold, WHITE).clone(),
                    self.labels.get(ui, &b, MAC_SUBHEAD_PT, Weight::Regular, WHITE).clone(),
                    self.labels.get(ui, &c, MAC_SUBHEAD_PT, Weight::Regular, WHITE).clone(),
                )
            } else {
                (row.title.clone(), row.date.clone(), row.snippet.clone())
            };
            let draw = |c: &mut Surface, img: &Image, x: f32, y: f32| {
                let off = (rx - x).max(0.0) as usize;
                if off < img.w {
                    c.blit(&img.crop(off, 0, img.w - off, img.h), x.max(rx) as i32, y as i32, 255, 255);
                }
            };
            let mid = ry + rh / 2.0;
            draw(&mut content, &title, tx, mid - title.h as f32 - ui.pt(1.0));
            let line2 = mid + ui.pt(1.0);
            draw(&mut content, &date, tx, line2);
            draw(&mut content, &snippet, tx + date.w as f32 + ui.pt(SPACE_S), line2);
            if self.view == View::All && !places[i].is_empty() && dx == 0.0 && !is_open {
                let tag = self.labels.get(ui, &ui.fit(&places[i], MAC_FOOTNOTE_PT, ui.pt(90.0)), MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
                let tagx = rx + rw - ui.pt(SPACE_S) - tag.w as f32;
                if tagx > tx + ui.pt(TAG_MIN_PT) {
                    content.blit(tag, tagx as i32, (ry + ui.pt(SPACE_S)) as i32, 255, 255);
                }
            }
            let next_open = i + 1 < n && open.as_deref() == Some(self.notes[self.shown[i + 1]].id.as_str());
            if i + 1 < n && !sections.iter().any(|(_, start)| *start == i + 1) && !is_open && !next_open {
                content.fill_rrect(rx + ui.pt(SPACE_L), ry + rh - 1.0, rw - ui.pt(SPACE_L), 1.0, 0.0, SEPARATOR_C.now());
            }
        }
        if n == 0 {
            let msg = if !self.search.is_empty() { "No Results" } else if self.view == View::Trash { "No Deleted Notes" } else { "No Notes" };
            let img = self.labels.get(ui, msg, MAC_TITLE3_PT, Weight::Regular, LABEL_2.now());
            centred(&mut content, img, (lx + lw / 2.0) as i32, (ui.pt(list_top_pt()) + ui.pt(SPACE_HUGE * 2.0)) as i32);
        }
    }

    /// The gallery: a grid of cards (the first picture, or a text preview, then title and date) in the list column.
    /// Selecting and opening a card works as a list row does.
    fn draw_gallery(&mut self, s: &mut Surface, ui: &Ui) {
        self.request_thumbs(ui);
        let (lx, _) = self.list_px(ui);
        let bottom = self.content_bottom(ui);
        let (cells, heads, _) = self.gallery_grid(ui);
        let off = self.list_scroll.offset();
        let top = ui.pt(nav_bottom_pt());
        let mut content = s.rows(top as usize, bottom as usize);
        for (k, y) in heads {
            let band_bottom = y - off + ui.pt(Self::header_pt(k));
            if band_bottom < top || y - off > bottom {
                continue;
            }
            let label = self.sections[k].0.clone();
            let img = self.labels.get(ui, &label, MAC_SUBHEAD_PT, Weight::Semibold, LABEL_2.now()).clone();
            content.blit(&img, (lx + ui.pt(SPACE_S)) as i32, (band_bottom - img.h as f32 - ui.pt(SPACE_XS)) as i32, 255, 255);
        }
        let open = self.editing().map(|note| note.id.clone());
        let selected = self.selected_index();
        let pad = ui.pt(SPACE_S);
        let radius = ui.pt(m::CARD_RADIUS);
        for (i, &(x, y, w, h)) in cells.iter().enumerate() {
            let ry = y - off;
            if ry + h < top || ry > bottom {
                continue;
            }
            let note = &self.notes[self.shown[i]];
            let is_open = open.as_deref() == Some(note.id.as_str());
            if is_open || selected == Some(i) {
                content.fill_rrect(x, ry, w, h, radius, YELLOW);
                content.fill_rrect(x + ui.pt(2.0), ry + ui.pt(2.0), w - ui.pt(4.0), h - ui.pt(4.0), radius, CARD_BG.now());
            } else {
                content.fill_rrect(x, ry, w, h, radius, CARD_BG.now());
            }
            let picture = Self::first_picture(&note.text);
            let mut text_top = ry + pad;
            if picture.is_some() {
                let thumb_h = ui.pt(GALLERY_THUMB_PT);
                if let (Some(name), Some((kept, Some(img)))) = (picture, self.thumbs.get(&note.id)) {
                    if kept == name {
                        let shown = img.crop(0, 0, img.w.min(w as usize), img.h.min(thumb_h as usize));
                        content.blit(&shown, x as i32, ry as i32, 255, 255);
                    }
                }
                text_top = ry + thumb_h + pad;
            }
            let inner = w - 2.0 * pad;
            let title_text = ui.fit(note.title(), MAC_BODY_PT, inner);
            let title = self.labels.get(ui, &title_text, MAC_BODY_PT, Weight::Bold, LABEL.now()).clone();
            content.blit(&title, (x + pad) as i32, text_top as i32, 255, 255);
            let date_text = self.rows[self.shown[i]].date_text.clone();
            let date = self.labels.get(ui, &date_text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now()).clone();
            let date_y = text_top + title.h as f32 + pad / 2.0;
            content.blit(&date, (x + pad) as i32, date_y as i32, 255, 255);
            if picture.is_none() {
                let preview_text = ui.fit(&note.row_snippet(), MAC_FOOTNOTE_PT, inner);
                let preview = self.labels.get(ui, &preview_text, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now()).clone();
                content.blit(&preview, (x + pad) as i32, (date_y + date.h as f32 + pad / 2.0) as i32, 255, 255);
            }
        }
        if cells.is_empty() {
            let msg = if !self.search.is_empty() { "No Results" } else if self.view == View::Trash { "No Deleted Notes" } else { "No Notes" };
            let img = self.labels.get(ui, msg, MAC_TITLE3_PT, Weight::Regular, LABEL_2.now()).clone();
            centred(&mut content, &img, (lx + ui.pt(SPACE_S)) as i32, (ui.pt(list_top_pt()) + ui.pt(SPACE_HUGE * 2.0)) as i32);
        }
    }

    fn draw_editor(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        let (ex, ew) = self.ed_px(ui);
        let bottom = self.content_bottom(ui);
        let top = ui.pt(nav_bottom_pt());
        let m = ui.pt(MARGIN_PT);
        if self.screen == Screen::List {
            let img = self.labels.get(ui, "No Note Selected", MAC_TITLE3_PT, Weight::Regular, LABEL_3.now());
            centred(s, img, (ex + ew / 2.0) as i32, ((top + bottom) / 2.0) as i32);
            return;
        }
        if self.note_settings.dark_background {
            s.fill_rect(ex as usize, top as usize, ew as usize, (bottom - top).max(0.0) as usize, CONTENT_BG.dark);
        }
        {
            let mut content = s.rows(top as usize, bottom as usize);
            let (ox, oy) = self.text_origin(ui);
            if let Some(d) = &self.date {
                centred(&mut content, d, (ex + ew / 2.0) as i32, (oy - ui.pt(TEXT_TOP_PT - DATE_TOP_PT)) as i32);
            }
            self.apply_folds(ui);
            self.editor.draw(&mut content, ui, ox, oy, t);
            let size = ui.pt(CHEVRON_PT);
            for (_, (hx, hy, _, hh), folded) in self.fold_chevrons(ui) {
                let (cx, cy) = (hx + ui.pt(CHEVRON_GAP_PT), hy + (hh - size) / 2.0);
                disclosure(&mut content, cx, cy, size, folded, LABEL_2.now());
            }
        }
        if let Some(line) = self.linked_label() {
            // the Linked Notes line: how many other notes link to the open one, under the text at the right edge
            let img = self.labels.get(ui, &line, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(img, (ex + ew - m - img.w as f32) as i32, (bottom - img.h as f32 - ui.pt(6.0)) as i32, 255, 255);
        }
        if self.find_strip() {
            let ftop = self.find_top(ui);
            let (cur, total) = self.match_state();
            s.fill_rect(ex as usize, ftop as usize, ew as usize, (self.h as f32 - ftop) as usize, CARD_BG.now());
            s.fill_rrect(ex, ftop, ew, 1.0, 0.0, SEPARATOR_C.now());
            let mid = ftop + ui.pt(SHORTCUT_PT) / 2.0;
            for (i, label) in ["Prev", "Next"].into_iter().enumerate() {
                let img = self.labels.get(ui, label, MAC_BODY_PT, Weight::Medium, YELLOW);
                centred(s, img, (ex + m + ui.pt(UNDO_W_PT * (0.5 + i as f32))) as i32, (mid - img.h as f32 / 2.0) as i32);
            }
            let img = self.labels.get(ui, &format!("{cur} of {total}"), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now());
            s.blit(img, (ex + ew - m - img.w as f32) as i32, (mid - img.h as f32 / 2.0) as i32, 255, 255);
        }
        if self.keyboard.shown() {
            let (dx, dy, dw, dh) = self.done_rect(ui);
            let done = self.labels.get(ui, "Done", MAC_BODY_PT, Weight::Semibold, YELLOW);
            s.blit(done, (dx + dw - m - done.w as f32) as i32, (dy + dh / 2.0 - done.h as f32 / 2.0) as i32, 255, 255);
        }
        if self.keyboard.shown() || self.keyboard.animating() {
            // the on-screen keyboard (handheld) with its shortcut row: undo, redo, formatting, paragraph style
            let kb_top = self.kb_top(ui);
            let slide = self.keyboard.offset(t);
            let bar = kb_top - ui.pt(SHORTCUT_PT) + slide;
            s.fill_rect(0, bar as usize, self.w as usize, self.h as usize, CARD_BG.now());
            s.fill_rrect(0.0, bar, self.w as f32, 1.0, 0.0, SEPARATOR_C.now());
            let mid = bar + ui.pt(SHORTCUT_PT) / 2.0;
            let has = |c: char| ui.font.glyph(c) != 0;
            let (undo, redo) = if has('↶') && has('↷') { ("↶", "↷") } else { ("Undo", "Redo") };
            for (i, (label, on)) in [(undo, self.editor.can_undo()), (redo, self.editor.can_redo())].into_iter().enumerate() {
                let img = self.labels.get(ui, label, MAC_TITLE1_PT, Weight::Regular, if on { LABEL.now() } else { LABEL_3.now() });
                centred(s, img, (m + ui.pt(UNDO_W_PT * (0.5 + i as f32))) as i32, (mid - img.h as f32 / 2.0) as i32);
            }
            for (k, (label, _)) in FMT_BUTTONS.iter().enumerate() {
                let cx = m + ui.pt(2.0 * UNDO_W_PT + FMT_W_PT * (k as f32 + 0.5));
                if cx + ui.pt(FMT_W_PT / 2.0) > self.w as f32 - m - ui.pt(60.0) {
                    break;
                }
                let weight = if k == 0 { Weight::Bold } else { Weight::Regular };
                let img = self.labels.get(ui, label, MAC_TITLE3_PT, weight, LABEL.now());
                centred(s, img, cx as i32, (mid - img.h as f32 / 2.0) as i32);
            }
            let aa = self.labels.get(ui, "Aa", MAC_TITLE3_PT, Weight::Regular, LABEL.now());
            centred(s, aa, (self.w as f32 - m - ui.pt(30.0)) as i32, (mid - aa.h as f32 / 2.0) as i32);
            self.keyboard.draw(s, ui, kb_top, t);
            s.fill_rect(0, (kb_top + self.keyboard.h() + slide) as usize, self.w as usize, self.h as usize, keyboard::BG);
        }
    }

    /// The Linked Notes line of the open note ("2 linked notes"), or `None` when no other note links to it.
    fn linked_label(&self) -> Option<String> {
        let Screen::Editor(i) = self.screen else { return None };
        let target = self.notes.get(i)?.id.as_str();
        let pairs: Vec<(&str, &str)> = self.notes.iter().map(|n| (n.id.as_str(), n.text.as_str())).collect();
        let panel = note_url::linked_panel(&pairs, target);
        (!panel.ids.is_empty()).then_some(panel.label)
    }

    /// The folder-name prompt at the top of the list column.
    fn draw_naming(&mut self, s: &mut Surface, ui: &Ui) {
        let (lx, lw) = self.list_px(ui);
        let Some(naming) = &self.naming else { return };
        let pad = ui.pt(SPACE_S);
        let (x, y, w, h) = (lx + pad, ui.pt(nav_bottom_pt()) + pad, lw - 2.0 * pad, ui.pt(56.0));
        s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        s.stroke_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), 1.0, SEPARATOR_C.now());
        let (label, placeholder) = match (&naming.link, &naming.old) {
            (Some(_), _) => ("Add Link", "Web Address"),
            (None, Some(_)) => ("Rename Folder", "Folder Name"),
            (None, None) => ("New Folder", "Folder Name"),
        };
        let mut field = bifrost_app::controls::TextField::new();
        field.push_str(naming.field.text());
        let img = self.labels.get(ui, label, MAC_FOOTNOTE_PT, Weight::Semibold, LABEL_2.now());
        s.blit(img, (x + ui.pt(SPACE_S)) as i32, (y + ui.pt(SPACE_XS)) as i32, 255, 255);
        let r = chrome::pt_rect(ui, (x + ui.pt(SPACE_S), y + ui.pt(SPACE_L + SPACE_XS), w - ui.pt(SPACE_L), ui.pt(SPACE_XXL)));
        bifrost_app::widgets::text_field(s, ui, &field, r, placeholder, true);
    }

    /// The open menu: the kit's `NativeMenu` drawing on the popover material.
    fn draw_menu(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(menu) = &self.menu else { return };
        let items = menu.items.iter().enumerate().map(|(k, (label, _, current))| {
            let (name, count) = label.split_once('\t').unwrap_or((label, ""));
            let item = bifrost_app::controls::menu::MenuItem::new(k as u32, name).checked(*current);
            if count.is_empty() { item } else { item.shortcut(count) }
        }).collect();
        let k = ui.pt(1.0);
        bifrost_app::widgets::menu(s, ui, (menu.x / k, menu.y / k), MENU_W_PT, &bifrost_app::controls::menu::Menu::new(items));
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        self.sync_width(ui);
        self.follow_appearance(ui, t);
        self.draw_background(s);
        self.draw_list(s, ui);
        self.draw_editor(s, ui, t);
        self.draw_chrome(s, ui);
        self.draw_naming(s, ui);
        self.draw_replacing(s, ui);
        self.draw_settings(s, ui);
        self.draw_menu(s, ui);
        self.draw_picker(s, ui);
        self.draw_sketch(s, ui);
        let (ex, ew) = self.ed_px(ui);
        if let Some(error) = self.pin_error.as_ref().or(self.notice.as_ref()) {
            let text = ui.fit(error, MAC_FOOTNOTE_PT, ew - ui.pt(SPACE_HUGE));
            let y = self.content_bottom(ui) - ui.pt(SPACE_HUGE);
            let image = self.labels.get(ui, &text, MAC_FOOTNOTE_PT, Weight::Regular, RED);
            s.fill_rrect(ex + ui.pt(SPACE_M), y, ew - ui.pt(SPACE_XL), ui.pt(36.0), ui.pt(m::CARD_RADIUS), CARD_BG.now());
            centred(s, image, (ex + ew / 2.0) as i32, (y + ui.pt(SPACE_XL) - image.h as f32 / 2.0) as i32);
        } else if self.undo_toast.is_some() {
            let (x, y, w, h) = self.toast_rect(ui);
            s.fill_rrect(x, y, w, h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
            let label = self.labels.get(ui, "Note deleted", MAC_FOOTNOTE_PT, Weight::Regular, LABEL.now());
            s.blit(label, (x + ui.pt(SPACE_M)) as i32, (y + h / 2.0 - label.h as f32 / 2.0) as i32, 255, 255);
            let undo = self.labels.get(ui, "Undo", MAC_FOOTNOTE_PT, Weight::Semibold, LINK);
            s.blit(undo, (x + w - ui.pt(SPACE_M) - undo.w as f32) as i32, (y + h / 2.0 - undo.h as f32 / 2.0) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn ui() -> Ui {
        crate::test_ui()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-notes-app")).join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// 2026-10-04 00:23 UTC on a frame clock that reads 0 then.
    fn wall() -> Wall {
        Wall { unix: 1_791_073_380, offset: 0, at: 0.0 }
    }

    /// Waits for the background image decode to land.
    fn settle_images(n: &mut Notes, ui: &Ui) {
        for i in 0..1000 {
            n.tick(ui, 3000.0 + i as f64);
            if n.images_slot.is_none() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn app(ui: &Ui, dir: &Path) -> Notes {
        let mut n = Notes::new(ui, 2400, 1500, dir.to_path_buf(), wall()); // a desktop window: the three-column toolbar fits
        // The tests type plain body text; the Title start is covered by the settings tests.
        n.note_settings.new_note_start = settings::NewNoteStart::Body;
        n
    }

    fn tap(n: &mut Notes, ui: &Ui, x: f32, y: f32, t: f64) {
        n.touch(ui, Event::Down(x, y), t);
        n.touch(ui, Event::Up(x, y), t + 30.0);
    }

    fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
        (x + w / 2.0, y + h / 2.0)
    }

    fn tap_key(n: &mut Notes, ui: &Ui, label: &str, t: f64) {
        let (x, y) = n.keyboard.centre(label).unwrap_or_else(|| panic!("no key {label}"));
        tap(n, ui, x, n.kb_top(ui) + y, t);
    }

    fn type_text(n: &mut Notes, ui: &Ui, s: &str, t: f64) {
        for c in s.chars() {
            n.key(ui, if c == '\n' { Out::Enter } else { Out::Text(c.to_string()) }, t);
        }
    }

    fn draw(n: &mut Notes, ui: &Ui, t: f64) -> Vec<u32> {
        let mut buf = vec![0u32; 2400 * 1500];
        n.draw(&mut Surface { w: 2400, h: 1500, stride: 2400, px: &mut buf, y0: 0 }, ui, t);
        buf
    }

    #[test]
    fn the_open_note_and_the_folder_view_come_back_after_a_relaunch() {
        let ui = ui();
        let dir = scratch("restore");
        let mut n = app(&ui, &dir);
        let id = n.import_note(&ui, "Shopping\nmilk", 1000.0);
        n.import_note(&ui, "Other", 1100.0);
        n.show_note(&ui, &id, 1200.0);
        let (view, open) = n.restorable();
        assert_eq!((view.clone(), open.clone()), (View::All, Some(id.clone())));
        n.flush();
        // a relaunch: a new Notes over the same directory shows the list with nothing open
        let mut again = app(&ui, &dir);
        assert!(again.editing().is_none());
        assert!(again.restore(&ui, view, open.as_deref(), 2000.0));
        assert_eq!(again.editing().map(|x| x.id.as_str()), Some(id.as_str()), "the same note is open");
        // a folder or note that no longer exists is not an error
        assert!(again.restore(&ui, View::Folder("Gone".into()), Some("missing"), 3000.0));
        assert_eq!(*again.view(), View::All, "an unknown folder falls back to All Notes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_autosave_firing_after_the_screen_changed_still_writes_the_edited_note() {
        let ui = ui();
        let dir = scratch("late-autosave");
        let mut n = app(&ui, &dir);
        let (cx, cy) = centre(n.compose_rect(&ui));
        tap(&mut n, &ui, cx, cy, 1000.0);
        tap_key(&mut n, &ui, "h", 2000.0);
        tap_key(&mut n, &ui, "i", 2100.0);
        n.screen = Screen::List;
        n.tick(&ui, 2700.0);
        let saved = store::load(&dir);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].text, "hi");
    }

    #[test]
    fn an_imported_file_becomes_a_note_saved_at_once_and_opened() {
        let ui = ui();
        let dir = scratch("import");
        let mut n = app(&ui, &dir);
        n.import_note(&ui, "Shopping\nmilk\neggs", 1000.0);
        assert_eq!(n.editing().unwrap().text, "Shopping\nmilk\neggs");
        let saved = store::load(&dir);
        assert_eq!((saved.len(), saved[0].text.as_str()), (1, "Shopping\nmilk\neggs"), "on disk without waiting for the autosave");
    }

    #[test]
    fn compose_creates_a_note_that_autosaves_half_a_second_after_typing() {
        let ui = ui();
        let dir = scratch("compose");
        let mut n = app(&ui, &dir);
        assert!(n.listed().is_empty());
        let (cx, cy) = centre(n.compose_rect(&ui));
        tap(&mut n, &ui, cx, cy, 1000.0);
        assert!(n.editing().is_some() && n.keyboard_shown());
        assert!(n.editor.focused);
        tap_key(&mut n, &ui, "h", 2000.0);
        tap_key(&mut n, &ui, "i", 2100.0);
        assert_eq!(n.editing().unwrap().text, "hi");
        n.tick(&ui, 2200.0);
        assert_eq!(n.next_wake(2200.0), Some(2630.0), "the autosave (and the caret blink) is the next deadline");
        n.tick(&ui, 2600.0);
        assert!(store::load(&dir).is_empty(), "not written yet");
        n.tick(&ui, 2630.0);
        let saved = store::load(&dir);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].text, "hi");
        assert!(!n.autosave.pending());
        tap_key(&mut n, &ui, "return", 3000.0);
        type_text(&mut n, &ui, "body", 3000.0);
        let (dx, dy) = centre(n.done_rect(&ui));
        tap(&mut n, &ui, dx, dy, 4000.0);
        assert!(!n.keyboard_shown(), "Done hides the keyboard");
        assert_eq!(store::load(&dir)[0].text, "hi\nbody", "and saves at once");
        draw(&mut n, &ui, 4100.0);
        assert!(n.escape(&ui, 5000.0), "Esc closes the note");
        assert_eq!(n.editing(), None);
        assert_eq!(n.listed(), vec![(saved[0].id.as_str(), "hi")]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_note_is_dropped_on_leaving() {
        let ui = ui();
        let dir = scratch("empty");
        let mut n = app(&ui, &dir);
        n.activate(&ui, Target::Compose, 0.0, 0.0, 1000.0);
        type_text(&mut n, &ui, " \n ", 1000.0);
        n.tick(&ui, 2000.0);
        assert_eq!(store::load(&dir).len(), 1);
        n.activate(&ui, Target::Back, 0.0, 0.0, 3000.0);
        assert!(n.listed().is_empty());
        assert!(store::load(&dir).is_empty(), "the file is gone too");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn seeded(ui: &Ui, dir: &Path) -> Notes {
        let now = wall().unix;
        store::save(dir, &Note { id: "1".into(), text: "Groceries\nmilk".into(), modified: 0 }).unwrap();
        store::save(dir, &Note { id: "2".into(), text: "Ideas\nrust shell".into(), modified: 0 }).unwrap();
        let mut n = app(ui, dir);
        // files share an mtime: pin the order
        for note in &mut n.notes {
            note.modified = if note.id == "2" { now - 86_400 * 3 } else { now - 60 };
        }
        n.rebuild(ui, 0.0);
        n
    }

    /// The ids a view lists, in list order.
    fn listed_ids(n: &mut Notes, ui: &Ui, view: View) -> Vec<String> {
        n.view = view;
        n.rebuild(ui, 10.0);
        n.listed().iter().map(|(id, _)| id.to_string()).collect()
    }

    #[test]
    fn smart_folders_pick_notes_by_their_fields() {
        let ui = ui();
        let dir = scratch("smart");
        let mut n = seeded(&ui, &dir);
        // Note 1 is an old picture note, note 2 a checklist changed a second ago (the week's cutoff is wall - 604800 s).
        let now = wall().unix;
        for note in &mut n.notes {
            note.modified = now - 40 * 86_400;
            if note.id == "1" {
                note.text = "Trip\n![image](a-1.png)".into();
            } else {
                note.text = "Shopping\n- [ ] milk\n- [x] eggs".into();
                note.modified = now - 1;
            }
        }
        n.loc.remove("1");
        n.loc.remove("2");
        n.pinned.insert("1".to_string());
        assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Checklists)), vec!["2"]);
        assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Photos)), vec!["1"]);
        assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Pinned)), vec!["1"]);
        assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Recent)), vec!["2"]);
        // the sidebar and the list menu count the same notes
        assert_eq!(n.count_in(&View::Smart(smart::Smart::Checklists)), 1);
        assert_eq!(n.count_in(&View::Smart(smart::Smart::Pinned)), 1);
        // a bin note never shows in a smart folder
        n.loc.insert("2".to_string(), folders::TRASH.to_string());
        assert_eq!(listed_ids(&mut n, &ui, View::Smart(smart::Smart::Checklists)), Vec::<String>::new());
        assert_eq!(n.view_title(), "Checklists");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_one_and_replace_all_change_the_open_note() {
        let ui = ui();
        let dir = scratch("replace");
        let mut n = seeded(&ui, &dir);
        n.new_note(&ui, 0.0);
        n.editor.set_text(&ui, "one cat, cat\nthe Cat sat");
        n.changed(&ui, 0.5);
        n.hl_query = "cat".to_string();
        n.refresh_highlights(&ui, false);
        assert_eq!(n.hl.len(), 3);
        n.start_replacing(&ui, 1.0);
        assert!(n.replacing.is_some() && n.keyboard_shown());
        n.replacing.as_mut().unwrap().field.set_text(&ui, "dog");
        // Replace changes the selected match only; the bar moves on to the next one
        n.replace_one(&ui, 2.0);
        assert_eq!(n.editor.text(), "one dog, cat\nthe Cat sat");
        assert_eq!(n.hl.len(), 2);
        assert_eq!(n.hl_cur, 0);
        // Replace All changes the rest in one step and closes the card
        n.replace_all(&ui, 3.0);
        assert_eq!(n.editor.text(), "one dog, dog\nthe dog sat");
        assert!(n.replacing.is_none() && !n.keyboard_shown());
        assert_eq!(n.editing().map(|note| note.text.as_str()), Some("one dog, dog\nthe dog sat"));
        assert!(n.editor.undo(&ui, 4.0));
        assert_eq!(n.editor.text(), "one dog, cat\nthe Cat sat", "one undo step takes back Replace All");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_needs_find_text_first() {
        let ui = ui();
        let dir = scratch("replace-none");
        let mut n = seeded(&ui, &dir);
        n.new_note(&ui, 0.0);
        n.hl_query.clear();
        n.start_replacing(&ui, 1.0);
        assert!(n.replacing.is_none(), "without find text the card does not open");
        assert_eq!(n.notice.as_deref(), Some("Search for text first, then replace it"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_sorts_newest_first_searches_and_opens_rows() {
        let ui = ui();
        let dir = scratch("list");
        let mut n = seeded(&ui, &dir);
        assert_eq!(n.listed(), vec![("1", "Groceries"), ("2", "Ideas")]);
        let (sx, sy) = centre(n.search_rect(&ui));
        tap(&mut n, &ui, sx, sy, 1000.0);
        assert!(n.keyboard_shown() && n.searching);
        type_text(&mut n, &ui, "RUST", 1100.0);
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        n.key(&ui, Out::Enter, 1200.0);
        assert!(!n.keyboard_shown());
        draw(&mut n, &ui, 1300.0);
        let (rx, ry) = centre(n.row_rect(&ui, 0));
        tap(&mut n, &ui, rx, ry, 2000.0);
        assert_eq!(n.editing().map(|e| e.id.as_str()), Some("2"));
        assert!(!n.keyboard_shown(), "opening an existing note shows it without the keyboard");
        let buf = draw(&mut n, &ui, 2100.0);
        assert!(buf.contains(&WHITE));
        // tapping the text brings the keyboard and places the caret
        let (ox, oy) = n.text_origin(&ui);
        tap(&mut n, &ui, ox + 1.0, oy + 5.0, 3000.0);
        assert!(n.keyboard_shown());
        assert_eq!(n.editor.caret(), 0);
        type_text(&mut n, &ui, "My ", 3100.0);
        assert_eq!(n.editing().unwrap().text, "My Ideas\nrust shell");
        n.tick(&ui, 4000.0);
        n.activate(&ui, Target::Back, 0.0, 0.0, 5000.0);
        assert_eq!(n.listed(), vec![("2", "My Ideas")], "the search still filters");
        n.key(&ui, Out::Backspace, 5100.0);
        n.key(&ui, Out::Backspace, 5100.0);
        n.key(&ui, Out::Backspace, 5100.0);
        n.key(&ui, Out::Backspace, 5100.0);
        assert_eq!(n.listed(), vec![("2", "My Ideas"), ("1", "Groceries")], "the edited note moved to the top");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swiping_right_reveals_pin_and_a_full_swipe_toggles_it() {
        let ui = ui();
        let dir = scratch("swipe-pin");
        let mut n = seeded(&ui, &dir);
        let (rx, ry, _, rh) = n.row_rect(&ui, 0);
        let y = ry + rh / 2.0;
        let id = n.notes[n.shown[0]].id.clone();
        // half swipe rests open with Pin revealed; tapping it pins the note
        let x0 = rx + 20.0;
        n.touch(&ui, Event::Down(x0, y), 0.0);
        for i in 1..=8 {
            n.touch(&ui, Event::Move(x0 + 10.0 * i as f32, y + 2.0), i as f64 * 16.0);
        }
        n.touch(&ui, Event::Up(x0 + 80.0, y), 200.0);
        assert!(n.reveal.is_some_and(|(r, dx)| r == 0 && dx > 0.0), "rests with Pin revealed");
        let buf = draw(&mut n, &ui, 300.0);
        assert!(buf.contains(&YELLOW));
        tap(&mut n, &ui, rx + 10.0, y, 400.0);
        assert!(n.is_pinned(&id), "tapping the revealed Pin pins it");
        assert_eq!(n.reveal, None);
        // full swipe on the same row toggles it back
        let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
        let y = ry + rh / 2.0;
        let x0 = rx + 10.0;
        n.touch(&ui, Event::Down(x0, y), 1000.0);
        for i in 1..=10 {
            n.touch(&ui, Event::Move(x0 + rw * 0.06 * i as f32, y + 2.0), 1000.0 + i as f64 * 16.0);
        }
        n.touch(&ui, Event::Up(x0 + rw * 0.6, y), 1300.0);
        assert!(!n.is_pinned(&id), "a full swipe toggles the pin");
        assert_eq!(n.reveal, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn swiping_a_row_reveals_delete_and_a_full_swipe_deletes() {
        let ui = ui();
        let dir = scratch("swipe");
        let mut n = seeded(&ui, &dir);
        let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
        let y = ry + rh / 2.0;
        let x0 = rx + rw - 50.0;
        n.touch(&ui, Event::Down(x0, y), 0.0);
        // the list column is narrower on the Mac layout: stay between the reveal width and a full swipe (FULL_SWIPE)
        let swipe = (ui.pt(DELETE_PT) + rw * FULL_SWIPE) / 2.0;
        for i in 1..=10 {
            n.touch(&ui, Event::Move(x0 - swipe * i as f32 / 10.0, y + 2.0), i as f64 * 16.0);
        }
        assert!(n.reveal.is_some_and(|(r, dx)| r == 0 && dx < 0.0));
        n.touch(&ui, Event::Up(x0 - swipe, y), 200.0);
        assert_eq!(n.reveal, Some((0, -ui.pt(DELETE_PT))), "rests open");
        assert_eq!(n.listed().len(), 2, "nothing deleted yet");
        let buf = draw(&mut n, &ui, 300.0);
        assert!(buf.contains(&DESTRUCTIVE));
        // a tap on the row body closes it without opening the note
        tap(&mut n, &ui, rx + 30.0, y, 1000.0);
        assert_eq!(n.reveal, None);
        assert_eq!(n.editing(), None);
        // open again and tap Delete
        n.touch(&ui, Event::Down(x0, y), 2000.0);
        n.touch(&ui, Event::Move(x0 - 150.0, y), 2050.0);
        n.touch(&ui, Event::Up(x0 - 150.0, y), 2100.0);
        assert!(n.reveal.is_some());
        tap(&mut n, &ui, rx + rw - 20.0, y, 3000.0);
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        assert_eq!(store::load(&dir).len(), 1);
        // a full swipe deletes on release
        let (rx, ry, rw, rh) = n.row_rect(&ui, 0);
        let (y, x0) = (ry + rh / 2.0, rx + rw - 10.0);
        n.touch(&ui, Event::Down(x0, y), 4000.0);
        n.touch(&ui, Event::Move(x0 - rw * 0.3, y), 4050.0);
        n.touch(&ui, Event::Move(x0 - rw * 0.6, y), 4100.0);
        n.touch(&ui, Event::Up(x0 - rw * 0.6, y), 4150.0);
        assert!(n.listed().is_empty());
        assert!(store::load(&dir).is_empty());
        let buf = draw(&mut n, &ui, 5000.0);
        assert!(buf.contains(&LABEL_2.now()), "the empty message shows");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_vertical_drag_on_a_row_scrolls_instead_of_opening() {
        let ui = ui();
        let dir = scratch("scroll");
        let mut n = app(&ui, &dir);
        for i in 0..40 {
            n.notes.push(Note { id: i.to_string(), text: format!("Note {i}"), modified: 1000 - i });
        }
        n.rebuild(&ui, 0.0);
        let (rx, ry, _, rh) = n.row_rect(&ui, 3);
        let (x, y0) = (rx + 100.0, ry + rh / 2.0);
        n.touch(&ui, Event::Down(x, y0), 0.0);
        for i in 1..=10 {
            n.touch(&ui, Event::Move(x, y0 - 30.0 * i as f32), i as f64 * 16.0);
        }
        n.touch(&ui, Event::Up(x, y0 - 300.0), 170.0);
        assert_eq!(n.editing(), None);
        assert!(n.list_scroll.offset() > 100.0, "scrolled: {}", n.list_scroll.offset());
        let mut t = 200.0;
        while n.tick(&ui, t) && t < 10_000.0 {
            t += 16.0;
        }
        draw(&mut n, &ui, t);
        let first = (0..40).find(|&i| n.row_rect(&ui, i).1 >= ui.pt(list_top_pt())).unwrap();
        assert!(first > 0, "earlier rows scrolled off the top");
        assert_eq!(n.hit(&ui, x, n.row_rect(&ui, first).1 + 5.0), Some(Target::Row(first)));
        assert!(n.next_wake(t).is_none(), "nothing pending once settled");
    }

    #[test]
    fn undo_redo_and_format_from_the_shortcut_bar() {
        let ui = ui();
        let dir = scratch("undo");
        let mut n = app(&ui, &dir);
        n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
        type_text(&mut n, &ui, "Plan\nstep one", 100.0);
        let top = n.shortcut_top(&ui).unwrap();
        let y = top + ui.pt(SHORTCUT_PT) / 2.0;
        assert_eq!(n.hit(&ui, ui.pt(MARGIN_PT + 20.0), y), Some(Target::Undo));
        assert_eq!(n.hit(&ui, ui.pt(MARGIN_PT + 100.0), y), Some(Target::Redo));
        assert_eq!(n.hit(&ui, 2400.0 - ui.pt(MARGIN_PT + 30.0), y), Some(Target::Format));
        tap(&mut n, &ui, ui.pt(MARGIN_PT + 20.0), y, 1000.0);
        assert_eq!(n.editing().unwrap().text, "Plan\nstep ");
        tap(&mut n, &ui, ui.pt(MARGIN_PT + 100.0), y, 2000.0);
        assert_eq!(n.editing().unwrap().text, "Plan\nstep one");
        tap(&mut n, &ui, 2400.0 - ui.pt(MARGIN_PT + 30.0), y, 3000.0);
        assert_eq!(n.editing().unwrap().text, "Plan\n## step one");
        n.activate(&ui, Target::Format, 0.0, 0.0, 3100.0);
        assert_eq!(n.editing().unwrap().text, "Plan\n# step one");
        n.activate(&ui, Target::Format, 0.0, 0.0, 3200.0);
        assert_eq!(n.editing().unwrap().text, "Plan\nstep one");
        assert!(n.autosave.pending());
        let buf = draw(&mut n, &ui, 3300.0);
        assert!(buf.contains(&YELLOW), "Done and the caret are yellow");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn typing_past_the_bottom_scrolls_the_caret_into_view() {
        let ui = ui();
        let dir = scratch("caret");
        let mut n = app(&ui, &dir);
        n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
        for i in 0..60 {
            type_text(&mut n, &ui, &format!("line {i}\n"), 100.0 + i as f64);
        }
        assert!(n.scroll.offset() > 0.0);
        let (_, oy) = n.text_origin(&ui);
        let (_, cy, _, ch) = n.editor.caret_rect(&ui);
        assert!(oy + cy + ch <= n.content_bottom(&ui), "caret above the keyboard");
        assert!(oy + cy >= ui.pt(nav_bottom_pt()));
        for _ in 0..70 {
            n.key(&ui, Out::Named("Up"), 200.0);
        }
        assert_eq!(n.scroll.offset(), 0.0, "scrolled back to the top");
        draw(&mut n, &ui, 300.0);
        assert!(n.describe().contains("notes_screen=editor:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn row_pin_control_persists_groups_and_search_keeps_pins_first() {
        let ui = ui();
        let dir = scratch("pin-grouping");
        let mut n = seeded(&ui, &dir);
        n.activate(&ui, Target::Pin(1), 0.0, 0.0, 1000.0);
        assert!(n.is_pinned("2"));
        assert!(n.editing().is_none(), "pinning a row does not open the note");
        assert_eq!(n.listed(), vec![("2", "Ideas"), ("1", "Groceries")]);
        assert_eq!(n.shown_pins(), 1);
        let pinned = n.row_rect(&ui, 0);
        let regular = n.row_rect(&ui, 1);
        assert_eq!(regular.1 - (pinned.1 + pinned.3), ui.pt(SECTION_PT + SECTION_GAP_PT));
        // Golden Gate Notes has no pin glyph on the row (Pin Note is in the More menu); the pinned section still draws
        let _ = draw(&mut n, &ui, 1100.0);
        assert!(n.labels.len() > 0, "section labels were rendered");

        // Both matches keep their grouping, and a regular-only match has no
        // blank pinned section or leftover section spacing.
        n.search.set_text(&ui, "i"); n.rebuild(&ui, 1200.0);
        assert_eq!(n.listed(), vec![("2", "Ideas"), ("1", "Groceries")]);
        n.search.set_text(&ui, "milk"); n.rebuild(&ui, 1300.0);
        assert_eq!(n.listed(), vec![("1", "Groceries")]);
        assert_eq!(n.shown_pins(), 0);
        assert_eq!(n.row_rect(&ui, 0).1, ui.pt(list_top_pt()));
        n.search.set_text(&ui, "RUST"); n.rebuild(&ui, 1400.0);
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        assert_eq!(n.shown_pins(), 1);
        draw(&mut n, &ui, 1500.0);

        let mut restarted = app(&ui, &dir);
        assert!(restarted.is_pinned("2"));
        assert_eq!(restarted.listed()[0].0, "2");
        restarted.activate(&ui, Target::Pin(0), 0.0, 0.0, 2000.0);
        assert!(!restarted.is_pinned("2"));
        assert!(!app(&ui, &dir).is_pinned("2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn editor_pin_and_editing_preserve_identity_and_pins_across_restart() {
        let ui = ui();
        let dir = scratch("pin-editing");
        let mut n = seeded(&ui, &dir);
        n.open_note(&ui, 1, true, 0.0);
        let open_id = n.editing().unwrap().id.clone();
        n.activate(&ui, Target::EditorPin, 0.0, 0.0, 1000.0);
        assert!(n.is_pinned(&open_id));
        assert_eq!(n.editing().unwrap().id, open_id, "pinning must not reorder the editor's index");
        type_text(&mut n, &ui, "\nPinned body edit", 1100.0);
        n.tick(&ui, 1800.0);
        n.close_note(&ui, 2000.0);
        let reloaded = app(&ui, &dir);
        assert!(reloaded.is_pinned(&open_id));
        assert_eq!(reloaded.listed()[0].0, open_id);
        assert!(store::load(&dir).iter().find(|note| note.id == open_id).unwrap().text.ends_with("Pinned body edit"));

        // Composing after sorting pinned notes cannot accidentally edit the
        // first pinned row instead of the newly created note.
        n.new_note(&ui, 3000.0);
        let new_id = n.editing().unwrap().id.clone();
        assert_ne!(new_id, open_id);
        assert!(n.editing().unwrap().text.is_empty());
        type_text(&mut n, &ui, "New regular note", 3100.0);
        n.close_note(&ui, 4000.0);
        assert_eq!(n.listed()[0].0, open_id);
        assert_eq!(n.listed()[1], (new_id.as_str(), "New regular note"));
        assert!(n.set_pinned(&ui, &new_id, true, 5000.0));
        let row = n.shown.iter().position(|&i| n.notes[i].id == new_id).unwrap();
        n.delete_row(&ui, row, 6000.0);
        assert!(!n.is_pinned(&new_id));
        assert!(!app(&ui, &dir).is_pinned(&new_id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_pin_write_leaves_pin_state_and_note_files_unchanged() {
        let ui = ui();
        let dir = scratch("pin-write-failure");
        let mut n = seeded(&ui, &dir);
        let before = std::fs::read(dir.join("1.md")).unwrap();
        let modified = std::fs::metadata(dir.join("1.md")).unwrap().modified().unwrap();
        std::fs::write(dir.join(".springboard-pins-v1"), "Existing unrelated file").unwrap();
        assert!(!n.set_pinned(&ui, "1", true, 1000.0));
        assert!(!n.is_pinned("1"));
        assert!(n.describe().contains("notes_pin_error=true"));
        assert_eq!(std::fs::read(dir.join("1.md")).unwrap(), before);
        assert_eq!(std::fs::metadata(dir.join("1.md")).unwrap().modified().unwrap(), modified);
        assert_eq!(std::fs::read_to_string(dir.join(".springboard-pins-v1")).unwrap(), "Existing unrelated file");
        draw(&mut n, &ui, 1100.0);

        std::fs::remove_file(dir.join(".springboard-pins-v1")).unwrap();
        assert!(n.set_pinned(&ui, "1", true, 1200.0));
        let marker = dir.join(".springboard-pins-v1/31.pin");
        std::fs::write(&marker, "Unknown future format").unwrap();
        assert!(!n.set_pinned(&ui, "1", false, 1300.0));
        assert!(n.is_pinned("1"), "failed unpin preserves its old visible state");
        assert_eq!(std::fs::read_to_string(marker).unwrap(), "Unknown future format");
        assert_eq!(std::fs::read(dir.join("1.md")).unwrap(), before);
        assert_eq!(std::fs::metadata(dir.join("1.md")).unwrap().modified().unwrap(), modified);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tapped_link_in_a_note_is_queued_for_the_shell_to_open() {
        let ui = ui();
        let dir = scratch("link");
        let mut n = app(&ui, &dir);
        n.new_note(&ui, 1000.0);
        n.editor.replace_range(&ui, 0..0, "https://bifrost.olibuijr.com\nhi", 1000.0, false);
        n.changed(&ui, 1000.0);
        let (ox, oy) = n.text_origin(&ui);
        // a tap on the link (first line, first character) opens it and does not move the caret
        let caret = n.editor.caret();
        n.activate(&ui, Target::Text, ox + 1.0, oy + 1.0, 1100.0);
        assert_eq!(n.take_link().as_deref(), Some("https://bifrost.olibuijr.com"));
        assert!(n.take_link().is_none(), "taken once");
        assert_eq!(n.editor.caret(), caret, "the caret stays where it was");
        // a tap on plain text places the caret and queues nothing
        n.activate(&ui, Target::Text, ox + 1.0, oy + ui.pt(120.0), 1200.0);
        assert!(n.take_link().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn share_button_in_the_editor_queues_the_open_note() {
        let ui = ui();
        let dir = scratch("share");
        let mut n = app(&ui, &dir);
        n.new_note(&ui, 1000.0);
        type_text(&mut n, &ui, "Shared body", 1100.0);
        let (x, y) = {
            let (rx, ry, rw, rh) = n.editor_share_rect(&ui);
            (rx + rw / 2.0, ry + rh / 2.0)
        };
        assert_eq!(n.hit(&ui, x, y), Some(Target::EditorShare));
        n.activate(&ui, Target::EditorShare, x, y, 1200.0);
        let item = n.take_share().expect("a share request");
        assert_eq!(item.file_name(), n.editing().map(|note| format!("{}.txt", note.title())).unwrap());
        assert!(n.take_share().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tiny_png(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![];
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&vec![200u8; (w * h * 4) as usize]).unwrap();
        }
        out
    }

    /// Puts `text` into the editor of a fresh note in one edit.
    fn open_with(n: &mut Notes, ui: &Ui, text: &str) {
        n.new_note(ui, 1000.0);
        n.editor.replace_range(ui, 0..0, text, 1000.0, false);
        n.changed(ui, 1000.0);
    }

    #[test]
    fn deleting_a_note_offers_undo_that_returns_it_to_its_folder() {
        let dir = scratch("undo-delete");
        let ui = ui();
        let mut n = app(&ui, &dir);
        n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
        type_text(&mut n, &ui, "Keep me", 100.0);
        n.activate(&ui, Target::Back, 0.0, 0.0, 300.0);
        let id = n.notes[0].id.clone();
        assert!(n.trash_note(0, 400.0));
        assert!(n.undo_toast.is_some(), "undo bar offered");
        assert_eq!(n.loc_of(&id), folders::TRASH);
        let _ = draw(&mut n, &ui, 401.0);
        n.activate(&ui, Target::UndoDelete, 0.0, 0.0, 500.0);
        assert!(n.undo_toast.is_none());
        assert_eq!(n.loc_of(&id), "", "back in All Notes");
        assert!(n.notes.iter().any(|x| x.id == id));
        n.trash_note(0, 600.0);
        n.tick(&ui, 600.0 + UNDO_TOAST_MS + 1.0);
        assert!(n.undo_toast.is_none(), "the bar goes away after five seconds");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_copies_text_and_images_and_clear_completed_drops_ticked_items() {
        let dir = scratch("duplicate");
        let ui = ui();
        let mut n = app(&ui, &dir);
        n.activate(&ui, Target::Compose, 0.0, 0.0, 0.0);
        type_text(&mut n, &ui, "Plan\n- [x] done\n- [ ] open\n", 100.0);
        let src = n.editing().unwrap().id.clone();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{src}-1.png")), b"img").unwrap();
        n.editor.set_text(&ui, &format!("Plan\n![image]({src}-1.png)\n- [x] done\n- [ ] open"));
        n.changed(&ui, 200.0);
        n.activate(&ui, Target::Back, 0.0, 0.0, 300.0);
        let before = n.notes.len();

        n.open_note(&ui, n.notes.iter().position(|x| x.id == src).unwrap(), false, 400.0);
        n.run(&ui, Act::Duplicate, 500.0);
        assert_eq!(n.notes.len(), before + 1, "one copy");
        let copy = n.editing().unwrap().clone();
        assert_ne!(copy.id, src, "the copy is a new note and it is open");
        assert!(copy.text.contains(&format!("({}-1.png)", copy.id)), "image reference follows the copy");
        assert!(dir.join(format!("{}-1.png", copy.id)).exists(), "image copied beside the note");
        assert!(dir.join(format!("{}.md", copy.id)).exists(), "copy saved");

        n.run(&ui, Act::ClearChecked, 600.0);
        assert!(!n.editor.text().contains("[x]"), "ticked items cleared");
        assert!(n.editor.text().contains("- [ ] open") && n.editor.text().contains("Plan"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cmd_b_i_delete_and_escape_follow_the_desktop_keys() {
        let dir = scratch("chords-more");
        let ui = ui();
        let mut n = app(&ui, &dir);
        for (text, t) in [("Keep", 0.0), ("Drop", 500.0)] {
            n.activate(&ui, Target::Compose, 0.0, 0.0, t);
            type_text(&mut n, &ui, text, t + 100.0);
            n.activate(&ui, Target::Back, 0.0, 0.0, t + 300.0);
        }
        // Cmd+Backspace on the list deletes the arrow-selected note
        n.key(&ui, Out::Named("Down"), 1000.0);
        let before = n.listed().len();
        assert!(n.chord(&ui, 14, bifrost_app::proto::CHORD_META, 1001.0), "Cmd+Backspace");
        assert_eq!(n.listed().len(), before - 1, "one note moved to Recently Deleted");
        // Cmd+B and Cmd+I wrap the editor's text in the markers
        n.activate(&ui, Target::Compose, 0.0, 0.0, 2000.0);
        assert!(n.chord(&ui, 48, bifrost_app::proto::CHORD_CTRL, 2001.0), "Ctrl+B");
        assert!(n.editing().unwrap().text.contains("**"), "bold markers");
        assert!(n.chord(&ui, 23, bifrost_app::proto::CHORD_META, 2002.0), "Cmd+I");
        assert!(n.editing().unwrap().text.contains('*'), "italic marker");
        // Escape closes the open note and returns to the list
        assert!(n.escape(&ui, 2003.0));
        assert!(matches!(n.screen, Screen::List));
        assert!(!n.escape(&ui, 2004.0), "nothing left to close");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cmd_chords_compose_search_and_undo() {
        let dir = scratch("chords");
        let ui = ui();
        let mut n = app(&ui, &dir);
        assert!(n.chord(&ui, 49, bifrost_app::proto::CHORD_CTRL, 0.0), "Ctrl+N");
        assert!(matches!(n.screen, Screen::Editor(_)), "a new note opens");
        assert!(n.chord(&ui, 44, bifrost_app::proto::CHORD_CTRL, 1.0), "Ctrl+Z in the editor");
        assert!(!n.chord(&ui, 30, bifrost_app::proto::CHORD_CTRL, 2.0), "Ctrl+A is not bound");
        n.activate(&ui, Target::Back, 0.0, 0.0, 3.0);
        assert!(n.chord(&ui, 33, bifrost_app::proto::CHORD_META, 4.0), "Cmd+F on the list");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arrow_keys_select_list_rows_and_enter_opens_the_selected_note() {
        let dir = scratch("arrow-list");
        let ui = ui();
        let mut n = app(&ui, &dir);
        for (text, t) in [("First note", 0.0), ("Second note", 500.0)] {
            n.activate(&ui, Target::Compose, 0.0, 0.0, t);
            type_text(&mut n, &ui, text, t + 100.0);
            n.activate(&ui, Target::Back, 0.0, 0.0, t + 300.0);
        }
        n.key(&ui, Out::Named("Down"), 1000.0);
        assert_eq!(n.selected_index(), Some(0), "Down from nothing selects the top row");
        n.key(&ui, Out::Named("Down"), 1001.0);
        n.key(&ui, Out::Named("Down"), 1002.0);
        assert_eq!(n.selected_index(), Some(1), "stops at the last row");
        n.key(&ui, Out::Named("Up"), 1003.0);
        assert_eq!(n.selected_index(), Some(0));
        n.key(&ui, Out::Enter, 1004.0);
        assert!(matches!(n.screen, Screen::Editor(_)), "Enter opens the selected note");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tapping_a_checkbox_toggles_it_and_undo_restores_it() {
        let ui = ui();
        let dir = scratch("checklist");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Todo\n- [ ] milk\n- [ ] eggs\n- [ ] bread");
        let (ox, oy) = n.text_origin(&ui);
        let at = |n: &Notes, c: usize| oy + n.editor.y_of(c) + 3.0;
        let y = at(&n, 5);
        tap(&mut n, &ui, ox + 2.0, y, 2000.0);
        assert_eq!(n.editing().unwrap().text, "Todo\n- [x] milk\n- [ ] eggs\n- [ ] bread");
        assert!(n.autosave.pending(), "the toggle autosaves");
        n.activate(&ui, Target::Undo, 0.0, 0.0, 2100.0);
        assert_eq!(n.editing().unwrap().text, "Todo\n- [ ] milk\n- [ ] eggs\n- [ ] bread");
        n.activate(&ui, Target::Redo, 0.0, 0.0, 2200.0);
        assert_eq!(n.editing().unwrap().text, "Todo\n- [x] milk\n- [ ] eggs\n- [ ] bread");
        let y = at(&n, 5);
        tap(&mut n, &ui, ox + 2.0, y, 3000.0);
        assert!(n.editing().unwrap().text.contains("- [ ] milk"), "tapping again unticks");
        // the box is the target; the text beside it just places the caret
        let y = at(&n, 5);
        tap(&mut n, &ui, ox + 300.0, y, 4000.0);
        assert!(n.editing().unwrap().text.contains("- [ ] milk"));

        // with "move checked to the bottom" a tick sinks the item, one undo step
        n.run(&ui, Act::ToggleCheckedBottom, 5000.0);
        assert!(n.prefs().checked_to_bottom && store::load_prefs(&dir).checked_to_bottom);
        let y = at(&n, 5);
        tap(&mut n, &ui, ox + 2.0, y, 6000.0);
        assert_eq!(n.editing().unwrap().text, "Todo\n- [ ] eggs\n- [ ] bread\n- [x] milk");
        n.activate(&ui, Target::Undo, 0.0, 0.0, 6100.0);
        assert_eq!(n.editing().unwrap().text, "Todo\n- [ ] milk\n- [ ] eggs\n- [ ] bread");
        // the one-shot action from the menu is undoable too
        n.editor.replace_range(&ui, 8..9, "x", 7000.0, false);
        n.run(&ui, Act::SortCheckedNow, 7001.0);
        assert!(n.editing().unwrap().text.ends_with("- [x] milk"), "{}", n.editing().unwrap().text);
        n.activate(&ui, Target::Undo, 0.0, 0.0, 7100.0);
        assert!(n.editing().unwrap().text.starts_with("Todo\n- [x] milk"));
        draw(&mut n, &ui, 8000.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn formatting_bar_and_list_continuation_are_undoable() {
        let ui = ui();
        let dir = scratch("format");
        let mut n = app(&ui, &dir);
        n.new_note(&ui, 0.0);
        type_text(&mut n, &ui, "Notes\n", 10.0);
        let y = n.shortcut_top(&ui).unwrap() + ui.pt(SHORTCUT_PT) / 2.0;
        let slot = |k: f32| ui.pt(MARGIN_PT) + ui.pt(2.0 * UNDO_W_PT + FMT_W_PT * (k + 0.5));
        assert_eq!(n.hit(&ui, slot(0.0), y), Some(Target::Fmt(Fmt::Bold)));
        assert_eq!(n.hit(&ui, slot(1.0), y), Some(Target::Fmt(Fmt::Italic)));
        assert_eq!(n.hit(&ui, slot(3.0), y), Some(Target::Fmt(Fmt::Bullet)));
        assert_eq!(n.hit(&ui, slot(5.0), y), Some(Target::Fmt(Fmt::Check)));
        tap(&mut n, &ui, slot(3.0), y, 100.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- ");
        type_text(&mut n, &ui, "milk\neggs", 200.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs", "Enter continued the list");
        n.key(&ui, Out::Enter, 300.0);
        n.key(&ui, Out::Enter, 301.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs\n", "a second Enter ends the list");
        n.activate(&ui, Target::Undo, 0.0, 0.0, 400.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs\n- ");
        n.activate(&ui, Target::Undo, 0.0, 0.0, 401.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs");
        let len = n.editor.len();
        n.editor.select(len - 4..len);
        tap(&mut n, &ui, slot(0.0), y, 500.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- **eggs**");
        tap(&mut n, &ui, slot(0.0), y, 600.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- eggs", "bold toggles off again");
        n.activate(&ui, Target::Undo, 0.0, 0.0, 700.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n- milk\n- **eggs**");
        // numbered style over several lines
        n.editor.select(6..n.editor.len());
        tap(&mut n, &ui, slot(4.0), y, 800.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n1. milk\n2. **eggs**");
        n.run(&ui, Act::Style(md::Block::Heading), 900.0);
        n.activate(&ui, Target::Undo, 0.0, 0.0, 901.0);
        assert_eq!(n.editing().unwrap().text, "Notes\n1. milk\n2. **eggs**");
        draw(&mut n, &ui, 1000.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidebar_lists_folders_with_counts_and_switches_the_list() {
        let ui = ui();
        let dir = scratch("folders-sidebar");
        let mut n = seeded(&ui, &dir);
        let (nx, ny) = centre(chrome::px(&ui, n.bar_rect(&ui, chrome::B_NEW_FOLDER).unwrap()));
        assert_eq!(n.hit(&ui, nx, ny), Some(Target::FolderNew));
        tap(&mut n, &ui, nx, ny, 2.0);
        n.naming.as_mut().unwrap().field.set_text(&ui, "Work");
        n.key(&ui, Out::Enter, 3.0);
        assert_eq!(n.folder_names(), ["Work".to_string()]);
        assert_eq!(n.view(), &View::Folder("Work".into()), "a new folder opens");
        draw(&mut n, &ui, 4.0);
        let list = n.sidebar();
        let rows: Vec<_> = list.items.iter().filter_map(|i| match i {
            bifrost_app::controls::sidebar::SidebarItem::Row { label, count, .. } => Some((label.clone(), *count)),
            _ => None,
        }).collect();
        assert_eq!(rows.len(), 7, "All Notes, the folder, the bin, then the four smart folders");
        assert_eq!(rows[..3].to_vec(), vec![("All Notes".into(), Some(2)), ("Work".into(), Some(0)), ("Recently Deleted".into(), Some(0))]);
        assert_eq!(rows[3].0, "Pinned");
        // a click on All Notes in the sidebar shows every note again
        let bounds = bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: n.side_pt(&ui), h: 1500.0 / ui.pt(1.0) };
        let r = list.item_rect(1, bounds, -(bifrost_app::layout::UNIFIED_BAR_PT + 4.0));
        let (x, y) = (ui.pt(r.x + r.w / 2.0), ui.pt(r.y + r.h / 2.0));
        assert!(matches!(n.hit(&ui, x, y), Some(Target::Sidebar(_))));
        tap(&mut n, &ui, x, y, 5.0);
        assert_eq!(n.view(), &View::All);
        assert_eq!(n.listed().len(), 2);
        // the sidebar folds away and comes back
        n.activate(&ui, Target::SidebarToggle, 0.0, 0.0, 6.0);
        assert_eq!(n.side_pt(&ui), 0.0);
        assert!(n.hit(&ui, x, y) != Some(Target::Sidebar(100)));
        n.activate(&ui, Target::SidebarToggle, 0.0, 0.0, 7.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folders_can_be_created_renamed_filled_and_deleted() {
        let ui = ui();
        let dir = scratch("folders");
        let mut n = seeded(&ui, &dir);
        n.run(&ui, Act::NewFolder, 1.0);
        assert!(n.naming.is_some() && n.keyboard_shown());
        draw(&mut n, &ui, 2.0);
        assert_eq!(n.naming.as_ref().unwrap().field.text(), "New Folder");
        n.naming.as_mut().unwrap().field.set_text(&ui, "Work");
        n.key(&ui, Out::Enter, 3.0);
        assert!(n.naming.is_none() && !n.keyboard_shown());
        assert_eq!(n.view(), &View::Folder("Work".into()));
        assert_eq!(n.folder_names(), ["Work".to_string()]);
        assert!(n.listed().is_empty(), "a new folder is empty");
        assert!(n.describe().contains("notes_view=folder:Work"));

        // move a note into it from the editor menu
        n.run(&ui, Act::View(View::All), 4.0);
        let i = n.notes.iter().position(|x| x.id == "1").unwrap();
        n.open_note(&ui, i, false, 5.0);
        n.open_editor_menu(&ui);
        assert!(n.menu.as_ref().unwrap().items.iter().any(|(label, ..)| label == "Move to Work"));
        n.run(&ui, Act::MoveTo("Work".into()), 6.0);
        assert!(dir.join("Work/1.md").exists() && !dir.join("1.md").exists());
        assert_eq!(n.folder_of("1"), Some("Work"));
        n.close_note(&ui, 7.0);
        assert_eq!(n.listed().len(), 2, "All Notes still shows it");
        n.run(&ui, Act::View(View::Folder("Work".into())), 8.0);
        assert_eq!(n.listed(), vec![("1", "Groceries")]);
        // searching looks through every folder
        n.search.set_text(&ui, "rust");
        n.rebuild(&ui, 9.0);
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        n.search.set_text(&ui, "");
        n.rebuild(&ui, 9.5);
        // editing inside a folder saves into it
        n.open_note(&ui, n.shown[0], true, 10.0);
        type_text(&mut n, &ui, "!", 11.0);
        n.tick(&ui, 1000.0);
        assert!(std::fs::read_to_string(dir.join("Work/1.md")).unwrap().ends_with('!'));
        assert!(!dir.join("1.md").exists());
        n.close_note(&ui, 1100.0);
        // a new note composed inside the folder lands there
        n.new_note(&ui, 1200.0);
        let new_id = n.editing().unwrap().id.clone();
        type_text(&mut n, &ui, "Fresh", 1300.0);
        n.close_note(&ui, 1400.0);
        assert!(dir.join("Work").join(format!("{new_id}.md")).exists());

        // rename keeps the notes
        n.run(&ui, Act::RenameFolder, 2000.0);
        assert_eq!(n.naming.as_ref().unwrap().field.text(), "Work");
        n.naming.as_mut().unwrap().field.set_text(&ui, "Projects");
        n.key(&ui, Out::Enter, 2001.0);
        assert_eq!(n.view(), &View::Folder("Projects".into()));
        assert!(dir.join("Projects/1.md").exists() && !dir.join("Work").exists());
        assert_eq!(n.listed().len(), 2);
        // a clashing or empty name leaves everything alone and reports it
        n.run(&ui, Act::NewFolder, 2100.0);
        n.naming.as_mut().unwrap().field.set_text(&ui, "   ");
        n.key(&ui, Out::Enter, 2101.0);
        assert!(n.notice.is_some());
        draw(&mut n, &ui, 2200.0);

        // deleting the folder moves its notes to Recently Deleted
        n.view = View::Folder("Projects".into());
        n.run(&ui, Act::DeleteFolder, 3000.0);
        assert_eq!(n.view(), &View::All);
        assert!(n.folder_names().is_empty());
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        n.run(&ui, Act::View(View::Trash), 3100.0);
        assert_eq!(n.listed().len(), 2);
        draw(&mut n, &ui, 3200.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_notes_can_be_recovered_erased_and_expire() {
        let ui = ui();
        let dir = scratch("trash");
        std::fs::create_dir_all(&dir).unwrap();
        store::save(&dir, &Note { id: "9".into(), text: "Ancient".into(), modified: 0 }).unwrap();
        folders::trash(&dir, "9", None, wall().unix - 31 * 86_400).unwrap();
        let mut n = seeded(&ui, &dir);
        assert!(!n.notes.iter().any(|x| x.id == "9"), "notes deleted more than 30 days ago are purged on launch");
        assert!(!dir.join(folders::TRASH).join("9.md").exists());
        n.delete_row(&ui, 0, 100.0);
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        assert!(dir.join(folders::TRASH).join("1.md").exists());
        n.run(&ui, Act::View(View::Trash), 200.0);
        assert_eq!(n.listed(), vec![("1", "Groceries")]);
        n.open_note(&ui, n.shown[0], false, 300.0);
        n.open_editor_menu(&ui);
        let labels: Vec<_> = n.menu.as_ref().unwrap().items.iter().map(|(l, ..)| l.as_str()).collect();
        assert!(labels.contains(&"Recover Note") && labels.contains(&"Delete Permanently") && !labels.contains(&"Delete Note"));
        n.run(&ui, Act::Recover, 400.0);
        assert_eq!(n.editing(), None);
        assert!(dir.join("1.md").exists());
        assert!(n.listed().is_empty(), "the bin view no longer lists it");
        n.run(&ui, Act::View(View::All), 500.0);
        assert_eq!(n.listed().len(), 2);
        // delete from the editor, then erase for good
        n.open_note(&ui, n.shown[0], false, 600.0);
        let id = n.editing().unwrap().id.clone();
        n.run(&ui, Act::DeleteNote, 700.0);
        assert_eq!(n.editing(), None);
        assert_eq!(n.folder_of(&id), Some(folders::TRASH));
        n.run(&ui, Act::View(View::Trash), 800.0);
        n.delete_row(&ui, 0, 900.0);
        assert!(n.listed().is_empty());
        assert!(!dir.join(folders::TRASH).join(format!("{id}.md")).exists());
        assert!(draw(&mut n, &ui, 1000.0).contains(&LABEL_2.now()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sort_options_and_pinned_section_persist() {
        let ui = ui();
        let dir = scratch("sort");
        let mut n = seeded(&ui, &dir);
        n.notes.push(Note { id: "3".into(), text: "Apricot".into(), modified: wall().unix - 86_400 * 9 });
        n.rebuild(&ui, 0.0);
        let ids = |n: &Notes| n.listed().iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&n), ["1", "2", "3"]);
        n.run(&ui, Act::Sort(store::SortKey::Title), 1.0);
        assert_eq!(ids(&n), ["3", "1", "2"]);
        assert!(n.set_pinned(&ui, "2", true, 2.0));
        assert_eq!(ids(&n), ["2", "3", "1"], "pinned first, then by title");
        n.run(&ui, Act::Sort(store::SortKey::DateCreated), 3.0);
        assert_eq!(ids(&n), ["2", "1", "3"]);
        n.open_list_menu(&ui);
        let menu = n.menu.as_ref().unwrap();
        assert!(menu.items.iter().any(|(l, a, on)| l == "Sort by Date Created" && *a == Act::Sort(store::SortKey::DateCreated) && *on));
        draw(&mut n, &ui, 4.0);
        assert_eq!(app(&ui, &dir).prefs().sort, store::SortKey::DateCreated, "the choice survives a restart");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn menus_are_modal_and_tapping_a_row_runs_it() {
        let ui = ui();
        let dir = scratch("menu");
        let mut n = seeded(&ui, &dir);
        let (tx, ty) = centre(n.title_rect(&ui));
        assert_eq!(n.hit(&ui, tx, ty), Some(Target::Title));
        tap(&mut n, &ui, tx, ty, 100.0);
        assert!(n.menu.is_some());
        let (mx, my, mw, _) = n.menu_rect(&ui, n.menu.as_ref().unwrap());
        // a tap outside only closes it
        tap(&mut n, &ui, 5.0, 1800.0, 200.0);
        assert!(n.menu.is_none() && n.editing().is_none());
        tap(&mut n, &ui, tx, ty, 300.0);
        let rows = n.menu.as_ref().unwrap().items.len();
        assert!(rows >= 7);
        let sort_title = n.menu.as_ref().unwrap().items.iter().position(|(_, a, _)| *a == Act::Sort(store::SortKey::Title)).unwrap();
        let y = my + ui.pt(MENU_PAD_PT) + ui.pt(MENU_ROW_PT) * (sort_title as f32 + 0.5);
        tap(&mut n, &ui, mx + mw / 2.0, y, 400.0);
        assert!(n.menu.is_none());
        assert_eq!(n.prefs().sort, store::SortKey::Title);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn search_highlights_matches_and_steps_through_them() {
        let ui = ui();
        let dir = scratch("find");
        let mut n = seeded(&ui, &dir);
        for note in &mut n.notes {
            if note.id == "2" {
                note.text = "Ideas\nrust shell\nmore Rust here".into();
            }
        }
        n.search.set_text(&ui, "RUST");
        n.rebuild(&ui, 0.0);
        assert_eq!(n.listed(), vec![("2", "Ideas")]);
        let (rx, ry) = centre(n.row_rect(&ui, 0));
        tap(&mut n, &ui, rx, ry, 100.0);
        assert_eq!(n.match_state(), (1, 2));
        assert!(n.find_strip());
        draw(&mut n, &ui, 200.0);
        let y = n.find_top(&ui) + ui.pt(SHORTCUT_PT) / 2.0;
        let ex = n.ed_px(&ui).0;
        assert_eq!(n.hit(&ui, ex + ui.pt(MARGIN_PT + 20.0), y), Some(Target::FindPrev));
        assert_eq!(n.hit(&ui, ex + ui.pt(MARGIN_PT + 100.0), y), Some(Target::FindNext));
        tap(&mut n, &ui, ex + ui.pt(MARGIN_PT + 100.0), y, 300.0);
        assert_eq!(n.match_state(), (2, 2));
        tap(&mut n, &ui, ex + ui.pt(MARGIN_PT + 100.0), y, 400.0);
        assert_eq!(n.match_state(), (1, 2), "wraps around");
        tap(&mut n, &ui, ex + ui.pt(MARGIN_PT + 20.0), y, 500.0);
        assert_eq!(n.match_state(), (2, 2));
        // typing keeps the matches current; the keyboard replaces the find bar
        n.activate(&ui, Target::Text, 5.0, ui.pt(nav_bottom_pt() + 10.0), 600.0);
        assert!(n.keyboard_shown() && !n.find_strip());
        n.editor.select(0..0);
        type_text(&mut n, &ui, "rust ", 700.0);
        assert_eq!(n.match_state().1, 3);
        n.activate(&ui, Target::Back, 0.0, 0.0, 800.0);
        assert_eq!(n.match_state(), (0, 0), "leaving the note clears the highlights");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn whole_word_find_and_check_spelling_step_through_their_matches() {
        let ui = ui();
        let dir = scratch("parity");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "rust rusty rust\nsee https://example.com on 2026-10-09");
        n.hl_query = "rust".into();
        n.refresh_highlights(&ui, false);
        assert_eq!(n.match_state().1, 3, "rusty counts as a hit");
        n.run(&ui, Act::ToggleWholeWord, 100.0);
        assert!(n.whole_word);
        assert_eq!(n.match_state().1, 2, "whole word skips rusty");
        n.open_editor_menu(&ui);
        let labels: Vec<_> = n.menu.as_ref().unwrap().items.iter().map(|(l, ..)| l.as_str()).collect();
        assert!(labels.contains(&"Check Spelling and Links") && labels.contains(&"Match Whole Word in Find"));
        n.menu = None;
        n.run(&ui, Act::CheckSpelling, 200.0);
        let text = n.editor.text().to_string();
        let flagged: Vec<String> = n.hl.iter().map(|r| text.chars().skip(r.start).take(r.end - r.start).collect()).collect();
        assert!(flagged.iter().any(|s| s == "https://example.com"), "{flagged:?}");
        assert!(flagged.iter().any(|s| s == "2026-10-09"), "{flagged:?}");
        assert!(n.match_state().0 == 1, "the first flagged item is selected");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn proofread_lists_the_suggestions_and_accepting_edits_the_note() {
        let ui = ui();
        let dir = scratch("proofread");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Say it it again, red,green");
        n.open_editor_menu(&ui);
        assert!(n.menu.as_ref().unwrap().items.iter().any(|(l, ..)| l == "Proofread"));
        n.menu = None;
        let rows = |n: &Notes| n.menu.as_ref().unwrap().items.iter().map(|(l, ..)| l.clone()).collect::<Vec<_>>();
        n.run(&ui, Act::Proofread, 100.0);
        assert_eq!(rows(&n), ["Remove \"it\"", "Add a space", "Accept All"]);
        n.run(&ui, Act::ProofreadAccept(0), 200.0);
        assert_eq!(n.editor.text(), "Say it again, red,green");
        assert_eq!(rows(&n), ["Add a space"], "the menu comes back with what is left");
        n.run(&ui, Act::ProofreadAccept(0), 300.0);
        assert_eq!(n.editor.text(), "Say it again, red, green");
        assert!(n.proofread.is_none());
        assert_eq!(n.notes[0].text, "Say it again, red, green", "the note itself changed");
        // Accept All applies the rest in one go
        n.menu = None;
        let all = n.editor.text().chars().count();
        n.editor.replace_range(&ui, 0..all, "the the  cat", 400.0, false);
        n.run(&ui, Act::Proofread, 500.0);
        n.run(&ui, Act::ProofreadAll, 600.0);
        assert_eq!(n.editor.text(), "the cat");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notes_export_as_markdown_or_plain_text() {
        let ui = ui();
        let dir = scratch("export");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "# My List\n- [x] **milk**\n- eggs");
        n.run(&ui, Act::ShareMd, 2000.0);
        let Some(springboard::shell::share::Item::File(path)) = n.take_share() else { panic!("a file share") };
        assert_eq!(path.file_name().unwrap(), "My List.md");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# My List\n- [x] **milk**\n- eggs");
        n.run(&ui, Act::ShareText, 2100.0);
        let item = n.take_share().expect("a text share");
        assert_eq!(item.file_name(), "My List.txt");
        assert_eq!(item.clip_text(), "My List\n\nMy List\n[x] milk\n\u{2022} eggs");
        n.run(&ui, Act::SharePdf, 2200.0);
        let Some(springboard::shell::share::Item::File(pdf_path)) = n.take_share() else { panic!("a PDF share") };
        assert_eq!(pdf_path.file_name().unwrap(), "My List.pdf");
        let bytes = std::fs::read(&pdf_path).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&bytes).contains("([x] milk) Tj"), "checklist lines keep their plain text");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_table_puts_a_pipe_table_on_its_own_paragraph() {
        let ui = ui();
        let dir = scratch("table");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Before");
        n.run(&ui, Act::InsertTable, 1900.0);
        let text = n.editor.text().to_string();
        assert!(text.starts_with("Before\n| Column | Column |\n| --- | --- |\n|  |  |\n"), "{text:?}");
        let pdf = pdf::render("Before", &md::to_plain(&text));
        assert!(String::from_utf8_lossy(&pdf).contains("Column   |   Column"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn images_attach_beside_the_note_and_show_inline() {
        let ui = ui();
        let dir = scratch("attach");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Trip\nnotes");
        let before = n.editor.height();
        n.attach_image(&ui, &tiny_png(40, 80), 2000.0).unwrap();
        settle_images(&mut n, &ui);
        let id = n.editing().unwrap().id.clone();
        assert!(dir.join(format!("{id}-1.png")).exists());
        assert_eq!(n.editing().unwrap().text, format!("Trip\nnotes\n![image]({id}-1.png)\n"));
        assert!(n.editor.height() > before + 40.0, "the picture takes room in the text");
        assert!(n.attach_image(&ui, b"junk", 2100.0).is_err());
        draw(&mut n, &ui, 2200.0);
        n.activate(&ui, Target::Undo, 0.0, 0.0, 2300.0);
        assert_eq!(n.editing().unwrap().text, "Trip\nnotes");
        // moving the note brings the picture along
        n.attach_image(&ui, &tiny_png(8, 8), 2400.0).unwrap();
        settle_images(&mut n, &ui);
        n.tick(&ui, 4000.0);
        folders::create(&dir, "Pics").unwrap();
        n.folders = folders::list(&dir);
        n.run(&ui, Act::MoveTo("Pics".into()), 4100.0);
        assert!(dir.join("Pics").join(format!("{id}.md")).exists());
        assert!(dir.join("Pics").join(format!("{id}-1.png")).exists() && dir.join("Pics").join(format!("{id}-2.png")).exists());
        assert!(!dir.join(format!("{id}-1.png")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn insert_sketch_saves_a_picture_and_its_drawing_beside_the_note() {
        let ui = ui();
        let dir = scratch("sketch");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Plan");
        let id = n.editing().unwrap().id.clone();
        let (w, head) = (n.w as f32, ui.pt(SKETCH_HEAD_PT));
        // Cancel (left zone of the header) closes the pad and saves nothing
        n.run(&ui, Act::InsertSketch, 1900.0);
        assert!(n.sketch.is_some(), "the pad opens over an open note");
        n.touch(&ui, Event::Down(5.0, 10.0), 1910.0);
        n.touch(&ui, Event::Up(5.0, 10.0), 1920.0);
        assert!(n.sketch.is_none());
        assert_eq!(n.editing().unwrap().text, "Plan");
        // a stroke, then Done (right zone of the header) saves it and inserts its picture at the caret
        n.run(&ui, Act::InsertSketch, 2000.0);
        n.touch(&ui, Event::Down(100.0, head + 40.0), 2100.0);
        n.touch(&ui, Event::Move(160.0, head + 60.0), 2110.0);
        n.touch(&ui, Event::Up(160.0, head + 60.0), 2120.0);
        draw(&mut n, &ui, 2150.0);
        n.touch(&ui, Event::Down(w - 5.0, 10.0), 2200.0);
        n.touch(&ui, Event::Up(w - 5.0, 10.0), 2210.0);
        assert!(n.sketch.is_none());
        settle_images(&mut n, &ui);
        assert_eq!(n.editing().unwrap().text, format!("Plan\n![image]({id}-1.png)\n"));
        assert!(dir.join(format!("{id}-1.png")).exists() && dir.join(format!("{id}-1.sketch")).exists());
        // an empty pad closes without inserting anything
        n.run(&ui, Act::InsertSketch, 2300.0);
        n.touch(&ui, Event::Down(w - 5.0, 10.0), 2310.0);
        n.touch(&ui, Event::Up(w - 5.0, 10.0), 2320.0);
        assert!(n.sketch.is_none());
        assert_eq!(n.editing().unwrap().text, format!("Plan\n![image]({id}-1.png)\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// The text of the open note with the caret put after the first occurrence of `after`.
    fn caret_after(n: &mut Notes, after: &str) {
        let text = n.editor.text().to_string();
        let at = text[..text.find(after).unwrap() + after.len()].chars().count();
        n.editor.set_caret(at);
    }

    #[test]
    fn dates_sort_oldest_first_on_request_and_the_choice_persists() {
        let ui = ui();
        let dir = scratch("sort-order");
        let mut n = seeded(&ui, &dir);
        let ids = |n: &Notes| n.listed().iter().map(|(id, _)| id.to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&n), ["1", "2"], "newest first by default");
        n.open_list_menu(&ui);
        let items = n.menu.as_ref().unwrap().items.clone();
        assert!(items.iter().any(|(l, a, on)| l == "Newest First" && *a == Act::SortOrder(false) && *on));
        assert!(items.iter().any(|(l, a, on)| l == "Oldest First" && *a == Act::SortOrder(true) && !*on));
        n.menu = None;
        n.run(&ui, Act::SortOrder(true), 1.0);
        assert_eq!(ids(&n), ["2", "1"]);
        assert!(app(&ui, &dir).prefs().oldest_first, "the direction survives a restart");
        n.run(&ui, Act::SortOrder(false), 2.0);
        assert_eq!(ids(&n), ["1", "2"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_all_and_unmark_all_flip_the_whole_checklist_in_one_undo_step() {
        let ui = ui();
        let dir = scratch("check-all");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Plan\n- [ ] a\n- [x] b\n- [ ] c");
        assert!(n.has_checklist());
        n.run(&ui, Act::Do(Format::CheckAll), 2000.0);
        assert_eq!(n.editor.text(), "Plan\n- [x] a\n- [x] b\n- [x] c");
        n.run(&ui, Act::Do(Format::UncheckAll), 2100.0);
        assert_eq!(n.editor.text(), "Plan\n- [ ] a\n- [ ] b\n- [ ] c");
        assert!(n.editor.undo(&ui, 2200.0));
        assert_eq!(n.editor.text(), "Plan\n- [x] a\n- [x] b\n- [x] c", "one undo step per command");
        n.open_format_menu(&ui);
        assert!(n.menu.as_ref().unwrap().items.iter().any(|(l, _, _)| l == "Mark All"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tags_are_listed_counted_and_filter_the_list() {
        let ui = ui();
        let dir = scratch("tags");
        let mut n = seeded(&ui, &dir);
        let now = wall().unix;
        n.notes.push(Note { id: "3".into(), text: "Chores\n#home #errands".into(), modified: now - 30 });
        n.notes.push(Note { id: "4".into(), text: "Garden\nplant #Home".into(), modified: now - 40 });
        n.rebuild(&ui, 0.0);
        assert_eq!(n.tag_list, vec![("errands".to_string(), 1), ("home".to_string(), 2)]);
        assert_eq!(n.count_in(&View::Tag("home".into())), 2);
        assert_eq!(listed_ids(&mut n, &ui, View::Tag("home".into())), ["3", "4"]);
        assert_eq!(n.view_title(), "#home");
        assert!(listed_ids(&mut n, &ui, View::Tag("nothing".into())).is_empty());
        n.view = View::All;
        n.rebuild(&ui, 11.0);
        let rows: Vec<_> = n.sidebar().items.iter().filter_map(|i| match i {
            bifrost_app::controls::sidebar::SidebarItem::Row { label, count, .. } => Some((label.clone(), *count)),
            _ => None,
        }).collect();
        assert!(rows.contains(&("#home".to_string(), Some(2))) && rows.contains(&("#errands".to_string(), Some(1))), "{rows:?}");
        n.open_list_menu(&ui);
        assert!(n.menu.as_ref().unwrap().items.iter().any(|(l, a, _)| l == "#home\t2" && *a == Act::View(View::Tag("home".into()))));
        // a note in Recently Deleted does not count
        n.loc.insert("3".into(), folders::TRASH.into());
        n.rebuild(&ui, 12.0);
        assert_eq!(n.tag_list, vec![("home".to_string(), 1)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tab_and_shift_tab_indent_list_lines_and_ctrl_cmd_arrows_move_them() {
        use bifrost_app::proto::{CHORD_CTRL, CHORD_META, CHORD_SHIFT};
        let ui = ui();
        let dir = scratch("indent");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Plan\n- a\n- b");
        n.key(&ui, Out::Named("Tab"), 2000.0);
        assert_eq!(n.editor.text(), "Plan\n- a\n  - b");
        assert_eq!(n.editor.caret(), "Plan\n- a\n  - b".chars().count(), "the caret stays on its line");
        assert!(n.chord(&ui, 15, CHORD_SHIFT, 2100.0), "Shift+Tab outdents");
        assert_eq!(n.editor.text(), "Plan\n- a\n- b");
        n.key(&ui, Out::Named("Tab"), 2200.0);
        n.editor.set_caret(0);
        n.key(&ui, Out::Named("Tab"), 2300.0);
        assert_eq!(n.editor.text(), "Plan\n- a\n  - b", "Tab on the title line (not a list) does nothing");
        caret_after(&mut n, "  - b");
        n.chord(&ui, 15, CHORD_SHIFT, 2400.0);
        assert_eq!(n.editor.text(), "Plan\n- a\n- b");
        // moving: "b" goes above "a"
        caret_after(&mut n, "- b");
        assert!(n.chord(&ui, 103, CHORD_CTRL | CHORD_META, 2500.0));
        assert_eq!(n.editor.text(), "Plan\n- b\n- a");
        assert_eq!(n.editor.caret(), "Plan\n- b".chars().count(), "the caret follows the item");
        assert!(n.chord(&ui, 108, CHORD_CTRL | CHORD_META, 2600.0));
        assert_eq!(n.editor.text(), "Plan\n- a\n- b");
        assert_eq!(n.notes[match n.screen { Screen::Editor(i) => i, Screen::List => panic!("closed") }].text, "Plan\n- a\n- b", "the note follows the editor");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_format_menu_edits_the_table_under_the_caret() {
        let ui = ui();
        let dir = scratch("table-menu");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "T\n| A | B |\n| --- | --- |\n| 1 | 2 |");
        caret_after(&mut n, "| 1 | 2");
        n.open_format_menu(&ui);
        let labels: Vec<String> = n.menu.as_ref().unwrap().items.iter().map(|(l, _, _)| l.clone()).collect();
        assert!(labels.contains(&"Add Row Below".to_string()) && labels.contains(&"Delete Column".to_string()), "{labels:?}");
        n.menu = None;
        n.run(&ui, Act::Do(Format::Table(table::Op::RowBelow)), 2000.0);
        assert_eq!(n.editor.text(), "T\n| A | B |\n| --- | --- |\n| 1 | 2 |\n|  |  |");
        caret_after(&mut n, "| 1 | 2");
        n.run(&ui, Act::Do(Format::Table(table::Op::DeleteColumn)), 2100.0);
        assert_eq!(n.editor.text(), "T\n| A |\n| --- |\n| 1 |\n|  |");
        n.run(&ui, Act::Do(Format::Table(table::Op::ToText)), 2200.0);
        assert_eq!(n.editor.text(), "T\nA\n1\n");
        n.open_format_menu(&ui);
        assert!(!n.menu.as_ref().unwrap().items.iter().any(|(l, _, _)| l == "Add Row Below"), "no table commands outside a table");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paragraph_and_character_styles_add_their_marks() {
        let ui = ui();
        let dir = scratch("styles");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Title\nplain line");
        caret_after(&mut n, "plain");
        n.run(&ui, Act::Style(md::Block::Quote), 2000.0);
        assert_eq!(n.editor.text(), "Title\n> plain line");
        n.run(&ui, Act::Style(md::Block::Quote), 2100.0);
        assert_eq!(n.editor.text(), "Title\nplain line", "choosing the style again returns to body text");
        n.run(&ui, Act::Style(md::Block::Mono), 2200.0);
        assert_eq!(n.editor.text(), "Title\n    plain line");
        n.run(&ui, Act::Style(md::Block::Dashed), 2300.0);
        assert_eq!(n.editor.text(), "Title\n\u{2013} plain line");
        n.editor.select("Title\n\u{2013} ".chars().count().."Title\n\u{2013} plain".chars().count());
        n.run(&ui, Act::Do(Format::Char(Fmt::Strike)), 2400.0);
        assert_eq!(n.editor.text(), "Title\n\u{2013} ~~plain~~ line");
        n.run(&ui, Act::Do(Format::Char(Fmt::Strike)), 2500.0);
        assert_eq!(n.editor.text(), "Title\n\u{2013} plain line", "the same command takes the marks off");
        n.editor.select("Title\n\u{2013} ".chars().count().."Title\n\u{2013} plain".chars().count());
        n.run(&ui, Act::Do(Format::Char(Fmt::Underline)), 2600.0);
        n.run(&ui, Act::Do(Format::Char(Fmt::Highlight)), 2700.0);
        assert!(n.editor.text().contains("==++plain++==") || n.editor.text().contains("++==plain==++"), "{:?}", n.editor.text());
        n.open_format_menu(&ui);
        let checked: Vec<String> = n.menu.as_ref().unwrap().items.iter().filter(|(_, _, on)| *on).map(|(l, _, _)| l.clone()).collect();
        assert_eq!(checked, vec!["Dashed List".to_string()], "the caret's paragraph style is checked");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn math_results_answer_a_calculation_when_equals_is_typed_or_on_request() {
        let ui = ui();
        let dir = scratch("math");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Sum\n2+2");
        n.editor.set_caret("Sum\n2+2".chars().count());
        n.key(&ui, Out::Text("=".into()), 2000.0);
        assert_eq!(n.editor.text(), "Sum\n2+2=", "Math Results is off until it is turned on");
        n.run(&ui, Act::ToggleMath, 2100.0);
        assert!(app(&ui, &dir).prefs().math_results, "the setting persists");
        n.editor.set_text(&ui, "Sum\n6*7");
        n.editor.set_caret("Sum\n6*7".chars().count());
        n.key(&ui, Out::Text("=".into()), 2200.0);
        assert_eq!(n.editor.text(), "Sum\n6*7= 42");
        assert_eq!(n.notes[match n.screen { Screen::Editor(i) => i, Screen::List => panic!("closed") }].text, "Sum\n6*7= 42");
        // typing = after plain text asks nothing
        n.editor.set_text(&ui, "Sum\nhello");
        n.editor.set_caret("Sum\nhello".chars().count());
        n.key(&ui, Out::Text("=".into()), 2300.0);
        assert_eq!(n.editor.text(), "Sum\nhello=");
        // Calculate Math Results answers every line at once
        n.editor.set_text(&ui, "Sum\n1+1=\nskip\n(2+3)*4=");
        n.run(&ui, Act::Do(Format::CalculateMath), 2400.0);
        assert_eq!(n.editor.text(), "Sum\n1+1= 2\nskip\n(2+3)*4= 20");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paste_variants_keep_or_strip_markdown_and_copy_as_markdown_reads_the_selection() {
        let ui = ui();
        let dir = scratch("paste");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Note\n");
        springboard::shell::clipboard::set_text("**bold** and ~~gone~~\n- item");
        assert!(n.paste_markdown(&ui, 2000.0));
        assert_eq!(n.editor.text(), "Note\n**bold** and ~~gone~~\n- item");
        n.editor.set_text(&ui, "Note\n");
        n.editor.set_caret(5);
        assert!(n.paste_match_style(&ui, 2100.0));
        assert_eq!(n.editor.text(), "Note\nbold and gone\n\u{2022} item", "the marks are stripped");
        n.editor.select(5..9);
        assert_eq!(n.selected_markdown().as_deref(), Some("bold"));
        n.close_note(&ui, 2200.0);
        assert!(!n.paste_markdown(&ui, 2300.0), "no note open, nothing to paste into");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_notes_open_by_title_and_folders_open_by_name() {
        let ui = ui();
        let dir = scratch("recent");
        let mut n = seeded(&ui, &dir);
        assert_eq!(n.recent_notes(), vec![("1".to_string(), "Groceries".to_string()), ("2".to_string(), "Ideas".to_string())]);
        assert_eq!(n.open_named(&ui, "ideas", 1.0).as_deref(), Some("Ideas"));
        assert_eq!(n.editing().unwrap().id, "2");
        assert_eq!(n.open_named(&ui, "groc", 2.0).as_deref(), Some("Groceries"), "a part of the title is enough");
        assert_eq!(n.open_named(&ui, "1", 3.0).as_deref(), Some("Groceries"), "the id works too");
        assert_eq!(n.open_named(&ui, "nothing like it", 4.0), None);
        assert_eq!(n.open_named(&ui, "  ", 5.0), None);
        n.close_note(&ui, 6.0);
        folders::create(&dir, "Work").unwrap();
        n.reload(&ui, 7.0);
        assert!(!n.open_folder(&ui, "Play", 8.0));
        assert!(n.open_folder(&ui, "work", 9.0));
        assert_eq!(n.view(), &View::Folder("Work".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_quick_note_is_remembered_and_can_be_resumed() {
        let ui = ui();
        let dir = scratch("quick");
        let mut n = app(&ui, &dir);
        n.quick_note(&ui, true, 1.0);
        assert!(n.editing().is_none(), "nothing to resume yet, and resuming never starts a note");
        n.quick_note(&ui, false, 2.0);
        let id = n.editing().unwrap().id.clone();
        assert_eq!(store::load_quick(&dir), Some(id.clone()));
        type_text(&mut n, &ui, "Idea\nsomething", 3.0);
        n.close_note(&ui, 4.0);
        assert!(n.editing().is_none());
        n.quick_note(&ui, true, 5.0);
        assert_eq!(n.editing().unwrap().id, id, "Resume Last Quick Note reopens it");
        n.close_note(&ui, 6.0);
        // by default New Quick Note starts another note; with the setting on it resumes
        n.quick_note(&ui, false, 7.0);
        let second = n.editing().unwrap().id.clone();
        assert_ne!(second, id);
        type_text(&mut n, &ui, "Second\nnote", 7.5);
        n.close_note(&ui, 8.0);
        n.prefs.resume_quick = true;
        let before = n.notes.len();
        n.quick_note(&ui, false, 9.0);
        assert_eq!(n.notes.len(), before, "no new note while the last quick note exists");
        assert_eq!(n.editing().unwrap().id, second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn group_by_date_cuts_the_list_into_sections_and_keeps_rows_clear_of_the_headers() {
        let ui = ui();
        let dir = scratch("group-date");
        let mut n = seeded(&ui, &dir);
        let now = wall().unix;
        n.notes.push(Note { id: "3".into(), text: "Old plan".into(), modified: now - 86_400 * 20 });
        n.notes.push(Note { id: "4".into(), text: "Ancient".into(), modified: now - 86_400 * 200 });
        n.rebuild(&ui, 0.0);
        assert!(n.sections.is_empty(), "no sections until Group by Date is on");
        let flat = n.row_rect(&ui, 0).1;
        n.run(&ui, Act::ToggleGroupDate, 1.0);
        let labels: Vec<_> = n.sections.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["Today", "Previous 7 Days", "Previous 30 Days", "March"], "{labels:?}");
        assert_eq!(n.sections.iter().map(|(_, i)| *i).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(n.section_offset(0), SECTION_PT);
        assert_eq!(n.section_offset(3), SECTION_PT + 3.0 * (SECTION_PT + SECTION_GAP_PT));
        assert!(n.row_rect(&ui, 0).1 > flat, "the first row sits under its header");
        let step = n.row_rect(&ui, 1).1 - n.row_rect(&ui, 0).1;
        assert!((step - ui.pt(ROW_PT + SECTION_PT + SECTION_GAP_PT)).abs() < 0.01, "a row, then the next header");
        draw(&mut n, &ui, 2.0);
        // a pinned note gets its own section above the dates; a search or the title sort drops the date sections
        assert!(n.set_pinned(&ui, "2", true, 3.0));
        assert_eq!(n.sections.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Pinned", "Today", "Previous 30 Days", "March"]);
        n.search.set_text(&ui, "plan");
        n.rebuild(&ui, 4.0);
        assert!(n.sections.is_empty(), "a search ranks by relevance, not by date");
        n.search.set_text(&ui, "");
        n.run(&ui, Act::Sort(store::SortKey::Title), 5.0);
        assert_eq!(n.sections.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), ["Pinned", "Notes"]);
        assert!(app(&ui, &dir).prefs().group_by_date, "the setting persists");
        n.run(&ui, Act::ToggleGroupDate, 6.0);
        assert!(!n.prefs.group_by_date);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_link_asks_for_an_address_and_wraps_the_selection() {
        use bifrost_app::proto::CHORD_META;
        let ui = ui();
        let dir = scratch("link");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "Note\nSee docs here");
        let start = "Note\nSee ".chars().count();
        n.editor.select(start..start + 4);
        assert!(n.chord(&ui, 37, CHORD_META, 2000.0), "Cmd+K");
        assert_eq!(n.naming.as_ref().unwrap().field.text(), "https://");
        n.naming.as_mut().unwrap().field.set_text(&ui, "example.com/a b");
        n.key(&ui, Out::Enter, 2100.0);
        assert!(n.naming.is_none());
        assert_eq!(n.editor.text(), "Note\nSee [docs](https://example.com/a%20b) here");
        assert_eq!(n.editing().unwrap().text, n.editor.text(), "the note follows the editor");
        // an address with nothing after the scheme is refused and the text stays
        n.run(&ui, Act::Do(Format::AddLink), 2200.0);
        n.key(&ui, Out::Enter, 2300.0);
        assert_eq!(n.notice.as_deref(), Some("Enter a web address for the link"));
        assert_eq!(n.editor.text(), "Note\nSee [docs](https://example.com/a%20b) here");
        // Escape cancels the prompt
        n.run(&ui, Act::Do(Format::AddLink), 2400.0);
        assert!(n.escape(&ui, 2500.0));
        assert!(n.naming.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn collapse_commands_fold_the_section_under_the_caret_and_survive_a_restart() {
        let ui = ui();
        let dir = scratch("fold-section");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "# Plan\nintro\n## Steps\none\ntwo\n## Notes\nlast");
        let text = n.editor.text().to_string();
        let id = n.editing().unwrap().id.clone();
        caret_after(&mut n, "one");
        n.run(&ui, Act::Do(Format::CollapseSection), 3000.0);
        assert!(n.folds[&id].contains("## Steps"));
        assert_eq!(n.editor.text(), text, "folding never changes the note text");
        n.run(&ui, Act::Do(Format::CollapseAll), 3100.0);
        assert_eq!(n.folds[&id].len(), 3, "Plan, Steps and Notes are folded");
        n.run(&ui, Act::Do(Format::ExpandSection), 3200.0);
        assert!(!n.folds[&id].contains("## Steps"), "Expand Section unfolds the heading above the caret");
        let reopened = app(&ui, &dir);
        assert_eq!(reopened.folds[&id].len(), 2, "the folds are saved with the note");
        n.run(&ui, Act::Do(Format::ExpandAll), 3300.0);
        assert!(n.folds.get(&id).is_none_or(|keys| keys.is_empty()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn view_menu_layout_choice_is_saved_with_the_prefs() {
        let ui = ui();
        let dir = scratch("layout");
        let mut n = seeded(&ui, &dir);
        assert_eq!(n.prefs().layout, gallery::Layout::List);
        n.set_layout(&ui, gallery::Layout::Gallery);
        assert_eq!(n.prefs().layout, gallery::Layout::Gallery);
        assert_eq!(app(&ui, &dir).prefs().layout, gallery::Layout::Gallery);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folded_section_hides_its_lines_and_its_chevron_unfolds_it() {
        let ui = ui();
        let dir = scratch("fold-hide");
        let mut n = app(&ui, &dir);
        open_with(&mut n, &ui, "# Plan\nintro\n## Steps\none\ntwo");
        caret_after(&mut n, "one");
        n.run(&ui, Act::Do(Format::CollapseSection), 3000.0);
        let one = n.editor.text().find("one").unwrap();
        assert_eq!(n.editor.line_span(one).1, 0.0, "the folded line takes no height");
        assert!(n.fold_chevrons(&ui).iter().any(|(line, _, folded)| *line == 2 && *folded), "the heading shows a folded chevron");
        draw(&mut n, &ui, 3100.0);
        n.activate(&ui, Target::Fold(2), 0.0, 0.0, 3200.0);
        assert!(n.editor.line_span(one).1 > 0.0, "tapping the chevron unfolds the section");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_gallery_puts_cards_in_rows_and_the_toolbar_toggles_it() {
        let ui = ui();
        let dir = scratch("gallery-grid");
        let mut n = seeded(&ui, &dir);
        n.set_layout(&ui, gallery::Layout::Gallery);
        let (cells, _, total) = n.gallery_grid(&ui);
        assert_eq!(cells.len(), n.shown.len());
        // the list column is at most 270 pt, so the card count per row follows from its width: two cards share a row
        // only when the column is wide enough for them, otherwise the cards stack one per row
        let cols = n.gallery_cols(&ui).0;
        assert!(cells.len() >= 2);
        if cols >= 2 {
            assert!(cells[0].1 == cells[1].1 && cells[1].0 > cells[0].0, "the first cards share a row");
        } else {
            assert!(cells[1].1 > cells[0].1, "one card per row when the list column is narrow");
        }
        assert!(total > cells[0].1, "the grid reports its height for the scroll range");
        draw(&mut n, &ui, 0.0);
        n.toggle_layout(&ui, 0.0);
        assert_eq!(n.prefs().layout, gallery::Layout::List, "the toolbar toggle switches back to rows");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
