//! Core Search index in the shell (`CSSearchableIndex`). Apps send `ToShell::SearchIndex` and
//! `SearchDelete`; this keeps a bounded index per app, saved as `<data>/quicksearch-index/<app id>.json`
//! (atomically replaced, only when it changed), and the Search panel searches it ("Documents" section). Opening a
//! result starts the app and sends `ToApp::ContinueActivity` (see `Shell::quicksearch_act`).
//!
//! The Search panel re-orders the matches this index returns with `bifrost_ui::search_rank` (BM25 over title and text,
//! recency of indexing, local open counts). The order below is this index's own matching order, which the panel keeps
//! for the items the ranker does not score.
//!
//! Ranking: the query is split into words (`bifrost_ui::lexical_tags::words`) and every word must match an item
//! (case and accent folded; prefix, word start or subsequence, see `super::fuzzy`). A title match outranks a keyword
//! match, which outranks a description match; then an item in the query's language (`bifrost_ui::language`) ranks
//! above one in another language; then shorter matches; then the newest item.
//! Expiry: an item can carry a Unix time after which it is dropped (`CSSearchableItem.expirationDate`), set through the
//! `search-index` service (`services/search_ranking.rs`).
//!
//! Filters (see [`parse_query`]): a word naming a file type ("pdf", "photos") keeps only items whose content URL has
//! that type (`bifrost_ui::uttype`); a time phrase ("last week", "yesterday", "this month", "past 3 days") keeps only
//! items indexed in that window (`bifrost_ui::metaquery` predicates). A query of filters alone lists the matching
//! items, newest first. Dates are UTC and weeks start on Monday.
//!
//! Bounds: `MAX_PER_APP` items per app (the oldest are dropped first), at most `MAX_APPS` apps, and every field is
//! already cut by the wire codec, so one app cannot grow the index without limit.
//!
//! Speed: a query that extends the previous word query (typing more) ranks only the items that query matched; only
//! the best `limit` are built; expiry is skipped until an item is due; and saves are written by one saver thread.

use super::fuzzy::Matcher;
use crate::gfx::{rgba, Image};
use bifrost_core::json::{self, Json};
use bifrost_proto::quicksearch::{SearchableItem, DELETE_ALL, DELETE_DOMAINS, DELETE_IDS};
use bifrost_ui::language::{dominant_language, Language};
use bifrost_ui::lexical_tags;
use bifrost_ui::metaquery::{self, Predicate};
use bifrost_ui::searchindex::{validate_name, IndexDescription, NameError, ProtectionClass};
use bifrost_ui::searchquery::{Query, Record};
use bifrost_ui::searchstore::{self, IndexStore, StoreSet, StoredItem};
use bifrost_ui::uttype;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_PER_APP: usize = 1000;
pub const MAX_APPS: usize = 64;

/// An indexed item with its folded search text.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    item: SearchableItem,
    title_folded: String,
    keywords_folded: String,
    desc_folded: String,
    /// the dominant language of the item's text, when one is recognisable
    lang: Option<Language>,
    /// Unix seconds after which the item is dropped; `None` keeps it until the app deletes it
    expires_at: Option<u64>,
    /// the type of the file the content URL names (`uttype` identifier), when its extension is known
    kind: Option<&'static str>,
    /// Unix seconds when the item was indexed (or last replaced); the time filters of a query read it
    indexed: u64,
}

/// The type of the file a content URL names, from its extension ("file:///home/a/report.pdf" is a PDF). A URL
/// without an extension has no type.
fn kind_of(url: &str) -> Option<&'static str> {
    let path = url.split(|c: char| c == '?' || c == '#').next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    if name.contains('.') {
        uttype::type_for_extension(name)
    } else {
        None
    }
}

impl Entry {
    fn new(item: SearchableItem, indexed: u64) -> Entry {
        let text = format!("{} {} {}", item.title, item.keywords.join(" "), item.description);
        Entry {
            title_folded: super::fuzzy::fold(&item.title),
            keywords_folded: super::fuzzy::fold(&item.keywords.join(" ")),
            desc_folded: super::fuzzy::fold(&item.description),
            lang: dominant_language(&text),
            expires_at: None,
            kind: kind_of(&item.content_url),
            indexed,
            item,
        }
    }
}

/// The rank of an item for a query: the weakest field among its words, the kinds of its matches, whether it is in
/// another language than the query, and the length of its matches. `None` when some word matches nothing.
fn score(words: &[Matcher], e: &Entry, query_lang: Option<Language>) -> Option<(u8, u8, bool, usize)> {
    let (mut field, mut kind, mut len) = (0u8, 0u8, 0usize);
    for w in words {
        let (f, r) = [(0u8, &e.title_folded), (1, &e.keywords_folded), (2, &e.desc_folded)].into_iter().filter_map(|(f, text)| w.rank_folded(text).map(|r| (f, r))).min()?;
        field = field.max(f);
        kind += r.kind as u8;
        len += r.len;
    }
    let other_language = matches!((query_lang, e.lang), (Some(q), Some(l)) if q != l);
    Some((field, kind, other_language, len))
}

/// A search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub app_id: String,
    pub unique_id: String,
    pub domain: String,
    pub title: String,
    /// the `uttype` identifier of the indexed file, when the content URL names one
    pub kind: Option<&'static str>,
    /// the thumbnail path the app gave, relative to its data dir ("" when none)
    pub thumbnail: String,
    /// keywords and description, joined: the text the ranker searches besides the title
    pub text: String,
    /// Unix seconds the item was indexed (or last replaced); 0 when unknown
    pub modified: u64,
}

/// The entries a word query matched, and the query they matched for. A query whose words extend these (more typing)
/// can only match these entries, so it ranks them instead of every entry.
#[derive(Default)]
struct Narrow {
    gen: u64,
    words: Vec<String>,
    kinds: Vec<&'static str>,
    window: Option<(u64, u64)>,
    /// (app position, entry position) in `apps` order
    hits: Vec<(usize, usize)>,
}

#[derive(Default)]
pub struct Index {
    apps: BTreeMap<String, Vec<Entry>>,
    dirty: Vec<String>,
    /// bumped by every change to the items (see `mark`); a narrowing is only reused within one generation
    gen: u64,
    /// no item expires before this (Unix seconds), so `prune` returns early until then; 0 when not known
    next_expiry: u64,
    narrow: RefCell<Option<Narrow>>,
    /// the named indexes (`CSSearchableIndex(name:protectionClass:)`), per app, kept apart from the default ones
    named: StoreSet,
    /// named indexes changed since the last save, as `<app>@<name>` file stems
    named_dirty: Vec<String>,
}

/// True when every word of `new` extends the word of `old` at its place (the last word of `old` may be shorter). Then
/// whatever matches `new` matches `old` too, so the entries `old` matched hold every entry `new` can match.
fn extends(old: &[String], new: &[String]) -> bool {
    match old.split_last() {
        Some((last, head)) => new.len() >= old.len() && head.iter().zip(new).all(|(a, b)| a == b) && new[old.len() - 1].starts_with(last.as_str()),
        None => false,
    }
}

fn safe_name(app_id: &str) -> bool {
    !app_id.is_empty() && app_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') && !app_id.starts_with('.')
}

