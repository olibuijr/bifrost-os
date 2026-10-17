//! Links to a note: `notes://note/<id>` names one note by its id. The model builds such a link and reads one back, so
//! a link can be copied from a note and opened later. Only the id is taken from a link; the scheme and the path are
//! fixed, and anything else is refused.

/// The scheme and the path that starts a note link.
pub const PREFIX: &str = "notes://note/";

/// The link that opens note `id`.
pub fn note_url(id: &str) -> String {
    format!("{PREFIX}{id}")
}

/// The note id a link names, or None when the text is not a note link. An id is one or more ASCII letters, digits or
/// dashes; a query, a fragment or a further path segment is not part of it and makes the link invalid.
pub fn note_id(url: &str) -> Option<&str> {
    let id = url.trim().strip_prefix(PREFIX)?;
    let valid = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    valid.then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_built_link_reads_back_to_its_note_id() {
        let link = note_url("8f2c-01");
        assert_eq!(link, "notes://note/8f2c-01");
        assert_eq!(note_id(&link), Some("8f2c-01"));
    }

    #[test]
    fn surrounding_space_is_trimmed() {
        assert_eq!(note_id("  notes://note/abc  "), Some("abc"));
    }

    #[test]
    fn other_schemes_paths_and_empty_ids_are_refused() {
        assert_eq!(note_id("https://example.com/notes://note/abc"), None);
        assert_eq!(note_id("notes://note/"), None);
        assert_eq!(note_id("notes://note/abc/def"), None, "a further path segment is not part of the id");
        assert_eq!(note_id("notes://note/abc?x=1"), None, "a query is not part of the id");
        assert_eq!(note_id("notes://note/ab c"), None);
        assert_eq!(note_id("notes://folder/abc"), None);
    }
}
