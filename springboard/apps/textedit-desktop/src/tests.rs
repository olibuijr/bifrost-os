use super::*;
use bifrost_app::document::Document;
use text::{Cell, Style};

fn file_item(name: &str) -> dnd::DragItem {
    dnd::DragItem::new(dnd::Payload::File { grant: [0; 16], name: name.into() })
}

#[test]
fn a_dropped_text_file_opens_as_a_document_and_a_picture_or_other_file_does_not() {
    assert!(is_document_file(&file_item("qa note.rtf")));
    assert!(is_document_file(&file_item("notes.txt")));
    assert!(!is_document_file(&file_item("photo.png")), "a picture is inserted, not opened");
    assert!(!is_document_file(&file_item("archive.zip")));
    assert!(!is_document_file(&dnd::DragItem::new(dnd::Payload::Text("hi".into()))), "dragged text is not a file");
}

fn doc(s: &str) -> TextDocument {
    let mut d = TextDocument::untitled();
    d.text.load(s.chars().map(|ch| Cell { ch, style: Style::default() }).collect());
    d
}

#[test]
fn an_untitled_document_holds_the_text_it_is_given_and_shows_from_the_top() {
    let ctx = window(1200, 780);
    let mut app = TextEditApp::new();
    app.scroll = 40.0;
    app.open_untitled(doc("shared text"));
    // the first document takes the window there is (`doc_windows`), from its top
    app.sync_windows(&ctx);
    assert_eq!(active_doc(&app.docs).unwrap().text.text(), "shared text");
    assert!(app.follow);
    assert_eq!(app.scroll, 0.0);
}

#[test]
fn spelling_works_on_the_word_at_the_caret_or_a_single_word_selection() {
    let mut d = doc("helo world");
    d.text.select(2, 2);
    assert_eq!(word_here(&d), Some((0..4, "helo".to_string())));
    d.text.select(5, 5);
    assert_eq!(word_here(&d), Some((5..10, "world".to_string())));
    d.text.select(0, 10);
    assert_eq!(word_here(&d), None, "a selection of two words is not a word");
    d.text.select(4, 4);
    assert_eq!(word_here(&d), Some((0..4, "helo".to_string())), "the end of a word still belongs to it");
}

#[test]
fn prevent_editing_refuses_text_changes_but_not_copying_or_view_commands() {
    assert!(edits_text(Action::Paste));
    assert!(edits_text(Action::Delete));
    assert!(edits_text(Action::Transform(Transform::Upper)));
    assert!(edits_text(Action::Strike));
    assert!(edits_text(Action::Undo));
    assert!(!edits_text(Action::Copy));
    assert!(!edits_text(Action::Find));
    assert!(!edits_text(Action::ZoomIn));
    assert!(!edits_text(Action::PreventEditing));
    assert!(!edits_text(Action::CopyStyle));
}

#[test]
fn zooming_scales_sizes_only() {
    let cells = vec![Cell { ch: 'a', style: Style { size: 12.0, ..Style::default() } }];
    let big = zoomed(&cells, 2.0);
    assert_eq!(big[0].style.size, 24.0);
    assert_eq!(big[0].ch, 'a');
    assert_eq!(cells[0].style.size, 12.0, "the document keeps its own size");
}

#[test]
fn formatting_commands_include_the_new_character_styles() {
    assert!(is_formatting(Action::Strike));
    assert!(is_formatting(Action::Superscript));
    assert!(is_formatting(Action::Highlight(2)));
    assert!(is_formatting(Action::PasteStyle));
    assert!(!is_formatting(Action::CopyStyle));
}

/// A host with a data directory and nothing else: enough for the controller's autosave copies.
struct DirHost(std::path::PathBuf);

impl DocumentHost for DirHost {
    fn open_panel(&self, _o: &OpenOptions) -> u32 {
        0
    }
    fn save_panel(&self, _n: &str, _f: &[&str]) -> u32 {
        0
    }
    fn save_in_place(&self, _g: bifrost_app::proto::GrantId) -> u32 {
        0
    }
    fn commit(&self, _r: u32) {}
    fn abort(&self, _r: u32) {}
    fn reopen(&self, _g: bifrost_app::proto::GrantId) -> u32 {
        0
    }
    fn release(&self, _g: bifrost_app::proto::GrantId) {}
    fn set_document_state(&self, _t: &str, _e: bool, _r: &str) {}
    fn data_dir(&self) -> std::path::PathBuf {
        self.0.clone()
    }
    fn pref(&self, _k: &str) -> Option<String> {
        None
    }
    fn set_pref(&self, _k: &str, _v: &str) {}
}

