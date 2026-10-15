//! Books: a PDF and EPUB reader. A library grid of covers; a reading view
//! that reflows text into pages at the chosen size, turns pages by tap or
//! swipe, resumes where you stopped, and offers contents, bookmarks, search
//! inside the book and font size / theme settings. Format parsing lives in
//! `engine`, pagination in `layout`, persistence in `store`.

pub use springboard::shell::books::engine;
#[cfg(test)]
pub mod icon;
pub mod layout;
pub mod narrate;
pub mod quote;
pub mod search_index;
pub mod share;
pub mod store;

mod draw;

pub(super) use springboard::layout::in_rect as within;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::controls::textfind;
use bifrost_app::controls::{pdfannot, pdfselect};
use bifrost_app::controls::pdfview::DisplayMode;
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_TITLE3_PT};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::proto::ContextItem;
use bifrost_app::widgets;
use bifrost_app::kit::ui::{SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XXL, SPACE_XXXL};
use springboard::shell::cache::TextCache;
use springboard::shell::keyboard::Out;
use springboard::shell::textfield::TextField;
use springboard::shell::ui::{home_zone, Ui};
use springboard::shell::Event;
use springboard::deildu::scroll::Scroller;
use springboard::gfx::{Image, Surface};
use engine::{Book, Hit};
use layout::{Layout, Metrics};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use store::{Pos, SortBy, Span, Status, Store, Theme, ViewMode, SIZES};

#[cfg(test)]
mod highlight_tests;
#[cfg(test)]
mod connect_tests;
#[cfg(test)]
mod narrate_tests;
#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod parity_app_books_tests;
#[cfg(test)]
mod share_tests;


/// The unified toolbar band of the full-size window (Golden Gate: no title row).
pub(super) fn nav_bottom_pt() -> f32 { UNIFIED_BAR_PT }
const SIDEBAR_PT: f32 = 200.0;
/// The contents / bookmarks / search / display pane at the reader's leading edge (Books for Mac).
const PANEL_PT: f32 = 340.0;
const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
// Toolbar item ids; they are also the control ids the shell does not drag the window over.
pub(super) const BAR_SIDEBAR: u32 = 1;
pub(super) const BAR_TITLE: u32 = 2;
pub(super) const BAR_CONTENTS: u32 = 10;
pub(super) const BAR_MARK: u32 = 11;
pub(super) const BAR_SEARCH: u32 = 12;
pub(super) const BAR_DISPLAY: u32 = 13;
pub(super) const BAR_CLOSE: u32 = 14;
/// The library's search field, at the trailing end of the unified toolbar (Books for Mac).
pub(super) const BAR_LIBRARY_SEARCH: u32 = 15;
const LIBRARY_SEARCH_PT: f32 = 220.0;
/// Sidebar row ids.
const ROW_BASE: u32 = 100;
/// Item ids of the library's context menu; the collections start at `CTX_COLLECTION_BASE`.
const CTX_OPEN: u32 = 1;
const CTX_WANT: u32 = 2;
const CTX_FINISHED: u32 = 3;
const CTX_COLLECTION_BASE: u32 = 100;
/// A context menu holds at most 32 items (the wire limit); the collections share it with three fixed items.
const MAX_COLLECTION_ITEMS: usize = 20;
/// A row of the library as a list.
const LIST_ROW_PT: f32 = 68.0;
const MARGIN_PT: f32 = 24.0;
/// The gap between the two pages of a spread (View > Two Pages).
const PAGE_GUTTER_PT: f32 = 40.0;
fn page_top_pt() -> f32 { UNIFIED_BAR_PT + 14.0 }
const FOOTER_PT: f32 = 72.0;
const BOTTOM_BAR_PT: f32 = 92.0;
const ROW_PT: f32 = 52.0;
const TAB_PT: f32 = 44.0;
const SWIPE_PT: f32 = 50.0;
const SEARCH_LIMIT: usize = 300;
/// A tap on this fraction of the width at either edge turns the page.
const EDGE: f32 = 0.28;
const SAVE_DELAY_MS: f64 = 1500.0;
pub(super) const SLIDE_MS: f64 = 320.0;
const SECTION_PT: f32 = 40.0;
/// Smallest cover column width the shelf fills the window with.
const COVER_MIN_PT: f32 = 170.0;
/// The theme swatch row below the text-size buttons (its top and height).
const THEME_ROW_PT: f32 = 130.0;
/// The rows below the themes in the Display panel (tops, from the size buttons) and the height of a choice button.
const SPACING_ROW_PT: f32 = 252.0;
const MARGIN_ROW_PT: f32 = 330.0;
const JUSTIFY_ROW_PT: f32 = 408.0;
const CHOICE_PT: f32 = 44.0;

/// A named colour at `a` opacity, premultiplied the way `gfx::rgba` does it. Derives tints from tokens.
pub(super) const fn fade(c: u32, a: u8) -> u32 {
    const fn channel(c: u32, shift: u32, a: u8) -> u8 {
        ((((c >> shift) & 0xFF) * a as u32 + 127) / 255) as u8
    }
    springboard::gfx::pack(channel(c, 0, a), channel(c, 8, a), channel(c, 16, a), a)
}

/// Results from pool jobs. A job calls `deliver`, which pushes on the main queue and so wakes the app loop; `tick`
/// drains the box. A job's clone of the box is dropped when the job ends, so `finished` tells the app that nothing
/// more is coming. Nothing polls for results.
type Inbox<T> = Arc<Mutex<VecDeque<T>>>;

fn deliver<T: Send + 'static>(inbox: &Inbox<T>, value: T) {
    let inbox = Arc::clone(inbox);
    DispatchQueue::main().async_(move || {
        inbox.lock().unwrap_or_else(|e| e.into_inner()).push_back(value);
    });
}

/// True once every job that could deliver into `inbox` has ended and its deliveries have run.
fn finished<T>(inbox: &Inbox<T>) -> bool {
    Arc::strong_count(inbox) == 1
}

fn drain<T>(inbox: &Inbox<T>) -> Vec<T> {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).drain(..).collect()
}

/// The hit a Find Next (`delta` 1) or Find Previous (`delta` -1) goes to, wrapping at both ends. With no hit visited
/// yet, Find Next takes the first and Find Previous the last. None when there are no hits.
pub(super) fn step_index(current: Option<usize>, delta: i32, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let n = n as i64;
    Some(match current {
        None if delta >= 0 => 0,
        None => (n - 1) as usize,
        Some(i) => (i as i64 + delta as i64).rem_euclid(n) as usize,
    })
}

/// The hits whose match is a whole word: the match touches no letter or digit on either side (textfind rule).
pub(super) fn keep_whole_words(blocks: &[engine::Block], query: &str, hits: Vec<Hit>) -> Vec<Hit> {
    let opts = textfind::Options { ignore_case: true, whole_word: true };
    let q = query.trim();
    hits.into_iter()
        .filter(|h| {
            let Some(text) = blocks.get(h.block).and_then(|b| b.text()) else { return false };
            textfind::find_all(text, q, opts).iter().any(|r| text[..r.start].chars().count() == h.offset)
        })
        .collect()
}

/// The text of one reflowed page, as pdfselect reads it: its lines in order, joined with line breaks. Pictures are
/// left out, so a character index in this text is the same one `draw_page` counts down the lines.
pub(super) fn page_text(page: &layout::Page) -> String {
    page.lines.iter().filter(|l| !matches!(l.kind, layout::Kind::Image { .. })).map(|l| l.text.as_str()).collect::<Vec<&str>>().join("\n")
}

/// The extra width each space of a body line takes when the text is justified (View > Justify Text, the Display panel):
/// the slack of the line shared between its spaces, see `layout::justify_extra`. Zero when justification is off, for a
/// heading, and for the last line of a paragraph, which stays ragged. `block_text` is the text of the line's block, `pt`
/// the point size the line is set in and `width` the column width in px.
pub(super) fn line_extra(ui: &Ui, justify: bool, block_text: &str, line: &layout::Line, pt: f32, width: f32) -> f32 {
    if !justify || !matches!(line.kind, layout::Kind::Body) || layout::ends_paragraph(block_text, line.offset, &line.text) {
        return 0.0;
    }
    let weight = springboard::shell::ui::Weight::Regular;
    layout::justify_extra(ui.measure(&line.text, pt, weight), width, line.text.matches(' ').count(), ui.measure(" ", pt, weight))
}

/// The page in view when the pages of a book follow each other `pitch` px apart and the window is scrolled `offset` px: the
/// one that covers the middle of the window.
pub(super) fn page_at(offset: f32, pitch: f32, pages: usize) -> usize {
    if pitch <= 0.0 || pages == 0 {
        return 0;
    }
    (((offset + pitch / 2.0) / pitch).floor().max(0.0) as usize).min(pages - 1)
}

/// The text of a book as a printout: its headings and paragraphs in reading order, a blank line between them. Pictures
/// are left out.
pub(super) fn book_text(blocks: &[engine::Block]) -> String {
    let mut out = String::new();
    for text in blocks.iter().filter_map(|b| b.text()).map(str::trim).filter(|t| !t.is_empty()) {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(text);
    }
    out
}

/// The page texts of a whole layout, in page order, as the document model of pdfselect.
fn document_text(layout: &Layout) -> Vec<String> {
    layout.pages.iter().map(page_text).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Contents,
    Bookmarks,
    /// one row per page, with the page's first line as its text thumbnail
    Pages,
    /// the notes written on highlights, with the note field above the list
    Notes,
    Search,
    Display,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    // library
    Cover(usize),
    Shelf,
    Sidebar(Filter),
    ToggleSidebar,
    LibrarySearch,
    LibraryClear,
    // reader chrome
    Back,
    NavContents,
    NavSearch,
    NavDisplay,
    NavMark,
    Slider,
    PageArea,
    // panels
    Tab(Panel),
    Row(usize),
    RowRemove(usize),
    SearchField,
    NoteField,
    SizeDown,
    SizeUp,
    ThemeButton(usize),
    /// Display panel: line spacing (index into `store::SPACINGS`), side margin (into `store::MARGINS`), justification
    Spacing(usize),
    Margin(usize),
    Justify,
    PanelBody,
}

/// One book on the shelf.
pub(super) struct Cover {
    pub path: PathBuf,
    pub title: String,
    pub author: String,
    pub image: Option<Image>,
    /// the image scaled to the tile it is drawn in
    pub tile: Option<Image>,
}

/// The message the library probe thread sends per book.
type Probed = (PathBuf, engine::Meta);

pub(super) struct Reader {
    pub path: PathBuf,
    pub book: Book,
    pub layout: Layout,
    pub page: usize,
    pub chrome: bool,
    pub panel: Option<Panel>,
    pub search: TextField,
    pub query: String,
    pub hits: Vec<Hit>,
    /// the hit the last Find Next or Find Previous went to
    pub find_at: Option<usize>,
    pub searched: bool,
    pub highlight: Option<(usize, usize, usize)>,
    /// pictures scaled for the current layout, by (block, width)
    pub images: HashMap<(usize, u32), Image>,
    /// the chapter being read aloud (Find menu), if any
    pub narration: Option<narrate::Narration>,
    /// the note field of the Notes panel
    pub note: TextField,
    /// the highlight whose note the note field edits
    pub note_target: Option<Span>,
    /// the text picked by Select Page, as character ranges over the layout's pages (see `page_text`)
    pub selection: Option<pdfselect::Selection>,
}

