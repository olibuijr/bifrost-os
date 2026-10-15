//! Search scopes, search tokens and suggestions for a search field.
//!
//! A search field can narrow its results to one scope (a choice from a few, such as "Everything" or "Titles"), carry
//! tokens (chips that stand for a filter the user picked) and offer suggestions while the user types. [`SearchBar`]
//! holds that state and answers the questions the field draws from it. It draws nothing and runs no search: the app
//! reads the scope and tokens and decides what to match. Scope and token ids belong to the app.

/// One scope the user can narrow a search to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchScope {
    pub id: u32,
    pub title: String,
}

/// A token in the field: a filter the user picked, shown as a chip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchToken {
    pub id: u32,
    pub title: String,
}

/// The scope, tokens and suggestions of one search field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchBar {
    scopes: Vec<SearchScope>,
    selected: Option<u32>,
    tokens: Vec<SearchToken>,
}

impl SearchBar {
    /// A field with `scopes`. The first scope is selected; with no scopes the field has no scope choice.
    pub fn new(scopes: Vec<SearchScope>) -> Self {
        let selected = scopes.first().map(|s| s.id);
        SearchBar { scopes, selected, tokens: Vec::new() }
    }

    pub fn scopes(&self) -> &[SearchScope] {
        &self.scopes
    }

    /// The id of the scope the search is narrowed to, or `None` when the field has no scopes.
    pub fn selected_scope(&self) -> Option<u32> {
        self.selected
    }

    /// Narrows the search to the scope `id`. Returns false when the field has no such scope.
    pub fn select_scope(&mut self, id: u32) -> bool {
        if self.scopes.iter().any(|s| s.id == id) {
            self.selected = Some(id);
            true
        } else {
            false
        }
    }

    /// Moves to the next scope, wrapping after the last one. Returns the scope now selected.
    pub fn next_scope(&mut self) -> Option<u32> {
        let at = self.scopes.iter().position(|s| Some(s.id) == self.selected)?;
        let next = self.scopes[(at + 1) % self.scopes.len()].id;
        self.selected = Some(next);
        self.selected
    }

    pub fn tokens(&self) -> &[SearchToken] {
        &self.tokens
    }

    /// Adds a token at the end of the field. Returns false when a token with the same id is already there.
    pub fn add_token(&mut self, token: SearchToken) -> bool {
        if self.tokens.iter().any(|t| t.id == token.id) {
            return false;
        }
        self.tokens.push(token);
        true
    }

    /// Removes the token `id`. Returns false when no such token is in the field.
    pub fn remove_token(&mut self, id: u32) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|t| t.id != id);
        self.tokens.len() != before
    }

    /// Suggestions for `text`: the candidates that start with the text, ignoring case and surrounding spaces, in
    /// their given order, without repeats and without the text itself, at most `limit` of them. Blank text gives none.
    pub fn suggestions<'a>(&self, text: &str, candidates: &[&'a str], limit: usize) -> Vec<&'a str> {
        let typed = text.trim().to_lowercase();
        if typed.is_empty() {
            return Vec::new();
        }
        let mut out: Vec<&'a str> = Vec::new();
        for &candidate in candidates {
            if out.len() >= limit {
                break;
            }
            let lower = candidate.to_lowercase();
            let is_match = lower.starts_with(&typed) && lower != typed;
            let is_repeat = out.iter().any(|s| s.to_lowercase() == lower);
            if is_match && !is_repeat {
                out.push(candidate);
            }
        }
        out
    }
}
