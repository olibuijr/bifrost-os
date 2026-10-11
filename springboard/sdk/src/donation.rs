//! Intent donations: the app tells the shell that the user did an action, so Search can suggest it again later.
//!
//! An app donates once each time the user completes an action (`donate`), with a persistent identifier that it
//! later uses to remove that donation (`delete`). The store keeps the most recent `limit` donations, replaces a
//! donation that comes back with the same identifier, and ranks what to suggest by how often each action was done
//! (`suggestions`). Times are plain seconds passed in by the caller, so the model stays pure.
//!
//! Mirrors the official articles on donating shortcuts and deleting donated shortcuts: interaction donations and
//! user activity donations (eligible for prediction), and the delete calls by identifier, by intent and all.
//! The shell side that shows suggestions is not built; see docs/bifrost-ui.md "App Intents".

use bifrost_core::json::Json;

/// One completed action: which intent ran, with what parameters, and when.
#[derive(Clone, Debug, PartialEq)]
pub struct Donation {
    /// Persistent identifier, unique within the app. A later donation with the same value replaces this one.
    pub identifier: String,
    /// The intent id (`intents::Intent::id`) that the action came from.
    pub intent_id: String,
    /// Human title shown in suggestions.
    pub title: String,
    /// Parameters the action ran with (`Json::Null` when there are none).
    pub params: Json,
    /// When the action was completed, in seconds since the epoch.
    pub at: u64,
}

impl Donation {
    pub fn new(identifier: &str, intent_id: &str, title: &str, at: u64) -> Donation {
        Donation { identifier: identifier.into(), intent_id: intent_id.into(), title: title.into(), params: Json::Null, at }
    }
    pub fn params(mut self, params: Json) -> Donation {
        self.params = params;
        self
    }
}

/// Why a donation was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DonationError {
    EmptyIdentifier,
    EmptyIntent,
    EmptyTitle,
}

/// A suggestion: one action (intent, title and parameters) with how many times it was donated and when last.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub intent_id: String,
    pub title: String,
    pub params: Json,
    pub count: usize,
    pub last: u64,
}

/// The donations of one app, bounded by `limit`.
#[derive(Clone, Debug, Default)]
pub struct DonationStore {
    items: Vec<Donation>,
    limit: usize,
}

