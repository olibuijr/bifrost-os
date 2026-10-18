//! Core Search content indexing (`CSSearchableIndex`, `CSSearchableItem`, `CSSearchableItemAttributeSet`).
//!
//! An app indexes its own content (notes, documents, contacts) so the Search panel finds it without launching the
//! app. The shell keeps a bounded on-disk index per app and searches it; when the user opens a result it starts the app
//! and calls [`crate::App::continue_activity`] with a [`UserActivity`] of type [`ACTION_TYPE`]
//! (`CSSearchableItemActionType`) whose `unique_identifier` is the item's. See docs/bifrost-ui.md "Core Search".

pub use crate::proto::quicksearch::{ACTION_TYPE, MAX_ITEMS};
use crate::locale::Locale;
use crate::proto::quicksearch::{SearchableItem, DELETE_ALL, DELETE_DOMAINS, DELETE_IDS, MAX_DOMAIN, MAX_ID, MAX_VALUES};
use crate::proto::ToShell;
use crate::Ctx;
use bifrost_core::json::Json;

/// A custom attribute: an app-defined key and value. Its value is folded into the item's keywords when indexed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomAttribute {
    pub key: String,
    pub value: String,
    /// `CSCustomAttributeKey.isSearchable` inverted so the default stays searchable: true keeps the value out of the keywords
    pub unsearchable: bool,
}

/// `CSCustomAttributeKey`: an app-defined attribute name. A searchable key folds its values into the item's keywords.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CSCustomAttributeKey {
    pub key_name: String,
    pub searchable: bool,
}

impl CSCustomAttributeKey {
    /// `init(keyName:searchable:searchableByDefault:unique:multiValued:)` reduced to the part the index uses.
    pub fn new(key_name: &str, searchable: bool) -> Self {
        CSCustomAttributeKey { key_name: key_name.into(), searchable }
    }
}

/// A person the item relates to (a sender, an author, an attendee). The display name and the handles are
/// folded into the item's keywords when indexed, so a search for the name or the address finds the item.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub display_name: String,
    /// `handles`: addresses and numbers such as an e-mail address or a phone number
    pub handles: Vec<String>,
    /// `handleIdentifier`: the kind of contact property the handles are, such as "email"
    pub handle_identifier: String,
    /// `contactIdentifier`: the contact record the person came from; not searched
    pub contact_identifier: String,
}

/// Display text per language. The text for the current locale is used when the item is indexed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LocalizedString {
    /// (locale identifier, text), for example `("en_US", "Milk")` or `("is_IS", "Mjólk")`
    pub strings: Vec<(String, String)>,
}

impl LocalizedString {
    pub fn new(strings: &[(&str, &str)]) -> Self {
        LocalizedString { strings: strings.iter().map(|(l, t)| (l.to_string(), t.to_string())).collect() }
    }

    /// The entry whose identifier equals the locale's, else the first entry in the same language,
    /// else the first entry. `None` when the table is empty.
    pub fn localized_string(&self, locale: &Locale) -> Option<&str> {
        let id = locale.identifier();
        let exact = self.strings.iter().find(|(l, _)| *l == id);
        let same_language = || self.strings.iter().find(|(l, _)| Locale::new(l).language == locale.language);
        exact.or_else(same_language).or(self.strings.first()).map(|(_, t)| t.as_str())
    }
}

/// `CSSearchableItemAttributeSet`: the searchable attributes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CSSearchableItemAttributeSet {
    pub title: String,
    pub content_description: String,
    pub keywords: Vec<String>,
    /// `thumbnailURL`: a path inside the app's data dir (kept with the item; the panel does not draw it yet)
    pub thumbnail_path: String,
    /// `contentURL`
    pub content_url: String,
    /// `customAttributes`: app-defined values, searched as keywords
    pub custom: Vec<CustomAttribute>,
    /// `persons`: people related to the item, searched by name and handle
    pub persons: Vec<Person>,
    /// Title per language. When it has a text for the current locale, that text is indexed instead of `title`.
    pub title_localized: Option<LocalizedString>,
    /// `contentType`: a type identifier such as `public.text`, searched as a keyword
    pub content_type: String,
    /// `contentCreationDate`: seconds since the Unix epoch (UTC), searched as `YYYY-MM-DD`
    pub content_creation_date: Option<i64>,
    /// `contentModificationDate`: seconds since the Unix epoch (UTC), searched as `YYYY-MM-DD`
    pub content_modification_date: Option<i64>,
    /// `authorNames`, searched by name
    pub author_names: Vec<String>,
    /// `displayName`: shown instead of `title` when not empty (a localized title still wins)
    pub display_name: String,
    /// `rankingHint`: a higher value ranks the item higher; kept with the item for the Search panel
    pub ranking_hint: Option<i32>,
}

