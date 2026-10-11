use super::fetch::tests::Fake;
use super::layout::Item;
use super::*;
use std::path::Path;

const LOREM: &str = "The quick brown fox jumps over the lazy dog while the rest of the village sleeps, and nobody notices.";
const PAGE1: &str = "<html><head><title>First</title></head><body><h1>Hello</h1><p>Visit <a href=\"/two\">the second page</a> now.</p></body></html>";
const PAGE2: &str = "<title>Second</title><p>Page two</p>";

fn ui() -> Ui {
    crate::shell::tests::ui()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-browser-app")).join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn wall() -> Wall {
    Wall { unix: 1_791_073_380, offset: 0, at: 0.0 }
}

fn browser(ui: &Ui, dir: &Path, sites: &[(&str, &str, &[u8])]) -> Browser {
    Browser::new(ui, 1200, 1920, dir.to_path_buf(), Fake::new(sites), wall(), 0.0)
}

fn html_site<'a>(key: &'a str, body: &'a str) -> (&'a str, &'a str, &'a [u8]) {
    (key, "text/html; charset=utf-8", body.as_bytes())
}

/// Ticks until the active tab has finished loading and the loader is idle.
fn settle(b: &mut Browser, ui: &Ui, t0: f64) -> f64 {
    let mut t = t0;
    for _ in 0..600 {
        t += 10.0;
        b.tick(ui, t);
        if b.tabs[b.active].loading.is_none() && !b.loader.busy() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    t += 10.0;
    b.tick(ui, t);
    t
}

fn tap(b: &mut Browser, ui: &Ui, x: f32, y: f32, t: f64) -> f64 {
    b.touch(ui, Event::Down(x, y), t);
    b.touch(ui, Event::Up(x, y), t + 40.0);
    t + 40.0
}

fn centre((x, y, w, h): (f32, f32, f32, f32)) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

fn tap_rect(b: &mut Browser, ui: &Ui, rect: impl FnOnce(&Browser, &Ui) -> (f32, f32, f32, f32), t: f64) -> f64 {
    let (x, y) = centre(rect(b, ui));
    tap(b, ui, x, y, t)
}

fn title(b: &Browser) -> String {
    b.tabs[b.active].view.as_ref().map(|v| v.title.clone()).unwrap_or_default()
}

/// Screen position of the first word of page text matching `pred`.
fn find_word(b: &Browser, ui: &Ui, pred: impl Fn(&str, Option<usize>) -> bool) -> (f32, f32) {
    let v = b.tabs[b.active].view.as_ref().unwrap();
    v.page
        .items
        .iter()
        .find_map(|i| match i {
            Item::Text { x, y, w, h, text, link, .. } if pred(text, *link) => Some(((x + w / 2.0) * ui.s, b.content_top(ui) + (y + h / 2.0) * ui.s - b.tabs[b.active].scroll.offset())),
            _ => None,
        })
        .expect("a matching word on the page")
}

#[test]
fn it_opens_on_the_start_page() {
    let ui = ui();
    let b = browser(&ui, &scratch("start"), &[]);
    assert_eq!(b.tabs.len(), 1);
    assert_eq!(title(&b), "Start");
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("Browser"));
    assert_eq!(b.shown_url(), "", "the bar shows its placeholder on the start page");
    let d = b.describe();
    assert!(d.contains("browser_url=about:home") && d.contains("browser_tabs=1") && d.contains("browser_loading=false"));
}

#[test]
fn typing_an_address_loads_it_and_records_history() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("typing"), &[html_site("x.test/", PAGE1)]);
    let t = tap_rect(&mut b, &ui, |b, ui| b.url_rect(ui), 100.0);
    assert!(b.editing && b.keyboard_shown());
    b.key(&ui, Out::Text("x.test".into()), t + 10.0);
    b.key(&ui, Out::Enter, t + 20.0);
    assert!(!b.editing && b.tabs[0].loading.is_some());
    settle(&mut b, &ui, t + 20.0);
    assert_eq!(title(&b), "First");
    assert_eq!(b.shown_url(), "x.test");
    assert_eq!(b.lib.history.len(), 1);
    assert_eq!(b.lib.history[0].title, "First");
    assert!(b.describe().contains("browser_title=First"));
}

