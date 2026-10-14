//! List markers for imported rich text. A list item of an RTF file is kept as a typed marker and a space before the
//! item text, in the default style, so the note reads the same as the list did.

use crate::model::Style;

/// Inserts each list marker before the character it is paired with (its index in the text), then merges the characters
/// back into runs of equal style. Markers are in text order, as `AttributedString::list_markers` returns them.
pub fn with_list_markers(runs: Vec<(String, Style)>, markers: &[(usize, String)]) -> Vec<(String, Style)> {
    let chars: Vec<(char, Style)> = runs.iter().flat_map(|(text, style)| text.chars().map(move |c| (c, *style))).collect();
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut pending = markers.iter().peekable();
    for (index, (c, style)) in chars.into_iter().enumerate() {
        while let Some((_, marker)) = pending.next_if(|(at, _)| *at == index) {
            for m in marker.chars().chain(std::iter::once(' ')) {
                push(&mut out, m, Style::default());
            }
        }
        push(&mut out, c, style);
    }
    out
}

fn push(out: &mut Vec<(String, Style)>, c: char, style: Style) {
    match out.last_mut() {
        Some((text, last)) if *last == style => text.push(c),
        _ => out.push((c.to_string(), style)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Style {
        Style { bold: true, ..Style::default() }
    }

    #[test]
    fn a_list_marker_is_inserted_in_the_default_style_before_its_item() {
        let runs = vec![("one".to_string(), bold()), ("two".to_string(), Style::default())];
        let out = with_list_markers(runs, &[(0, "•".to_string())]);
        assert_eq!(out, vec![("• ".to_string(), Style::default()), ("one".to_string(), bold()), ("two".to_string(), Style::default())]);
    }

    #[test]
    fn a_marker_in_the_middle_of_a_run_splits_it() {
        let runs = vec![("ab".to_string(), bold())];
        let out = with_list_markers(runs, &[(1, "2.".to_string())]);
        assert_eq!(out, vec![("a".to_string(), bold()), ("2. ".to_string(), Style::default()), ("b".to_string(), bold())]);
    }

    #[test]
    fn text_without_markers_keeps_its_runs_merged() {
        let runs = vec![("a".to_string(), bold()), ("b".to_string(), bold()), ("c".to_string(), Style::default())];
        let out = with_list_markers(runs, &[]);
        assert_eq!(out, vec![("ab".to_string(), bold()), ("c".to_string(), Style::default())]);
    }

    #[test]
    fn consecutive_markers_each_get_their_own_prefix() {
        let runs = vec![("xy".to_string(), Style::default())];
        let out = with_list_markers(runs, &[(0, "1.".to_string()), (1, "2.".to_string())]);
        assert_eq!(out, vec![("1. x2. y".to_string(), Style::default())], "each item starts with its own marker");
    }
}