struct Loading {
    path: PathBuf,
    inbox: Inbox<Result<Book, String>>,
}

/// The library collection shown, one per sidebar row (Books for Mac: Library > All, Books, PDFs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Reading,
    Books,
    Pdfs,
    WantToRead,
    Finished,
    /// a collection the reader made, by its place in `Store::collections`
    Collection(usize),
}

impl Filter {
    /// The fixed rows of the sidebar, in order: Library (4), then Collections (2). The reader's own collections follow.
    pub const ALL: [Filter; 6] = [Filter::Reading, Filter::All, Filter::Books, Filter::Pdfs, Filter::WantToRead, Filter::Finished];
    /// Where the Collections section starts in `ALL`.
    const COLLECTIONS_AT: usize = 4;
    pub fn name(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Reading => "Reading Now",
            Filter::Books => "Books",
            Filter::Pdfs => "PDFs",
            Filter::WantToRead => "Want to Read",
            Filter::Finished => "Finished",
            Filter::Collection(_) => "Collection",
        }
    }
    fn symbol(self) -> &'static str {
        match self {
            Filter::All => "square.grid.2x2",
            Filter::Reading => "clock",
            Filter::Books => "doc",
            Filter::Pdfs => "doc",
            Filter::WantToRead => "star",
            Filter::Finished => "checkmark.circle",
            Filter::Collection(_) => "folder",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Library,
    Reader,
}

pub struct Books {
    pub(super) w: i32,
    pub(super) h: i32,
    dirs: Vec<PathBuf>,
    pub(super) store: Store,
    pub(super) shelf: Vec<Cover>,
    probes: Option<Inbox<Probed>>,
    screen: Screen,
    pub(super) reader: Option<Reader>,
    loading: Option<Loading>,
    pub(super) error: Option<String>,
    pub(super) shelf_scroll: Scroller,
    pub(super) panel_scroll: Scroller,
    pub(super) labels: TextCache,
    pressed: Option<Target>,
    down: Option<(f32, f32)>,
    scrolling: bool,
    save_at: Option<f64>,
    pub(super) dirty: bool,
    /// page slide in flight: (page left behind, direction, start ms)
    pub(super) slide: Option<(usize, i32, f64)>,
    pub(super) filter: Filter,
    pub(super) sidebar_hidden: bool,
    /// the library's search text; it narrows the shelf by title and author
    pub(super) library_search: TextField,
    /// Find in book matches whole words only (Find menu); kept for the session
    pub(super) whole_words: bool,
    /// a one-line result of Look Up or Translate, shown on the page until the next tap
    pub(super) notice: Option<String>,
    /// View menu: one page, or two pages side by side (the pdfview display modes). Kept for the session.
    pub(super) display: DisplayMode,
    /// where Add to Library and shared files are copied to
    import_dir: PathBuf,
    /// the book a context menu in the library was opened on
    context_book: Option<PathBuf>,
    /// counts the time a book is open towards the reading goal
    clock: store::ReadingClock,
    /// View menu: the pages of the book follow each other down the window and scroll. Kept for the session.
    continuous: bool,
    /// the scroll position of the pages in continuous scrolling, in px from the top of the first page
    pub(super) page_scroll: Scroller,
    /// the distance from the top of one page to the top of the next in continuous scrolling, in px
    pitch: f32,
}

impl Books {
    /// Hosts may supply their platform's document folders while keeping state private. their platform's document folders while keeping state private.
    pub fn with_library_dirs(ui: &Ui, w: i32, h: i32, dir: PathBuf, additional: Vec<PathBuf>) -> Self {
        let store = Store::load(dir.join("state.tsv"));
        // the first folder the host names is the Books folder; without one, books go in the private data folder
        let import_dir = additional.first().cloned().unwrap_or_else(|| dir.clone());
        let mut dirs: Vec<PathBuf> = vec![dir.clone()];
        for folder in additional {
            if !dirs.contains(&folder) { dirs.push(folder); }
        }
        let mut b = Books {
            w,
            h,
            dirs,
            store,
            shelf: vec![],
            probes: None,
            screen: Screen::Library,
            reader: None,
            loading: None,
            error: None,
            shelf_scroll: Scroller::new(),
            panel_scroll: Scroller::new(),
            labels: TextCache::default(),
            pressed: None,
            down: None,
            scrolling: false,
            save_at: None,
            dirty: true,
            slide: None,
            filter: Filter::All,
            sidebar_hidden: false,
            library_search: {
                let mut f = TextField::new(ui, "Search", MAC_BODY_PT, false, 100);
                f.focused = false;
                f
            },
            whole_words: false,
            notice: None,
            display: DisplayMode::SinglePage,
            import_dir,
            context_book: None,
            clock: store::ReadingClock::default(),
            continuous: false,
            page_scroll: Scroller::new(),
            pitch: 1.0,
        };
        b.rescan();
        b
    }

    /// The surface size changed (rotation).
    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    /// Re-reads the folders; covers fill in as the probe thread finishes each book.
    pub fn rescan(&mut self) {
        let paths = store::library(&self.store, &self.dirs);
        let old: HashMap<PathBuf, Cover> = std::mem::take(&mut self.shelf).into_iter().map(|c| (c.path.clone(), c)).collect();
        let mut todo = vec![];
        self.shelf = paths
            .into_iter()
            .map(|p| match old.get(&p) {
                Some(c) => Cover { path: p, title: c.title.clone(), author: c.author.clone(), image: c.image.clone(), tile: None },
                None => {
                    todo.push(p.clone());
                    Cover { title: engine::file_title(&p), author: String::new(), image: None, tile: None, path: p }
                }
            })
            .collect();
        if !todo.is_empty() {
            let inbox: Inbox<Probed> = Arc::default();
            let job = Arc::clone(&inbox);
            // one pool job probes the new books in order; each cover is delivered as it finishes
            DispatchQueue::global(Qos::Utility).async_(move || {
                for p in todo {
                    let meta = engine::probe(&p);
                    deliver(&job, (p, meta));
                }
            });
            self.probes = Some(inbox);
        }
        self.dirty = true;
    }

    pub(super) fn screen_is_reader(&self) -> bool {
        self.screen == Screen::Reader
    }

    #[cfg(test)]
    /// True while the reader hides the status bar and home indicator.
    pub fn fullscreen(&self) -> bool {
        self.screen == Screen::Reader && self.reader.as_ref().is_some_and(|r| !r.chrome && r.panel.is_none())
    }

    pub fn reading(&self) -> Option<&Path> {
        match self.screen {
            Screen::Reader => self.reader.as_ref().map(|r| r.path.as_path()),
            Screen::Library => None,
        }
    }

    #[cfg(test)]
    pub fn page(&self) -> Option<(usize, usize)> {
        self.reader.as_ref().filter(|_| self.screen == Screen::Reader).map(|r| (r.page, r.layout.pages.len()))
    }

    #[cfg(test)]
    pub fn shelf_titles(&self) -> Vec<&str> {
        self.shelf.iter().map(|c| c.title.as_str()).collect()
    }

    /// The reading settings: text size, theme, line spacing, margins and justification.
    pub fn settings(&self) -> store::Settings {
        self.store.settings
    }

    /// Starts opening `path` (from the shelf, or a Files hand-off). The book
    /// is read on the pool; `tick` swaps the reader in.
    pub fn open_path(&mut self, path: &Path) {
        if self.loading.is_some() {
            return;
        }
        self.library_search.focused = false;
        if !self.shelf.iter().any(|c| c.path == path) {
            self.store.entry_mut(path);
            self.rescan();
        }
        let inbox: Inbox<Result<Book, String>> = Arc::default();
        let job = Arc::clone(&inbox);
        let p = path.to_path_buf();
        DispatchQueue::global(Qos::UserInitiated).async_(move || deliver(&job, engine::load(&p)));
        self.loading = Some(Loading { path: path.to_path_buf(), inbox });
        self.error = None;
        self.dirty = true;
    }

    /// Opens an already parsed book (used by `tick`, and by tests).
    pub(super) fn show_book(&mut self, ui: &Ui, path: PathBuf, book: Book) {
        self.store.touch(&path);
        let entry = self.store.entry(&path).cloned();
        let mut r = Reader {
            path,
            book,
            layout: Layout::default(),
            page: 0,
            chrome: false,
            panel: None,
            search: TextField::new(ui, "Search in book", MAC_BODY_PT, false, 100),
            query: String::new(),
            hits: vec![],
            find_at: None,
            searched: false,
            highlight: None,
            images: HashMap::new(),
            narration: None,
            note: TextField::new(ui, "Add a note", MAC_BODY_PT, false, 500),
            note_target: None,
            selection: None,
        };
        r.search.focused = false;
        r.note.focused = false;
        self.reader = Some(r);
        self.screen = Screen::Reader;
        self.repaginate(ui, entry.map(|e| e.pos).unwrap_or_default());
        self.save_now();
        self.dirty = true;
    }

    // --- pagination ---

    fn metrics(&self, ui: &Ui) -> Metrics {
        let size = SIZES[self.store.settings.size.min(SIZES.len() - 1)];
        // a two-page spread gives each page half the text width, less the gutter between the pages
        let column = self.w as f32 - 2.0 * self.page_margin(ui);
        let width = if self.two_pages() { (column - ui.pt(PAGE_GUTTER_PT)) / 2.0 } else { column };
        Metrics {
            width,
            height: self.h as f32 - ui.pt(page_top_pt()) - ui.pt(FOOTER_PT),
            body: ui.pt(size) * ui.text_scale,
            line_height: self.store.settings.line_height(),
        }
    }

    /// The side margin of the reading column in pixels (the Display panel's Margins).
    pub(super) fn page_margin(&self, ui: &Ui) -> f32 {
        ui.pt(self.store.settings.margin_pt())
    }

    /// Point size to hand `Ui::text`/`measure` for a size in px, so layout
    /// and drawing agree under Dynamic Type.
    pub(super) fn pt_of(ui: &Ui, px: f32) -> f32 {
        px / (ui.s * ui.text_scale)
    }

    /// Rebuilds the pages for the current size and keeps `keep` on screen.
    fn repaginate(&mut self, ui: &Ui, keep: Pos) {
        let m = self.metrics(ui);
        let Some(r) = &mut self.reader else { return };
        // widths by (size, weight) then text, so a lookup borrows the text instead of allocating a key
        let cache = std::cell::RefCell::new(HashMap::<(u32, bool), HashMap<String, f32>>::new());
        let measure = |s: &str, px: f32, bold: bool| -> f32 {
            let style = (px.to_bits(), bold);
            if let Some(w) = cache.borrow().get(&style).and_then(|m| m.get(s)) {
                return *w;
            }
            let w = ui.measure(s, Self::pt_of(ui, px), if bold { springboard::shell::ui::Weight::Bold } else { springboard::shell::ui::Weight::Regular });
            cache.borrow_mut().entry(style).or_default().insert(s.to_string(), w);
            w
        };
        r.layout = layout::paginate(&r.book.blocks, m, &measure);
        r.page = r.layout.page_of(keep);
        r.images.clear();
        r.highlight = r.highlight.filter(|_| r.panel.is_none());
        // the selection's character ranges belong to the old pages
        r.selection = None;
        self.labels.clear();
        self.fit_page_scroll(ui);
        self.sync_scroll();
        self.dirty = true;
    }