impl Index {
    /// `indexSearchableItems`: items with an existing identifier replace the old one (and move to the newest end).
    pub fn index(&mut self, app_id: &str, items: Vec<SearchableItem>) {
        self.index_at(app_id, items, now_secs());
    }

    /// Like `index`, with the time of indexing given (Unix seconds).
    pub fn index_at(&mut self, app_id: &str, items: Vec<SearchableItem>, now: u64) {
        if !self.apps.contains_key(app_id) && self.apps.len() >= MAX_APPS {
            return;
        }
        let list = self.apps.entry(app_id.to_string()).or_default();
        for item in items.into_iter().filter(|i| !i.unique_id.is_empty()) {
            list.retain(|e| e.item.unique_id != item.unique_id);
            list.push(Entry::new(item, now));
        }
        if list.len() > MAX_PER_APP {
            list.drain(..list.len() - MAX_PER_APP);
        }
        self.mark(app_id);
    }

    /// `deleteSearchableItems(...)` and `deleteAllSearchableItems`; `kind` is a `DELETE_*`.
    pub fn delete(&mut self, app_id: &str, kind: u8, values: &[String]) {
        let Some(list) = self.apps.get_mut(app_id) else { return };
        match kind {
            DELETE_IDS => list.retain(|e| !values.contains(&e.item.unique_id)),
            DELETE_DOMAINS => list.retain(|e| !values.contains(&e.item.domain)),
            DELETE_ALL => list.clear(),
            _ => return,
        }
        if list.is_empty() {
            self.apps.remove(app_id);
        }
        self.mark(app_id);
    }

    fn mark(&mut self, app_id: &str) {
        self.gen += 1;
        if !self.dirty.iter().any(|d| d == app_id) {
            self.dirty.push(app_id.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.apps.values().map(Vec::len).sum()
    }

    /// The best `limit` items for `query` (every word of it must match, and the filters of it must hold): see the
    /// module notes on ranking and filters.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Found> {
        self.rank_hits(None, query, limit, now_secs())
    }

    /// Like `search`, with the current time given (Unix seconds): the time filters read it.
    pub fn search_at(&self, query: &str, limit: usize, now: u64) -> Vec<Found> {
        self.rank_hits(None, query, limit, now)
    }

    /// Like `search`, over the items of `app_id` only.
    pub fn search_app(&self, app_id: &str, query: &str, limit: usize) -> Vec<Found> {
        self.rank_hits(Some(app_id), query, limit, now_secs())
    }

    /// Ranks the matches, best first, and builds only the `limit` best. The order is the score, then the newest
    /// (higher position in its app), then the app order. A word query also leaves its matches in `narrow`.
    fn rank_hits(&self, only: Option<&str>, query: &str, limit: usize, now: u64) -> Vec<Found> {
        let f = parse_query(query, now);
        if f.words.is_empty() && f.kinds.is_empty() && f.window.is_none() {
            return vec![];
        }
        let words: Vec<Matcher> = f.words.iter().map(|w| Matcher::new(w.as_str())).collect();
        let query_lang = if words.is_empty() { None } else { dominant_language(&f.words.join(" ")) };
        let window = f.window.map(|(from, to)| window_predicate(from, to));
        let lists: Vec<(&str, &[Entry])> = self.apps.iter().map(|(app, list)| (app.as_str(), list.as_slice())).collect();
        let narrowed = if only.is_none() && !f.words.is_empty() { self.narrowed(&f) } else { None };
        let mut ranked: Vec<(((u8, u8, bool, usize), std::cmp::Reverse<usize>), usize, usize)> = vec![];
        {
            let mut consider = |ai: usize, ei: usize| {
                let e = &lists[ai].1[ei];
                if !passes(e, &f.kinds, window.as_ref()) {
                    return;
                }
                // a query of filters alone ranks every match equal, so the newest comes first
                let key = if words.is_empty() { Some((0, 0, false, 0)) } else { score(&words, e, query_lang) };
                if let Some(key) = key {
                    ranked.push(((key, std::cmp::Reverse(ei)), ai, ei));
                }
            };
            match &narrowed {
                Some(prev) => {
                    for &(ai, ei) in prev {
                        consider(ai, ei);
                    }
                }
                None => {
                    for (ai, (app, list)) in lists.iter().enumerate() {
                        if only.map_or(false, |a| a != *app) {
                            continue;
                        }
                        for ei in 0..list.len() {
                            consider(ai, ei);
                        }
                    }
                }
            }
        }
        if only.is_none() && !f.words.is_empty() {
            let hits = ranked.iter().map(|&(_, ai, ei)| (ai, ei)).collect();
            *self.narrow.borrow_mut() = Some(Narrow { gen: self.gen, words: f.words.clone(), kinds: f.kinds.clone(), window: f.window, hits });
        }
        if ranked.len() > limit {
            ranked.select_nth_unstable(limit);
            ranked.truncate(limit);
        }
        ranked.sort_unstable();
        ranked
            .into_iter()
            .map(|(_, ai, ei)| {
                let (app, list) = lists[ai];
                let e = &list[ei];
                let text = format!("{} {}", e.item.keywords.join(" "), e.item.description);
                Found { app_id: app.to_string(), unique_id: e.item.unique_id.clone(), domain: e.item.domain.clone(), title: e.item.title.clone(), kind: e.kind, thumbnail: e.item.thumbnail.clone(), text, modified: e.indexed }
            })
            .collect()
    }

    /// The entries the last word query matched, when this query extends it under the same filters and no item has
    /// changed since. Taking them clears the stored narrowing; `rank_hits` stores the new one.
    fn narrowed(&self, f: &Filters) -> Option<Vec<(usize, usize)>> {
        let prev = self.narrow.borrow_mut().take()?;
        (prev.gen == self.gen && prev.kinds == f.kinds && prev.window == f.window && extends(&prev.words, &f.words)).then_some(prev.hits)
    }

    /// Sets (or with `None` clears) the expiry of one item. False when the app has no item with that identifier.
    pub fn set_expiry(&mut self, app_id: &str, unique_id: &str, at: Option<u64>) -> bool {
        let Some(e) = self.apps.get_mut(app_id).and_then(|l| l.iter_mut().find(|e| e.item.unique_id == unique_id)) else {
            return false;
        };
        e.expires_at = at;
        if let Some(t) = at {
            self.next_expiry = self.next_expiry.min(t);
        }
        self.mark(app_id);
        true
    }

    /// Drops the items whose expiry is at or before `now` (Unix seconds). Returns how many went. Returns at once while
    /// no item can have expired yet, so a keystroke's search does not walk every item.
    pub fn prune(&mut self, now: u64) -> usize {
        if now < self.next_expiry {
            return 0;
        }
        let mut gone = 0;
        let mut changed = vec![];
        for (app, list) in self.apps.iter_mut() {
            let before = list.len();
            list.retain(|e| e.expires_at.map_or(true, |t| t > now));
            if list.len() != before {
                gone += before - list.len();
                changed.push(app.clone());
            }
        }
        self.apps.retain(|_, l| !l.is_empty());
        self.next_expiry = self.apps.values().flatten().filter_map(|e| e.expires_at).min().unwrap_or(u64::MAX);
        for app in changed {
            self.mark(&app);
        }
        gone
    }

    fn to_json(list: &[Entry]) -> String {
        let q = json::quote;
        let items: Vec<String> = list
            .iter()
            .map(|e| {
                let i = &e.item;
                let kw: Vec<String> = i.keywords.iter().map(|k| q(k)).collect();
                let expires = e.expires_at.map(|t| format!(",\"expires\":{t}")).unwrap_or_default();
                format!("{{\"id\":{},\"domain\":{},\"title\":{},\"desc\":{},\"keywords\":[{}],\"thumb\":{},\"url\":{},\"indexed\":{}{}}}", q(&i.unique_id), q(&i.domain), q(&i.title), q(&i.description), kw.join(","), q(&i.thumbnail), q(&i.content_url), e.indexed, expires)
            })
            .collect();
        format!("[{}]\n", items.join(",\n"))
    }

    fn from_json(text: &str) -> Vec<Entry> {
        let Ok(v) = json::parse(text) else { return vec![] };
        let Some(arr) = v.array() else { return vec![] };
        arr.iter()
            .filter_map(|o| {
                let unique_id = o.str_field("id").filter(|s| !s.is_empty())?;
                let keywords = o.get("keywords").and_then(Json::array).map(|a| a.iter().filter_map(|k| k.string().map(str::to_string)).collect()).unwrap_or_default();
                let indexed = o.get("indexed").and_then(Json::as_u64).unwrap_or(0);
                let mut e = Entry::new(
                    SearchableItem {
                        unique_id,
                        domain: o.str_field("domain").unwrap_or_default(),
                        title: o.str_field("title").unwrap_or_default(),
                        description: o.str_field("desc").unwrap_or_default(),
                        keywords,
                        thumbnail: o.str_field("thumb").unwrap_or_default(),
                        content_url: o.str_field("url").unwrap_or_default(),
                    },
                    indexed,
                );
                e.expires_at = o.get("expires").and_then(Json::as_u64);
                Some(e)
            })
            .take(MAX_PER_APP)
            .collect()
    }

    /// Reads every `<app id>.json` under `dir`.
    pub fn load(dir: &Path) -> Index {
        let mut ix = Index::default();
        let Ok(rd) = std::fs::read_dir(dir) else { return ix };
        for ent in rd.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            let Some(app) = name.strip_suffix(".json").filter(|a| safe_name(a)) else { continue };
            if let Some((app, index)) = name.strip_suffix(".json").and_then(|s| s.split_once('@')) {
                // a named index, `<app>@<name>.json`, which the default-index loader below skips
                if safe_name(app) && validate_name(index).is_ok() {
                    if let Some(store) = named_from_json(&std::fs::read_to_string(ent.path()).unwrap_or_default()) {
                        ix.named.put(app, Some(index), store);
                    }
                }
                continue;
            }
            if ix.apps.len() >= MAX_APPS {
                break;
            }
            let list = Index::from_json(&std::fs::read_to_string(ent.path()).unwrap_or_default());
            if !list.is_empty() {
                ix.apps.insert(app.to_string(), list);
            }
        }
        ix
    }

