//! Tests for the library features: Want to Read and Finished, collections, sorting, the list view, the reading goal,
//! adding books, and the reading settings (line spacing, margins, justification, highlight colours).

use super::engine::zip::tests::build;
use super::engine::{Block, Book, TocEntry};
use super::store::{SortBy, Status, ViewMode};
use super::*;

fn ui() -> Ui {
    springboard::shell::tests::ui()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-library")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn tap(b: &mut Books, ui: &Ui, x: f32, y: f32, t: f64) {
    b.touch(ui, Event::Down(x, y), t);
    b.touch(ui, Event::Up(x, y), t + 30.0);
}

/// The path of the shelf book called `title` in `shelf`.
fn path(title: &str) -> PathBuf {
    PathBuf::from(format!("/virtual/{title}.epub"))
}

/// A library with these books on the shelf (title, author), none of them on disk.
fn shelf(ui: &Ui, dir: &Path, books: &[(&str, &str)]) -> Books {
    let mut b = Books::with_library_dirs(ui, 1200, 1920, dir.to_path_buf(), Vec::new());
    for (title, author) in books {
        b.shelf.push(Cover { path: path(title), title: title.to_string(), author: author.to_string(), image: None, tile: None });
    }
    b
}

fn long_book() -> Book {
    let mut blocks = vec![Block::Heading(1, "Chapter One".into())];
    for i in 0..120 {
        blocks.push(Block::Para(format!("Paragraph {i}. The quick brown fox jumps over the lazy dog while the needle{i} waits quietly for somebody to read this somewhat long sentence all the way to the end of the line and beyond.")));
    }
    let toc = vec![TocEntry { title: "Chapter One".into(), block: 0, level: 0 }];
    Book { title: "Long Book".into(), author: "A. Tester".into(), blocks, toc, cover: None }
}

fn reader(dir: &Path, ui: &Ui) -> Books {
    let mut b = Books::with_library_dirs(ui, 1200, 1920, dir.to_path_buf(), Vec::new());
    b.show_book(ui, PathBuf::from("/virtual/long.epub"), long_book());
    b
}

fn render(b: &mut Books, ui: &Ui, t: f64) -> Vec<u32> {
    let mut buf = vec![0u32; 1200 * 1920];
    b.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, t);
    buf
}

fn tiny_epub() -> Vec<u8> {
    let container = r#"<container><rootfiles><rootfile full-path="c.opf"/></rootfiles></container>"#;
    let opf = r#"<package><metadata><dc:title>Tiny Book</dc:title><dc:creator>Me</dc:creator></metadata><manifest><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#;
    let a = "<html><body><h1>Hello</h1><p>A tiny book for the library test.</p></body></html>";
    build(&[("META-INF/container.xml", 0, container.as_bytes(), container.len()), ("c.opf", 0, opf.as_bytes(), opf.len()), ("a.xhtml", 0, a.as_bytes(), a.len())])
}

