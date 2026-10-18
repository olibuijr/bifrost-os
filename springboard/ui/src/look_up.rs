//! Look Up: the model behind the popover for a selected word or phrase. It holds the dictionary definition and the
//! quick search hits for the term, and the suggested actions for the items the selection holds (links, dates, email
//! addresses; `suggested_actions`), and lays them out as rows. A pure model; the shell fills it and draws the rows.

use crate::suggested_actions::{Offer, Route};

/// The longest term Look Up takes, in characters.
pub const TERM_MAX_CHARS: usize = 64;

/// How many quick search hits the popover lists.
pub const HITS_SHOWN: usize = 5;

/// How many suggested actions the popover lists.
pub const SUGGESTIONS_SHOWN: usize = 4;

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
    /// A suggested action for an item of the selection: the action's title, the item it acts on, and what it does.
    Suggest { title: &'static str, item: String, route: Route },
}

/// The popover's content for one term.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LookUp {
    pub term: String,
    /// The definition the dictionary returned, when one is installed for the term.
    pub definition: Option<String>,
    /// Quick search hits for the term, best first (titles).
    pub search_hits: Vec<String>,
    /// Actions for the items the selection holds, in reading order (`suggested_actions::offers`).
    pub suggestions: Vec<Offer>,
}

impl LookUp {
    pub fn new(term: impl Into<String>, definition: Option<String>, search_hits: Vec<String>) -> Self {
        LookUp { term: term.into(), definition, search_hits, suggestions: Vec::new() }
    }

    /// The same content with suggested actions for the selection's items.
    pub fn with_suggestions(mut self, suggestions: Vec<Offer>) -> Self {
        self.suggestions = suggestions;
        self
    }

    /// The popover rows: the dictionary section, the search section, then the two actions (Search and Open in
    /// Dictionary), then the suggested actions when the selection holds items that have some.
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
        if !self.suggestions.is_empty() {
            rows.push(Row::Heading("Suggested Actions"));
            rows.extend(self.suggestions.iter().take(SUGGESTIONS_SHOWN).map(|o| Row::Suggest { title: o.title, item: o.item.clone(), route: o.route.clone() }));
        }
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
    fn suggested_actions_follow_the_two_actions_and_are_capped() {
        use crate::suggested_actions::{offers, ALL_KINDS};
        let text = "https://a.is https://b.is https://c.is";
        let look = LookUp::new("a", None, vec![]).with_suggestions(offers(text, &ALL_KINDS, 8));
        let rows = look.rows();
        let at = rows.iter().position(|r| *r == Row::Heading("Suggested Actions")).expect("a suggestions section");
        assert_eq!(rows[at - 1], Row::OpenInDictionary { word: "a".into() });
        let listed: Vec<&Row> = rows[at + 1..].iter().collect();
        assert_eq!(listed.len(), SUGGESTIONS_SHOWN);
        assert!(matches!(listed[0], Row::Suggest { title: "Open Link", route: Route::OpenUrl(u), .. } if u == "https://a.is"));
    }

    #[test]
    fn no_items_add_no_section() {
        let rows = LookUp::new("cat", None, vec![]).with_suggestions(vec![]).rows();
        assert!(!rows.contains(&Row::Heading("Suggested Actions")));
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