/// A quit that is not asked (logout, shutdown): the unsaved Untitled is autosaved as the app quits (`suspend`) and the next launch opens it
/// again, edited, instead of a blank document.
#[test]
fn an_unsaved_untitled_document_comes_back_on_the_next_launch() {
    let dir = std::env::temp_dir().join(format!("bifrost-textedit-restore-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let host = DirHost(dir.clone());
    let mut app = TextEditApp::new();
    app.start_documents(&host);
    let id = app.docs.active().unwrap();
    *app.docs.get_mut(id).unwrap() = doc("repro unsaved text");
    app.docs.mark_edited(id);
    app.docs.suspend(&host);

    let mut next = TextEditApp::new();
    next.start_documents(&host);
    assert_eq!(next.docs.len(), 1, "the restored document replaces the blank one");
    let back = next.docs.active().unwrap();
    assert!(next.docs.is_edited(back), "closing it still asks");
    assert_eq!(active_doc(&next.docs).unwrap().text.text(), "repro unsaved text");

    // with nothing autosaved a launch starts with one blank Untitled
    let _ = std::fs::remove_dir_all(&dir);
    let mut fresh = TextEditApp::new();
    fresh.start_documents(&DirHost(dir));
    assert_eq!(fresh.docs.len(), 1);
    assert!(!fresh.docs.is_edited(fresh.docs.active().unwrap()));
}

fn app_with_plain(bytes: &[u8]) -> TextEditApp {
    let mut app = TextEditApp::new();
    let id = app.docs.new_document();
    *app.docs.get_mut(id).unwrap() = TextDocument::read_bytes(bytes, false).unwrap();
    app
}

#[test]
fn choosing_an_encoding_marks_a_plain_document_edited_and_only_when_it_changes() {
    let mut app = app_with_plain(b"abc");
    let id = app.docs.active().unwrap();
    assert!(!app.docs.is_edited(id));
    assert!(!app.set_encoding(Encoding::Utf8), "the encoding it already has changes nothing");
    assert!(!app.docs.is_edited(id));
    assert!(app.set_encoding(Encoding::Utf16Le));
    assert_eq!(active_doc(&app.docs).unwrap().encoding, Encoding::Utf16Le);
    assert!(app.docs.is_edited(id));
}

#[test]
fn rich_text_and_read_only_documents_refuse_an_encoding() {
    let mut app = TextEditApp::new();
    app.docs.new_document();
    assert!(app.set_encoding(Encoding::Latin1));
    assert_eq!(active_doc(&app.docs).unwrap().encoding, Encoding::Utf8, "rich text keeps its encoding");
    assert!(app.note.as_deref().is_some_and(|n| n.contains("Make Plain Text")));
    let mut plain = app_with_plain(b"abc");
    active_doc_mut(&mut plain.docs).unwrap().read_only = true;
    assert!(plain.set_encoding(Encoding::Latin1));
    assert_eq!(active_doc(&plain.docs).unwrap().encoding, Encoding::Utf8);
}

#[test]
fn reinterpreting_rereads_the_text_and_is_undoable() {
    // a classic-set file that was guessed as Windows text
    let mut app = app_with_plain(&[0x8e, b'x']);
    assert_eq!(active_doc(&app.docs).unwrap().encoding, Encoding::Windows1252);
    assert_eq!(active_doc(&app.docs).unwrap().text.text(), "\u{17d}x");
    assert!(app.reinterpret(Encoding::Roman));
    let doc = active_doc(&app.docs).unwrap();
    assert_eq!(doc.text.text(), "éx");
    assert_eq!(doc.encoding, Encoding::Roman);
    assert!(doc.text.can_undo());
    // text that is not bytes of the old set cannot be reread
    let mut app = app_with_plain("→".as_bytes());
    active_doc_mut(&mut app.docs).unwrap().encoding = Encoding::Latin1;
    assert!(app.reinterpret(Encoding::Roman));
    assert_eq!(active_doc(&app.docs).unwrap().text.text(), "→");
    assert!(app.note.as_deref().is_some_and(|n| n.contains("cannot be read")));
}

#[test]
fn the_properties_panel_applies_on_done_and_discards_on_cancel() {
    let mut app = app_with_plain(b"abc");
    let id = app.docs.active().unwrap();
    assert!(app.open_properties());
    app.props.as_mut().unwrap().1.type_text("  Plan ");
    assert!(app.close_properties(false));
    assert!(app.props.is_none());
    assert!(active_doc(&app.docs).unwrap().properties.is_empty(), "Cancel keeps nothing");
    assert!(!app.docs.is_edited(id));

    app.open_properties();
    app.props.as_mut().unwrap().1.type_text("  Plan ");
    app.props.as_mut().unwrap().1.next_field();
    app.props.as_mut().unwrap().1.type_text("Ada");
    assert!(app.close_properties(true));
    let props = &active_doc(&app.docs).unwrap().properties;
    assert_eq!((props.title.as_str(), props.author.as_str()), ("Plan", "Ada"));
    assert!(app.docs.is_edited(id), "changed properties are unsaved changes");

    // unchanged properties are not an edit
    let mut quiet = app_with_plain(b"x");
    let qid = quiet.docs.active().unwrap();
    quiet.open_properties();
    quiet.close_properties(true);
    assert!(!quiet.docs.is_edited(qid));
}

#[test]
fn properties_cannot_change_while_editing_is_prevented() {
    let mut app = app_with_plain(b"abc");
    active_doc_mut(&mut app.docs).unwrap().read_only = true;
    app.open_properties();
    app.props.as_mut().unwrap().1.type_text("Title");
    app.close_properties(true);
    assert!(active_doc(&app.docs).unwrap().properties.is_empty());
}

fn app_with_text(s: &str, select: Range<usize>) -> TextEditApp {
    let mut app = TextEditApp::new();
    let id = app.docs.new_document();
    let doc = app.docs.get_mut(id).unwrap();
    doc.text.load(s.chars().map(|ch| Cell { ch, style: Style::default() }).collect());
    doc.text.select(select.start, select.end);
    app
}

#[test]
fn adding_a_link_links_the_selection_and_remove_link_takes_it_off() {
    let mut app = app_with_text("see the docs", 4..12);
    let id = app.docs.active().unwrap();
    assert!(app.open_link_sheet());
    assert!(!app.link.as_ref().unwrap().had_link);
    app.link.as_mut().unwrap().url = "example.com/docs".into();
    assert!(app.finish_link(true));
    assert!(app.link.is_none());
    let doc = active_doc(&app.docs).unwrap();
    assert_eq!(links::url(doc.text.link_at(5)).as_deref(), Some("https://example.com/docs"));
    assert_eq!(doc.text.link_at(3), 0);
    assert!(app.docs.is_edited(id));

    // with the caret inside the link the panel opens on it, with its address
    active_doc_mut(&mut app.docs).unwrap().text.select(6, 6);
    assert!(app.open_link_sheet());
    let sheet = app.link.as_ref().unwrap();
    assert_eq!((sheet.url.as_str(), sheet.had_link, sheet.range.clone()), ("https://example.com/docs", true, 4..12));
    assert!(app.finish_link(false));
    assert_eq!(active_doc(&app.docs).unwrap().text.link_at(5), 0);
}

#[test]
fn an_empty_address_removes_the_link_and_a_bad_one_stays_in_the_panel() {
    let mut app = app_with_text("a link here", 2..6);
    app.open_link_sheet();
    app.link.as_mut().unwrap().url = "two words".into();
    assert!(app.finish_link(true));
    assert!(app.link.is_some(), "the panel stays open for another try");
    assert!(app.note.as_deref().is_some_and(|n| n.contains("not an address")));
    assert_eq!(active_doc(&app.docs).unwrap().text.link_at(3), 0);
    app.link.as_mut().unwrap().url = "https://example.org".into();
    assert!(app.finish_link(true));
    assert_ne!(active_doc(&app.docs).unwrap().text.link_at(3), 0);
    app.open_link_sheet();
    app.link.as_mut().unwrap().url.clear();
    assert!(app.finish_link(true));
    assert_eq!(active_doc(&app.docs).unwrap().text.link_at(3), 0, "Done with no address removes the link");
}

#[test]
fn links_need_a_selection_rich_text_and_an_editable_document() {
    let mut none = app_with_text("text", 2..2);
    assert!(none.open_link_sheet());
    assert!(none.link.is_none());
    assert!(none.note.as_deref().is_some_and(|n| n.contains("Select the text")));

    let mut plain = app_with_text("text", 0..4);
    active_doc_mut(&mut plain.docs).unwrap().rich = false;
    assert!(plain.open_link_sheet());
    assert!(plain.link.is_none());
    assert!(plain.note.as_deref().is_some_and(|n| n.contains("rich text")));

    let mut locked = app_with_text("text", 0..4);
    active_doc_mut(&mut locked.docs).unwrap().read_only = true;
    assert!(locked.open_link_sheet());
    assert!(locked.link.is_none());
}

#[test]
fn smart_links_link_an_address_when_the_word_ends() {
    let mut app = app_with_text("", 0..0);
    app.opts.smart_links = true;
    assert!(app.insert_raw("see www.example.com"));
    assert_eq!(active_doc(&app.docs).unwrap().text.link_at(8), 0, "no link until the word ends");
    assert!(app.insert_raw(" "));
    let doc = active_doc(&app.docs).unwrap();
    assert_eq!(links::url(doc.text.link_at(8)).as_deref(), Some("https://www.example.com"));
    assert_eq!(doc.text.link_at(3), 0);
    assert_eq!(doc.text.link_at(19), 0, "the blank after it is not part of the link");
    assert_eq!(doc.text.style().link, 0, "typing goes on outside the link");
    // a plain word, and an address that is already a link, are left alone
    assert!(app.insert_raw("hello "));
    assert!(active_doc(&app.docs).unwrap().text.cells()[20..].iter().all(|c| c.style.link == 0));
    // with the option off nothing is linked
    let mut off = app_with_text("", 0..0);
    off.insert_raw("www.example.com ");
    assert_eq!(active_doc(&off.docs).unwrap().text.link_at(2), 0);
    // a plain text document is never linked
    let mut plain = app_with_text("", 0..0);
    plain.opts.smart_links = true;
    active_doc_mut(&mut plain.docs).unwrap().rich = false;
    plain.insert_raw("www.example.com ");
    assert_eq!(active_doc(&plain.docs).unwrap().text.link_at(2), 0);
}

#[test]
fn typing_into_the_link_panel_takes_an_address_without_blanks() {
    let mut app = app_with_text("text", 0..4);
    app.open_link_sheet();
    assert!(app.type_into("https://exa mple.com"));
    assert_eq!(app.link.as_ref().unwrap().url, "https://example.com");
    assert!(app.backspace());
    assert_eq!(app.link.as_ref().unwrap().url, "https://example.co");
    assert_eq!(active_doc(&app.docs).unwrap().text.text(), "text", "the page is untouched");
}

#[test]
fn only_rich_text_takes_formatting_commands() {
    assert!(is_formatting(Action::Bold));
    assert!(is_formatting(Action::Color(2)));
    assert!(is_formatting(Action::List(text::ListKind::Bullet)));
    assert!(!is_formatting(Action::MakePlain));
    assert!(!is_formatting(Action::Find));
    assert!(!is_formatting(Action::Print));
}

// ---- the page and the format bar, through a context with no shell behind it ----

const SENTENCE: &str = "Hello from the BifrOSt E2E run.";

fn window(w: u32, h: u32) -> Ctx {
    use bifrost_app::kit::{Font, Ui};
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = bifrost_app::Configure { w, h, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
}

fn carets_sent(ctx: &Ctx) -> Vec<(f32, f32, f32, f32)> {
    use bifrost_app::proto::ToShell;
    ctx.take_sent().into_iter().filter_map(|m| if let ToShell::CaretRect { x, y, w, h } = m { Some((x, y, w, h)) } else { None }).collect()
}

/// E2E9: the key window reports its caret to the shell in window points, on the page's first line after the typed
/// word, so Look Up opens under the selection instead of under the menu bar; an unchanged caret is not sent again.
#[test]
fn painting_reports_the_caret_of_the_key_window() {
    let ctx = window(1200, 780);
    let mut app = TextEditApp::new();
    app.configure(&ctx);
    app.window_event(&ctx, bifrost_app::app_windows::WindowId::MAIN, Input::Key(KeyInput::Text("orange".into())));
    ctx.take_sent();
    let mut buf = vec![0u32; 1200 * 780];
    app.draw(&ctx, &mut Surface { w: 1200, h: 780, stride: 1200, px: &mut buf, y0: 0 }, 0.0);
    let carets = carets_sent(&ctx);
    assert_eq!(carets.len(), 1, "one caret for the key window");
    let geom = app.geometry(&ctx.ui, 1200, 780);
    let (x, y, _, h) = carets[0];
    assert!(x > geom.text_x, "after the typed word");
    assert!(y >= geom.body_top && y < geom.body_bottom && h > 0.0, "on the page");
    assert_eq!(caret_in_window(&geom, 0.0, (10.0, 0.0, 16.0)), (geom.text_x + 10.0, geom.body_top, 1.0, 16.0));
    app.draw(&ctx, &mut Surface { w: 1200, h: 780, stride: 1200, px: &mut buf, y0: 0 }, 0.0);
    assert!(carets_sent(&ctx).is_empty(), "nothing moved: nothing is sent");
}

/// The window point a quarter of the way into the character at `index` on the first line of the page: the caret nearest
/// to it is the one before that character, and the character under it is that one.
fn middle_of(app: &TextEditApp, ctx: &Ctx, text: &str, index: usize) -> (f32, f32) {
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    let width = |ch: char| view::char_width(&ctx.ui, ch, &Style::default());
    let before: f32 = text.chars().take(index).map(width).sum();
    let this = text.chars().nth(index).map_or(0.0, width);
    (geom.text_x + before + this * 0.25, geom.body_top + 6.0)
}

/// The shell's Services submenu and Look Up read the text state: the selection, or the word at the caret.
#[test]
fn the_selection_and_caret_are_reported_as_the_text_state() {
    use bifrost_app::proto::ToShell;
    let ctx = window(1200, 780);
    let mut app = app_with_text("look up quokka now", 8..14);
    app.sync_text_state(&ctx);
    let states: Vec<_> = ctx.take_sent().into_iter().filter_map(|m| if let ToShell::TextState(t) = m { Some(t) } else { None }).collect();
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].selected_text(), "quokka");
    assert_eq!(app.selected_text(&ctx), Some("quokka".to_string()));
    // unchanged: not sent again
    app.sync_text_state(&ctx);
    assert!(!ctx.take_sent().iter().any(|m| matches!(m, ToShell::TextState(_))));
    // a caret inside a word: nothing selected, Look Up takes the word
    active_doc_mut(&mut app.docs).unwrap().text.select(10, 10);
    app.sync_text_state(&ctx);
    let caret = active_doc(&app.docs).map(text_state_of).unwrap();
    assert_eq!(caret.selected_text(), "");
    assert_eq!(caret.word_at_cursor(), "quokka");
    assert_eq!(app.selected_text(&ctx), Some("quokka".to_string()));
    // a caret before the first word takes that word
    active_doc_mut(&mut app.docs).unwrap().text.select(0, 0);
    assert_eq!(app.selected_text(&ctx), Some("look".to_string()));
}

/// The reported window is a piece of a long document around the selection, with byte offsets into it.
#[test]
fn the_text_state_is_a_window_around_the_selection_in_bytes() {
    let long = format!("{}é quokka {}", "x".repeat(5000), "y".repeat(5000));
    let start = 5000 + 2;
    let app = app_with_text(&long, start..start + 6);
    let t = text_state_of(active_doc(&app.docs).unwrap());
    assert!(t.text.len() <= bifrost_app::proto::MAX_SURROUNDING);
    assert_eq!(t.selected_text(), "quokka");
    assert!(t.text.contains("é quokka "));
    assert!(t.text.is_char_boundary(t.cursor as usize) && t.text.is_char_boundary(t.anchor as usize));
}

fn press(app: &mut TextEditApp, ctx: &Ctx, kind: TouchKind, (x, y): (f32, f32), t: f64) -> bool {
    app.touch(ctx, kind, x * ctx.ui.s, y * ctx.ui.s, t)
}

fn selected(app: &TextEditApp) -> String {
    active_doc(&app.docs).unwrap().text.selected_text().unwrap_or_default()
}

fn wheel_at(app: &mut TextEditApp, ctx: &Ctx, (x, y): (f32, f32), dy: f32) -> bool {
    use bifrost_app::pointer_events::{Phase, PointerEvent};
    let ev = PointerEvent::Wheel { x: x * ctx.ui.s, y: y * ctx.ui.s, dx: 0.0, dy: dy * ctx.ui.s, precise: false, phase: Phase::None, momentum: Phase::None };
    app.pointer(ctx, &ev)
}

/// E2E: the wheel scrolls the page (the emulated drag it replaces placed the caret and selected text), stops at the
/// ends of the text, and over the Fonts panel it goes to the family column, never to the page under the panel.
#[test]
fn the_wheel_scrolls_the_page_and_over_the_fonts_panel_its_families() {
    let ctx = window(1200, 780);
    assert_eq!(TextEditApp::new().pointer_interest(), bifrost_app::pointer_events::WHEEL);
    let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let mut app = app_with_text(&long, 0..0);
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    app.ensure_layout(&ctx.ui, geom.text_w);
    let page = (geom.text_x + 10.0, geom.body_top + 40.0);
    assert!(wheel_at(&mut app, &ctx, page, -40.0), "a wheel turned down shows the text below");
    assert_eq!(app.scroll, 40.0);
    assert_eq!(selected(&app), "", "no text is selected by the wheel");
    assert_eq!(active_doc(&app.docs).unwrap().text.head(), 0, "and the caret stays");
    assert!(wheel_at(&mut app, &ctx, page, 15.0));
    assert_eq!(app.scroll, 25.0);
    wheel_at(&mut app, &ctx, page, 1.0e6);
    assert_eq!(app.scroll, 0.0, "the top of the text");
    assert!(!wheel_at(&mut app, &ctx, page, 30.0), "already at the top");
    wheel_at(&mut app, &ctx, page, -1.0e6);
    let bottom = app.scroll;
    assert!(bottom > 0.0 && bottom < app.cache.as_ref().unwrap().layout.height, "the end of the text: {bottom}");
    assert!(!wheel_at(&mut app, &ctx, (page.0, geom.body_top - 4.0), 30.0), "the format bar does not scroll");

    // the Fonts panel: its family column takes the wheel (three families fit, so nothing moves) and the page under
    // the panel keeps its place
    app.fonts = Some(font_panel::Fonts::new(0));
    let layout = font_panel::fonts_layout(geom.width, geom.body_top);
    let column = (layout.family.x + 4.0, layout.family.y + 4.0);
    assert!(!wheel_at(&mut app, &ctx, column, 30.0));
    assert!(!wheel_at(&mut app, &ctx, (layout.done.x + 1.0, layout.done.y + 1.0), 30.0));
    assert_eq!(app.scroll, bottom);
    // beside the panel the page still scrolls
    let beside = (layout.frame.x + layout.frame.w + 4.0, geom.body_bottom - 10.0);
    assert!(!layout.frame.contains(beside.0, beside.1));
    assert!(wheel_at(&mut app, &ctx, beside, 30.0));
    assert_eq!(app.scroll, bottom - 30.0);
}

#[test]
fn a_press_on_the_first_line_places_the_caret_though_the_find_fields_overlap_it() {
    // the report: clicks never moved the caret, because the closed find bar's fields sat over the first line and took the press
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, SENTENCE.len()..SENTENCE.len());
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    let point = middle_of(&app, &ctx, SENTENCE, 3);
    assert!(geom.find_field.contains(point.0, point.1), "the point is under the hidden find field: {point:?} {:?}", geom.find_field);
    assert!(press(&mut app, &ctx, TouchKind::Down, point, 0.0));
    let doc = active_doc(&app.docs).unwrap();
    assert_eq!(doc.text.head(), 3, "the caret is at the character pressed, not at the end");
    assert_eq!(app.focus, Focus::Body);
    // typing goes in at the caret
    press(&mut app, &ctx, TouchKind::Up, point, 10.0);
    app.type_into("X");
    let text = active_doc(&app.docs).unwrap().text.text();
    assert_eq!(text, "HelXlo from the BifrOSt E2E run.");
}