/// `YYYY-MM-DD` of a Unix timestamp (proleptic Gregorian, UTC).
fn iso_date(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl CSSearchableItemAttributeSet {
    pub fn new(title: &str) -> Self {
        CSSearchableItemAttributeSet { title: title.into(), ..Default::default() }
    }

    pub fn custom_attribute(mut self, key: &str, value: &str) -> Self {
        self.custom.push(CustomAttribute { key: key.into(), value: value.into(), unsearchable: false });
        self
    }
    /// `setValue(_:forCustomKey:)`: the value is searched only when the key is searchable.
    pub fn custom_value(mut self, key: &CSCustomAttributeKey, value: &str) -> Self {
        self.custom.push(CustomAttribute { key: key.key_name.clone(), value: value.into(), unsearchable: !key.searchable });
        self
    }
    pub fn content_type(mut self, v: &str) -> Self {
        self.content_type = v.into();
        self
    }
    pub fn content_creation_date(mut self, secs: i64) -> Self {
        self.content_creation_date = Some(secs);
        self
    }
    pub fn content_modification_date(mut self, secs: i64) -> Self {
        self.content_modification_date = Some(secs);
        self
    }
    pub fn author_names(mut self, v: &[&str]) -> Self {
        self.author_names = v.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn display_name(mut self, v: &str) -> Self {
        self.display_name = v.into();
        self
    }
    pub fn ranking_hint(mut self, v: i32) -> Self {
        self.ranking_hint = Some(v);
        self
    }
    pub fn person(mut self, person: Person) -> Self {
        self.persons.push(person);
        self
    }
    pub fn localized_title(mut self, title: LocalizedString) -> Self {
        self.title_localized = Some(title);
        self
    }
    pub fn content_description(mut self, v: &str) -> Self {
        self.content_description = v.into();
        self
    }
    pub fn keywords(mut self, v: &[&str]) -> Self {
        self.keywords = v.iter().map(|s| s.to_string()).collect();
        self
    }
    pub fn thumbnail_path(mut self, v: &str) -> Self {
        self.thumbnail_path = v.into();
        self
    }
    pub fn content_url(mut self, v: &str) -> Self {
        self.content_url = v.into();
        self
    }

    /// The title and keywords sent to the shell for `locale`. The localized title wins when it has a non-empty text
    /// for the locale. Custom values, person names and handles are appended to the keywords, skipping empty and
    /// repeated values; the wire codec keeps only the first `MAX_KEYWORDS`, so explicit keywords come first.
    fn wire_text(&self, locale: &Locale) -> (String, Vec<String>) {
        let base = if self.display_name.is_empty() { self.title.as_str() } else { self.display_name.as_str() };
        let title = self.title_localized.as_ref().and_then(|t| t.localized_string(locale)).filter(|t| !t.is_empty()).unwrap_or(base).to_string();
        let mut keywords = self.keywords.clone();
        let dates = [self.content_creation_date, self.content_modification_date].map(|d| d.map(iso_date).unwrap_or_default());
        let extra = self
            .custom
            .iter()
            .filter(|c| !c.unsearchable)
            .map(|c| c.value.as_str())
            .chain(self.author_names.iter().map(|a| a.as_str()))
            .chain(std::iter::once(self.content_type.as_str()))
            .chain(dates.iter().map(|d| d.as_str()))
            .chain(self.persons.iter().flat_map(|p| std::iter::once(p.display_name.as_str()).chain(p.handles.iter().map(|h| h.as_str()))));
        for text in extra {
            if !text.is_empty() && !keywords.iter().any(|k| k.as_str() == text) {
                keywords.push(text.to_string());
            }
        }
        (title, keywords)
    }
}

/// `CSSearchableItem`. Indexing an item with an existing `unique_identifier` replaces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CSSearchableItem {
    pub unique_identifier: String,
    /// `domainIdentifier`: lets [`CSSearchableIndex::delete_searchable_items_with_domain_identifiers`] drop a group
    pub domain_identifier: String,
    pub attribute_set: CSSearchableItemAttributeSet,
    /// `expirationDate`: seconds since the Unix epoch after which the item is dropped; `None` keeps it
    pub expiration_date: Option<i64>,
    /// `hideInSpotlight`: the item stays indexed but the shell leaves it out of search results
    pub hide_in_spotlight: bool,
}

impl CSSearchableItem {
    pub fn new(unique_identifier: &str, domain_identifier: &str, attribute_set: CSSearchableItemAttributeSet) -> Self {
        CSSearchableItem {
            unique_identifier: unique_identifier.into(),
            domain_identifier: domain_identifier.into(),
            attribute_set,
            expiration_date: None,
            hide_in_spotlight: false,
        }
    }

    /// Sets `expirationDate`.
    pub fn expiration_date(mut self, secs: i64) -> Self {
        self.expiration_date = Some(secs);
        self
    }

    /// Sets `hideInSpotlight`: the item is indexed but not shown in search results.
    pub fn hide_in_spotlight(mut self) -> Self {
        self.hide_in_spotlight = true;
        self
    }

    /// Whether the item has expired at `now` (Unix seconds). An item without an expiration date never does.
    pub fn is_expired(&self, now: i64) -> bool {
        self.expiration_date.is_some_and(|at| at <= now)
    }
}

/// Why an item was refused before it was sent to the shell. Refused items are not indexed; the rest of
/// the batch still is. The shell's own failures (full index, unavailable service) are not reported back yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexError {
    /// The identifier is empty, so the item could never be replaced or deleted.
    EmptyIdentifier,
    /// The identifier is longer than [`MAX_ID`] bytes.
    IdentifierTooLong,
    /// The domain identifier is longer than [`MAX_DOMAIN`] bytes, so deleting by domain would not match the item.
    DomainTooLong,
}

impl IndexError {
    /// The error domain name, for logs.
    pub const DOMAIN: &'static str = "bifrost.quicksearch.index";

    /// The check run on every item by [`CSSearchableIndex::index_checked`].
    pub fn check(item: &CSSearchableItem) -> Result<(), IndexError> {
        if item.unique_identifier.is_empty() {
            return Err(IndexError::EmptyIdentifier);
        }
        if item.unique_identifier.len() > MAX_ID {
            return Err(IndexError::IdentifierTooLong);
        }
        if item.domain_identifier.len() > MAX_DOMAIN {
            return Err(IndexError::DomainTooLong);
        }
        Ok(())
    }

    /// The error code within [`IndexError::DOMAIN`]. Stable: logs and tests may rely on it.
    pub fn code(self) -> u32 {
        match self {
            IndexError::EmptyIdentifier => 1,
            IndexError::IdentifierTooLong => 2,
            IndexError::DomainTooLong => 3,
        }
    }
}

/// `NativeUserActivity` as delivered by `App::continue_activity`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserActivity {
    /// [`ACTION_TYPE`] for an item opened from Search
    pub activity_type: String,
    /// `CSSearchableItemActivityIdentifier`: the item's `unique_identifier`
    pub unique_identifier: String,
    pub domain_identifier: String,
}

/// `CSSearchableIndex.default()`.
#[derive(Clone, Copy, Debug, Default)]
pub struct CSSearchableIndex;

impl CSSearchableIndex {
    pub fn default_index() -> Self {
        CSSearchableIndex
    }

    /// `indexSearchableItems(_:)`. Items without an identifier are skipped. The shell keeps at most 1000 items per app
    /// (the oldest are dropped first) and cuts long text, so index a title, a short description and a few keywords.
    pub fn index_searchable_items(&self, ctx: &Ctx, items: &[CSSearchableItem]) {
        self.index_checked(ctx, items);
    }

