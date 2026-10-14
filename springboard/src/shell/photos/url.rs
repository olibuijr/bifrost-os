//! The `photos:` link scheme. `photos:photo/<absolute path>` opens a picture in the viewer and
//! `photos:album/<name>` opens an album: `all`, `recents`, `favorites`, `screenshots`, `deleted`, `hidden`,
//! or the name of an album, a smart album, a memory, a place or a keyword. Paths and names are percent-encoded.

use std::path::PathBuf;

/// What a link names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Photo(PathBuf),
    Album(String),
}

/// The scheme, as the app catalog registers it.
pub const SCHEME: &str = "photos";

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Decodes `%XX` escapes; None when an escape is cut short or the bytes are not UTF-8.
pub fn percent_decode(text: &str) -> Option<String> {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let (h, l) = (hex(*b.get(i + 1)?)?, hex(*b.get(i + 2)?)?);
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Encodes everything but letters, digits and `-._~/` as `%XX`.
pub fn percent_encode(text: &str) -> String {
    text.bytes().map(|c| if c.is_ascii_alphanumeric() || b"-._~/".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect()
}

/// The link a URL holds, or None when it is not a Photos link.
pub fn parse(url: &str) -> Option<Link> {
    let head = url.get(..SCHEME.len() + 1)?;
    if !head.get(..SCHEME.len())?.eq_ignore_ascii_case(SCHEME) || !head.ends_with(':') {
        return None;
    }
    let rest = url[SCHEME.len() + 1..].trim_start_matches("//");
    let (kind, value) = rest.split_once('/')?;
    match kind.to_ascii_lowercase().as_str() {
        "photo" => {
            let decoded = percent_decode(value)?;
            let path = if decoded.starts_with('/') { decoded } else { format!("/{decoded}") };
            (!path.contains("..")).then(|| Link::Photo(PathBuf::from(path)))
        }
        "album" => {
            let name = percent_decode(value)?;
            (!name.trim().is_empty()).then(|| Link::Album(name.trim().to_string()))
        }
        _ => None,
    }
}

/// The link that opens the picture at `path`.
pub fn photo_link(path: &std::path::Path) -> String {
    format!("{SCHEME}:photo/{}", percent_encode(&path.to_string_lossy()))
}

/// The link that opens the album called `name`.
pub fn album_link(name: &str) -> String {
    format!("{SCHEME}:album/{}", percent_encode(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_and_album_links_round_trip() {
        let p = std::path::Path::new("/home/me/Pictures/Trip 1/IMG 2.jpg");
        assert_eq!(parse(&photo_link(p)), Some(Link::Photo(p.to_path_buf())));
        assert_eq!(parse(&album_link("Summer & Sea")), Some(Link::Album("Summer & Sea".into())));
        assert_eq!(parse("PHOTOS://album/recents"), Some(Link::Album("recents".into())));
    }

    #[test]
    fn other_schemes_and_bad_links_are_not_photos_links() {
        assert_eq!(parse("https://example.org/photo/x"), None);
        assert_eq!(parse("photos:"), None);
        assert_eq!(parse("photos:album/"), None);
        assert_eq!(parse("photos:video/x"), None);
        assert_eq!(parse("photos:photo/%2Fhome%2F..%2Fetc%2Fpasswd"), None, "no parent steps");
        assert_eq!(parse("photos:album/%ZZ"), None);
    }

    #[test]
    fn escapes_decode_and_encode() {
        assert_eq!(percent_decode("a%20b%2Fc").as_deref(), Some("a b/c"));
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_encode("a b&c"), "a%20b%26c");
    }
}