#[test]
fn a_search_query_goes_to_the_search_engine() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("search"), &[html_site("duckduckgo.com/html/", "<title>Results</title><p>found</p>")]);
    let t = tap_rect(&mut b, &ui, |b, ui| b.url_rect(ui), 100.0);
    b.key(&ui, Out::Text("rust lifetimes".into()), t + 10.0);
    b.key(&ui, Out::Enter, t + 20.0);
    settle(&mut b, &ui, t + 20.0);
    assert_eq!(title(&b), "Results");
    assert!(b.tabs[0].view.as_ref().unwrap().url.ends_with("q=rust+lifetimes"));
}

#[test]
fn tapping_a_link_navigates_and_back_and_forward_restore_pages() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("links"), &[html_site("x.test/", PAGE1), html_site("x.test/two", PAGE2)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    let (x, y) = find_word(&b, &ui, |_, link| link.is_some());
    let t = tap(&mut b, &ui, x, y, t + 100.0);
    let t = settle(&mut b, &ui, t);
    assert_eq!(title(&b), "Second");
    assert_eq!(b.tabs[0].view.as_ref().unwrap().url, "http://x.test/two");
    assert_eq!((b.tabs[0].entries.len(), b.tabs[0].pos), (3, 2));
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 1), t + 100.0);
    assert!(b.tabs[0].loading.is_none(), "Back is served from the page cache");
    assert_eq!(title(&b), "First");
    assert_eq!(b.tabs[0].pos, 1);
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 2), t + 100.0);
    assert_eq!(title(&b), "Second");
    assert_eq!(b.tabs[0].pos, 2);
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 2), t + 100.0);
    assert_eq!(b.tabs[0].pos, 2, "nothing forward of the last page");
    let _ = t;
}

#[test]
fn back_reloads_when_the_page_is_no_longer_cached() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("reload-back"), &[html_site("x.test/", PAGE1), html_site("x.test/two", PAGE2)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    settle(&mut b, &ui, 100.0);
    b.navigate(&ui, "http://x.test/two", 1000.0);
    let t = settle(&mut b, &ui, 1000.0);
    b.tabs[0].cache.clear();
    b.go(&ui, -1, t + 10.0);
    assert!(b.tabs[0].loading.is_some());
    settle(&mut b, &ui, t + 10.0);
    assert_eq!(title(&b), "First");
}

#[test]
fn a_late_answer_to_an_abandoned_navigation_is_ignored() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("stale"), &[html_site("x.test/", PAGE1), html_site("x.test/two", PAGE2)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    b.navigate(&ui, "http://x.test/two", 110.0);
    settle(&mut b, &ui, 110.0);
    assert_eq!(title(&b), "Second");
    assert_eq!(b.tabs[0].entries.len(), 3, "start page, then both navigations");
}

#[test]
fn unreachable_pages_show_an_error_that_is_not_remembered() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("error"), &[]);
    b.navigate(&ui, "http://nowhere.test/", 100.0);
    settle(&mut b, &ui, 100.0);
    assert_eq!(title(&b), "Can't open page");
    assert!(b.tabs[0].view.as_ref().unwrap().failed);
    assert!(b.lib.history.is_empty());
    b.toggle_bookmark(500.0);
    assert!(b.lib.bookmarks.is_empty(), "an error page cannot be bookmarked");
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("no such host"));
}

#[test]
fn reader_mode_shows_only_the_article_and_toggles_back() {
    let ui = ui();
    let page = format!("<title>Story</title><nav>site menu</nav><article><h1>Big News</h1><p>{LOREM}</p><p>{LOREM}</p></article><div id=footer>legal</div>");
    let mut b = browser(&ui, &scratch("reader"), &[html_site("x.test/", &page)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("site menu"));
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 10), t + 50.0);
    let v = b.tabs[0].view.as_ref().unwrap();
    assert!(v.reader_on);
    let text = v.page.text();
    assert!(text.contains("quick brown fox") && text.contains("Big News"));
    assert!(!text.contains("site menu") && !text.contains("legal"));
    assert_eq!(v.page.background, Some(css::Color::rgb(0xf7, 0xf3, 0xea)));
    assert!(b.describe().contains("browser_reader=true"));
    tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 10), t + 50.0);
    assert!(!b.tabs[0].view.as_ref().unwrap().reader_on);
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("site menu"));
}