    /// The changed apps since the last call, as (app id, their JSON, or None when the app is gone).
    fn take_writes(&mut self) -> Vec<(String, Option<String>)> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .filter(|app| safe_name(app))
            .map(|app| {
                let text = self.apps.get(&app).map(|list| Index::to_json(list));
                (app, text)
            })
            .collect()
    }

    /// Writes the apps that changed since the last call (and removes the file of an emptied app), then the named
    /// indexes that changed.
    pub fn save(&mut self, dir: &Path) {
        for (app, text) in self.take_writes() {
            write_app(dir, &app, text);
        }
        for stem in std::mem::take(&mut self.named_dirty) {
            let text = stem.split_once('@').and_then(|(app, index)| self.named.get(app, Some(index))).map(named_to_json);
            write_app(dir, &stem, text);
        }
    }
}

/// Replaces `<dir>/<app>.json` with `text` (through a temporary file), or removes it when `text` is None.
fn write_app(dir: &Path, app: &str, text: Option<String>) {
    let path = dir.join(format!("{app}.json"));
    match text {
        Some(text) => {
            let _ = std::fs::create_dir_all(dir);
            let tmp = dir.join(format!("{app}.json.tmp"));
            if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// The writer thread's queue: (index dir, writes), in the order they were taken.
type SaveJob = (PathBuf, Vec<(String, Option<String>)>);
static SAVER: Mutex<Option<std::sync::mpsc::Sender<SaveJob>>> = Mutex::new(None);

impl Index {

    /// Drops the items of an app that was uninstalled.
    pub fn forget_app(&mut self, app_id: &str) {
        if self.apps.remove(app_id).is_some() {
            self.mark(app_id);
        }
        let named: Vec<String> = self.named.keys().into_iter().filter(|(a, _)| *a == app_id).filter_map(|(_, n)| n.map(str::to_string)).collect();
        for index in named {
            self.named.delete_named_index(app_id, &index);
            self.mark_named(app_id, &index);
        }
    }

    /// Drops the items of every app not in `installed`; returns how many apps went.
    pub fn forget_missing(&mut self, installed: &[&str]) -> usize {
        let gone: Vec<String> = self.apps.keys().filter(|app| !installed.contains(&app.as_str())).cloned().collect();
        for app in &gone {
            self.forget_app(app);
        }
        gone.len()
    }
}

impl Index {
    fn mark_named(&mut self, app_id: &str, index: &str) {
        let stem = format!("{app_id}@{index}");
        if !self.named_dirty.contains(&stem) {
            self.named_dirty.push(stem);
        }
    }

    /// `indexSearchableItems` on the named index `desc` of `app_id`: items with a known identifier replace the old
    /// one. A new index is refused once the shell holds [`MAX_APPS`] named indexes.
    pub fn index_named(&mut self, app_id: &str, desc: &IndexDescription, items: Vec<StoredItem>) -> Result<(), NameError> {
        let known = self.named.get(app_id, Some(&desc.name)).is_some();
        if !known && self.named.keys().len() >= MAX_APPS {
            return Ok(());
        }
        self.named.named_index(app_id, desc)?.index(items);
        self.mark_named(app_id, &desc.name);
        Ok(())
    }

    /// `deleteSearchableItems(withIdentifiers:)` on a named index.
    pub fn delete_named_identifiers(&mut self, app_id: &str, index: &str, ids: &[&str]) {
        if let Some(store) = self.named.get_mut(app_id, Some(index)) {
            store.delete_identifiers(ids);
            self.mark_named(app_id, index);
        }
    }

    /// `deleteSearchableItems(withDomainIdentifiers:)` on a named index.
    pub fn delete_named_domains(&mut self, app_id: &str, index: &str, domains: &[&str]) {
        if let Some(store) = self.named.get_mut(app_id, Some(index)) {
            store.delete_domains(domains);
            self.mark_named(app_id, index);
        }
    }

    /// `deleteAllSearchableItems` on a named index; the index itself stays.
    pub fn delete_named_all(&mut self, app_id: &str, index: &str) {
        if let Some(store) = self.named.get_mut(app_id, Some(index)) {
            store.delete_all();
            self.mark_named(app_id, index);
        }
    }

    /// Deletes a named index and its items (`CSSearchableIndex.deleteIndex`). False when it did not exist.
    pub fn delete_named_index(&mut self, app_id: &str, index: &str) -> bool {
        let gone = self.named.delete_named_index(app_id, index);
        if gone {
            self.mark_named(app_id, index);
        }
        gone
    }

    /// Runs `query` over the named index `index` of `owner` as `requester`: the owner sees its live items, another app
    /// only the public ones. At most `limit` hits, best first. A missing index gives none.
    pub fn search_named(&self, requester: &str, owner: &str, index: &str, query: &str, limit: usize, now: i64) -> Result<Vec<searchstore::Found>, bifrost_ui::searchquery::Error> {
        let mut hits = self.named.search(requester, owner, Some(index), &Query::new(query), now)?;
        hits.truncate(limit);
        Ok(hits)
    }
}

/// The saved form of a named index: its protection class and its items, with the expiry and the public flag.
fn named_to_json(store: &IndexStore) -> String {
    let q = json::quote;
    let items: Vec<String> = store
        .items()
        .iter()
        .map(|i| {
            let kw: Vec<String> = i.record.keywords.iter().map(|k| q(k)).collect();
            let expires = i.expiration.map(|t| format!(",\"expires\":{t}")).unwrap_or_default();
            format!(
                "{{\"id\":{},\"domain\":{},\"title\":{},\"desc\":{},\"keywords\":[{}],\"public\":{}{}}}",
                q(&i.unique_identifier),
                q(&i.domain_identifier),
                q(&i.record.title),
                q(&i.record.description),
                kw.join(","),
                i.public,
                expires
            )
        })
        .collect();
    let protection = store.protection().map_or("null".to_string(), |p| q(p.raw()));
    format!("{{\"protection\":{protection},\"items\":[{}]}}\n", items.join(",\n"))
}

/// Reads a saved named index. None when the text is not one.
fn named_from_json(text: &str) -> Option<IndexStore> {
    let v = json::parse(text).ok()?;
    let protection = v.get("protection").and_then(Json::as_str).and_then(ProtectionClass::from_raw);
    let items = parse_named_items(v.get("items").and_then(Json::array)?);
    Some(IndexStore::from_items(items, protection))
}

/// Items of a named index, from a list of objects with `id`, `domain`, `title`, `desc`, `keywords` (strings),
/// `public` (bool) and `expires` (Unix seconds). An object without an `id` is skipped; at most `MAX_ITEMS`.
pub fn parse_named_items(list: &[Json]) -> Vec<StoredItem> {
    list.iter()
        .filter_map(|o| {
            let id = o.str_field("id").filter(|s| !s.is_empty())?;
            let keywords = o.get("keywords").and_then(Json::array).map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let record = Record { title: o.str_field("title").unwrap_or_default(), description: o.str_field("desc").unwrap_or_default(), keywords };
            let mut item = StoredItem::new(&id, &o.str_field("domain").unwrap_or_default(), record);
            item.expiration = o.get("expires").and_then(Json::as_u64).map(|t| t as i64);
            item.public = matches!(o.get("public"), Some(Json::Bool(true)));
            Some(item)
        })
        .take(searchstore::MAX_ITEMS)
        .collect()
}

// ---- filters: file types and time phrases -------------------------------------------------------------------------

const DAY: u64 = 86_400;
/// Words that name a file type by its extension; other words are plain text.
const EXTENSION_WORDS: &[&str] = &["pdf", "png", "jpg", "jpeg", "gif", "webp", "svg", "mp3", "m4a", "flac", "wav", "mp4", "mov", "zip", "md", "txt", "csv", "json", "html"];
/// Words that may introduce a time phrase ("from last week"); the phrase is what filters.
const LEAD_WORDS: &[&str] = &["from", "in", "on", "during", "since"];
/// Thumbnails larger than this are not read, and decoded images above this many pixels are refused.
const MAX_THUMB_BYTES: u64 = 4 << 20;
const MAX_THUMB_PIXELS: usize = 4_000_000;

/// A query split into what it searches for and what it filters by.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    /// the words every hit must match, lowercase
    pub words: Vec<String>,
    /// `uttype` identifiers the hit's file must conform to ("pdf" is `com.adobe.pdf`, "photos" is `public.image`)
    pub kinds: Vec<&'static str>,
    /// the indexing time must fall in `[from, to)` (Unix seconds); `to` is `u64::MAX` for "since"
    pub window: Option<(u64, u64)>,
}

/// Splits a Search query into words, file type filters and a time window. `now` is the current Unix time in seconds.
/// Time phrases: "today", "yesterday", "this|last week|month|year" (calendar periods), "past week|month|year" and
/// "last|past N days|weeks" (rolling). A lead word before a phrase ("from last week") is dropped with it.
pub fn parse_query(query: &str, now: u64) -> Filters {
    let tokens: Vec<String> = lexical_tags::words(query).into_iter().map(|(w, _)| w.to_lowercase()).collect();
    let mut f = Filters::default();
    let mut i = 0;
    while i < tokens.len() {
        if let Some((n, window)) = time_phrase(&tokens[i..], now) {
            f.window = Some(window);
            i += n;
            continue;
        }
        let t = tokens[i].as_str();
        if LEAD_WORDS.contains(&t) && i + 1 < tokens.len() && time_phrase(&tokens[i + 1..], now).is_some() {
            i += 1;
            continue;
        }
        match kind_word(t) {
            Some(k) => {
                if !f.kinds.contains(&k) {
                    f.kinds.push(k);
                }
            }
            None => f.words.push(tokens[i].clone()),
        }
        i += 1;
    }
    f
}

/// The type a word filters by, if it names one.
fn kind_word(t: &str) -> Option<&'static str> {
    let named = match t {
        "image" | "images" | "photo" | "photos" | "picture" | "pictures" => "public.image",
        "video" | "videos" | "movie" | "movies" => "public.movie",
        "audio" | "music" | "song" | "songs" => "public.audio",
        "document" | "documents" => "public.content",
        "text" => "public.text",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named);
    }
    if EXTENSION_WORDS.contains(&t) {
        uttype::type_for_extension(t)
    } else {
        None
    }
}