    /// `indexSearchableItems(_:)` with the refusals reported. Returns the position of each refused item and why; the
    /// other items are sent. An identifier or domain longer than the wire cap is refused rather than cut, because a
    /// cut identifier would never match the one the app later deletes or continues by.
    pub fn index_checked(&self, ctx: &Ctx, items: &[CSSearchableItem]) -> Vec<(usize, IndexError)> {
        let locale = Locale::current();
        let mut refused = Vec::new();
        let mut wire: Vec<SearchableItem> = Vec::new();
        for (pos, i) in items.iter().enumerate() {
            if let Err(e) = IndexError::check(i) {
                refused.push((pos, e));
                continue;
            }
            let (title, keywords) = i.attribute_set.wire_text(&locale);
            wire.push(SearchableItem {
                unique_id: i.unique_identifier.clone(),
                domain: i.domain_identifier.clone(),
                title,
                description: i.attribute_set.content_description.clone(),
                keywords,
                thumbnail: i.attribute_set.thumbnail_path.clone(),
                content_url: i.attribute_set.content_url.clone(),
            });
        }
        for chunk in wire.chunks(MAX_ITEMS) {
            ctx.send_to_shell(ToShell::QuickSearchIndex(chunk.to_vec()));
        }
        // The shell drops an item at its expiry through the search-index service, sent after the batch so the item
        // exists when the expiry is set.
        for i in items.iter().filter(|i| i.expiration_date.is_some() && IndexError::check(i).is_ok()) {
            let at = i.expiration_date.unwrap_or(0).max(0) as u64;
            crate::search_ranking::expire(ctx, &i.unique_identifier, Some(at));
        }
        refused
    }

    /// `deleteSearchableItems(withIdentifiers:)`.
    pub fn delete_searchable_items(&self, ctx: &Ctx, identifiers: &[&str]) {
        Self::delete(ctx, DELETE_IDS, identifiers);
    }

    /// `deleteSearchableItems(withDomainIdentifiers:)`.
    pub fn delete_searchable_items_with_domain_identifiers(&self, ctx: &Ctx, domains: &[&str]) {
        Self::delete(ctx, DELETE_DOMAINS, domains);
    }

    /// `deleteAllSearchableItems`.
    pub fn delete_all_searchable_items(&self, ctx: &Ctx) {
        ctx.send_to_shell(ToShell::QuickSearchDelete { kind: DELETE_ALL, values: Vec::new() });
    }

    fn delete(ctx: &Ctx, kind: u8, values: &[&str]) {
        for chunk in values.chunks(MAX_VALUES) {
            ctx.send_to_shell(ToShell::QuickSearchDelete { kind, values: chunk.iter().map(|s| s.to_string()).collect() });
        }
    }
}

/// `CSSearchableIndexDelegate`: the app's answers when it reindexes its own content (see
/// [`CSSearchableIndex::reindex_all_with`] and [`CSSearchableIndex::reindex_identifiers_with`]). Each method has an empty
/// default, so a delegate implements only what it can answer.
///
/// Gaps: the shell never asks an app to reindex (no request message from the shell exists), so the app chooses when to
/// call these. The data-protection and throttling callbacks are not present.
pub trait CSSearchableIndexDelegate {
    /// `searchableIndex(_:reindexAllSearchableItemsWithAcknowledgementHandler:)`: the items to index again after the
    /// index is cleared.
    fn reindex_all_searchable_items(&self, _ctx: &Ctx) -> Vec<CSSearchableItem> {
        Vec::new()
    }

    /// `searchableItems(forIdentifiers:searchableItemsHandler:)`: the items for `identifiers`. An identifier the app does
    /// not hold yields nothing.
    fn searchable_items_for(&self, _ctx: &Ctx, _identifiers: &[&str]) -> Vec<CSSearchableItem> {
        Vec::new()
    }
}

impl CSSearchableIndex {
    /// `searchableIndex(_:reindexAllSearchableItemsWithAcknowledgementHandler:)`, run by the app: clears the index
    /// (`deleteAllSearchableItems`) and indexes what the delegate returns. Refused items are reported as in
    /// [`CSSearchableIndex::index_checked`].
    pub fn reindex_all_with(&self, ctx: &Ctx, delegate: &dyn CSSearchableIndexDelegate) -> Vec<(usize, IndexError)> {
        self.delete_all_searchable_items(ctx);
        let items = delegate.reindex_all_searchable_items(ctx);
        self.index_checked(ctx, &items)
    }

    /// `searchableIndex(_:reindexSearchableItemsWithIdentifiers:acknowledgementHandler:)`, run by the app: indexes the
    /// items the delegate returns for `identifiers`. The other indexed items are kept.
    pub fn reindex_identifiers_with(&self, ctx: &Ctx, identifiers: &[&str], delegate: &dyn CSSearchableIndexDelegate) -> Vec<(usize, IndexError)> {
        let items = delegate.searchable_items_for(ctx, identifiers);
        self.index_checked(ctx, &items)
    }
}

/// `CSIndexExtensionRequestHandler`: the app-side answers to a regeneration request for its index. The shell does not
/// send these requests yet, so the app calls [`IndexExtensionRequests::reindex_all`] or
/// [`IndexExtensionRequests::reindex_identifiers`] itself. The return value is the acknowledgement: the refused items
/// by position and reason, empty when every item was sent.
pub struct IndexExtensionRequests<'d> {
    delegate: &'d dyn CSSearchableIndexDelegate,
}

impl<'d> IndexExtensionRequests<'d> {
    /// Handles requests for `delegate`'s items.
    pub fn new(delegate: &'d dyn CSSearchableIndexDelegate) -> Self {
        IndexExtensionRequests { delegate }
    }

    /// `searchableIndex(_:reindexAllSearchableItemsWithAcknowledgementHandler:)`: clears the index and indexes again.
    pub fn reindex_all(&self, ctx: &Ctx) -> Vec<(usize, IndexError)> {
        CSSearchableIndex::default_index().reindex_all_with(ctx, self.delegate)
    }