#[test]
fn reader_mode_declines_pages_without_an_article() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("no-reader"), &[]);
    tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 10), 100.0);
    assert!(!b.tabs[0].view.as_ref().unwrap().reader_on);
    assert!(b.toast.as_ref().is_some_and(|(t, _)| t.contains("No article")));
}

#[test]
fn bookmarks_toggle_persist_list_and_remove() {
    let ui = ui();
    let dir = scratch("bookmarks");
    let mut b = browser(&ui, &dir, &[html_site("x.test/", PAGE1)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, -3), t + 50.0);
    assert_eq!(b.lib.bookmarks.len(), 1);
    assert!(b.toast.as_ref().is_some_and(|(t, _)| t == "Bookmark added"));
    assert!(browser(&ui, &dir, &[]).lib.is_bookmarked("http://x.test/"), "saved to disk");
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 0), t + 50.0);
    assert_eq!(title(&b), "Bookmarks");
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("First"));
    b.navigate(&ui, "about:remove-bookmark?u=http%3A%2F%2Fx.test%2F", t + 50.0);
    assert!(b.lib.bookmarks.is_empty());
    assert_eq!(title(&b), "Bookmarks");
    assert_eq!(b.tabs[0].entries[b.tabs[0].pos].url, "about:bookmarks", "the action leaves no trace in history");
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("No bookmarks yet"));
}

#[test]
fn history_page_lists_visits_and_clears() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("history"), &[html_site("x.test/", PAGE1)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    b.navigate(&ui, nav::HISTORY, t + 10.0);
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("First"));
    b.navigate(&ui, "about:clear-history", t + 20.0);
    assert!(b.lib.history.is_empty());
    assert!(b.tabs[0].view.as_ref().unwrap().page.text().contains("No history"));
}

#[test]
fn tabs_open_switch_close_and_come_back_next_session() {
    let ui = ui();
    let dir = scratch("tabs");
    let mut b = browser(&ui, &dir, &[html_site("x.test/", PAGE1)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    let (new_x, new_y) = (b.w as f32 - ui.pt(77.0), b.tool_top(&ui) + ui.pt(25.0));
    let bar_y = b.tab_top(&ui) + ui.pt(10.0);
    let t = tap(&mut b, &ui, new_x, new_y, t + 50.0);
    assert_eq!((b.tabs.len(), b.active), (2, 1));
    assert_eq!(title(&b), "Start");
    let tw = b.tab_w(&ui);
    let t = tap(&mut b, &ui, ui.pt(100.0), bar_y, t + 50.0);
    assert_eq!(b.active, 0);
    assert_eq!(title(&b), "First");
    b.flush();
    let b2 = browser(&ui, &dir, &[html_site("x.test/", PAGE1)]);
    assert_eq!((b2.tabs.len(), b2.active), (2, 0));
    assert!(b2.tabs[1].view.is_none(), "background tabs load when selected");
    let t = tap(&mut b, &ui, tw + ui.pt(18.0), bar_y, t + 50.0);
    assert_eq!(b.tabs.len(), 1, "the close button of the second tab");
    let t = tap(&mut b, &ui, ui.pt(18.0), bar_y, t + 50.0);
    assert_eq!((b.tabs.len(), b.active), (1, 0), "closing the last tab opens a fresh one");
    assert_eq!(title(&b), "Start");
    let _ = t;
}

#[test]
fn suggestions_come_from_history_and_bookmarks() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("suggest"), &[html_site("example.com/", "<title>Example Domain</title><p>hi</p>")]);
    b.lib.visit("https://example.com/", "Example Domain", 1);
    b.lib.toggle_bookmark("Rust docs", "https://doc.rust-lang.org/");
    let t = tap_rect(&mut b, &ui, |b, ui| b.url_rect(ui), 100.0);
    b.key(&ui, Out::Text("exa".into()), t + 10.0);
    assert_eq!(b.suggestions.iter().map(|s| s.url.as_str()).collect::<Vec<_>>(), ["https://example.com/"]);
    assert_eq!(b.row_count(), 2, "the typed text and one suggestion");
    let mut px = vec![0u32; 1200 * 1920];
    b.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut px, y0: 0 }, &ui, t + 20.0);
    let t = tap_rect(&mut b, &ui, |b, ui| b.row_rect(ui, 1), t + 100.0);
    assert!(!b.editing);
    settle(&mut b, &ui, t);
    assert_eq!(title(&b), "Example Domain");
}