/// A time phrase at the start of `t`: how many words it takes and the window it names.
fn time_phrase(t: &[String], now: u64) -> Option<(usize, (u64, u64))> {
    let first = t.first()?.as_str();
    let today = now / DAY * DAY;
    if first == "today" {
        return Some((1, (today, u64::MAX)));
    }
    if first == "yesterday" {
        return Some((1, (today.saturating_sub(DAY), today)));
    }
    let second = t.get(1).map(String::as_str);
    if let Some(n) = second.and_then(|s| s.parse::<u64>().ok()) {
        // "last 3 days": a rolling window of N units back from now
        let len = rolling(t.get(2)?)?;
        return match first {
            "last" | "past" => Some((3, (now.saturating_sub(n.saturating_mul(len)), u64::MAX))),
            _ => None,
        };
    }
    match (first, second?) {
        ("this", unit) => calendar(unit, false, now).map(|w| (2, w)),
        ("last", unit) => calendar(unit, true, now).map(|w| (2, w)),
        ("past", unit) => rolling(unit).map(|len| (2, (now.saturating_sub(len), u64::MAX))),
        _ => None,
    }
}

/// The length of a rolling unit in seconds ("past week" is seven days).
fn rolling(unit: &str) -> Option<u64> {
    match unit {
        "day" | "days" => Some(DAY),
        "week" | "weeks" => Some(7 * DAY),
        "month" | "months" => Some(30 * DAY),
        "year" | "years" => Some(365 * DAY),
        _ => None,
    }
}

