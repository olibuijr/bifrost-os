//! Rich pasteboard (`NativePasteboard`) on the shell side: items keyed by UTType id, mapped to MIME types for the Wayland
//! clipboard. The plain-text clipboard path (`clipboard::set_text`) stays the carrier for text inside the shell;
//! `pasteboard_sync` keeps the two and the window server's selection one clipboard.

use std::sync::Mutex;

/// The change count, the representations of every item in order, and how many representations each item holds (the
/// lengths add up to the number of representations). A flat write makes every representation its own item.
type State = (u32, Vec<(String, Vec<u8>)>, Vec<usize>);

static STATE: Mutex<State> = Mutex::new((0, Vec::new(), Vec::new()));

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
