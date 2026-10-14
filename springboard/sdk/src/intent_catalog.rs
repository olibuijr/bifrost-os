//! The App Intents catalog: the intents an app declares, checked together. `App::intents()` returns a list, and the
//! catalog refuses a list that the control socket or the shell cannot serve: an identifier that is not a method token,
//! two intents with one identifier, a blank title, or a shortcut that needs a parameter (Search runs a shortcut
//! without asking for input). It also gives the lookups the shell and the tests use.

use std::collections::HashSet;

use crate::intents::Intent;

/// Why an intent list is not a valid catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// The identifier is empty or has a character outside `a-z`, `0-9`, `-` and `_`.
    BadId(String),
    /// Two intents share this identifier.
    Duplicate(String),
    /// The intent has a blank title.
    NoTitle(String),
    /// The intent is a shortcut but has a required parameter.
    ShortcutNeedsParameter(String),
}

/// A checked list of intents in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    intents: Vec<Intent>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

impl Catalog {
    /// Checks every intent and keeps the list in its order. The first problem found is returned.
    pub fn new(intents: Vec<Intent>) -> Result<Catalog, CatalogError> {
        let mut seen = HashSet::new();
        for intent in &intents {
            if !valid_id(&intent.id) {
                return Err(CatalogError::BadId(intent.id.clone()));
            }
            if !seen.insert(intent.id.as_str()) {
                return Err(CatalogError::Duplicate(intent.id.clone()));
            }
            if intent.title.trim().is_empty() {
                return Err(CatalogError::NoTitle(intent.id.clone()));
            }
            if intent.shortcut && intent.params.iter().any(|p| !p.optional) {
                return Err(CatalogError::ShortcutNeedsParameter(intent.id.clone()));
            }
        }
        Ok(Catalog { intents })
    }

    /// The intent with this identifier, matched exactly.
    pub fn get(&self, id: &str) -> Option<&Intent> {
        self.intents.iter().find(|i| i.id == id)
    }

    /// Every intent in declaration order.
    pub fn intents(&self) -> &[Intent] {
        &self.intents
    }

    /// The identifiers in declaration order.
    pub fn ids(&self) -> Vec<&str> {
        self.intents.iter().map(|i| i.id.as_str()).collect()
    }

    /// The intents Search lists as shortcuts, in declaration order.
    pub fn shortcuts(&self) -> Vec<&Intent> {
        self.intents.iter().filter(|i| i.shortcut).collect()
    }

    pub fn len(&self) -> usize {
        self.intents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }
}
