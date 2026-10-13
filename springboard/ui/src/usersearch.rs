//! User-typed queries (`CSUserQuery`, `CSUserQueryContext`, `CSSuggestion`): ranked items for what the user typed,
//! plus completions for the word being typed.
//!
//! Items come from [`crate::searchquery::run`], so they match and rank exactly as a `CSSearchQuery` does. A suggestion
//! is a word from the records' titles and keywords that starts with the last word typed and is longer than it. The
//! suggestions that appear in more records come first, then the shorter ones, then alphabetical order.
//!
//! Pure model. Gaps: suggestions are plain words (no attributed text), the kind is always `Default`, and results
//! are returned at once rather than streamed.

use crate::searchquery::{run, Error, Hit, Query, Record};
use std::collections::HashMap;

/// `CSSuggestion.Kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    None,
    Custom,
    Default,
}

/// `CSSuggestion`: a completion of the text typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// `localizedSuggestion`: the whole query with its last word completed.
    pub localized_suggestion: String,
    pub kind: SuggestionKind,
}

/// `CSUserQueryContext`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserQueryContext {
    /// `maxResultCount`
    pub max_result_count: usize,
    /// `maxSuggestionCount`
    pub max_suggestion_count: usize,
    /// `fetchAttributes`
    pub fetch_attributes: Vec<String>,
}

impl Default for UserQueryContext {
    fn default() -> Self {
        UserQueryContext { max_result_count: 20, max_suggestion_count: 5, fetch_attributes: Vec::new() }
    }
}

/// What a user query found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserQueryResults {
    /// Matching records, best first, at most `max_result_count`.
    pub items: Vec<Hit>,
    /// Completions, at most `max_suggestion_count`.
    pub suggestions: Vec<Suggestion>,
}

fn lower_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

/// `CSUserQuery(userQueryString:userQueryContext:)` and its run over `records`.
pub fn run_user_query(user_query_string: &str, context: &UserQueryContext, records: &[Record]) -> Result<UserQueryResults, Error> {
    let mut items = run(&Query::new(user_query_string), records)?;
    items.truncate(context.max_result_count);
    Ok(UserQueryResults { items, suggestions: suggest(user_query_string, context.max_suggestion_count, records) })
}

fn suggest(typed: &str, limit: usize, records: &[Record]) -> Vec<Suggestion> {
    let Some(last) = lower_words(typed).pop() else { return Vec::new() };
    // A trailing space or quote means the last word is finished, so there is nothing to complete.
    if typed.ends_with(|c: char| !c.is_alphanumeric()) {
        return Vec::new();
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in records {
        let mut seen: Vec<String> = Vec::new();
        for w in lower_words(&r.title).into_iter().chain(r.keywords.iter().flat_map(|k| lower_words(k))) {
            if w.len() > last.len() && w.starts_with(&last) && !seen.contains(&w) {
                seen.push(w);
            }
        }
        for w in seen {
            *counts.entry(w).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.len().cmp(&b.0.len())).then(a.0.cmp(&b.0)));
    let head = typed.trim_end_matches(|c: char| c.is_alphanumeric());
    ranked.into_iter().take(limit).map(|(w, _)| Suggestion { localized_suggestion: format!("{head}{w}"), kind: SuggestionKind::Default }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(title: &str, keywords: &[&str]) -> Record {
        Record { title: title.into(), description: String::new(), keywords: keywords.iter().map(|s| s.to_string()).collect() }
    }
    fn texts(r: &UserQueryResults) -> Vec<&str> {
        r.suggestions.iter().map(|s| s.localized_suggestion.as_str()).collect()
    }

    #[test]
    fn returns_ranked_items_and_completions() {
        let records = [rec("Milk", &[]), rec("Milkshake recipe", &[]), rec("Mild salsa", &["milk"])];
        let out = run_user_query("mil", &UserQueryContext::default(), &records).unwrap();
        assert_eq!(out.items.len(), 3);
        assert_eq!(texts(&out), ["milk", "mild", "milkshake"]);
        assert!(out.suggestions.iter().all(|s| s.kind == SuggestionKind::Default));
    }

    #[test]
    fn a_completion_keeps_the_words_typed_before_it() {
        let records = [rec("Oat milk", &[])];
        let out = run_user_query("oat mi", &UserQueryContext::default(), &records).unwrap();
        assert_eq!(texts(&out), ["oat milk"]);
    }

    #[test]
    fn limits_apply_and_a_finished_word_has_no_completions() {
        let records = [rec("Milk", &[]), rec("Mild", &[]), rec("Mile", &[])];
        let ctx = UserQueryContext { max_result_count: 2, max_suggestion_count: 1, ..UserQueryContext::default() };
        let out = run_user_query("mil", &ctx, &records).unwrap();
        assert_eq!(out.items.len(), 2);
        assert_eq!(out.suggestions.len(), 1);
        assert!(run_user_query("mil ", &ctx, &records).unwrap().suggestions.is_empty());
    }

    #[test]
    fn an_empty_query_is_refused() {
        assert_eq!(run_user_query("  ", &UserQueryContext::default(), &[]), Err(Error::EmptyQueryString));
    }
}