    /// `searchableIndex(_:reindexSearchableItemsWithIdentifiers:acknowledgementHandler:)`: indexes the items for
    /// `identifiers` and keeps the others.
    pub fn reindex_identifiers(&self, ctx: &Ctx, identifiers: &[&str]) -> Vec<(usize, IndexError)> {
        CSSearchableIndex::default_index().reindex_identifiers_with(ctx, identifiers, self.delegate)
    }
}

/// An item the app's own query found in the shell's index (`CSSearchableItem` identifiers and title).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundItem {
    pub unique_identifier: String,
    pub domain_identifier: String,
    pub title: String,
    /// the `uttype` identifier of the item's file, when the shell knows it
    pub content_type: Option<String>,
}

/// `CSSearchQuery` over the shell's index: runs `query_string` over the items this app indexed. The shell answers with
/// the app's own items only, ranked, every word must match ("pdf" and "last week" filter; see
/// `springboard/src/shell/services/search_ranking.rs`).
///
/// ```ignore
/// let id = CSSearchQuery::new("milk").limit(5).start(ctx)?;
/// // in App::service_reply, for that id:
/// //   let items = quicksearch::found_items(&reply?);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CSSearchQuery {
    query_string: String,
    limit: usize,
    /// the named index to search, or None for the default index
    index: Option<String>,
}

/// The most items one query asks the shell for (the shell caps it too).
pub const SEARCH_QUERY_DEFAULT_LIMIT: usize = 10;

impl CSSearchQuery {
    pub fn new(query_string: &str) -> Self {
        CSSearchQuery { query_string: query_string.to_string(), limit: SEARCH_QUERY_DEFAULT_LIMIT, index: None }
    }

    /// The most items to return.
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(1);
        self
    }

    /// Searches the named index `name` of this app (see [`NamedSearchableIndex`]) instead of the default one.
    pub fn in_index(mut self, name: &str) -> Self {
        self.index = Some(name.to_string());
        self
    }

    /// Sends the query to the shell and returns its request id. A blank query string is refused with
    /// `SearchQueryError::EmptyQueryString`, as the query model does.
    pub fn start(&self, ctx: &Ctx) -> Result<u32, SearchQueryError> {
        if self.query_string.trim().is_empty() {
            return Err(SearchQueryError::EmptyQueryString);
        }
        Ok(match &self.index {
            Some(name) => crate::search_ranking::search_named(ctx, name, &self.query_string, self.limit),
            None => crate::search_ranking::search(ctx, &self.query_string, self.limit),
        })
    }
}

/// `CSSearchableIndex(name:protectionClass:)`: a named index of this app, kept apart from the default one. Its items are
/// searched with [`CSSearchQuery::in_index`]. Other apps do not see them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedSearchableIndex {
    description: IndexDescription,
}

impl NamedSearchableIndex {
    /// The named index `name` with its protection class. Refuses a name the shell cannot use.
    pub fn new(name: &str, protection: Option<ProtectionClass>) -> Result<Self, IndexNameError> {
        let mut description = IndexDescription::new(name)?;
        description.protection_class = protection;
        Ok(NamedSearchableIndex { description })
    }

    /// The index name.
    pub fn name(&self) -> &str {
        &self.description.name
    }

    /// `indexSearchableItems` on this index. Items that fail [`IndexError::check`] are not sent; their positions and
    /// reasons are returned. The shell keeps the expiry of each item it is sent.
    pub fn index_searchable_items(&self, ctx: &Ctx, items: &[CSSearchableItem]) -> Vec<(usize, IndexError)> {
        let locale = Locale::current();
        let mut refused = Vec::new();
        let mut wire: Vec<Json> = Vec::new();
        for (pos, i) in items.iter().enumerate() {
            if let Err(e) = IndexError::check(i) {
                refused.push((pos, e));
                continue;
            }
            let (title, keywords) = i.attribute_set.wire_text(&locale);
            let mut obj = vec![
                ("id".to_string(), Json::Str(i.unique_identifier.clone())),
                ("domain".to_string(), Json::Str(i.domain_identifier.clone())),
                ("title".to_string(), Json::Str(title)),
                ("desc".to_string(), Json::Str(i.attribute_set.content_description.clone())),
                ("keywords".to_string(), Json::Arr(keywords.into_iter().map(Json::Str).collect())),
            ];
            if let Some(at) = i.expiration_date {
                obj.push(("expires".to_string(), Json::Num(at.max(0) as f64)));
            }
            wire.push(Json::Obj(obj));
        }
        for chunk in wire.chunks(MAX_ITEMS) {
            let _ = crate::search_ranking::index_named(ctx, self.name(), self.description.protection_class, chunk.to_vec());
        }
        refused
    }

    /// `deleteSearchableItems(withIdentifiers:)` on this index.
    pub fn delete_searchable_items(&self, ctx: &Ctx, identifiers: &[&str]) {
        let values: Vec<String> = identifiers.iter().map(|s| s.to_string()).collect();
        let _ = crate::search_ranking::delete_named(ctx, self.name(), "ids", &values);
    }

    /// `deleteSearchableItems(withDomainIdentifiers:)` on this index.
    pub fn delete_searchable_items_with_domain_identifiers(&self, ctx: &Ctx, domains: &[&str]) {
        let values: Vec<String> = domains.iter().map(|s| s.to_string()).collect();
        let _ = crate::search_ranking::delete_named(ctx, self.name(), "domains", &values);
    }

    /// `deleteAllSearchableItems` on this index; the index stays.
    pub fn delete_all_searchable_items(&self, ctx: &Ctx) {
        let _ = crate::search_ranking::delete_named(ctx, self.name(), "all", &[]);
    }

    /// `deleteIndex`: removes this index and its items.
    pub fn delete_index(&self, ctx: &Ctx) {
        let _ = crate::search_ranking::delete_named_index(ctx, self.name());
    }
}

/// The items of a [`CSSearchQuery`] reply, best match first. A reply that is not a list gives none.
pub fn found_items(reply: &bifrost_core::json::Json) -> Vec<FoundItem> {
    crate::search_ranking::parse_hits(reply)
        .into_iter()
        .map(|h| FoundItem { unique_identifier: h.id, domain_identifier: h.domain, title: h.title, content_type: h.kind })
        .collect()
}