#[test]
fn a_double_click_selects_the_word_and_typing_replaces_it() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, SENTENCE.len()..SENTENCE.len());
    let point = middle_of(&app, &ctx, SENTENCE, 7);
    press(&mut app, &ctx, TouchKind::Down, point, 0.0);
    press(&mut app, &ctx, TouchKind::Up, point, 40.0);
    assert_eq!(selected(&app), "");
    press(&mut app, &ctx, TouchKind::Down, point, 120.0);
    press(&mut app, &ctx, TouchKind::Up, point, 160.0);
    assert_eq!(selected(&app), "from");
    app.type_into("X");
    assert_eq!(active_doc(&app.docs).unwrap().text.text(), "Hello X the BifrOSt E2E run.");
}

#[test]
fn a_triple_click_selects_the_paragraph_and_a_fourth_press_starts_over() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 0..0);
    let point = middle_of(&app, &ctx, SENTENCE, 7);
    for (k, t) in [0.0, 100.0, 200.0].into_iter().enumerate() {
        press(&mut app, &ctx, TouchKind::Down, point, t);
        press(&mut app, &ctx, TouchKind::Up, point, t + 30.0);
        let expected = [0, 4, SENTENCE.len()][k];
        assert_eq!(selected(&app).len(), expected, "press {}", k + 1);
    }
    press(&mut app, &ctx, TouchKind::Down, point, 300.0);
    assert_eq!(selected(&app), "", "the fourth press places the caret again");
}