/// A calendar week, month or year: the current one (`last` false, open-ended) or the previous one (`last` true).
fn calendar(unit: &str, last: bool, now: u64) -> Option<(u64, u64)> {
    let days = (now / DAY) as i64;
    let (start, prev) = match unit {
        "week" | "weeks" => {
            // 1970-01-01 was a Thursday; with Monday as 0 that day is 3
            let start = days - (days + 3).rem_euclid(7);
            (start, start - 7)
        }
        "month" | "months" => {
            let (y, m) = year_month(days);
            let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
            (days_from_civil(y, m, 1), days_from_civil(py, pm, 1))
        }
        "year" | "years" => {
            let (y, _) = year_month(days);
            (days_from_civil(y, 1, 1), days_from_civil(y - 1, 1, 1))
        }
        _ => return None,
    };
    Some(if last { (day_secs(prev), day_secs(start)) } else { (day_secs(start), u64::MAX) })
}

fn day_secs(day: i64) -> u64 {
    day.max(0) as u64 * DAY
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The (year, month) of a day count since 1970-01-01.
fn year_month(days: i64) -> (i64, i64) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m)
}

/// The time predicate of a window, over an entry's indexing time.
fn window_predicate(from: u64, to: u64) -> Predicate {
    let cap = |t: u64| t.min(i64::MAX as u64) as i64;
    Predicate::All(vec![Predicate::ModifiedAtOrAfter(cap(from)), Predicate::ModifiedBefore(cap(to))])
}

/// Whether an entry has every filtered type and falls in the window.
fn passes(e: &Entry, kinds: &[&str], window: Option<&Predicate>) -> bool {
    if !kinds.iter().all(|k| e.kind.map_or(false, |t| uttype::conforms(t, k))) {
        return false;
    }
    window.map_or(true, |p| {
        let record = metaquery::Metadata { name: String::new(), path: String::new(), is_dir: false, size: 0, modified: e.indexed.min(i64::MAX as u64) as i64, attributes: BTreeMap::new() };
        p.matches(&record)
    })
}

// ---- thumbnails ---------------------------------------------------------------------------------------------------

/// The thumbnail of an indexed item, decoded from its PNG and fitted (aspect kept, centred) into a `size` by `size`
/// image. The path is inside the app's data dir (`<root>/appdata/<app id>`): relative paths are joined to it, and a
/// path that leaves it (`..`, another directory) is refused. `None` when the file is missing or not a PNG.
pub fn load_thumbnail(app_id: &str, thumb: &str, size: usize) -> Option<Image> {
    if !safe_name(app_id) || thumb.is_empty() || size == 0 {
        return None;
    }
    let data = super::apphost::root().join("appdata").join(app_id);
    let rel = Path::new(thumb);
    let path = if rel.is_absolute() { rel.to_path_buf() } else { data.join(rel) };
    if !path.starts_with(&data) || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return None;
    }
    if std::fs::metadata(&path).ok()?.len() > MAX_THUMB_BYTES {
        return None;
    }
    decode_thumbnail(&std::fs::read(&path).ok()?, size)
}

fn decode_thumbnail(bytes: &[u8], size: usize) -> Option<Image> {
    use png::{BitDepth, ColorType, Transformations};
    let mut d = png::Decoder::new(std::io::Cursor::new(bytes));
    d.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut r = d.read_info().ok()?;
    let (ct, depth) = r.output_color_type();
    if depth != BitDepth::Eight {
        return None;
    }
    let (w, h) = (r.info().width as usize, r.info().height as usize);
    if w == 0 || h == 0 || w * h > MAX_THUMB_PIXELS {
        return None;
    }
    let channels = match ct {
        ColorType::Rgba => 4,
        ColorType::Rgb => 3,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Grayscale => 1,
        _ => return None,
    };
    let mut src: Vec<u32> = Vec::with_capacity(w * h);
    while let Some(row) = r.next_row().ok()? {
        for p in row.data().chunks_exact(channels) {
            src.push(match ct {
                ColorType::Rgba => rgba(p[0], p[1], p[2], p[3]),
                ColorType::Rgb => rgba(p[0], p[1], p[2], 255),
                ColorType::GrayscaleAlpha => rgba(p[0], p[0], p[0], p[1]),
                _ => rgba(p[0], p[0], p[0], 255),
            });
        }
    }
    if src.len() != w * h {
        return None;
    }
    let scale = (size as f64 / w as f64).min(size as f64 / h as f64);
    let dw = ((w as f64 * scale).round() as usize).clamp(1, size);
    let dh = ((h as f64 * scale).round() as usize).clamp(1, size);
    let (ox, oy) = ((size - dw) / 2, (size - dh) / 2);
    let mut out = Image::new(size, size);
    for y in 0..dh {
        let sy = y * h / dh;
        for x in 0..dw {
            out.px[(oy + y) * size + ox + x] = src[sy * w + x * w / dw];
        }
    }
    Some(out)
}

// ---- the shell's index ------------------------------------------------------------------------------------------

fn dir() -> PathBuf {
    super::apphost::root().join("quicksearch-index")
}

static INDEX: Mutex<Option<Index>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Index) -> R) -> R {
    let mut g = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(|| Index::load(&dir())))
}

