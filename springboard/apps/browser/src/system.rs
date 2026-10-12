//! The Browser's use of the desktop's system services: the pasteboard (a copied link as its URL, a link in HTML and the
//! URL as text; a copied image file), the network path (the offline page that loads the address when the network comes
//! back), the pointer shape (an I-beam over the address field) and the dock's download progress.
//!
//! Each part is a small model with no drawing, so the rules are unit tested here. The sends are one call each, made by
//! `main.rs` on a real event only (a key chord, a path change, a finger or pointer move over the chrome, a download
//! tick that changed the total). Services that an older shell does not have answer "no service", and the Browser
//! carries on: a copy then falls back to the synchronous pasteboard.
use std::fs;
use std::path::Path;

use bifrost_app::controls::cursor::Kind;
use bifrost_app::dock_tile;
use bifrost_app::cursor_service;
use bifrost_app::network_path::Status;
use bifrost_app::pasteboard::NativePasteboard;
use bifrost_app::pasteboard_types::{Board, Item, HTML, PNG, STRING, URL};
use bifrost_app::webview::WebViewId;
use bifrost_app::Ctx;

use crate::downloads::{self, State};

/// The JPEG pasteboard type (its UTI).
pub const JPEG: &str = "public.jpeg";
/// Images larger than this are not read for the pasteboard.
pub const MAX_IMAGE_BYTES: u64 = 16 << 20;
/// The service sends an image as hex in one JSON body of at most 64 KiB; larger images take the synchronous path.
const SERVICE_IMAGE_MAX: usize = 24 * 1024;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---- Pasteboard -------------------------------------------------------------------------------------------------

/// The representations of a copied link: the URL, the title as an HTML link, and the URL as plain text. A blank title
/// is replaced by the URL.
pub fn link_item(url: &str, title: &str) -> Item {
    let title = title.trim();
    let title = if title.is_empty() { url } else { title };
    let html = format!("<a href=\"{}\">{}</a>", esc(url), esc(title));
    Item::default().with(URL, url.as_bytes()).with(STRING, url.as_bytes()).with(HTML, html.as_bytes())
}

/// The pasteboard type of an image file the Browser can copy (PNG or JPEG, by extension).
pub fn image_uti(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some(PNG),
        "jpg" | "jpeg" => Some(JPEG),
        _ => None,
    }
}

/// Reads an image file for the pasteboard: its type and bytes. None for other types, missing files and files over
/// [`MAX_IMAGE_BYTES`].
pub fn read_image(path: &Path) -> Option<(&'static str, Vec<u8>)> {
    let uti = image_uti(path)?;
    if fs::metadata(path).ok()?.len() > MAX_IMAGE_BYTES {
        return None;
    }
    Some((uti, fs::read(path).ok()?))
}

/// The newest finished download that is an image. Downloads are kept newest first.
pub fn newest_image(items: &[downloads::Item]) -> Option<&downloads::Item> {
    items.iter().find(|i| i.state == State::Done && image_uti(&i.path).is_some())
}

/// How an image reaches the pasteboard.
#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// The `pasteboard` service, which answers and can fail on an older shell.
    Service,
    /// The synchronous pasteboard, for images too large for one service body.
    Direct,
}

pub fn image_route(len: usize) -> Route {
    if len <= SERVICE_IMAGE_MAX {
        Route::Service
    } else {
        Route::Direct
    }
}

/// One copy the Browser makes. It is kept until the service answers, so a failed call can fall back.
#[derive(Debug, PartialEq)]
pub enum Clip {
    Link { url: String, title: String },
    Image { uti: &'static str, bytes: Vec<u8> },
}

/// Sends a copy to the shared pasteboard. Returns the request id and the copy when the answer is still due (the
/// fallback is then `fall_back`); None when the copy was made on the synchronous path already.
pub fn send_copy(ctx: &Ctx, copy: Clip) -> Option<(u32, Clip)> {
    let id = match &copy {
        Clip::Link { url, title } => Some(Board::general().write(ctx, &[link_item(url, title)])),
        Clip::Image { uti, bytes } => match image_route(bytes.len()) {
            Route::Service => Some(Board::general().write(ctx, &[Item::default().with(uti, bytes)])),
            Route::Direct => {
                NativePasteboard::general().set_data(ctx, bytes, uti);
                None
            }
        },
    };
    id.map(|id| (id, copy))
}

/// Puts a copy that the service refused on the synchronous pasteboard: the URL as text, or the image bytes.
pub fn fall_back(ctx: &Ctx, copy: &Clip) {
    match copy {
        Clip::Link { url, .. } => NativePasteboard::general().set_string(ctx, url),
        Clip::Image { uti, bytes } => NativePasteboard::general().set_data(ctx, bytes, uti),
    }
}

// ---- Network path -----------------------------------------------------------------------------------------------

/// The address the Browser shows while the network is down: a page that says so, with a link to try again.
pub fn offline_url(url: &str) -> String {
    // compliance-allow: LAY-03 (600 is a CSS font weight, not a screen size)
    const HEADING: &str = "font-size:20px;font-weight:600";
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>You are offline</title>\
         <body style=\"font:15px -apple-system,sans-serif;text-align:center;padding:18vh 2em;color:#888\">\
         <h2 style=\"{HEADING}\">You are offline</h2>\
         <p>The page loads when the network comes back.</p><p>{}</p>\
         <p><a href=\"{}\">Try again</a></p>",
        esc(url),
        esc(url)
    );
    let enc: String = html.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("data:text/html;charset=utf-8,{enc}")
}

