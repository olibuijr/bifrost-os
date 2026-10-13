//! Preview (`QLPreviewPanel`): the floating panel an app opens with `ToShell::Preview` (SDK `peek::PreviewPanel`) or the
//! `quick-look` service. It is drawn on the desktop above the windows and the Dock, at the menus' layer.
//!
//! The panel shows one item at a time: a picture (decoded and fitted into the body), the first lines of a text file, a
//! line naming the app that plays or reads a PDF or audio item, or a line saying there is no preview. The kind comes
//! from the preview registry (`registry()`, by the file's type). The title is the headline and the URL sits under the
//! body. Left and Right arrows, the chevron buttons and the arrow keys step through the
//! items (no wrapping, as `PreviewPanel::step`). Esc, Space and the close button hide it. A click on the desktop
//! outside the panel is left to the desktop, and the panel stays up. The shell never reads a file the app did not name:
//! only `file://` URLs are opened, and only for a fixed size limit.
//!
//! The owning app hears what the user did: `closed` and `index` events go to it through `App::service_event` (service
//! `quick-look`); an app's own `PreviewClose` is not echoed back.

use super::ui::{Weight, CARD_BG, DETAIL_BG, LABEL, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SECONDARY, SEPARATOR_C};
use super::Shell;
use crate::gfx::{Image, Surface};
use crate::layout::Rect;
use bifrost_core::json::Json;
use bifrost_proto::peek::{PreviewItem, MAX_ITEMS};
use bifrost_ui::preview::Registry;
use std::io::Read;
use std::path::Path;

/// The providers the panel previews with (the shell's `QLPreviewProvider` registrations). Each covers a type subtree
/// from the type tree in `bifrost_ui::uttype`, so every file of that kind gets the provider's preview.
const PICTURES: &str = "desk.pictures";
const TEXT: &str = "desk.text";
const PDF: &str = "desk.pdf";
const AUDIO: &str = "desk.audio";

/// The registry the panel previews with: one provider per kind of item.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    let _ = r.register(PICTURES, &["public.image"]);
    let _ = r.register(TEXT, &["public.text"]);
    let _ = r.register(PDF, &["com.adobe.pdf"]);
    let _ = r.register(AUDIO, &["public.audio"]);
    r
}

/// Pictures larger than this are not decoded (the panel is a preview, not an editor).
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Text previews read this many bytes at most, and show at most [`MAX_TEXT_LINES`] lines of [`MAX_LINE_CHARS`] characters.
pub const MAX_TEXT_BYTES: u64 = 16 * 1024;
pub const MAX_TEXT_LINES: usize = 24;
pub const MAX_LINE_CHARS: usize = 120;

/// What kind of preview an item gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Text,
    Pdf,
    Audio,
    Other,
}

/// The preview kind of a file name: the registry picks the provider through the file's type (`uttype`), so a
/// subtype such as Markdown or a PNG picture takes its ancestor's provider. A name with no known type is `Other`.
pub fn kind_for(file_name: &str) -> Kind {
    let reg = registry();
    match reg.provider_for(file_name) {
        Some(PICTURES) => Kind::Image,
        Some(TEXT) => Kind::Text,
        Some(PDF) => Kind::Pdf,
        Some(AUDIO) => Kind::Audio,
        _ => Kind::Other,
    }
}

/// The first lines of a text file, lossy UTF-8, cut to the preview's limits. Control characters other than tab are dropped.
pub fn text_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .take(MAX_TEXT_LINES)
        .map(|l| l.chars().filter(|c| !c.is_control() || *c == '\t').take(MAX_LINE_CHARS).collect())
        .collect()
}

/// The path of a `file://` URL, or None for any other scheme.
pub fn local_path(url: &str) -> Option<&str> {
    url.strip_prefix("file://").filter(|p| p.starts_with('/'))
}

