//! Rich pasteboard (`NativePasteboard`) on the shell side: items keyed by UTType id, mapped to MIME types for the Wayland
//! clipboard. The plain-text clipboard path (`clipboard::set_text`) stays the carrier for text inside the shell;
//! `pasteboard_sync` keeps the two and the window server's selection one clipboard.

use std::sync::Mutex;

/// The change count, the representations of every item in order, and how many representations each item holds (the
/// lengths add up to the number of representations). A flat write makes every representation its own item.
type State = (u32, Vec<(String, Vec<u8>)>, Vec<usize>);

static STATE: Mutex<State> = Mutex::new((0, Vec::new(), Vec::new()));

/// How long the contents are meant to live. A write that carries the `ConcealedType` marker holds a secret (a password): it
/// is kept in memory only, goes to the app in front but not to the apps in the background, reaches other programs only
/// with the password-manager hint, and is cleared after [`bifrost_proto::PB_CONCEALED_SECS`] when nothing else was copied.
/// A `TransientType` write is also kept in memory only and flagged for other programs, but stays until replaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Normal,
    Transient,
    Concealed,
}

impl Kind {
    /// Whether the contents must stay off the disk and out of the apps in the background.
    pub fn is_private(self) -> bool {
        self != Kind::Normal
    }
}

/// The kind of the contents written at change count `.1`, and when (monotonic ms). A later write bumps the count, so a
/// mark of an earlier count says nothing about what is on the board now.
static MARK: Mutex<(Kind, u32, f64)> = Mutex::new((Kind::Normal, 0, 0.0));

/// The kind an app's write asks for: concealed beats transient beats normal.
pub fn kind_of_items(items: &[(String, Vec<u8>)]) -> Kind {
    if items.iter().any(|(t, _)| t == bifrost_proto::PB_CONCEALED) {
        Kind::Concealed
    } else if items.iter().any(|(t, _)| t == bifrost_proto::PB_TRANSIENT) {
        Kind::Transient
    } else {
        Kind::Normal
    }
}

