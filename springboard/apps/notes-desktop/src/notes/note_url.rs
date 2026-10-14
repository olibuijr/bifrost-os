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