/// A reindex request from the shell (`CSIndexExtensionRequestHandler`): all the app's content, or the items of some
/// identifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexRequest {
    ReindexAll,
    ReindexIdentifiers(Vec<String>),
}

/// The shell's answer to an acknowledged index batch (see [`CSSearchableIndex::index_searchable_items_acked`]).
#[derive(Debug)]
pub struct IndexAck {
    request: u32,
}

impl IndexAck {
    /// The outcome once the shell has answered, `None` while it has not. The answer is taken once. A stored batch gives
    /// the count the shell kept; an error answer from the shell gives `RemoteConnectionError`.
    pub fn outcome(&self) -> Option<Result<usize, crate::search_index_batch::CSIndexError>> {
        use crate::search_index_batch::{CSIndexError, CSIndexErrorCode};
        let reply = crate::service::take_reply(self.request)?;
        Some(match reply {
            Ok(json) => Ok(json.get("stored").and_then(Json::as_u64).unwrap_or(0) as usize),
            Err(_) => Err(CSIndexError { code: CSIndexErrorCode::RemoteConnectionError, item_position: None }),
        })
    }
}

impl CSSearchableIndex {
    /// `indexSearchableItems(_:completionHandler:)` with the shell's acknowledgement. The items that pass
    /// [`IndexError::check`] go to the shell in one `index` call; the refused ones are returned with their reasons. The
    /// [`IndexAck`] carries the shell's answer once it has come back.
    pub fn index_searchable_items_acked(&self, ctx: &Ctx, items: &[CSSearchableItem]) -> (IndexAck, Vec<(usize, IndexError)>) {
        let locale = Locale::current();
        let mut refused = Vec::new();
        let mut wire: Vec<Json> = Vec::new();
        for (pos, i) in items.iter().enumerate() {
            if let Err(e) = IndexError::check(i) {
                refused.push((pos, e));
                continue;
            }
            let (title, keywords) = i.attribute_set.wire_text(&locale);
            let mut obj = vec![
                ("id".to_string(), Json::Str(i.unique_identifier.clone())),
                ("domain".to_string(), Json::Str(i.domain_identifier.clone())),
                ("title".to_string(), Json::Str(title)),
                ("desc".to_string(), Json::Str(i.attribute_set.content_description.clone())),
                ("keywords".to_string(), Json::Arr(keywords.into_iter().map(Json::Str).collect())),
                ("thumb".to_string(), Json::Str(i.attribute_set.thumbnail_path.clone())),
                ("url".to_string(), Json::Str(i.attribute_set.content_url.clone())),
            ];
            if let Some(at) = i.expiration_date {
                obj.push(("expires".to_string(), Json::Num(at.max(0) as f64)));
            }
            wire.push(Json::Obj(obj));
        }
        let request = crate::service::call(ctx, crate::search_ranking::SERVICE, "index", &Json::Obj(vec![("items".into(), Json::Arr(wire))]));
        (IndexAck { request }, refused)
    }
}

/// `CSSearchableIndexDescription` and the protection class of a named index (`bifrost_ui::searchindex`).
pub use bifrost_ui::searchindex::{IndexDescription, NameError as IndexNameError, ProtectionClass};

/// Per-app, per-name index stores (`bifrost_ui::searchstore`) and user queries with suggestions (`bifrost_ui::usersearch`).
pub use bifrost_ui::searchstore::{BatchError as IndexBatchError, IndexStore, StoreSet, StoredItem};
pub use bifrost_ui::usersearch::{run_user_query, Suggestion as CSSuggestion, SuggestionKind as CSSuggestionKind, UserQueryContext as CSUserQueryContext, UserQueryResults};

/// The query model (`bifrost_ui::searchquery`): a query string, the fields it searches and its context.
pub use bifrost_ui::searchquery::{Context as SearchQueryContext, Error as SearchQueryError, Field as SearchQueryField, Hit as SearchQueryHit, Query as SearchQuery};