#[test]
fn dragging_selects_by_characters_and_by_words_after_a_double_click() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 0..0);
    // a plain drag from the middle of "from" to the middle of "BifrOSt"
    let (from, to) = (middle_of(&app, &ctx, SENTENCE, 7), middle_of(&app, &ctx, SENTENCE, 16));
    press(&mut app, &ctx, TouchKind::Down, from, 0.0);
    assert!(press(&mut app, &ctx, TouchKind::Move, to, 20.0));
    assert_eq!(selected(&app), "rom the B", "from the character pressed to the one under the pointer");
    press(&mut app, &ctx, TouchKind::Up, to, 30.0);
    assert!(!press(&mut app, &ctx, TouchKind::Move, from, 40.0), "a move with the button up does nothing");
    // press twice and hold: the drag takes whole words
    press(&mut app, &ctx, TouchKind::Down, from, 2000.0);
    press(&mut app, &ctx, TouchKind::Up, from, 2010.0);
    press(&mut app, &ctx, TouchKind::Down, from, 2100.0);
    press(&mut app, &ctx, TouchKind::Move, to, 2120.0);
    assert_eq!(selected(&app), "from the BifrOSt");
}

#[test]
fn the_navigation_chords_extend_the_selection_and_plain_keys_collapse_it() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, SENTENCE.len()..SENTENCE.len());
    // Shift+Home (evdev 102) selects back to the start of the line, Shift+End forward again
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(102, CHORD_SHIFT))));
    assert_eq!(selected(&app), SENTENCE);
    // Shift+Right moves the head from the start; the anchor stays at the end of the text
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(106, CHORD_SHIFT))));
    assert_eq!(selected(&app), &SENTENCE[1..]);
    // Alt+Shift+Right moves the head one word on
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(106, CHORD_SHIFT | CHORD_ALT))));
    assert_eq!(selected(&app), &SENTENCE[5..]);
    // Left alone collapses to the start of the selection, then Home (plain code 36) goes to the line start
    assert!(app.event(&ctx, Input::Key(KeyInput::Code(37))));
    assert_eq!(active_doc(&app.docs).unwrap().text.selection(), 5..5);
    assert!(app.event(&ctx, Input::Key(KeyInput::Code(36))));
    assert_eq!(active_doc(&app.docs).unwrap().text.head(), 0);
    assert!(app.event(&ctx, Input::Key(KeyInput::Code(35))));
    assert_eq!(active_doc(&app.docs).unwrap().text.head(), SENTENCE.len());
    // Cmd+Left is the line start as well, and a chord on another key is not a move
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(105, CHORD_META))));
    assert_eq!(active_doc(&app.docs).unwrap().text.head(), 0);
    assert!(!app.event(&ctx, Input::Key(KeyInput::chord(49, CHORD_META))), "a chord on another key is not a move");
    // Cmd+A as a chord (the shell sends it so when its last report of the commands is stale) selects everything
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(30, CHORD_META))));
    assert_eq!(active_doc(&app.docs).unwrap().text.selection(), 0..SENTENCE.len());
}

#[test]
fn the_navigation_keys_leave_a_panel_or_the_find_field_alone() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 3..3);
    app.focus = Focus::Find;
    assert!(!app.event(&ctx, Input::Key(KeyInput::chord(102, CHORD_SHIFT))), "the find field has the keyboard");
    assert_eq!(active_doc(&app.docs).unwrap().text.head(), 3);
    app.focus = Focus::Body;
    app.open_line_sheet();
    assert!(!app.event(&ctx, Input::Key(KeyInput::chord(107, CHORD_SHIFT))), "a panel is open over the page");
    assert_eq!(active_doc(&app.docs).unwrap().text.head(), 3);
}