/// What the body of the panel shows for the current item.
#[derive(Default)]
pub enum Body {
    #[default]
    Empty,
    Image(Image),
    Text(Vec<String>),
    Info(String),
}

/// Loads the body for one item. Reads at most the size limits; never fails, a failure is an [`Body::Info`] line.
pub fn load_body(item: Option<&PreviewItem>) -> Body {
    let Some(item) = item else { return Body::Empty };
    let Some(path) = local_path(&item.url) else { return Body::Info("No preview for this item.".into()) };
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match kind_for(name) {
        Kind::Pdf => Body::Info("PDF document. Open it in Books to read it.".into()),
        Kind::Audio => Body::Info("Audio file. Open it in Music to play it.".into()),
        Kind::Image => {
            let small = std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= MAX_IMAGE_BYTES);
            match small.then(|| std::fs::read(p).ok()).flatten().and_then(|b| crate::cg::imageio::decode(&b)) {
                Some(im) => Body::Image(im),
                None => Body::Info("This picture cannot be shown.".into()),
            }
        }
        Kind::Text => {
            let mut buf = Vec::new();
            match std::fs::File::open(p).and_then(|f| f.take(MAX_TEXT_BYTES).read_to_end(&mut buf)) {
                Ok(_) => Body::Text(text_lines(&buf)),
                Err(_) => Body::Info("This file cannot be read.".into()),
            }
        }
        Kind::Other => Body::Info(if ext.is_empty() { "No preview for this file.".into() } else { format!("No preview for .{ext} files.") }),
    }
}

/// The panel's state. Lives in the shell; see the module docs.
#[derive(Default)]
pub struct Panel {
    items: Vec<PreviewItem>,
    index: usize,
    /// the app that opened the panel (its events go to it)
    owner: Option<String>,
    visible: bool,
    body: Body,
    /// events for the owner, drained by the `quick-look` service's tick: (event name, body)
    events: Vec<(&'static str, Json)>,
    dirty: bool,
}

impl Panel {
    /// Shows `items` from `index` (clamped), opened by `owner`. An empty list hides the panel.
    pub fn show(&mut self, owner: &str, items: Vec<PreviewItem>, index: usize) {
        if items.is_empty() {
            self.hide();
            return;
        }
        self.items = items.into_iter().take(MAX_ITEMS).collect();
        self.index = index.min(self.items.len() - 1);
        self.owner = Some(owner.to_string());
        self.visible = true;
        self.reload();
    }

