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

/// Every note id that `text` links to, in the order the links appear and without repeats. The id after a link is the
/// run of letters, digits and dashes that follows the prefix, so `[title](notes://note/abc)` links to `abc`. Text
/// with no note link gives an empty list.
pub fn linked_ids(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(PREFIX) {
        let body = &rest[at + PREFIX.len()..];
        let end = body.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(body.len());
        let id = &body[..end];
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
        rest = &body[end..];
    }
    out
}

/// The ids of the notes whose text links to `target`, in the order given. A note never lists itself.
/// The answer comes from the shared document link index (`bifrost_core::link_index`), filled with each note's links.
pub fn backlinks<'a>(notes: &[(&'a str, &str)], target: &str) -> Vec<&'a str> {
    let mut index = bifrost_core::link_index::LinkIndex::new();
    for (id, body) in notes {
        index.set_links(*id, linked_ids(body));
    }
    let linking = index.backlinks(target);
    notes.iter().filter(|(id, _)| linking.contains(id)).map(|(id, _)| *id).collect()
}

/// The line a linked-notes panel shows above its list: how many notes link to the open one.
pub fn backlink_label(count: usize) -> String {
    match count {
        0 => "No linked notes".to_string(),
        1 => "1 linked note".to_string(),
        n => format!("{n} linked notes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linked_notes_label_counts_the_notes() {
        assert_eq!(backlink_label(0), "No linked notes");
        assert_eq!(backlink_label(1), "1 linked note");
        assert_eq!(backlink_label(5), "5 linked notes");
    }

    #[test]
    fn the_label_counts_the_backlinks_of_a_note() {
        let notes = [("a", "notes://note/t"), ("b", "nothing"), ("c", "notes://note/t")];
        assert_eq!(backlink_label(backlinks(&notes, "t").len()), "2 linked notes");
        assert_eq!(backlink_label(backlinks(&notes, "b").len()), "No linked notes");
    }

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

    #[test]
    fn linked_ids_finds_each_link_once_in_order() {
        let text = "see notes://note/abc and [again](notes://note/abc), then notes://note/x-9.";
        assert_eq!(linked_ids(text), vec!["abc", "x-9"]);
    }

    #[test]
    fn text_without_a_note_link_has_no_ids() {
        assert!(linked_ids("plain text, https://example.com/abc").is_empty());
        assert!(linked_ids("notes://note/").is_empty());
        assert!(linked_ids("notes://folder/abc").is_empty());
    }

    #[test]
    fn backlinks_lists_the_notes_that_link_to_the_target_without_itself() {
        let notes = [
            ("a", "links to notes://note/t"),
            ("t", "links to itself notes://note/t"),
            ("b", "nothing here"),
            ("c", "[t](notes://note/t) and more"),
        ];
        assert_eq!(backlinks(&notes, "t"), vec!["a", "c"]);
        assert!(backlinks(&notes, "b").is_empty());
    }
}