#[test]
fn tapping_below_the_suggestions_dismisses_the_keyboard() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("dismiss"), &[]);
    let t = tap_rect(&mut b, &ui, |b, ui| b.url_rect(ui), 100.0);
    assert!(b.editing);
    let y = b.content_top(&ui) + ui.pt(400.0);
    tap(&mut b, &ui, 600.0, y, t + 100.0);
    assert!(!b.editing && !b.keyboard.shown());
}

#[test]
fn long_press_selects_a_paragraph_and_copy_puts_it_on_the_clipboard() {
    let _g = clipboard::tests::exclusive();
    let ui = ui();
    let mut b = browser(&ui, &scratch("copy"), &[html_site("x.test/", PAGE1)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0) + 100.0;
    let (x, y) = find_word(&b, &ui, |w, _| w == "Hello");
    b.touch(&ui, Event::Down(x, y), t);
    b.tick(&ui, t + 600.0);
    assert!(matches!(b.tabs[0].selection, Some(Sel::Block(_))));
    b.touch(&ui, Event::Up(x, y), t + 650.0);
    assert!(b.tabs[0].selection.is_some(), "lifting does not drop the selection or follow a link");
    let menu = b.selection_menu(&ui).expect("a menu over the selection");
    let (mx, my, _, mh) = menu.rect();
    let t = tap(&mut b, &ui, mx + ui.pt(20.0), my + mh / 2.0, t + 800.0);
    assert_eq!(clipboard::text().as_deref(), Some("Hello"));
    assert_eq!(b.tabs[0].selection, None);
    let _ = t;
}

#[test]
fn select_all_copies_the_whole_page() {
    let _g = clipboard::tests::exclusive();
    let ui = ui();
    let mut b = browser(&ui, &scratch("select-all"), &[html_site("x.test/", PAGE1)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0) + 100.0;
    b.tabs[0].selection = Some(Sel::Block(0));
    let menu = b.selection_menu(&ui).unwrap();
    let (mx, my, mw, mh) = menu.rect();
    let t = tap(&mut b, &ui, mx + mw - ui.pt(20.0), my + mh / 2.0, t);
    assert_eq!(b.tabs[0].selection, Some(Sel::All));
    let menu = b.selection_menu(&ui).unwrap();
    let (mx, my, _, mh) = menu.rect();
    tap(&mut b, &ui, mx + ui.pt(20.0), my + mh / 2.0, t + 100.0);
    assert_eq!(clipboard::text().as_deref(), Some("Hello\n\nVisit the second page now."));
}

#[test]
fn dragging_scrolls_instead_of_following_a_link() {
    let ui = ui();
    let long: String = (0..120).map(|i| format!("<p>Paragraph {i} of a long page that scrolls.</p>")).collect();
    let page = format!("<title>Long</title><p><a href=\"/two\">top link</a></p>{long}");
    let mut b = browser(&ui, &scratch("scroll"), &[html_site("x.test/", &page), html_site("x.test/two", PAGE2)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0) + 100.0;
    let (x, y) = find_word(&b, &ui, |_, link| link.is_some());
    b.touch(&ui, Event::Down(x, y + 300.0), t);
    for i in 1..=10 {
        b.touch(&ui, Event::Move(x, y + 300.0 - 30.0 * i as f32), t + 10.0 * i as f64);
    }
    b.touch(&ui, Event::Up(x, y), t + 120.0);
    b.tick(&ui, t + 130.0);
    assert!(b.tabs[0].scroll.offset() > 100.0, "scrolled {}", b.tabs[0].scroll.offset());
    assert_eq!(title(&b), "Long", "the drag did not open anything");
    // a tap on the link at the top, after scrolling back, does follow it
    b.tabs[0].scroll.jump_top();
    let (x, y) = find_word(&b, &ui, |_, link| link.is_some());
    let t = tap(&mut b, &ui, x, y, t + 1000.0);
    settle(&mut b, &ui, t);
    assert_eq!(title(&b), "Second");
}

#[test]
fn pictures_load_and_appear_in_the_layout() {
    let ui = ui();
    let png = crate::imgdec::tests::png_bytes(4, 2, [255, 0, 0, 255]);
    let page = "<title>Pic</title><p>before</p><img src=\"/p.png\" width=40 height=20><p>after</p>";
    let sites: Vec<(&str, &str, &[u8])> = vec![html_site("x.test/", page), ("x.test/p.png", "image/png", png.as_slice())];
    let mut b = browser(&ui, &scratch("images"), &sites);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    let v = b.tabs[0].view.as_ref().unwrap();
    assert!(v.images.has("/p.png"));
    assert_eq!(v.sizes.get("/p.png"), Some(&(4, 2)));
    assert!(v.page.items.iter().any(|i| matches!(i, Item::Image { w, h, .. } if *w == 40.0 && *h == 20.0)));
    let mut px = vec![0u32; 1200 * 1920];
    b.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut px, y0: 0 }, &ui, t + 10.0);
    assert!(px.contains(&crate::gfx::pack(255, 0, 0, 255)), "the picture reached the screen");
}

#[test]
fn style_sheets_load_and_restyle_the_page() {
    let ui = ui();
    let page = "<title>Styled</title><link rel=stylesheet href=\"/s.css\"><body><p>text</p></body>";
    let sites: Vec<(&str, &str, &[u8])> = vec![html_site("x.test/", page), ("x.test/s.css", "text/css", b"body { background: #102030 }")];
    let mut b = browser(&ui, &scratch("css"), &sites);
    b.navigate(&ui, "http://x.test/", 100.0);
    settle(&mut b, &ui, 100.0);
    let v = b.tabs[0].view.as_ref().unwrap();
    assert!(v.sources[0].pending.is_none());
    assert_eq!(v.sources[0].rules.len(), 1);
    assert_eq!(v.page.background, Some(css::Color::rgb(0x10, 0x20, 0x30)), "the linked sheet was fetched and applied");
}

#[test]
fn drawing_paints_the_chrome_and_the_page() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("draw"), &[]);
    let mut px = vec![0u32; 1200 * 1920];
    b.draw(&mut Surface { w: 1200, h: 1920, stride: 1200, px: &mut px, y0: 0 }, &ui, 100.0);
    let at = |x: f32, y: f32| px[y as usize * 1200 + x as usize];
    assert_eq!(at(5.0, b.tab_top(&ui) + 2.0), TAB_ACTIVE, "the active tab");
    assert_eq!(at(2.0, b.tool_top(&ui) + 5.0), BAR, "the toolbar");
    assert_eq!(at(5.0, b.content_top(&ui) + 2.0), crate::shell::browser::css::Color::WHITE.pack(), "the start page background");
    assert!(!b.needs_frame());
    b.touch(&ui, Event::Down(5.0, 5.0), 200.0);
    assert!(b.needs_frame());
}