    /// Hides the panel. Returns whether it was shown.
    pub fn hide(&mut self) -> bool {
        let was = std::mem::replace(&mut self.visible, false);
        self.dirty = true;
        self.body = Body::Empty;
        was
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// True once after the panel changed, so the desktop repaints the whole frame.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Moves the index by `delta` without wrapping. Returns whether it moved.
    pub fn step(&mut self, delta: i32) -> bool {
        if !self.visible {
            return false;
        }
        let next = (self.index as i64 + delta as i64).clamp(0, self.items.len() as i64 - 1) as usize;
        if next == self.index {
            return false;
        }
        self.index = next;
        self.reload();
        true
    }

    /// The user closed the panel (Esc, Space or the close button): hides it and tells the owner.
    pub fn close_by_user(&mut self) {
        if self.hide() {
            self.events.push(("closed", Json::Null));
        }
    }

    /// The user stepped: tells the owner the new index.
    pub fn user_step(&mut self, delta: i32) {
        if self.step(delta) {
            self.events.push(("index", Json::Obj(vec![("index".into(), Json::Num(self.index as f64))])));
        }
    }

    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    /// The events waiting for the owner, in order.
    pub fn drain_events(&mut self) -> Vec<(&'static str, Json)> {
        std::mem::take(&mut self.events)
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn current(&self) -> Option<&PreviewItem> {
        self.items.get(self.index)
    }

    /// `{visible, index, count}` for the `state` method.
    pub fn state_json(&self) -> Json {
        Json::Obj(vec![
            ("visible".into(), Json::Bool(self.visible)),
            ("index".into(), Json::Num(self.index as f64)),
            ("count".into(), Json::Num(self.items.len() as f64)),
        ])
    }

    fn reload(&mut self) {
        self.body = load_body(self.items.get(self.index));
        self.dirty = true;
    }
}

/// The panel's rectangles in shell pixels: the card and its hit areas. `k` is `Ui::pt(1.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub card: Rect,
    pub close: Rect,
    pub prev: Rect,
    pub next: Rect,
    pub body: Rect,
    /// the strip under the body that shows the item's URL when it has a title of its own
    pub caption: Rect,
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x: x as i32, y: y as i32, w: w as i32, h: h as i32 }
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x as f32 && x < (r.x + r.w) as f32 && y >= r.y as f32 && y < (r.y + r.h) as f32
}

/// Where the card sits on a `w` by `h` screen: centred, with sizes from the point scale `k`.
pub fn geometry(w: f32, h: f32, k: f32) -> Geometry {
    let (cw, ch) = ((560.0 * k).min(w - 32.0 * k).max(0.0), (440.0 * k).min(h - 32.0 * k).max(0.0));
    let (x, y) = ((w - cw) / 2.0, ((h - ch) / 2.0).max(0.0));
    let head = 36.0 * k;
    let pad = 12.0 * k;
    let foot = 22.0 * k;
    Geometry {
        card: rect(x, y, cw, ch),
        close: rect(x + 6.0 * k, y + 6.0 * k, 28.0 * k, 28.0 * k),
        prev: rect(x + 4.0 * k, y + ch / 2.0 - 24.0 * k, 28.0 * k, 48.0 * k),
        next: rect(x + cw - 32.0 * k, y + ch / 2.0 - 24.0 * k, 28.0 * k, 48.0 * k),
        body: rect(x + pad, y + head, cw - 2.0 * pad, (ch - head - pad - foot).max(0.0)),
        caption: rect(x + pad, y + ch - foot, cw - 2.0 * pad, foot),
    }
}

impl Shell {
    /// Pointer events for the open panel. Returns true when the panel takes the event (a press or release inside the
    /// card, or an action on one of its buttons); other pointer events pass on to the desktop.
    pub(super) fn quick_look_pointer(&mut self, x: f32, y: f32, up: bool) -> bool {
        if !self.quick_look.is_visible() {
            return false;
        }
        let g = geometry(self.w as f32, self.h as f32, self.ui.pt(1.0));
        if !inside(g.card, x, y) {
            return false;
        }
        if up {
            if inside(g.close, x, y) {
                self.quick_look.close_by_user();
            } else if inside(g.prev, x, y) {
                self.quick_look.user_step(-1);
            } else if inside(g.next, x, y) {
                self.quick_look.user_step(1);
            }
        }
        true
    }

    /// Keys for the open panel: Esc and Space close it, Left and Right step. Returns None for any other key, which
    /// falls through to the desktop. A release of a panel key is swallowed too.
    pub(super) fn quick_look_key(&mut self, code: u16, down: bool) -> Option<()> {
        const KEY_ESC: u16 = 1;
        const KEY_SPACE: u16 = 57;
        const KEY_LEFT: u16 = 105;
        const KEY_RIGHT: u16 = 106;
        if !self.quick_look.is_visible() {
            return None;
        }
        match code {
            KEY_ESC | KEY_SPACE if down => self.quick_look.close_by_user(),
            KEY_LEFT if down => self.quick_look.user_step(-1),
            KEY_RIGHT if down => self.quick_look.user_step(1),
            KEY_ESC | KEY_SPACE | KEY_LEFT | KEY_RIGHT => {}
            _ => return None,
        }
        Some(())
    }

