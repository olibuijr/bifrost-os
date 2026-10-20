//! Keeps three clipboards one: the shell-wide text clipboard (`clipboard`, what the shell's own fields copy and paste),
//! the typed pasteboard (`pasteboard`, what the apps copy through `NativePasteboard`) and the window server's selection,
//! which every other Wayland program reads (`wl-paste`, a terminal, the browser).
//!
//! ```text
//!   shell field --set_text--> clipboard --+                       +--> apps (ToApp::Pasteboard + Clipboard)
//!   app A -- SetClipboard / PasteboardSet-+--> pasteboard (count) +--> window server selection (offer)
//!   program --selection (incoming)--------+--------^
//! ```
//!
//! `tick` runs once per shell pass (`Shell::ext_tick`):
//!
//! 1. A selection another program set since the last pass is imported: its text and rich types become the pasteboard,
//!    its text the shell clipboard. It is not offered back (the program keeps serving it).
//! 2. A change of the shell clipboard that the pasteboard does not show yet (a copy in a shell field, an app that sent
//!    only text) becomes a plain-text pasteboard item.
//! 3. A pasteboard whose change count differs from the one last offered is offered to the window server as the selection,
//!    in every MIME type that stands for it ([`offers`]).
//!
//! The apps follow the pasteboard's change count on their own (`AppHost::sync_pasteboard`).
//!
//! The window server side is a data-control client (`pasteboard_wl.rs`, a [`Transport`]): the shell connects through the
//! privileged socket, where the window server lets it set the selection without a keyboard-focused surface. Nothing
//! here logs what was copied.

use super::{clipboard, pasteboard};
use std::sync::{Arc, Mutex};

/// One offered type: the MIME type and its bytes.
pub type Offer = (String, Arc<Vec<u8>>);

/// What was read from another program's selection: the MIME type and its bytes.
pub type Read = (String, Vec<u8>);

/// The window server's selection, as the shell sees it.
pub trait Transport: Send {
    /// Makes `offers` the selection (an empty list clears it).
    fn set(&mut self, offers: Vec<Offer>);
    /// The selection another program set since the last call, as the types worth importing (see [`wanted`]) with their
    /// bytes; `None` when nothing new arrived.
    fn incoming(&mut self) -> Option<Vec<Read>>;
}

impl Transport for Box<dyn Transport> {
    fn set(&mut self, offers: Vec<Offer>) {
        (**self).set(offers)
    }
    fn incoming(&mut self) -> Option<Vec<Read>> {
        (**self).incoming()
    }
}

/// The MIME types text goes out as, most specific first: the standard ones, then the names older programs ask for.
pub const TEXT_MIMES: [&str; 5] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING"];

/// The most one imported type may weigh (a larger one is left out).
pub const MAX_IMPORT_BYTES: usize = 32 << 20;

fn is_text_mime(mime: &str) -> bool {
    let base = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    base == "text/plain" || matches!(mime, "UTF8_STRING" | "TEXT" | "STRING")
}

/// The types the pasteboard `items` are offered as. Text goes out under every name in [`TEXT_MIMES`]; each other item
/// under its own MIME type (`pasteboard::mime_for`); a type with no MIME form stays inside BifrOSt.
pub fn offers(items: &[(String, Vec<u8>)]) -> Vec<Offer> {
    let mut out: Vec<Offer> = Vec::new();
    if let Some(text) = pasteboard::text_of(items) {
        let bytes = Arc::new(text.into_bytes());
        out.extend(TEXT_MIMES.iter().map(|m| (m.to_string(), bytes.clone())));
    }
    for (uti, data) in items {
        let mime = pasteboard::mime_for(uti);
        if !mime.contains('/') || is_text_mime(mime) || out.iter().any(|(m, _)| m == mime) {
            continue;
        }
        let bytes = Arc::new(data.clone());
        if mime == "text/rtf" {
            out.push(("application/rtf".to_string(), bytes.clone()));
        }
        out.push((mime.to_string(), bytes));
    }
    out
}

/// The UTType id of a MIME type read from another program, or `None` for a type the pasteboard does not carry.
pub fn uti_for(mime: &str) -> Option<&'static str> {
    if is_text_mime(mime) {
        return Some(pasteboard::PLAIN_TEXT);
    }
    match mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase().as_str() {
        "text/html" => Some("public.html"),
        "text/rtf" | "application/rtf" => Some("public.rtf"),
        "image/png" => Some("public.png"),
        "image/jpeg" => Some("public.jpeg"),
        "text/uri-list" => Some("public.file-url"),
        _ => None,
    }
}

