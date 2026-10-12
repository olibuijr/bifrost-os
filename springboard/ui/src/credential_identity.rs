//! Credential identities (parity: Authentication Services, "AutoFill credentials"): the records a credential manager
//! saves into a shared store, so the system can offer them as AutoFill suggestions for a service such as a web site.
//!
//! Mirrors `ASCredentialIdentityStore` (save, replace, remove, remove all, the store state and fetching for a service),
//! `ASCredentialServiceIdentifier` (URL, domain and app identifier types) and the identity fields `rank`, `user`,
//! `recordIdentifier` and `serviceIdentifier`. Pure model: no shared store on disk, no suggestion bar and no extension
//! process. The caller decides what to save and when to ask.
//!
//! Matching rule: a URL or domain identifier matches a query when the query's host is the identifier's host or a
//! subdomain of it (`login.example.com` matches `example.com`, `notexample.com` does not). An app identifier matches
//! only the same identifier, compared exactly.

/// The kind of service an identifier names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentifierType {
    /// A URL, such as `https://login.example.com/session`.
    Url,
    /// A domain, such as `example.com`.
    Domain,
    /// An application identifier, compared as a whole string.
    App,
}

/// Names the service a credential belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceIdentifier {
    pub identifier: String,
    pub kind: IdentifierType,
}

impl ServiceIdentifier {
    pub fn new(identifier: &str, kind: IdentifierType) -> ServiceIdentifier {
        ServiceIdentifier { identifier: identifier.to_string(), kind }
    }

    /// The lower-cased host of a URL or domain identifier: the text after any scheme, up to the first `/`, `?` or `#`,
    /// without user information or a port. An app identifier has no host and returns an empty string.
    pub fn host(&self) -> String {
        if self.kind == IdentifierType::App {
            return String::new();
        }
        let rest = match self.identifier.find("://") {
            Some(i) => &self.identifier[i + 3..],
            None => self.identifier.as_str(),
        };
        let authority = rest.split(|c: char| c == '/' || c == '?' || c == '#').next().unwrap_or("");
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        host_port.split(':').next().unwrap_or("").to_ascii_lowercase()
    }

    /// Whether this identifier covers `query`. See the module note for the rule.
    pub fn covers(&self, query: &ServiceIdentifier) -> bool {
        if self.kind == IdentifierType::App || query.kind == IdentifierType::App {
            return self.kind == query.kind && self.identifier == query.identifier;
        }
        let host = self.host();
        let query_host = query.host();
        if host.is_empty() || query_host.is_empty() {
            return false;
        }
        query_host == host || query_host.ends_with(&format!(".{host}"))
    }
}

/// The kinds of credential an identity can describe. Mirrors `ASCredentialIdentityStore.IdentityTypes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityKind {
    Passkey,
    Password,
    OneTimeCode,
}

/// A set of identity kinds, used to filter a fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityTypes {
    pub passkey: bool,
    pub password: bool,
    pub one_time_code: bool,
}

impl IdentityTypes {
    /// Every kind.
    pub fn all() -> IdentityTypes {
        IdentityTypes { passkey: true, password: true, one_time_code: true }
    }

    /// Whether the set includes `kind`.
    pub fn contains(&self, kind: IdentityKind) -> bool {
        match kind {
            IdentityKind::Passkey => self.passkey,
            IdentityKind::Password => self.password,
            IdentityKind::OneTimeCode => self.one_time_code,
        }
    }
}

/// One saved credential: which service it is for, which user name, and the record in the owner's own database.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialIdentity {
    pub service: ServiceIdentifier,
    pub user: String,
    /// Correlates the identity to a record in the app's database. Saving an identity with the same record and kind
    /// replaces the earlier one.
    pub record_identifier: String,
    /// Higher ranks are offered first.
    pub rank: i64,
    pub kind: IdentityKind,
}

/// Why the store refused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The store is switched off, so it takes no changes.
    Disabled,
}

/// The shared store of credential identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityStore {
    identities: Vec<CredentialIdentity>,
    enabled: bool,
}

impl Default for IdentityStore {
    fn default() -> IdentityStore {
        IdentityStore { identities: Vec::new(), enabled: true }
    }
}

impl IdentityStore {
    /// An empty, enabled store.
    pub fn new() -> IdentityStore {
        IdentityStore::default()
    }

