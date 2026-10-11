//! Core Search content indexing (`CSSearchableIndex`, `CSSearchableItem`, `CSSearchableItemAttributeSet`).
//!
//! An app indexes its own content (notes, documents, contacts) so the Search panel finds it without launching the
//! app. The shell keeps a bounded on-disk index per app and searches it; when the user opens a result it starts the app
//! and calls [`crate::App::continue_activity`] with a [`UserActivity`] of type [`ACTION_TYPE`]
//! (`CSSearchableItemActionType`) whose `unique_identifier` is the item's. See docs/bifrost-ui.md "Core Search".

pub use crate::proto::quicksearch::{ACTION_TYPE, MAX_ITEMS};
use crate::proto::quicksearch::{SearchableItem, DELETE_ALL, DELETE_DOMAINS, DELETE_IDS, MAX_VALUES};
use crate::proto::ToShell;
use crate::Ctx;

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
}

impl CSSearchableItemAttributeSet {
    pub fn new(title: &str) -> Self {
        CSSearchableItemAttributeSet { title: title.into(), ..Default::default() }
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
        let wire: Vec<SearchableItem> = items
            .iter()
            .filter(|i| !i.unique_identifier.is_empty())
            .map(|i| SearchableItem {
                unique_id: i.unique_identifier.clone(),
                domain: i.domain_identifier.clone(),
                title: i.attribute_set.title.clone(),
                description: i.attribute_set.content_description.clone(),
                keywords: i.attribute_set.keywords.clone(),
                thumbnail: i.attribute_set.thumbnail_path.clone(),
                content_url: i.attribute_set.content_url.clone(),
            })
            .collect();
        for chunk in wire.chunks(MAX_ITEMS) {
            ctx.send_to_shell(ToShell::QuickSearchIndex(chunk.to_vec()));
        }
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
    fn continue_activity_reaches_the_wire_format() {
        let m = crate::proto::ToApp::ContinueActivity { activity_type: ACTION_TYPE.into(), unique_id: "n1".into(), domain: "notes".into() };
        assert_eq!(crate::proto::ToApp::decode(&m.encode()), Ok(m));
    }
}
