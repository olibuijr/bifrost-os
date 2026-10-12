//! File provider domains, provider errors and enumerator signals (`domain manager`, `provider domain` and
//! `provider error` parity). The desktop does not host provider extensions yet, so this is the in-process model
//! that an app or a future extension host uses: a registry of domains, the official error code vocabulary, and the
//! change signals an enumerator is waiting for. Items and their capabilities come from `bifrost_ui::file_provider`.
//!
//! Mirrors `provider domain` (identifier, displayName, pathRelativeToDocumentStorage, isReplicated, isHidden,
//! userEnabled, isDisconnected), `domain manager` (add, remove, getDomains, disconnect, reconnect,
//! signalEnumerator), `provider error code`, and cloud search: the string search request
//! (`query`, `desiredNumberOfResults`), the search enumerator's batches and `invalidate()`, and the opt-in switch
//! (`supportsStringSearchRequest`). Search runs over the items a provider has handed to
//! [`SearchIndex`]; nothing is queried remotely.
//!
//! Gaps: no extension process and no IPC, no reimport or materialized-item enumerators, no waitForChanges completion,
//! no known-folder replication, no testing modes, no download or upload progress, no remote query, and no
//! search-result observer callbacks. Errors are plain values: there is no user-info dictionary, and the error domain
//! is this crate's own name.

use std::fmt;

pub use bifrost_ui::file_provider::{ItemCapabilities, ItemError, ItemIdentifier, ProvidedItem, TypeAndCreator};

/// The error domain of [`ProviderError`]. Reverse-DNS, named for this crate.
pub const ERROR_DOMAIN: &str = "is.bifrost.provider-errors";

/// `provider error code`: the codes a provider reports. The names are the official case names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorCode {
    NoSuchItem,
    FilenameCollision,
    NotAuthenticated,
    ServerUnreachable,
    SyncAnchorExpired,
    PageExpired,
    DirectoryNotEmpty,
    ProviderNotFound,
    CannotSynchronize,
    DeletionRejected,
    VersionNoLongerAvailable,
    InsufficientQuota,
    UnsyncedEdits,
    NonEvictable,
    NonEvictableChildren,
    ExcludedFromSync,
    DomainDisabled,
    ProviderDomainNotFound,
    ProviderDomainTemporarilyUnavailable,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSuchItem => "noSuchItem",
            Self::FilenameCollision => "filenameCollision",
            Self::NotAuthenticated => "notAuthenticated",
            Self::ServerUnreachable => "serverUnreachable",
            Self::SyncAnchorExpired => "syncAnchorExpired",
            Self::PageExpired => "pageExpired",
            Self::DirectoryNotEmpty => "directoryNotEmpty",
            Self::ProviderNotFound => "providerNotFound",
            Self::CannotSynchronize => "cannotSynchronize",
            Self::DeletionRejected => "deletionRejected",
            Self::VersionNoLongerAvailable => "versionNoLongerAvailable",
            Self::InsufficientQuota => "insufficientQuota",
            Self::UnsyncedEdits => "unsyncedEdits",
            Self::NonEvictable => "nonEvictable",
            Self::NonEvictableChildren => "nonEvictableChildren",
            Self::ExcludedFromSync => "excludedFromSync",
            Self::DomainDisabled => "domainDisabled",
            Self::ProviderDomainNotFound => "providerDomainNotFound",
            Self::ProviderDomainTemporarilyUnavailable => "providerDomainTemporarilyUnavailable",
        }
    }

    /// The official descriptions say "try again later" for these codes.
    pub fn retry_later(self) -> bool {
        matches!(self, Self::ServerUnreachable | Self::CannotSynchronize | Self::ProviderDomainTemporarilyUnavailable)
    }

    /// The caller must restart the enumeration from the beginning.
    pub fn restarts_enumeration(self) -> bool {
        matches!(self, Self::SyncAnchorExpired | Self::PageExpired)
    }
}

/// A provider error: a code and, for item errors such as a missing or colliding item, the item it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderError {
    pub code: ErrorCode,
    pub item: Option<ItemIdentifier>,
}

