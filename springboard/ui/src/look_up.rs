//! Look Up: the model behind the popover for a selected word or phrase. It holds the dictionary definition and the
//! quick search hits for the term, and lays them out as rows. A pure model; the shell fills it and draws the rows.

/// The longest term Look Up takes, in characters.
pub const TERM_MAX_CHARS: usize = 64;

/// How many quick search hits the popover lists.
pub const HITS_SHOWN: usize = 5;

/// The longest definition the popover shows, in characters; a longer one is cut and ends in an ellipsis.
pub const DEFINITION_MAX_CHARS: usize = 320;

/// The term to look up: the first line of `text`, trimmed and cut to [`TERM_MAX_CHARS`]. None when there is no visible
/// text or the first line has a control character.
pub fn term_of(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    if line.chars().any(char::is_control) {
        return None;
    }
    Some(line.chars().take(TERM_MAX_CHARS).collect())
}

/// One row of the popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// A section title: "Dictionary" or "Search".
    Heading(&'static str),
    /// Text in a section: a definition, a hit, or a note that nothing was found.
    Text(String),
    /// Opens Search with the term.
    Search { query: String },
    /// Opens the term in the Dictionary app.
    OpenInDictionary { word: String },
}

/// The popover's content for one term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookUp {
    pub term: String,
    /// The definition the dictionary returned, when one is installed for the term.
    pub definition: Option<String>,
    /// Quick search hits for the term, best first (titles).
    pub search_hits: Vec<String>,
}

impl LookUp {
    pub fn new(term: impl Into<String>, definition: Option<String>, search_hits: Vec<String>) -> Self {
        LookUp { term: term.into(), definition, search_hits }
    }

    /// The popover rows: the dictionary section, the search section, then the two actions (Search and Open in
    /// Dictionary).
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::Heading("Dictionary")];
        match self.definition.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(def) => rows.push(Row::Text(cut_definition(def))),
            None => rows.push(Row::Text("No definition in the installed dictionaries.".into())),
        }
        rows.push(Row::Heading("Search"));
        rows.extend(self.search_hits.iter().take(HITS_SHOWN).map(|h| Row::Text(h.clone())));
        rows.push(Row::Search { query: self.term.clone() });
        rows.push(Row::OpenInDictionary { word: self.term.clone() });
        rows
    }
}

/// `def` cut to [`DEFINITION_MAX_CHARS`] characters, with an ellipsis when cut.
fn cut_definition(def: &str) -> String {
    if def.chars().count() <= DEFINITION_MAX_CHARS {
        return def.to_string();
    }
    let mut cut: String = def.chars().take(DEFINITION_MAX_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_term_is_the_first_non_blank_line_trimmed() {
        assert_eq!(term_of("  Cat  \nsecond"), Some("Cat".into()));
        assert_eq!(term_of("\n\n  word\n"), Some("word".into()));
    }

    #[test]
    fn blank_or_control_text_has_no_term() {
        assert_eq!(term_of("   \n\t "), None);
        assert_eq!(term_of("a\u{7}b"), None);
    }

    #[test]
    fn a_long_term_is_cut_to_the_limit() {
        let long = "a".repeat(100);
        assert_eq!(term_of(&long).map(|t| t.chars().count()), Some(TERM_MAX_CHARS));
    }

    #[test]
    fn rows_with_a_definition_and_hits() {
        let look = LookUp::new("cat", Some(" a small domesticated feline ".into()), vec!["Cat photos".into(), "Catalogue".into()]);
        assert_eq!(
            look.rows(),
            vec![
                Row::Heading("Dictionary"),
                Row::Text("a small domesticated feline".into()),
                Row::Heading("Search"),
                Row::Text("Cat photos".into()),
                Row::Text("Catalogue".into()),
                Row::Search { query: "cat".into() },
                Row::OpenInDictionary { word: "cat".into() },
            ]
        );
    }

    #[test]
    fn no_definition_says_so_and_hits_are_capped() {
        let hits: Vec<String> = (0..9).map(|i| format!("hit {i}")).collect();
        let rows = LookUp::new("zzz", Some("   ".into()), hits).rows();
        assert_eq!(rows[1], Row::Text("No definition in the installed dictionaries.".into()));
        assert_eq!(rows.iter().filter(|r| matches!(r, Row::Text(t) if t.starts_with("hit"))).count(), HITS_SHOWN);
        assert_eq!(rows.last(), Some(&Row::OpenInDictionary { word: "zzz".into() }));
        assert_eq!(rows[rows.len() - 2], Row::Search { query: "zzz".into() });
    }

    #[test]
    fn a_long_definition_is_cut_with_an_ellipsis() {
        let long = "d".repeat(DEFINITION_MAX_CHARS + 50);
        let rows = LookUp::new("w", Some(long), vec![]).rows();
        let Row::Text(text) = &rows[1] else { panic!("definition row") };
        assert_eq!(text.chars().count(), DEFINITION_MAX_CHARS + 1);
        assert!(text.ends_with('…'));
    }
}