/// `items` without the marker types (they describe the write; no app or program is given them as data).
pub fn without_markers(items: Vec<(String, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
    items.into_iter().filter(|(t, _)| !bifrost_proto::is_pb_marker(t)).collect()
}

/// Like [`set`] for an app's write: the marker types are taken off the items and decide the board's [`Kind`] from `now`
/// (monotonic ms). Returns the text to publish on the plain clipboard and the kind.
pub fn set_marked(items: Vec<(String, Vec<u8>)>, now: f64) -> (Option<String>, Kind) {
    let kind = kind_of_items(&items);
    let text = set(without_markers(items));
    mark(kind, now);
    (text, kind)
}

/// Records that the contents now on the board are of `kind`, written at `now` (monotonic ms).
pub fn mark(kind: Kind, now: f64) {
    let count = count();
    *MARK.lock().unwrap_or_else(|e| e.into_inner()) = (kind, count, now);
}

/// The kind of the contents on the board now.
pub fn kind() -> Kind {
    let (kind, at, _) = *MARK.lock().unwrap_or_else(|e| e.into_inner());
    if at == count() {
        kind
    } else {
        Kind::Normal
    }
}

/// Whether concealed contents have been on the board for their lifetime at `now` (monotonic ms) and are still the latest
/// write.
pub fn expired(now: f64) -> bool {
    let (kind, at, since) = *MARK.lock().unwrap_or_else(|e| e.into_inner());
    kind == Kind::Concealed && at == count() && now - since >= bifrost_proto::PB_CONCEALED_SECS as f64 * 1000.0
}

/// When (monotonic ms) the concealed contents on the board are due to be cleared, `None` when there are none (a transient copy
/// has no deadline). The shell's idle loop wakes then, so the clear is not left to the next unrelated wake-up.
pub fn expiry_deadline() -> Option<f64> {
    let (kind, at, since) = *MARK.lock().unwrap_or_else(|e| e.into_inner());
    (kind == Kind::Concealed && at == count()).then(|| since + bifrost_proto::PB_CONCEALED_SECS as f64 * 1000.0)
}

/// The state is process-wide, so the tests that write and read it take turns.
#[cfg(test)]
pub(crate) static TURN: Mutex<()> = Mutex::new(());

/// The plain-text type.
pub const PLAIN_TEXT: &str = "public.utf8-plain-text";

/// UTType id to MIME type (`public.utf8-plain-text` to `text/plain;charset=utf-8`, and so on).
pub fn mime_for(uti: &str) -> &str {
    match uti {
        "public.utf8-plain-text" | "public.plain-text" => "text/plain;charset=utf-8",
        "public.html" => "text/html",
        "public.rtf" => "text/rtf",
        "public.png" => "image/png",
        "public.jpeg" => "image/jpeg",
        "public.file-url" | "public.url" => "text/uri-list",
        other => other,
    }
}

/// The text to publish on the plain clipboard: the plain-text item, else the file URLs, else the HTML source.
pub fn text_of(items: &[(String, Vec<u8>)]) -> Option<String> {
    ["public.utf8-plain-text", "public.file-url", "public.html"]
        .iter()
        .find_map(|t| items.iter().find(|(k, _)| k == t))
        .and_then(|(_, d)| String::from_utf8(d.clone()).ok())
}

/// Replaces the contents (`clearContents` + `setData`), bumps the change count, and returns the text to publish on the
/// plain clipboard: the plain-text item, else the file URLs, else the HTML source. Every entry is its own item; use
/// [`set_items`] to keep the representations of one copy together.
pub fn set(items: Vec<(String, Vec<u8>)>) -> Option<String> {
    let text = text_of(&items);
    let lens = vec![1; items.len()];
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.0 = s.0.wrapping_add(1);
    s.1 = items;
    s.2 = lens;
    text
}

/// Like [`set`], but each inner list is one item holding several representations of the same copy (a name, a link and
/// a picture of one thing), so a reader sees the same grouping the writer sent. An empty inner list is dropped.
pub fn set_items(items: Vec<Vec<(String, Vec<u8>)>>) -> Option<String> {
    let items: Vec<Vec<(String, Vec<u8>)>> = items.into_iter().filter(|i| !i.is_empty()).collect();
    let lens: Vec<usize> = items.iter().map(Vec::len).collect();
    let flat: Vec<(String, Vec<u8>)> = items.into_iter().flatten().collect();
    let text = text_of(&flat);
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.0 = s.0.wrapping_add(1);
    s.1 = flat;
    s.2 = lens;
    text
}

/// The change count alone, without copying the items (a polling reader checks this first).
pub fn count() -> u32 {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).0
}

/// The text on the board right now (see [`text_of`]).
pub fn text() -> Option<String> {
    text_of(&STATE.lock().unwrap_or_else(|e| e.into_inner()).1)
}

/// Replaces the board with one plain-text item (an empty `text` empties it) and bumps the change count: what the shell
/// does for a copy made in one of its own fields, or by an app that sent only text.
pub fn set_plain(text: &str) -> u32 {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.0 = s.0.wrapping_add(1);
    if text.is_empty() {
        s.1.clear();
        s.2.clear();
    } else {
        s.1 = vec![(PLAIN_TEXT.to_string(), text.as_bytes().to_vec())];
        s.2 = vec![1];
    }
    s.0
}

/// The change count and items now on the pasteboard.
pub fn snapshot() -> (u32, Vec<(String, Vec<u8>)>) {
    let s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    (s.0, s.1.clone())
}