#[test]
fn the_editing_commands_act_on_the_focused_find_field_or_sheet_not_the_document() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 0..5);
    app.find_open = true;
    app.focus = Focus::Find;
    clipboard::set_text("needle\nsecond");
    assert!(app.event(&ctx, Input::Command(Command::Paste)));
    assert_eq!(app.find, "needle", "paste goes into the find field");
    assert_eq!(text_of(&app), SENTENCE, "the document is untouched");
    // Select All, Copy and Cut are the field's (QA W2-12: Cmd+A appended to the query instead); Undo does nothing
    assert!(app.event(&ctx, Input::Command(Command::SelectAll)), "Cmd+A selects the find field's text");
    assert!(app.find_sel);
    assert!(!app.event(&ctx, Input::Command(Command::Undo)));
    clipboard::set_text("");
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(45, CHORD_META))), "Cmd+X as a chord cuts the field's text");
    assert_eq!((app.find.as_str(), clipboard::text().as_deref()), ("", Some("needle")));
    assert_eq!(text_of(&app), SENTENCE, "cut and undo left the document alone");
    assert_eq!(caret_of(&app), 0..5, "select all left the selection alone");
    // a sheet open over the page: the same
    app.find_open = false;
    app.focus = Focus::Body;
    app.open_line_sheet();
    for command in [Command::Cut, Command::SelectAll, Command::Undo] {
        assert!(!app.event(&ctx, Input::Command(command)));
    }
    assert_eq!(text_of(&app), SENTENCE);
    assert_eq!(caret_of(&app), 0..5);
}

/// After Replace All and Done, typing continues after the last replacement, not at the start of the document.
#[test]
fn typing_after_replace_all_and_done_continues_after_the_last_replacement() {
    let ctx = window(1200, 780);
    let mut app = app_with_text("one cat, two cats", 0..0);
    assert!(app.perform(&ctx, Action::Find));
    app.find = "cat".into();
    app.replace = "dog".into();
    assert!(app.perform(&ctx, Action::ReplaceAll));
    assert!(app.perform(&ctx, Action::FindDone));
    assert_eq!(text_of(&app), "one dog, two dogs");
    // right after the last replacement, the "dog" of "dogs"
    assert_eq!(caret_of(&app), 16..16);
    app.event(&ctx, Input::Key(KeyInput::Text("!".into())));
    assert_eq!(text_of(&app), "one dog, two dog!s");
}

/// QA W2-12: Cmd+A in the find field, live matches while typing, the previous query selected on reopening, and no
/// "not found" left in the status row after the bar closes.
#[test]
fn the_find_bar_searches_as_it_is_typed_and_reopens_with_the_query_selected() {
    let ctx = window(1200, 780);
    let mut app = app_with_text("the lazy dog and the lazy cat", 0..0);
    assert!(app.perform(&ctx, Action::Find));
    // the shared find pasteboard may hold another test's query
    app.find.clear();
    app.find_sel = false;
    for c in ["l", "a", "z"] {
        app.event(&ctx, Input::Key(KeyInput::Text(c.into())));
    }
    assert_eq!(caret_of(&app), 4..7, "the first match is selected while typing");
    assert_eq!(app.find_matches(), vec![4..7, 21..24], "every match is highlighted");
    app.event(&ctx, Input::Key(KeyInput::Text("q".into())));
    assert_eq!(app.note.as_deref(), Some("\"lazq\" not found"));
    assert!(app.find_matches().is_empty());
    app.backspace();
    assert_eq!((app.find.as_str(), app.note.as_deref()), ("laz", None));
    // closing clears the note and the highlight
    app.event(&ctx, Input::Key(KeyInput::Text("q".into())));
    assert!(app.perform(&ctx, Action::FindDone));
    assert_eq!(app.note, None);
    assert!(app.find_matches().is_empty(), "no highlight once the bar is closed");
    // reopening selects the previous query: typing replaces it, Cmd+A selects it again
    app.perform(&ctx, Action::Find);
    assert!(app.find_sel);
    app.event(&ctx, Input::Key(KeyInput::Text("cat".into())));
    assert_eq!(app.find, "cat");
    assert_eq!(caret_of(&app), 26..29);
    app.event(&ctx, Input::Command(Command::SelectAll));
    app.event(&ctx, Input::Key(KeyInput::Text("dog".into())));
    assert_eq!(app.find, "dog", "Cmd+A then typing replaces the query");
    app.event(&ctx, Input::Command(Command::SelectAll));
    app.backspace();
    assert_eq!(app.find, "", "Backspace over the selected query clears it");
    assert_eq!(text_of(&app), "the lazy dog and the lazy cat", "the page was never edited");
}

#[test]
fn find_all_lists_every_match_without_overlap() {
    let app = app_with_text("aaaa Aa", 0..0);
    let doc = active_doc(&app.docs).unwrap();
    assert_eq!(doc.text.find_all("aa", false, 10), vec![0..2, 2..4, 5..7]);
    assert_eq!(doc.text.find_all("aa", true, 10), vec![0..2, 2..4]);
    assert_eq!(doc.text.find_all("aa", false, 1), vec![0..2], "the limit holds");
    assert!(doc.text.find_all("", false, 10).is_empty());
}

/// The titles of the direct children of submenu `header` of `menu` (its top level when `header` is None).
fn level_titles(menu: &AppMenu, header: Option<&str>) -> Vec<String> {
    let (start, depth) = match header {
        None => (0, 0),
        Some(h) => {
            let at = menu.items.iter().position(|i| i.submenu && i.title == h).unwrap_or_else(|| panic!("no {h} submenu"));
            (at + 1, menu.items[at].depth + 1)
        }
    };
    menu.items[start..].iter().take_while(|i| i.depth >= depth).filter(|i| i.depth == depth).map(|i| i.title.clone()).collect()
}

/// QA: Format reached the menu bar's old 48-item cap before Color, so Color, Copy and Paste Style, Make Plain Text and the
/// encodings were dropped. Format is now laid out as the reference's (Font and Text submenus), every item reaches the
/// shell, and the Font submenu lists every family.
#[test]
fn the_format_menu_has_font_and_text_submenus_and_nothing_is_cut() {
    let app = app_with_text("Hello world", 0..5);
    let format = app.format_menu();
    let top = level_titles(&format, None);
    assert_eq!(&top[..2], ["Font", "Text"], "{top:?}");
    for want in ["Prevent Editing", "Allow Hyphenation", "Text Encoding", "Reinterpret As"] {
        assert!(top.iter().any(|t| t == want), "{want} is in Format: {top:?}");
    }
    assert!(top.iter().any(|t| t == "Make Plain Text" || t == "Make Rich Text"));
    assert!(top.iter().any(|t| t == "Wrap to Window" || t == "Wrap to Page"));
    let font = level_titles(&format, Some("Font"));
    for want in ["Show Fonts", "Bold", "Italic", "Underline", "Outline", "Styles\u{2026}", "Bigger", "Smaller", "Kern", "Ligatures", "Baseline", "Show Colors", "Color", "Copy Style", "Paste Style"] {
        assert!(font.iter().any(|t| t == want), "{want} is in Format > Font: {font:?}");
    }
    for family in text::families() {
        assert!(font.iter().any(|t| t == family), "the family {family} is in Format > Font");
    }
    let text = level_titles(&format, Some("Text"));
    for want in ["Align Left", "Center", "Justify", "Align Right", "Writing Direction", "Copy Ruler", "Paste Ruler", "Spacing\u{2026}", "Lists", "Tables\u{2026}"] {
        assert!(text.iter().any(|t| t == want), "{want} is in Format > Text: {text:?}");
    }
    assert!(text.iter().any(|t| t == "Show Ruler" || t == "Hide Ruler"));
    assert_eq!(level_titles(&format, Some("Writing Direction")), ["Left to Right", "Right to Left"]);
    assert_eq!(level_titles(&format, Some("Baseline")), ["Superscript", "Subscript"]);
    // the shell takes every item of every menu, and they fit the packets
    let menus = vec![app.file_menu(), app.edit_menu(), format.clone(), app.view_menu()];
    for m in &menus {
        assert_eq!(m.dropped_items(), 0, "{} loses items", m.title);
        assert!(m.items.iter().all(|i| i.depth <= bifrost_app::proto::MAX_APP_MENU_DEPTH), "{}", m.title);
    }
    assert!(bifrost_app::proto::app_menu_packets(&menus).dropped.is_empty());
    // the ruler and the wrapping width moved to Format; View has the zoom
    let view = level_titles(&app.view_menu(), None);
    assert!(!view.iter().any(|t| t.contains("Ruler") || t.starts_with("Wrap")), "{view:?}");
    // every enabled item's id is one the app reads back
    for i in format.items.iter().filter(|i| i.enabled && !i.submenu && !i.separator) {
        assert!(Action::from_id(i.id).is_some(), "{} has id {}", i.title, i.id);
    }
}

