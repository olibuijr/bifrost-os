//! Search result cards that apps supply themselves: the card provider layer of the system search.
//!
//! An app publishes the cards it wants shown for a query (a title, a subtitle and an id it can act on later), and the
//! shell asks the registry for the cards that match what the user typed. Matching follows the Search Kit rule: every
//! query word must be the start of a word in the card's title or subtitle, and a card whose title matches ranks above
//! one that matches only in the subtitle. Cards are grouped by app, so one app's cards replace its earlier set and an
//! app that quits can have its cards removed.
//!
//! ```ignore
//! let mut cards = CardProviders::new();
//! cards.set_cards("notes", vec![Card::new("notes", "n1", "Oat milk list", "Groceries")]);
//! let hits = cards.results("oat", 8);   // [Card { title: "Oat milk list", .. }]
//! ```

use std::collections::BTreeMap;

/// One result card an app supplies: the app that owns it, the id the app uses to act on it, and the text shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub app: String,
    pub id: String,
    pub title: String,
    pub subtitle: String,
}

impl Card {
    /// A card for `app` with the given id, title and subtitle.
    pub fn new(app: &str, id: &str, title: &str, subtitle: &str) -> Card {
        Card { app: app.to_string(), id: id.to_string(), title: title.to_string(), subtitle: subtitle.to_string() }
    }
}

/// The cards every app has published, keyed by app id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardProviders {
    apps: BTreeMap<String, Vec<Card>>,
}

/// Lowercase runs of letters and digits.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Whether every query word starts a word of `text`.
fn all_words_start(text: &[String], query: &[String]) -> bool {
    query.iter().all(|q| text.iter().any(|w| w.starts_with(q.as_str())))
}

impl CardProviders {
    /// An empty registry.
    pub fn new() -> CardProviders {
        CardProviders::default()
    }

    /// Replaces the cards of `app` with `cards`. A card whose `app` field names another app is stored under its own
    /// app id, so an app cannot publish cards for someone else by accident.
    pub fn set_cards(&mut self, app: &str, cards: Vec<Card>) {
        self.apps.remove(app);
        for card in cards {
            self.apps.entry(card.app.clone()).or_default().push(card);
        }
        self.apps.retain(|_, list| !list.is_empty());
    }

    /// Removes every card of `app`; `false` when that app had none.
    pub fn remove_app(&mut self, app: &str) -> bool {
        self.apps.remove(app).is_some()
    }

    /// How many cards the registry holds across all apps.
    pub fn card_count(&self) -> usize {
        self.apps.values().map(Vec::len).sum()
    }

    /// The cards matching `query`, best first: title matches before subtitle-only matches, then by app and id. At
    /// most `limit` cards. An empty query matches nothing.
    pub fn results(&self, query: &str, limit: usize) -> Vec<Card> {
        let query = words(query);
        if query.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut ranked: Vec<(bool, &Card)> = Vec::new();
        for card in self.apps.values().flatten() {
            let title = words(&card.title);
            let subtitle = words(&card.subtitle);
            if all_words_start(&title, &query) {
                ranked.push((true, card));
            } else if all_words_start(&[title, subtitle].concat(), &query) {
                ranked.push((false, card));
            }
        }
        ranked.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| a.1.app.cmp(&b.1.app)).then_with(|| a.1.id.cmp(&b.1.id))
        });
        ranked.into_iter().take(limit).map(|(_, card)| card.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CardProviders {
        let mut cards = CardProviders::new();
        cards.set_cards(
            "notes",
            vec![Card::new("notes", "n1", "Oat milk list", "Groceries"), Card::new("notes", "n2", "Trip plan", "Oat cake recipe")],
        );
        cards.set_cards("calendar", vec![Card::new("calendar", "e1", "Dentist", "Oat Street 4, 10:00")]);
        cards
    }

    #[test]
    fn title_matches_rank_above_subtitle_matches() {
        // "Oat milk list" matches in its title; the other two match only in their subtitles, and "calendar" sorts
        // before "notes".
        let ids: Vec<String> = sample().results("oat", 8).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, vec!["n1".to_string(), "e1".to_string(), "n2".to_string()]);
    }

    #[test]
    fn every_query_word_must_match_and_prefixes_count() {
        let cards = sample();
        assert_eq!(cards.results("oat mi", 8).len(), 1);
        assert_eq!(cards.results("oat zzz", 8).len(), 0);
        assert_eq!(cards.results("gro", 8)[0].id, "n1");
    }

    #[test]
    fn limit_and_empty_query() {
        let cards = sample();
        assert_eq!(cards.results("oat", 1).len(), 1);
        assert!(cards.results("", 8).is_empty());
        assert!(cards.results("oat", 0).is_empty());
    }

    #[test]
    fn set_cards_replaces_an_app_and_remove_app_drops_it() {
        let mut cards = sample();
        assert_eq!(cards.card_count(), 3);
        cards.set_cards("notes", vec![Card::new("notes", "n9", "Only one", "")]);
        assert_eq!(cards.card_count(), 2);
        assert!(cards.results("oat", 8).iter().all(|c| c.app != "notes" || c.id != "n1"));
        assert!(cards.remove_app("calendar"));
        assert!(!cards.remove_app("calendar"));
        assert_eq!(cards.card_count(), 1);
    }

    #[test]
    fn a_card_is_filed_under_its_own_app_id() {
        let mut cards = CardProviders::new();
        cards.set_cards("notes", vec![Card::new("mail", "m1", "Oat", "")]);
        assert_eq!(cards.results("oat", 8)[0].app, "mail");
        assert_eq!(cards.card_count(), 1);
    }
}
