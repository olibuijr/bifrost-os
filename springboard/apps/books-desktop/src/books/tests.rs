use super::engine::zip::tests::build;
use super::engine::{Block, Book, TocEntry};
use super::*;

fn ui() -> Ui {
    springboard::shell::tests::ui()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-books-app")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tap(b: &mut Books, ui: &Ui, x: f32, y: f32, t: f64) {
    b.touch(ui, Event::Down(x, y), t);
    b.touch(ui, Event::Up(x, y), t + 30.0);
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn long_book() -> Book {
    let mut blocks = vec![Block::Heading(1, "Chapter One".into())];
    for i in 0..120 {
        blocks.push(Block::Para(format!("Paragraph {i}. The quick brown fox jumps over the lazy dog while the needle{i} waits quietly for somebody to read this somewhat long sentence all the way to the end of the line and beyond.")));
    }
    blocks.push(Block::Heading(1, "Chapter Two".into()));
    blocks.push(Block::Para("The second chapter begins here.".into()));
    let toc = vec![TocEntry { title: "Chapter One".into(), block: 0, level: 0 }, TocEntry { title: "Chapter Two".into(), block: 121, level: 0 }];
    Book { title: "Long Book".into(), author: "A. Tester".into(), blocks, toc, cover: None }
}

fn reader(dir: &Path, ui: &Ui) -> Books {
    let mut b = Books::with_library_dirs(ui, 1200, 1920, dir.to_path_buf(), Vec::new());
    b.show_book(ui, PathBuf::from("/virtual/long.epub"), long_book());
    b
}

fn draw(b: &mut Books, ui: &Ui, t: f64) {
    let mut buf = vec![0u32; 1200 * 1920];
    b.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut buf, y0: 0 }, ui, t);
}