/// Format > Font lists 256 families without losing an item: a submenu's own budget holds them.
#[test]
fn a_font_submenu_with_every_family_still_fits() {
    let app = app_with_text("Hello", 0..0);
    let mut format = app.format_menu();
    let at = format.items.iter().position(|i| i.title == "Copy Style").unwrap();
    for k in 0..256u32 {
        format.items.insert(at + 1, AppMenuItem::new(1 + k, &format!("Installed Family {k}")).depth(1));
    }
    assert_eq!(format.dropped_items(), 0);
    let out = bifrost_app::proto::app_menu_packets(&[app.file_menu(), app.edit_menu(), format.clone(), app.view_menu()]);
    assert!(out.dropped.is_empty(), "{:?}", out.dropped);
}

/// The Make Plain Text confirmation takes the kit's sheet keys: Return OK, Escape and Cmd+. Cancel, Tab and Space.
#[test]
fn the_make_plain_text_sheet_takes_return_escape_cmd_period_and_the_tab_focus_ring() {
    let ctx = window(1200, 780);
    let mut app = app_with_text("Hello", 0..0);
    let (tab, space) = (|| Input::Key(KeyInput::Code(9)), || Input::Key(KeyInput::Text(" ".into())));
    assert!(app.perform(&ctx, Action::MakePlain) && app.plain_sheet);
    assert!(!app.event(&ctx, Input::Key(KeyInput::Text("x".into()))), "typing goes nowhere behind the sheet");
    assert!(!app.event(&ctx, space()), "Space with no focus presses nothing");
    assert_eq!(app.plain_alert_sheet().unwrap().focus, None, "no ring until Tab");
    // Cmd+. (evdev key 52) cancels
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(52, CHORD_META))));
    assert!(!app.plain_sheet && active_doc(&app.docs).unwrap().rich);
    // Escape cancels
    app.perform(&ctx, Action::MakePlain);
    assert!(app.event(&ctx, Input::Key(KeyInput::Code(1))));
    assert!(!app.plain_sheet && active_doc(&app.docs).unwrap().rich);
    // Tab: OK, then Cancel; Space presses the focused one
    app.perform(&ctx, Action::MakePlain);
    assert!(app.event(&ctx, tab()));
    assert_eq!(app.plain_alert_sheet().unwrap().focus, Some(view::PLAIN_OK));
    assert!(app.event(&ctx, tab()));
    assert_eq!(app.plain_alert_sheet().unwrap().focus, Some(view::PLAIN_CANCEL));
    assert!(app.event(&ctx, space()));
    assert!(!app.plain_sheet && active_doc(&app.docs).unwrap().rich, "Space on Cancel keeps the rich text");
    // the ring starts over; Return is OK
    app.perform(&ctx, Action::MakePlain);
    assert_eq!(app.plain_alert_sheet().unwrap().focus, None);
    assert!(app.event(&ctx, Input::Key(KeyInput::Code(13))));
    assert!(!app.plain_sheet && !active_doc(&app.docs).unwrap().rich, "Return converts");
}

/// E2E10 finding 7: Make Plain Text converted at once and left the text in Helvetica.
#[test]
fn make_plain_text_asks_first_and_switches_to_the_plain_text_font() {
    let ctx = window(1200, 780);
    let mut app = app_with_text("Hello", 0..0);
    assert!(active_doc(&app.docs).unwrap().rich);
    assert!(app.perform(&ctx, Action::MakePlain));
    assert!(app.plain_sheet, "a rich document with text asks before converting");
    assert!(active_doc(&app.docs).unwrap().rich, "nothing changes until the sheet is answered");
    // E2E11 finding 6: the kit's alert sheet hanging from the title bar, buttons stacked full width, OK on top
    let sheet = app.plain_alert_sheet().expect("the confirmation is an alert sheet");
    assert!(sheet.alert);
    assert_eq!(sheet.title, view::PLAIN_TITLE);
    assert_eq!(sheet.default_button(), Some(view::PLAIN_OK));
    assert_eq!(sheet.cancel_button(), Some(view::PLAIN_CANCEL));
    let geom = app.geom(&ctx);
    let (parent, bar) = close_sheet_parent(&geom);
    let placed = sheet.layout(parent, bar, 1.0);
    assert_eq!(placed.panel.y, bar, "attached under the title bar");
    let ids: Vec<u32> = placed.buttons.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![view::PLAIN_OK, view::PLAIN_CANCEL], "OK on top, Cancel under it");
    let (ok, cancel) = (placed.buttons[0].1, placed.buttons[1].1);
    assert!(cancel.y > ok.y && cancel.x == ok.x && cancel.w == ok.w, "stacked, not side by side");
    assert!(app.perform(&ctx, Action::PlainCancel));
    assert!(!app.plain_sheet && active_doc(&app.docs).unwrap().rich, "Cancel keeps the rich text");
    app.perform(&ctx, Action::MakePlain);
    assert!(app.perform(&ctx, Action::PlainDone));
    let doc = active_doc(&app.docs).unwrap();
    assert!(!doc.rich && !app.plain_sheet);
    assert!(doc.text.cells().iter().all(|c| c.style.font == text::PLAIN_FONT), "the text takes the plain text font");
    assert_eq!(view::PLAIN_TITLE, "Convert this document to plain text?");
    // E2E11 finding 5: Undo put the font back but left the document in plain text mode (no format bar, no ruler)
    assert!(app.perform(&ctx, Action::Undo));
    assert!(active_doc(&app.docs).unwrap().rich, "Undo of Make Plain Text makes the document rich text again");
    assert!(app.perform(&ctx, Action::Redo));
    assert!(!active_doc(&app.docs).unwrap().rich, "Redo makes it plain text again");
}

/// Settings > Default Font takes every family a document can use, and choosing one keeps it. The settings are in the
/// Settings panel (the application menu's Settings\u{2026}), not in a menu of their own.
#[test]
fn default_font_takes_every_family() {
    let ctx = window(1200, 780);
    let mut app = app_with_text("Hello", 0..0);
    let last = (text::families().len() - 1) as u8;
    assert!(app.menu_action(&ctx, Action::DefaultFont(last).id().unwrap()));
    assert_eq!(app.opts.default_font, last);
    assert!(!app.perform(&ctx, Action::DefaultFont(last + 1)), "an index past the families is refused");
}

