//! Books as an installed BifrOSt app: the former in-shell PDF/EPUB reader,
//! unchanged, hosted through the SDK. Typing comes from the hardware keyboard.

mod books;

use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, TouchKind, KeyInput};
use springboard::shell::keyboard::Out;
use std::path::{Path, PathBuf};

const MENU_PRINT: u32 = 1;
const MENU_FIND_NEXT: u32 = 2;
const MENU_FIND_PREVIOUS: u32 = 3;
const MENU_HIGHLIGHT: u32 = 4;
const MENU_WHOLE_WORDS: u32 = 5;
const MENU_COPY_QUOTE: u32 = 6;

/// How the shelf reaches the search index: a book is sent when it is new or changed (see `books::search_index`).
struct BooksApp {
    books: Option<books::Books>,
    index: books::search_index::SearchIndex,
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
        let open = self.books.as_ref().is_some_and(|b| b.reading().is_some());
        let hits = self.books.as_ref().is_some_and(|b| b.has_hits());
        let highlight = self.books.as_ref().and_then(|b| b.match_span());
        let whole = self.books.as_ref().is_some_and(|b| b.whole_words());
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![AppMenuItem::new(MENU_PRINT, "Print...").key("cmd+p").enabled(open && bifrost_app::print::available())],
            },
            AppMenu {
                title: "Edit".into(),
                items: vec![AppMenuItem::new(MENU_COPY_QUOTE, "Copy Quote").key("cmd+shift+c").enabled(highlight.is_some())],
            },
            AppMenu {
                title: "Find".into(),
                items: vec![
                    AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(hits),
                    AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(hits),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_HIGHLIGHT, "Highlight Match").key("cmd+shift+y").enabled(highlight.is_some()).checked(highlight.is_some_and(|(_, saved)| saved)),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_WHOLE_WORDS, "Match Whole Words").enabled(open).checked(whole),
                ],
            },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        if id == MENU_COPY_QUOTE {
            // a quotation goes to the shared clipboard as plain text, with its citation; no frame is needed
            if let Some(quotation) = self.books.as_ref().and_then(|b| b.quotation()) {
                books::quote::copy(ctx, &quotation);
            }
            return false;
        }
        if matches!(id, MENU_FIND_NEXT | MENU_FIND_PREVIOUS | MENU_HIGHLIGHT | MENU_WHOLE_WORDS) {
            let Some(books) = &mut self.books else { return false };
            let t = bifrost_app::proto::sys::now_ms();
            match id {
                MENU_FIND_NEXT => books.find_step(1, t),
                MENU_FIND_PREVIOUS => books.find_step(-1, t),
                MENU_HIGHLIGHT => {
                    books.toggle_match_highlight();
                }
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
        // PDF books print as-is; other formats have no print rendering yet (docs/bifrost-ui.md "Printing")
        let Some(path) = self.books.as_ref().and_then(|b| b.reading()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"))) else { return false };
        let _ = bifrost_app::print::PrintOperation::new(path).run();
        false
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
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