/// Offline state from the network path. While offline, an address the user chooses shows the offline page, and the
/// address is remembered per tab so it loads once the network is back.
#[derive(Debug, Default)]
pub struct Network {
    offline: bool,
    waiting: Vec<(WebViewId, String)>,
}

impl Network {
    pub fn offline(&self) -> bool {
        self.offline
    }

    /// Records a path status. True when the connection changed (online and offline both count).
    pub fn set_status(&mut self, status: Status) -> bool {
        let offline = status != Status::Satisfied;
        let changed = offline != self.offline;
        self.offline = offline;
        changed
    }

    /// The tab shows the offline page for `url`.
    pub fn hold(&mut self, view: WebViewId, url: &str) {
        self.forget(view);
        self.waiting.push((view, url.to_string()));
    }

    /// The tab left the offline page (the user chose something else, or the page committed).
    pub fn forget(&mut self, view: WebViewId) {
        self.waiting.retain(|(v, _)| *v != view);
    }

    /// The tabs to load again now that the network is back.
    pub fn take_waiting(&mut self) -> Vec<(WebViewId, String)> {
        std::mem::take(&mut self.waiting)
    }
}

// ---- Pointer shape ----------------------------------------------------------------------------------------------

/// The pointer shape over the Browser's chrome: an I-beam over the address field, the arrow elsewhere.
#[derive(Debug, Default)]
pub struct Pointer {
    pushed: bool,
}

#[derive(Debug, PartialEq)]
pub enum PointerChange {
    Push(Kind),
    Pop,
}

impl Pointer {
    /// The change to send for the pointer's new place, or None when the shape stays as it is.
    pub fn update(&mut self, over_text: bool) -> Option<PointerChange> {
        match (self.pushed, over_text) {
            (false, true) => {
                self.pushed = true;
                Some(PointerChange::Push(Kind::IBeam))
            }
            (true, false) => {
                self.pushed = false;
                Some(PointerChange::Pop)
            }
            _ => None,
        }
    }
}

pub fn send_pointer(ctx: &Ctx, change: PointerChange) {
    match change {
        PointerChange::Push(kind) => {
            cursor_service::push(ctx, kind);
        }
        PointerChange::Pop => {
            cursor_service::pop(ctx);
        }
    }
}

// ---- Dock progress ----------------------------------------------------------------------------------------------

/// The dock's download progress: none, an unknown total, or the share of the known total received.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub enum Dock {
    #[default]
    Idle,
    Indeterminate,
    Fraction(f32),
}

/// The dock progress of the running downloads. Any running download with an unknown total makes it indeterminate.
pub fn dock_progress(items: &[downloads::Item]) -> Dock {
    let active: Vec<&downloads::Item> = items.iter().filter(|i| i.state == State::Active).collect();
    if active.is_empty() {
        return Dock::Idle;
    }
    let (mut got, mut total) = (0u64, 0u64);
    for i in active {
        match i.total {
            Some(t) if t > 0 => {
                got += i.received.min(t);
                total += t;
            }
            _ => return Dock::Indeterminate,
        }
    }
    Dock::Fraction((got as f32 / total as f32).clamp(0.0, 1.0))
}

/// True when the dock must hear about `next`: a change of kind, or a fraction that moved by at least 1 %.
pub fn dock_changed(prev: Dock, next: Dock) -> bool {
    match (prev, next) {
        (Dock::Fraction(a), Dock::Fraction(b)) => (a - b).abs() >= 0.01,
        (a, b) => a != b,
    }
}