impl ProviderError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, item: None }
    }

    pub fn for_item(code: ErrorCode, item: ItemIdentifier) -> Self {
        Self { code, item: Some(item) }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.item {
            Some(item) => write!(f, "{}: {}", self.code.as_str(), item),
            None => f.write_str(self.code.as_str()),
        }
    }
}

impl std::error::Error for ProviderError {}

/// `provider domain`: one provider's namespace as the system sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain {
    pub identifier: String,
    pub display_name: String,
    /// `pathRelativeToDocumentStorage`: a subfolder of the provider's shared container.
    pub storage_path: Option<String>,
    /// `isReplicated`: local copies are kept in sync with remote storage.
    pub replicated: bool,
    /// `isHidden`: the domain is not shown to the user.
    pub hidden: bool,
    /// `userEnabled`: the user has the domain switched on. On by default.
    pub user_enabled: bool,
    /// `isDisconnected`: the domain is registered, but its provider is not connected.
    pub disconnected: bool,
    generation: u64,
}

impl Domain {
    /// The identifier must be non-empty and hold no `/`; the display name must not be blank.
    pub fn new(identifier: &str, display_name: &str) -> Result<Self, RegistryError> {
        if identifier.is_empty() || identifier.contains('/') {
            return Err(RegistryError::Invalid("identifier"));
        }
        if display_name.trim().is_empty() {
            return Err(RegistryError::Invalid("displayName"));
        }
        Ok(Self {
            identifier: identifier.into(),
            display_name: display_name.into(),
            storage_path: None,
            replicated: false,
            hidden: false,
            user_enabled: true,
            disconnected: false,
            generation: 0,
        })
    }

    /// Sets `pathRelativeToDocumentStorage`. The path is relative and has no `..` component.
    pub fn with_storage_path(mut self, path: &str) -> Result<Self, RegistryError> {
        if path.is_empty() || path.starts_with('/') || path.split('/').any(|c| c == "..") {
            return Err(RegistryError::Invalid("pathRelativeToDocumentStorage"));
        }
        self.storage_path = Some(path.into());
        Ok(self)
    }