    fn progress(r: &Reader) -> u16 {
        let blocks = r.book.blocks.len().max(1);
        let at = r.layout.pos_of(r.page).block;
        ((at * 1000) / blocks).min(1000) as u16
    }

    fn remember(&mut self, t: f64) {
        let Some(r) = &self.reader else { return };
        let pos = r.layout.pos_of(r.page);
        let progress = Self::progress(r);
        let path = r.path.clone();
        self.store.set_pos(&path, pos, progress);
        self.save_at = Some(t + SAVE_DELAY_MS);
    }

    fn save_now(&mut self) {
        self.store.save();
        self.save_at = None;
    }

    /// Writes the reading position and settings (before power-off or close).
    pub fn flush(&mut self) {
        if self.reader.is_some() {
            self.remember(0.0);
        }
        self.save_now();
    }

    /// Turns to page `p` (clamped). In continuous scrolling the window scrolls to the top of the page.
    pub(super) fn goto_page(&mut self, p: usize, t: f64) {
        self.set_page(p, t);
        self.sync_scroll();
    }

    /// Makes page `p` (clamped) the current one without moving the scroll position of continuous scrolling: the reading
    /// position is remembered, and a book whose last page is reached is marked Finished.
    fn set_page(&mut self, p: usize, t: f64) {
        let Some(r) = &mut self.reader else { return };
        let p = p.min(r.layout.pages.len().saturating_sub(1));
        let mut finished = None;
        if p != r.page {
            r.page = p;
            r.highlight = None;
            r.selection = None;
            // the page, or the spread it opens, reaches the last page of a book that has more than one
            if r.layout.pages.len() > 1 && p + self.display.pages_per_row() >= r.layout.pages.len() {
                finished = Some(r.path.clone());
            }
        }
        if let Some(path) = finished {
            self.store.set_status(&path, Status::Finished);
        }
        self.remember(t);
        self.dirty = true;
    }

    fn goto_pos(&mut self, pos: Pos, t: f64) {
        let Some(r) = &self.reader else { return };
        let p = r.layout.page_of(pos);
        self.goto_page(p, t);
    }

    fn turn(&mut self, delta: i32, t: f64) {
        // a spread turns by its own page count, so the next turn lands on the next spread's left page
        let delta = delta * self.display.pages_per_row() as i32;
        let Some(r) = &self.reader else { return };
        let old = r.page;
        let p = (r.page as i32 + delta).clamp(0, r.layout.pages.len() as i32 - 1);
        self.goto_page(p as usize, t);
        // a spread changes at once; only a single page slides
        if p as usize != old && !self.two_pages() && !self.continuous {
            self.slide = Some((old, delta.signum(), t));
        }
    }

    /// View menu: whether the reader shows two pages side by side.
    pub fn two_pages(&self) -> bool {
        self.display == DisplayMode::TwoUp
    }

    /// View menu: one page or two pages side by side. The text is paginated again for the column width, and the reader
    /// keeps the place it was on.
    pub fn set_two_pages(&mut self, ui: &Ui, on: bool) {
        let mode = if on { DisplayMode::TwoUp } else { DisplayMode::SinglePage };
        if self.display == mode {
            return;
        }
        self.display = mode;
        // a spread is turned, not scrolled
        self.continuous = self.continuous && !on;
        self.slide = None;
        let keep = self.reader.as_ref().map(|r| r.layout.pos_of(r.page));
        if let Some(keep) = keep {
            self.repaginate(ui, keep);
        }
        self.dirty = true;
    }

    /// Select Page: selects the text on the screen (both pages of a spread). False when the page has no text.
    pub fn select_page(&mut self) -> bool {
        let spread = self.display.pages_per_row() - 1;
        let texts = match &self.reader {
            Some(r) => document_text(&r.layout),
            None => return false,
        };
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let doc = pdfselect::DocumentText::new(&refs);
        if doc.page_count() == 0 {
            return false;
        }
        let Some(r) = &mut self.reader else { return false };
        let last = (r.page + spread).min(doc.page_count() - 1);
        let mut selection = pdfselect::Selection::new();
        for page in r.page..=last {
            let len = doc.number_of_characters(page).unwrap_or(0);
            if let Some(part) = pdfselect::Selection::between(&doc, (page, 0), (page, len)) {
                selection.add(&part);
            }
        }
        let found = !selection.is_empty();
        r.selection = found.then_some(selection);
        self.dirty = true;
        found
    }

    /// File > Print for a book that is not a PDF: the job name and the text of the open book (see `book_text`). None when
    /// no book is open or it has no text.
    pub fn print_text(&self) -> Option<(String, String)> {
        let r = self.reader.as_ref()?;
        let text = book_text(&r.book.blocks);
        (!text.is_empty()).then(|| (r.book.title.clone(), text))
    }

    /// Whether a selection is on the page (Find menu: Copy Selection).
    pub fn has_selection(&self) -> bool {
        self.reader.as_ref().is_some_and(|r| r.selection.is_some())
    }

    /// The selected text, for the general board. None when nothing is selected.
    pub fn selection_text(&self) -> Option<String> {
        let r = self.reader.as_ref()?;
        let selection = r.selection.as_ref()?;
        let texts = document_text(&r.layout);
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let doc = pdfselect::DocumentText::new(&refs);
        let text = selection.string(&doc);
        (!text.is_empty()).then_some(text)
    }

    /// For a PDF book, the saved highlights of page `page_no` as highlight annotations (`pdfannot`): one per highlighted
    /// run of a line, with its bounds in px from the text column's left edge and the line's top, and the highlight's
    /// note as its contents. None for other formats, which keep drawing their highlights as text spans.
    #[cfg(test)]
    pub(super) fn pdf_highlights(&self, ui: &Ui, page_no: usize) -> Option<pdfannot::PageAnnotations> {
        let marks = self.pdf_highlight_marks(ui, page_no)?;
        let mut all = pdfannot::PageAnnotations::new();
        for (mark, _) in marks {
            all.add(mark);
        }
        Some(all)
    }

