//! Books as an installed BifrOSt app: the former in-shell PDF/EPUB reader,
//! unchanged, hosted through the SDK. Typing comes from the hardware keyboard.

mod books;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, OpenOptions, TouchKind};
use books::store::{self, SortBy, Status, ViewMode};
use springboard::shell::keyboard::Out;
use std::path::{Path, PathBuf};

const MENU_PRINT: u32 = 1;
const MENU_FIND_NEXT: u32 = 2;
const MENU_FIND_PREVIOUS: u32 = 3;
const MENU_HIGHLIGHT: u32 = 4;
const MENU_WHOLE_WORDS: u32 = 5;
const MENU_COPY_QUOTE: u32 = 6;
const MENU_LOOK_UP: u32 = 7;
const MENU_TRANSLATE: u32 = 8;
const MENU_READ_ALOUD: u32 = 9;
const MENU_STOP_READING: u32 = 10;
const MENU_ADD_NOTE: u32 = 11;
const MENU_SINGLE_PAGE: u32 = 12;
const MENU_TWO_PAGES: u32 = 13;
const MENU_SELECT_PAGE: u32 = 14;
const MENU_COPY_SELECTION: u32 = 15;
const MENU_ADD_TO_LIBRARY: u32 = 16;
const MENU_WANT_TO_READ: u32 = 17;
const MENU_FINISHED: u32 = 18;
const MENU_NEW_COLLECTION: u32 = 19;
const MENU_RENAME_COLLECTION: u32 = 20;
const MENU_DELETE_COLLECTION: u32 = 21;
const MENU_NO_COLLECTIONS: u32 = 22;
const MENU_VIEW_GRID: u32 = 25;
const MENU_VIEW_LIST: u32 = 26;
const MENU_JUSTIFY: u32 = 27;
const MENU_CONTINUOUS: u32 = 28;
/// Submenu items count up from their base: Sort By (an index into `SortBy::ALL`), Reading Goal (into `store::GOALS`),
/// Highlight Color (into `store::HIGHLIGHT_STYLES`), Line Spacing, Margins, and Add to Collection.
const MENU_SORT_BASE: u32 = 30;
const MENU_GOAL_BASE: u32 = 40;
const MENU_COLOR_BASE: u32 = 50;
const MENU_SPACING_BASE: u32 = 60;
const MENU_MARGIN_BASE: u32 = 70;
const MENU_COLLECTION_BASE: u32 = 100;
/// How many collections the Add to Collection menu lists.
const MAX_MENU_COLLECTIONS: usize = 20;
/// Document requests with this bit set were not asked for by this app: a file shared to it (`shell/sharing.rs`).
const INBOUND: u32 = 0x8000_0000;

/// Which of `count` submenu items the menu item `id` is, counting from `base`.
fn offset(id: u32, base: u32, count: usize) -> Option<usize> {
    id.checked_sub(base).map(|d| d as usize).filter(|d| *d < count)
}

/// What the menus show: which items are on, off or ticked. Built from the app's state by `BooksApp::main_menu`.
#[derive(Default)]
struct MenuState {
    /// a book is open in the reader
    open: bool,
    can_print: bool,
    has_hits: bool,
    /// the text the last find went to, and whether it is highlighted
    highlight: Option<(books::store::Span, bool)>,
    /// the style of that highlight
    match_style: Option<u8>,
    whole_words: bool,
    two_pages: bool,
    continuous: bool,
    has_selection: bool,
    /// Read Aloud: None when nothing is read, otherwise whether it is paused
    speaking: Option<bool>,
    status: Option<Status>,
    /// the collections, with whether the open book is in each
    collections: Vec<(String, bool)>,
    /// the library shows one of the reader's collections
    collection_shown: bool,
    /// the library search field has text
    search_text: bool,
    sort: SortBy,
    list: bool,
    goal: u32,
    settings: store::Settings,
}