/// Which of the `offered` MIME types are worth reading from another program's selection, in reading order: the best
/// text type first (one is enough: they stand for the same string), then each rich type once.
pub fn wanted(offered: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(text) = TEXT_MIMES.iter().find_map(|t| offered.iter().find(|o| o.eq_ignore_ascii_case(t))).or_else(|| offered.iter().find(|o| is_text_mime(o))) {
        out.push(text.clone());
    }
    let mut seen: Vec<&'static str> = vec![pasteboard::PLAIN_TEXT];
    for mime in offered {
        if let Some(uti) = uti_for(mime) {
            if !seen.contains(&uti) {
                seen.push(uti);
                out.push(mime.clone());
            }
        }
    }
    out
}

/// The pasteboard items for what was read: one entry per UTType (the first read wins), text made valid UTF-8, empty
/// and oversized data dropped. The plain-text item comes first.
pub fn items_from(read: Vec<Read>) -> Vec<(String, Vec<u8>)> {
    let mut items: Vec<(String, Vec<u8>)> = Vec::new();
    for (mime, data) in read {
        let Some(uti) = uti_for(&mime) else { continue };
        if data.is_empty() || data.len() > MAX_IMPORT_BYTES || items.iter().any(|(u, _)| u == uti) {
            continue;
        }
        let data = if uti == pasteboard::PLAIN_TEXT { String::from_utf8_lossy(&data).into_owned().into_bytes() } else { data };
        items.push((uti.to_string(), data));
    }
    items.sort_by_key(|(u, _)| u != pasteboard::PLAIN_TEXT);
    items
}

/// The state of one synchronisation: the transport and what has been reflected so far.
pub struct Board<T: Transport> {
    transport: T,
    /// The shell clipboard's revision the pasteboard reflects.
    seen_rev: u64,
    /// The pasteboard change count the selection was last made from (`None`: never).
    offered: Option<u32>,
}

impl<T: Transport> Board<T> {
    pub fn new(transport: T) -> Self {
        Board { transport, seen_rev: 0, offered: None }
    }

    /// One pass (see the module docs).
    pub fn pass(&mut self) {
        if let Some(read) = self.transport.incoming() {
            let items = items_from(read);
            if !items.is_empty() {
                let text = pasteboard::set_items(vec![items]);
                clipboard::set_text(text.as_deref().unwrap_or(""));
                self.seen_rev = clipboard::revision();
                self.offered = Some(pasteboard::count());
                return;
            }
        }
        let rev = clipboard::revision();
        if rev != self.seen_rev {
            self.seen_rev = rev;
            let now = clipboard::text();
            if pasteboard::text() != now {
                pasteboard::set_plain(now.as_deref().unwrap_or(""));
            }
        }
        let count = pasteboard::count();
        if self.offered != Some(count) {
            let first = self.offered.is_none();
            self.offered = Some(count);
            let (_, items) = pasteboard::snapshot();
            // an empty board at start must not wipe a selection that was there before the shell
            if !(first && items.is_empty()) {
                self.transport.set(offers(&items));
            }
        }
    }
}

/// The transport for a shell with no window server connection (and for tests): offers go nowhere, nothing arrives.
pub struct Detached;

impl Transport for Detached {
    fn set(&mut self, _offers: Vec<Offer>) {}
    fn incoming(&mut self) -> Option<Vec<Read>> {
        None
    }
}

static SYNC: Mutex<Option<Board<Box<dyn Transport>>>> = Mutex::new(None);

#[cfg(all(feature = "desktop-wayland", not(test)))]
fn default_transport() -> Box<dyn Transport> {
    match super::pasteboard_wl::Wl::start() {
        Some(wl) => Box::new(wl),
        None => Box::new(Detached),
    }
}

#[cfg(not(all(feature = "desktop-wayland", not(test))))]
fn default_transport() -> Box<dyn Transport> {
    Box::new(Detached)
}

/// The shell's pass (`Shell::ext_tick`): starts the window server connection on first use.
pub fn tick() {
    let mut g = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| Board::new(default_transport())).pass();
}

/// A descriptor that becomes readable when a program's selection arrived, for the shell's poll loop; `-1` before the
/// connection exists.
#[cfg(all(feature = "desktop-wayland", not(test)))]
pub fn poll_fd() -> std::ffi::c_int {
    super::pasteboard_wl::notify_fd()
}