/// QA W2-11: Cmd+T did nothing and the Format menu had no Show Fonts or Show Colors.
#[test]
fn show_fonts_and_show_colors_open_panels_that_format_the_selection() {
    let ctx = window(1200, 780);
    let mut app = app_with_text("Hello world", 0..5);
    let format = app.format_menu();
    let item = |title: &str| format.items.iter().find(|i| i.title == title).cloned();
    assert_eq!(item("Show Fonts").map(|i| (i.id, i.key)), Some((Action::ShowFonts.id().unwrap(), "cmd+t".to_string())));
    assert_eq!(item("Show Colors").map(|i| (i.id, i.key)), Some((Action::ShowColors.id().unwrap(), "cmd+shift+c".to_string())));
    // the menu item (Cmd+T) opens the panel; a family, a typeface and a size apply to the selection at once
    assert!(app.menu_action(&ctx, Action::ShowFonts.id().unwrap()), "Cmd+T is the menu item's key equivalent");
    assert!(app.fonts.is_some() && !app.page_has_keys());
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    let l = font_panel::fonts_layout(geom.width, geom.body_top);
    let row = |r: bifrost_app::controls::Rect, i: usize| (r.x + 6.0, r.y + font_panel::ROW_H * (i as f32 + 0.5));
    press(&mut app, &ctx, TouchKind::Down, row(l.family, 2), 0.0);
    press(&mut app, &ctx, TouchKind::Down, row(l.face, 1), 100.0);
    press(&mut app, &ctx, TouchKind::Down, row(l.size, 5), 200.0);
    let style = active_doc(&app.docs).unwrap().text.cells()[0].style;
    assert_eq!((style.font, style.bold, style.italic, style.size), (2, true, false, text::SIZES[5]));
    assert_eq!(active_doc(&app.docs).unwrap().text.cells()[6].style.font, 0, "only the selection");
    // Up steps to the previous family; typing a name jumps to it; Esc closes
    app.event(&ctx, Input::Key(KeyInput::Code(38)));
    assert_eq!(app.status().style.font, 1);
    app.event(&ctx, Input::Key(KeyInput::Text("hel".into())));
    assert_eq!(app.status().style.font, 0, "Helvetica");
    app.event(&ctx, Input::Key(KeyInput::Code(27)));
    assert!(app.fonts.is_none() && app.page_has_keys());
    // Show Colors: a swatch colours the selection, Done closes
    app.perform(&ctx, Action::ShowColors);
    let c = font_panel::colors_layout(geom.width, geom.body_top);
    let s = c.swatches[1];
    press(&mut app, &ctx, TouchKind::Down, (s.x + 4.0, s.y + 4.0), 300.0);
    assert_eq!(active_doc(&app.docs).unwrap().text.cells()[0].style.color, 1);
    press(&mut app, &ctx, TouchKind::Down, (c.done.x + 2.0, c.done.y + 2.0), 400.0);
    assert!(!app.colors_panel);
    assert!(app.docs.is_edited(app.docs.active().unwrap()), "formatting is an unsaved change");
}

#[test]
fn an_installed_family_beyond_the_base_three_maps_to_a_base_face_and_keeps_its_name() {
    // the tests see only the base families; the mapping rules still hold for any index
    assert_eq!(text::base_family(1), 1);
    assert_eq!(text::base_family(200), 0, "an index past the list falls back to the default");
    assert_eq!(text::export_name(2), "Courier New");
    assert_eq!(text::family_index("times"), Some(1));
    assert_eq!(text::family_for_name("Liberation Mono"), 2, "a fixed-width name maps to Courier");
}

fn text_of(app: &TextEditApp) -> String {
    active_doc(&app.docs).unwrap().text.text()
}

fn caret_of(app: &TextEditApp) -> Range<usize> {
    active_doc(&app.docs).unwrap().text.selection()
}

/// The report: Cmd+A, C, X, V did nothing and Undo had nothing to undo. The shell sends them as the standard commands
/// (`Input::Command`); each one acts on the page, and each edit is one undo step.
#[test]
fn select_all_copy_cut_paste_and_undo_act_on_the_page() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 5..5);
    assert!(app.event(&ctx, Input::Command(Command::SelectAll)));
    assert_eq!(caret_of(&app), 0..SENTENCE.len(), "Cmd+A selects everything");
    app.event(&ctx, Input::Command(Command::Copy));
    assert_eq!(clipboard::text().as_deref(), Some(SENTENCE), "Cmd+C reaches the clipboard");
    assert_eq!(text_of(&app), SENTENCE, "copy leaves the text");
    assert!(app.event(&ctx, Input::Command(Command::Cut)));
    assert_eq!(text_of(&app), "", "Cmd+X removes the selection");
    assert_eq!(clipboard::text().as_deref(), Some(SENTENCE));
    assert!(app.event(&ctx, Input::Command(Command::Undo)));
    assert_eq!(text_of(&app), SENTENCE, "Cmd+Z puts the cut back");
    assert!(app.event(&ctx, Input::Command(Command::Redo)));
    assert_eq!(text_of(&app), "", "Shift+Cmd+Z cuts again");
    assert!(app.event(&ctx, Input::Command(Command::Paste)));
    assert_eq!(text_of(&app), SENTENCE, "Cmd+V pastes the clipboard");
    assert!(app.event(&ctx, Input::Command(Command::Undo)));
    assert_eq!(text_of(&app), "", "the paste is an undo step of its own");
    // the app reports what it takes, so the shell sends the command (and not the chord)
    let commands = app.commands(&ctx);
    assert!(commands.supports(Command::SelectAll) && commands.supports(Command::Paste) && commands.supports(Command::Redo));
}

/// Option+Backspace deletes a word, Cmd+Backspace the line before the caret, Option+Delete a word forward, Fn+Delete a
/// character forward, and Ctrl+K the rest of the paragraph; each is one chord and one undo step.
#[test]
fn the_delete_chords_remove_a_word_or_a_line() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 22..22);
    // Option+Backspace (evdev 14) after "BifrOSt" removes the word
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(14, CHORD_ALT))));
    assert_eq!(text_of(&app), "Hello from the  E2E run.");
    assert_eq!(caret_of(&app), 15..15);
    // Cmd+Backspace removes everything back to the start of the line
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(14, CHORD_META))));
    assert_eq!(text_of(&app), " E2E run.");
    assert_eq!(caret_of(&app), 0..0);
    // Option+Delete (evdev 111) removes the next word, blanks first; plain Delete one character
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(111, CHORD_ALT))));
    assert_eq!(text_of(&app), " run.");
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(111, 0))));
    assert_eq!(text_of(&app), "run.");
    // Ctrl+E is the line end, Ctrl+H a backspace, Ctrl+A the line start, Ctrl+D a forward delete, Ctrl+K the rest
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(18, CHORD_CTRL))));
    assert_eq!(caret_of(&app), 4..4);
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(35, CHORD_CTRL))));
    assert_eq!(text_of(&app), "run");
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(30, CHORD_CTRL))));
    assert_eq!(caret_of(&app), 0..0);
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(32, CHORD_CTRL))));
    assert_eq!(text_of(&app), "un");
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(37, CHORD_CTRL))));
    assert_eq!(text_of(&app), "");
    // with a selection each of them deletes the selection
    let mut app = app_with_text(SENTENCE, 6..10);
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(14, CHORD_ALT))));
    assert_eq!(text_of(&app), "Hello  the BifrOSt E2E run.");
    // the undo stack has the steps
    assert!(app.event(&ctx, Input::Command(Command::Undo)));
    assert_eq!(text_of(&app), SENTENCE);
}

#[test]
fn page_up_and_page_down_move_a_window_of_lines_and_extend_with_shift() {
    let ctx = window(1200, 780);
    let long: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let mut app = app_with_text(&long, 0..0);
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(109, 0))), "Page Down");
    let down = caret_of(&app).start;
    assert!(down > 0 && down < long.len(), "the caret moved down by a page, not to the end: {down}");
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(109, CHORD_SHIFT))), "Shift+Page Down");
    let sel = caret_of(&app);
    assert_eq!(sel.start, down, "the selection grows from where Page Down left the caret");
    assert!(sel.end > sel.start);
    assert!(app.event(&ctx, Input::Key(KeyInput::chord(104, 0))), "Page Up");
    assert_eq!(caret_of(&app).start, caret_of(&app).end, "a plain Page Up collapses the selection");
}
fn centre(r: bifrost_app::controls::Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

#[test]
fn the_format_bar_buttons_format_the_selection_and_show_its_state() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 6..10);
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
    let bold = centre(placed.rect_of(format_bar::BOLD).unwrap());
    assert!(press(&mut app, &ctx, TouchKind::Down, bold, 0.0));
    let doc = active_doc(&app.docs).unwrap();
    assert_eq!(doc.text.selection(), 6..10, "the press on a button keeps the selection");
    assert!(doc.text.cells()[6..10].iter().all(|c| c.style.bold));
    assert!(!doc.text.cells()[0].style.bold && !doc.text.cells()[10].style.bold);
    assert!(app.status().style.bold, "the bar shows the style of the selection");
    // without a selection the button sets the typing style
    active_doc_mut(&mut app.docs).unwrap().text.select(31, 31);
    let italic = centre(placed.rect_of(format_bar::ITALIC).unwrap());
    press(&mut app, &ctx, TouchKind::Down, italic, 100.0);
    app.type_into("z");
    let typed = active_doc(&app.docs).unwrap().text.cells().last().copied().unwrap();
    assert!(typed.ch == 'z' && typed.style.italic && !typed.style.bold, "{typed:?}");
}