fn build_menus(m: &MenuState) -> Vec<AppMenu> {
    let read_title = match m.speaking {
        None => "Read Aloud",
        Some(false) => "Pause Reading",
        Some(true) => "Resume Reading",
    };
    let status = m.status.unwrap_or(Status::None);
    let mut library = vec![
        AppMenuItem::new(MENU_WANT_TO_READ, "Want to Read").enabled(m.open).checked(status == Status::WantToRead),
        AppMenuItem::new(MENU_FINISHED, "Finished").enabled(m.open).checked(status == Status::Finished),
        AppMenuItem::separator(),
        AppMenuItem::submenu("Add to Collection"),
    ];
    if m.collections.is_empty() {
        library.push(AppMenuItem::new(MENU_NO_COLLECTIONS, "No Collections").enabled(false).depth(1));
    }
    for (i, (name, inside)) in m.collections.iter().enumerate().take(MAX_MENU_COLLECTIONS) {
        library.push(AppMenuItem::new(MENU_COLLECTION_BASE + i as u32, name).enabled(m.open).checked(*inside).depth(1));
    }
    library.push(AppMenuItem::separator());
    // a new collection takes the name in the search field; Rename takes it for the collection on show
    library.push(AppMenuItem::new(MENU_NEW_COLLECTION, "New Collection"));
    library.push(AppMenuItem::new(MENU_RENAME_COLLECTION, "Rename Collection to Search Text").enabled(m.collection_shown && m.search_text));
    library.push(AppMenuItem::new(MENU_DELETE_COLLECTION, "Delete Collection").enabled(m.collection_shown));
    library.push(AppMenuItem::separator());
    library.push(AppMenuItem::submenu("Reading Goal"));
    for (i, minutes) in store::GOALS.iter().enumerate() {
        let title = if *minutes == 0 { "Off".to_string() } else { format!("{minutes} Minutes a Day") };
        library.push(AppMenuItem::new(MENU_GOAL_BASE + i as u32, &title).checked(m.goal == *minutes).depth(1));
    }

    let mut find = vec![
        AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(m.has_hits),
        AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(m.has_hits),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_HIGHLIGHT, "Highlight Match").key("cmd+shift+y").enabled(m.highlight.is_some()).checked(m.highlight.is_some_and(|(_, saved)| saved)),
        AppMenuItem::submenu("Highlight Color"),
    ];
    for (i, name) in store::HIGHLIGHT_STYLES.iter().enumerate() {
        find.push(AppMenuItem::new(MENU_COLOR_BASE + i as u32, name).enabled(m.highlight.is_some()).checked(m.match_style == Some(i as u8)).depth(1));
    }
    find.extend([
        // the shell owns Edit; copying the match as a quote sits with the match's other actions
        AppMenuItem::new(MENU_COPY_QUOTE, "Copy Quote").key("cmd+shift+c").enabled(m.highlight.is_some()),
        // Select Page takes the text on the screen; Copy Selection puts that text on the board
        AppMenuItem::new(MENU_SELECT_PAGE, "Select Page").enabled(m.open),
        AppMenuItem::new(MENU_COPY_SELECTION, "Copy Selection").enabled(m.has_selection),
        // a note sits on the match's highlight; the Notes panel opens with the note field ready
        AppMenuItem::new(MENU_ADD_NOTE, "Add Note").enabled(m.highlight.is_some()),
        AppMenuItem::separator(),
        // Look Up and Translate act on the text the last find went to, like Copy Quote
        AppMenuItem::new(MENU_LOOK_UP, "Look Up").enabled(m.highlight.is_some()),
        AppMenuItem::new(MENU_TRANSLATE, "Translate").enabled(m.highlight.is_some()),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_READ_ALOUD, read_title).enabled(m.open),
        AppMenuItem::new(MENU_STOP_READING, "Stop Reading").enabled(m.speaking.is_some()),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_WHOLE_WORDS, "Match Whole Words").enabled(m.open).checked(m.whole_words),
    ]);

    // the View menu merges into the shell's View menu
    let mut view = vec![
        AppMenuItem::new(MENU_SINGLE_PAGE, "Single Page").enabled(m.open).checked(!m.two_pages && !m.continuous),
        AppMenuItem::new(MENU_TWO_PAGES, "Two Pages").enabled(m.open).checked(m.two_pages),
        AppMenuItem::new(MENU_CONTINUOUS, "Continuous Scroll").enabled(m.open).checked(m.continuous),
        AppMenuItem::separator(),
        AppMenuItem::new(MENU_VIEW_GRID, "as Grid").checked(!m.list),
        AppMenuItem::new(MENU_VIEW_LIST, "as List").checked(m.list),
        AppMenuItem::submenu("Sort By"),
    ];
    for (i, sort) in SortBy::ALL.iter().enumerate() {
        view.push(AppMenuItem::new(MENU_SORT_BASE + i as u32, sort.name()).checked(m.sort == *sort).depth(1));
    }
    view.push(AppMenuItem::separator());
    view.push(AppMenuItem::submenu("Line Spacing"));
    for (i, (name, _)) in store::SPACINGS.iter().enumerate() {
        view.push(AppMenuItem::new(MENU_SPACING_BASE + i as u32, name).checked(m.settings.spacing == i).depth(1));
    }
    view.push(AppMenuItem::submenu("Margins"));
    for (i, (name, _)) in store::MARGINS.iter().enumerate() {
        view.push(AppMenuItem::new(MENU_MARGIN_BASE + i as u32, name).checked(m.settings.margin == i).depth(1));
    }
    view.push(AppMenuItem::new(MENU_JUSTIFY, "Justify Text").checked(m.settings.justify));

    vec![
        AppMenu {
            title: "File".into(),
            items: vec![
                AppMenuItem::new(MENU_ADD_TO_LIBRARY, "Add to Library...").key("cmd+o"),
                AppMenuItem::separator(),
                AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(m.open && m.can_print),
            ],
        },
        AppMenu { title: "Library".into(), items: library },
        AppMenu { title: "Find".into(), items: find },
        AppMenu { title: "View".into(), items: view },
    ]
}

