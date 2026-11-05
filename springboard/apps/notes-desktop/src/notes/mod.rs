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
pub mod exchange;
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
use bifrost_app::controls::sheet::{Hit, Sheet};

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
/// evdev keys of the editing chords: Backspace, A, C, V, X, Home, End, Left, Right and forward Delete.
const KEY_BACKSPACE: u32 = 14;
const KEY_A: u32 = 30;
const KEY_C: u32 = 46;
const KEY_V: u32 = 47;
const KEY_X: u32 = 45;
const KEY_HOME: u32 = 102;
const KEY_END: u32 = 107;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DELETE: u32 = 111;
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

/// The Markdown marker of an inline character style.
fn marker_of(f: Fmt) -> &'static str {
    match f {
        Fmt::Bold => "**",
        Fmt::Italic => "*",
        Fmt::Strike => "~~",
        Fmt::Underline => "++",
        Fmt::Highlight => "==",
        _ => "`",
    }
}

/// The inline style a character format button gives the text (`None` for the paragraph styles).
fn style_of(f: Fmt) -> Option<md::Style> {
    match f {
        Fmt::Bold => Some(md::Style::Bold),
        Fmt::Italic => Some(md::Style::Italic),
        Fmt::Code => Some(md::Style::Code),
        Fmt::Strike => Some(md::Style::Strike),
        Fmt::Underline => Some(md::Style::Underline),
        Fmt::Highlight => Some(md::Style::Highlight),
        Fmt::Bullet | Fmt::Number | Fmt::Check => None,
    }
}

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
    /// Delete Permanently: asks first (`erase_ask`)
    EraseNote,
    /// the alert's Delete: the note goes for good
    EraseConfirmed,
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
    /// a toolbar item chosen from a bar's `»` overflow menu (`chrome::B_*`)
    Bar(u32),
}

/// Find > Replace: the replacement text being typed in the replace card over the editor.
struct Replacing {
    field: TextField,
}

/// The folder-name card's height (pt), over the top of the list while it is open.
const NAMING_CARD_PT: f32 = 56.0;
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
    /// cycles the paragraph style marker (the touch shortcut row)
    Format,
    /// the toolbar's Aa button: the paragraph style popover
    StyleMenu,
    /// the `»` button of toolbar `n`: the items the bar had no room for
    Overflow(usize),
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
    /// Shift was held for the pointer event being delivered (`set_pointer_shift`): a Shift-click extends the selection
    pointer_shift: bool,
    /// the mouse button went down on the note text (desktop): moves extend the selection instead of scrolling
    text_drag: bool,
    /// character styles chosen with nothing selected (Cmd+B at the caret): the next text typed at `pending_at` gets
    /// them, as the typing attributes of the reference do; nothing is inserted until then
    pending: Vec<Fmt>,
    pending_at: usize,
    /// the Markdown of the last Copy or Cut from the note, so a paste of the same text keeps its styles
    copied: Option<String>,
    /// what each row of the context menu the shell shows does (row id = index + 1)
    context: Vec<Act>,
    /// the id of the note in Recently Deleted that Delete Note or Delete Permanently asks about: the alert is up
    erase_ask: Option<String>,
    /// the alert's button that has the keyboard focus ring (Tab moves it, Space presses it); `None` until Tab
    erase_focus: Option<u32>,
    /// Cmd+B (or another character style) inside a run of that style, with nothing selected: the run is split at
    /// `pending_at` by the next typing there, which goes in plain. Nothing is inserted until then, so no empty marker
    /// pair is left in the note when the caret moves on or nothing is typed
    split: Option<Fmt>,
}

/// The buttons of the Delete Permanently alert.
const ERASE_DELETE: u32 = 1;
const ERASE_CANCEL: u32 = 2;
const ERASE_TITLE: &str = "Are you sure you want to delete this note?";
const ERASE_MESSAGE: &str = "This note will be deleted immediately. You can\u{2019}t undo this action.";