#[test]
fn want_to_read_and_finished_hold_the_books_with_that_status() {
    let ui = ui();
    let dir = scratch("status");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A"), ("Beta", "B"), ("Gamma", "C")]);
    b.store.set_status(&path("Alpha"), Status::WantToRead);
    b.store.set_status(&path("Gamma"), Status::Finished);
    b.filter = Filter::WantToRead;
    assert_eq!(b.visible(), vec![0]);
    b.filter = Filter::Finished;
    assert_eq!(b.visible(), vec![2]);
    assert_eq!(b.empty_title(), "No Finished Books");
    b.filter = Filter::All;
    assert_eq!(b.visible(), vec![0, 1, 2]);
    // the sidebar counts every row: Reading Now, All, Books, PDFs, Want to Read, Finished
    let counts: Vec<u32> = b.sidebar().items.iter().filter_map(|i| if let SidebarItem::Row { count, .. } = i { *count } else { None }).collect();
    assert_eq!(counts, vec![0, 3, 3, 0, 1, 1]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_library_search_narrows_a_status_collection_too() {
    let ui = ui();
    let dir = scratch("status-search");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A"), ("Beta", "B")]);
    b.store.set_status(&path("Alpha"), Status::Finished);
    b.store.set_status(&path("Beta"), Status::Finished);
    b.filter = Filter::Finished;
    b.library_search.set_text(&ui, "bet");
    assert_eq!(b.visible(), vec![1]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reaching_the_last_page_marks_the_book_finished() {
    let ui = ui();
    let dir = scratch("finish");
    let mut b = reader(&dir, &ui);
    let book = PathBuf::from("/virtual/long.epub");
    let (_, total) = b.page().unwrap();
    b.goto_page(total / 2, 100.0);
    assert_eq!(b.store.status_of(&book), Status::None, "the middle of the book is not the end");
    b.goto_page(total - 1, 200.0);
    assert_eq!(b.store.status_of(&book), Status::Finished);
    // the reader can take it back; staying on the last page does not mark it again
    b.toggle_open_status(Status::Finished);
    assert_eq!(b.open_status(), Status::None);
    b.goto_page(total - 1, 300.0);
    assert_eq!(b.open_status(), Status::None);
    // a spread that shows the last page counts as well
    b.toggle_open_status(Status::WantToRead);
    b.set_two_pages(&ui, true);
    b.goto_page(0, 350.0);
    let spread_total = b.page().unwrap().1;
    b.goto_page(spread_total - 2, 400.0);
    assert_eq!(b.open_status(), Status::Finished);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn status_survives_a_restart() {
    let ui = ui();
    let dir = scratch("status-restart");
    let mut b = reader(&dir, &ui);
    b.toggle_open_status(Status::WantToRead);
    b.flush();
    let again = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(again.store.status_of(Path::new("/virtual/long.epub")), Status::WantToRead);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sort_by_orders_the_shelf_and_leaves_the_reading_now_row_to_recent() {
    let ui = ui();
    let dir = scratch("sort");
    let mut b = shelf(&ui, &dir, &[("beta", "Zed"), ("Alpha", ""), ("gamma", "Abe")]);
    assert_eq!(b.visible(), vec![0, 1, 2], "the shelf order is the order of last opening");
    b.set_sort(SortBy::Title);
    assert_eq!(b.visible(), vec![1, 0, 2]);
    b.set_sort(SortBy::Author);
    assert_eq!(b.visible(), vec![2, 0, 1], "Abe, Zed, then the book without an author");
    b.set_sort(SortBy::Recent);
    assert_eq!(b.visible(), vec![0, 1, 2]);
    b.store.touch(&path("beta"));
    assert_eq!(b.reading_count(&ui), 1, "the Reading Now row shows the book opened last");
    b.set_sort(SortBy::Title);
    assert_eq!(b.reading_count(&ui), 0, "a sorted library has one list");
    b.flush();
    let again = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(again.sort_by(), SortBy::Title);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_list_view_has_a_row_per_book_that_a_tap_reaches() {
    let ui = ui();
    let dir = scratch("list");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A"), ("Beta", "B"), ("Gamma", "C")]);
    b.set_view_mode(ViewMode::List);
    assert!(b.list_view());
    for k in 0..3 {
        let (x, y) = centre(b.list_row_rect(&ui, k));
        assert_eq!(b.hit(&ui, x, y), Some(Target::Cover(k)), "row {k}");
    }
    let (_, y0, _, h0) = b.list_row_rect(&ui, 0);
    let (_, y1, _, _) = b.list_row_rect(&ui, 1);
    assert!((y1 - y0 - h0).abs() < 0.01, "rows follow each other");
    let before = render(&mut b, &ui, 0.0);
    b.set_view_mode(ViewMode::Grid);
    assert!(!b.list_view());
    assert_ne!(before, render(&mut b, &ui, 1.0), "the grid looks different from the list");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_list_scrolls_to_its_last_row() {
    let ui = ui();
    let dir = scratch("list-scroll");
    let many: Vec<(String, String)> = (0..60).map(|i| (format!("Book {i:02}"), "A".to_string())).collect();
    let refs: Vec<(&str, &str)> = many.iter().map(|(t, a)| (t.as_str(), a.as_str())).collect();
    let mut b = shelf(&ui, &dir, &refs);
    b.set_view_mode(ViewMode::List);
    b.tick(&ui, 0.0);
    let (_, last_y, _, last_h) = b.list_row_rect(&ui, 59);
    assert!(last_y + last_h > b.h as f32, "the last row starts below the window");
    b.shelf_scroll.jump_to(1.0e6);
    b.tick(&ui, 1.0);
    let (_, last_y, _, last_h) = b.list_row_rect(&ui, 59);
    assert!(last_y + last_h <= b.h as f32 + 1.0, "scrolled to the end, the last row is on the screen");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn collections_are_made_renamed_filled_and_deleted() {
    let ui = ui();
    let dir = scratch("collections");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A"), ("Beta", "B")]);
    b.library_search.set_text(&ui, "Weekend");
    b.new_collection(&ui);
    assert_eq!(b.current_collection(), Some(0));
    assert_eq!(b.store.collections[0].name, "Weekend");
    assert!(b.library_search.is_empty(), "the name left the search field");
    b.new_collection(&ui);
    assert_eq!(b.current_collection(), Some(1));
    assert_eq!(b.store.collections[1].name, "New Collection");
    assert_eq!(b.filter_name(Filter::Collection(1)), "New Collection");
    assert_eq!(b.empty_title(), "Empty Collection");
    // the sidebar lists them under their own header
    let items = b.sidebar().items;
    assert!(items.iter().any(|i| matches!(i, SidebarItem::Header(h) if h == "My Collections")));
    assert!(items.iter().any(|i| matches!(i, SidebarItem::Row { label, .. } if label == "Weekend")));
    assert_eq!(b.sidebar_filters().len(), Filter::ALL.len() + 2);
    // a right click on a cover offers the collections
    b.filter = Filter::All;
    let (x, y) = centre(b.cover_rect(&ui, 1));
    let items = b.context_items(&ui, x, y);
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, ["Open", "Add to Want to Read", "Mark as Finished", "Add to Weekend", "Add to New Collection"]);
    assert!(b.context_action(CTX_COLLECTION_BASE));
    assert!(b.store.in_collection(0, &path("Beta")) && !b.store.in_collection(0, &path("Alpha")));
    b.filter = Filter::Collection(0);
    assert_eq!(b.visible(), vec![1]);
    // renaming takes the search text; a taken name is refused
    b.filter = Filter::Collection(1);
    b.library_search.set_text(&ui, "weekend");
    assert!(!b.rename_collection(&ui), "Weekend is taken, ignoring case");
    b.library_search.set_text(&ui, "Evening");
    assert!(b.rename_collection(&ui));
    assert_eq!(b.filter_name(Filter::Collection(1)), "Evening");
    assert!(b.library_search.is_empty());
    assert!(!b.rename_collection(&ui), "an empty name renames nothing");
    // deleting leaves the books and shows the whole library
    b.delete_collection();
    assert_eq!(b.filter, Filter::All);
    assert_eq!(b.store.collections.len(), 1);
    assert_eq!(b.visible().len(), 2);
    b.flush();
    let again = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(again.store.collections.len(), 1);
    assert!(again.store.in_collection(0, &path("Beta")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_open_books_collections_are_in_the_menu_state() {
    let ui = ui();
    let dir = scratch("open-collections");
    let mut b = reader(&dir, &ui);
    b.library_search.set_text(&ui, "Travel");
    b.new_collection(&ui);
    assert_eq!(b.collection_menu(), vec![("Travel".to_string(), false)]);
    b.toggle_open_in_collection(0);
    assert_eq!(b.collection_menu(), vec![("Travel".to_string(), true)]);
    b.toggle_open_in_collection(0);
    assert_eq!(b.collection_menu(), vec![("Travel".to_string(), false)]);
    b.toggle_open_in_collection(7);
    assert_eq!(b.collection_menu().len(), 1, "an unknown collection changes nothing");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_context_menu_marks_a_book_and_only_acts_on_the_book_it_was_made_for() {
    let ui = ui();
    let dir = scratch("context");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A"), ("Beta", "B")]);
    assert!(!b.context_action(CTX_WANT), "no menu was shown");
    let (x, y) = centre(b.cover_rect(&ui, 0));
    let items = b.context_items(&ui, x, y);
    assert_eq!(items.iter().map(|i| i.id).collect::<Vec<_>>(), vec![CTX_OPEN, CTX_WANT, CTX_FINISHED]);
    assert!(b.context_action(CTX_WANT));
    assert_eq!(b.store.status_of(&path("Alpha")), Status::WantToRead);
    assert_eq!(b.store.status_of(&path("Beta")), Status::None);
    let items = b.context_items(&ui, x, y);
    assert_eq!(items[1].label, "Remove from Want to Read");
    assert!(b.context_action(CTX_FINISHED));
    assert_eq!(b.store.status_of(&path("Alpha")), Status::Finished, "a book has one status");
    let items = b.context_items(&ui, x, y);
    assert_eq!(items[2].label, "Mark as Unfinished");
    assert!(b.context_items(&ui, 2.0, 2.0).is_empty(), "the toolbar is not a book");
    assert!(!b.context_action(CTX_FINISHED), "the menu is spent");
    assert_eq!(b.store.status_of(&path("Alpha")), Status::Finished);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_library_caption_says_where_a_book_stands() {
    let ui = ui();
    let dir = scratch("caption");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A"), ("Beta", "B")]);
    b.store.set_status(&path("Alpha"), Status::Finished);
    b.store.set_status(&path("Beta"), Status::WantToRead);
    let entry = |t: &str| b.store.entry(&path(t)).map(|e| store::caption(e.status, e.progress)).unwrap();
    assert_eq!((entry("Alpha"), entry("Beta")), ("Finished".to_string(), "Want to Read".to_string()));
    // both views draw without trouble
    let grid = render(&mut b, &ui, 0.0);
    b.set_view_mode(ViewMode::List);
    assert_ne!(grid, render(&mut b, &ui, 1.0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_reading_goal_counts_the_time_a_book_is_open() {
    let ui = ui();
    let dir = scratch("goal");
    let mut b = reader(&dir, &ui);
    assert_eq!(b.goal_text(), None, "no goal, no line");
    b.set_reading_goal(30);
    assert_eq!(b.reading_goal(), 30);
    assert_eq!(b.goal_text(), Some("Today: 0 of 30 min".to_string()));
    b.tick(&ui, 1000.0);
    b.tick(&ui, 61_000.0);
    assert_eq!(b.store.seconds_on(store::today()), 60);
    assert_eq!(b.goal_text(), Some("Today: 1 of 30 min".to_string()));
    b.close_reader(62_000.0);
    assert_eq!(b.store.seconds_on(store::today()), 61, "closing counts the last second");
    b.tick(&ui, 500_000.0);
    b.tick(&ui, 900_000.0);
    assert_eq!(b.store.seconds_on(store::today()), 61, "a closed book is not read");
    b.set_reading_goal(1);
    assert_eq!(b.goal_text(), Some("Goal reached: 1 min today".to_string()));
    // the goal and the time are kept
    b.flush();
    let again = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(again.reading_goal(), 1);
    assert_eq!(again.store.seconds_on(store::today()), 61);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_goal_shows_in_the_reading_now_section() {
    let ui = ui();
    let dir = scratch("goal-draw");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A")]);
    b.store.touch(&path("Alpha"));
    let without = render(&mut b, &ui, 0.0);
    b.set_reading_goal(30);
    assert_ne!(without, render(&mut b, &ui, 1.0), "the goal line is drawn at the end of the Reading Now heading");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn adding_books_copies_them_into_the_books_folder_and_the_shelf() {
    let ui = ui();
    let dir = scratch("add");
    let folder = dir.join("Books");
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.join("data"), vec![folder.clone(), dir.join("Downloads")]);
    let bytes = tiny_epub();
    let mut source: &[u8] = &bytes;
    let added = b.import_book("Tiny.epub", &mut source).expect("an EPUB is accepted");
    assert_eq!(added, folder.join("Tiny.epub"));
    assert_eq!(std::fs::read(&added).unwrap(), bytes);
    assert!(b.shelf.iter().any(|c| c.path == added), "the book is on the shelf");
    let mut again: &[u8] = &bytes;
    assert_eq!(b.import_book("Tiny.epub", &mut again).unwrap(), folder.join("Tiny 2.epub"), "the first copy stays");
    let mut text: &[u8] = b"not a book";
    assert!(b.import_book("notes.txt", &mut text).is_err());
    assert_eq!(b.shelf.len(), 2);
    b.show_error("Books opens .epub and .pdf files, not notes.txt.".to_string());
    assert!(b.error.is_some());
    draw_ok(&mut b, &ui);
    let _ = std::fs::remove_dir_all(dir);
}

fn draw_ok(b: &mut Books, ui: &Ui) {
    let buf = render(b, ui, 0.0);
    assert!(buf.iter().any(|p| *p != buf[0]), "something is drawn");
}

#[test]
fn without_a_books_folder_books_are_added_to_the_private_folder() {
    let ui = ui();
    let dir = scratch("add-private");
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    let mut source: &[u8] = b"%PDF-1.4";
    assert_eq!(b.import_book("a.pdf", &mut source).unwrap(), dir.join("a.pdf"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn line_spacing_margins_and_justification_lay_the_pages_out_again_and_persist() {
    let ui = ui();
    let dir = scratch("spacing");
    let mut b = reader(&dir, &ui);
    let pages = |b: &Books| b.page().unwrap().1;
    let normal = pages(&b);
    b.set_line_spacing(&ui, 2);
    let loose = pages(&b);
    assert!(loose >= normal, "loose lines never need fewer pages");
    assert_eq!(b.metrics(&ui).line_height, store::SPACINGS[2].1);
    b.set_line_spacing(&ui, 0);
    let tight = pages(&b);
    assert!(tight <= normal, "tight lines never need more");
    assert!(tight < loose, "{tight} tight pages against {loose} loose ones");
    b.set_line_spacing(&ui, 1);
    assert_eq!(pages(&b), normal, "back to the normal layout");
    b.set_margins(&ui, 2);
    assert!(pages(&b) >= normal, "wide margins narrow the column");
    let wide = b.metrics(&ui).width;
    b.set_margins(&ui, 0);
    assert!(pages(&b) <= normal);
    assert!(b.metrics(&ui).width > wide);
    b.set_margins(&ui, 1);
    assert_eq!(pages(&b), normal);
    b.set_justified(&ui, true);
    assert_eq!(pages(&b), normal, "justifying moves no line to another page");
    b.set_line_spacing(&ui, 99);
    assert_eq!(b.settings().spacing, store::SPACINGS.len() - 1, "an unknown choice clamps");
    b.set_line_spacing(&ui, 1);
    let again = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(again.settings(), store::Settings { justify: true, ..store::Settings::default() });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_display_panel_buttons_change_the_reading_settings() {
    let ui = ui();
    let dir = scratch("panel");
    let mut b = reader(&dir, &ui);
    b.reader.as_mut().unwrap().panel = Some(Panel::Display);
    render(&mut b, &ui, 0.0);
    let button = |b: &Books, t: Target| b.display_buttons(&ui).into_iter().find(|x| x.0 == t).unwrap_or_else(|| panic!("{t:?}")).1;
    let (x, y) = centre(button(&b, Target::Spacing(2)));
    tap(&mut b, &ui, x, y, 100.0);
    assert_eq!(b.settings().spacing, 2);
    let (x, y) = centre(button(&b, Target::Margin(0)));
    tap(&mut b, &ui, x, y, 200.0);
    assert_eq!(b.settings().margin, 0);
    let (x, y) = centre(button(&b, Target::Justify));
    tap(&mut b, &ui, x, y, 300.0);
    assert!(b.settings().justify);
    render(&mut b, &ui, 350.0);
    tap(&mut b, &ui, x, y, 400.0);
    assert!(!b.settings().justify, "a second tap turns it off");
    // the buttons do not overlap the theme or size buttons
    let all = b.display_buttons(&ui);
    for (i, a) in all.iter().enumerate() {
        for c in all.iter().skip(i + 1) {
            let (ax, ay, aw, ah) = a.1;
            let (cx, cy, cw, ch) = c.1;
            let apart = ax + aw <= cx || cx + cw <= ax || ay + ah <= cy || cy + ch <= ay;
            assert!(apart, "{:?} overlaps {:?}", a.0, c.0);
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn only_lines_inside_a_paragraph_are_stretched_when_justified() {
    let ui = ui();
    let dir = scratch("justify");
    let b = reader(&dir, &ui);
    let m = b.metrics(&ui);
    let pt = Books::pt_of(&ui, m.body);
    let r = b.reader.as_ref().unwrap();
    let mut stretched = 0;
    for line in &r.layout.pages[0].lines {
        let text = r.book.blocks[line.block].text().unwrap_or("");
        assert_eq!(line_extra(&ui, false, text, line, pt, m.width), 0.0, "off means off");
        let extra = line_extra(&ui, true, text, line, pt, m.width);
        let inside = matches!(line.kind, layout::Kind::Body) && !layout::ends_paragraph(text, line.offset, &line.text);
        if inside {
            assert!(extra >= 0.0);
            stretched += (extra > 0.0) as usize;
        } else {
            assert_eq!(extra, 0.0, "{:?} stays ragged", line.text);
        }
    }
    assert!(stretched > 0, "some line of the page is stretched");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_justified_page_is_drawn_differently() {
    let ui = ui();
    let dir = scratch("justify-draw");
    let mut b = reader(&dir, &ui);
    let ragged = render(&mut b, &ui, 0.0);
    b.set_justified(&ui, true);
    let justified = render(&mut b, &ui, 1.0);
    assert_ne!(ragged, justified);
    b.set_justified(&ui, false);
    assert_eq!(ragged, render(&mut b, &ui, 2.0), "and the same again when it is off");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn highlight_colours_and_the_underline_draw_differently() {
    let ui = ui();
    let dir = scratch("colours");
    let mut b = reader(&dir, &ui);
    let book = PathBuf::from("/virtual/long.epub");
    let span = Span { block: 1, offset: 0, len: 9 };
    b.store.toggle_highlight(&book, span);
    let yellow = render(&mut b, &ui, 0.0);
    let mut seen = vec![yellow.clone()];
    for style in 1..store::HIGHLIGHT_STYLES.len() as u8 {
        b.store.set_highlight_style(&book, span, style);
        let drawn = render(&mut b, &ui, 1.0 + style as f64);
        assert!(!seen.contains(&drawn), "{} looks like an earlier style", store::HIGHLIGHT_STYLES[style as usize]);
        seen.push(drawn);
    }
    b.store.set_highlight_style(&book, span, 0);
    assert_eq!(yellow, render(&mut b, &ui, 9.0), "yellow is the default");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_highlight_color_menu_styles_the_text_the_last_find_went_to() {
    let ui = ui();
    let dir = scratch("match-style");
    let mut b = reader(&dir, &ui);
    let book = PathBuf::from("/virtual/long.epub");
    assert!(!b.set_match_style(2), "no match, nothing to colour");
    assert_eq!(b.match_style(), None);
    b.reader.as_mut().unwrap().highlight = Some((1, 0, 9));
    assert_eq!(b.match_style(), None, "the match is not highlighted yet");
    assert!(b.set_match_style(2));
    let span = Span { block: 1, offset: 0, len: 9 };
    assert_eq!(b.store.highlight_style(&book, span), 2);
    assert_eq!(b.match_style(), Some(2), "choosing a colour highlights the match");
    assert!(b.set_match_style(store::UNDERLINE));
    assert_eq!(b.match_style(), Some(store::UNDERLINE));
    assert!(!b.toggle_match_highlight(), "the highlight toggles off");
    assert_eq!(b.match_style(), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pdf_highlight_annotations_carry_their_style() {
    let ui = ui();
    let dir = scratch("pdf-style");
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    let book = PathBuf::from("/virtual/long.pdf");
    b.show_book(&ui, book.clone(), long_book());
    let one = Span { block: 1, offset: 0, len: 4 };
    let two = Span { block: 2, offset: 0, len: 4 };
    b.store.set_highlight_style(&book, one, 3);
    b.store.toggle_highlight(&book, two);
    let page = b.reader.as_ref().unwrap().layout.page_of(Pos { block: 1, offset: 0 });
    let marks = b.pdf_highlight_marks(&ui, page).expect("a PDF keeps its highlights as annotations");
    let styles: Vec<u8> = marks.iter().map(|(_, style)| *style).collect();
    assert_eq!(styles, vec![3, 0], "pink, then the default yellow");
    // and a justified page moves the run along with its stretched line
    b.set_justified(&ui, true);
    let page = b.reader.as_ref().unwrap().layout.page_of(Pos { block: 1, offset: 0 });
    assert!(b.pdf_highlight_marks(&ui, page).is_some());
    draw_ok(&mut b, &ui);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_cover_tile_is_scaled_again_when_its_size_changes() {
    let ui = ui();
    let dir = scratch("tile");
    let mut b = shelf(&ui, &dir, &[("Alpha", "A")]);
    b.shelf[0].image = Some(Image::new(40, 60));
    render(&mut b, &ui, 0.0);
    let grid_tile = b.shelf[0].tile.as_ref().map(|t| (t.w, t.h)).expect("the grid made a tile");
    b.set_view_mode(ViewMode::List);
    render(&mut b, &ui, 1.0);
    let list_tile = b.shelf[0].tile.as_ref().map(|t| (t.w, t.h)).expect("the list made a tile");
    assert_ne!(grid_tile, list_tile, "the list uses a smaller cover");
    let _ = std::fs::remove_dir_all(dir);
}