    /// How many enumerator signals this domain has received.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// Why a registry operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    /// A field is empty or malformed. The payload names the field.
    Invalid(&'static str),
    /// A domain with this identifier is already registered.
    Duplicate(String),
    /// A provider error, such as a domain that is not registered.
    Provider(ProviderError),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(field) => write!(f, "invalid {field}"),
            Self::Duplicate(id) => write!(f, "domain {id} is already registered"),
            Self::Provider(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<ProviderError> for RegistryError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

/// A pending `signalEnumerator(for:)` request: the enumerator for `container` in `domain` must run again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumeratorSignal {
    pub domain: String,
    pub container: ItemIdentifier,
    /// The domain's generation after the latest signal for this container.
    pub generation: u64,
}

/// `domain manager` over a set of domains: registers them, tracks their state and queues enumerator signals.
#[derive(Clone, Debug, Default)]
pub struct DomainRegistry {
    domains: Vec<Domain>,
    signals: Vec<EnumeratorSignal>,
}

fn not_found() -> RegistryError {
    RegistryError::Provider(ProviderError::new(ErrorCode::ProviderDomainNotFound))
}

impl DomainRegistry {
    /// `add(_:completionHandler:)`. Identifiers are unique.
    pub fn add(&mut self, domain: Domain) -> Result<(), RegistryError> {
        if self.get(&domain.identifier).is_some() {
            return Err(RegistryError::Duplicate(domain.identifier));
        }
        self.domains.push(domain);
        Ok(())
    }

    /// `remove(_:completionHandler:)`: drops the domain and any signal still pending for it.
    pub fn remove(&mut self, identifier: &str) -> Result<Domain, RegistryError> {
        let at = self.position(identifier)?;
        self.signals.retain(|s| s.domain != identifier);
        Ok(self.domains.remove(at))
    }

    /// `getDomains`: every domain, in the order they were added.
    pub fn all(&self) -> &[Domain] {
        &self.domains
    }

    pub fn get(&self, identifier: &str) -> Option<&Domain> {
        self.domains.iter().find(|d| d.identifier == identifier)
    }

    /// The domains the user can see, which are the ones that are not hidden.
    pub fn visible(&self) -> impl Iterator<Item = &Domain> {
        self.domains.iter().filter(|d| !d.hidden)
    }

    pub fn set_user_enabled(&mut self, identifier: &str, enabled: bool) -> Result<(), RegistryError> {
        self.domain_mut(identifier)?.user_enabled = enabled;
        Ok(())
    }

    /// `disconnect(reason:options:completionHandler:)`: the domain stays registered but stops taking signals.
    pub fn disconnect(&mut self, identifier: &str) -> Result<(), RegistryError> {
        self.domain_mut(identifier)?.disconnected = true;
        Ok(())
    }

    /// `reconnect(completionHandler:)`.
    pub fn reconnect(&mut self, identifier: &str) -> Result<(), RegistryError> {
        self.domain_mut(identifier)?.disconnected = false;
        Ok(())
    }

    /// `signalEnumerator(for:completionHandler:)`. Returns the domain's generation after the signal. A disconnected
    /// domain refuses the signal with a code that says to try again later. Repeated signals for one container
    /// coalesce into a single pending entry that carries the newest generation.
    pub fn signal_enumerator(&mut self, identifier: &str, container: &ItemIdentifier) -> Result<u64, RegistryError> {
        let domain = self.domain_mut(identifier)?;
        if domain.disconnected {
            return Err(ProviderError::new(ErrorCode::ProviderDomainTemporarilyUnavailable).into());
        }
        domain.generation += 1;
        let generation = domain.generation;
        match self.signals.iter_mut().find(|s| s.domain == identifier && &s.container == container) {
            Some(s) => s.generation = generation,
            None => self.signals.push(EnumeratorSignal {
                domain: identifier.into(),
                container: container.clone(),
                generation,
            }),
        }
        Ok(generation)
    }

    /// Hands out the pending signals in the order they were first queued, and clears the queue.
    pub fn take_signals(&mut self) -> Vec<EnumeratorSignal> {
        std::mem::take(&mut self.signals)
    }

    fn position(&self, identifier: &str) -> Result<usize, RegistryError> {
        self.domains.iter().position(|d| d.identifier == identifier).ok_or_else(not_found)
    }

    fn domain_mut(&mut self, identifier: &str) -> Result<&mut Domain, RegistryError> {
        let at = self.position(identifier)?;
        Ok(&mut self.domains[at])
    }
}

/// `string search request`: the plain text the person typed and how many results the system wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringSearchRequest {
    /// `query`: trimmed of surrounding whitespace. Never empty.
    pub query: String,
    /// `desiredNumberOfResults`: a hint. Zero means no limit.
    pub desired_number_of_results: usize,
}

impl StringSearchRequest {
    /// `None` when the query is blank once trimmed.
    pub fn new(query: &str) -> Option<Self> {
        let query = query.trim();
        (!query.is_empty()).then(|| Self { query: query.into(), desired_number_of_results: 0 })
    }

    pub fn with_desired_results(mut self, count: usize) -> Self {
        self.desired_number_of_results = count;
        self
    }
}

/// One item a search found: the domain it belongs to, its identifier and its filename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub domain: String,
    pub identifier: ItemIdentifier,
    pub filename: String,
}

/// `search enumerator`: hands out the hits of one search in batches. It is finished once every hit has
/// been handed out, or once [`SearchEnumerator::invalidate`] cancels it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchEnumerator {
    hits: Vec<SearchHit>,
    next: usize,
    invalidated: bool,
}

impl SearchEnumerator {
    /// The next batch of at most `max` hits. Empty once the search is finished.
    pub fn next_batch(&mut self, max: usize) -> Vec<SearchHit> {
        if self.invalidated {
            return Vec::new();
        }
        let end = self.next.saturating_add(max).min(self.hits.len());
        let batch = self.hits[self.next..end].to_vec();
        self.next = end;
        batch
    }