/// The change count and the items now on the pasteboard, each with its representations together as written.
pub fn snapshot_items() -> (u32, Vec<Vec<(String, Vec<u8>)>>) {
    let s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let mut rest = s.1.iter();
    let items = s.2.iter().map(|n| rest.by_ref().take(*n).cloned().collect()).collect();
    (s.0, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::TURN;

    #[test]
    fn markers_decide_the_kind_and_are_not_data() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let w = |extra: &[&str]| -> Vec<(String, Vec<u8>)> {
            let mut v = vec![("public.utf8-plain-text".to_string(), b"hunter2".to_vec())];
            v.extend(extra.iter().map(|t| (t.to_string(), Vec::new())));
            v
        };
        assert_eq!(kind_of_items(&w(&[])), Kind::Normal);
        assert_eq!(kind_of_items(&w(&[bifrost_proto::PB_TRANSIENT])), Kind::Transient);
        assert_eq!(kind_of_items(&w(&[bifrost_proto::PB_TRANSIENT, bifrost_proto::PB_CONCEALED])), Kind::Concealed);
        let (text, kind_now) = set_marked(w(&[bifrost_proto::PB_CONCEALED]), 1000.0);
        assert_eq!(text.as_deref(), Some("hunter2"));
        assert_eq!(kind_now, Kind::Concealed);
        assert_eq!(snapshot().1.len(), 1, "the marker is not an item");
        assert_eq!(kind(), Kind::Concealed);
        assert!(kind().is_private());
    }

    #[test]
    fn a_concealed_copy_expires_unless_something_else_was_copied() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let secs = bifrost_proto::PB_CONCEALED_SECS as f64 * 1000.0;
        set_marked(vec![("public.utf8-plain-text".into(), b"pw".to_vec()), (bifrost_proto::PB_CONCEALED.into(), Vec::new())], 5000.0);
        assert!(!expired(5000.0 + secs - 1.0));
        assert!(expired(5000.0 + secs));
        assert_eq!(expiry_deadline(), Some(5000.0 + secs), "the idle loop wakes for it");
        // a newer write of any kind ends the mark
        set_plain("later");
        assert!(!expired(5000.0 + secs * 2.0));
        assert_eq!(expiry_deadline(), None, "nothing to clear once something else was copied");
        assert_eq!(kind(), Kind::Normal);
        // a transient copy never expires on a timer
        set_marked(vec![("public.utf8-plain-text".into(), b"t".to_vec()), (bifrost_proto::PB_TRANSIENT.into(), Vec::new())], 0.0);
        assert!(!expired(1e12));
        assert_eq!(kind(), Kind::Transient);
    }

    #[test]
    fn maps_and_counts() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(mime_for("public.png"), "image/png");
        let t = set(vec![("public.html".into(), b"<i>x</i>".to_vec())]);
        assert_eq!(t.as_deref(), Some("<i>x</i>"));
        assert!(snapshot().0 >= 1);
    }

    fn rep(uti: &str, data: &str) -> (String, Vec<u8>) {
        (uti.to_string(), data.as_bytes().to_vec())
    }

    #[test]
    fn a_plain_write_is_one_text_item_and_an_empty_one_clears_the_board() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        set(vec![rep("public.png", "x"), rep("public.html", "<i>y</i>")]);
        let before = count();
        assert_eq!(set_plain("hello there"), before.wrapping_add(1));
        assert_eq!(snapshot_items().1, vec![vec![rep(PLAIN_TEXT, "hello there")]]);
        assert_eq!(text().as_deref(), Some("hello there"));
        set_plain("");
        assert!(snapshot().1.is_empty() && snapshot_items().1.is_empty());
        assert_eq!(text(), None);
        assert_eq!(count(), snapshot().0);
    }

    #[test]
    fn text_comes_from_the_plain_item_then_file_urls_then_html() {
        assert_eq!(text_of(&[rep("public.html", "<b>"), rep("public.file-url", "file:///a"), rep(PLAIN_TEXT, "t")]).as_deref(), Some("t"));
        assert_eq!(text_of(&[rep("public.html", "<b>"), rep("public.file-url", "file:///a")]).as_deref(), Some("file:///a"));
        assert_eq!(text_of(&[rep("public.html", "<b>")]).as_deref(), Some("<b>"));
        assert_eq!(text_of(&[rep("public.png", "x")]), None);
    }

    #[test]
    fn items_keep_their_representations_together_and_in_order() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let text = set_items(vec![
            vec![rep("public.utf8-plain-text", "Ada"), rep("public.url", "https://ada.is")],
            Vec::new(),
            vec![rep("public.utf8-plain-text", "Bob")],
        ]);
        assert_eq!(text.as_deref(), Some("Ada"));
        let (count, items) = snapshot_items();
        assert!(count >= 1);
        assert_eq!(items.len(), 2, "an empty item is dropped");
        assert_eq!(items[0].len(), 2);
        assert_eq!(items[1], vec![rep("public.utf8-plain-text", "Bob")]);
        // the flat view still lists every representation, for readers that do not group
        assert_eq!(snapshot().1.len(), 3);
    }

    #[test]
    fn a_flat_write_makes_each_entry_its_own_item() {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        set(vec![rep("public.png", "x"), rep("public.html", "<i>y</i>")]);
        let (_, items) = snapshot_items();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| i.len() == 1));
    }
}