/// A descriptor that becomes readable when a program's selection arrived (none without the window server client).
#[cfg(not(all(feature = "desktop-wayland", not(test))))]
pub fn poll_fd() -> std::ffi::c_int {
    -1
}

#[cfg(test)]
mod tests {
    use super::super::pasteboard::TURN;
    use super::*;
    use std::collections::VecDeque;

    /// A window server in a box: remembers the offers, hands out the selections a test queues.
    #[derive(Default)]
    struct Fake {
        offered: Vec<Vec<Offer>>,
        arriving: VecDeque<Vec<Read>>,
    }

    impl Transport for Arc<Mutex<Fake>> {
        fn set(&mut self, offers: Vec<Offer>) {
            self.lock().unwrap().offered.push(offers);
        }
        fn incoming(&mut self) -> Option<Vec<Read>> {
            self.lock().unwrap().arriving.pop_front()
        }
    }

    fn rep(uti: &str, data: &str) -> (String, Vec<u8>) {
        (uti.to_string(), data.as_bytes().to_vec())
    }

    fn mimes(offers: &[Offer]) -> Vec<&str> {
        offers.iter().map(|(m, _)| m.as_str()).collect()
    }

    fn setup() -> (Board<Arc<Mutex<Fake>>>, Arc<Mutex<Fake>>) {
        let fake = Arc::new(Mutex::new(Fake::default()));
        (Board::new(fake.clone()), fake)
    }

    /// Takes the turn on the process-wide pasteboard, empties it, and gives the test an empty clipboard of its own.
    fn fresh() -> (std::sync::MutexGuard<'static, ()>, std::sync::MutexGuard<'static, ()>) {
        let turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let g = clipboard::tests::exclusive();
        pasteboard::set_plain("");
        (turn, g)
    }

    #[test]
    fn text_goes_out_under_every_text_name_with_the_rich_types_beside_it() {
        let items = vec![rep("public.utf8-plain-text", "góðan"), rep("public.html", "<b>góðan</b>"), rep("public.rtf", "{\\rtf1}"), rep("com.example.private", "x")];
        let out = offers(&items);
        assert_eq!(mimes(&out), ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING", "text/html", "application/rtf", "text/rtf"]);
        assert!(out.iter().take(5).all(|(_, d)| d.as_slice() == "góðan".as_bytes()));
        assert_eq!(offers(&[]).len(), 0);
        let pic = offers(&[rep("public.png", "png")]);
        assert_eq!(mimes(&pic), ["image/png"], "an image alone offers no text");
    }

    #[test]
    fn file_urls_are_offered_as_a_uri_list_and_as_text() {
        let out = offers(&[rep("public.file-url", "file:///home/a.txt")]);
        assert_eq!(mimes(&out), ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING", "text/uri-list"]);
    }

    #[test]
    fn mime_types_map_back_to_pasteboard_types() {
        for t in TEXT_MIMES {
            assert_eq!(uti_for(t), Some("public.utf8-plain-text"), "{t}");
        }
        assert_eq!(uti_for("text/plain;charset=UTF-8"), Some("public.utf8-plain-text"));
        assert_eq!(uti_for("text/html"), Some("public.html"));
        assert_eq!(uti_for("application/rtf"), Some("public.rtf"));
        assert_eq!(uti_for("image/png"), Some("public.png"));
        assert_eq!(uti_for("text/uri-list"), Some("public.file-url"));
        assert_eq!(uti_for("application/x-secret"), None);
    }

    #[test]
    fn reading_takes_one_text_type_and_each_rich_type_once() {
        let offered: Vec<String> = ["STRING", "text/x-private-context", "UTF8_STRING", "text/plain", "text/html", "image/png", "text/rtf", "application/rtf", "text/uri-list"].iter().map(|s| s.to_string()).collect();
        assert_eq!(wanted(&offered), ["text/plain", "text/html", "image/png", "text/rtf", "text/uri-list"]);
        assert_eq!(wanted(&["STRING".to_string()]), ["STRING"]);
        assert!(wanted(&["application/x-x".to_string()]).is_empty());
    }

    #[test]
    fn imported_items_are_clean_and_text_comes_first() {
        let read = vec![("image/png".to_string(), vec![1, 2]), ("text/plain".to_string(), vec![0x68, 0xff, 0x69]), ("STRING".to_string(), b"dup".to_vec()), ("text/html".to_string(), Vec::new()), ("application/x-x".to_string(), vec![1])];
        let items = items_from(read);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].0, "public.utf8-plain-text");
        assert_eq!(String::from_utf8(items[0].1.clone()).unwrap(), "h\u{fffd}i", "bytes that are not UTF-8 are replaced, not dropped");
        assert_eq!(items[1], ("public.png".to_string(), vec![1, 2]));
    }

