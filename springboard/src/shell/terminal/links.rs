//! Links in the terminal: the first web link in the selection, opened from the More menu.
use super::profile_table::detect_urls;

/// The first link in `text` (one or more screen rows), in reading order.
pub fn first_link(text: &str) -> Option<String> {
    text.lines().flat_map(detect_urls).map(|(_, _, url)| url).next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_link_is_taken_from_selected_rows() {
        assert_eq!(first_link("read  https://a.io/x.\nmore").as_deref(), Some("https://a.io/x"));
        assert_eq!(first_link("second\nhttp://b.dev/p, then").as_deref(), Some("http://b.dev/p"));
        assert_eq!(first_link("no link here"), None);
        assert_eq!(first_link(""), None);
    }
}