    /// `invalidate()`: cancels the search. No further hits are handed out.
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }

    /// True when the search was cancelled or every hit has been handed out.
    pub fn is_finished(&self) -> bool {
        self.invalidated || self.next >= self.hits.len()
    }
}

/// The items a provider's search can find, per domain. The provider fills it with [`SearchIndex::set_items`] and
/// must opt in with [`SearchIndex::with_string_search`], the same switch as `supportsStringSearchRequest`.
#[derive(Clone, Debug, Default)]
pub struct SearchIndex {
    supports_string_search: bool,
    items: Vec<(String, ProvidedItem)>,
}

impl SearchIndex {
    pub fn with_string_search(mut self, on: bool) -> Self {
        self.supports_string_search = on;
        self
    }

    /// Replaces the items of one domain. The items of other domains are kept.
    pub fn set_items(&mut self, domain: &str, items: Vec<ProvidedItem>) {
        self.items.retain(|(d, _)| d != domain);
        self.items.extend(items.into_iter().map(|item| (domain.to_string(), item)));
    }

    /// `searchEnumerator(for:)`. `None` when the provider did not opt in to string search.
    ///
    /// Only domains that are registered, enabled, visible and connected are searched. The well-known containers are
    /// not files, so they are never hits. A file matches when its filename contains the query, ignoring case. A
    /// `desired_number_of_results` above zero caps the hits.
    pub fn search_enumerator(&self, registry: &DomainRegistry, request: &StringSearchRequest) -> Option<SearchEnumerator> {
        if !self.supports_string_search {
            return None;
        }
        let needle = request.query.to_lowercase();
        let searched = |domain: &str| registry.get(domain).is_some_and(|d| d.user_enabled && !d.disconnected && !d.hidden);
        let containers = [ItemIdentifier::root_container(), ItemIdentifier::working_set(), ItemIdentifier::trash_container()];
        let mut hits: Vec<SearchHit> = self
            .items
            .iter()
            .filter(|(domain, item)| {
                searched(domain.as_str())
                    && !containers.contains(&item.identifier)
                    && item.filename().to_lowercase().contains(&needle)
            })
            .map(|(domain, item)| SearchHit {
                domain: domain.clone(),
                identifier: item.identifier.clone(),
                filename: item.filename().to_string(),
            })
            .collect();
        if request.desired_number_of_results > 0 {
            hits.truncate(request.desired_number_of_results);
        }
        Some(SearchEnumerator { hits, next: 0, invalidated: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(id: &str) -> Domain {
        Domain::new(id, "Test Domain").unwrap()
    }

    #[test]
    fn domains_are_unique_and_listed_in_insertion_order() {
        let mut r = DomainRegistry::default();
        r.add(domain("b")).unwrap();
        r.add(domain("a")).unwrap();
        assert_eq!(r.add(domain("b")), Err(RegistryError::Duplicate("b".into())));
        let ids: Vec<&str> = r.all().iter().map(|d| d.identifier.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        assert!(r.get("a").is_some());
        assert!(r.get("c").is_none());
    }

    #[test]
    fn domain_fields_are_validated() {
        assert_eq!(Domain::new("", "x"), Err(RegistryError::Invalid("identifier")));
        assert_eq!(Domain::new("a/b", "x"), Err(RegistryError::Invalid("identifier")));
        assert_eq!(Domain::new("x", "  "), Err(RegistryError::Invalid("displayName")));
        let d = domain("x");
        assert!(d.clone().with_storage_path("../escape").is_err());
        assert!(d.clone().with_storage_path("/absolute").is_err());
        assert_eq!(d.with_storage_path("sub/dir").unwrap().storage_path.as_deref(), Some("sub/dir"));
    }

    #[test]
    fn remove_returns_the_domain_and_drops_its_pending_signals() {
        let mut r = DomainRegistry::default();
        r.add(domain("a")).unwrap();
        r.add(domain("b")).unwrap();
        let root = ItemIdentifier::root_container();
        r.signal_enumerator("a", &root).unwrap();
        r.signal_enumerator("b", &root).unwrap();
        assert_eq!(r.remove("a").unwrap().identifier, "a");
        let left = r.take_signals();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].domain, "b");
        assert_eq!(r.remove("a"), Err(not_found()));
    }

    #[test]
    fn signals_coalesce_per_container_and_count_generations() {
        let mut r = DomainRegistry::default();
        r.add(domain("a")).unwrap();
        let root = ItemIdentifier::root_container();
        let docs = ItemIdentifier::new("docs").unwrap();
        assert_eq!(r.signal_enumerator("a", &root), Ok(1));
        assert_eq!(r.signal_enumerator("a", &docs), Ok(2));
        assert_eq!(r.signal_enumerator("a", &root), Ok(3));
        let drained = r.take_signals();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].container, root);
        assert_eq!(drained[0].generation, 3);
        assert!(r.take_signals().is_empty());
    }