/// How the shelf reaches the search index: a book is sent when it is new or changed (see `books::search_index`).
struct BooksApp {
    books: Option<books::Books>,
    index: books::search_index::SearchIndex,
}

impl BooksApp {
    /// The menu items about the library and the reading settings. None when `id` is not one of them.
    fn library_action(&mut self, ctx: &Ctx, id: u32) -> Option<bool> {
        if id == MENU_ADD_TO_LIBRARY {
            let options = OpenOptions { title: "Add to Library".into(), filters: vec!["epub".into(), "pdf".into()], multiple: true, ..OpenOptions::default() };
            let _ = ctx.open_panel(&options);
            return Some(false);
        }
        let books = self.books.as_mut()?;
        if let Some(k) = offset(id, MENU_SORT_BASE, SortBy::ALL.len()) {
            books.set_sort(SortBy::ALL[k]);
        } else if let Some(k) = offset(id, MENU_GOAL_BASE, store::GOALS.len()) {
            books.set_reading_goal(store::GOALS[k]);
        } else if let Some(k) = offset(id, MENU_COLOR_BASE, store::HIGHLIGHT_STYLES.len()) {
            books.set_match_style(k as u8);
        } else if let Some(k) = offset(id, MENU_SPACING_BASE, store::SPACINGS.len()) {
            books.set_line_spacing(&ctx.ui, k);
        } else if let Some(k) = offset(id, MENU_MARGIN_BASE, store::MARGINS.len()) {
            books.set_margins(&ctx.ui, k);
        } else if let Some(k) = offset(id, MENU_COLLECTION_BASE, MAX_MENU_COLLECTIONS) {
            books.toggle_open_in_collection(k);
        } else {
            match id {
                MENU_WANT_TO_READ => books.toggle_open_status(Status::WantToRead),
                MENU_FINISHED => books.toggle_open_status(Status::Finished),
                MENU_NEW_COLLECTION => books.new_collection(&ctx.ui),
                MENU_RENAME_COLLECTION => {
                    books.rename_collection(&ctx.ui);
                }
                MENU_DELETE_COLLECTION => books.delete_collection(),
                MENU_VIEW_GRID => books.set_view_mode(ViewMode::Grid),
                MENU_VIEW_LIST => books.set_view_mode(ViewMode::List),
                MENU_JUSTIFY => {
                    let on = !books.settings().justify;
                    books.set_justified(&ctx.ui, on);
                }
                MENU_CONTINUOUS => {
                    let on = !books.continuous();
                    books.set_continuous(&ctx.ui, on);
                }
                _ => return None,
            }
        }
        Some(true)
    }
}