impl DonationStore {
    /// A store that keeps at most `limit` donations (at least one).
    pub fn new(limit: usize) -> DonationStore {
        DonationStore { items: Vec::new(), limit: limit.max(1) }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Records a donation. A donation with the same identifier replaces the earlier one, so an action donated again
    /// under one identifier is still one donation. When the store is full, the oldest donation is dropped.
    pub fn donate(&mut self, d: Donation) -> Result<(), DonationError> {
        if d.identifier.trim().is_empty() {
            return Err(DonationError::EmptyIdentifier);
        }
        if d.intent_id.trim().is_empty() {
            return Err(DonationError::EmptyIntent);
        }
        if d.title.trim().is_empty() {
            return Err(DonationError::EmptyTitle);
        }
        self.items.retain(|old| old.identifier != d.identifier);
        self.items.push(d);
        while self.items.len() > self.limit {
            let oldest = self.items.iter().enumerate().min_by_key(|(i, d)| (d.at, *i)).map(|(i, _)| i).unwrap_or(0);
            self.items.remove(oldest);
        }
        Ok(())
    }

    /// Removes the donations with these identifiers and returns how many were removed.
    pub fn delete(&mut self, identifiers: &[&str]) -> usize {
        let before = self.items.len();
        self.items.retain(|d| !identifiers.contains(&d.identifier.as_str()));
        before - self.items.len()
    }

    /// Removes every donation of one intent and returns how many were removed.
    pub fn delete_intent(&mut self, intent_id: &str) -> usize {
        let before = self.items.len();
        self.items.retain(|d| d.intent_id != intent_id);
        before - self.items.len()
    }

    /// Removes every donation and returns how many there were.
    pub fn delete_all(&mut self) -> usize {
        let n = self.items.len();
        self.items.clear();
        n
    }

    /// The `k` most donated actions. Ranked by count (most first), then by the most recent donation, then by title.
    /// Donations of the same intent with different parameters are separate suggestions.
    pub fn suggestions(&self, k: usize) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> = Vec::new();
        for d in &self.items {
            match out.iter_mut().find(|s| s.intent_id == d.intent_id && s.title == d.title && s.params == d.params) {
                Some(s) => {
                    s.count += 1;
                    s.last = s.last.max(d.at);
                }
                None => out.push(Suggestion { intent_id: d.intent_id.clone(), title: d.title.clone(), params: d.params.clone(), count: 1, last: d.at }),
            }
        }
        out.sort_by(|a, b| b.count.cmp(&a.count).then(b.last.cmp(&a.last)).then(a.title.cmp(&b.title)));
        out.truncate(k);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(id: &str, at: u64) -> Donation {
        Donation::new(id, "order-soup", "Order soup", at)
    }

    #[test]
    fn refuses_donations_without_identifier_intent_or_title() {
        let mut s = DonationStore::new(10);
        assert_eq!(s.donate(Donation::new(" ", "x", "X", 1)), Err(DonationError::EmptyIdentifier));
        assert_eq!(s.donate(Donation::new("a", "", "X", 1)), Err(DonationError::EmptyIntent));
        assert_eq!(s.donate(Donation::new("a", "x", "  ", 1)), Err(DonationError::EmptyTitle));
        assert!(s.is_empty());
    }

    #[test]
    fn same_identifier_replaces_the_earlier_donation() {
        let mut s = DonationStore::new(10);
        s.donate(order("o1", 100)).unwrap();
        s.donate(order("o1", 200)).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s.suggestions(5)[0].last, 200);
    }

    #[test]
    fn full_store_drops_the_oldest_donation() {
        let mut s = DonationStore::new(2);
        s.donate(order("a", 30)).unwrap();
        s.donate(order("b", 10)).unwrap();
        s.donate(order("c", 20)).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s.delete(&["b"]), 0, "the oldest one, b, was already dropped");
        assert_eq!(s.delete(&["a", "c"]), 2);
    }

    #[test]
    fn delete_by_identifier_intent_and_all() {
        let mut s = DonationStore::new(10);
        s.donate(order("a", 1)).unwrap();
        s.donate(order("b", 2)).unwrap();
        s.donate(Donation::new("n", "new-note", "New note", 3)).unwrap();
        assert_eq!(s.delete(&["a", "zz"]), 1);
        assert_eq!(s.delete_intent("order-soup"), 1);
        assert_eq!(s.len(), 1);
        assert_eq!(s.delete_all(), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn suggestions_rank_by_count_then_recency() {
        let mut s = DonationStore::new(10);
        s.donate(order("a", 5)).unwrap();
        s.donate(order("b", 9)).unwrap();
        s.donate(Donation::new("n", "new-note", "New note", 50)).unwrap();
        let top = s.suggestions(5);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].title, "Order soup");
        assert_eq!(top[0].count, 2);
        assert_eq!(top[0].last, 9);
        assert_eq!(top[1].title, "New note");
        assert_eq!(s.suggestions(1).len(), 1);
    }

    #[test]
    fn same_intent_with_different_params_is_a_separate_suggestion() {
        let mut s = DonationStore::new(10);
        let soup = Json::Obj(vec![("dish".into(), Json::Str("soup".into()))]);
        let bread = Json::Obj(vec![("dish".into(), Json::Str("bread".into()))]);
        s.donate(order("a", 1).params(soup.clone())).unwrap();
        s.donate(order("b", 2).params(soup.clone())).unwrap();
        s.donate(order("c", 3).params(bread.clone())).unwrap();
        let top = s.suggestions(5);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].params, soup);
        assert_eq!(top[0].count, 2);
        assert_eq!(top[1].params, bread);
    }
}