    #[test]
    fn disconnected_domains_refuse_signals_with_a_retry_later_error() {
        let mut r = DomainRegistry::default();
        r.add(domain("a")).unwrap();
        r.disconnect("a").unwrap();
        let err = r.signal_enumerator("a", &ItemIdentifier::root_container()).unwrap_err();
        let RegistryError::Provider(p) = err else { panic!("expected a provider error") };
        assert_eq!(p.code, ErrorCode::ProviderDomainTemporarilyUnavailable);
        assert!(p.code.retry_later());
        r.reconnect("a").unwrap();
        assert_eq!(r.signal_enumerator("a", &ItemIdentifier::root_container()), Ok(1));
    }

    #[test]
    fn hidden_domains_are_not_visible_and_disabled_state_is_kept_per_domain() {
        let mut r = DomainRegistry::default();
        let mut hidden = domain("h");
        hidden.hidden = true;
        r.add(hidden).unwrap();
        r.add(domain("v")).unwrap();
        let visible: Vec<&str> = r.visible().map(|d| d.identifier.as_str()).collect();
        assert_eq!(visible, ["v"]);
        r.set_user_enabled("v", false).unwrap();
        assert!(!r.get("v").unwrap().user_enabled);
        assert!(r.get("h").unwrap().hidden);
        assert!(r.get("h").unwrap().user_enabled, "enabled by default");
    }

    #[test]
    fn error_codes_classify_retries_and_print_their_item() {
        assert!(ErrorCode::ServerUnreachable.retry_later());
        assert!(!ErrorCode::NoSuchItem.retry_later());
        assert!(ErrorCode::PageExpired.restarts_enumeration());
        assert!(!ErrorCode::InsufficientQuota.restarts_enumeration());
        let item = ItemIdentifier::new("doc-1").unwrap();
        assert_eq!(ProviderError::for_item(ErrorCode::FilenameCollision, item).to_string(), "filenameCollision: doc-1");
        assert_eq!(ProviderError::new(ErrorCode::NoSuchItem).to_string(), "noSuchItem");
    }

    fn named(id: &str, filename: &str) -> ProvidedItem {
        ProvidedItem::new(ItemIdentifier::new(id).unwrap(), ItemIdentifier::root_container(), filename).unwrap()
    }

    /// One enabled domain "a" with four files; "invoice" matches items 1, 3 and 4 in that order.
    fn searchable() -> (DomainRegistry, SearchIndex) {
        let mut registry = DomainRegistry::default();
        registry.add(domain("a")).unwrap();
        let mut index = SearchIndex::default().with_string_search(true);
        index.set_items(
            "a",
            vec![
                named("1", "Invoice-2026.pdf"),
                named("2", "notes.txt"),
                named("3", "invoice draft.md"),
                named("4", "Photos INVOICE.jpg"),
            ],
        );
        (registry, index)
    }

    fn names(hits: &[SearchHit]) -> Vec<&str> {
        hits.iter().map(|h| h.filename.as_str()).collect()
    }