    /// Whether the store is enabled. Mirrors the store state that `getState` reports.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Switches the store on or off. Switching it off clears every identity and returns how many were cleared; the
    /// system does the same when the person disables the extension.
    pub fn set_enabled(&mut self, enabled: bool) -> usize {
        self.enabled = enabled;
        if enabled {
            0
        } else {
            let cleared = self.identities.len();
            self.identities.clear();
            cleared
        }
    }

    /// Number of identities held.
    pub fn len(&self) -> usize {
        self.identities.len()
    }

    /// Whether the store holds no identities.
    pub fn is_empty(&self) -> bool {
        self.identities.is_empty()
    }

    /// Adds each identity, replacing an existing one with the same record and kind. Returns the number of identities
    /// that were new (not replacements).
    pub fn save(&mut self, items: &[CredentialIdentity]) -> Result<usize, StoreError> {
        self.check_enabled()?;
        let mut added = 0;
        for item in items {
            match self.identities.iter().position(|e| same_record(e, item)) {
                Some(i) => self.identities[i] = item.clone(),
                None => {
                    self.identities.push(item.clone());
                    added += 1;
                }
            }
        }
        Ok(added)
    }

    /// Removes the identities that match the record and kind of each item. Returns how many were removed.
    pub fn remove(&mut self, items: &[CredentialIdentity]) -> Result<usize, StoreError> {
        self.check_enabled()?;
        let before = self.identities.len();
        self.identities.retain(|e| !items.iter().any(|item| same_record(e, item)));
        Ok(before - self.identities.len())
    }

    /// Replaces the whole store with `items`.
    pub fn replace(&mut self, items: &[CredentialIdentity]) -> Result<(), StoreError> {
        self.check_enabled()?;
        self.identities.clear();
        self.save(items).map(|_| ())
    }

    /// Removes every identity and returns how many were removed.
    pub fn remove_all(&mut self) -> Result<usize, StoreError> {
        self.check_enabled()?;
        let removed = self.identities.len();
        self.identities.clear();
        Ok(removed)
    }

    /// The identities of the kinds in `types` that cover `service`, or every such identity when `service` is `None`.
    /// Sorted by rank, highest first; ties by user name, then record identifier, so the order is stable.
    pub fn identities_for(&self, service: Option<&ServiceIdentifier>, types: IdentityTypes) -> Vec<&CredentialIdentity> {
        let mut found: Vec<&CredentialIdentity> = self
            .identities
            .iter()
            .filter(|e| types.contains(e.kind))
            .filter(|e| service.map_or(true, |s| e.service.covers(s)))
            .collect();
        found.sort_by(|a, b| {
            b.rank.cmp(&a.rank).then_with(|| a.user.cmp(&b.user)).then_with(|| a.record_identifier.cmp(&b.record_identifier))
        });
        found
    }

    fn check_enabled(&self) -> Result<(), StoreError> {
        if self.enabled {
            Ok(())
        } else {
            Err(StoreError::Disabled)
        }
    }
}