/// Applies a `SearchIndex` or `SearchDelete` message of `app_id`.
pub fn apply(app_id: &str, msg: bifrost_proto::ToShell) {
    use bifrost_proto::ToShell;
    with(|ix| match msg {
        ToShell::QuickSearchIndex(items) => ix.index(app_id, items),
        ToShell::QuickSearchDelete { kind, values } => ix.delete(app_id, kind, &values),
        _ => {}
    });
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Drops the items whose expiry has passed; returns how many went.
pub fn expire_due() -> usize {
    let now = now_secs();
    with(|ix| ix.prune(now))
}

/// Sets (or clears, with `None`) the expiry of an item of `app_id`; false when there is no such item.
pub fn set_expiry(app_id: &str, unique_id: &str, at: Option<u64>) -> bool {
    with(|ix| ix.set_expiry(app_id, unique_id, at))
}

/// `indexSearchableItems` on the named index `desc` of `app_id`.
pub fn index_named(app_id: &str, desc: &IndexDescription, items: Vec<StoredItem>) -> Result<(), NameError> {
    with(|ix| ix.index_named(app_id, desc, items))
}

/// Deletes items of the named index `index` of `app_id`: `kind` is a `DELETE_*` and `values` are identifiers or
/// domains (ignored for `DELETE_ALL`).
pub fn delete_named(app_id: &str, index: &str, kind: u8, values: &[&str]) {
    with(|ix| match kind {
        DELETE_IDS => ix.delete_named_identifiers(app_id, index, values),
        DELETE_DOMAINS => ix.delete_named_domains(app_id, index, values),
        DELETE_ALL => ix.delete_named_all(app_id, index),
        _ => {}
    })
}

/// Deletes the named index `index` of `app_id` and its items; false when it did not exist.
pub fn delete_named_index(app_id: &str, index: &str) -> bool {
    with(|ix| ix.delete_named_index(app_id, index))
}

/// The items of the named index `index` of `app_id` matching `query`, at most `limit`. Expired items are not returned.
pub fn search_named(app_id: &str, index: &str, query: &str, limit: usize) -> Result<Vec<searchstore::Found>, bifrost_ui::searchquery::Error> {
    let now = now_secs() as i64;
    with(|ix| ix.search_named(app_id, app_id, index, query, limit, now))
}

/// Drops the items of apps that are no longer installed (an uninstalled app's items go with it); returns how many apps.
pub fn forget_missing(installed: &[&str]) -> usize {
    with(|ix| ix.forget_missing(installed))
}

/// Items of `app_id` matching `query`, at most `limit`. Expired items are not returned.
pub fn search_app(app_id: &str, query: &str, limit: usize) -> Vec<Found> {
    let now = now_secs();
    with(|ix| {
        ix.prune(now);
        ix.search_app(app_id, query, limit)
    })
}

/// Items matching `query`, at most `limit`, of apps that are still installed. Expired items are not returned.
pub fn search(query: &str, limit: usize) -> Vec<Found> {
    let installed = super::apphost::installed();
    let now = now_secs();
    with(|ix| {
        ix.prune(now);
        ix.search(query, limit + 16)
            .into_iter()
            .filter(|f| installed.iter().any(|a| a.manifest.id == f.app_id))
            .take(limit)
            .collect()
    })
}

/// Saves what changed; cheap when nothing did. The JSON is made under the lock, and the files are written by one saver
/// thread in the order they were taken, so the shell's frame does not wait on the disk.
pub fn flush() {
    let writes = with(|ix| ix.take_writes());
    if writes.is_empty() {
        return;
    }
    let mut job: SaveJob = (dir(), writes);
    let mut saver = SAVER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tx) = saver.as_ref() {
        match tx.send(job) {
            Ok(()) => return,
            Err(std::sync::mpsc::SendError(back)) => job = back,
        }
    }
    let (tx, rx) = std::sync::mpsc::channel::<SaveJob>();
    crate::bare::sys::spawn_named("quicksearch-save", move || {
        for (dir, writes) in rx {
            for (app, text) in writes {
                write_app(&dir, &app, text);
            }
        }
    });
    let _ = tx.send(job);
    *saver = Some(tx);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it(id: &str, domain: &str, title: &str, desc: &str, kw: &[&str]) -> SearchableItem {
        SearchableItem { unique_id: id.into(), domain: domain.into(), title: title.into(), description: desc.into(), keywords: kw.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn title_outranks_keyword_outranks_description() {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "", "Groceries", "buy milk", &[]), it("2", "", "Plan", "", &["milk"]), it("3", "", "Milk run", "", &[])]);
        let ids: Vec<String> = ix.search("milk", 10).into_iter().map(|f| f.unique_id).collect();
        assert_eq!(ids, ["3", "2", "1"]);
        assert!(ix.search("  ", 10).is_empty());
        assert_eq!(ix.search("milk", 1).len(), 1);
    }

    #[test]
    fn same_identifier_replaces_and_deletes_work() {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "d1", "One", "", &[]), it("2", "d2", "Two", "", &[]), it("3", "d1", "Three", "", &[])]);
        ix.index("a", vec![it("1", "d1", "Uno", "", &[])]);
        assert_eq!(ix.len(), 3);
        assert_eq!(ix.search("uno", 5)[0].unique_id, "1");
        assert!(ix.search("one", 5).is_empty(), "the old title is gone");
        ix.delete("a", DELETE_IDS, &["2".into()]);
        assert_eq!(ix.len(), 2);
        ix.delete("a", DELETE_DOMAINS, &["d1".into()]);
        assert_eq!(ix.len(), 0);
        ix.index("a", vec![it("9", "", "Nine", "", &[])]);
        ix.delete("a", DELETE_ALL, &[]);
        assert_eq!(ix.len(), 0);
    }

    #[test]
    fn size_is_bounded_per_app_and_in_apps() {
        let mut ix = Index::default();
        let items: Vec<_> = (0..MAX_PER_APP + 50).map(|i| it(&i.to_string(), "", "t", "", &[])).collect();
        ix.index("a", items);
        assert_eq!(ix.len(), MAX_PER_APP);
        assert!(ix.search("t", 2000).iter().all(|f| f.unique_id.parse::<usize>().unwrap() >= 50), "the oldest were dropped");
        for n in 0..MAX_APPS + 5 {
            ix.index(&format!("app{n}"), vec![it("1", "", "t", "", &[])]);
        }
        assert!(ix.apps.len() <= MAX_APPS);
    }

    #[test]
    fn every_word_must_match_in_any_order() {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "", "Trip to Reykjavík", "", &[]), it("2", "", "Trip home", "Reykjavík hotel", &[]), it("3", "", "Trip home", "", &[])]);
        let ids = |q: &str| -> Vec<String> { ix.search(q, 10).into_iter().map(|f| f.unique_id).collect() };
        assert_eq!(ids("reykjavik trip"), ["1", "2"], "the title match ranks first");
        assert_eq!(ids("trip reyk"), ["1", "2"]);
        assert!(ids("trip paris").is_empty());
        assert!(ids("  ").is_empty());
    }

    #[test]
    fn the_query_language_breaks_ties() {
        let e = Entry::new(it("1", "", "Halló", "", &[]), 0);
        let e = Entry { lang: Some(Language::Icelandic), ..e };
        let words = vec![Matcher::new("hallo")];
        let same = score(&words, &e, Some(Language::Icelandic)).unwrap();
        let other = score(&words, &e, Some(Language::English)).unwrap();
        assert!(!same.2 && other.2);
        assert!(same < other, "an item in the query's language sorts first");
        assert!(score(&words, &e, None).is_some_and(|s| !s.2), "no query language means no penalty");
    }

    #[test]
    fn expired_items_are_dropped_and_the_expiry_is_kept() {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "", "Ticket", "", &[]), it("2", "", "Ticket two", "", &[])]);
        assert!(ix.set_expiry("a", "1", Some(100)));
        assert!(!ix.set_expiry("a", "nope", Some(100)));
        assert!(!ix.set_expiry("zz", "1", None));
        assert_eq!(ix.prune(99), 0);
        assert_eq!(ix.prune(100), 1);
        let left: Vec<String> = ix.search("ticket", 10).into_iter().map(|f| f.unique_id).collect();
        assert_eq!(left, ["2"]);
        assert!(ix.set_expiry("a", "2", Some(500)));
        let back = Index::from_json(&Index::to_json(&ix.apps["a"]));
        assert_eq!(back[0].expires_at, Some(500));
        assert_eq!(ix.prune(500), 1);
        assert_eq!(ix.len(), 0);
    }

    #[test]
    fn search_app_sees_only_its_own_items() {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "", "Mail draft", "", &[])]);
        ix.index("b", vec![it("1", "", "Mail note", "", &[])]);
        assert_eq!(ix.search_app("a", "mail", 10).len(), 1);
        assert_eq!(ix.search_app("a", "mail", 10)[0].app_id, "a");
        assert_eq!(ix.search("mail", 10).len(), 2);
    }

    #[test]
    fn forgetting_missing_apps_drops_only_their_items() {
        let mut ix = Index::default();
        ix.index("notes", vec![it("1", "", "Groceries", "", &[])]);
        ix.index("mail", vec![it("1", "", "Groceries receipt", "", &[])]);
        ix.index("gone", vec![it("1", "", "Groceries list", "", &[])]);
        assert_eq!(ix.forget_missing(&["notes", "mail"]), 1);
        assert_eq!(ix.search_app("gone", "groceries", 10).len(), 0);
        assert_eq!(ix.search("groceries", 10).len(), 2);
        assert_eq!(ix.forget_missing(&["notes", "mail"]), 0, "nothing left to forget");
    }

    #[test]
    fn saves_and_loads_from_disk() {
        let dir = std::env::temp_dir().join(format!("bifrost-spotidx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut ix = Index::default();
        let mut item = it("n\"1", "notes", "Milk \u{fe}\u{e6}r", "oat\nmilk", &["shop", "ü"]);
        item.content_url = "file:///x".into();
        ix.index_at("is.example.notes", vec![item.clone()], 1_700_000_000);
        ix.index("../evil", vec![it("1", "", "x", "", &[])]);
        ix.save(&dir);
        assert!(!dir.join("../evil.json").exists());
        let back = Index::load(&dir);
        assert_eq!(back.apps["is.example.notes"], vec![Entry::new(item, 1_700_000_000)]);
        let mut back = back;
        back.delete("is.example.notes", DELETE_ALL, &[]);
        back.save(&dir);
        assert!(!dir.join("is.example.notes.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 2000-03-01 01:00 UTC, a Wednesday (day 11017 since 1970-01-01).
    const NOW: u64 = 11_017 * DAY + 3_600;

    fn filters(q: &str) -> Filters {
        parse_query(q, NOW)
    }

    fn ids_at(ix: &Index, q: &str) -> Vec<String> {
        ix.search_at(q, 10, NOW).into_iter().map(|f| f.unique_id).collect()
    }

    fn with_url(id: &str, title: &str, url: &str) -> SearchableItem {
        let mut i = it(id, "", title, "", &[]);
        i.content_url = url.into();
        i
    }

    #[test]
    fn civil_dates_match_the_calendar() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        assert_eq!(days_from_civil(2000, 2, 1), 10_988);
        assert_eq!(days_from_civil(1999, 1, 1), 10_592);
        assert_eq!(year_month(0), (1970, 1));
        assert_eq!(year_month(11_017), (2000, 3));
        assert_eq!(year_month(10_988), (2000, 2));
    }

    #[test]
    fn a_file_type_word_is_a_filter_and_a_time_phrase_is_a_window() {
        let f = filters("pdf from last week");
        assert!(f.words.is_empty());
        assert_eq!(f.kinds, vec!["com.adobe.pdf"]);
        // last week: Monday 2000-02-21 (day 11008) up to Monday 2000-02-28 (day 11015)
        assert_eq!(f.window, Some((11_008 * DAY, 11_015 * DAY)));
    }

    #[test]
    fn plain_words_stay_words_and_kinds_are_collected() {
        let f = filters("Trip to Reykjavík photos");
        assert_eq!(f.words, vec!["trip", "to", "reykjavík"]);
        assert_eq!(f.kinds, vec!["public.image"]);
        assert_eq!(f.window, None);
    }

    #[test]
    fn calendar_and_rolling_time_phrases() {
        let window = |q: &str| filters(q).window;
        assert_eq!(window("yesterday"), Some((11_016 * DAY, 11_017 * DAY)));
        assert_eq!(window("today"), Some((11_017 * DAY, u64::MAX)));
        assert_eq!(window("this week"), Some((11_015 * DAY, u64::MAX)));
        assert_eq!(window("last month"), Some((10_988 * DAY, 11_017 * DAY)));
        assert_eq!(window("this month"), Some((11_017 * DAY, u64::MAX)));
        assert_eq!(window("last year"), Some((10_592 * DAY, 10_957 * DAY)));
        assert_eq!(window("past 3 days"), Some((NOW - 3 * DAY, u64::MAX)));
        assert_eq!(window("last 2 weeks"), Some((NOW - 14 * DAY, u64::MAX)));
        assert_eq!(window("past week"), Some((NOW - 7 * DAY, u64::MAX)));
        assert_eq!(window("this"), None, "a bare lead word is text, not a window");
    }

    #[test]
    fn kind_comes_from_the_content_url_extension() {
        assert_eq!(kind_of("file:///home/a/Report.PDF"), Some("com.adobe.pdf"));
        assert_eq!(kind_of("file:///a/b.pdf?x=1"), Some("com.adobe.pdf"));
        assert_eq!(kind_of("file:///x"), None);
        assert_eq!(kind_of("pdf"), None, "a bare word is not a file name");
    }

    #[test]
    fn filter_only_queries_list_matches_newest_first() {
        let mut ix = Index::default();
        // pushed in this order, so the last pushed ("d") is the newest in the list
        // 2000-02-21 (the Monday of last week)
        ix.index_at("a", vec![with_url("b", "Old contract", "file:///c.pdf")], NOW - 7 * DAY - 2 * DAY + 100);
        ix.index_at("a", vec![with_url("a", "Q1 report", "file:///r.pdf")], NOW - 2 * DAY);
        ix.index_at("a", vec![with_url("c", "Holiday", "file:///p.png")], NOW - DAY);
        ix.index_at("a", vec![with_url("d", "Notes", "file:///n.txt")], NOW - DAY);
        assert_eq!(ids_at(&ix, "pdf"), ["a", "b"]);
        assert_eq!(ids_at(&ix, "pdf from last week"), ["b"]);
        assert_eq!(ids_at(&ix, "photos"), ["c"]);
        assert_eq!(ids_at(&ix, "txt"), ["d"]);
        assert_eq!(ids_at(&ix, "documents"), ["d", "a", "b"], "a text file and a PDF are documents, a picture is not");
        assert_eq!(ids_at(&ix, "this week"), ["d", "c", "a"], "the item of last week is out");
        assert_eq!(ids_at(&ix, "notes this week"), ["d"], "a word and a filter both have to hold");
        assert!(ids_at(&ix, "").is_empty());
    }

    #[test]
    fn decodes_a_png_thumbnail_into_a_centred_square() {
        let mut data = Vec::new();
        for _y in 0..2u32 {
            for x in 0..4u32 {
                data.extend_from_slice(&[x as u8 * 60, 1, 2, 255]);
            }
        }
        let mut png_bytes = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png_bytes, 4, 2);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            let mut wr = e.write_header().unwrap();
            wr.write_image_data(&data).unwrap();
        }
        let img = decode_thumbnail(&png_bytes, 4).expect("a 4 by 2 PNG fits a 4 pixel square");
        assert_eq!((img.w, img.h), (4, 4));
        // 4 by 2 scales by 1 and sits in the middle: the top and bottom rows are clear
        assert_eq!(img.px[0], 0);
        assert_eq!(img.px[3 * 4], 0);
        assert_eq!(img.px[4 + 3], rgba(180, 1, 2, 255));
        assert!(decode_thumbnail(b"not a png", 4).is_none());
    }

    #[test]
    fn thumbnails_outside_the_app_data_dir_are_refused() {
        assert!(load_thumbnail("a", "../secret.png", 8).is_none());
        assert!(load_thumbnail("a", "/etc/passwd", 8).is_none());
        assert!(load_thumbnail("../a", "t.png", 8).is_none());
        assert!(load_thumbnail("a", "", 8).is_none());
    }

    fn ids(list: Vec<Found>) -> Vec<String> {
        list.into_iter().map(|f| f.unique_id).collect()
    }

    fn sample() -> Index {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "", "Milk", "", &[]), it("2", "", "Milkshake recipe", "", &[]), it("3", "", "Mild weather", "", &[]), it("4", "", "Smilk", "", &[])]);
        ix.index("b", vec![it("5", "", "Milky way", "", &[]), it("6", "", "Mill plans", "", &[])]);
        ix
    }

    #[test]
    fn a_query_that_extends_the_last_one_ranks_the_same_as_a_fresh_search() {
        let typed = sample();
        let _ = typed.search("m", 50);
        let _ = typed.search("mi", 50);
        for q in ["mil", "milk", "milk s", "milks", "milk sh"] {
            let narrowed = ids(typed.search(q, 50));
            let fresh = ids(sample().search(q, 50));
            assert_eq!(narrowed, fresh, "query {q:?}");
        }
        // a word that does not extend the last one is not narrowed (backspace widens the set again)
        assert_eq!(ids(typed.search("m", 50)), ids(sample().search("m", 50)));
    }

    #[test]
    fn a_change_of_items_drops_the_narrowing() {
        let mut ix = sample();
        assert!(ids(ix.search("milk", 50)).contains(&"1".to_string()));
        ix.index("a", vec![it("9", "", "Milk tea", "", &[])]);
        assert!(ids(ix.search("milk t", 50)).contains(&"9".to_string()), "the new item is found after a narrowing");
        ix.delete("a", DELETE_IDS, &["1".into()]);
        assert!(!ids(ix.search("milk", 50)).contains(&"1".to_string()), "the deleted item is gone");
    }

    #[test]
    fn only_the_best_limit_is_built_in_score_order() {
        let mut ix = Index::default();
        let items: Vec<_> = (0..40).map(|i| it(&i.to_string(), "", &format!("note {}", i % 3), "", &[])).collect();
        ix.index("a", items);
        let all = ids(ix.search("note", 100));
        assert_eq!(all.len(), 40);
        assert_eq!(ids(ix.search("note", 5)), all[..5].to_vec(), "the top five of the full order");
        assert_eq!(ids(ix.search("note", 0)), Vec::<String>::new());
        // the newest comes first among equal matches
        assert_eq!(all[0], "39");
    }

    #[test]
    fn extends_only_when_each_word_grows_its_place() {
        let w = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(extends(&w(&["mi"]), &w(&["mil"])));
        assert!(extends(&w(&["mi"]), &w(&["mi", "x"])));
        assert!(extends(&w(&["a", "mi"]), &w(&["a", "mil"])));
        assert!(!extends(&w(&["mil"]), &w(&["mi"])), "backspace widens");
        assert!(!extends(&w(&["a", "mi"]), &w(&["b", "mil"])));
        assert!(!extends(&w(&["mi", "x"]), &w(&["mi"])));
        assert!(!extends(&[], &w(&["mi"])));
    }

    #[test]
    fn prune_skips_the_walk_until_an_expiry_is_due() {
        let mut ix = Index::default();
        ix.index("a", vec![it("1", "", "Ticket", "", &[]), it("2", "", "Ticket two", "", &[])]);
        assert!(ix.set_expiry("a", "2", Some(300)));
        assert_eq!(ix.prune(100), 0);
        assert_eq!(ix.prune(299), 0);
        assert_eq!(ix.prune(300), 1);
        assert_eq!(ix.prune(400), 0, "nothing left to expire");
        assert_eq!(ix.len(), 1);
    }

    #[test]
    fn saving_takes_the_changed_apps_and_removes_an_emptied_one() {
        let mut ix = sample();
        let writes = ix.take_writes();
        let names: Vec<&str> = writes.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert!(writes.iter().all(|(_, t)| t.as_deref().is_some_and(|s| s.starts_with('['))));
        assert!(ix.take_writes().is_empty(), "nothing changed since");
        ix.delete("b", DELETE_ALL, &[]);
        let writes = ix.take_writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0], ("b".to_string(), None));
    }

    fn named_item(id: &str, title: &str) -> StoredItem {
        StoredItem::new(id, "notes", Record { title: title.into(), description: "d".into(), keywords: vec!["shop".into()] })
    }

    #[test]
    fn named_indexes_keep_their_items_apart_and_only_public_items_reach_other_apps() {
        let mut ix = Index::default();
        let desc = IndexDescription::new("archive").unwrap().protection(ProtectionClass::Complete);
        ix.index_named("notes", &desc, vec![named_item("a", "Milk"), named_item("b", "Tea").public()]).unwrap();
        // the owner sees both, another app only the public one
        assert_eq!(ix.search_named("notes", "notes", "archive", "milk", 5, 0).unwrap().len(), 1);
        assert_eq!(ix.search_named("notes", "notes", "archive", "shop", 5, 0).unwrap().len(), 2);
        assert_eq!(ix.search_named("mail", "notes", "archive", "shop", 5, 0).unwrap().iter().map(|f| f.unique_identifier.as_str()).collect::<Vec<_>>(), ["b"]);
        // the default index is a different index
        assert!(ix.search_named("notes", "notes", "missing", "milk", 5, 0).unwrap().is_empty());
        assert_eq!(ix.search_named("notes", "notes", "archive", "shop", 1, 0).unwrap().len(), 1, "limit caps the hits");
        // a bad name is refused
        assert_eq!(ix.index_named("notes", &IndexDescription { name: "a/b".into(), protection_class: None }, vec![]), Err(NameError::BadCharacter('/')));
    }

    #[test]
    fn named_index_deletes_and_forgetting_an_app_mark_the_files() {
        let mut ix = Index::default();
        let desc = IndexDescription::new("archive").unwrap();
        ix.index_named("notes", &desc, vec![named_item("a", "Milk"), named_item("b", "Tea")]).unwrap();
        ix.named_dirty.clear();
        ix.delete_named_identifiers("notes", "archive", &["a"]);
        assert_eq!(ix.named_dirty, ["notes@archive"]);
        assert!(ix.search_named("notes", "notes", "archive", "milk", 5, 0).unwrap().is_empty());
        ix.named_dirty.clear();
        ix.forget_app("notes");
        assert_eq!(ix.named_dirty, ["notes@archive"]);
        assert!(ix.search_named("notes", "notes", "archive", "tea", 5, 0).unwrap().is_empty());
    }

    #[test]
    fn a_named_index_round_trips_through_its_saved_form() {
        let desc = IndexDescription::new("archive").unwrap().protection(ProtectionClass::CompleteUnlessOpen);
        let mut store = IndexStore::from_items(vec![], Some(desc.protection_class.unwrap()));
        let mut expiring = named_item("a", "Milk \"fresh\"");
        expiring.expiration = Some(1_800_000_000);
        store.index(vec![expiring, named_item("b", "Tea").public()]);
        let back = named_from_json(&named_to_json(&store)).unwrap();
        assert_eq!(back.protection(), Some(ProtectionClass::CompleteUnlessOpen));
        assert_eq!(back.items(), store.items());
        assert!(named_from_json("not json").is_none());
    }
}