#[test]
fn a_pop_up_opens_its_menu_and_the_choice_formats_the_selection() {
    use bifrost_app::proto::ToShell;
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 0..5);
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
    let size = centre(placed.rect_of(format_bar::SIZE).unwrap());
    assert!(!press(&mut app, &ctx, TouchKind::Down, size, 0.0));
    let sent = ctx.take_sent();
    let items = sent.iter().find_map(|m| match m {
        ToShell::ContextMenu { items, .. } => Some(items.clone()),
        _ => None,
    });
    let items = items.expect("the size pop-up opens a menu");
    assert_eq!(items.len(), text::SIZES.len());
    // the choice comes back with the menu id of its action
    let eighteen = items.iter().find(|i| i.label == "18").expect("a row for 18").id;
    assert!(app.event(&ctx, Input::ContextAction(eighteen)));
    assert!(active_doc(&app.docs).unwrap().text.cells()[..5].iter().all(|c| c.style.size == 18.0));
    assert_eq!(app.status().style.size, 18.0);
    // the style pop-up sets bold and italic together
    assert!(app.event(&ctx, Input::ContextAction(Action::FontStyle(3).id().unwrap())));
    let first = active_doc(&app.docs).unwrap().text.cells()[0].style;
    assert!(first.bold && first.italic);
    assert!(app.event(&ctx, Input::ContextAction(Action::FontStyle(0).id().unwrap())));
    let first = active_doc(&app.docs).unwrap().text.cells()[0].style;
    assert!(!first.bold && !first.italic);
}

#[test]
fn the_overflow_button_of_a_narrow_window_opens_the_hidden_controls() {
    use bifrost_app::proto::ToShell;
    let ctx = window(520, 600);
    let mut app = app_with_text(SENTENCE, 0..0);
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
    let more = placed.overflow_button.expect("the bar overflows at 520 points");
    let nodes = app.controls(&ctx);
    assert!(nodes.iter().any(|n| n.label == "More Formatting" && n.bounds == more), "the button is a control");
    assert!(nodes.iter().all(|n| n.bounds.right() <= geom.width + 0.01), "no control is cut by the window's edge");
    press(&mut app, &ctx, TouchKind::Down, centre(more), 0.0);
    let menu = ctx.take_sent().into_iter().find_map(|m| match m {
        ToShell::ContextMenu { items, .. } => Some(items),
        _ => None,
    });
    assert!(menu.is_some_and(|items| !items.is_empty() && items.len() <= 32));
    // the same through the control: assistive technology activates it
    assert!(!app.event(&ctx, Input::Activate(format_bar::NODE_BASE + u64::from(format_bar::OVERFLOW))));
    assert!(ctx.take_sent().iter().any(|m| matches!(m, ToShell::ContextMenu { .. })));
}

#[test]
fn the_controls_the_window_publishes_are_the_format_bar() {
    let ctx = window(1200, 780);
    let app = app_with_text(SENTENCE, 0..0);
    let nodes = app.controls(&ctx);
    assert_eq!(nodes.len(), format_bar::ALL.len());
    assert!(nodes.iter().all(|n| !n.label.is_empty()));
    // none of the old text buttons (New, Open, Save, Undo, Redo, Find, Print) is a control any more
    for gone in ["New", "Open", "Save", "Undo", "Redo", "Find", "Print", "Bullets", "Numbers"] {
        assert!(!nodes.iter().any(|n| n.label == gone), "{gone}");
    }
}

#[test]
fn the_title_row_names_the_document_and_says_when_it_has_unsaved_changes() {
    let mut app = app_with_text("", 0..0);
    assert_eq!(app.window_title(), "Untitled");
    app.type_into("x");
    assert_eq!(app.window_title(), "Untitled \u{2013} Edited");
    assert_eq!(TextEditApp::new().window_title(), "TextEdit", "with no document the app's own name shows");
}

/// E2E9 11: a plain text document shows no format bar (the reference shows none): no items are placed, published
/// or pressed, and the page starts under the title row.
#[test]
fn a_plain_text_document_has_no_format_bar() {
    let ctx = window(1200, 780);
    let mut app = app_with_text(SENTENCE, 0..5);
    let rich = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    assert_eq!(rich.format_h, view::FORMAT_H, "rich text has the bar");
    active_doc_mut(&mut app.docs).unwrap().rich = false;
    let geom = app.geometry(&ctx.ui, ctx.cfg.w, ctx.cfg.h);
    assert_eq!(geom.format_h, 0.0);
    assert_eq!(geom.body_top, bifrost_app::layout::UNIFIED_BAR_PT);
    let placed = format_bar::layout(&ctx.ui, &geom, &app.status());
    assert!(placed.rect_of(format_bar::BOLD).is_none(), "nothing is placed");
    assert!(format_bar::hit(&placed, 100.0, geom.format_top + 10.0).is_none());
    assert!(app.controls(&ctx).is_empty(), "no format controls are published");
}

#[test]
fn the_size_and_style_actions_count_as_formatting() {
    assert!(is_formatting(Action::Size(3)));
    assert!(is_formatting(Action::FontStyle(1)));
}

/// E2E: a .txt opened with its default app (the Dock's Downloads stack, a double-click in Files) arrives as an inbound
/// grant no panel of ours asked for. It opens as a document in front; opened again it brings that document back.
#[test]
fn a_file_opened_with_textedit_from_files_or_the_dock_opens_as_a_document() {
    let ctx = window(1200, 780);
    let mut app = TextEditApp::new();
    let untitled = app.docs.new_document();
    let dir = std::env::temp_dir().join(format!("textedit-inbound-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("e2e7-notes.txt");
    std::fs::write(&path, SENTENCE).unwrap();
    let granted = |request: u32, grant: u8| bifrost_app::Document {
        request,
        grant: [grant; 16],
        name: "e2e7-notes.txt".into(),
        size: SENTENCE.len() as u64,
        mode: bifrost_app::proto::DocMode::Read,
        file: std::fs::File::open(&path).unwrap(),
    };
    assert!(app.document(&ctx, DocumentEvent::Opened(granted(bifrost_app::share::INBOUND | 1, 1))));
    let opened = app.docs.active().unwrap();
    assert_ne!(opened, untitled, "a new document, not the empty Untitled one");
    assert_eq!(app.docs.display_name(opened).as_deref(), Some("e2e7-notes.txt"));
    assert_eq!(active_doc(&app.docs).unwrap().text.text(), SENTENCE);
    // the same file again, under the shell's new grant: no second copy, the open one comes to the front
    app.docs.set_active(untitled);
    assert!(app.document(&ctx, DocumentEvent::Opened(granted(bifrost_app::share::INBOUND | 2, 2))));
    assert_eq!(app.docs.len(), 2);
    assert_eq!(app.docs.active(), Some(opened));
    let _ = std::fs::remove_dir_all(&dir);
}