impl App for BooksApp {
    fn id(&self) -> &str {
        "is.olibuijr.books.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        let (w, h) = (ctx.cfg.w as i32, ctx.cfg.h as i32);
        match &mut self.books {
            Some(b) => b.set_size(w, h),
            None => {
                // under the testing kit the library is the app's own (empty) data dir, not the machine's files
                let home = if bifrost_app::testing::pinned() { ctx.data_dir.clone() } else { PathBuf::from(std::env::var("HOME").unwrap_or_default()) };
                let folders = ["Books", "Documents", "Downloads"].iter().map(|name| home.join(name)).collect();
                self.books = Some(books::Books::with_library_dirs(&ctx.ui, w, h, ctx.data_dir.clone(), folders));
            },
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let state = match self.books.as_ref() {
            Some(b) => MenuState {
                open: b.reading().is_some(),
                can_print: bifrost_app::print::available(),
                has_hits: b.has_hits(),
                highlight: b.match_span(),
                match_style: b.match_style(),
                whole_words: b.whole_words(),
                two_pages: b.two_pages(),
                continuous: b.continuous(),
                has_selection: b.has_selection(),
                speaking: b.speaking(),
                status: Some(b.open_status()),
                collections: b.collection_menu(),
                collection_shown: b.current_collection().is_some(),
                search_text: b.has_search_text(),
                sort: b.sort_by(),
                list: b.view_mode() == ViewMode::List,
                goal: b.reading_goal(),
                settings: b.settings(),
            },
            None => MenuState::default(),
        };
        build_menus(&state)
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_COPY_QUOTE {
            // a quotation goes to the shared clipboard as plain text, with its citation; no frame is needed
            if let Some(quotation) = self.books.as_ref().and_then(|b| b.quotation()) {
                books::quote::copy(ctx, &quotation);
            }
            return false;
        }
        if id == MENU_COPY_SELECTION {
            // the selected text goes to the shared clipboard as plain text, like a quotation
            if let Some(text) = self.books.as_ref().and_then(|b| b.selection_text()) {
                books::quote::copy(ctx, &text);
            }
            return false;
        }
        if let Some(redraw) = self.library_action(ctx, id) {
            return redraw;
        }
        if matches!(id, MENU_FIND_NEXT | MENU_FIND_PREVIOUS | MENU_HIGHLIGHT | MENU_WHOLE_WORDS | MENU_LOOK_UP | MENU_TRANSLATE | MENU_READ_ALOUD | MENU_STOP_READING | MENU_ADD_NOTE | MENU_SINGLE_PAGE | MENU_TWO_PAGES | MENU_SELECT_PAGE) {
            let Some(books) = &mut self.books else { return false };
            let t = bifrost_app::proto::sys::now_ms();
            match id {
                MENU_FIND_NEXT => books.find_step(1, t),
                MENU_FIND_PREVIOUS => books.find_step(-1, t),
                MENU_SINGLE_PAGE => {
                    books.set_continuous(&ctx.ui, false);
                    books.set_two_pages(&ctx.ui, false);
                }
                MENU_TWO_PAGES => books.set_two_pages(&ctx.ui, true),
                MENU_SELECT_PAGE => {
                    books.select_page();
                }
                MENU_HIGHLIGHT => {
                    books.toggle_match_highlight();
                }
                MENU_LOOK_UP => books.look_up(),
                MENU_TRANSLATE => books.translate(),
                MENU_READ_ALOUD => books.read_aloud(t),
                MENU_STOP_READING => books.stop_reading(),
                MENU_ADD_NOTE => books.add_note(&ctx.ui),
                _ => {
                    let on = !books.whole_words();
                    books.set_whole_words(on);
                }
            }
            return true;
        }
        if id != MENU_PRINT {
            return false;
        }
        // PDF books print as they are; an EPUB prints as the text of the book, laid out for the paper
        let Some(books) = self.books.as_ref() else { return false };
        let Some(path) = books.reading() else { return false };
        if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            let _ = bifrost_app::print::PrintOperation::new(path).run();
        } else if let Some((title, text)) = books.print_text() {
            let _ = bifrost_app::print::PrintOperation::text(title, text).run();
        }
        false
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        if let Input::Context { x, y } = &ev {
            // a right click on a cover in the library
            let Some(books) = &mut self.books else { return false };
            let items = books.context_items(&ctx.ui, *x, *y);
            if !items.is_empty() {
                ctx.context_menu(*x, *y, items);
            }
            return true;
        }
        if let Input::ContextAction(id) = &ev {
            return self.books.as_mut().is_some_and(|b| b.context_action(*id));
        }
        if let Input::Open(url) = &ev {
            let path = if url.starts_with("file:") {
                let Ok(path) = url::Url::parse(url).and_then(|uri| uri.to_file_path().map_err(|_| url::ParseError::RelativeUrlWithoutBase)) else { return false };
                path
            } else if Path::new(url).is_absolute() {
                PathBuf::from(url)
            } else {
                return false;
            };
            if let Some(books) = &mut self.books { books.open_path(&path); return true; }
            return false;
        }
        if let Input::Key(key) = &ev {
            let out = match key {
                KeyInput::Text(text) => Out::Text(text.clone()),
                KeyInput::Code(8) => Out::Backspace,
                KeyInput::Code(13) => Out::Enter,
                KeyInput::Code(27) => Out::Hide,
                KeyInput::Code(9) => Out::Named("Tab"),
                KeyInput::Code(37) => Out::Named("Left"),
                KeyInput::Code(39) => Out::Named("Right"),
                _ => return false,
            };
            if let Some(app) = &mut self.books { app.key(&ctx.ui, out, bifrost_app::proto::sys::now_ms()); return true; }
            return false;
        }
        let (Some(b), Input::Touch { kind, x, y, t, .. }) = (&mut self.books, ev) else { return false };
        let e = match kind {
            TouchKind::Down => Event::Down(x, y),
            TouchKind::Move => Event::Move(x, y),
            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
        };
        b.touch(&ctx.ui, e, t);
        true
    }