    #[test]
    fn a_copy_in_a_shell_field_reaches_the_selection_and_the_pasteboard() {
        let _fresh = fresh();
        let (mut sync, fake) = setup();
        sync.pass();
        assert!(fake.lock().unwrap().offered.is_empty(), "an empty board does not wipe the selection at start");
        clipboard::set_text("Settings search text");
        sync.pass();
        assert_eq!(pasteboard::text().as_deref(), Some("Settings search text"), "apps read it through the pasteboard");
        let f = fake.lock().unwrap();
        assert_eq!(f.offered.len(), 1);
        assert_eq!(mimes(&f.offered[0])[..2], ["text/plain;charset=utf-8", "text/plain"]);
        assert_eq!(f.offered[0][0].1.as_slice(), b"Settings search text");
        drop(f);
        sync.pass();
        assert_eq!(fake.lock().unwrap().offered.len(), 1, "nothing new: nothing offered again");
    }

    #[test]
    fn a_text_only_copy_from_an_app_replaces_rich_items() {
        let _fresh = fresh();
        pasteboard::set(vec![rep("public.png", "old")]);
        let (mut sync, fake) = setup();
        sync.pass();
        assert_eq!(mimes(&fake.lock().unwrap().offered[0]), ["image/png"]);
        // the shell handles an app's `SetClipboard` by putting the text on the shell clipboard
        clipboard::set_text("typed in Text Editor");
        sync.pass();
        let (_, items) = pasteboard::snapshot();
        assert_eq!(items, vec![rep("public.utf8-plain-text", "typed in Text Editor")]);
        assert_eq!(fake.lock().unwrap().offered.len(), 2);
    }

    #[test]
    fn a_typed_copy_from_an_app_is_offered_once_with_its_rich_types() {
        let _fresh = fresh();
        let (mut sync, fake) = setup();
        sync.pass();
        // `PasteboardSet`: the pasteboard first, then the text on the shell clipboard
        let text = pasteboard::set(vec![rep("public.utf8-plain-text", "Ada"), rep("public.html", "<i>Ada</i>")]);
        clipboard::set_text(&text.unwrap());
        sync.pass();
        sync.pass();
        let f = fake.lock().unwrap();
        assert_eq!(f.offered.len(), 1);
        assert!(mimes(&f.offered[0]).contains(&"text/html"));
        drop(f);
        assert_eq!(pasteboard::snapshot().1.len(), 2, "the rich items were not flattened to text");
    }

    #[test]
    fn a_selection_from_another_program_becomes_the_pasteboard_and_is_not_offered_back() {
        let _fresh = fresh();
        let (mut sync, fake) = setup();
        sync.pass();
        fake.lock().unwrap().arriving.push_back(vec![("text/plain;charset=utf-8".to_string(), "from wl-copy".as_bytes().to_vec()), ("text/html".to_string(), b"<p>from wl-copy</p>".to_vec())]);
        sync.pass();
        assert_eq!(clipboard::text().as_deref(), Some("from wl-copy"), "a shell field pastes it");
        assert_eq!(pasteboard::text().as_deref(), Some("from wl-copy"), "an app reads it from the pasteboard");
        assert_eq!(pasteboard::snapshot().1.len(), 2);
        sync.pass();
        assert!(fake.lock().unwrap().offered.is_empty(), "the other program keeps serving it");
    }

    #[test]
    fn an_empty_or_unusable_selection_changes_nothing() {
        let _fresh = fresh();
        clipboard::set_text("keep");
        let (mut sync, fake) = setup();
        sync.pass();
        fake.lock().unwrap().arriving.push_back(vec![("application/x-x".to_string(), vec![1])]);
        sync.pass();
        assert_eq!(clipboard::text().as_deref(), Some("keep"));
    }

    #[test]
    fn a_cleared_clipboard_clears_the_selection() {
        let _fresh = fresh();
        clipboard::set_text("x");
        let (mut sync, fake) = setup();
        sync.pass();
        clipboard::set_text("");
        sync.pass();
        let f = fake.lock().unwrap();
        assert_eq!(f.offered.len(), 2);
        assert!(f.offered[1].is_empty());
    }
}
