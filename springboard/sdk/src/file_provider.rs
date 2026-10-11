//! File provider domains, provider errors and enumerator signals (`domain manager`, `provider domain` and
//! `provider error` parity). The desktop does not host provider extensions yet, so this is the in-process model
//! that an app or a future extension host uses: a registry of domains, the official error code vocabulary, and the
//! change signals an enumerator is waiting for. Items and their capabilities come from `bifrost_ui::file_provider`.
//!
//! Mirrors `provider domain` (identifier, displayName, pathRelativeToDocumentStorage, isReplicated, isHidden,
//! userEnabled, isDisconnected), `domain manager` (add, remove, getDomains, disconnect, reconnect,
//! signalEnumerator) and `provider error code`.
//!
//! Gaps: no extension process and no IPC, no reimport or materialized-item enumerators, no waitForChanges completion,
//! no known-folder replication, no testing modes, no download or upload progress, and no provider search. Errors are
//! plain values: there is no user-info dictionary, and the error domain is this crate's own name.

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
}