pub fn send_dock(ctx: &Ctx, dock: Dock) {
    match dock {
        Dock::Idle => {
            dock_tile::set_progress(ctx, None);
        }
        Dock::Indeterminate => {
            dock_tile::set_indeterminate(ctx);
        }
        Dock::Fraction(f) => {
            dock_tile::set_progress(ctx, Some(f));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(item: &Item, uti: &str) -> Option<Vec<u8>> {
        item.data(uti).map(|d| d.to_vec())
    }

    #[test]
    fn a_copied_link_carries_its_url_the_title_as_html_and_the_url_as_text() {
        let item = link_item("https://a.test/?x=1&y=2", "A <b>title</b>");
        assert_eq!(rep(&item, URL).unwrap(), b"https://a.test/?x=1&y=2");
        assert_eq!(rep(&item, STRING).unwrap(), b"https://a.test/?x=1&y=2");
        let html = String::from_utf8(rep(&item, HTML).unwrap()).unwrap();
        assert_eq!(html, "<a href=\"https://a.test/?x=1&amp;y=2\">A &lt;b&gt;title&lt;/b&gt;</a>");
    }

    #[test]
    fn a_blank_title_falls_back_to_the_url() {
        let item = link_item("https://a.test/", "   ");
        let html = String::from_utf8(rep(&item, HTML).unwrap()).unwrap();
        assert_eq!(html, "<a href=\"https://a.test/\">https://a.test/</a>");
    }

    #[test]
    fn image_types_follow_the_extension() {
        assert_eq!(image_uti(Path::new("/d/cat.PNG")), Some(PNG));
        assert_eq!(image_uti(Path::new("/d/cat.jpeg")), Some(JPEG));
        assert_eq!(image_uti(Path::new("/d/cat.jpg")), Some(JPEG));
        assert_eq!(image_uti(Path::new("/d/cat.webp")), None);
        assert_eq!(image_uti(Path::new("/d/cat")), None);
    }

    #[test]
    fn small_images_go_through_the_service_and_large_ones_take_the_synchronous_path() {
        assert_eq!(image_route(SERVICE_IMAGE_MAX), Route::Service);
        assert_eq!(image_route(SERVICE_IMAGE_MAX + 1), Route::Direct);
    }

    #[test]
    fn the_newest_finished_image_is_the_one_copied() {
        let items = vec![
            downloads::Item::sample("/d/new.png", State::Active, Some(10), 4),
            downloads::Item::sample("/d/doc.pdf", State::Done, None, 9),
            downloads::Item::sample("/d/photo.jpg", State::Done, None, 9),
            downloads::Item::sample("/d/old.png", State::Done, None, 9),
        ];
        assert_eq!(newest_image(&items).unwrap().path, Path::new("/d/photo.jpg"));
        assert!(newest_image(&items[..1]).is_none());
    }

    #[test]
    fn the_dock_shows_idle_indeterminate_or_the_received_share() {
        assert_eq!(dock_progress(&[]), Dock::Idle);
        let done = [downloads::Item::sample("/d/a", State::Done, Some(4), 4)];
        assert_eq!(dock_progress(&done), Dock::Idle);
        let unknown = [downloads::Item::sample("/d/a", State::Active, None, 100)];
        assert_eq!(dock_progress(&unknown), Dock::Indeterminate);
        let two = [
            downloads::Item::sample("/d/a", State::Active, Some(100), 50),
            downloads::Item::sample("/d/b", State::Active, Some(300), 150),
        ];
        // 50 of 100 and 150 of 300: 200 of 400 bytes
        assert_eq!(dock_progress(&two), Dock::Fraction(0.5));
    }

    #[test]
    fn the_dock_is_told_only_about_real_changes() {
        assert!(dock_changed(Dock::Idle, Dock::Fraction(0.0)));
        assert!(!dock_changed(Dock::Fraction(0.5), Dock::Fraction(0.505)));
        assert!(dock_changed(Dock::Fraction(0.5), Dock::Fraction(0.52)));
        assert!(dock_changed(Dock::Fraction(0.5), Dock::Indeterminate));
        assert!(!dock_changed(Dock::Idle, Dock::Idle));
    }

    #[test]
    fn network_changes_are_reported_once_and_held_pages_are_released() {
        let mut n = Network::default();
        assert!(!n.set_status(Status::Satisfied));
        assert!(n.set_status(Status::Unsatisfied));
        assert!(n.offline());
        assert!(!n.set_status(Status::Unsatisfied));
        n.hold(1, "https://a.test/");
        n.hold(1, "https://b.test/");
        n.hold(2, "https://c.test/");
        n.forget(2);
        assert_eq!(n.take_waiting(), vec![(1, "https://b.test/".to_string())]);
        assert!(n.take_waiting().is_empty());
        assert!(n.set_status(Status::Satisfied));
        assert!(!n.offline());
    }

    #[test]
    fn the_pointer_is_pushed_once_over_the_address_field_and_popped_on_leaving() {
        let mut p = Pointer::default();
        assert_eq!(p.update(false), None);
        assert_eq!(p.update(true), Some(PointerChange::Push(Kind::IBeam)));
        assert_eq!(p.update(true), None);
        assert_eq!(p.update(false), Some(PointerChange::Pop));
        assert_eq!(p.update(false), None);
    }

    #[test]
    fn the_offline_page_is_a_data_page_that_links_back_to_the_address() {
        let url = offline_url("https://a.test/?q=<x>");
        assert!(url.starts_with("data:text/html;charset=utf-8,"));
        let page = url.trim_start_matches("data:text/html;charset=utf-8,");
        assert!(!page.contains('<') && !page.contains(' '));
        assert!(page.contains("%3Ca%20href%3D%22https%3A%2F%2Fa%2Etest%2F%3Fq%3D%26lt%3Bx%26gt%3B%22"));
    }
}
