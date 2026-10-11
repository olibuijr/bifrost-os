//! A change history for a local store (`NativePersistentHistoryTransaction`). Each save appends one transaction that lists
//! the inserts, updates and deletes it made. A reader keeps the token of the last transaction it processed and asks for
//! what came after it. Purging drops transactions that every reader has seen. A reader whose token predates a purge is
//! told that its history is incomplete.

/// A bookmark in the history. Tokens increase with each transaction and are never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Token(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Insert,
    Update,
    Delete,
}

/// One object change: which entity and which object, and how it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub entity: String,
    pub id: String,
    pub kind: ChangeKind,
}

impl Change {
    pub fn new(entity: &str, id: &str, kind: ChangeKind) -> Self {
        Change { entity: entity.into(), id: id.into(), kind }
    }
}

/// One save: who made it, and the changes it contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transaction {
    pub token: Token,
    pub author: String,
    pub changes: Vec<Change>,
}

/// The kept transactions of one store, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    transactions: Vec<Transaction>,
    last: u64,
    /// Everything at or below this token has been purged.
    purged_through: Option<Token>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records one save by `author`. A save with no changes records nothing and returns `None`.
    pub fn record(&mut self, author: &str, changes: Vec<Change>) -> Option<Token> {
        if changes.is_empty() {
            return None;
        }
        self.last += 1;
        let token = Token(self.last);
        self.transactions.push(Transaction { token, author: author.into(), changes });
        Some(token)
    }

    /// The token of the newest transaction: the bookmark to keep once a reader has processed everything.
    pub fn latest(&self) -> Option<Token> {
        self.transactions.last().map(|t| t.token)
    }

    /// The transactions after `since`, oldest first. `None` reads from the start of what is kept.
    pub fn since(&self, since: Option<Token>) -> impl Iterator<Item = &Transaction> + '_ {
        self.transactions.iter().filter(move |t| since.map_or(true, |s| t.token > s))
    }

    /// Whether a reader at `since` sees every change after it, rather than a history that purging has cut short.
    pub fn is_complete_since(&self, since: Option<Token>) -> bool {
        match (self.purged_through, since) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(purged), Some(s)) => s >= purged,
        }
    }

    /// Drops transactions up to and including `through`, once every reader has processed them. Returns how many went.
    pub fn purge_through(&mut self, through: Token) -> usize {
        let before = self.transactions.len();
        self.transactions.retain(|t| t.token > through);
        let removed = before - self.transactions.len();
        if removed > 0 {
            self.purged_through = Some(self.purged_through.map_or(through, |p| p.max(through)));
        }
        removed
    }

    pub fn len(&self) -> usize {
        self.transactions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transactions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, kind: ChangeKind) -> Change {
        Change::new("Note", id, kind)
    }

    #[test]
    fn each_save_gets_the_next_token_and_empty_saves_record_nothing() {
        let mut h = History::new();
        assert_eq!(h.record("notes", vec![]), None);
        assert!(h.is_empty());
        assert_eq!(h.record("notes", vec![note("a", ChangeKind::Insert)]), Some(Token(1)));
        assert_eq!(h.record("notes", vec![note("a", ChangeKind::Update)]), Some(Token(2)));
        assert_eq!(h.latest(), Some(Token(2)));
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn a_reader_gets_only_the_transactions_after_its_bookmark_in_order() {
        let mut h = History::new();
        h.record("notes", vec![note("a", ChangeKind::Insert)]);
        h.record("calendar", vec![note("b", ChangeKind::Insert)]);
        h.record("notes", vec![note("a", ChangeKind::Delete)]);
        let after_first: Vec<Token> = h.since(Some(Token(1))).map(|t| t.token).collect();
        assert_eq!(after_first, vec![Token(2), Token(3)]);
        let everything: Vec<&str> = h.since(None).map(|t| t.author.as_str()).collect();
        assert_eq!(everything, ["notes", "calendar", "notes"]);
    }

    #[test]
    fn transactions_keep_their_author_and_changes() {
        let mut h = History::new();
        h.record("contacts", vec![note("x", ChangeKind::Insert), note("y", ChangeKind::Update)]);
        let t = h.since(None).next().unwrap();
        assert_eq!(t.author, "contacts");
        assert_eq!(t.changes, vec![note("x", ChangeKind::Insert), note("y", ChangeKind::Update)]);
    }

    #[test]
    fn purging_drops_seen_transactions_and_flags_readers_that_fell_behind() {
        let mut h = History::new();
        for id in ["a", "b", "c"] {
            h.record("notes", vec![note(id, ChangeKind::Insert)]);
        }
        assert_eq!(h.purge_through(Token(2)), 2);
        assert_eq!(h.len(), 1);
        let kept: Vec<Token> = h.since(None).map(|t| t.token).collect();
        assert_eq!(kept, vec![Token(3)]);
        assert!(!h.is_complete_since(None), "a reader from the start missed the purged transactions");
        assert!(!h.is_complete_since(Some(Token(1))), "a reader at token 1 still needs token 2");
        assert!(h.is_complete_since(Some(Token(2))), "a reader at the purge point is complete");
    }

    #[test]
    fn tokens_keep_increasing_after_a_purge() {
        let mut h = History::new();
        h.record("notes", vec![note("a", ChangeKind::Insert)]);
        h.purge_through(Token(1));
        assert!(h.is_empty());
        assert_eq!(h.latest(), None);
        assert_eq!(h.record("notes", vec![note("b", ChangeKind::Insert)]), Some(Token(2)));
    }
}