    #[test]
    fn blank_queries_are_refused_and_the_rest_is_trimmed() {
        assert!(StringSearchRequest::new("   ").is_none());
        assert_eq!(StringSearchRequest::new("  Hi ").unwrap().query, "Hi");
    }

    #[test]
    fn string_search_is_off_until_the_provider_opts_in() {
        let (registry, index) = searchable();
        let request = StringSearchRequest::new("invoice").unwrap();
        assert!(index.with_string_search(false).search_enumerator(&registry, &request).is_none());
        assert!(searchable().1.search_enumerator(&registry, &request).is_some());
    }

    #[test]
    fn matching_ignores_case_and_keeps_the_provider_order() {
        let (registry, index) = searchable();
        let request = StringSearchRequest::new("INVOICE").unwrap();
        let mut search = index.search_enumerator(&registry, &request).unwrap();
        assert_eq!(names(&search.next_batch(10)), ["Invoice-2026.pdf", "invoice draft.md", "Photos INVOICE.jpg"]);
        assert!(search.is_finished());
    }

    #[test]
    fn disabled_disconnected_hidden_and_unregistered_domains_are_not_searched() {
        let mut registry = DomainRegistry::default();
        for id in ["a", "b", "c", "d"] {
            let mut d = domain(id);
            d.hidden = id == "d";
            registry.add(d).unwrap();
        }
        registry.set_user_enabled("b", false).unwrap();
        registry.disconnect("c").unwrap();
        let mut index = SearchIndex::default().with_string_search(true);
        for id in ["a", "b", "c", "d", "zz"] {
            index.set_items(id, vec![named("x", "report.txt")]);
        }
        let request = StringSearchRequest::new("report").unwrap();
        let mut search = index.search_enumerator(&registry, &request).unwrap();
        let hits = search.next_batch(10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].domain, "a");
    }

    #[test]
    fn well_known_containers_are_never_hits() {
        let mut registry = DomainRegistry::default();
        registry.add(domain("a")).unwrap();
        let mut index = SearchIndex::default().with_string_search(true);
        index.set_items("a", vec![named("root", "root-notes.txt"), named("9", "root-plan.txt")]);
        let request = StringSearchRequest::new("root").unwrap();
        let hits = index.search_enumerator(&registry, &request).unwrap().next_batch(10);
        let ids: Vec<&str> = hits.iter().map(|h| h.identifier.as_str()).collect();
        assert_eq!(ids, ["9"]);
    }

    #[test]
    fn the_desired_count_caps_hits_and_batches_drain_them() {
        let (registry, index) = searchable();
        let request = StringSearchRequest::new("invoice").unwrap().with_desired_results(2);
        let mut search = index.search_enumerator(&registry, &request).unwrap();
        assert_eq!(names(&search.next_batch(1)), ["Invoice-2026.pdf"]);
        assert!(!search.is_finished());
        assert_eq!(names(&search.next_batch(5)), ["invoice draft.md"]);
        assert!(search.is_finished());
        assert!(search.next_batch(5).is_empty());
    }

    #[test]
    fn invalidate_cancels_the_rest_of_the_search() {
        let (registry, index) = searchable();
        let request = StringSearchRequest::new("invoice").unwrap();
        let mut search = index.search_enumerator(&registry, &request).unwrap();
        assert_eq!(search.next_batch(1).len(), 1);
        search.invalidate();
        assert!(search.is_finished());
        assert!(search.next_batch(10).is_empty());
    }

    #[test]
    fn set_items_replaces_one_domain_and_keeps_the_others() {
        let mut registry = DomainRegistry::default();
        registry.add(domain("a")).unwrap();
        registry.add(domain("b")).unwrap();
        let mut index = SearchIndex::default().with_string_search(true);
        index.set_items("a", vec![named("1", "report.txt")]);
        index.set_items("b", vec![named("2", "report.txt")]);
        index.set_items("a", Vec::new());
        let request = StringSearchRequest::new("report").unwrap();
        let hits = index.search_enumerator(&registry, &request).unwrap().next_batch(10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].domain, "b");
    }
}
