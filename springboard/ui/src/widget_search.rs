//! The search view of a Today widget (the reference SDK's default search view for a widget, deprecated there in favour
//! of the widget framework): the line that says what is searched, the text shown when there are no results, and the
//! results the widget's delegate returns, each with the string the list shows for it.
//!
//! The reference API names the string with a key path. Here the widget passes an accessor function instead, which is
//! the same contract checked at compile time. Pure model: the delegate runs the search and calls
//! [`WidgetSearch::set_results`]; the shell draws [`WidgetSearch::rows`] and, when that is empty,
//! [`WidgetSearch::placeholder`].

/// The search view's state for results of type `T`.
#[derive(Clone, Debug)]
pub struct WidgetSearch<T> {
    description: Option<String>,
    results_placeholder: Option<String>,
    /// The string a result shows in the list (the reference SDK's result key path).
    title_of: fn(&T) -> &str,
    results: Vec<T>,
}

impl<T> WidgetSearch<T> {
    /// A search view that shows each result with `title_of`, and has no description and no placeholder yet.
    pub fn new(title_of: fn(&T) -> &str) -> Self {
        WidgetSearch { description: None, results_placeholder: None, title_of, results: Vec::new() }
    }

    /// `searchDescription`: a localized line that says what the search covers. Blank text clears it.
    pub fn with_description(mut self, description: &str) -> Self {
        self.description = non_blank(description);
        self
    }

    /// `searchResultsPlaceholderString`: shown in the list when there are no results. Blank text clears it.
    pub fn with_placeholder(mut self, placeholder: &str) -> Self {
        self.results_placeholder = non_blank(placeholder);
        self
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn results_placeholder(&self) -> Option<&str> {
        self.results_placeholder.as_deref()
    }

    /// `searchResults`: replaces the results with the ones a new search returned, in their order.
    pub fn set_results(&mut self, results: Vec<T>) {
        self.results = results;
    }

    /// Removes every result, as when the search field is cleared.
    pub fn clear(&mut self) {
        self.results.clear();
    }

    pub fn results(&self) -> &[T] {
        &self.results
    }

    /// The string each result shows, in result order.
    pub fn rows(&self) -> Vec<&str> {
        self.results.iter().map(|r| (self.title_of)(r)).collect()
    }

    /// The placeholder to draw in place of the list: present only when there are no results and a placeholder is set.
    pub fn placeholder(&self) -> Option<&str> {
        if self.results.is_empty() {
            self.results_placeholder()
        } else {
            None
        }
    }
}

/// The text with its ends trimmed, or `None` when nothing is left.
fn non_blank(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: &String) -> &str {
        s.as_str()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn with_no_results_the_placeholder_is_drawn_in_place_of_the_list() {
        let s: WidgetSearch<String> = WidgetSearch::new(name).with_placeholder("No notes match");
        assert!(s.rows().is_empty());
        assert_eq!(s.placeholder(), Some("No notes match"));
    }

    #[test]
    fn results_replace_the_placeholder_and_keep_their_order() {
        let mut s = WidgetSearch::new(name).with_placeholder("Nothing");
        s.set_results(names(&["Groceries", "Groceries list", "Plan"]));
        assert_eq!(s.rows(), ["Groceries", "Groceries list", "Plan"]);
        assert_eq!(s.placeholder(), None);
    }

    #[test]
    fn a_new_search_replaces_the_old_results_and_clear_empties_them() {
        let mut s = WidgetSearch::new(name).with_placeholder("Nothing");
        s.set_results(names(&["a", "b"]));
        s.set_results(names(&["c"]));
        assert_eq!(s.rows(), ["c"]);
        s.clear();
        assert!(s.results().is_empty());
        assert_eq!(s.placeholder(), Some("Nothing"));
    }

    #[test]
    fn no_placeholder_is_drawn_when_none_was_set() {
        let mut s: WidgetSearch<String> = WidgetSearch::new(name);
        assert_eq!(s.placeholder(), None);
        s.set_results(names(&["x"]));
        assert_eq!(s.placeholder(), None);
    }

    #[test]
    fn description_and_placeholder_are_trimmed_and_blank_text_clears_them() {
        let s: WidgetSearch<String> = WidgetSearch::new(name).with_description("  Notes  ").with_placeholder("   ");
        assert_eq!(s.description(), Some("Notes"));
        assert_eq!(s.results_placeholder(), None);
        let s: WidgetSearch<String> = s.with_description("");
        assert_eq!(s.description(), None);
    }
}
