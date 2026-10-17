//! Route suggestions for the sound output picker. Suggestions are the destinations an account has used before, most
//! used first and, among equally used ones, the most recently used first. The history is the list of destination
//! identifiers the picker chose, oldest first, so the model needs no clock.

use crate::route_picker::Destination;

/// Returns up to `limit` destinations from `destinations` that appear in `history`, ranked as described above.
/// A destination never chosen is not suggested. Identifiers in `history` that match no destination are ignored.
pub fn suggest<'a>(destinations: &'a [Destination], history: &[&str], limit: usize) -> Vec<&'a Destination> {
    let mut ranked: Vec<(usize, usize, &Destination)> = destinations
        .iter()
        .filter_map(|destination| {
            let count = history.iter().filter(|id| **id == destination.id).count();
            let last = history.iter().rposition(|id| *id == destination.id)?;
            Some((count, last, destination))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    ranked.into_iter().take(limit).map(|(_, _, destination)| destination).collect()
}

/// The most destination choices kept. Older choices are dropped first, so the file stays small.
pub const MAX_HISTORY: usize = 256;

/// The persisted picker history: destination identifiers the picker chose, oldest first, at most [`MAX_HISTORY`].
/// It is stored as one identifier per line. A blank line or one starting with `#` is skipped when read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouteHistory {
    choices: Vec<String>,
}

impl RouteHistory {
    /// An empty history.
    pub fn new() -> RouteHistory {
        RouteHistory::default()
    }

    /// Records a choice as the newest entry. Identifiers that are blank or contain a line break are refused, since
    /// they could not be stored one per line.
    pub fn record(&mut self, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() || id.contains(['\n', '\r']) || id.starts_with('#') {
            return false;
        }
        self.choices.push(id.to_string());
        if self.choices.len() > MAX_HISTORY {
            let excess = self.choices.len() - MAX_HISTORY;
            self.choices.drain(..excess);
        }
        true
    }

    /// The recorded identifiers, oldest first, in the form [`suggest`] takes.
    pub fn ids(&self) -> Vec<&str> {
        self.choices.iter().map(String::as_str).collect()
    }

    /// Reads a history file. Lines are trimmed; blank lines and `#` comments are skipped. The newest [`MAX_HISTORY`]
    /// choices are kept, the same bound [`record`](RouteHistory::record) applies.
    pub fn parse(text: &str) -> RouteHistory {
        let mut history = RouteHistory::new();
        for line in text.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                history.record(line);
            }
        }
        history
    }

    /// Writes the history, one identifier per line, oldest first.
    pub fn to_text(&self) -> String {
        self.choices.iter().map(|id| format!("{id}\n")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_round_trips_in_order() {
        let mut history = RouteHistory::new();
        assert!(history.record("speaker"));
        assert!(history.record("receiver-kitchen"));
        assert!(history.record("speaker"));
        let text = history.to_text();
        assert_eq!(text, "speaker\nreceiver-kitchen\nspeaker\n");
        assert_eq!(RouteHistory::parse(&text), history);
        assert_eq!(RouteHistory::parse(&text).ids(), vec!["speaker", "receiver-kitchen", "speaker"]);
    }

    #[test]
    fn history_refuses_blank_and_multiline_ids() {
        let mut history = RouteHistory::new();
        assert!(!history.record("   "));
        assert!(!history.record("kitchen\nspeaker"));
        assert!(!history.record("# comment"));
        assert!(history.ids().is_empty());
    }

    #[test]
    fn history_keeps_only_the_newest_choices() {
        let mut history = RouteHistory::new();
        for n in 0..MAX_HISTORY + 10 {
            history.record(&format!("dest-{n}"));
        }
        let ids = history.ids();
        assert_eq!(ids.len(), MAX_HISTORY);
        assert_eq!(ids[0], "dest-10");
        assert_eq!(ids[MAX_HISTORY - 1], format!("dest-{}", MAX_HISTORY + 9));
    }

    #[test]
    fn parsed_history_feeds_the_suggestions() {
        let destinations = vec![
            Destination { id: "speaker".into(), name: "Speaker".into(), local: true, active: false },
            Destination { id: "receiver-kitchen".into(), name: "Kitchen".into(), local: false, active: false },
            Destination { id: "receiver-hall".into(), name: "Hall".into(), local: false, active: false },
        ];
        let history = RouteHistory::parse("# picks\nreceiver-kitchen\nspeaker\nreceiver-kitchen\n\n");
        let suggested = suggest(&destinations, &history.ids(), 2);
        let ids: Vec<&str> = suggested.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["receiver-kitchen", "speaker"]);
    }
}
