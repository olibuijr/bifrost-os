//! Links that open Photos. `photos://photo/<path>` shows a picture, `photos://album/<name>` opens an album and
//! `photos://search/<text>` runs a search; `photos:` alone just brings the app up. Text in a link is percent-encoded.
//! The shell routes the `photos` scheme here through the default-apps registry, like any other URL scheme.

use std::path::{Path, PathBuf};

/// What a link asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// open the app and leave it as it is
    App,
    /// show this picture
    Photo(PathBuf),
    /// open the album with this name
    Album(String),
    /// search for this text
    Search(String),
}

/// The link a URL stands for; None when it is not a Photos link or is malformed.
pub fn parse(url: &str) -> Option<Link> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.eq_ignore_ascii_case("photos") {
        return None;
    }
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    if rest.is_empty() {
        return Some(Link::App);
    }
    let (kind, tail) = rest.split_once('/').unwrap_or((rest, ""));
    let text = decode(tail)?;
    if text.is_empty() {
        return None;
    }
    match kind.to_ascii_lowercase().as_str() {
        "photo" => Some(Link::Photo(PathBuf::from(format!("/{}", text.trim_start_matches('/'))))),
        "album" => Some(Link::Album(text)),
        "search" => Some(Link::Search(text)),
        _ => None,
    }
}

/// The text of a percent-encoded link part; None on a broken escape or bytes that are not UTF-8.
pub fn decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `text` percent-encoded; `keep_slash` leaves `/` alone (for a path).
fn encode(text: &str, keep_slash: bool) -> String {
    let mut out = String::new();
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') || (keep_slash && b == b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The link that shows the picture at `path`.
pub fn photo_url(path: &Path) -> String {
    format!("photos://photo/{}", encode(path.to_string_lossy().trim_start_matches('/'), true))
}

/// The link that opens the album called `name`.
pub fn album_url(name: &str) -> String {
    format!("photos://album/{}", encode(name, false))
}

/// The link that searches for `text`.
pub fn search_url(text: &str) -> String {
    format!("photos://search/{}", encode(text, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_parse_to_what_they_ask_for() {
        assert_eq!(parse("photos:"), Some(Link::App));
        assert_eq!(parse("photos://"), Some(Link::App));
        assert_eq!(parse("PHOTOS://photo/home/me/Pictures/IMG_1.png"), Some(Link::Photo(PathBuf::from("/home/me/Pictures/IMG_1.png"))));
        assert_eq!(parse("photos://photo//home/me/a.png"), Some(Link::Photo(PathBuf::from("/home/me/a.png"))), "the path is always absolute");
        assert_eq!(parse("photos://album/Recently%20Deleted"), Some(Link::Album("Recently Deleted".to_string())));
        assert_eq!(parse("photos://search/%23beach%20is%3Afavorite"), Some(Link::Search("#beach is:favorite".to_string())));
        assert_eq!(parse("photos://Album/Trip"), Some(Link::Album("Trip".to_string())), "the kind is not case sensitive");
    }

    #[test]
    fn other_schemes_and_broken_links_are_refused() {
        for bad in ["", "https://photo/a.png", "mailto:me@example.com", "photos://nothing/a", "photos://photo/", "photos://album/%zz", "photos://album/%4", "photos://search/%ff", "no scheme"] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn built_links_read_back() {
        let path = Path::new("/home/me/Pictures/Sun set #2 (50%).png");
        let url = photo_url(path);
        assert!(url.starts_with("photos://photo/home/me/Pictures/") && !url.contains(' ') && !url.contains('#'), "{url}");
        assert_eq!(parse(&url), Some(Link::Photo(path.to_path_buf())));
        assert_eq!(parse(&album_url("Trip / 2026")), Some(Link::Album("Trip / 2026".to_string())));
        assert_eq!(parse(&search_url("rating:3 #sun_set")), Some(Link::Search("rating:3 #sun_set".to_string())));
        assert_eq!(decode("a%2Fb"), Some("a/b".to_string()));
        assert_eq!(decode("plain"), Some("plain".to_string()));
    }
}