#[test]
fn frames_are_requested_while_loading_and_pressing() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("wake"), &[html_site("x.test/", PAGE1)]);
    assert_eq!(b.next_wake(0.0), None);
    b.navigate(&ui, "http://x.test/", 100.0);
    assert!(b.next_wake(100.0).is_some_and(|w| w <= 130.0), "polling for the reply");
    let t = settle(&mut b, &ui, 100.0);
    b.press = Some((10.0, 10.0, t));
    assert_eq!(b.next_wake(t), Some(t + crate::shell::accessibility::long_press_ms(LONG_PRESS_MS)));
}

#[test]
fn reload_and_stop() {
    let ui = ui();
    let mut b = browser(&ui, &scratch("reload"), &[html_site("x.test/", PAGE1)]);
    b.navigate(&ui, "http://x.test/", 100.0);
    let t = settle(&mut b, &ui, 100.0);
    let nav_before = b.tabs[0].nav;
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 11), t + 50.0);
    assert!(b.tabs[0].loading.is_some() && b.tabs[0].nav > nav_before);
    let t = tap_rect(&mut b, &ui, |b, ui| b.button_rect(ui, 11), t + 50.0);
    assert!(b.tabs[0].loading.is_none(), "the same button stops a load");
    assert_eq!(title(&b), "First", "stopping keeps the page that was showing");
    let _ = t;
}