fn wait_until(b: &mut Books, ui: &Ui, what: &str, done: impl Fn(&Books) -> bool) {
    for i in 0..400 {
        bifrost_app::dispatch::drain_main();
        b.tick(ui, 1000.0 + i as f64 * 10.0);
        if done(b) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

fn tiny_epub() -> Vec<u8> {
    let container = r#"<container><rootfiles><rootfile full-path="c.opf"/></rootfiles></container>"#;
    let opf = r#"<package><metadata><dc:title>Tiny Book</dc:title><dc:creator>Me</dc:creator></metadata><manifest><item id="a" href="a.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#;
    let a = "<html><body><h1>Hello</h1><p>A tiny book for the library test.</p></body></html>";
    build(&[("META-INF/container.xml", 0, container.as_bytes(), container.len()), ("c.opf", 0, opf.as_bytes(), opf.len()), ("a.xhtml", 0, a.as_bytes(), a.len())])
}

#[test]
fn library_probes_titles_and_opens_a_book() {
    let ui = ui();
    let dir = scratch("library");
    std::fs::write(dir.join("tiny_book.epub"), tiny_epub()).unwrap();
    std::fs::write(dir.join("notes.txt"), b"not a book").unwrap();
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(b.shelf_titles(), vec!["tiny book"], "file name until the probe lands");
    wait_until(&mut b, &ui, "the probe", |b| b.shelf_titles() == vec!["Tiny Book"]);
    draw(&mut b, &ui, 0.0);
    let (x, y) = centre(b.cover_rect(&ui, 0));
    tap(&mut b, &ui, x, y, 5000.0);
    assert!(b.describe().contains("books_loading=true"));
    wait_until(&mut b, &ui, "the reader", |b| b.reading().is_some());
    assert!(b.describe().contains("reader:Tiny Book:1/1"));
    draw(&mut b, &ui, 0.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_missing_book_reports_an_error_and_stays_in_the_library() {
    let ui = ui();
    let dir = scratch("error");
    let mut b = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    b.open_path(&dir.join("gone.epub"));
    wait_until(&mut b, &ui, "the failure", |b| b.error.is_some());
    assert!(b.reading().is_none());
    draw(&mut b, &ui, 0.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn taps_and_swipes_turn_pages_and_the_centre_toggles_the_bars() {
    let ui = ui();
    let dir = scratch("turn");
    let mut b = reader(&dir, &ui);
    let (_, total) = b.page().unwrap();
    assert!(total > 5, "{total} pages");
    assert!(b.fullscreen(), "bars start hidden");
    tap(&mut b, &ui, 1100.0, 900.0, 100.0);
    assert_eq!(b.page(), Some((1, total)));
    tap(&mut b, &ui, 100.0, 900.0, 200.0);
    assert_eq!(b.page(), Some((0, total)));
    tap(&mut b, &ui, 100.0, 900.0, 300.0);
    assert_eq!(b.page(), Some((0, total)), "no page before the first");
    b.touch(&ui, Event::Down(900.0, 900.0), 400.0);
    b.touch(&ui, Event::Move(600.0, 910.0), 410.0);
    b.touch(&ui, Event::Up(300.0, 920.0), 420.0);
    assert_eq!(b.page(), Some((1, total)), "swipe left = next");
    b.touch(&ui, Event::Down(300.0, 900.0), 500.0);
    b.touch(&ui, Event::Up(900.0, 900.0), 520.0);
    assert_eq!(b.page(), Some((0, total)), "swipe right = previous");
    tap(&mut b, &ui, 600.0, 900.0, 600.0);
    assert!(!b.fullscreen(), "centre tap shows the bars");
    draw(&mut b, &ui, 700.0);
    tap(&mut b, &ui, 600.0, 900.0, 800.0);
    assert!(b.fullscreen());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_reading_position_is_saved_and_resumed() {
    let ui = ui();
    let dir = scratch("resume");
    let mut b = reader(&dir, &ui);
    for i in 0..4 {
        tap(&mut b, &ui, 1100.0, 900.0, 100.0 + i as f64 * 100.0);
    }
    let (page, _) = b.page().unwrap();
    assert_eq!(page, 4);
    let start = b.reader.as_ref().unwrap().layout.pos_of(page);
    b.flush();
    let again = reader(&dir, &ui);
    assert_eq!(again.reader.as_ref().unwrap().layout.pos_of(again.page().unwrap().0), start);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn bookmarks_toggle_list_and_jump() {
    let ui = ui();
    let dir = scratch("marks");
    let mut b = reader(&dir, &ui);
    for i in 0..3 {
        tap(&mut b, &ui, 1100.0, 900.0, 100.0 + i as f64 * 100.0);
    }
    tap(&mut b, &ui, 600.0, 900.0, 500.0); // bars
    let mark = b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavMark).unwrap().2;
    let (mx, my) = centre(mark);
    tap(&mut b, &ui, mx, my, 600.0);
    assert_eq!(b.page_marks(3).len(), 1);
    draw(&mut b, &ui, 650.0);
    // go elsewhere, open the bookmarks list and jump back
    b.goto_page(0, 700.0);
    let contents = centre(b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavContents).unwrap().2);
    tap(&mut b, &ui, contents.0, contents.1, 800.0);
    let tab = b.tab_rects(&ui).into_iter().find(|t| t.0 == Panel::Bookmarks).unwrap().1;
    let (tx, ty) = centre(tab);
    tap(&mut b, &ui, tx, ty, 900.0);
    assert_eq!(b.panel_rows(), 1);
    draw(&mut b, &ui, 950.0);
    let (rx, ry) = centre(b.row_rect(&ui, 0));
    tap(&mut b, &ui, rx - 100.0, ry, 1000.0);
    assert_eq!(b.page().unwrap().0, 3, "the row jumps to the bookmarked page");
    assert!(b.reader.as_ref().unwrap().panel.is_none());
    // toggling again removes it
    let (mx, my) = centre(b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavMark).unwrap().2);
    tap(&mut b, &ui, mx, my, 1200.0);
    assert!(b.page_marks(3).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn contents_jump_to_a_chapter() {
    let ui = ui();
    let dir = scratch("toc");
    let mut b = reader(&dir, &ui);
    tap(&mut b, &ui, 600.0, 900.0, 100.0);
    let (cx, cy) = centre(b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavContents).unwrap().2);
    tap(&mut b, &ui, cx, cy, 200.0);
    assert_eq!(b.panel_rows(), 2);
    draw(&mut b, &ui, 250.0);
    let (rx, ry) = centre(b.row_rect(&ui, 1));
    tap(&mut b, &ui, rx, ry, 300.0);
    let r = b.reader.as_ref().unwrap();
    assert!(r.layout.pages[r.page].lines.iter().any(|l| l.block == 121), "Chapter Two is on screen");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn search_finds_text_jumps_to_it_and_highlights() {
    let ui = ui();
    let dir = scratch("search");
    let mut b = reader(&dir, &ui);
    tap(&mut b, &ui, 600.0, 900.0, 100.0);
    let (sx, sy) = centre(b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavSearch).unwrap().2);
    tap(&mut b, &ui, sx, sy, 200.0);
    assert!(b.reader.as_ref().unwrap().search.focused, "the search field has focus");
    for c in "needle77".chars() {
        b.key(&ui, Out::Text(c.to_string()), 300.0);
    }
    b.key(&ui, Out::Enter, 400.0);
    assert_eq!(b.panel_rows(), 1);
    draw(&mut b, &ui, 450.0);
    let (rx, ry) = centre(b.row_rect(&ui, 0));
    tap(&mut b, &ui, rx, ry, 500.0);
    let r = b.reader.as_ref().unwrap();
    assert!(r.panel.is_none());
    let (block, _, len) = r.highlight.unwrap();
    assert_eq!((block, len), (78, 8));
    assert!(r.layout.pages[r.page].lines.iter().any(|l| l.block == 78), "the hit is on the shown page");
    draw(&mut b, &ui, 600.0);
    // a miss reports nothing
    let (sx, sy) = centre(b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavSearch).unwrap().2);
    tap(&mut b, &ui, sx, sy, 800.0);
    b.reader.as_mut().unwrap().search.set_text(&ui, "zebra");
    b.key(&ui, Out::Enter, 900.0);
    assert_eq!(b.panel_rows(), 0);
    draw(&mut b, &ui, 950.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn font_size_and_theme_change_repaginate_and_persist() {
    let ui = ui();
    let dir = scratch("display");
    let mut b = reader(&dir, &ui);
    for i in 0..5 {
        tap(&mut b, &ui, 1100.0, 900.0, 100.0 + i as f64 * 100.0);
    }
    let before = b.reader.as_ref().unwrap().layout.pos_of(b.page().unwrap().0);
    let pages_before = b.page().unwrap().1;
    tap(&mut b, &ui, 600.0, 900.0, 700.0);
    let (dx, dy) = centre(b.nav_buttons(&ui).into_iter().find(|b| b.0 == Target::NavDisplay).unwrap().2);
    tap(&mut b, &ui, dx, dy, 800.0);
    draw(&mut b, &ui, 850.0);
    let up = b.display_buttons(&ui).into_iter().find(|x| x.0 == Target::SizeUp).unwrap().1;
    let (ux, uy) = centre(up);
    tap(&mut b, &ui, ux, uy, 900.0);
    assert_eq!(b.settings().size, store::DEFAULT_SIZE + 1);
    assert!(b.page().unwrap().1 > pages_before, "bigger text needs more pages");
    let after = b.reader.as_ref().unwrap().layout.pos_of(b.page().unwrap().0);
    assert!((after.block, after.offset) <= (before.block, before.offset) || after.block == before.block);
    let theme = b.display_buttons(&ui).into_iter().find(|x| x.0 == Target::ThemeButton(2)).unwrap().1;
    let (tx, ty) = centre(theme);
    tap(&mut b, &ui, tx, ty, 1000.0);
    assert_eq!(b.settings().theme, Theme::Dark);
    draw(&mut b, &ui, 1050.0);
    let again = Books::with_library_dirs(&ui, 1200, 1920, dir.clone(), Vec::new());
    assert_eq!(again.settings(), store::Settings { size: store::DEFAULT_SIZE + 1, theme: Theme::Dark });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn back_returns_to_the_library_and_keeps_the_place() {
    let ui = ui();
    let dir = scratch("back");
    let mut b = reader(&dir, &ui);
    tap(&mut b, &ui, 1100.0, 900.0, 100.0);
    tap(&mut b, &ui, 600.0, 900.0, 200.0);
    let (x, y) = centre(b.back_rect(&ui));
    tap(&mut b, &ui, x, y, 300.0);
    assert!(b.reading().is_none());
    assert!(b.describe().contains("books_screen=library"));
    let e = b.store.entry(Path::new("/virtual/long.epub")).unwrap();
    assert!(e.pos != Pos::default() && e.progress < 1000);
    draw(&mut b, &ui, 400.0);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn host_library_folders_open_documents_and_flush_private_state() {
    let ui = ui();
    let dir = scratch("host-folders");
    let state = dir.join("private");
    let documents = dir.join("Documents");
    std::fs::create_dir_all(&documents).unwrap();
    let book = documents.join("selected book.epub");
    std::fs::write(&book, tiny_epub()).unwrap();
    let mut app = Books::with_library_dirs(&ui, 1000, 720, state.clone(), vec![documents.clone(), documents.clone()]);
    assert_eq!(app.dirs, vec![state.clone(), documents]);
    wait_until(&mut app, &ui, "host library probe", |app| app.shelf_titles() == vec!["Tiny Book"]);
    app.open_path(&book);
    wait_until(&mut app, &ui, "host document", |app| app.reading().is_some());
    assert_eq!(app.reading(), Some(book.as_path()));
    app.flush();
    assert!(state.join("state.tsv").is_file());
    drop(app);
    let reopened = Books::with_library_dirs(&ui, 900, 600, state, Vec::new());
    assert!(reopened.shelf.iter().any(|entry| entry.path == book));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn add_note_opens_the_notes_panel_and_saves_what_is_typed() {
    let ui = ui();
    let dir = scratch("add-note");
    let mut b = reader(&dir, &ui);
    // a find went to the word "Chapter" at the start of the second chapter's heading (block 121)
    let span = Span { block: 121, offset: 0, len: 7 };
    b.reader.as_mut().unwrap().highlight = Some((span.block, span.offset, span.len));
    b.add_note(&ui);
    {
        let r = b.reader.as_ref().unwrap();
        assert_eq!(r.panel, Some(Panel::Notes));
        assert!(r.note.focused, "the note field takes the typing");
        assert_eq!(r.note_target, Some(span));
    }
    assert!(b.match_span().unwrap().1, "Add Note highlights the match");
    for c in "Hello".chars() {
        b.key(&ui, Out::Text(c.to_string()), 2000.0);
    }
    b.key(&ui, Out::Enter, 2100.0);
    assert_eq!(b.store.note_of(Path::new("/virtual/long.epub"), span).as_deref(), Some("Hello"));
    assert!(!b.reader.as_ref().unwrap().note.focused, "Return ends the edit");
    assert_eq!(b.panel_rows(), 1);
    // tapping the note's row goes to its page and closes the panel
    let want = b.reader.as_ref().unwrap().layout.page_of(Pos { block: span.block, offset: span.offset });
    b.activate(&ui, Target::Row(0), 0.0, 3000.0);
    assert_eq!(b.page().unwrap().0, want);
    assert!(b.reader.as_ref().unwrap().panel.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn pages_row_jumps_to_its_page() {
    let ui = ui();
    let dir = scratch("pages");
    let mut b = reader(&dir, &ui);
    let pages = b.reader.as_ref().unwrap().layout.pages.len();
    assert!(pages > 3, "the long book paginates into several pages");
    b.reader.as_mut().unwrap().panel = Some(Panel::Pages);
    assert_eq!(b.panel_rows(), pages, "one row per page");
    b.activate(&ui, Target::Row(2), 0.0, 1000.0);
    assert_eq!(b.page().unwrap().0, 2);
    assert!(b.reader.as_ref().unwrap().panel.is_none());
    let _ = std::fs::remove_dir_all(dir);
}