fn same_record(a: &CredentialIdentity, b: &CredentialIdentity) -> bool {
    a.record_identifier == b.record_identifier && a.kind == b.kind
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(record: &str, user: &str, service: &str, kind: IdentifierType, rank: i64, key: IdentityKind) -> CredentialIdentity {
        CredentialIdentity {
            service: ServiceIdentifier::new(service, kind),
            user: user.to_string(),
            record_identifier: record.to_string(),
            rank,
            kind: key,
        }
    }

    fn password(record: &str, user: &str, domain: &str, rank: i64) -> CredentialIdentity {
        identity(record, user, domain, IdentifierType::Domain, rank, IdentityKind::Password)
    }

    fn url(text: &str) -> ServiceIdentifier {
        ServiceIdentifier::new(text, IdentifierType::Url)
    }

    #[test]
    fn host_drops_scheme_user_info_port_and_path_and_is_lower_cased() {
        assert_eq!(url("https://user@Login.Example.com:8443/path?x=1").host(), "login.example.com");
        assert_eq!(ServiceIdentifier::new("Example.COM", IdentifierType::Domain).host(), "example.com");
        assert_eq!(ServiceIdentifier::new("com.example.app", IdentifierType::App).host(), "");
    }

    #[test]
    fn a_domain_covers_its_subdomains_but_not_a_longer_name_that_ends_the_same_way() {
        let domain = ServiceIdentifier::new("example.com", IdentifierType::Domain);
        assert!(domain.covers(&url("https://login.example.com/session")));
        assert!(domain.covers(&url("https://example.com")));
        assert!(!domain.covers(&url("https://notexample.com")));
        assert!(!domain.covers(&url("https://example.com.evil.net")));
    }

    #[test]
    fn an_app_identifier_matches_only_the_same_app_exactly() {
        let app = ServiceIdentifier::new("com.example.app", IdentifierType::App);
        assert!(app.covers(&ServiceIdentifier::new("com.example.app", IdentifierType::App)));
        assert!(!app.covers(&ServiceIdentifier::new("com.example.app2", IdentifierType::App)));
        assert!(!app.covers(&url("https://example.com")));
    }

    #[test]
    fn save_replaces_an_identity_with_the_same_record_and_kind() {
        let mut store = IdentityStore::new();
        assert_eq!(store.save(&[password("r1", "anna", "example.com", 1)]), Ok(1));
        assert_eq!(store.save(&[password("r1", "anna.b", "example.com", 3)]), Ok(0));
        assert_eq!(store.len(), 1);
        let found = store.identities_for(None, IdentityTypes::all());
        assert_eq!(found[0].user, "anna.b");
        assert_eq!(found[0].rank, 3);
    }

    #[test]
    fn fetch_orders_by_rank_then_user_name() {
        let mut store = IdentityStore::new();
        store
            .save(&[
                password("r1", "bob", "example.com", 1),
                password("r2", "zoe", "example.com", 5),
                password("r3", "anna", "example.com", 5),
            ])
            .unwrap();
        let users: Vec<&str> = store.identities_for(None, IdentityTypes::all()).iter().map(|i| i.user.as_str()).collect();
        assert_eq!(users, vec!["anna", "zoe", "bob"]);
    }

    #[test]
    fn fetch_for_a_service_skips_identities_of_other_domains_and_unselected_kinds() {
        let mut store = IdentityStore::new();
        store
            .save(&[
                password("r1", "anna", "example.com", 1),
                password("r2", "other", "unrelated.org", 9),
                identity("r3", "pk", "example.com", IdentifierType::Domain, 2, IdentityKind::Passkey),
            ])
            .unwrap();
        let service = url("https://login.example.com/");
        let passwords_only = IdentityTypes { passkey: false, password: true, one_time_code: false };
        let found = store.identities_for(Some(&service), passwords_only);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].user, "anna");
        assert_eq!(store.identities_for(Some(&service), IdentityTypes::all()).len(), 2);
    }

    #[test]
    fn remove_counts_only_the_identities_it_actually_removes() {
        let mut store = IdentityStore::new();
        store.save(&[password("r1", "anna", "example.com", 1), password("r2", "bob", "example.com", 1)]).unwrap();
        let gone = store.remove(&[password("r1", "", "", 0), password("r9", "", "", 0)]);
        assert_eq!(gone, Ok(1));
        assert_eq!(store.len(), 1);
        assert_eq!(store.remove_all(), Ok(1));
        assert!(store.is_empty());
    }

    #[test]
    fn replace_swaps_the_whole_store_for_the_new_list() {
        let mut store = IdentityStore::new();
        store.save(&[password("old", "old", "example.com", 1)]).unwrap();
        store.replace(&[password("new1", "a", "example.com", 1), password("new2", "b", "example.com", 1)]).unwrap();
        let mut records: Vec<String> =
            store.identities_for(None, IdentityTypes::all()).iter().map(|i| i.record_identifier.clone()).collect();
        records.sort();
        assert_eq!(records, vec!["new1".to_string(), "new2".to_string()]);
    }

    #[test]
    fn a_disabled_store_refuses_changes_and_switching_it_off_clears_it() {
        let mut store = IdentityStore::new();
        store.save(&[password("r1", "anna", "example.com", 1), password("r2", "bob", "example.com", 1)]).unwrap();
        assert_eq!(store.set_enabled(false), 2);
        assert!(!store.is_enabled());
        assert!(store.is_empty());
        assert_eq!(store.save(&[password("r3", "c", "example.com", 1)]), Err(StoreError::Disabled));
        assert_eq!(store.remove_all(), Err(StoreError::Disabled));
        assert_eq!(store.set_enabled(true), 0);
        assert_eq!(store.save(&[password("r3", "c", "example.com", 1)]), Ok(1));
    }
}
