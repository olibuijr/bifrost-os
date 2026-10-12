//! Core Search index in the shell (`CSSearchableIndex`). Apps send `ToShell::SearchIndex` and
//! `SearchDelete`; this keeps a bounded index per app, saved as `<data>/quicksearch-index/<app id>.json`
//! (atomically replaced, only when it changed), and the Search panel searches it ("Documents" section). Opening a
//! result starts the app and sends `ToApp::ContinueActivity` (see `Shell::quicksearch_act`).
//!
//! Ranking: the query is split into words (`bifrost_ui::lexical_tags::words`) and every word must match an item
//! (case and accent folded; prefix, word start or subsequence, see `super::fuzzy`). A title match outranks a keyword
//! match, which outranks a description match; then an item in the query's language (`bifrost_ui::language`) ranks
//! above one in another language; then shorter matches; then the newest item.
//! Expiry: an item can carry a Unix time after which it is dropped (`CSSearchableItem.expirationDate`), set through the
//! `search-index` service (`services/search_ranking.rs`).
//!
//! Bounds: `MAX_PER_APP` items per app (the oldest are dropped first), at most `MAX_APPS` apps, and every field is
//! already cut by the wire codec, so one app cannot grow the index without limit.

use super::fuzzy::Matcher;
use bifrost_core::json::{self, Json};
use bifrost_proto::quicksearch::{SearchableItem, DELETE_ALL, DELETE_DOMAINS, DELETE_IDS};
use bifrost_ui::language::{dominant_language, Language};
use bifrost_ui::lexical_tags;
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
}

impl Entry {
    fn new(item: SearchableItem) -> Entry {
        let text = format!("{} {} {}", item.title, item.keywords.join(" "), item.description);
        Entry {
            title_folded: super::fuzzy::fold(&item.title),
            keywords_folded: super::fuzzy::fold(&item.keywords.join(" ")),
            desc_folded: super::fuzzy::fold(&item.description),
            lang: dominant_language(&text),
            expires_at: None,
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
}

#[derive(Default)]
pub struct Index {
    apps: BTreeMap<String, Vec<Entry>>,
    dirty: Vec<String>,
}

fn safe_name(app_id: &str) -> bool {
    !app_id.is_empty() && app_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_') && !app_id.starts_with('.')
}

impl Index {
    /// `indexSearchableItems`: items with an existing identifier replace the old one (and move to the newest end).
    pub fn index(&mut self, app_id: &str, items: Vec<SearchableItem>) {
        if !self.apps.contains_key(app_id) && self.apps.len() >= MAX_APPS {
            return;
        }
        let list = self.apps.entry(app_id.to_string()).or_default();
        for item in items.into_iter().filter(|i| !i.unique_id.is_empty()) {
            list.retain(|e| e.item.unique_id != item.unique_id);
            list.push(Entry::new(item));
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
        if !self.dirty.iter().any(|d| d == app_id) {
            self.dirty.push(app_id.to_string());
        }
    }

    pub fn len(&self) -> usize {
        self.apps.values().map(Vec::len).sum()
    }

    /// The best `limit` items for `query` (every word of it must match): see the module notes on ranking.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Found> {
        self.rank_hits(None, query, limit)
    }

    /// Like `search`, over the items of `app_id` only.
    pub fn search_app(&self, app_id: &str, query: &str, limit: usize) -> Vec<Found> {
        self.rank_hits(Some(app_id), query, limit)
    }

    fn rank_hits(&self, only: Option<&str>, query: &str, limit: usize) -> Vec<Found> {
        let words: Vec<Matcher> = lexical_tags::words(query).into_iter().map(|(w, _)| Matcher::new(w)).collect();
        if words.is_empty() {
            return vec![];
        }
        let query_lang = dominant_language(query);
        let mut hits = vec![];
        for (app, list) in &self.apps {
            if only.map_or(false, |a| a != app.as_str()) {
                continue;
            }
            for (age, e) in list.iter().enumerate().rev() {
                if let Some(key) = score(&words, e, query_lang) {
                    hits.push(((key, std::cmp::Reverse(age)), Found { app_id: app.clone(), unique_id: e.item.unique_id.clone(), domain: e.item.domain.clone(), title: e.item.title.clone() }));
                }
            }
        }
        hits.sort_by(|a, b| a.0.cmp(&b.0));
        hits.into_iter().take(limit).map(|(_, f)| f).collect()
    }

    /// Sets (or with `None` clears) the expiry of one item. False when the app has no item with that identifier.
    pub fn set_expiry(&mut self, app_id: &str, unique_id: &str, at: Option<u64>) -> bool {
        let Some(e) = self.apps.get_mut(app_id).and_then(|l| l.iter_mut().find(|e| e.item.unique_id == unique_id)) else {
            return false;
        };
        e.expires_at = at;
        self.mark(app_id);
        true
    }

    /// Drops the items whose expiry is at or before `now` (Unix seconds). Returns how many went.
    pub fn prune(&mut self, now: u64) -> usize {
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
                format!("{{\"id\":{},\"domain\":{},\"title\":{},\"desc\":{},\"keywords\":[{}],\"thumb\":{},\"url\":{}{}}}", q(&i.unique_id), q(&i.domain), q(&i.title), q(&i.description), kw.join(","), q(&i.thumbnail), q(&i.content_url), expires)
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
                let mut e = Entry::new(SearchableItem {
                    unique_id,
                    domain: o.str_field("domain").unwrap_or_default(),
                    title: o.str_field("title").unwrap_or_default(),
                    description: o.str_field("desc").unwrap_or_default(),
                    keywords,
                    thumbnail: o.str_field("thumb").unwrap_or_default(),
                    content_url: o.str_field("url").unwrap_or_default(),
                });
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

    /// Writes the apps that changed since the last call (and removes the file of an emptied app).
    pub fn save(&mut self, dir: &Path) {
        for app in std::mem::take(&mut self.dirty) {
            if !safe_name(&app) {
                continue;
            }
            let path = dir.join(format!("{app}.json"));
            match self.apps.get(&app) {
                Some(list) => {
                    let _ = std::fs::create_dir_all(dir);
                    let tmp = dir.join(format!("{app}.json.tmp"));
                    if std::fs::write(&tmp, Index::to_json(list)).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                        let _ = std::fs::remove_file(&tmp);
                    }
                }
                None => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }

    /// Drops the items of an app that was uninstalled.
    pub fn forget_app(&mut self, app_id: &str) {
        if self.apps.remove(app_id).is_some() {
            self.mark(app_id);
        }
    }
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

/// Saves what changed; called every tick, cheap when nothing did.
pub fn flush() {
    let d = dir();
    with(|ix| ix.save(&d));
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
        let e = Entry::new(it("1", "", "Halló", "", &[]));
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
    fn saves_and_loads_from_disk() {
        let dir = std::env::temp_dir().join(format!("bifrost-spotidx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut ix = Index::default();
        let mut item = it("n\"1", "notes", "Milk \u{fe}\u{e6}r", "oat\nmilk", &["shop", "ü"]);
        item.content_url = "file:///x".into();
        ix.index("is.example.notes", vec![item.clone()]);
        ix.index("../evil", vec![it("1", "", "x", "", &[])]);
        ix.save(&dir);
        assert!(!dir.join("../evil.json").exists());
        let back = Index::load(&dir);
        assert_eq!(back.apps["is.example.notes"], vec![Entry::new(item)]);
        let mut back = back;
        back.delete("is.example.notes", DELETE_ALL, &[]);
        back.save(&dir);
        assert!(!dir.join("is.example.notes.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
