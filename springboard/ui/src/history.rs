//! Change history with transaction tokens (`HistoryDescriptor`, `HistoryTransaction`, `HistoryChange`). Every commit
//! of changes gets the next token. A reader keeps the last token it has seen and fetches what came after it. Pure:
//! the caller supplies the author and the time, and decides how much of the log to keep.

/// A position in the history. Tokens only grow, and `Token(0)` is before the first transaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(pub u64);

/// One change to a record. `record` is the store's id for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Insert { record: u64, entity: String },
    Update { record: u64, entity: String },
    Delete { record: u64, entity: String },
}

impl Change {
    pub fn record(&self) -> u64 {
        match self {
            Change::Insert { record, .. } | Change::Update { record, .. } | Change::Delete { record, .. } => *record,
        }
    }
}

/// One saved transaction: its token, its author, its time and what changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub token: Token,
    pub author: String,
    pub at: u64,
    pub changes: Vec<Change>,
}

/// Which transactions to fetch: those after `after`, optionally by one author, at most `limit` of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Descriptor {
    pub after: Token,
    pub author: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct History {
    log: Vec<Transaction>,
    last: u64,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one transaction and returns its token. An empty change list records nothing and returns `None`.
    pub fn commit(&mut self, author: &str, at: u64, changes: Vec<Change>) -> Option<Token> {
        if changes.is_empty() {
            return None;
        }
        self.last += 1;
        let token = Token(self.last);
        self.log.push(Transaction { token, author: author.to_string(), at, changes });
        Some(token)
    }

    /// The newest token, or `Token(0)` when nothing was ever committed.
    pub fn latest(&self) -> Token {
        Token(self.last)
    }

    /// The transactions that match the descriptor, oldest first.
    pub fn fetch(&self, d: &Descriptor) -> Vec<&Transaction> {
        self.log
            .iter()
            .filter(|t| t.token > d.after && d.author.as_deref().map_or(true, |a| t.author == a))
            .take(d.limit.unwrap_or(usize::MAX))
            .collect()
    }

    /// Drops every transaction up to and including `through`. Returns how many were dropped. Later tokens keep
    /// their numbers.
    pub fn prune(&mut self, through: Token) -> usize {
        let before = self.log.len();
        self.log.retain(|t| t.token > through);
        before - self.log.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(record: u64) -> Change {
        Change::Insert { record, entity: "Note".into() }
    }

    #[test]
    fn commit_numbers_transactions_and_skips_an_empty_change_list() {
        let mut h = History::new();
        assert_eq!(h.latest(), Token(0));
        assert_eq!(h.commit("ui", 1, vec![insert(1)]), Some(Token(1)));
        assert_eq!(h.commit("ui", 2, vec![]), None, "nothing changed, so nothing is recorded");
        assert_eq!(h.commit("ui", 3, vec![insert(2)]), Some(Token(2)), "a skipped commit takes no token");
        assert_eq!(h.latest(), Token(2));
    }

    #[test]
    fn fetch_returns_only_newer_transactions_oldest_first() {
        let mut h = History::new();
        for n in 1..=3 {
            h.commit("ui", n, vec![insert(n)]);
        }
        let d = Descriptor { after: Token(1), ..Descriptor::default() };
        let tokens: Vec<Token> = h.fetch(&d).iter().map(|t| t.token).collect();
        assert_eq!(tokens, vec![Token(2), Token(3)]);
        assert_eq!(h.fetch(&Descriptor::default()).len(), 3, "a reader that has seen nothing gets everything");
    }

    #[test]
    fn fetch_filters_by_author_and_applies_the_limit() {
        let mut h = History::new();
        h.commit("ui", 1, vec![insert(1)]);
        h.commit("sync", 2, vec![insert(2)]);
        h.commit("ui", 3, vec![insert(3)]);
        h.commit("ui", 4, vec![insert(4)]);
        let d = Descriptor { author: Some("ui".into()), limit: Some(2), ..Descriptor::default() };
        let got: Vec<Token> = h.fetch(&d).iter().map(|t| t.token).collect();
        assert_eq!(got, vec![Token(1), Token(3)]);
    }

    #[test]
    fn prune_drops_old_transactions_and_keeps_later_tokens() {
        let mut h = History::new();
        for n in 1..=3 {
            h.commit("ui", n, vec![insert(n)]);
        }
        assert_eq!(h.prune(Token(2)), 2);
        let left: Vec<Token> = h.fetch(&Descriptor::default()).iter().map(|t| t.token).collect();
        assert_eq!(left, vec![Token(3)]);
        assert_eq!(h.commit("ui", 4, vec![insert(4)]), Some(Token(4)), "pruning does not reuse numbers");
    }

    #[test]
    fn change_reports_its_record_for_every_kind() {
        let changes = [
            Change::Insert { record: 7, entity: "Note".into() },
            Change::Update { record: 7, entity: "Note".into() },
            Change::Delete { record: 7, entity: "Note".into() },
        ];
        assert!(changes.iter().all(|c| c.record() == 7));
        assert!(matches!(changes[2], Change::Delete { .. }));
    }
}
