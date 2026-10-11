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

/// A custom attribute: an app-defined key and value. Its value is folded into the item's keywords when indexed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomAttribute {
    pub key: String,
    pub value: String,
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
}

impl CSSearchableItemAttributeSet {
    pub fn new(title: &str) -> Self {
        CSSearchableItemAttributeSet { title: title.into(), ..Default::default() }
    }

    pub fn custom_attribute(mut self, key: &str, value: &str) -> Self {
        self.custom.push(CustomAttribute { key: key.into(), value: value.into() });
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
        let title = self.title_localized.as_ref().and_then(|t| t.localized_string(locale)).filter(|t| !t.is_empty()).unwrap_or(self.title.as_str()).to_string();
        let mut keywords = self.keywords.clone();
        let extra = self
            .custom
            .iter()
            .map(|c| c.value.as_str())
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
}

impl CSSearchableItem {
    pub fn new(unique_identifier: &str, domain_identifier: &str, attribute_set: CSSearchableItemAttributeSet) -> Self {
        CSSearchableItem { unique_identifier: unique_identifier.into(), domain_identifier: domain_identifier.into(), attribute_set }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Configure;
    use bifrost_render::font::Font;
    use bifrost_render::ui::Ui;

    fn ctx() -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
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

    #[test]
    fn continue_activity_reaches_the_wire_format() {
        let m = crate::proto::ToApp::ContinueActivity { activity_type: ACTION_TYPE.into(), unique_id: "n1".into(), domain: "notes".into() };
        assert_eq!(crate::proto::ToApp::decode(&m.encode()), Ok(m));
    }
}
