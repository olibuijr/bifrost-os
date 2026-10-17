//! Recipient recents: the addresses Mail and Messages used last, offered again as suggestions when a recipient is typed.
//! A pure model with no I/O, built on `shared_list`. An address is the list key, compared without case, so it is
//! returned in lower case; the name is the title. Most recent first, capped like the recents menu. A favourite
//! recipient is never dropped by the cap, and `suggest` lists favourites first.

use crate::shared_list::SharedList;

/// One recipient as the suggestion list shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipient {
    /// The address in lower case.
    pub address: String,
    /// The last name recorded for the address, or the address itself when none was given.
    pub name: String,
    pub favourite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipientRecents {
    list: SharedList,
}

impl RecipientRecents {
    /// A list that keeps `capacity` recent recipients (at least one).
    pub fn new(capacity: usize) -> Self {
        RecipientRecents { list: SharedList::new(capacity) }
    }

    /// Records a use of `address`, moving it to the front. An address needs one `@` with text on both sides and no
    /// spaces, or it is refused and false is returned. An empty `name` keeps the name recorded before.
    pub fn record(&mut self, address: &str, name: &str) -> bool {
        let address = address.trim().to_lowercase();
        if !is_address(&address) {
            return false;
        }
        let name = name.trim();
        let title = if name.is_empty() {
            self.name_of(&address).unwrap_or_else(|| address.clone())
        } else {
            name.to_string()
        };
        self.list.add(&address, &title)
    }

    /// Recipients whose address or name contains `text` (ignoring case), favourites first then most recent. An empty
    /// `text` matches everyone. At most `limit` are returned.
    pub fn suggest(&self, text: &str, limit: usize) -> Vec<Recipient> {
        let needle = text.trim().to_lowercase();
        self.list
            .entries()
            .into_iter()
            .filter(|i| needle.is_empty() || i.key.contains(&needle) || i.title.to_lowercase().contains(&needle))
            .take(limit)
            .map(|i| Recipient { address: i.key.clone(), name: i.title.clone(), favourite: i.favourite })
            .collect()
    }

    /// Marks or unmarks a recipient as a favourite. False when the address is not in the list.
    pub fn set_favourite(&mut self, address: &str, on: bool) -> bool {
        self.list.set_favourite(&address.trim().to_lowercase(), on)
    }

    /// Removes one recipient, favourite or not. False when the address is not in the list.
    pub fn forget(&mut self, address: &str) -> bool {
        self.list.remove(&address.trim().to_lowercase())
    }

    /// Clears the recent recipients; favourites stay.
    pub fn clear_recents(&mut self) {
        self.list.clear_recents();
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    fn name_of(&self, address: &str) -> Option<String> {
        self.list.entries().into_iter().find(|i| i.key == address).map(|i| i.title.clone())
    }
}

/// True for `local@domain` with text on both sides of one `@` and no whitespace.
fn is_address(s: &str) -> bool {
    if s.chars().any(char::is_whitespace) {
        return false;
    }
    match s.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && !domain.contains('@'),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addresses(r: &[Recipient]) -> Vec<&str> {
        r.iter().map(|x| x.address.as_str()).collect()
    }

    #[test]
    fn newest_recipient_is_suggested_first() {
        let mut r = RecipientRecents::new(5);
        assert!(r.record("ann@example.com", "Ann"));
        assert!(r.record("bob@example.com", "Bob"));
        assert_eq!(addresses(&r.suggest("", 10)), vec!["bob@example.com", "ann@example.com"]);
    }

    #[test]
    fn addresses_ignore_case_and_reuse_does_not_duplicate() {
        let mut r = RecipientRecents::new(5);
        r.record("Ann@Example.com", "Ann");
        r.record("ann@example.com", "Ann B");
        assert_eq!(r.len(), 1);
        let s = r.suggest("", 10);
        assert_eq!(s[0].address, "ann@example.com");
        assert_eq!(s[0].name, "Ann B", "the latest name wins");
    }

    #[test]
    fn an_empty_name_keeps_the_name_recorded_before() {
        let mut r = RecipientRecents::new(5);
        r.record("ann@example.com", "Ann");
        r.record("ann@example.com", "  ");
        assert_eq!(r.suggest("", 1)[0].name, "Ann");
        r.record("new@example.com", "");
        assert_eq!(r.suggest("", 1)[0].name, "new@example.com", "no name falls back to the address");
    }

    #[test]
    fn malformed_addresses_are_refused() {
        let mut r = RecipientRecents::new(5);
        assert!(!r.record("", "Nobody"));
        assert!(!r.record("no-at-sign", ""));
        assert!(!r.record("@example.com", ""));
        assert!(!r.record("ann@", ""));
        assert!(!r.record("ann @example.com", ""));
        assert!(!r.record("a@b@c", ""));
        assert!(r.is_empty());
    }

    #[test]
    fn suggest_matches_address_or_name_ignoring_case_and_respects_limit() {
        let mut r = RecipientRecents::new(5);
        r.record("ann@example.com", "Ann Smith");
        r.record("bob@other.org", "Bob");
        r.record("carl@example.com", "Carl");
        assert_eq!(addresses(&r.suggest("EXAMPLE", 10)), vec!["carl@example.com", "ann@example.com"]);
        assert_eq!(addresses(&r.suggest("smi", 10)), vec!["ann@example.com"]);
        assert_eq!(r.suggest("", 2).len(), 2);
        assert!(r.suggest("zed", 10).is_empty());
    }

    #[test]
    fn recents_are_capped_and_the_oldest_is_dropped() {
        let mut r = RecipientRecents::new(2);
        r.record("a@x.org", "");
        r.record("b@x.org", "");
        r.record("c@x.org", "");
        assert_eq!(addresses(&r.suggest("", 10)), vec!["c@x.org", "b@x.org"]);
    }

    #[test]
    fn a_favourite_survives_the_cap_and_is_listed_first() {
        let mut r = RecipientRecents::new(1);
        r.record("a@x.org", "");
        assert!(r.set_favourite("A@x.org", true));
        r.record("b@x.org", "");
        r.record("c@x.org", "");
        assert_eq!(addresses(&r.suggest("", 10)), vec!["a@x.org", "c@x.org"]);
        assert!(r.suggest("", 10)[0].favourite);
    }

    #[test]
    fn forget_and_clear_keep_favourites_where_they_should() {
        let mut r = RecipientRecents::new(5);
        r.record("a@x.org", "");
        r.record("b@x.org", "");
        r.set_favourite("a@x.org", true);
        assert!(r.forget("B@x.org"));
        assert!(!r.forget("b@x.org"), "a second forget finds nothing");
        r.record("c@x.org", "");
        r.clear_recents();
        assert_eq!(addresses(&r.suggest("", 10)), vec!["a@x.org"]);
    }

    #[test]
    fn set_favourite_on_an_unknown_address_is_refused() {
        let mut r = RecipientRecents::new(5);
        assert!(!r.set_favourite("nobody@x.org", true));
    }
}