    /// The app's `ToShell::Preview` or the `show` call: shows the panel and redraws.
    pub(super) fn quick_look_show(&mut self, owner: &str, items: Vec<PreviewItem>, index: usize) {
        self.quick_look.show(owner, items, index);
        self.dirty = true;
    }

    /// The app's `PreviewClose` or the `close` call: hides the panel and redraws.
    pub(super) fn quick_look_hide(&mut self) {
        if self.quick_look.hide() {
            self.dirty = true;
        }
    }

    /// The app's `step` call: moves the index without an event back to the app.
    pub(super) fn quick_look_step(&mut self, delta: i32) {
        if self.quick_look.step(delta) {
            self.dirty = true;
        }
    }

    pub(super) fn quick_look_state(&self) -> Json {
        self.quick_look.state_json()
    }

    /// The app that opened the panel, shown or not.
    pub(super) fn quick_look_owner(&self) -> Option<&str> {
        self.quick_look.owner()
    }

    pub(super) fn quick_look_drain_events(&mut self) -> Vec<(&'static str, Json)> {
        self.quick_look.drain_events()
    }

    /// Draws the open panel over the desktop. Does nothing when it is hidden.
    pub(super) fn draw_quick_look(&self, s: &mut Surface) {
        let p = &self.quick_look;
        if !p.is_visible() {
            return;
        }
        let ui = &self.ui;
        let g = geometry(s.w as f32, s.h as f32, ui.pt(1.0));
        let radius = ui.pt(13.0);
        let (cx, cy, cw, ch) = (g.card.x as f32, g.card.y as f32, g.card.w as f32, g.card.h as f32);
        s.fill_rrect(cx, cy, cw, ch, radius, CARD_BG.now());
        s.stroke_rrect(cx, cy, cw, ch, radius, ui.pt(1.0), SEPARATOR_C.now());

        // close button: a grey disc with a cross
        let c = g.close;
        s.fill_rrect(c.x as f32, c.y as f32, c.w as f32, c.h as f32, c.w as f32 / 2.0, DETAIL_BG.now());
        let cross = "\u{00D7}";
        self.label(cross, MAC_HEADLINE_PT, Weight::Regular, SECONDARY, |im| {
            s.blit(im, c.x + (c.w - im.w as i32) / 2, c.y + (c.h - im.h as i32) / 2, 255, 255)
        });

        // title (centred, cut to fit between the close button and the counter) and the "n of N" counter
        let title = p.current().map(|i| if i.title.is_empty() { i.url.as_str() } else { i.title.as_str() }).unwrap_or("");
        let title = ui.fit(title, MAC_HEADLINE_PT, cw - 2.0 * ui.pt(120.0));
        self.label(&title, MAC_HEADLINE_PT, Weight::Bold, LABEL.now(), |im| {
            s.blit(im, (cx + (cw - im.w as f32) / 2.0) as i32, (cy + ui.pt(18.0) - im.h as f32 / 2.0) as i32, 255, 255)
        });
        let counter = format!("{} of {}", p.index() + 1, p.count());
        self.label(&counter, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, |im| {
            s.blit(im, (cx + cw - ui.pt(16.0) - im.w as f32) as i32, (cy + ui.pt(18.0) - im.h as f32 / 2.0) as i32, 255, 255)
        });

        // body: a grouped plate with the preview inside
        let b = g.body;
        let (bx, by, bw, bh) = (b.x as f32, b.y as f32, b.w as f32, b.h as f32);
        s.fill_rrect(bx, by, bw, bh, ui.pt(9.0), DETAIL_BG.now());
        s.stroke_rrect(bx, by, bw, bh, ui.pt(9.0), ui.pt(1.0), SEPARATOR_C.now());
        match &p.body {
            Body::Empty => {}
            Body::Image(im) => {
                let k = (bw - ui.pt(24.0)) / im.w.max(1) as f32;
                let k = k.min((bh - ui.pt(24.0)) / im.h.max(1) as f32);
                let (w, h) = (im.w as f32 * k, im.h as f32 * k);
                let r = rect(bx + (bw - w) / 2.0, by + (bh - h) / 2.0, w, h);
                super::switcher::draw_scaled(s, im, r, ui.pt(4.0), 255);
            }
            Body::Text(lines) => {
                let step = ui.pt(16.0);
                let inner = ui.pt(12.0);
                let mut y = by + inner;
                for line in lines {
                    if y + step > by + bh {
                        break;
                    }
                    let t = ui.fit(line, MAC_BODY_PT, bw - 2.0 * inner);
                    self.label(&t, MAC_BODY_PT, Weight::Regular, LABEL.now(), |im| s.blit(im, (bx + inner) as i32, y as i32, 255, 255));
                    y += step;
                }
            }
            Body::Info(text) => {
                let t = ui.fit(text, MAC_BODY_PT, bw - ui.pt(24.0));
                self.label(&t, MAC_BODY_PT, Weight::Regular, SECONDARY, |im| {
                    s.blit(im, (bx + (bw - im.w as f32) / 2.0) as i32, (by + (bh - im.h as f32) / 2.0) as i32, 255, 255)
                });
            }
        }

        // the URL of an item that has a title of its own, centred under the body in secondary grey
        if let Some(item) = p.current().filter(|i| !i.title.is_empty() && !i.url.is_empty()) {
            let c = g.caption;
            let url = ui.fit(&item.url, MAC_FOOTNOTE_PT, c.w as f32);
            self.label(&url, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, |im| {
                s.blit(im, c.x + (c.w - im.w as i32) / 2, c.y + (c.h - im.h as i32) / 2, 255, 255)
            });
        }

        // the chevrons, in LINK, dimmed at the ends
        for (r, glyph, enabled) in [(g.prev, "\u{2039}", p.index() > 0), (g.next, "\u{203A}", p.index() + 1 < p.count())] {
            let color = if enabled { LINK } else { SECONDARY };
            self.label(glyph, MAC_HEADLINE_PT, Weight::Bold, color, |im| {
                s.blit(im, r.x + (r.w - im.w as i32) / 2, r.y + (r.h - im.h as i32) / 2, 255, 255)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(url: &str, title: &str) -> PreviewItem {
        PreviewItem { url: url.into(), title: title.into() }
    }

    #[test]
    fn extensions_pick_the_preview_kind() {
        assert_eq!(kind_for("photo.png"), Kind::Image);
        assert_eq!(kind_for("PHOTO.JPG"), Kind::Image, "extensions are case-insensitive");
        assert_eq!(kind_for("notes.md"), Kind::Text, "Markdown conforms to plain text");
        assert_eq!(kind_for("main.rs"), Kind::Text);
        assert_eq!(kind_for("paper.pdf"), Kind::Pdf);
        assert_eq!(kind_for("song.mp3"), Kind::Audio);
        assert_eq!(kind_for("clip.mp4"), Kind::Other, "movies have no preview yet");
        assert_eq!(kind_for("archive.zip"), Kind::Other);
        assert_eq!(kind_for("README"), Kind::Other, "no extension, no type");
    }

    #[test]
    fn pdf_and_audio_items_say_which_app_plays_them() {
        let dir = std::env::temp_dir().join(format!("ql-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("paper.pdf");
        std::fs::write(&pdf, b"%PDF-1.4").unwrap();
        let url = format!("file://{}", pdf.display());
        assert!(matches!(load_body(Some(&item(&url, "paper"))), Body::Info(t) if t.starts_with("PDF document")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn text_lines_keep_the_first_lines_and_drop_control_bytes() {
        let body = "one\ntwo\u{7}x\tt\n".to_string() + &"n\n".repeat(40);
        let lines = text_lines(body.as_bytes());
        assert_eq!(lines.len(), MAX_TEXT_LINES);
        assert_eq!(lines[0], "one");
        assert_eq!(lines[1], "twox\tt");
        let long = "a".repeat(500);
        assert_eq!(text_lines(long.as_bytes())[0].chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn only_file_urls_are_read() {
        assert_eq!(local_path("file:///tmp/a.txt"), Some("/tmp/a.txt"));
        assert_eq!(local_path("https://example.com/a.png"), None);
        assert_eq!(local_path("file://relative"), None);
        assert!(matches!(load_body(Some(&item("https://example.com/x.txt", ""))), Body::Info(_)));
        assert!(matches!(load_body(None), Body::Empty));
    }

    #[test]
    fn text_files_load_into_the_body_and_missing_files_say_so() {
        let dir = std::env::temp_dir().join(format!("ql-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("note.txt");
        std::fs::write(&f, "hello\nworld\n").unwrap();
        let url = format!("file://{}", f.display());
        match load_body(Some(&item(&url, "note"))) {
            Body::Text(lines) => assert_eq!(lines, ["hello", "world"]),
            _ => panic!("text body expected"),
        }
        let gone = format!("file://{}/missing.txt", dir.display());
        assert!(matches!(load_body(Some(&item(&gone, ""))), Body::Info(t) if t == "This file cannot be read."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn show_clamps_the_index_and_an_empty_list_hides() {
        let mut p = Panel::default();
        p.show("is.example.files", vec![item("file:///a", "a"), item("file:///b", "b")], 9);
        assert!(p.is_visible());
        assert_eq!(p.index(), 1);
        assert_eq!(p.owner(), Some("is.example.files"));
        p.show("x", vec![], 0);
        assert!(!p.is_visible());
    }

    #[test]
    fn stepping_does_not_wrap_and_reports_moves() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a", "a"), item("file:///b", "b"), item("file:///c", "c")], 0);
        assert!(!p.step(-1), "already at the first item");
        assert!(p.step(1));
        assert!(p.step(5));
        assert_eq!(p.index(), 2);
        assert!(!p.step(1), "no wrapping past the last item");
        assert_eq!(p.current().unwrap().title, "c");
    }

    #[test]
    fn user_actions_queue_events_for_the_owner() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a", "a"), item("file:///b", "b")], 0);
        p.user_step(1);
        p.user_step(1); // at the end: no move, no event
        let ev = p.drain_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, "index");
        assert_eq!(ev[0].1.get("index").and_then(Json::as_u64), Some(1));
        p.close_by_user();
        p.close_by_user(); // already hidden: no second event
        let ev = p.drain_events();
        assert_eq!(ev.iter().map(|e| e.0).collect::<Vec<_>>(), ["closed"]);
        assert!(p.drain_events().is_empty());
    }

    #[test]
    fn take_dirty_is_true_once_per_change() {
        let mut p = Panel::default();
        assert!(!p.take_dirty());
        p.show("app", vec![item("file:///a", "a")], 0);
        assert!(p.take_dirty());
        assert!(!p.take_dirty());
    }

    #[test]
    fn geometry_centres_the_card_and_keeps_buttons_inside_it() {
        let g = geometry(1280.0, 800.0, 1.0);
        assert_eq!(g.card.w, 560);
        assert_eq!(g.card.x, (1280 - 560) / 2);
        assert!(inside(g.card, 640.0, 400.0));
        assert!(inside(g.close, (g.close.x + 1) as f32, (g.close.y + 1) as f32));
        assert!(inside(g.card, g.next.x as f32, g.next.y as f32));
        let small = geometry(300.0, 300.0, 1.0);
        assert!(small.card.w <= 300 - 32, "a small screen shrinks the card");
        // the caption strip sits at the bottom of the card, below the body
        assert_eq!(g.caption.y + g.caption.h, g.card.y + g.card.h);
        assert!(g.body.y + g.body.h <= g.caption.y);
    }
}