    /// The books chosen in File > Add to Library, or a PDF shared to Books: each is copied into the Books folder and
    /// put on the shelf. A single shared book opens at once; books chosen in the panel only join the shelf.
    fn document(&mut self, _ctx: &Ctx, ev: DocumentEvent) -> bool {
        let docs = match ev {
            DocumentEvent::Opened(doc) => vec![doc],
            DocumentEvent::OpenedMany { docs, .. } => docs,
            _ => return false,
        };
        let Some(books) = &mut self.books else { return false };
        let mut added: Vec<(PathBuf, bool)> = Vec::new();
        for doc in &docs {
            let mut file = &doc.file;
            match books.import_book(&doc.name, &mut file) {
                Ok(path) => added.push((path, doc.request & INBOUND != 0)),
                Err(e) => books.show_error(e),
            }
        }
        if let Some(path) = books::share::book_to_open(&added) {
            books.open_path(path);
        }
        true
    }

    fn suspend(&mut self, _ctx: &Ctx) {
        if let Some(books) = &mut self.books { books.flush(); }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<bifrost_app::controls::Node> {
        self.books.as_ref().map_or_else(Vec::new, |b| b.controls(&ctx.ui))
    }

    fn continue_activity(&mut self, _ctx: &Ctx, activity: bifrost_app::quicksearch::UserActivity) -> bool {
        // a result of this app picked in the desktop search panel opens its book
        let Some(path) = books::search_index::path_from_activity(&activity) else { return false };
        let Some(books) = &mut self.books else { return false };
        books.open_path(&path);
        true
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        // sends only the books that changed since the last frame (see `books::search_index`)
        if let Some(b) = &self.books {
            self.index.sync(ctx, &b.index_entries());
        }
        self.books.as_mut().is_some_and(|b| b.tick(&ctx.ui, t))
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.books.as_ref().and_then(|b| b.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        if let Some(b) = &mut self.books {
            b.draw(s, &ctx.ui, t);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(BooksApp { books: None, index: Default::default() });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn titles(items: &[AppMenuItem]) -> Vec<(&str, u8)> {
        items.iter().filter(|i| !i.separator).map(|i| (i.title.as_str(), i.depth)).collect()
    }

    fn menu<'a>(menus: &'a [AppMenu], title: &str) -> &'a AppMenu {
        menus.iter().find(|m| m.title == title).expect("the menu exists")
    }

    fn item<'a>(menus: &'a [AppMenu], title: &str) -> &'a AppMenuItem {
        menus.iter().flat_map(|m| &m.items).find(|i| i.title == title).expect("the item exists")
    }

