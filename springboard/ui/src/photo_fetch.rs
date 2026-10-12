//! Fetch options for a photo library listing: which items a query returns, in what order, and how many. The shell
//! keeps the library items; an app asks with a `FetchOptions` and gets the matching items back. Trashed items are
//! left out unless the query asks for them. Items with the same creation time are ordered by identifier, so the
//! same query always returns the same order. Pure: no file or shell access.

use crate::photo_media::Kind;

/// One item of a photo library listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    pub favourite: bool,
    pub trashed: bool,
    /// Creation time, for example seconds since the epoch.
    pub created: u64,
}

/// The order of the returned items, by creation time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    NewestFirst,
    OldestFirst,
}

/// What a fetch keeps. The default keeps every item that is not in the trash, newest first, with no limit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchOptions {
    /// Only items of this kind.
    pub media: Option<Kind>,
    /// Only items marked as favourites.
    pub favourites_only: bool,
    /// Only items created at or after this time.
    pub from: Option<u64>,
    /// Only items created before this time (the bound itself is left out).
    pub to: Option<u64>,
    pub sort: Sort,
    /// At most this many items, taken after sorting.
    pub limit: Option<usize>,
    /// Keep items in the trash too.
    pub include_trashed: bool,
}

/// The items that match `opts`, in the order it asks for. The input is not changed.
pub fn fetch<'a>(items: &'a [Item], opts: &FetchOptions) -> Vec<&'a Item> {
    let mut out: Vec<&Item> = items
        .iter()
        .filter(|it| opts.include_trashed || !it.trashed)
        .filter(|it| opts.media.map_or(true, |m| it.kind == m))
        .filter(|it| !opts.favourites_only || it.favourite)
        .filter(|it| opts.from.map_or(true, |f| it.created >= f))
        .filter(|it| opts.to.map_or(true, |t| it.created < t))
        .collect();
    match opts.sort {
        Sort::NewestFirst => out.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.id.cmp(&b.id))),
        Sort::OldestFirst => out.sort_by(|a, b| a.created.cmp(&b.created).then_with(|| a.id.cmp(&b.id))),
    }
    if let Some(n) = opts.limit {
        out.truncate(n);
    }
    out
}

/// The number of items `fetch` would return, without building the list.
pub fn count(items: &[Item], opts: &FetchOptions) -> usize {
    fetch(items, opts).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: Kind, favourite: bool, trashed: bool, created: u64) -> Item {
        Item { id: id.to_string(), kind, favourite, trashed, created }
    }

    fn ids(list: &[&Item]) -> Vec<String> {
        list.iter().map(|it| it.id.clone()).collect()
    }

    fn library() -> Vec<Item> {
        vec![
            item("a", Kind::Image, true, false, 100),
            item("b", Kind::Video, false, false, 300),
            item("c", Kind::Image, false, true, 200),
            item("d", Kind::Image, true, false, 200),
        ]
    }

    #[test]
    fn default_drops_trashed_and_sorts_newest_first() {
        let lib = library();
        // c is trashed. b (300) comes before d (200), then a (100).
        assert_eq!(ids(&fetch(&lib, &FetchOptions::default())), vec!["b", "d", "a"]);
    }

    #[test]
    fn media_kind_filters_items() {
        let lib = library();
        let opts = FetchOptions { media: Some(Kind::Image), ..Default::default() };
        // Images that are not trashed: a (100) and d (200), newest first.
        assert_eq!(ids(&fetch(&lib, &opts)), vec!["d", "a"]);
    }

    #[test]
    fn favourites_only_with_oldest_first() {
        let lib = library();
        let opts = FetchOptions { favourites_only: true, sort: Sort::OldestFirst, ..Default::default() };
        // Favourites that are not trashed: a (100) and d (200).
        assert_eq!(ids(&fetch(&lib, &opts)), vec!["a", "d"]);
    }

    #[test]
    fn date_range_includes_from_and_excludes_to() {
        let lib = library();
        let opts = FetchOptions { from: Some(200), to: Some(300), ..Default::default() };
        // d (200) is in range; b (300) is the excluded bound; a (100) is before the range.
        assert_eq!(ids(&fetch(&lib, &opts)), vec!["d"]);
        let opts = FetchOptions { from: Some(200), to: Some(301), ..Default::default() };
        assert_eq!(ids(&fetch(&lib, &opts)), vec!["b", "d"]);
    }

    #[test]
    fn limit_takes_the_first_items_after_sorting() {
        let lib = library();
        let opts = FetchOptions { limit: Some(2), ..Default::default() };
        assert_eq!(ids(&fetch(&lib, &opts)), vec!["b", "d"]);
        assert_eq!(count(&lib, &opts), 2);
        // A limit larger than the list returns everything.
        let opts = FetchOptions { limit: Some(10), ..Default::default() };
        assert_eq!(count(&lib, &opts), 3);
    }

    #[test]
    fn trashed_items_come_back_with_ties_ordered_by_id() {
        let lib = library();
        let opts = FetchOptions { include_trashed: true, sort: Sort::OldestFirst, ..Default::default() };
        // a (100), then c and d at 200 ordered by identifier ("c" before "d"), then b (300).
        assert_eq!(ids(&fetch(&lib, &opts)), vec!["a", "c", "d", "b"]);
    }

    #[test]
    fn empty_library_gives_no_items() {
        assert_eq!(count(&[], &FetchOptions::default()), 0);
    }
}