    /// The annotations of `pdf_highlights`, each with the style of its highlight (an index into
    /// `store::HIGHLIGHT_STYLES`), in the order the page draws them.
    pub(super) fn pdf_highlight_marks(&self, ui: &Ui, page_no: usize) -> Option<Vec<(pdfannot::Annotation, u8)>> {
        let r = self.reader.as_ref()?;
        if !r.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            return None;
        }
        let page = r.layout.pages.get(page_no)?;
        let spans: Vec<Span> = self.store.entry(&r.path).map_or_else(Vec::new, |e| e.highlights.clone());
        let m = self.metrics(ui);
        let body_pt = Self::pt_of(ui, m.body);
        let mut marks = Vec::new();
        for line in &page.lines {
            let (pt, weight) = match line.kind {
                layout::Kind::Heading(level) => (body_pt * layout::heading_scale(level), springboard::shell::ui::Weight::Bold),
                layout::Kind::Image { .. } => continue,
                layout::Kind::Body => (body_pt, springboard::shell::ui::Weight::Regular),
            };
            let text = r.book.blocks.get(line.block).and_then(|b| b.text()).unwrap_or("");
            let lh = ui.pt(pt) * ui.text_scale * m.line_height;
            let extra = line_extra(ui, self.store.settings.justify, text, line, pt, m.width);
            for span in spans.iter().filter(|sp| sp.block == line.block) {
                let (a, b) = layout::line_cols(text, line.offset, &line.text, span.offset, span.offset + span.len);
                if a >= b {
                    continue;
                }
                let prefix: String = line.text.chars().take(a).collect();
                let word: String = line.text.chars().skip(a).take(b - a).collect();
                let x = ui.measure(&prefix, pt, weight) + extra * prefix.matches(' ').count() as f32;
                let w = ui.measure(&word, pt, weight) + extra * word.matches(' ').count() as f32;
                if let Ok(mut mark) = pdfannot::Annotation::new(pdfannot::Subtype::Highlight, Rect { x, y: line.y, w, h: lh }) {
                    mark.set_highlighted(true);
                    mark.set_contents(&self.store.note_of(&r.path, *span).unwrap_or_default());
                    marks.push((mark, self.store.highlight_style(&r.path, *span)));
                }
            }
        }
        Some(marks)
    }

    /// Bookmarks that sit on the page `page`.
    pub(super) fn page_marks(&self, page: usize) -> Vec<Pos> {
        let Some(r) = &self.reader else { return vec![] };
        self.store.entry(&r.path).map_or(vec![], |e| e.bookmarks.iter().copied().filter(|&b| r.layout.page_of(b) == page).collect())
    }

    fn toggle_mark(&mut self) {
        let Some(r) = &self.reader else { return };
        let (path, page) = (r.path.clone(), r.page);
        let start = r.layout.pos_of(page);
        let marks = self.page_marks(page);
        if marks.is_empty() {
            self.store.toggle_bookmark(&path, start);
        } else {
            for m in marks {
                self.store.toggle_bookmark(&path, m);
            }
        }
        self.save_now();
    }

    fn close_reader(&mut self, t: f64) {
        self.notice = None;
        self.credit_reading(t);
        self.remember(t);
        self.save_now();
        self.reader = None;
        self.screen = Screen::Library;
        self.pressed = None;
        // the order and progress changed
        self.rescan();
    }

    fn change_settings(&mut self, ui: &Ui, size: usize, theme: Theme) {
        self.apply_settings(ui, store::Settings { size, theme, ..self.store.settings });
    }

    /// Takes new reading settings: the pages are laid out again when the size, the line spacing or the margins changed,
    /// and the reader keeps its place.
    fn apply_settings(&mut self, ui: &Ui, settings: store::Settings) {
        let keep = self.reader.as_ref().map(|r| r.layout.pos_of(r.page)).unwrap_or_default();
        let old = self.store.settings;
        let relayout = (settings.size, settings.spacing, settings.margin) != (old.size, old.spacing, old.margin);
        self.store.settings = settings;
        self.save_now();
        if relayout {
            self.repaginate(ui, keep);
        } else {
            self.labels.clear();
        }
        self.dirty = true;
    }

    /// Display panel: the line spacing, an index into `store::SPACINGS`.
    pub fn set_line_spacing(&mut self, ui: &Ui, spacing: usize) {
        let settings = self.store.settings;
        self.apply_settings(ui, store::Settings { spacing: spacing.min(store::SPACINGS.len() - 1), ..settings });
    }

    /// Display panel: the side margins, an index into `store::MARGINS`.
    pub fn set_margins(&mut self, ui: &Ui, margin: usize) {
        let settings = self.store.settings;
        self.apply_settings(ui, store::Settings { margin: margin.min(store::MARGINS.len() - 1), ..settings });
    }

    /// Display panel: whether body text is set flush on both sides.
    pub fn set_justified(&mut self, ui: &Ui, on: bool) {
        let settings = self.store.settings;
        self.apply_settings(ui, store::Settings { justify: on, ..settings });
    }

    // --- search ---

    fn run_search(&mut self) {
        let whole = self.whole_words;
        let Some(r) = &mut self.reader else { return };
        r.query = r.search.text().trim().to_string();
        r.hits = r.book.search(&r.query, SEARCH_LIMIT);
        if whole {
            r.hits = keep_whole_words(&r.book.blocks, &r.query, std::mem::take(&mut r.hits));
        }
        r.find_at = None;
        r.searched = true;
        self.panel_scroll.jump_top();
        self.dirty = true;
    }

    /// Find menu: match whole words only (or any part of a word). Searches again when a search is open.
    pub fn set_whole_words(&mut self, on: bool) {
        self.whole_words = on;
        if self.reader.as_ref().is_some_and(|r| r.searched) {
            self.run_search();
        }
    }

    pub fn whole_words(&self) -> bool {
        self.whole_words
    }

    /// Whether the open book's search has matches (Find Next and Find Previous are enabled).
    pub fn has_hits(&self) -> bool {
        self.reader.as_ref().is_some_and(|r| !r.hits.is_empty())
    }

    /// Find menu: Find Next (`delta` 1) or Find Previous (-1) through the open search's hits.
    pub fn find_step(&mut self, delta: i32, t: f64) {
        let Some(r) = &mut self.reader else { return };
        let Some(i) = step_index(r.find_at, delta, r.hits.len()) else { return };
        r.find_at = Some(i);
        self.open_hit(i, t);
    }

    /// The text the last find went to, as a highlight span, and whether it is already highlighted.
    pub fn match_span(&self) -> Option<(Span, bool)> {
        let r = self.reader.as_ref()?;
        let (block, offset, len) = r.highlight?;
        let span = Span { block, offset, len };
        let saved = self.store.entry(&r.path).is_some_and(|e| e.highlights.contains(&span));
        Some((span, saved))
    }

    /// Find menu: highlights the found text, or removes its highlight. True when it is now highlighted.
    /// Find menu: writes a note on the match the last find went to. The match is highlighted first if it is not;
    /// the Notes panel opens with the note field focused, showing the note the highlight already has.
    pub fn add_note(&mut self, ui: &Ui) {
        let Some((span, saved)) = self.match_span() else { return };
        let Some(path) = self.reader.as_ref().map(|r| r.path.clone()) else { return };
        if !saved {
            self.store.toggle_highlight(&path, span);
        }
        let existing = self.store.note_of(&path, span).unwrap_or_default();
        if let Some(r) = &mut self.reader {
            r.panel = Some(Panel::Notes);
            r.search.focused = false;
            r.note_target = Some(span);
            r.note.set_text(ui, &existing);
            r.note.focused = true;
        }
        self.panel_scroll.jump_top();
        self.save_now();
        self.dirty = true;
    }

    /// Ends the note field's edit and writes its text as the note of the highlight it edits.
    fn save_note(&mut self) {
        let Some(r) = &mut self.reader else { return };
        r.note.focused = false;
        let Some(span) = r.note_target else { return };
        let text = r.note.text().to_string();
        let path = r.path.clone();
        self.store.set_note(&path, span, &text);
        self.save_now();
        self.dirty = true;
    }

    /// The notes of the open book, in reading order.
    pub(super) fn notes_list(&self) -> Vec<(Span, String)> {
        let Some(r) = &self.reader else { return vec![] };
        self.store.entry(&r.path).map_or_else(Vec::new, |e| e.notes.clone())
    }

    pub fn toggle_match_highlight(&mut self) -> bool {
        let Some((span, _)) = self.match_span() else { return false };
        let Some(path) = self.reader.as_ref().map(|r| r.path.clone()) else { return false };
        let on = self.store.toggle_highlight(&path, span);
        self.save_now();
        self.dirty = true;
        on
    }

    /// Edit menu: the quotation for the match the last find went to, with its citation (see `quote`).
    pub fn quotation(&self) -> Option<String> {
        let r = self.reader.as_ref()?;
        let (block, offset, len) = r.highlight?;
        quote::quotation(&r.book, Span { block, offset, len })
    }

    // --- read aloud, look up, translate ---

    /// The word the last find went to and the sentence it sits in (Look Up and Translate work on these).
    fn match_text(&self) -> Option<(String, String)> {
        let r = self.reader.as_ref()?;
        let (block, offset, len) = r.highlight?;
        let word: String = r.book.blocks.get(block)?.text()?.chars().skip(offset).take(len).collect();
        let sentence = quote::sentence(&r.book.blocks, Span { block, offset, len }).unwrap_or_else(|| word.clone());
        Some((word, sentence))
    }

    /// Find menu: the state of Read Aloud. None when nothing is being read, otherwise whether it is paused.
    pub fn speaking(&self) -> Option<bool> {
        self.reader.as_ref()?.narration.as_ref().map(|n| n.is_paused())
    }

    /// Find menu: reads the chapter aloud from the current page to its end. Pauses or resumes a reading in progress.
    pub fn read_aloud(&mut self, t: f64) {
        let Some(r) = &mut self.reader else { return };
        if let Some(n) = &mut r.narration {
            if n.is_paused() {
                n.resume(t);
            } else {
                n.pause(t);
            }
            self.dirty = true;
            return;
        }
        let pos = r.layout.pos_of(r.page);
        let starts: Vec<usize> = r.book.toc.iter().map(|e| e.block).collect();
        let end = narrate::chapter_end(&starts, pos.block, r.book.blocks.len());
        let mut sentences = Vec::new();
        for (block, b) in r.book.blocks.iter().enumerate().take(end).skip(pos.block) {
            let Some(text) = b.text() else { continue };
            let from = if block == pos.block { pos.offset } else { 0 };
            sentences.extend(narrate::sentence_spans(text, block, from));
        }
        match narrate::Narration::start(sentences, t) {
            Some(n) => r.narration = Some(n),
            None => self.notice = Some("Nothing to read from this page to the end of the chapter.".to_string()),
        }
        self.dirty = true;
    }

    /// Find menu: stops reading aloud.
    pub fn stop_reading(&mut self) {
        if let Some(r) = &mut self.reader {
            r.narration = None;
        }
        self.dirty = true;
    }

    /// Find menu: Look Up for the text the last find went to. The result is a notice on the page.
    pub fn look_up(&mut self) {
        let Some((word, sentence)) = self.match_text() else { return };
        let language = bifrost_app::controls::language::dominant_language(&sentence);
        self.notice = Some(narrate::look_up_notice(&word, language));
        self.dirty = true;
    }

    /// Find menu: Translate the sentence the last find went to. The result is a notice on the page.
    pub fn translate(&mut self) {
        let Some((_, sentence)) = self.match_text() else { return };
        let language = bifrost_app::controls::language::dominant_language(&sentence);
        self.notice = Some(narrate::translate_notice(language));
        self.dirty = true;
    }

    /// Moves the reading aloud on once its sentence is due, turning to the page that sentence is on.
    fn follow_narration(&mut self, t: f64) -> bool {
        let Some(r) = &mut self.reader else { return false };
        let Some(n) = &mut r.narration else { return false };
        let step = n.tick(t);
        let mut turned = false;
        match step {
            narrate::Step::Same => return false,
            narrate::Step::Done => r.narration = None,
            narrate::Step::Moved(span) => {
                let p = r.layout.page_of(Pos { block: span.block, offset: span.offset });
                if p != r.page {
                    r.page = p;
                    turned = true;
                }
            }
        }
        if turned {
            self.remember(t);
            self.sync_scroll();
        }
        self.dirty = true;
        true
    }

    /// The shelf as the search index sees it: each book's path, title and author.
    pub fn index_entries(&self) -> Vec<search_index::Entry> {
        self.shelf
            .iter()
            .map(|c| {
                let pdf = c.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
                search_index::Entry { path: c.path.clone(), title: c.title.clone(), author: c.author.clone(), format: if pdf { "PDF" } else { "EPUB" } }
            })
            .collect()
    }

    fn open_hit(&mut self, i: usize, t: f64) {
        let Some(r) = &mut self.reader else { return };
        let Some(h) = r.hits.get(i).cloned() else { return };
        let len = r.query.chars().count();
        r.highlight = Some((h.block, h.offset, len));
        r.panel = None;
        let p = r.layout.page_of(Pos { block: h.block, offset: h.offset });
        // goto_page clears the highlight only when the page changes
        let keep = r.highlight;
        self.goto_page(p, t);
        if let Some(r) = &mut self.reader {
            r.highlight = keep;
        }
    }

    // --- reading status, collections, sorting, goals, importing ---

    /// Counts the time since the last call towards today's reading goal while a book is open.
    fn credit_reading(&mut self, t: f64) {
        let reading = self.screen == Screen::Reader && self.reader.is_some();
        let secs = self.clock.advance(t, reading);
        if secs > 0 {
            self.store.add_reading(store::today(), secs);
            self.save_at.get_or_insert(t + SAVE_DELAY_MS);
        }
    }

    /// The reading goal in minutes a day; 0 = none.
    pub fn reading_goal(&self) -> u32 {
        self.store.library.goal
    }

    /// Library menu: sets the daily reading goal in minutes (0 = no goal).
    pub fn set_reading_goal(&mut self, minutes: u32) {
        self.store.library.goal = minutes;
        self.save_now();
        self.dirty = true;
    }

    /// The goal line of the Reading Now section: today's minutes against the goal, and the days in a row the goal
    /// was met. None without a goal.
    pub(super) fn goal_text(&self) -> Option<String> {
        let today = store::today();
        let label = store::goal_label(self.store.seconds_on(today), self.store.library.goal)?;
        Some(match self.store.goal_streak(today) {
            n if n >= 2 => format!("{label}, {n} days in a row"),
            _ => label,
        })
    }

    /// The standing of the open book (Library menu: Want to Read, Finished).
    pub fn open_status(&self) -> Status {
        self.reading().map_or(Status::None, |p| self.store.status_of(p))
    }

    /// Library menu: marks the open book Want to Read or Finished; choosing the same again takes it back.
    pub fn toggle_open_status(&mut self, status: Status) {
        let Some(path) = self.reading().map(Path::to_path_buf) else { return };
        self.toggle_status_of(&path, status);
    }

    fn toggle_status_of(&mut self, path: &Path, status: Status) {
        self.store.toggle_status(path, status);
        self.save_now();
        self.dirty = true;
    }

    /// The collections as the Add to Collection menu lists them: each name, and whether the open book is in it.
    pub fn collection_menu(&self) -> Vec<(String, bool)> {
        let open = self.reading();
        self.store.collections.iter().enumerate().map(|(i, c)| (c.name.clone(), open.is_some_and(|p| self.store.in_collection(i, p)))).collect()
    }

    /// Library menu: puts the open book in collection `i`, or takes it out.
    pub fn toggle_open_in_collection(&mut self, i: usize) {
        let Some(path) = self.reading().map(Path::to_path_buf) else { return };
        self.toggle_in_collection(i, &path);
    }

    fn toggle_in_collection(&mut self, i: usize, path: &Path) {
        self.store.toggle_in_collection(i, path);
        self.save_now();
        self.dirty = true;
    }

    /// The collection the sidebar shows, if it shows one.
    pub fn current_collection(&self) -> Option<usize> {
        match self.filter {
            Filter::Collection(i) if i < self.store.collections.len() => Some(i),
            _ => None,
        }
    }

    /// Whether the library search field has text (Rename Collection takes its name from it).
    pub fn has_search_text(&self) -> bool {
        !self.library_search.text().trim().is_empty()
    }

    /// Library menu: adds an empty collection and shows it. It is named after the library search text when there is
    /// some, otherwise "New Collection"; a name that is taken gets a number.
    pub fn new_collection(&mut self, ui: &Ui) {
        let name = self.store.unique_collection_name(self.library_search.text());
        if let Some(i) = self.store.create_collection(&name) {
            self.filter = Filter::Collection(i);
            self.library_search.set_text(ui, "");
            self.shelf_scroll.jump_top();
            self.save_now();
            self.dirty = true;
        }
    }

    /// Library menu: renames the shown collection to the library search text. False when nothing was renamed (no
    /// collection is shown, the text is empty, or another collection has the name).
    pub fn rename_collection(&mut self, ui: &Ui) -> bool {
        let Some(i) = self.current_collection() else { return false };
        let name = self.library_search.text().trim().to_string();
        if !self.store.rename_collection(i, &name) {
            return false;
        }
        self.library_search.set_text(ui, "");
        self.save_now();
        self.dirty = true;
        true
    }

    /// Library menu: deletes the shown collection. Its books stay in the library.
    pub fn delete_collection(&mut self) {
        let Some(i) = self.current_collection() else { return };
        self.store.delete_collection(i);
        self.filter = Filter::All;
        self.shelf_scroll.jump_top();
        self.save_now();
        self.dirty = true;
    }

    /// The name of a sidebar row: the fixed names, or the reader's own name for a collection.
    pub(super) fn filter_name(&self, f: Filter) -> String {
        match f {
            Filter::Collection(i) => self.store.collections.get(i).map_or_else(|| f.name().to_string(), |c| c.name.clone()),
            other => other.name().to_string(),
        }
    }

    /// What the shelf says when the shown collection has no books.
    pub(super) fn empty_title(&self) -> String {
        match self.filter {
            Filter::Collection(_) => "Empty Collection".to_string(),
            Filter::WantToRead => "No Books to Read".to_string(),
            Filter::Finished => "No Finished Books".to_string(),
            f => format!("No {}", f.name()),
        }
    }

    /// The sidebar rows in order: the fixed ones, then the reader's collections.
    fn sidebar_filters(&self) -> Vec<Filter> {
        let mut rows = Filter::ALL.to_vec();
        rows.extend((0..self.store.collections.len()).map(Filter::Collection));
        rows
    }

    pub fn sort_by(&self) -> SortBy {
        self.store.library.sort
    }

    /// View menu: Sort By.
    pub fn set_sort(&mut self, sort: SortBy) {
        if self.store.library.sort == sort {
            return;
        }
        self.store.library.sort = sort;
        self.shelf_scroll.jump_top();
        self.save_now();
        self.dirty = true;
    }

    pub fn view_mode(&self) -> ViewMode {
        self.store.library.view
    }

    /// View menu: the library as a grid of covers or as a list.
    pub fn set_view_mode(&mut self, view: ViewMode) {
        if self.store.library.view == view {
            return;
        }
        self.store.library.view = view;
        // the scaled covers were made for the old tile size
        for c in &mut self.shelf {
            c.tile = None;
        }
        self.shelf_scroll.jump_top();
        self.save_now();
        self.dirty = true;
    }

    /// Find menu: the style of the highlight on the text the last find went to (an index into
    /// `store::HIGHLIGHT_STYLES`), or None when that text is not highlighted.
    pub fn match_style(&self) -> Option<u8> {
        let (span, saved) = self.match_span()?;
        let r = self.reader.as_ref()?;
        saved.then(|| self.store.highlight_style(&r.path, span))
    }

    /// Find menu: highlights the text the last find went to in a colour, or underlines it. False without a match.
    pub fn set_match_style(&mut self, style: u8) -> bool {
        let Some((span, _)) = self.match_span() else { return false };
        let Some(path) = self.reader.as_ref().map(|r| r.path.clone()) else { return false };
        self.store.set_highlight_style(&path, span, style);
        self.save_now();
        self.dirty = true;
        true
    }

    /// Copies a book that arrived as a stream (File > Add to Library, a shared PDF) into the Books folder and puts it
    /// on the shelf. The path of the copy.
    pub fn import_book(&mut self, name: &str, source: &mut dyn std::io::Read) -> Result<PathBuf, String> {
        let dest = store::import_into(&self.import_dir, name, source)?;
        self.rescan();
        Ok(dest)
    }

    /// Shows an error banner in the library.
    pub fn show_error(&mut self, message: String) {
        self.error = Some(message);
        self.dirty = true;
    }

    /// The menu of a right click on a cover in the library: Open, Want to Read, Finished and the collections. The book
    /// it was opened on is kept for `context_action`. Empty when the click is not on a cover.
    pub fn context_items(&mut self, ui: &Ui, x: f32, y: f32) -> Vec<ContextItem> {
        self.context_book = None;
        if self.screen != Screen::Library {
            return Vec::new();
        }
        let Some(Target::Cover(i)) = self.hit(ui, x, y) else { return Vec::new() };
        let Some(path) = self.shelf.get(i).map(|c| c.path.clone()) else { return Vec::new() };
        let status = self.store.status_of(&path);
        let item = |id: u32, label: String| ContextItem { id, label, enabled: true };
        let mut items = vec![
            item(CTX_OPEN, "Open".to_string()),
            item(CTX_WANT, if status == Status::WantToRead { "Remove from Want to Read" } else { "Add to Want to Read" }.to_string()),
            item(CTX_FINISHED, if status == Status::Finished { "Mark as Unfinished" } else { "Mark as Finished" }.to_string()),
        ];
        for (k, c) in self.store.collections.iter().enumerate().take(MAX_COLLECTION_ITEMS) {
            let verb = if self.store.in_collection(k, &path) { "Remove from" } else { "Add to" };
            items.push(item(CTX_COLLECTION_BASE + k as u32, format!("{verb} {}", c.name)));
        }
        self.context_book = Some(path);
        items
    }

    /// The user chose item `id` of the menu `context_items` made. True when a frame is needed.
    pub fn context_action(&mut self, id: u32) -> bool {
        let Some(path) = self.context_book.take() else { return false };
        match id {
            CTX_OPEN => self.open_path(&path),
            CTX_WANT => self.toggle_status_of(&path, Status::WantToRead),
            CTX_FINISHED => self.toggle_status_of(&path, Status::Finished),
            k if k >= CTX_COLLECTION_BASE => self.toggle_in_collection((k - CTX_COLLECTION_BASE) as usize, &path),
            _ => return false,
        }
        self.dirty = true;
        true
    }

    // --- geometry ---

    fn nav_bottom(&self, ui: &Ui) -> f32 {
        ui.pt(nav_bottom_pt())
    }

    pub(super) fn panel_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let top = self.nav_bottom(ui);
        (0.0, top, (self.w as f32).min(ui.pt(PANEL_PT)), self.h as f32 - top)
    }

    pub(super) fn bottom_bar_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let hz = home_zone(ui);
        let bh = ui.pt(BOTTOM_BAR_PT);
        (0.0, self.h as f32 - hz - bh, self.w as f32, bh + hz)
    }

    /// The seek track inside the bottom bar.
    pub(super) fn slider_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (_, by, _, _) = self.bottom_bar_rect(ui);
        let m = ui.pt(MARGIN_PT);
        (m, by + ui.pt(46.0), self.w as f32 - 2.0 * m, ui.pt(SPACE_XXXL))
    }

    // --- sidebar and toolbar (points) ---

    /// Width of the library sidebar in pixels; the reader has none.
    pub(super) fn side(&self, ui: &Ui) -> f32 {
        if self.screen == Screen::Library && !self.sidebar_hidden { ui.pt(SIDEBAR_PT) } else { 0.0 }
    }

    pub(super) fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: SIDEBAR_PT, h: self.h as f32 / ui.pt(1.0) }
    }

    pub(super) fn sidebar(&self) -> SourceList {
        let filters = self.sidebar_filters();
        let mut items = vec![SidebarItem::header("Library")];
        for (i, f) in filters.iter().enumerate() {
            if i == Filter::COLLECTIONS_AT {
                items.push(SidebarItem::header("Collections"));
            }
            if i == Filter::ALL.len() {
                items.push(SidebarItem::header("My Collections"));
            }
            items.push(SidebarItem::row(ROW_BASE + i as u32, &self.filter_name(*f)).with_symbol(f.symbol()).with_count(self.matching(*f).len() as u32));
        }
        let mut list = SourceList::new(items, 32.0, 30.0);
        list.set_selected(filters.iter().position(|&f| f == self.filter).map(|i| ROW_BASE + i as u32));
        list
    }

    /// The title shown in the toolbar.
    pub(super) fn title(&self) -> String {
        match (&self.screen, &self.reader) {
            (Screen::Reader, Some(r)) => r.book.title.clone(),
            _ => self.filter_name(self.filter),
        }
    }

    fn title_w(&self, ui: &Ui) -> f32 {
        (ui.measure(&self.title(), MAC_TITLE3_PT, springboard::shell::ui::Weight::Bold) / ui.pt(1.0) + 4.0).clamp(40.0, (self.bar_bounds(ui).w * 0.4).max(40.0))
    }

    pub(super) fn bar_bounds(&self, ui: &Ui) -> Rect {
        let side = self.side(ui) / ui.pt(1.0);
        let x = if side > 0.0 { side + 12.0 } else { traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT };
        let h = widgets::CAPSULE_PT;
        let w = self.w as f32 / ui.pt(1.0);
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (w - x - 16.0).max(0.0), h }
    }

    /// The unified toolbar: the sidebar button and the collection in the library; in the reader (Books for Mac)
    /// contents and bookmark at the leading edge, the book title centred, search, display and close at the trailing edge.
    pub(super) fn bar(&self, ui: &Ui) -> Toolbar {
        let title = ToolbarItem::custom(BAR_TITLE, "Title", self.title_w(ui));
        match (&self.screen, &self.reader) {
            (Screen::Reader, Some(r)) => {
                let marked = !self.page_marks(r.page).is_empty();
                let panel = r.panel;
                Toolbar::new(vec![
                    ToolbarItem::icon(BAR_CONTENTS, "Contents", "list.bullet").in_group(1).selected(matches!(panel, Some(Panel::Contents | Panel::Bookmarks | Panel::Pages | Panel::Notes))),
                    ToolbarItem::icon(BAR_MARK, if marked { "Remove Bookmark" } else { "Bookmark" }, "tag").in_group(1).selected(marked),
                    ToolbarItem::flexible_space(),
                    title,
                    ToolbarItem::flexible_space(),
                    ToolbarItem::icon(BAR_SEARCH, "Search", "magnifyingglass").in_group(2).selected(panel == Some(Panel::Search)),
                    ToolbarItem::button(BAR_DISPLAY, "Aa").in_group(2).selected(panel == Some(Panel::Display)),
                    ToolbarItem::icon(BAR_CLOSE, "Close Book", "xmark.circle.fill"),
                ])
            }
            _ => Toolbar::new(vec![
                ToolbarItem::icon(BAR_SIDEBAR, "Show Sidebar", "sidebar.left").selected(!self.sidebar_hidden),
                title,
                ToolbarItem::flexible_space(),
                ToolbarItem::custom(BAR_LIBRARY_SEARCH, "Search", LIBRARY_SEARCH_PT),
            ]),
        }
    }

    /// Lays the bar out exactly as `widgets::toolbar` draws it.
    pub(super) fn bar_layout(&self, ui: &Ui) -> ToolbarLayout {
        let b = self.bar_bounds(ui);
        let h = widgets::CAPSULE_PT.min(b.h);
        let inner = Rect { x: b.x, y: b.y + (b.h - h) / 2.0, w: b.w, h };
        let width = |it: &ToolbarItem| {
            if it.symbol.is_some() { widgets::CAPSULE_PT } else { (ui.measure(&it.label, MAC_BODY_PT, springboard::shell::ui::Weight::Regular) / ui.pt(1.0) + 2.0 * SPACE_S * 1.5).max(widgets::CAPSULE_PT) }
        };
        self.bar(ui).layout_with(inner, width, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT)
    }

    /// Every visible toolbar button as a semantic control (points), so the shell does not drag the window over it.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let bar = self.bar(ui);
        self.bar_layout(ui)
            .visible
            .iter()
            .filter(|(id, _)| *id != BAR_TITLE)
            .filter_map(|(id, r)| {
                let i = bar.items.iter().find(|i| i.id == *id)?;
                let (role, value) = if *id == BAR_LIBRARY_SEARCH { (Role::TextField, self.library_search.text().to_string()) } else { (Role::Button, String::new()) };
                Some(Node { id: *id as u64, role, label: i.label.clone(), value, bounds: *r, enabled: i.enabled })
            })
            .collect()
    }

    /// The library search field inside its toolbar slot (points).
    pub(super) fn library_search_rect(&self, ui: &Ui) -> Option<Rect> {
        let r = self.bar_layout(ui).rect_of(BAR_LIBRARY_SEARCH)?;
        Some(Rect { x: r.x, y: r.y + 4.0, w: r.w, h: r.h - 8.0 })
    }

    fn bar_px(&self, ui: &Ui, id: u32) -> (f32, f32, f32, f32) {
        let k = ui.pt(1.0);
        self.bar_layout(ui).rect_of(id).map_or((0.0, 0.0, 0.0, 0.0), |r| (r.x * k, r.y * k, r.w * k, r.h * k))
    }

    /// The reader's toolbar buttons in pixels.
    pub(super) fn nav_buttons(&self, ui: &Ui) -> Vec<(Target, &'static str, (f32, f32, f32, f32))> {
        [(Target::NavContents, "Contents", BAR_CONTENTS), (Target::NavMark, "Bookmark", BAR_MARK), (Target::NavSearch, "Search", BAR_SEARCH), (Target::NavDisplay, "Aa", BAR_DISPLAY)]
            .into_iter()
            .map(|(t, l, id)| (t, l, self.bar_px(ui, id)))
            .collect()
    }

    /// The Close Book button (back to the library), in pixels.
    pub(super) fn back_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        self.bar_px(ui, BAR_CLOSE)
    }

    /// Shelf indices of the books the selected collection shows.
    pub(super) fn visible(&self) -> Vec<usize> {
        self.visible_of(self.filter)
    }

    fn visible_of(&self, f: Filter) -> Vec<usize> {
        let mut shown = self.matching(f);
        let sort = self.store.library.sort;
        // the shelf is already in the order of last opening; the other orders sort the books shown
        if sort != SortBy::Recent {
            shown.sort_by_cached_key(|&i| {
                let cover = &self.shelf[i];
                let progress = self.store.entry(&cover.path).map_or(0, |e| e.progress);
                store::sort_key(sort, &cover.title, &cover.author, progress)
            });
        }
        shown
    }

    /// Shelf indices of the books `f` holds that match the library search, in shelf order.
    fn matching(&self, f: Filter) -> Vec<usize> {
        let pdf = |c: &Cover| c.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
        let query = self.library_search.text().trim().to_lowercase();
        let matches = |c: &Cover| query.is_empty() || c.title.to_lowercase().contains(&query) || c.author.to_lowercase().contains(&query);
        (0..self.shelf.len())
            .filter(|&i| {
                let c = &self.shelf[i];
                if !matches(c) {
                    return false;
                }
                match f {
                    Filter::All => true,
                    Filter::Reading => self.store.entry(&c.path).is_some_and(|e| e.opened > 0),
                    Filter::Books => !pdf(c),
                    Filter::Pdfs => pdf(c),
                    Filter::WantToRead => self.store.status_of(&c.path) == Status::WantToRead,
                    Filter::Finished => self.store.status_of(&c.path) == Status::Finished,
                    Filter::Collection(k) => self.store.in_collection(k, &c.path),
                }
            })
            .collect()
    }

    /// Whether the library is a list of rows rather than a grid of covers.
    pub(super) fn list_view(&self) -> bool {
        self.store.library.view == ViewMode::List
    }

    /// Screen rect of the `k`th book the selected collection shows when the library is a list.
    pub(super) fn list_row_rect(&self, ui: &Ui, k: usize) -> (f32, f32, f32, f32) {
        let m = ui.pt(MARGIN_PT);
        let x = self.side(ui) + m;
        (x, self.grid_top(ui) - self.shelf_scroll.offset() + k as f32 * ui.pt(LIST_ROW_PT), self.w as f32 - x - m, ui.pt(LIST_ROW_PT))
    }

    /// Screen rect of the `k`th book shown, in whichever view the library has.
    pub(super) fn item_rect(&self, ui: &Ui, k: usize) -> (f32, f32, f32, f32) {
        if self.list_view() { self.list_row_rect(ui, k) } else { self.cover_rect(ui, k) }
    }

    /// Library grid geometry: (columns, tile width, cover height, cell height, gap).
    pub(super) fn grid(&self, ui: &Ui) -> (usize, f32, f32, f32, f32) {
        let room = self.w as f32 - self.side(ui);
        let cols = ((room / ui.pt(COVER_MIN_PT)) as usize).clamp(2, 8);
        let (m, gap) = (ui.pt(MARGIN_PT), ui.pt(SPACE_XL));
        let tw = (room - 2.0 * m - gap * (cols - 1) as f32) / cols as f32;
        let ch = tw * 1.5;
        (cols, tw, ch, ch + ui.pt(64.0), gap)
    }

    pub(super) fn grid_top(&self, ui: &Ui) -> f32 {
        ui.pt(UNIFIED_BAR_PT + SPACE_L)
    }

    /// Screen rect of the `i`th cover the selected collection shows, at the current scroll.
    pub(super) fn cover_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let (cols, tw, ch, cell, gap) = self.grid(ui);
        let n = self.reading_count(ui);
        let (x0, base) = (self.side(ui) + ui.pt(MARGIN_PT), self.grid_top(ui) - self.shelf_scroll.offset());
        if i < n {
            return (x0 + i as f32 * (tw + gap), base + ui.pt(SECTION_PT), tw, ch);
        }
        let (row, col) = ((i - n) / cols, (i - n) % cols);
        (x0 + col as f32 * (tw + gap), base + self.library_offset(ui) + row as f32 * cell, tw, ch)
    }

    /// Books in the Reading Now row of All: the most recently opened, at most one row.
    pub(super) fn reading_count(&self, ui: &Ui) -> usize {
        // the row is the books last opened: it is shown in the grid of All, in the order of last opening, without a search
        if self.filter != Filter::All || self.list_view() || self.store.library.sort != SortBy::Recent || !self.library_search.is_empty() {
            return 0;
        }
        let (cols, _, _, _, _) = self.grid(ui);
        self.shelf.iter().take(cols).take_while(|c| self.store.entry(&c.path).is_some_and(|e| e.opened > 0)).count()
    }

    /// Offset of the Library grid's first row below the grid top.
    pub(super) fn library_offset(&self, ui: &Ui) -> f32 {
        let (_, _, _, cell, _) = self.grid(ui);
        if self.reading_count(ui) > 0 { ui.pt(SECTION_PT) * 2.0 + cell } else { 0.0 }
    }

    fn fit_shelf_scroll(&mut self, ui: &Ui) {
        let (cols, _, _, cell, _) = self.grid(ui);
        let shown = self.visible().len();
        let content = if self.list_view() {
            self.grid_top(ui) + shown as f32 * ui.pt(LIST_ROW_PT) + ui.pt(SPACE_XXL)
        } else {
            let rows = shown.saturating_sub(self.reading_count(ui)).div_ceil(cols);
            self.grid_top(ui) + self.library_offset(ui) + rows as f32 * cell + ui.pt(SPACE_XXL)
        };
        self.shelf_scroll.set_max(content - (self.h as f32 - home_zone(ui)));
    }

    /// Number of rows the open panel lists.
    pub(super) fn panel_rows(&self) -> usize {
        let Some(r) = &self.reader else { return 0 };
        match r.panel {
            Some(Panel::Contents) => r.book.toc.len(),
            Some(Panel::Bookmarks) => self.store.entry(&r.path).map_or(0, |e| e.bookmarks.len()),
            Some(Panel::Search) => r.hits.len(),
            Some(Panel::Pages) => r.layout.pages.len(),
            Some(Panel::Notes) => self.store.entry(&r.path).map_or(0, |e| e.notes.len()),
            _ => 0,
        }
    }

    /// Top of the scrolling list inside the open panel.
    pub(super) fn list_top(&self, ui: &Ui) -> f32 {
        let (_, py, _, _) = self.panel_rect(ui);
        match self.reader.as_ref().and_then(|r| r.panel) {
            Some(Panel::Contents | Panel::Bookmarks | Panel::Pages) | Some(Panel::Search) => py + ui.pt(TAB_PT + 8.0),
            Some(Panel::Notes) => {
                let (_, fy, _, fh) = self.note_field_rect(ui);
                fy + fh + ui.pt(SPACE_S)
            }
            _ => py,
        }
    }

    pub(super) fn row_rect(&self, ui: &Ui, i: usize) -> (f32, f32, f32, f32) {
        let m = ui.pt(MARGIN_PT);
        let (_, _, pw, _) = self.panel_rect(ui);
        (m, self.list_top(ui) + i as f32 * ui.pt(ROW_PT) - self.panel_scroll.offset(), pw - 2.0 * m, ui.pt(ROW_PT))
    }

    /// The distance in px from the top of one page to the top of the next in continuous scrolling: the window below the
    /// toolbar, so one page fills the window when the scroll rests on it.
    pub(super) fn page_pitch(&self, ui: &Ui) -> f32 {
        (self.h as f32 - ui.pt(page_top_pt())).max(1.0)
    }

    /// Keeps the page distance and the end of the scroll range in step with the window and the number of pages.
    fn fit_page_scroll(&mut self, ui: &Ui) {
        self.pitch = self.page_pitch(ui);
        let pages = self.reader.as_ref().map_or(1, |r| r.layout.pages.len());
        self.page_scroll.set_max(pages.saturating_sub(1) as f32 * self.pitch);
    }

    /// Puts the scroll of continuous scrolling at the top of the current page.
    fn sync_scroll(&mut self) {
        if !self.continuous {
            return;
        }
        let page = self.reader.as_ref().map_or(0, |r| r.page);
        self.page_scroll.jump_to(page as f32 * self.pitch);
    }

    /// Makes the page in the middle of the window the current one while the pages are scrolled.
    fn follow_scroll(&mut self, t: f64) {
        if !self.continuous {
            return;
        }
        let Some(r) = &self.reader else { return };
        let page = page_at(self.page_scroll.offset(), self.pitch, r.layout.pages.len());
        if page != r.page {
            self.set_page(page, t);
        }
    }

    /// View menu: whether the pages follow each other down the window.
    pub fn continuous(&self) -> bool {
        self.continuous
    }

    /// View menu: Continuous Scroll shows the pages one below the other and scrolls through them; off, one page is shown at a
    /// time. It replaces Two Pages, and the reader keeps its place.
    pub fn set_continuous(&mut self, ui: &Ui, on: bool) {
        if self.continuous == on {
            return;
        }
        self.slide = None;
        if on && self.two_pages() {
            self.set_two_pages(ui, false);
        }
        self.continuous = on;
        self.fit_page_scroll(ui);
        self.sync_scroll();
        self.dirty = true;
    }

    fn fit_panel_scroll(&mut self, ui: &Ui) {
        let (_, py, _, ph) = self.panel_rect(ui);
        let list_h = self.panel_rows() as f32 * ui.pt(ROW_PT) + ui.pt(SPACE_L);
        let view = py + ph - self.list_top(ui);
        self.panel_scroll.set_max(list_h - view);
    }

    pub(super) fn tab_rects(&self, ui: &Ui) -> Vec<(Panel, (f32, f32, f32, f32))> {
        let (px, py, pw, _) = self.panel_rect(ui);
        let m = ui.pt(MARGIN_PT);
        let tabs = [Panel::Contents, Panel::Bookmarks, Panel::Pages, Panel::Notes];
        let w = (pw - 2.0 * m) / tabs.len() as f32;
        tabs.iter().enumerate().map(|(i, &p)| (p, (px + m + w * i as f32, py + ui.pt(SPACE_S), w, ui.pt(TAB_PT - 6.0)))).collect()
    }

    /// The note field of the Notes panel: below the tabs.
    pub(super) fn note_field_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (_, py, pw, _) = self.panel_rect(ui);
        let m = ui.pt(MARGIN_PT);
        (m, py + ui.pt(SPACE_S) + ui.pt(TAB_PT), pw - 2.0 * m, ui.pt(TAB_PT - 6.0))
    }

    pub(super) fn search_field_rect(&self, ui: &Ui) -> (f32, f32, f32, f32) {
        let (_, py, pw, _) = self.panel_rect(ui);
        let m = ui.pt(MARGIN_PT);
        (m, py + ui.pt(SPACE_S), pw - 2.0 * m, ui.pt(TAB_PT - 6.0))
    }

    /// Buttons of the Display panel.
    pub(super) fn display_buttons(&self, ui: &Ui) -> Vec<(Target, (f32, f32, f32, f32))> {
        let (_, py, pw, _) = self.panel_rect(ui);
        let m = ui.pt(MARGIN_PT);
        let bw = (pw - 2.0 * m - ui.pt(SPACE_L)) / 2.0;
        let y = py + ui.pt(SPACE_XXL);
        let mut out = vec![(Target::SizeDown, (m, y, bw, ui.pt(56.0))), (Target::SizeUp, (m + bw + ui.pt(SPACE_L), y, bw, ui.pt(56.0)))];
        let tw = (pw - 2.0 * m - 2.0 * ui.pt(SPACE_M)) / 3.0;
        for i in 0..3 {
            out.push((Target::ThemeButton(i), (m + i as f32 * (tw + ui.pt(SPACE_M)), y + ui.pt(THEME_ROW_PT), tw, ui.pt(90.0))));
        }
        // line spacing and margins: a row of choices each; justification: one wide switch
        let choice = |n: usize| (pw - 2.0 * m - (n - 1) as f32 * ui.pt(SPACE_M)) / n as f32;
        let (sw, mw) = (choice(store::SPACINGS.len()), choice(store::MARGINS.len()));
        for i in 0..store::SPACINGS.len() {
            out.push((Target::Spacing(i), (m + i as f32 * (sw + ui.pt(SPACE_M)), y + ui.pt(SPACING_ROW_PT), sw, ui.pt(CHOICE_PT))));
        }
        for i in 0..store::MARGINS.len() {
            out.push((Target::Margin(i), (m + i as f32 * (mw + ui.pt(SPACE_M)), y + ui.pt(MARGIN_ROW_PT), mw, ui.pt(CHOICE_PT))));
        }
        out.push((Target::Justify, (m, y + ui.pt(JUSTIFY_ROW_PT), pw - 2.0 * m, ui.pt(CHOICE_PT))));
        out
    }

    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        match self.screen {
            Screen::Library => {
                if self.loading.is_some() {
                    return Some(Target::Shelf);
                }
                let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
                if y < self.nav_bottom(ui) && x >= self.side(ui) {
                    return Some(match self.bar_layout(ui).hit(px, py) {
                        Some(BAR_SIDEBAR) => Target::ToggleSidebar,
                        Some(BAR_LIBRARY_SEARCH) => {
                            let clear = self.library_search_rect(ui).is_some_and(|r| bifrost_app::controls::search::hits_clear(r, !self.library_search.is_empty(), px, py));
                            if clear { Target::LibraryClear } else { Target::LibrarySearch }
                        }
                        _ => Target::PanelBody,
                    });
                }
                if x < self.side(ui) {
                    let row = self.sidebar().hit(self.sidebar_bounds(ui), SIDEBAR_SCROLL, px, py);
                    let filters = self.sidebar_filters();
                    return Some(row.and_then(|id| filters.get(id.checked_sub(ROW_BASE)? as usize).copied()).map_or(Target::PanelBody, Target::Sidebar));
                }
                let vis = self.visible();
                (0..vis.len()).find(|&k| within(x, y, self.item_rect(ui, k))).map(|k| Target::Cover(vis[k])).or(Some(Target::Shelf))
            }
            Screen::Reader => {
                let r = self.reader.as_ref()?;
                if y < self.nav_bottom(ui) {
                    if within(x, y, self.back_rect(ui)) {
                        return Some(Target::Back);
                    }
                    return self.nav_buttons(ui).into_iter().find(|&(_, _, rect)| within(x, y, rect)).map(|(t, _, _)| t).or(Some(Target::PanelBody));
                }
                if let Some(panel) = r.panel {
                    if !within(x, y, self.panel_rect(ui)) {
                        return Some(Target::PageArea);
                    }
                    return Some(self.panel_hit(ui, panel, x, y));
                }
                if r.chrome && within(x, y, self.bottom_bar_rect(ui)) {
                    let (sx, sy, sw, sh) = self.slider_rect(ui);
                    if within(x, y, (sx - ui.pt(SPACE_M), sy - ui.pt(SPACE_M), sw + ui.pt(SPACE_XL), sh + ui.pt(SPACE_XL))) {
                        return Some(Target::Slider);
                    }
                    return Some(Target::PanelBody);
                }
                Some(Target::PageArea)
            }
        }
    }

    fn panel_hit(&self, ui: &Ui, panel: Panel, x: f32, y: f32) -> Target {
        match panel {
            Panel::Display => self.display_buttons(ui).into_iter().find(|&(_, rect)| within(x, y, rect)).map_or(Target::PanelBody, |(t, _)| t),
            Panel::Contents | Panel::Bookmarks | Panel::Pages | Panel::Notes | Panel::Search => {
                if panel == Panel::Search {
                    if within(x, y, self.search_field_rect(ui)) {
                        return Target::SearchField;
                    }
                } else {
                    if panel == Panel::Notes && within(x, y, self.note_field_rect(ui)) {
                        return Target::NoteField;
                    }
                    if let Some((p, _)) = self.tab_rects(ui).into_iter().find(|&(_, rect)| within(x, y, rect)) {
                        return Target::Tab(p);
                    }
                }
                if y < self.list_top(ui) {
                    return Target::PanelBody;
                }
                for i in 0..self.panel_rows() {
                    let rect = self.row_rect(ui, i);
                    if within(x, y, rect) {
                        let (rx, _, rw, _) = rect;
                        if panel == Panel::Bookmarks && x > rx + rw - ui.pt(90.0) {
                            return Target::RowRemove(i);
                        }
                        return Target::Row(i);
                    }
                }
                Target::PanelBody
            }
        }
    }

    // --- input ---

    pub fn key(&mut self, ui: &Ui, out: Out, t: f64) {
        self.dirty = true;
        if self.screen == Screen::Library {
            if self.library_search.focused {
                match out {
                    Out::Hide | Out::Enter => self.library_search.focused = false,
                    other => {
                        self.library_search.edit(ui, &other, t);
                        self.shelf_scroll.jump_top();
                    }
                }
            }
            return;
        }
        let Some(r) = &mut self.reader else { return };
        if r.panel == Some(Panel::Notes) {
            if !r.note.focused {
                return;
            }
            match out {
                Out::Hide => r.note.focused = false,
                Out::Enter => self.save_note(),
                other => {
                    r.note.edit(ui, &other, t);
                }
            }
            return;
        }
        if r.panel != Some(Panel::Search) {
            return;
        }
        match out {
            Out::Hide => {
                r.search.focused = false;
            }
            Out::Enter => {
                r.search.focused = false;
                self.run_search();
            }
            other => {
                r.search.edit(ui, &other, t);
            }
        }
    }

    fn open_panel(&mut self, ui: &Ui, panel: Panel) {
        let Some(r) = &mut self.reader else { return };
        r.panel = if r.panel == Some(panel) { None } else { Some(panel) };
        self.panel_scroll.jump_top();
        if r.panel == Some(Panel::Search) {
            r.search.focused = true;
        } else {
            r.search.focused = false;
        }
        r.note.focused = false;
        // start the contents list at the current chapter
        if panel == Panel::Contents && r.panel.is_some() {
            let block = r.layout.pos_of(r.page).block;
            if let Some(i) = r.book.toc.iter().rposition(|e| e.block <= block) {
                self.fit_panel_scroll(ui);
                self.panel_scroll.jump_to((i as f32 * ui.pt(ROW_PT) - ui.pt(ROW_PT) * 2.0).max(0.0));
            }
        }
        self.dirty = true;
    }

    fn activate(&mut self, ui: &Ui, target: Target, x: f32, t: f64) {
        self.dirty = true;
        if !matches!(target, Target::LibrarySearch | Target::LibraryClear) {
            self.library_search.focused = false;
        }
        match target {
            Target::LibrarySearch => {
                if let Some(r) = self.library_search_rect(ui) {
                    // the caret counts from the start of the text, which `search_field` draws after the magnifier
                    let text_x = ui.pt(bifrost_app::controls::search::layout(r, false).text.x);
                    self.library_search.focused = true;
                    self.library_search.tap(ui, x - text_x, t);
                }
            }
            Target::LibraryClear => {
                self.library_search.set_text(ui, "");
                self.library_search.focused = true;
                self.shelf_scroll.jump_top();
            }
            Target::Cover(i) => {
                if let Some(c) = self.shelf.get(i) {
                    let p = c.path.clone();
                    self.open_path(&p);
                }
            }
            Target::Back => self.close_reader(t),
            Target::NavContents => self.open_panel(ui, Panel::Contents),
            Target::NavSearch => self.open_panel(ui, Panel::Search),
            Target::NavDisplay => self.open_panel(ui, Panel::Display),
            Target::NavMark => self.toggle_mark(),
            Target::PageArea => {
                let Some(r) = &mut self.reader else { return };
                if r.panel.is_some() {
                    r.panel = None;
                    r.search.focused = false;
                    r.note.focused = false;
                    return;
                }
                let w = self.w as f32;
                if x < w * EDGE {
                    self.turn(-1, t);
                } else if x > w * (1.0 - EDGE) {
                    self.turn(1, t);
                } else {
                    r.chrome = !r.chrome;
                }
            }
            Target::Tab(p) => {
                if let Some(r) = &mut self.reader {
                    r.panel = Some(p);
                    r.note.focused = false;
                }
                self.panel_scroll.jump_top();
            }
            Target::Row(i) => {
                let Some(r) = &self.reader else { return };
                let panel = r.panel;
                match panel {
                    Some(Panel::Contents) => {
                        if let Some(e) = r.book.toc.get(i) {
                            let pos = Pos { block: e.block, offset: 0 };
                            self.close_panel();
                            self.goto_pos(pos, t);
                        }
                    }
                    Some(Panel::Bookmarks) => {
                        let pos = self.store.entry(&r.path).and_then(|e| e.bookmarks.get(i).copied());
                        if let Some(pos) = pos {
                            self.close_panel();
                            self.goto_pos(pos, t);
                        }
                    }
                    Some(Panel::Search) => self.open_hit(i, t),
                    Some(Panel::Pages) => {
                        self.close_panel();
                        self.goto_page(i, t);
                    }
                    Some(Panel::Notes) => {
                        let span = self.notes_list().get(i).map(|(span, _)| *span);
                        if let Some(span) = span {
                            self.close_panel();
                            self.goto_pos(Pos { block: span.block, offset: span.offset }, t);
                        }
                    }
                    _ => {}
                }
            }
            Target::RowRemove(i) => {
                let Some(r) = &self.reader else { return };
                let path = r.path.clone();
                if let Some(pos) = self.store.entry(&path).and_then(|e| e.bookmarks.get(i).copied()) {
                    self.store.toggle_bookmark(&path, pos);
                    self.save_now();
                }
            }
            Target::SearchField => {
                let (fx, ..) = self.search_field_rect(ui);
                if let Some(r) = &mut self.reader {
                    r.search.focused = true;
                    r.search.tap(ui, x - fx - ui.pt(SPACE_M), t);
                }
            }
            Target::NoteField => {
                let (fx, ..) = self.note_field_rect(ui);
                if let Some(r) = &mut self.reader {
                    // the field edits only a note that Add Note opened (see `add_note`)
                    if r.note_target.is_some() {
                        r.note.focused = true;
                        r.note.tap(ui, x - fx - ui.pt(SPACE_M), t);
                    }
                }
            }
            Target::SizeDown | Target::SizeUp => {
                let s = self.store.settings;
                let size = if target == Target::SizeUp { (s.size + 1).min(SIZES.len() - 1) } else { s.size.saturating_sub(1) };
                self.change_settings(ui, size, s.theme);
            }
            Target::ThemeButton(i) => {
                let s = self.store.settings;
                self.change_settings(ui, s.size, Theme::ALL[i.min(2)]);
            }
            Target::Spacing(i) => self.set_line_spacing(ui, i),
            Target::Margin(i) => self.set_margins(ui, i),
            Target::Justify => {
                let on = !self.store.settings.justify;
                self.set_justified(ui, on);
            }
            Target::Shelf => self.error = None,
            Target::Sidebar(f) => {
                self.filter = f;
                self.shelf_scroll.jump_top();
            }
            Target::ToggleSidebar => self.sidebar_hidden = !self.sidebar_hidden,
            Target::Slider | Target::PanelBody => {}
        }
    }

    fn close_panel(&mut self) {
        if let Some(r) = &mut self.reader {
            r.panel = None;
            r.search.focused = false;
            r.note.focused = false;
        }
    }

    fn seek(&mut self, ui: &Ui, x: f32, t: f64) {
        let (sx, _, sw, _) = self.slider_rect(ui);
        let Some(r) = &self.reader else { return };
        let n = r.layout.pages.len();
        if n < 2 {
            return;
        }
        let f = ((x - sx) / sw).clamp(0.0, 1.0);
        self.goto_page((f * (n - 1) as f32).round() as usize, t);
    }

    fn scroller(&mut self) -> &mut Scroller {
        if self.screen == Screen::Library {
            &mut self.shelf_scroll
        } else if self.scrolls_pages() {
            &mut self.page_scroll
        } else {
            &mut self.panel_scroll
        }
    }

    /// Whether a drag on the page scrolls the pages: continuous scrolling, with no panel over them.
    fn scrolls_pages(&self) -> bool {
        self.continuous && self.screen == Screen::Reader && self.reader.as_ref().is_some_and(|r| r.panel.is_none())
    }

    pub fn touch(&mut self, ui: &Ui, ev: Event, t: f64) {
        self.dirty = true;
        // a tap dismisses the notice of Look Up or Translate
        if matches!(ev, Event::Down(..)) {
            self.notice = None;
        }
        let (x, y) = match ev {
            Event::Down(x, y) | Event::Move(x, y) | Event::Up(x, y) => (x, y),
            _ => return,
        };
        self.fit_shelf_scroll(ui);
        self.fit_panel_scroll(ui);
        self.fit_page_scroll(ui);
        match ev {
            Event::Down(..) => {
                self.down = Some((x, y));
                self.pressed = self.hit(ui, x, y);
                match self.pressed {
                    Some(Target::Cover(_) | Target::Shelf | Target::Row(_) | Target::RowRemove(_)) => {
                        self.scroller().down(y, t);
                        self.scrolling = true;
                    }
                    Some(Target::PageArea) if self.scrolls_pages() => {
                        self.scroller().down(y, t);
                        self.scrolling = true;
                    }
                    Some(Target::Slider) => self.seek(ui, x, t),
                    _ => {}
                }
            }
            Event::Move(..) => {
                if self.scrolling {
                    self.scroller().moved(y, t);
                    if self.scroller().dragging() {
                        self.pressed = None;
                    }
                } else if self.pressed == Some(Target::Slider) {
                    self.seek(ui, x, t);
                }
            }
            _ => {
                let pressed = self.pressed.take();
                let down = self.down.take();
                let tap = if std::mem::take(&mut self.scrolling) { self.scroller().up(y, t) } else { true };
                if pressed == Some(Target::PageArea) {
                    if let Some((x0, y0)) = down {
                        let (dx, dy) = (x - x0, y - y0);
                        if dx.abs() > ui.pt(SWIPE_PT) && dx.abs() > 2.0 * dy.abs() {
                            let no_panel = self.reader.as_ref().is_some_and(|r| r.panel.is_none());
                            if no_panel {
                                self.turn(if dx < 0.0 { 1 } else { -1 }, t);
                            }
                            return;
                        }
                        if dx.hypot(dy) > ui.pt(SPACE_XXL) {
                            return;
                        }
                    }
                }
                if tap {
                    if let Some(p) = pressed.filter(|&p| Some(p) == self.hit(ui, x, y)) {
                        self.activate(ui, p, x, t);
                    }
                }
            }
        }
    }

    /// Worker results, the save timer, held keys, scrolling; true when a frame is needed.
    pub fn tick(&mut self, ui: &Ui, t: f64) -> bool {
        // library probes
        let mut got = false;
        if let Some(inbox) = &self.probes {
            let done = finished(inbox);
            for (path, meta) in drain(inbox) {
                if let Some(c) = self.shelf.iter_mut().find(|c| c.path == path) {
                    c.title = meta.title;
                    c.author = meta.author;
                    c.image = meta.cover;
                    c.tile = None;
                }
                got = true;
            }
            if done {
                self.probes = None;
            }
        }
        // the book being opened
        let mut opened = None;
        if let Some(l) = &self.loading {
            match drain(&l.inbox).pop() {
                Some(result) => opened = Some((l.path.clone(), result)),
                None if finished(&l.inbox) => opened = Some((l.path.clone(), Err("the reader stopped unexpectedly".to_string()))),
                None => {}
            }
        }
        if let Some((path, result)) = opened {
            self.loading = None;
            match result {
                Ok(book) => self.show_book(ui, path, book),
                Err(e) => self.error = Some(e),
            }
            got = true;
        }
        self.credit_reading(t);
        if self.save_at.is_some_and(|at| t >= at) {
            self.save_now();
        }
        self.fit_shelf_scroll(ui);
        self.fit_panel_scroll(ui);
        self.fit_page_scroll(ui);
        self.follow_scroll(t);
        if self.slide.is_some_and(|(_, _, t0)| t - t0 >= SLIDE_MS) {
            self.slide = None;
            self.dirty = true;
        }
        let narrated = self.follow_narration(t);
        let moving = self.shelf_scroll.tick(t) | self.panel_scroll.tick(t) | self.page_scroll.tick(t) | self.slide.is_some();
        let blinking = self.screen == Screen::Library && self.library_search.focused;
        self.dirty |= got || moving || blinking || narrated;
        self.dirty
    }

    /// When `tick` must run again without input (worker polling, save timer, caret).
    pub fn next_wake(&self, t: f64) -> Option<f64> {
        let mut due: Option<f64> = None;
        let mut want = |w: f64| due = Some(due.map_or(w, |d| d.min(w)));
        // worker results (book opens, cover probes) wake the loop through `deliver`; no polling here
        if self.slide.is_some() {
            want(t + 16.0);
        }
        if let Some(at) = self.save_at {
            want(at);
        }
        if let Some(r) = &self.reader {
            if r.panel == Some(Panel::Search) {
                if let Some(w) = r.search.next_wake(t) {
                    want(w);
                }
            }
        }
        if let Some(w) = self.reader.as_ref().and_then(|r| r.narration.as_ref()).and_then(|n| n.wake()) {
            want(w);
        }
        if self.screen == Screen::Library && self.library_search.focused {
            if let Some(w) = self.library_search.next_wake(t) {
                want(w);
            }
        }
        due
    }

    /// key=value lines for remote inspection. Tests read them; the app has no control method for them (that needs a
    /// `bifrost-core` dependency for the JSON reply).
    #[cfg(test)]
    pub fn describe(&self) -> String {
        let screen = match (&self.screen, &self.reader) {
            (Screen::Reader, Some(r)) => format!("reader:{}:{}/{}", r.book.title, r.page + 1, r.layout.pages.len()),
            _ => "library".to_string(),
        };
        format!("books_screen={screen}\nbooks_shelf={}\nbooks_loading={}\nbooks_theme={}\nbooks_size={}\n", self.shelf.len(), self.loading.is_some(), self.store.settings.theme.name(), SIZES[self.store.settings.size.min(SIZES.len() - 1)])
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, t: f64) {
        self.dirty = false;
        match self.screen {
            Screen::Library => self.draw_library(s, ui),
            Screen::Reader => self.draw_reader(s, ui, t),
        }
    }
}

#[cfg(test)]
mod tests;