    #[test]
    fn submenu_ids_map_back_to_their_index() {
        assert_eq!(offset(30, 30, 3), Some(0));
        assert_eq!(offset(32, 30, 3), Some(2));
        assert_eq!(offset(33, 30, 3), None, "past the last item");
        assert_eq!(offset(29, 30, 3), None, "before the first");
        assert_eq!(offset(5, 30, 3), None, "a smaller id must not wrap");
    }

    #[test]
    fn every_menu_item_has_its_own_id_and_the_titles_stay_unique() {
        let state = MenuState { open: true, collections: vec![("Poetry".into(), false), ("Travel".into(), true)], ..MenuState::default() };
        let menus = build_menus(&state);
        let ids: Vec<u32> = menus.iter().flat_map(|m| &m.items).filter(|i| i.id != 0).map(|i| i.id).collect();
        assert_eq!(ids.len(), ids.iter().collect::<HashSet<_>>().len(), "ids repeat: {ids:?}");
        let names: Vec<&str> = menus.iter().map(|m| m.title.as_str()).collect();
        assert_eq!(names, ["File", "Library", "Find", "View"]);
        for m in &menus {
            assert!(m.items.len() <= bifrost_app::proto::MAX_APP_MENU_ITEMS, "{} has {} items", m.title, m.items.len());
            for i in &m.items {
                assert!(i.separator || i.submenu || i.id != 0, "{} has no id", i.title);
            }
        }
    }

    #[test]
    fn the_library_menu_lists_collections_and_ticks_the_open_books_own() {
        let state = MenuState { open: true, collections: vec![("Poetry".into(), false), ("Travel".into(), true)], status: Some(Status::Finished), ..MenuState::default() };
        let menus = build_menus(&state);
        let library = menu(&menus, "Library");
        let names = titles(&library.items);
        assert!(names.contains(&("Poetry", 1)) && names.contains(&("Travel", 1)));
        assert!(!item(&menus, "Poetry").checked && item(&menus, "Travel").checked);
        assert_eq!(item(&menus, "Travel").id, MENU_COLLECTION_BASE + 1);
        assert!(item(&menus, "Finished").checked && !item(&menus, "Want to Read").checked);
        assert!(item(&menus, "Finished").enabled);
    }

