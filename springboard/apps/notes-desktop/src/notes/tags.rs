//! Inline `#tags` in note text: the tags of a note, the tag browser's counts, and the filter that picks the notes
//! carrying every selected tag. Matching ignores case and keeps the first spelling. Pure functions over `Note`.
#![allow(dead_code)] // the sidebar Tags section and the filter chips wire these in next

use super::store::Note;

/// The tags in `text`, each once, in order. A tag is a `#` at the start of a line or after a space or punctuation
/// (so `C#` and `a#b` are not tags), then a letter, then letters, digits, `_` or `-` (a trailing `-` is dropped).
/// `#` followed by a space or digit is a heading or a number, not a tag.
pub fn tags_in(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = vec![];
    for i in 0..chars.len() {
        if chars[i] != '#' {
            continue;
        }
        if i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '#' || chars[i - 1] == '&') {
            continue;
        }
        if !chars.get(i + 1).is_some_and(|c| c.is_alphabetic()) {
            continue;
        }
        let word: String = chars[i + 1..]
            .iter()
            .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '-')
            .collect();
        let word = word.trim_end_matches('-').to_string();
        if out.iter().all(|t| !t.eq_ignore_ascii_case(&word) && t.to_lowercase() != word.to_lowercase()) {
            out.push(word);
        }
    }
    out
}

/// Every tag across `notes` with the number of notes carrying it, sorted by name without case.
pub fn tag_counts(notes: &[Note]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = vec![];
    for note in notes {
        for tag in tags_in(&note.text) {
            match counts.iter_mut().find(|(t, _)| t.to_lowercase() == tag.to_lowercase()) {
                Some((_, n)) => *n += 1,
                None => counts.push((tag, 1)),
            }
        }
    }
    counts.sort_by_key(|(t, _)| t.to_lowercase());
    counts
}

/// The notes (by index) that carry every tag in `selected`. No selection picks every note.
pub fn filter(notes: &[Note], selected: &[String]) -> Vec<usize> {
    notes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            let have: Vec<String> = tags_in(&n.text).iter().map(|t| t.to_lowercase()).collect();
            selected.iter().all(|s| have.contains(&s.trim_start_matches('#').to_lowercase()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// The tags that can still narrow a selection: those on at least one note that carries all of `selected`, other
/// than the selected ones, with their counts inside that narrowed set.
pub fn narrowing(notes: &[Note], selected: &[String]) -> Vec<(String, usize)> {
    let picked: Vec<Note> = filter(notes, selected).into_iter().map(|i| notes[i].clone()).collect();
    tag_counts(&picked)
        .into_iter()
        .filter(|(t, _)| !selected.iter().any(|s| s.trim_start_matches('#').eq_ignore_ascii_case(t)))
        .collect()
}

/// `text` with the tag `from` renamed to `to` everywhere it appears as a tag (other text is untouched).
pub fn rename(text: &str, from: &str, to: &str) -> String {
    let from = from.trim_start_matches('#').to_lowercase();
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '#' || chars[i - 1] == '&')) {
            let word: String = chars[i + 1..]
                .iter()
                .take_while(|c| c.is_alphanumeric() || **c == '_' || **c == '-')
                .collect();
            let bare = word.trim_end_matches('-');
            if chars.get(i + 1).is_some_and(|c| c.is_alphabetic()) && bare.to_lowercase() == from {
                out.push('#');
                out.push_str(to.trim_start_matches('#'));
                i += 1 + bare.chars().count();
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, text: &str) -> Note {
        Note { id: id.into(), text: text.into(), modified: 0 }
    }

    #[test]
    fn finds_tags_and_skips_headings_and_sharps() {
        assert_eq!(tags_in("Plan #home and #Work-stuff, not C# or a#b or # heading or #2024"), vec!["home", "Work-stuff"]);
        assert_eq!(tags_in("#first\nsecond #FIRST"), vec!["first"]);
        assert_eq!(tags_in("trailing #dash- here"), vec!["dash"]);
        assert!(tags_in("## Heading").is_empty());
    }

    #[test]
    fn counts_sorted_without_case() {
        let notes = vec![note("1", "#beta #Alpha"), note("2", "#alpha"), note("3", "none")];
        assert_eq!(tag_counts(&notes), vec![("Alpha".to_string(), 2), ("beta".to_string(), 1)]);
    }

    #[test]
    fn filter_needs_every_selected_tag() {
        let notes = vec![note("1", "#a #b"), note("2", "#a"), note("3", "#b")];
        assert_eq!(filter(&notes, &[]), vec![0, 1, 2]);
        assert_eq!(filter(&notes, &["a".to_string()]), vec![0, 1]);
        assert_eq!(filter(&notes, &["A".to_string(), "#b".to_string()]), vec![0]);
        assert!(filter(&notes, &["zzz".to_string()]).is_empty());
    }

    #[test]
    fn narrowing_lists_remaining_tags() {
        let notes = vec![note("1", "#a #b"), note("2", "#a #c"), note("3", "#d")];
        assert_eq!(narrowing(&notes, &["a".to_string()]), vec![("b".to_string(), 1), ("c".to_string(), 1)]);
    }

    #[test]
    fn rename_touches_only_tags() {
        assert_eq!(rename("a #Old b old #older", "old", "new"), "a #new b old #older");
        assert_eq!(rename("#old", "#old", "#fresh"), "#fresh");
    }
}