/// Runs `query` over the items the app indexed (`CSSearchQuery` with the app's own content). The title is the one
/// for the current locale and the keywords are the ones the shell indexes (custom values and persons included), so a
/// query finds an item here exactly when the Search panel would. Returns the matching items, best match first.
///
/// This searches only the items passed in, which the app holds itself. The shell's on-device index is not queried.
pub fn search_items<'a>(query: &SearchQuery, items: &'a [CSSearchableItem]) -> Result<Vec<&'a CSSearchableItem>, SearchQueryError> {
    let locale = Locale::current();
    let records: Vec<bifrost_ui::searchquery::Record> = items
        .iter()
        .map(|i| {
            let (title, keywords) = i.attribute_set.wire_text(&locale);
            bifrost_ui::searchquery::Record { title, description: i.attribute_set.content_description.clone(), keywords }
        })
        .collect();
    let hits = bifrost_ui::searchquery::run(query, &records)?;
    Ok(hits.into_iter().map(|h| &items[h.index]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Configure;

    #[test]
    fn an_item_expires_at_its_expiration_date() {
        let plain = CSSearchableItem::new("a", "d", CSSearchableItemAttributeSet::new("Milk"));
        assert!(!plain.is_expired(i64::MAX));
        let timed = plain.expiration_date(100);
        assert!(!timed.is_expired(99));
        assert!(timed.is_expired(100));
    }

    #[test]
    fn search_items_finds_by_title_description_and_keywords() {
        let items = [
            CSSearchableItem::new("n1", "notes", CSSearchableItemAttributeSet::new("Milk").content_description("oat")),
            CSSearchableItem::new("n2", "notes", CSSearchableItemAttributeSet::new("Bread").keywords(&["shop"])),
            CSSearchableItem::new("n3", "notes", CSSearchableItemAttributeSet::new("Tea").custom_attribute("status", "shop")),
        ];
        let ids = |q: &str| -> Vec<String> { search_items(&SearchQuery::new(q), &items).unwrap().iter().map(|i| i.unique_identifier.clone()).collect() };
        assert_eq!(ids("oat"), ["n1"]);
        assert_eq!(ids("shop"), ["n2", "n3"]);
        assert_eq!(ids("mil"), ["n1"]);
    }

    #[test]
    fn search_items_applies_filter_queries_and_reports_refused_queries() {
        let items = [
            CSSearchableItem::new("a", "notes", CSSearchableItemAttributeSet::new("Milk").keywords(&["shop"])),
            CSSearchableItem::new("b", "notes", CSSearchableItemAttributeSet::new("Milk").keywords(&["home"])),
        ];
        let ctx = SearchQueryContext { filter_queries: vec!["home".into()], ..SearchQueryContext::default() };
        let found = search_items(&SearchQuery::with_context("milk", ctx), &items).unwrap();
        assert_eq!(found.iter().map(|i| i.unique_identifier.as_str()).collect::<Vec<_>>(), ["b"]);
        assert_eq!(search_items(&SearchQuery::new("  "), &items).unwrap_err(), SearchQueryError::EmptyQueryString);
        assert_eq!(SearchQueryError::NoFields.code(), 2);
    }

    use bifrost_render::font::Font;
    use bifrost_render::ui::Ui;

    fn ctx() -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
    }

    #[test]
    fn extended_attributes_round_trip_and_are_searched() {
        let secret = CSCustomAttributeKey::new("pin", false);
        let tag = CSCustomAttributeKey::new("tag", true);
        let set = CSSearchableItemAttributeSet::new("Report")
            .display_name("Q3 Report")
            .content_type("public.text")
            .content_creation_date(1_700_000_000)
            .content_modification_date(0)
            .author_names(&["Ada Lovelace"])
            .ranking_hint(5)
            .custom_value(&tag, "finance")
            .custom_value(&secret, "9999");
        assert_eq!(set.ranking_hint, Some(5));
        assert_eq!(set.content_creation_date, Some(1_700_000_000));
        let items = [CSSearchableItem::new("r", "docs", set), CSSearchableItem::new("o", "docs", CSSearchableItemAttributeSet::new("Other"))];
        let ids = |q: &str| -> Vec<String> { search_items(&SearchQuery::new(q), &items).unwrap().iter().map(|i| i.unique_identifier.clone()).collect() };
        assert_eq!(ids("lovelace"), ["r"]);
        assert_eq!(ids("finance"), ["r"]);
        assert_eq!(ids("public.text"), ["r"]);
        assert_eq!(ids("2023-11-14"), ["r"]);
        assert_eq!(ids("1970-01-01"), ["r"]);
        assert_eq!(ids("q3"), ["r"]);
        assert!(ids("9999").is_empty());
    }

    #[test]
    fn iso_date_handles_epoch_and_leap_days() {
        assert_eq!(iso_date(0), "1970-01-01");
        assert_eq!(iso_date(951_782_400), "2000-02-29");
        assert_eq!(iso_date(-86_400), "1969-12-31");
    }

    fn item(id: &str) -> CSSearchableItem {
        CSSearchableItem::new(id, "notes", CSSearchableItemAttributeSet::new("Milk").content_description("oat").keywords(&["shop"]).thumbnail_path("t.png").content_url("file:///n"))
    }

    #[test]
    fn index_sends_the_flattened_attributes_in_chunks() {
        let c = ctx();
        let items: Vec<_> = (0..MAX_ITEMS + 2).map(|i| item(&format!("n{i}"))).collect();
        CSSearchableIndex::default_index().index_searchable_items(&c, &items);
        let sent = c.take_sent();
        assert_eq!(sent.len(), 2);
        let ToShell::QuickSearchIndex(first) = &sent[0] else { panic!() };
        assert_eq!(first.len(), MAX_ITEMS);
        assert_eq!(first[0], SearchableItem { unique_id: "n0".into(), domain: "notes".into(), title: "Milk".into(), description: "oat".into(), keywords: vec!["shop".into()], thumbnail: "t.png".into(), content_url: "file:///n".into() });
    }

    #[test]
    fn items_without_an_identifier_are_skipped() {
        let c = ctx();
        CSSearchableIndex.index_searchable_items(&c, &[item("")]);
        assert!(c.take_sent().is_empty());
    }

    #[test]
    fn deletes_by_identifier_domain_and_all() {
        let c = ctx();
        let ix = CSSearchableIndex::default_index();
        ix.delete_searchable_items(&c, &["a", "b"]);
        ix.delete_searchable_items_with_domain_identifiers(&c, &["notes"]);
        ix.delete_all_searchable_items(&c);
        ix.delete_searchable_items(&c, &[]);
        assert_eq!(
            c.take_sent(),
            [
                ToShell::QuickSearchDelete { kind: DELETE_IDS, values: vec!["a".into(), "b".into()] },
                ToShell::QuickSearchDelete { kind: DELETE_DOMAINS, values: vec!["notes".into()] },
                ToShell::QuickSearchDelete { kind: DELETE_ALL, values: vec![] },
            ]
        );
    }

    #[test]
    fn refused_items_are_reported_by_position_and_the_rest_are_sent() {
        let c = ctx();
        let refused = CSSearchableIndex::default_index().index_checked(&c, &[item("a"), item(""), item("b")]);
        assert_eq!(refused, vec![(1, IndexError::EmptyIdentifier)]);
        let sent = c.take_sent();
        let ToShell::QuickSearchIndex(batch) = &sent[0] else { panic!() };
        assert_eq!(batch.iter().map(|i| i.unique_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    }

    #[test]
    fn identifier_and_domain_lengths_are_checked_against_the_wire_cap() {
        let at_cap = "x".repeat(MAX_ID);
        assert_eq!(IndexError::check(&item(&at_cap)), Ok(()));
        assert_eq!(IndexError::check(&item(&"x".repeat(MAX_ID + 1))), Err(IndexError::IdentifierTooLong));
        let long_domain = CSSearchableItem::new("n", &"d".repeat(MAX_DOMAIN + 1), CSSearchableItemAttributeSet::new("t"));
        assert_eq!(IndexError::check(&long_domain), Err(IndexError::DomainTooLong));
        let c = ctx();
        let refused = CSSearchableIndex::default_index().index_checked(&c, &[long_domain]);
        assert_eq!(refused, vec![(0, IndexError::DomainTooLong)]);
        assert!(c.take_sent().is_empty());
    }

    #[test]
    fn error_codes_and_domain_are_stable() {
        assert_eq!(IndexError::EmptyIdentifier.code(), 1);
        assert_eq!(IndexError::IdentifierTooLong.code(), 2);
        assert_eq!(IndexError::DomainTooLong.code(), 3);
        assert_eq!(IndexError::DOMAIN, "bifrost.quicksearch.index");
    }

    #[test]
    fn localized_title_follows_the_locale_and_falls_back() {
        let t = LocalizedString::new(&[("en_US", "Milk"), ("is_IS", "Mjólk")]);
        assert_eq!(t.localized_string(&Locale::from_env("is_IS", None)), Some("Mjólk"));
        assert_eq!(t.localized_string(&Locale::from_env("is", None)), Some("Mjólk"));
        assert_eq!(t.localized_string(&Locale::from_env("de_DE", None)), Some("Milk"));
        assert_eq!(LocalizedString::default().localized_string(&Locale::from_env("en_US", None)), None);
        let set = CSSearchableItemAttributeSet::new("Plain").localized_title(LocalizedString::new(&[("is_IS", "")]));
        let (title, _) = set.wire_text(&Locale::from_env("is_IS", None));
        assert_eq!(title, "Plain");
    }

    #[test]
    fn custom_values_persons_and_handles_are_folded_into_keywords() {
        let set = CSSearchableItemAttributeSet::new("Invoice")
            .keywords(&["money"])
            .custom_attribute("status", "paid")
            .custom_attribute("empty", "")
            .person(Person {
                display_name: "Ada Lovelace".into(),
                handles: vec!["ada@example.test".into(), "money".into()],
                handle_identifier: "email".into(),
                contact_identifier: "contact-7".into(),
            });
        let (title, keywords) = set.wire_text(&Locale::from_env("en_US", None));
        assert_eq!(title, "Invoice");
        assert_eq!(keywords, ["money", "paid", "Ada Lovelace", "ada@example.test"]);
    }

    /// A delegate holding items in memory: all of them for a reindex, and those with a matching identifier otherwise.
    struct Store(Vec<CSSearchableItem>);

    impl CSSearchableIndexDelegate for Store {
        fn reindex_all_searchable_items(&self, _ctx: &Ctx) -> Vec<CSSearchableItem> {
            self.0.clone()
        }
        fn searchable_items_for(&self, _ctx: &Ctx, identifiers: &[&str]) -> Vec<CSSearchableItem> {
            self.0.iter().filter(|i| identifiers.contains(&i.unique_identifier.as_str())).cloned().collect()
        }
    }

    struct Silent;
    impl CSSearchableIndexDelegate for Silent {}

    fn batch_ids(m: &ToShell) -> Vec<&str> {
        let ToShell::QuickSearchIndex(batch) = m else { panic!("not an index message") };
        batch.iter().map(|i| i.unique_id.as_str()).collect()
    }

    #[test]
    fn reindex_all_clears_the_index_then_indexes_the_delegate_items() {
        let c = ctx();
        let refused = CSSearchableIndex::default_index().reindex_all_with(&c, &Store(vec![item("a"), item("b")]));
        assert!(refused.is_empty());
        let sent = c.take_sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0], ToShell::QuickSearchDelete { kind: DELETE_ALL, values: vec![] });
        assert_eq!(batch_ids(&sent[1]), ["a", "b"]);
    }

    #[test]
    fn reindex_identifiers_indexes_only_the_asked_items_and_deletes_nothing() {
        let c = ctx();
        let refused = CSSearchableIndex::default_index().reindex_identifiers_with(&c, &["b", "zzz"], &Store(vec![item("a"), item("b")]));
        assert!(refused.is_empty());
        let sent = c.take_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(batch_ids(&sent[0]), ["b"]);
    }

    #[test]
    fn reindex_identifiers_with_no_known_identifier_sends_nothing() {
        let c = ctx();
        CSSearchableIndex::default_index().reindex_identifiers_with(&c, &["zzz"], &Store(vec![item("a")]));
        assert!(c.take_sent().is_empty());
    }

    #[test]
    fn the_default_delegate_only_clears_the_index_on_reindex_all() {
        let c = ctx();
        CSSearchableIndex::default_index().reindex_all_with(&c, &Silent);
        assert_eq!(c.take_sent(), [ToShell::QuickSearchDelete { kind: DELETE_ALL, values: vec![] }]);
    }

    #[test]
    fn reindex_reports_refused_items_by_position_and_sends_the_rest() {
        let c = ctx();
        let refused = CSSearchableIndex::default_index().reindex_all_with(&c, &Store(vec![item(""), item("c")]));
        assert_eq!(refused, vec![(0, IndexError::EmptyIdentifier)]);
        let sent = c.take_sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(batch_ids(&sent[1]), ["c"]);
    }

    #[test]
    fn continue_activity_reaches_the_wire_format() {
        let m = crate::proto::ToApp::ContinueActivity { activity_type: ACTION_TYPE.into(), unique_id: "n1".into(), domain: "notes".into() };
        assert_eq!(crate::proto::ToApp::decode(&m.encode()), Ok(m));
    }

    #[test]
    fn an_expiration_date_is_sent_to_the_shell_after_the_item() {
        let c = ctx();
        let timed = CSSearchableItem::new("t1", "notes", CSSearchableItemAttributeSet::new("Ticket")).expiration_date(1_800_000_000);
        CSSearchableIndex::default_index().index_searchable_items(&c, &[timed]);
        let sent = c.take_sent();
        assert_eq!(sent.len(), 2);
        assert_eq!(batch_ids(&sent[0]), ["t1"]);
        let ToShell::ServiceCall { service, method, body, .. } = &sent[1] else { panic!("no expiry call") };
        assert_eq!(service.as_str(), crate::search_ranking::SERVICE);
        assert_eq!(method.as_str(), "expire");
        assert!(body.contains("1800000000") && body.contains("t1"));
    }

    #[test]
    fn index_extension_reindex_all_clears_then_indexes_and_acknowledges_refusals() {
        let c = ctx();
        let store = Store(vec![item("a"), item("")]);
        let refused = IndexExtensionRequests::new(&store).reindex_all(&c);
        assert_eq!(refused, vec![(1, IndexError::EmptyIdentifier)]);
        let sent = c.take_sent();
        assert_eq!(sent[0], ToShell::QuickSearchDelete { kind: DELETE_ALL, values: vec![] });
        assert_eq!(batch_ids(&sent[1]), ["a"]);
    }

    #[test]
    fn index_extension_reindex_identifiers_keeps_the_other_items() {
        let c = ctx();
        let store = Store(vec![item("a"), item("b")]);
        let refused = IndexExtensionRequests::new(&store).reindex_identifiers(&c, &["a"]);
        assert!(refused.is_empty());
        let sent = c.take_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(batch_ids(&sent[0]), ["a"]);
    }

    #[test]
    fn a_search_query_goes_to_the_shell_index_service() {
        let c = ctx();
        let id = CSSearchQuery::new("milk").limit(5).start(&c).unwrap();
        let sent = c.take_sent();
        assert_eq!(sent.len(), 1);
        let ToShell::ServiceCall { request, service, method, body } = &sent[0] else { panic!("no search call") };
        assert_eq!(*request, id);
        assert_eq!(service.as_str(), crate::search_ranking::SERVICE);
        assert_eq!(method.as_str(), "search");
        let body = bifrost_core::json::parse(body).unwrap();
        assert_eq!(body.get("query").and_then(|q| q.as_str()), Some("milk"));
        assert_eq!(body.get("limit").and_then(|l| l.as_u64()), Some(5));
    }

    #[test]
    fn a_blank_search_query_is_refused_and_sends_nothing() {
        let c = ctx();
        assert_eq!(CSSearchQuery::new("   ").start(&c), Err(SearchQueryError::EmptyQueryString));
        assert!(c.take_sent().is_empty());
    }

    #[test]
    fn a_search_query_reply_decodes_to_found_items() {
        let reply = bifrost_core::json::parse(r#"[{"id":"n1","domain":"notes","title":"Milk","kind":null},{"id":"r1","domain":"files","title":"Q1","kind":"com.adobe.pdf"}]"#).unwrap();
        let items = found_items(&reply);
        assert_eq!(items[0], FoundItem { unique_identifier: "n1".into(), domain_identifier: "notes".into(), title: "Milk".into(), content_type: None });
        assert_eq!(items[1].content_type.as_deref(), Some("com.adobe.pdf"));
        assert!(found_items(&bifrost_core::json::Json::Null).is_empty());
    }

    #[test]
    fn a_named_index_sends_its_items_with_the_protection_class_and_expiry() {
        let c = ctx();
        let index = NamedSearchableIndex::new("archive", Some(ProtectionClass::Complete)).unwrap();
        assert_eq!(index.name(), "archive");
        let timed = item("t1").expiration_date(1_800_000_000);
        let refused = index.index_searchable_items(&c, &[timed, item("")]);
        assert_eq!(refused, vec![(1, IndexError::EmptyIdentifier)]);
        let sent = c.take_sent();
        assert_eq!(sent.len(), 1);
        let ToShell::ServiceCall { service, method, body, .. } = &sent[0] else { panic!("no index call") };
        assert_eq!(service.as_str(), crate::search_ranking::SERVICE);
        assert_eq!(method.as_str(), "index_named");
        let body = bifrost_core::json::parse(body).unwrap();
        assert_eq!(body.get("index").and_then(|v| v.as_str()), Some("archive"));
        assert_eq!(body.get("protection").and_then(|v| v.as_str()), Some("complete"));
        let items = body.get("items").and_then(|v| v.as_array()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("id").and_then(|v| v.as_str()), Some("t1"));
        assert_eq!(items[0].get("expires").and_then(|v| v.as_u64()), Some(1_800_000_000));
    }

    #[test]
    fn a_named_index_refuses_a_bad_name_and_a_query_can_target_it() {
        assert_eq!(NamedSearchableIndex::new("a/b", None).unwrap_err(), IndexNameError::BadCharacter('/'));
        let c = ctx();
        CSSearchQuery::new("milk").in_index("archive").start(&c).unwrap();
        let sent = c.take_sent();
        let ToShell::ServiceCall { method, body, .. } = &sent[0] else { panic!("no search call") };
        assert_eq!(method.as_str(), "search_named");
        let body = bifrost_core::json::parse(body).unwrap();
        assert_eq!(body.get("index").and_then(|v| v.as_str()), Some("archive"));
        assert_eq!(body.get("query").and_then(|v| v.as_str()), Some("milk"));
    }

    #[test]
    fn deleting_from_a_named_index_names_the_index_and_the_kind() {
        let c = ctx();
        let index = NamedSearchableIndex::new("archive", None).unwrap();
        index.delete_searchable_items(&c, &["a"]);
        index.delete_all_searchable_items(&c);
        index.delete_index(&c);
        let sent = c.take_sent();
        let methods: Vec<String> = sent
            .iter()
            .map(|m| match m {
                ToShell::ServiceCall { method, body, .. } => format!("{method} {body}"),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        assert!(methods[0].starts_with("delete_named ") && methods[0].contains("\"kind\":\"ids\""));
        assert!(methods[1].contains("\"kind\":\"all\""));
        assert!(methods[2].starts_with("delete_named_index "));
    }

    #[test]
    fn an_acknowledged_batch_sends_one_index_call_and_has_no_outcome_until_the_shell_answers() {
        let c = ctx();
        let timed = item("t1").expiration_date(1_800_000_000);
        let (ack, refused) = CSSearchableIndex::default_index().index_searchable_items_acked(&c, &[timed, item("")]);
        assert_eq!(refused, vec![(1, IndexError::EmptyIdentifier)]);
        let sent = c.take_sent();
        assert_eq!(sent.len(), 1);
        let ToShell::ServiceCall { service, method, body, .. } = &sent[0] else { panic!("no index call") };
        assert_eq!(service.as_str(), crate::search_ranking::SERVICE);
        assert_eq!(method.as_str(), "index");
        let body = bifrost_core::json::parse(body).unwrap();
        let items = body.get("items").and_then(|v| v.as_array()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("id").and_then(|v| v.as_str()), Some("t1"));
        assert_eq!(items[0].get("expires").and_then(|v| v.as_u64()), Some(1_800_000_000));
        assert!(ack.outcome().is_none(), "no reply yet");
    }
}