    #[test]
    fn collection_items_need_an_open_book_and_an_empty_list_says_so() {
        let menus = build_menus(&MenuState::default());
        let none = item(&menus, "No Collections");
        assert!(!none.enabled && none.depth == 1);
        assert!(!item(&menus, "Want to Read").enabled, "no book is open");
        assert!(!item(&menus, "Delete Collection").enabled);
        assert!(!item(&menus, "Rename Collection to Search Text").enabled);
        assert!(item(&menus, "New Collection").enabled);
        assert!(item(&menus, "Add to Library...").enabled, "adding books needs no open book");
        let shown = build_menus(&MenuState { collection_shown: true, ..MenuState::default() });
        assert!(item(&shown, "Delete Collection").enabled);
        assert!(!item(&shown, "Rename Collection to Search Text").enabled, "there is no new name yet");
        let named = build_menus(&MenuState { collection_shown: true, search_text: true, ..MenuState::default() });
        assert!(item(&named, "Rename Collection to Search Text").enabled);
    }

    #[test]
    fn the_view_menu_ticks_the_current_sort_view_and_reading_settings() {
        let settings = store::Settings { spacing: 2, margin: 0, justify: true, ..store::Settings::default() };
        let menus = build_menus(&MenuState { sort: SortBy::Author, list: true, settings, goal: 30, ..MenuState::default() });
        assert!(item(&menus, "Author").checked && !item(&menus, "Title").checked && !item(&menus, "Recent").checked);
        assert!(item(&menus, "as List").checked && !item(&menus, "as Grid").checked);
        assert!(item(&menus, "Loose").checked && item(&menus, "Narrow").checked && item(&menus, "Justify Text").checked);
        assert!(item(&menus, "30 Minutes a Day").checked && !item(&menus, "10 Minutes a Day").checked && !item(&menus, "Off").checked);
        assert_eq!(item(&menus, "Title").id, MENU_SORT_BASE + 1);
        assert_eq!(item(&menus, "Wide").id, MENU_MARGIN_BASE + 2);
    }

    #[test]
    fn highlight_colours_follow_the_match() {
        let none = build_menus(&MenuState { open: true, ..MenuState::default() });
        assert!(!item(&none, "Green").enabled, "no match to colour");
        let span = books::store::Span { block: 1, offset: 2, len: 3 };
        let on = build_menus(&MenuState { open: true, highlight: Some((span, true)), match_style: Some(1), ..MenuState::default() });
        assert!(item(&on, "Green").enabled && item(&on, "Green").checked);
        assert!(!item(&on, "Yellow").checked && !item(&on, "Underline").checked);
        assert_eq!(item(&on, "Underline").id, MENU_COLOR_BASE + store::UNDERLINE as u32);
    }

    #[test]
    fn continuous_scroll_is_a_view_choice_beside_the_page_modes() {
        let menus = build_menus(&MenuState { open: true, ..MenuState::default() });
        assert!(item(&menus, "Single Page").checked && !item(&menus, "Continuous Scroll").checked);
        assert!(item(&menus, "Continuous Scroll").enabled);
        let scrolling = build_menus(&MenuState { open: true, continuous: true, ..MenuState::default() });
        assert!(item(&scrolling, "Continuous Scroll").checked);
        assert!(!item(&scrolling, "Single Page").checked && !item(&scrolling, "Two Pages").checked);
        assert!(!item(&build_menus(&MenuState::default()), "Continuous Scroll").enabled, "no book, no scrolling");
    }

    #[test]
    fn print_needs_a_book_and_a_print_service() {
        assert!(!item(&build_menus(&MenuState { open: true, ..MenuState::default() }), "Print...").enabled);
        assert!(item(&build_menus(&MenuState { open: true, can_print: true, ..MenuState::default() }), "Print...").enabled);
        assert!(!item(&build_menus(&MenuState { can_print: true, ..MenuState::default() }), "Print...").enabled);
    }

    #[test]
    fn the_collection_submenu_is_capped() {
        let many: Vec<(String, bool)> = (0..40).map(|i| (format!("Shelf {i}"), false)).collect();
        let menus = build_menus(&MenuState { collections: many, ..MenuState::default() });
        let listed = menu(&menus, "Library").items.iter().filter(|i| i.title.starts_with("Shelf ")).count();
        assert_eq!(listed, MAX_MENU_COLLECTIONS);
    }
}
