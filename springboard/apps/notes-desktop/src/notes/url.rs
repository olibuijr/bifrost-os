//! Links to a note or a folder: `notes://note/<id>` and `notes://folder/<name>`. The app declares the `notes` scheme in
//! its manifest, so opening such a link anywhere brings the note up. Names are percent-encoded in a link.

/// What a `notes:` link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Note(String),
    Folder(String),
}

/// The scheme the manifest declares.
pub const SCHEME: &str = "notes";

/// The link for a note.
pub fn note_url(id: &str) -> String {
    format!("{SCHEME}://note/{}", encode(id))
}

/// The link for a folder.
pub fn folder_url(name: &str) -> String {
    format!("{SCHEME}://folder/{}", encode(name))
}

/// Reads a `notes:` link; None for another scheme or a link that names nothing.
pub fn parse(url: &str) -> Option<Link> {
    let rest = url.trim().strip_prefix(SCHEME)?.strip_prefix(':')?;
    let rest = rest.trim_start_matches('/');
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let (kind, name) = rest.split_once('/')?;
    let name = decode(name.trim_end_matches('/'));
    if name.is_empty() {
        return None;
    }
    match kind {
        "note" => Some(Link::Note(name)),
        "folder" => Some(Link::Folder(name)),
        _ => None,
    }
}

fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn decode(text: &str) -> String {
    let raw = text.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && raw.get(i + 1..i + 3).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).unwrap_or("00");
            bytes.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip() {
        assert_eq!(parse(&note_url("1790000000123")), Some(Link::Note("1790000000123".into())));
        assert_eq!(parse(&folder_url("Work & Home")), Some(Link::Folder("Work & Home".into())));
        assert_eq!(folder_url("a b"), "notes://folder/a%20b");
    }

    #[test]
    fn other_schemes_and_empty_names_are_refused() {
        assert_eq!(parse("https://note/1"), None);
        assert_eq!(parse("notes://note/"), None);
        assert_eq!(parse("notes://other/1"), None);
        assert_eq!(parse("notes:note"), None);
        assert_eq!(parse("/home/u/a.md"), None);
    }

    #[test]
    fn a_link_may_lack_the_slashes_and_carry_a_query() {
        assert_eq!(parse("notes:note/42?x=1"), Some(Link::Note("42".into())));
        assert_eq!(parse("notes:///folder/Plans/"), Some(Link::Folder("Plans".into())));
    }
}