/// The Delete Permanently confirmation as the kit's alert: Delete is destructive, so it is never the Return button.
fn erase_alert() -> bifrost_app::controls::nav::Alert {
    use bifrost_app::controls::nav::{Alert, AlertStyle};
    Alert::new(ERASE_TITLE, ERASE_MESSAGE).action(ERASE_DELETE, "Delete", AlertStyle::Destructive).action(ERASE_CANCEL, "Cancel", AlertStyle::Cancel)
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
        // a desktop window is driven by a mouse: clicks and drags select, and there are no touch handles
        self.editor.set_pointer_mode(on);
        if on {
            self.keyboard.set_shown(false, 0.0);
        }
    }

    /// Whether Shift is held for the pointer event about to be delivered (`touch`): a Shift-click extends the selection.
    pub fn set_pointer_shift(&mut self, on: bool) {
        self.pointer_shift = on;
    }

    /// The scroll wheel (or a two-finger scroll) at pixel `(x, y)`: `dy` px moves the content of the column under the
    /// pointer (positive scrolls toward the top). True when something moved.
    pub fn wheel(&mut self, ui: &Ui, x: f32, _y: f32, dy: f32) -> bool {
        if self.menu.is_some() || self.erase_ask.is_some() || self.settings_open || self.sketch.is_some() || self.picker.is_some() || dy == 0.0 {
            return false;
        }
        self.fit_scroll(ui);
        let s = if x >= self.ed_px(ui).0 {
            if !matches!(self.screen, Screen::Editor(_)) {
                return false;
            }
            &mut self.scroll
        } else if x >= self.list_px(ui).0 {
            &mut self.list_scroll
        } else {
            return false;
        };
        let before = s.offset();
        s.jump_to(before - dy);
        let moved = s.offset() != before;
        self.dirty |= moved;
        moved
    }

    /// A text field over the list or the note has the keys: the search field, a name prompt, or the replace card.
    pub fn field_focused(&self) -> bool {
        self.search.focused || self.naming.is_some() || self.replacing.is_some()
    }

    /// Sends a standard editing command (`Copy`, `Cut`, `Paste`, `SelectAll`) to the focused text field. False when no
    /// field has the keys.
    pub fn field_command(&mut self, ui: &Ui, name: &'static str, t: f64) -> bool {
        if !self.field_focused() {
            return false;
        }
        self.key(ui, Out::Named(name), t);
        true
    }

    /// Something is selected where the keys go: the focused field's text, or the note's.
    pub fn has_selection(&self) -> bool {
        if let Some(r) = &self.replacing {
            let (a, b) = r.field.selection();
            return a != b;
        }
        if let Some(n) = &self.naming {
            let (a, b) = n.field.selection();
            return a != b;
        }
        if self.search.focused {
            let (a, b) = self.search.selection();
            return a != b;
        }
        self.selected_markdown().is_some()
    }

    /// Edit > Select All: the focused field's text, or the whole note. False when neither takes it.
    pub fn select_all(&mut self, ui: &Ui, t: f64) -> bool {
        if self.field_command(ui, "SelectAll", t) {
            return true;
        }
        if !self.is_editing() {
            return false;
        }
        self.editor.select_all();
        self.editor.focused = true;
        self.clear_pending();
        self.dirty = true;
        true
    }

    /// Edit > Paste: the pasteboard's text goes into the focused field, or into the note at the caret (replacing the
    /// selection). Text this note copied comes back with its Markdown styles.
    pub fn paste(&mut self, ui: &Ui, t: f64) -> bool {
        if self.field_command(ui, "Paste", t) {
            return true;
        }
        if !self.is_editing() {
            return false;
        }
        let Some(text) = springboard::shell::clipboard::text() else { return false };
        let text = match &self.copied {
            Some(copied) if md::to_plain(copied) == text || *copied == text => copied.clone(),
            _ => text,
        };
        self.clear_pending();
        self.paste_text(ui, &text, t);
        true
    }

    /// Edit > Copy and Cut from the note: remembers the Markdown for a paste back into Notes and, for `cut`, removes
    /// the selection. The caller puts the text on the pasteboard. `None` when nothing is selected in the note.
    pub fn take_selection(&mut self, ui: &Ui, cut: bool, t: f64) -> Option<String> {
        let markdown = self.selected_markdown()?;
        self.copied = Some(markdown.clone());
        if cut {
            self.editor.replace_selection(ui, "", t);
            self.changed(ui, t);
        }
        Some(markdown)
    }

    /// The character styles waiting for the next typing at the caret (Cmd+B with nothing selected).
    pub fn pending_styles(&self) -> &[Fmt] {
        &self.pending
    }

    /// The right click at pixel `(x, y)`: the rows of the context menu for what is under it, as `(id, title)`. A note
    /// row is selected first and offers Pin, Duplicate, Move and Delete; a folder in the sidebar is shown and offers
    /// New, Rename and Delete Folder. Empty when nothing there has a menu.
    pub fn context_menu(&mut self, ui: &Ui, x: f32, y: f32, t: f64) -> Vec<(u32, String)> {
        self.context.clear();
        let mut rows: Vec<(String, Act)> = vec![];
        match self.hit(ui, x, y) {
            Some(Target::Sidebar(id)) => {
                let Some(view) = self.row_view(id) else { return vec![] };
                let folder = matches!(view, View::Folder(_));
                self.show_view(ui, view, t);
                rows.push(("New Folder".into(), Act::NewFolder));
                if folder {
                    rows.push(("Rename Folder".into(), Act::RenameFolder));
                    rows.push(("Delete Folder".into(), Act::DeleteFolder));
                }
            }
            Some(Target::Row(i) | Target::Pin(i) | Target::Delete(i)) => {
                let Some(&k) = self.shown.get(i) else { return vec![] };
                self.reveal = None;
                self.open_note(ui, k, false, t);
                let Screen::Editor(k) = self.screen else { return vec![] };
                let id = self.notes[k].id.clone();
                let here = self.loc_of(&id).to_string();
                if here == folders::TRASH {
                    rows.push(("Recover Note".into(), Act::Recover));
                    rows.push(("Delete Permanently".into(), Act::EraseNote));
                } else {
                    rows.push((if self.is_pinned(&id) { "Unpin Note" } else { "Pin Note" }.into(), Act::TogglePin));
                    rows.push(("Duplicate Note".into(), Act::Duplicate));
                    if !here.is_empty() {
                        rows.push(("Move to All Notes".into(), Act::MoveTo(String::new())));
                    }
                    for f in self.folders.iter().filter(|f| **f != here) {
                        rows.push((format!("Move to {f}"), Act::MoveTo(f.clone())));
                    }
                    rows.push(("Delete Note".into(), Act::DeleteNote));
                }
            }
            _ => return vec![],
        }
        self.dirty = true;
        self.context = rows.iter().map(|(_, act)| act.clone()).collect();
        rows.into_iter().enumerate().map(|(i, (title, _))| (i as u32 + 1, title)).collect()
    }

    /// A row of the context menu `context_menu` built was chosen. False for an id it did not hand out.
    pub fn context_action(&mut self, ui: &Ui, id: u32, t: f64) -> bool {
        let Some(act) = id.checked_sub(1).and_then(|i| self.context.get(i as usize).cloned()) else { return false };
        self.context.clear();
        self.run(ui, act, t);
        true
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
            pointer_shift: false,
            text_drag: false,
            pending: vec![],
            pending_at: 0,
            copied: None,
            context: vec![],
            erase_ask: None,
            erase_focus: None,
            split: None,
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

    /// Exchange notes: writes any unsaved edit, then lists the local changes to send to `account` (see [`exchange`]).
    pub fn exchange_plan(&mut self, account: &bifrost_app::controls::internet_accounts::Account) -> Vec<bifrost_app::exchange_notes::Op> {
        self.flush();
        exchange::plan(&self.dir, account)
    }

    /// Exchange notes: takes in what a round got from the server. The open note and an unsaved one are left alone, and
    /// the list is read again when the files changed.
    pub fn exchange_apply(&mut self, ui: &Ui, t: f64, account: &bifrost_app::controls::internet_accounts::Account, report: &bifrost_app::exchange_notes::Report) -> exchange::Applied {
        let mut busy: Vec<String> = self.editing().map(|n| n.id.clone()).into_iter().collect();
        busy.extend(self.unsaved.clone());
        // the list is read again from disk below, so an edit that is waiting for the autosave goes to disk first
        self.flush();
        let applied = exchange::apply(&self.dir, account, report, &busy, self.wall.now(t));
        if applied.changed() {
            self.reload(ui, t);
        }
        applied
    }

    /// The folder (or bin) a note is in; None for the root.
    pub fn folder_of(&self, id: &str) -> Option<&str> {
        Some(self.loc_of(id)).filter(|l| !l.is_empty())
    }

    /// Re-reads every note from disk (after folder changes).
    fn reload(&mut self, ui: &Ui, t: f64) {
        let (notes, loc, folders) = load_everything(&self.dir, self.wall.now(t));
        // the open note is found by id: its index is into the list being replaced
        let open = self.editing().map(|n| n.id.clone());
        self.pinned = store::load_pins(&self.dir, &notes);
        self.notes = notes;
        self.loc = loc;
        self.folders = folders;
        self.screen = match open.and_then(|id| self.notes.iter().position(|n| n.id == id)) {
            Some(i) => Screen::Editor(i),
            None => Screen::List,
        };
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
                    title: ui.text(&ui.fit(&n.plain_title(), MAC_BODY_PT, text_w), MAC_BODY_PT, Weight::Bold, LABEL.now()),
                    date,
                    snippet: ui.text(&ui.fit(&snippet, MAC_SUBHEAD_PT, snippet_w), MAC_SUBHEAD_PT, Weight::Regular, LABEL_2.now()),
                    title_text: ui.fit(&n.plain_title(), MAC_BODY_PT, text_w),
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

    /// Where the rows start (pt): below the folder-name card while it is open, so the card never covers a row.
    fn list_top(&self) -> f32 {
        list_top_pt() + if self.naming.is_some() { NAMING_CARD_PT + SPACE_S } else { 0.0 }
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
        (lx + m, ui.pt(self.list_top() + section + ROW_PT * i as f32) - self.list_scroll.offset(), lw - 2.0 * m, ui.pt(ROW_PT))
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

    /// The Share button's rectangle in window points, where the share menu opens under it (the whole window's top
    /// edge when the button is not on the bar).
    pub fn share_anchor(&self, ui: &Ui) -> bifrost_app::controls::Rect {
        self.bar_rect(ui, chrome::B_SHARE).unwrap_or(bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 })
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
        self.notes.iter().find(|n| n.id == id).map(Note::plain_title)
    }

    /// Draws one of the note windows (`note_windows::draw`); `key` says it has the keys (its caret shows). The main
    /// window is drawn by `draw`.
    pub fn draw_window(&mut self, s: &mut Surface, ui: &Ui, window: bifrost_app::app_windows::WindowId, key: bool, t: f64) {
        let text = self.windows.note_of(window).map(str::to_owned);
        let text = text.as_deref().and_then(|id| self.notes.iter().find(|n| n.id == id)).map(|n| n.text.as_str());
        note_windows::draw(window, &mut self.windows, s, ui, text, self.note_settings, key, t);
    }

    /// An input method's composition in a note window (`App::window_text_input`): the marked text composes there and
    /// a change goes back to the note. True when the window needs a new frame.
    pub fn window_text_input(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, ev: &bifrost_app::proto::ToApp, t: f64) -> bool {
        match self.windows.text_input(ui, window, ev, t) {
            Some(changed) => {
                if changed {
                    self.window_changed(ui, window, t);
                }
                true
            }
            None => false,
        }
    }

    /// The scroll wheel over a note window, `dy` in px. True when its text moved.
    pub fn window_wheel(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, dy: f32) -> bool {
        self.windows.wheel(ui, window, dy)
    }

    /// The Edit menu's Undo and Redo titles while a note window has the keys.
    pub fn window_undo_titles(&self, window: bifrost_app::app_windows::WindowId) -> (String, String) {
        self.windows.undo_titles(window)
    }

    /// An input method's composition in the main window (`App::text_input`): marked text, unmark and delete surrounding
    /// go to the focused search field, or to the open note's editor, whose change is saved as typing is. True when a
    /// frame is needed.
    pub fn text_input(&mut self, ui: &Ui, ev: &bifrost_app::proto::ToApp, t: f64) -> bool {
        use bifrost_app::proto::ToApp;
        if self.sketch.is_some() || self.naming.is_some() || self.replacing.is_some() || self.erase_ask.is_some() {
            return false;
        }
        if self.search.focused {
            match ev {
                ToApp::SetMarkedText { text, sel_start, .. } => self.search.set_marked(ui, text, *sel_start as usize),
                ToApp::UnmarkText { commit } => self.search.unmark(ui, *commit),
                _ => return false,
            }
            self.rebuild(ui, t);
            self.dirty = true;
            return true;
        }
        if !matches!(self.screen, Screen::Editor(_)) {
            return false;
        }
        let before = self.editor.text().to_string();
        let handled = match ev {
            ToApp::SetMarkedText { text, sel_start, sel_len } => {
                self.editor.focused = true;
                self.editor.set_marked_text(ui, text, *sel_start as usize, *sel_len as usize, t);
                true
            }
            ToApp::UnmarkText { commit } => self.editor.unmark_text(ui, *commit, t),
            ToApp::DeleteSurrounding { before, after } => self.editor.delete_surrounding(ui, *before as usize, *after as usize, t),
            _ => false,
        };
        if !handled {
            return false;
        }
        if self.editor.text() != before {
            self.changed(ui, t);
        } else {
            self.show_caret(ui);
        }
        self.dirty = true;
        true
    }

    /// A note window's text changed (typing, Cut, Paste, Undo in it): the note takes the text and the autosave is armed
    /// (an unsaved edit of another note is written first); the main editor, when it shows the same note, follows.
    fn window_changed(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, t: f64) {
        let Some(id) = self.windows.note_of(window).map(str::to_owned) else { return };
        let Some(text) = self.windows.text(window).map(str::to_owned) else { return };
        if self.unsaved.as_deref().is_some_and(|u| u != id) {
            self.save(t, true);
        }
        let Some(i) = self.notes.iter().position(|n| n.id == id) else { return };
        self.notes[i].text = text;
        self.notes[i].modified = self.wall.now(t);
        self.unsaved = Some(id);
        self.autosave.touch(t);
        if self.screen == Screen::Editor(i) && self.editor.text() != self.notes[i].text {
            let text = self.notes[i].text.clone();
            self.editor.set_text(ui, &text);
        }
        self.dirty = true;
    }

    /// The mouse in a note window (`NoteWindows::pointer`), Shift as `set_pointer_shift` last said. True when the window
    /// needs a new frame.
    pub fn window_pointer(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, press: note_windows::Press, x: f32, y: f32, t: f64) -> bool {
        if press == note_windows::Press::Down {
            self.windows.focus(window);
        }
        self.windows.pointer(ui, window, press, x, y, t, self.pointer_shift)
    }

    /// A desktop chord in a note window: the editing bindings of the main editor (`chord`), for that window's text.
    /// False when the chord has no meaning there.
    pub fn window_chord(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, key: u32, mods: u32, t: f64) -> bool {
        use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
        let (ctrl, shift, alt, meta) = (mods & CHORD_CTRL != 0, mods & CHORD_SHIFT != 0, mods & CHORD_ALT != 0, mods & CHORD_META != 0);
        if (ctrl || meta) && !alt && !shift && key == KEY_V {
            return self.window_paste(ui, window, t);
        }
        let Some(combo) = bifrost_app::controls::keys::KeyCombo::from_evdev(key, ctrl, shift, alt, meta) else { return false };
        match self.windows.key_combo(ui, window, combo, t) {
            Some(true) => {
                self.window_changed(ui, window, t);
                true
            }
            Some(false) => true,
            None => false,
        }
    }

    /// Typing and the plain editing keys in a note window. True when the window needs a new frame.
    pub fn window_key(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, out: Out, t: f64) -> bool {
        if self.windows.edit(ui, window, &out, t) {
            self.window_changed(ui, window, t);
        }
        self.windows.text(window).is_some()
    }

    /// Edit > Select All in a note window.
    pub fn window_select_all(&mut self, window: bifrost_app::app_windows::WindowId) -> bool {
        self.windows.select_all(window)
    }

    /// Edit > Paste in a note window: the pasteboard's text replaces the selection; text Notes copied keeps its Markdown.
    pub fn window_paste(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, t: f64) -> bool {
        let Some(text) = springboard::shell::clipboard::text() else { return false };
        let text = match &self.copied {
            Some(copied) if md::to_plain(copied) == text || *copied == text => copied.clone(),
            _ => text,
        };
        if !self.windows.replace_selection(ui, window, &text, t) {
            return false;
        }
        self.window_changed(ui, window, t);
        true
    }

    /// Edit > Copy and Cut in a note window: the selected Markdown (remembered for a paste back into Notes); `cut`
    /// removes it. The caller puts it on the pasteboard. `None` when nothing is selected there.
    pub fn window_take_selection(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, cut: bool, t: f64) -> Option<String> {
        let markdown = self.windows.selected_text(window)?;
        self.copied = Some(markdown.clone());
        if cut && self.windows.replace_selection(ui, window, "", t) {
            self.window_changed(ui, window, t);
        }
        Some(markdown)
    }

    /// Edit > Undo (or Redo, with `redo`) in a note window.
    pub fn window_undo(&mut self, ui: &Ui, window: bifrost_app::app_windows::WindowId, redo: bool, t: f64) -> bool {
        if !self.windows.undo(ui, window, redo, t) {
            return false;
        }
        self.window_changed(ui, window, t);
        true
    }

    /// Whether something is selected in a note window (Cut and Copy apply).
    pub fn window_has_selection(&self, window: bifrost_app::app_windows::WindowId) -> bool {
        self.windows.selected_text(window).is_some()
    }

    /// Whether Undo and Redo have a step in a note window.
    pub fn window_can_undo_redo(&self, window: bifrost_app::app_windows::WindowId) -> (bool, bool) {
        self.windows.can_undo_redo(window)
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

    /// True when pixel `(x, y)` is on a link or a checkbox of the note: a click there acts on the release, as a tap.
    fn on_link_or_checkbox(&self, ui: &Ui, x: f32, y: f32) -> bool {
        let (ox, oy) = self.text_origin(ui);
        self.editor.link_at(x - ox, y - oy).is_some() || self.editor.checkbox_at(x - ox, y - oy).is_some()
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
        if let Some((bar, _)) = self.overflow_hit(ui, x, y) {
            return Some(Target::Overflow(bar));
        }
        if let Some(id) = self.bar_hit(ui, x, y) {
            return Some(Self::bar_target(id));
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

    /// What a toolbar item does (`chrome::B_*`).
    fn bar_target(id: u32) -> Target {
        match id {
            chrome::B_NEW_FOLDER => Target::FolderNew,
            chrome::B_SIDEBAR => Target::SidebarToggle,
            chrome::B_LIST_MORE => Target::Title,
            chrome::B_COMPOSE => Target::Compose,
            chrome::B_STYLE => Target::StyleMenu,
            chrome::B_CHECK => Target::Fmt(Fmt::Check),
            chrome::B_BULLET => Target::Fmt(Fmt::Bullet),
            chrome::B_SHARE => Target::EditorShare,
            chrome::B_MORE => Target::EditorMore,
            chrome::B_SEARCH => Target::Search,
            other => Target::Bar(other),
        }
    }

    /// Keeps the scroll range in step with the content.
    fn fit_scroll(&mut self, ui: &Ui) {
        let bottom = self.content_bottom(ui);
        let section = self.sections_total();
        let list_h = if self.prefs.layout == gallery::Layout::Gallery {
            self.gallery_grid(ui).2 + ui.pt(SPACE_L)
        } else {
            ui.pt(self.list_top() + section) + ui.pt(ROW_PT) * self.shown.len() as f32 + ui.pt(SPACE_L)
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

    /// File > New Folder (Shift+Cmd+N): the folder name prompt, its proposed name selected.
    pub fn new_folder(&mut self, ui: &Ui, t: f64) {
        self.activate(ui, Target::FolderNew, 0.0, 0.0, t);
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
        live.into_iter().take(RECENT_MENU).map(|n| (n.id.clone(), n.plain_title())).collect()
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
                .filter(|(_, n)| self.loc_of(&n.id) != folders::TRASH && keep(&n.plain_title().to_lowercase()))
                .max_by_key(|(_, n)| n.modified)
                .map(|(i, _)| i)
        };
        let by_id = self.notes.iter().position(|n| n.id == query.trim() && self.loc_of(&n.id) != folders::TRASH);
        let i = by_id.or_else(|| newest(&|title| title == q)).or_else(|| newest(&|title| title.contains(&q)))?;
        let title = self.notes[i].plain_title();
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

    /// What the main window tells the shell about the open note (`Ctx::text_state`, `Ctx::caret_rect`), so Look Up
    /// and the Services menu find the selection or the word at the caret and the popover opens under it: the note's
    /// text around the caret with the selection, and the caret's box in window px. `None` when no note is open or a
    /// field (Search, a name, Find and Replace) has the keys.
    pub fn text_report(&self, ui: &Ui) -> Option<(bifrost_app::proto::TextState, (f32, f32, f32, f32))> {
        if !self.is_editing() || self.field_focused() {
            return None;
        }
        let text = self.editor.text();
        let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
        let caret = self.editor.caret();
        let anchor = self.editor.selection().map_or(caret, |r| if r.start == caret { r.end } else { r.start });
        let state = bifrost_app::proto::TextState::around(text, byte(caret), byte(anchor));
        let (x, y, w, h) = self.editor.caret_rect(ui);
        let (ox, oy) = self.text_origin(ui);
        Some((state, (ox + x, oy + y, w, h)))
    }

    /// The selection Look Up and the Services menu act on (`App::selected_text`), as plain text: that of the note
    /// window `key` when it has the keys, else the open note's. `Some("")` when nothing is selected there (the shell
    /// then takes the word at the caret); `None` when a field has the keys (the SDK answers from what was reported).
    pub fn look_up_selection(&self, key: bifrost_app::app_windows::WindowId) -> Option<String> {
        if !key.is_main() && self.windows.note_of(key).is_some() {
            return Some(self.windows.selected_text(key).map(|m| md::to_plain(&m)).unwrap_or_default());
        }
        if self.field_focused() {
            return None;
        }
        Some(self.selected_markdown().map(|m| md::to_plain(&m)).unwrap_or_default())
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
                // Delete Note on a selected note in Recently Deleted asks first, as it does with the note open
                Some(row) if self.loc_of(&self.notes[self.shown[row]].id) == folders::TRASH => self.ask_erase(self.shown[row]),
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

    /// Delete Note or Delete Permanently on note `i` in Recently Deleted: the alert asks first, as the reference does
    /// (it cannot be undone).
    fn ask_erase(&mut self, i: usize) {
        self.menu = None;
        self.erase_ask = Some(self.notes[i].id.clone());
        self.erase_focus = None;
        self.dirty = true;
    }

    /// The alert's Delete: the asked-about note goes for good (closed first when it is open).
    fn erase_confirmed(&mut self, ui: &Ui, t: f64) {
        let Some(id) = self.erase_ask.take() else { return };
        self.dirty = true;
        let Some(i) = self.notes.iter().position(|n| n.id == id) else { return };
        if self.screen == Screen::Editor(i) {
            self.autosave.fire(t, true);
            self.unsaved = None;
            self.screen = Screen::List;
            self.keyboard.set_shown(false, t);
            self.editor.focused = false;
            self.hl.clear();
        }
        self.erase(i);
        self.rebuild(ui, t);
    }

    /// The Delete Permanently alert while it is up: the kit's alert (E2E12 finding 6), shown as TextEdit's
    /// confirmations are, a document-modal sheet hanging from the title bar with a bold question, the informative text
    /// under it and full-width stacked buttons (Delete, destructive, on top; Cancel under it).
    fn erase_sheet(&self) -> Option<Sheet> {
        self.erase_ask.as_ref()?;
        Some(erase_alert().sheet().with_focus(self.erase_focus))
    }

    /// Whether the Delete Permanently alert is up, so it takes the keyboard (`erase_key`).
    pub fn erase_open(&self) -> bool {
        self.erase_ask.is_some()
    }

    /// A key press while the alert is up, through the kit's sheet keys (`Sheet::handle_key`): Escape and Cmd+. cancel,
    /// Cmd+D presses Delete, Tab moves the focus ring (drawn on the button) and Space presses the focused button. Return
    /// answers nothing: Delete is destructive, so it is never the default. True when a frame is needed.
    pub fn erase_key(&mut self, ui: &Ui, key: &bifrost_app::KeyInput, t: f64) -> bool {
        use bifrost_app::document::{sheet_key, AlertKey};
        let Some(mut sheet) = self.erase_sheet() else { return false };
        match sheet_key(&mut sheet, key) {
            AlertKey::Answer(ERASE_DELETE) => self.run(ui, Act::EraseConfirmed, t),
            AlertKey::Answer(_) => self.erase_ask = None,
            AlertKey::Focus => self.erase_focus = sheet.focus,
            AlertKey::Ignored => return false,
        }
        self.dirty = true;
        true
    }

    /// The window in points (the alert's parent) and the height of its title bar, which the alert hangs from.
    fn sheet_parent(&self, ui: &Ui) -> (bifrost_app::controls::Rect, f32) {
        let k = ui.pt(1.0);
        (bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: self.w as f32 / k, h: self.h as f32 / k }, nav_bottom_pt())
    }

    /// A tap while the alert is up: Delete erases, Cancel closes it, and the rest of the window is blocked.
    fn erase_tap(&mut self, ui: &Ui, x: f32, y: f32, t: f64) {
        let Some(sheet) = self.erase_sheet() else { return };
        let (parent, bar) = self.sheet_parent(ui);
        let k = ui.pt(1.0);
        match sheet.hit(parent, bar, 1.0, x / k, y / k) {
            Hit::Button(ERASE_DELETE) => self.run(ui, Act::EraseConfirmed, t),
            Hit::Button(_) => self.erase_ask = None,
            _ => {}
        }
    }

    /// Draws the alert over the whole window (`draw` calls it last).
    fn draw_erase_ask(&self, s: &mut Surface, ui: &Ui) {
        let Some(sheet) = self.erase_sheet() else { return };
        let (parent, bar) = self.sheet_parent(ui);
        bifrost_app::widgets::sheet(s, ui, parent, bar, &sheet, 1.0, None);
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
        let caret = self.editor.caret();
        let para = self.editor.paragraph_of(caret);
        let line: String = self.editor.text().chars().skip(para.start).take(para.len()).collect();
        let info = md::classify(&line);
        // a list or checklist line becomes the heading with its marker gone (`md::set_block`), never `## - [ ] item`
        let next = match info.block {
            md::Block::Heading => md::Block::Title,
            md::Block::Title => md::Block::Body,
            _ => md::Block::Heading,
        };
        let new = md::set_block(&line, next);
        let into = caret.saturating_sub(para.start + info.prefix);
        let at = para.start + md::classify(&new).prefix + into;
        self.editor.replace_range(ui, para.clone(), &new, t, false);
        self.editor.set_caret(at.min(para.start + new.chars().count()));
        self.changed(ui, t);
    }

    /// The Aa button: the paragraph styles as a popover under it (Title, Heading, Subheading, Body, Monostyled, Block
    /// Quote and the lists), the caret's style checked. With the button behind `»` the popover opens there.
    fn open_style_menu(&mut self, ui: &Ui) {
        if !self.is_editing() {
            return;
        }
        let para = self.editor.paragraph_of(self.editor.caret());
        let line: String = self.editor.text().chars().skip(para.start).take(para.len()).collect();
        let current = md::classify(&line).block;
        let same = |b: md::Block| std::mem::discriminant(&b) == std::mem::discriminant(&current);
        let items = PARAGRAPH_STYLES.iter().map(|(label, block)| (label.to_string(), Act::Style(*block), same(*block))).collect();
        let (x, y, _, h) = match self.bar_rect(ui, chrome::B_STYLE).or_else(|| self.editor_overflow(ui)) {
            Some(r) => chrome::px(ui, r),
            None => (self.ed_px(ui).0, 0.0, 0.0, ui.pt(bifrost_app::layout::UNIFIED_BAR_PT)),
        };
        self.show_menu(ui, items, x, y + h + ui.pt(SPACE_XS));
    }

    /// The `»` button of toolbar `bar`: a menu of the items the bar had no room for.
    fn open_overflow_menu(&mut self, ui: &Ui, bar: usize, x: f32, y: f32) {
        let items: Vec<(String, Act, bool)> = self.overflow_items(ui, bar).into_iter().filter(|(_, _, on)| *on).map(|(id, label, _)| (label, Act::Bar(id), false)).collect();
        if items.is_empty() {
            return;
        }
        let (bx, by, bw, bh) = self.overflow_hit(ui, x, y).map_or((x, y, 0.0, 0.0), |(_, r)| chrome::px(ui, r));
        self.show_menu(ui, items, bx + bw - ui.pt(MENU_W_PT), by + bh + ui.pt(SPACE_XS));
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
                if sel.is_empty() && self.end_style_at_caret(ui, f, sel.start, t) {
                    return;
                }
                if sel.is_empty() {
                    // nothing selected: the style waits for the next typing (a typing attribute), so no markers
                    // appear in the note until there is text for them to hold
                    if self.pending_at != sel.start {
                        self.clear_pending();
                        self.pending_at = sel.start;
                    }
                    match self.pending.iter().position(|p| *p == f) {
                        Some(k) => {
                            self.pending.remove(k);
                        }
                        None => self.pending.push(f),
                    }
                    self.dirty = true;
                    return;
                }
                let r = md::toggle_wrap_lines(&text, sel, marker_of(f));
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

    /// Cmd+B (or another character style) at a caret inside text that already has the style: the typing that follows
    /// is without it, as the reference's typing attributes turn off. At the end of the styled run the caret steps out
    /// past the closing marker, at its start before the opening one, and inside it the run is split at the caret
    /// (one undo step). False when the caret is not in such a run.
    fn end_style_at_caret(&mut self, ui: &Ui, f: Fmt, caret: usize, t: f64) -> bool {
        let _ = (ui, t);
        let Some(style) = style_of(f) else { return false };
        if self.split == Some(f) && self.pending_at == caret {
            // pressed again before any typing: the style goes on for the typing after all
            self.split = None;
            self.dirty = true;
            return true;
        }
        let para = self.editor.paragraph_of(caret);
        let line: String = self.editor.text().chars().skip(para.start).take(para.len()).collect();
        let c = caret - para.start;
        let Some((inner, ml)) = md::emphasis_at(&line, md::classify(&line).prefix, c, style) else { return false };
        self.clear_pending();
        self.dirty = true;
        if c == inner.end {
            self.editor.set_caret(caret + ml);
        } else if c == inner.start {
            self.editor.set_caret(caret - ml);
        } else {
            // the split waits for the typing (`type_pending`): inserting an empty `****` now would stay in the note
            // when nothing is typed there
            self.split = Some(f);
            self.pending_at = caret;
        }
        true
    }

    /// Drops the styles waiting for the next typing and a split waiting at the caret.
    fn clear_pending(&mut self) {
        self.pending.clear();
        self.split = None;
    }

    /// Typed `s` at the caret with styles pending (Cmd+B with nothing selected): the text goes in wrapped in their
    /// markers and the caret stays inside them, so the typing that follows keeps the style. False (nothing done) when
    /// no style is pending at the caret; a pending style left behind by a caret move is dropped.
    fn type_pending(&mut self, ui: &Ui, s: &str, t: f64) -> bool {
        if self.pending.is_empty() && self.split.is_none() {
            return false;
        }
        let caret = self.editor.caret();
        if self.editor.selection().is_some() || caret != self.pending_at {
            self.clear_pending();
            return false;
        }
        // a split run closes before the typing and opens again after it, so the typing between is without its style
        let cut = self.split.map_or("", marker_of);
        let open: String = std::iter::once(cut).chain(self.pending.iter().map(|f| marker_of(*f))).collect();
        let close: String = self.pending.iter().rev().map(|f| marker_of(*f)).chain(std::iter::once(cut)).collect();
        self.clear_pending();
        self.editor.replace_range(ui, caret..caret, &format!("{open}{s}{close}"), t, false);
        self.editor.set_caret(caret + open.chars().count() + s.chars().count());
        true
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
        // the proposed name is selected, as in the reference: typing replaces it instead of appending to it
        field.edit(ui, &Out::Named("SelectAll"), t);
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
                match &naming.old {
                    // a search left in the field would list every folder's matches in the new, empty folder; the
                    // open note is not in it, so the switch closes the editor as a sidebar switch does
                    None => {
                        self.searching = false;
                        self.show_view(ui, View::Folder(name), t);
                    }
                    Some(old) => {
                        if self.view == View::Folder(old.clone()) {
                            self.view = View::Folder(name);
                        }
                        self.reload(ui, t);
                    }
                }
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
                    self.share_req = Some(springboard::shell::share::Item::Text { title: note.plain_title(), body: md::to_plain(&note.text) });
                }
            }
            Act::SharePdf => {
                if let Screen::Editor(i) = self.screen {
                    self.save(t, true);
                    let note = &self.notes[i];
                    let bytes = pdf::render(&note.plain_title(), &md::to_plain(&note.text));
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
                    let page = html::render(&note.plain_title(), &note.text);
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
                    // a note already in Recently Deleted goes for good, after the alert Delete Permanently shows too:
                    // trashing it again would move it onto itself
                    if self.loc_of(&self.notes[i].id) == folders::TRASH {
                        self.run(ui, Act::EraseNote, t);
                        return;
                    }
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
                    self.ask_erase(i);
                }
            }
            Act::EraseConfirmed => self.erase_confirmed(ui, t),
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
            Act::Bar(id) => self.activate(ui, Self::bar_target(id), 0.0, 0.0, t),
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
        let mut y = ui.pt(self.list_top());
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
                    if bifrost_app::pans_now() {
                        p.scroll = (p.scroll + y0 - y).clamp(0.0, (total - (sh - head)).max(0.0));
                    }
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
        if self.erase_ask.is_some() {
            // the alert takes the keys: Return does not answer its destructive Delete, and Escape (`escape`) cancels
            if matches!(out, Out::Hide) {
                self.erase_ask = None;
            }
            return;
        }
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
                if let Out::Text(s) = &out {
                    if self.type_pending(ui, s, t) {
                        self.changed(ui, t);
                        return;
                    }
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

    /// Desktop chords (evdev keys with `CHORD_*` modifiers). Cmd or Ctrl: N new note and F search, Z undo and Shift+Z
    /// redo, B, I, U and Shift+X, Shift+E the character styles, Shift+T, H, J, B, M, L the paragraph styles, K a link,
    /// A select all, V paste, Backspace Delete Note (the Note menu's key equivalent, which wins over the text's
    /// delete-to-line-start as a menu key equivalent does). Without them, and for every other key, the editing keys of
    /// `bifrost_ui::text::binding` (the manifest's `edit_keys` sends the arrows, Home, End and Delete with their
    /// modifiers): Shift extends the selection, Option moves by word, Cmd to the line and text ends, and the emacs Ctrl
    /// keys. A focused field (search, a name prompt) takes the movement keys itself. False when the chord has no
    /// meaning here.
    pub fn chord(&mut self, ui: &Ui, key: u32, mods: u32, t: f64) -> bool {
        use bifrost_app::proto::{CHORD_ALT, CHORD_CTRL, CHORD_META, CHORD_SHIFT};
        let editing = matches!(self.screen, Screen::Editor(_));
        let field = self.field_focused();
        let (ctrl, shift, alt, meta) = (mods & CHORD_CTRL != 0, mods & CHORD_SHIFT != 0, mods & CHORD_ALT != 0, mods & CHORD_META != 0);
        if self.erase_ask.is_some() {
            // the alert is modal: no command reaches the note behind it
            return true;
        }
        if key == KEY_TAB && mods == CHORD_SHIFT && editing {
            // Shift+Tab takes a level off the list lines
            self.shift_indent(ui, false, t);
            return true;
        }
        if (ctrl || meta) && !alt {
            if let Some(done) = self.command_chord(ui, key, ctrl, shift, meta, t) {
                self.dirty = true;
                return done;
            }
        }
        if field {
            return self.field_chord(ui, key, shift, alt || ctrl, meta, t);
        }
        if !editing {
            return false;
        }
        let Some(combo) = bifrost_app::controls::keys::KeyCombo::from_evdev(key, ctrl, shift, alt, meta) else { return false };
        let Some(changed) = self.editor.key_combo(ui, combo, t) else { return false };
        self.dirty = true;
        if changed {
            self.clear_pending();
            self.changed(ui, t);
        } else {
            self.show_caret(ui);
        }
        true
    }

    /// The Cmd (or Ctrl) chords that are commands rather than editing keys. `None` when `key` is not one of them here,
    /// so the editing keys get it; `Some(false)` when it is but there was nothing to do.
    fn command_chord(&mut self, ui: &Ui, key: u32, ctrl: bool, shift: bool, meta: bool, t: f64) -> Option<bool> {
        let editing = matches!(self.screen, Screen::Editor(_));
        let field = self.field_focused();
        if editing && ctrl && meta && (key == KEY_UP || key == KEY_DOWN) {
            // Ctrl+Cmd+Up and Down move the list item
            self.move_items(ui, key == KEY_UP, t);
            return Some(true);
        }
        if key == KEY_K && !shift && editing && !field {
            // Cmd+K asks for a web address
            self.start_link(ui, t);
            return Some(true);
        }
        if key == KEY_BACKSPACE && meta && !ctrl && !shift && !field {
            // Cmd+Backspace: Note > Delete Note, for the open note or the arrow-selected row
            return Some(self.can_delete() && self.delete_current(ui, t));
        }
        if editing && !field && meta && shift && !ctrl {
            // the Format menu's paragraph styles: Shift+Cmd+T, H, J, B, M, L
            let block = match key {
                20 => Some(md::Block::Title),
                35 => Some(md::Block::Heading),
                36 => Some(md::Block::Subheading),
                48 => Some(md::Block::Body),
                50 => Some(md::Block::Mono),
                38 => Some(md::Block::Check(false)),
                _ => None,
            };
            if let Some(block) = block {
                return Some(self.style(ui, block, t));
            }
        }
        if !shift {
            match key {
                // Cmd+A selects all (Ctrl+A is the line start: an editing key)
                KEY_A if meta => return Some(self.select_all(ui, t)),
                KEY_V => return Some(self.paste(ui, t)),
                // Copy and Cut in the note go through the app (the pasteboard service); here only a field's
                KEY_C if field => return Some(self.field_command(ui, "Copy", t)),
                KEY_X if field => return Some(self.field_command(ui, "Cut", t)),
                _ => {}
            }
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
            (Screen::Editor(_), 18, true) => Target::Fmt(Fmt::Highlight),
            _ => return None,
        };
        self.activate(ui, target, 0.0, 0.0, t);
        Some(true)
    }

    /// A movement or delete chord for the focused field (search, a name prompt, the replace card), in the names the
    /// shell's text field reads. False for any other key.
    fn field_chord(&mut self, ui: &Ui, key: u32, shift: bool, word: bool, line: bool, t: f64) -> bool {
        let pick = |plain: &'static str, extend: &'static str| if shift { extend } else { plain };
        let name = match key {
            KEY_HOME => pick("Home", "ShiftHome"),
            KEY_END => pick("End", "ShiftEnd"),
            KEY_LEFT if line => pick("Home", "ShiftHome"),
            KEY_RIGHT if line => pick("End", "ShiftEnd"),
            KEY_LEFT if word => pick("WordLeft", "ShiftWordLeft"),
            KEY_RIGHT if word => pick("WordRight", "ShiftWordRight"),
            KEY_LEFT => pick("Left", "ShiftLeft"),
            KEY_RIGHT => pick("Right", "ShiftRight"),
            KEY_DELETE if word => "DeleteWordForward",
            KEY_DELETE => "DeleteForward",
            KEY_BACKSPACE if line => "DeleteLine",
            KEY_BACKSPACE if word => "DeleteWord",
            _ => return false,
        };
        self.key(ui, Out::Named(name), t);
        true
    }

    /// Escape: closes a menu, cancels a name prompt, clears the search text (then leaves the empty field), or drops the
    /// note's selection and any pending style. The open note stays open, as in the reference (Back and the list close it).
    pub fn escape(&mut self, ui: &Ui, t: f64) -> bool {
        if self.erase_ask.take().is_some() {
            // Escape is the alert's Cancel
            self.dirty = true;
            return true;
        }
        if self.menu.take().is_some() {
            self.dirty = true;
            return true;
        }
        if self.replacing.is_some() {
            self.cancel_replacing(t);
            return true;
        }
        if self.naming.is_some() {
            self.cancel_naming(t);
            return true;
        }
        if self.search.focused {
            // a query goes first: the list shows every note again and the field stays open for the next search
            if !self.search.is_empty() {
                self.search.set_text(ui, "");
                self.rebuild(ui, t);
                self.dirty = true;
                return true;
            }
            self.search.focused = false;
            self.searching = false;
            self.keyboard.set_shown(false, t);
            self.dirty = true;
            return true;
        }
        match self.screen {
            Screen::Editor(_) => {
                let had = self.editor.selection().is_some() || !self.pending.is_empty() || self.split.is_some();
                let caret = self.editor.caret();
                self.editor.set_caret(caret);
                self.clear_pending();
                self.dirty |= had;
                had
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
                    self.share_req = Some(springboard::shell::share::Item::Text { title: self.notes[i].plain_title(), body: self.notes[i].text.clone() });
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
            Target::StyleMenu => self.open_style_menu(ui),
            Target::Overflow(bar) => self.open_overflow_menu(ui, bar, x, y),
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
        if self.erase_ask.is_some() {
            // the alert is modal: its buttons answer and the window behind it is blocked
            if let Event::Up(..) = ev {
                self.erase_tap(ui, x, y, t);
            }
            return;
        }
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
                    Some(Target::Text) if self.hw_kb && !self.on_link_or_checkbox(ui, x, y) => {
                        // the desktop: the press places the caret (a double or triple click selects the word or the
                        // paragraph, Shift extends) and a drag selects; the wheel scrolls (`wheel`)
                        let (ox, oy) = self.text_origin(ui);
                        self.pressed = None;
                        self.text_drag = true;
                        self.clear_pending();
                        self.search.focused = false;
                        self.editor.press(ui, x - ox, y - oy, t, self.pointer_shift);
                    }
                    Some(Target::Text) => {
                        self.scroll.down(y, t);
                        self.scrolling = true;
                    }
                    _ => {}
                }
            }
            Event::Move(..) => {
                if self.text_drag {
                    let (ox, oy) = self.text_origin(ui);
                    self.editor.drag_to(x - ox, y - oy);
                } else if self.kb_touch {
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
                if std::mem::take(&mut self.text_drag) {
                    let (ox, oy) = self.text_origin(ui);
                    self.editor.drag_to(x - ox, y - oy);
                    self.editor.release();
                    return;
                }
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
            let waiting = self.unsaved.is_some();
            self.save(t, false);
            if waiting && self.unsaved.is_none() {
                // the debounce lapsed: the list row (title, snippet, date, order) follows the text without a refocus
                self.rebuild(ui, t);
                self.dirty = true;
            }
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
            // the editor's caret blinks while the on-screen keyboard is up, and always with a hardware keyboard
            Screen::Editor(_) if self.keyboard.shown() || self.hw_kb => self.editor.next_blink(t),
            _ => None,
        };
        // the caret of the key note window blinks on the same clock
        for b in blink.into_iter().chain(self.windows.next_blink(t)) {
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
        e.set_pointer_mode(self.hw_kb);
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
            let top = ui.pt(self.list_top());
            let img = self.labels.get(ui, msg, MAC_TITLE3_PT, Weight::Regular, LABEL_2.now());
            centred(&mut content, img, (lx + lw / 2.0) as i32, (top + ui.pt(SPACE_HUGE * 2.0)) as i32);
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
            let title_text = ui.fit(&note.plain_title(), MAC_BODY_PT, inner);
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
            centred(&mut content, &img, (lx + ui.pt(SPACE_S)) as i32, (ui.pt(self.list_top()) + ui.pt(SPACE_HUGE * 2.0)) as i32);
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
        let (x, y, w, h) = (lx + pad, ui.pt(nav_bottom_pt()) + pad, lw - 2.0 * pad, ui.pt(NAMING_CARD_PT));
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
        self.draw_erase_ask(s, ui);
    }
}

#[cfg(test)]
mod tests;
