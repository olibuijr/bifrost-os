//! On-demand resources: an app ships a small base package and names the rest by tag; a tag's payload is fetched when
//! the app asks for it, kept while the app uses it, and purged when the disk budget needs room (the job of the
//! reference OS's on-demand resource service and its bundle resource request).
//!
//! A [`Catalog`] lists the tags an app has: each tag is one payload with a size and a SHA-256 digest
//! ([`DownloadSpec`]) and a preservation priority from 0.0 (purge first) to 1.0 (keep as long as possible). A
//! [`ResourceStore`] keeps the installed payloads under `<root>/<tag>/<name>` and decides what stays. A
//! [`ResourceRequest`] names tags the app wants: [`ResourceStore::begin_accessing`] fetches what is missing (through a
//! caller-supplied fetch, so the store needs no network of its own and the app or the PC App Store decides where the
//! bytes come from), verifies it against the digest, installs it and pins every tag of the request. A pinned tag is
//! never purged. [`ResourceStore::end_accessing`] unpins; [`ResourceStore::purge`] and the room-making inside
//! `begin_accessing` drop unpinned tags, lowest preservation priority first and the least recently used first among
//! equals.
//!
//! Mirrors `NSBundleResourceRequest` (`beginAccessingResources`, `conditionallyBeginAccessingResources`,
//! `endAccessingResources`, `loadingPriority`, `progress`) and the preservation priority of a tag.
//!
//! Gaps: fetches run on the calling thread in the order the tags are listed (the loading priority is recorded for a
//! scheduler, not used to reorder), a tag is one payload rather than a set of files, and a payload is installed
//! as given (an app that ships an archive unpacks it itself).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::background_download::{stage, DownloadError, DownloadSpec};
use bifrost_core::json::{self, Json};

/// Why a call was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum OdrError {
    /// A tag is empty, too long, or holds a character other than letters, digits, `.`, `_` and `-`.
    BadTag(String),
    /// The catalog does not list the tag.
    UnknownTag(String),
    /// The fetch for a tag failed.
    Fetch { tag: String, message: String },
    /// The payload failed its size or digest check, or could not be written.
    Download(DownloadError),
    /// There is no room for the payload even after purging every unpinned tag.
    OutOfSpace { needed: u64 },
    /// A catalog text could not be read.
    BadCatalog(String),
    /// The store directory could not be made or removed.
    Io(String),
}

/// One tag: its payload and how long it should stay.
#[derive(Clone, Debug, PartialEq)]
pub struct TagEntry {
    pub spec: DownloadSpec,
    /// 0.0 is purged first, 1.0 last
    pub preservation: f64,
}

/// The tags an app has.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    tags: BTreeMap<String, TagEntry>,
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 64 && tag.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) && tag != "." && tag != ".."
}

impl Catalog {
    pub fn new() -> Catalog {
        Catalog::default()
    }

    /// Adds a tag. A tag already listed is replaced.
    pub fn add(&mut self, tag: &str, spec: DownloadSpec, preservation: f64) -> Result<(), OdrError> {
        if !valid_tag(tag) {
            return Err(OdrError::BadTag(tag.to_string()));
        }
        self.tags.insert(tag.to_string(), TagEntry { spec, preservation: preservation.clamp(0.0, 1.0) });
        Ok(())
    }

    pub fn get(&self, tag: &str) -> Option<&TagEntry> {
        self.tags.get(tag)
    }

    /// The tags, in name order.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.tags.keys().map(String::as_str)
    }

    /// Reads `{"tags":[{"tag":"level-2","name":"level2.pack","size":1024,"sha256":"<64 hex>","preservation":0.5}]}`.
    /// The preservation priority defaults to 0.5.
    pub fn parse(text: &str) -> Result<Catalog, OdrError> {
        let bad = |m: &str| OdrError::BadCatalog(m.to_string());
        let root = json::parse(text).map_err(|e| OdrError::BadCatalog(e))?;
        let list = root.get("tags").and_then(Json::as_array).ok_or_else(|| bad("no tags list"))?;
        let mut catalog = Catalog::new();
        for item in list {
            let tag = item.str_field("tag").ok_or_else(|| bad("a tag has no name"))?;
            let name = item.str_field("name").ok_or_else(|| bad("a tag has no file name"))?;
            let size = item.get("size").and_then(Json::as_u64).ok_or_else(|| bad("a tag has no size"))?;
            let digest = item.str_field("sha256").ok_or_else(|| bad("a tag has no digest"))?;
            let spec = DownloadSpec::new(&name, size, &digest).map_err(OdrError::Download)?;
            let preservation = item.get("preservation").and_then(Json::number).unwrap_or(0.5);
            catalog.add(&tag, spec, preservation)?;
        }
        Ok(catalog)
    }
}

/// A request for tagged resources.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceRequest {
    tags: Vec<String>,
    /// 0.0 to 1.0; recorded for a scheduler
    pub loading_priority: f64,
    accessing: bool,
}

impl ResourceRequest {
    /// A request for `tags`, duplicates dropped, in the order given.
    pub fn new(tags: &[&str]) -> ResourceRequest {
        let mut list: Vec<String> = Vec::new();
        for t in tags {
            if !list.iter().any(|x| x == t) {
                list.push((*t).to_string());
            }
        }
        ResourceRequest { tags: list, loading_priority: 0.5, accessing: false }
    }

    pub fn with_loading_priority(mut self, priority: f64) -> ResourceRequest {
        self.loading_priority = priority.clamp(0.0, 1.0);
        self
    }

    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Whether the request holds its tags (between `begin_accessing` and `end_accessing`).
    pub fn is_accessing(&self) -> bool {
        self.accessing
    }
}

/// The fetch the store calls for a missing tag: the payload bytes, or a reason it could not be had.
pub type Fetch<'a> = &'a mut dyn FnMut(&str, &DownloadSpec) -> Result<Vec<u8>, String>;

/// The installed payloads and what keeps them.
#[derive(Debug)]
pub struct ResourceStore {
    root: PathBuf,
    budget: u64,
    catalog: Catalog,
    pins: BTreeMap<String, u32>,
    last_used: BTreeMap<String, u64>,
    tick: u64,
}

impl ResourceStore {
    /// A store under `root` that keeps at most `budget` bytes of payloads.
    pub fn new(root: &Path, budget: u64, catalog: Catalog) -> Result<ResourceStore, OdrError> {
        fs::create_dir_all(root).map_err(|e| OdrError::Io(e.to_string()))?;
        Ok(ResourceStore { root: root.to_path_buf(), budget, catalog, pins: BTreeMap::new(), last_used: BTreeMap::new(), tick: 0 })
    }

    fn file_of(&self, tag: &str) -> Option<PathBuf> {
        let entry = self.catalog.get(tag)?;
        Some(self.root.join(tag).join(entry.spec.name()))
    }

    /// Whether the tag's payload is on disk at its full size.
    pub fn is_installed(&self, tag: &str) -> bool {
        let (Some(path), Some(entry)) = (self.file_of(tag), self.catalog.get(tag)) else { return false };
        fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == entry.spec.expected_size())
    }

    /// The installed payload of a tag.
    pub fn path_of(&self, tag: &str) -> Option<PathBuf> {
        self.is_installed(tag).then(|| self.file_of(tag)).flatten()
    }

    /// Bytes the installed payloads take.
    pub fn installed_bytes(&self) -> u64 {
        self.catalog.tags().filter(|t| self.is_installed(t)).filter_map(|t| self.catalog.get(t)).map(|e| e.spec.expected_size()).sum()
    }

    /// The share of the request's bytes that is installed, 0.0 to 1.0 (a request for nothing is complete).
    pub fn progress(&self, request: &ResourceRequest) -> f32 {
        let mut total = 0u64;
        let mut have = 0u64;
        for t in request.tags() {
            if let Some(e) = self.catalog.get(t) {
                total += e.spec.expected_size();
                if self.is_installed(t) {
                    have += e.spec.expected_size();
                }
            }
        }
        if total == 0 {
            1.0
        } else {
            have as f32 / total as f32
        }
    }

    fn touch(&mut self, tag: &str) {
        self.tick += 1;
        self.last_used.insert(tag.to_string(), self.tick);
    }

    fn pin(&mut self, tag: &str) {
        *self.pins.entry(tag.to_string()).or_insert(0) += 1;
        self.touch(tag);
    }

    fn unpin(&mut self, tag: &str) {
        if let Some(n) = self.pins.get_mut(tag) {
            *n -= 1;
            if *n == 0 {
                self.pins.remove(tag);
            }
        }
    }

    /// Whether any request holds the tag.
    pub fn is_pinned(&self, tag: &str) -> bool {
        self.pins.contains_key(tag)
    }

    /// The unpinned installed tag to drop next: lowest preservation priority, then least recently used.
    fn next_victim(&self) -> Option<String> {
        self.catalog
            .tags()
            .filter(|t| self.is_installed(t) && !self.is_pinned(t))
            .min_by(|a, b| {
                let (pa, pb) = (self.catalog.get(a).map_or(0.0, |e| e.preservation), self.catalog.get(b).map_or(0.0, |e| e.preservation));
                pa.total_cmp(&pb).then_with(|| self.last_used.get(*a).copied().unwrap_or(0).cmp(&self.last_used.get(*b).copied().unwrap_or(0)))
            })
            .map(str::to_string)
    }

    fn evict(&mut self, tag: &str) {
        let _ = fs::remove_dir_all(self.root.join(tag));
        self.last_used.remove(tag);
    }

    /// Drops unpinned tags until the installed bytes plus `extra` fit the budget. False when they still do not.
    fn make_room(&mut self, extra: u64, evicted: &mut Vec<String>) -> bool {
        while self.installed_bytes() + extra > self.budget {
            let Some(victim) = self.next_victim() else { return false };
            self.evict(&victim);
            evicted.push(victim);
        }
        true
    }

    /// Drops unpinned tags until the installed bytes fit the budget; returns the tags dropped, in the order dropped.
    pub fn purge(&mut self) -> Vec<String> {
        let mut evicted = Vec::new();
        self.make_room(0, &mut evicted);
        evicted
    }

    /// Fetches what is missing, installs it, and pins every tag of the request. On a failure nothing stays pinned by
    /// this call; tags installed before the failure stay on disk. A request that is already accessing does nothing.
    pub fn begin_accessing(&mut self, request: &mut ResourceRequest, fetch: Fetch<'_>) -> Result<(), OdrError> {
        if request.accessing {
            return Ok(());
        }
        for t in request.tags() {
            if self.catalog.get(t).is_none() {
                return Err(OdrError::UnknownTag(t.clone()));
            }
        }
        // hold the tags already here first, so making room for the others cannot drop them
        let mut held: Vec<String> = Vec::new();
        for t in request.tags.clone() {
            if self.is_installed(&t) {
                self.pin(&t);
                held.push(t);
            }
        }
        for t in request.tags.clone() {
            if held.contains(&t) {
                continue;
            }
            match self.install(&t, fetch) {
                Ok(()) => {
                    self.pin(&t);
                    held.push(t);
                }
                Err(e) => {
                    for h in held {
                        self.unpin(&h);
                    }
                    return Err(e);
                }
            }
        }
        request.accessing = true;
        Ok(())
    }

    fn install(&mut self, tag: &str, fetch: Fetch<'_>) -> Result<(), OdrError> {
        let spec = self.catalog.get(tag).map(|e| e.spec.clone()).ok_or_else(|| OdrError::UnknownTag(tag.to_string()))?;
        let mut evicted = Vec::new();
        if !self.make_room(spec.expected_size(), &mut evicted) {
            return Err(OdrError::OutOfSpace { needed: spec.expected_size() });
        }
        let bytes = fetch(tag, &spec).map_err(|message| OdrError::Fetch { tag: tag.to_string(), message })?;
        let dir = self.root.join(tag);
        // a partial or wrong-size leftover of an earlier try is replaced
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).map_err(|e| OdrError::Io(e.to_string()))?;
        if let Err(e) = stage(&dir, &spec, &bytes) {
            let _ = fs::remove_dir_all(&dir);
            return Err(OdrError::Download(e));
        }
        Ok(())
    }

    /// Pins the request's tags only when every one is already installed; never fetches. True when it now holds them.
    pub fn conditionally_begin_accessing(&mut self, request: &mut ResourceRequest) -> bool {
        if request.accessing {
            return true;
        }
        if request.tags().iter().any(|t| !self.is_installed(t)) {
            return false;
        }
        for t in request.tags.clone() {
            self.pin(&t);
        }
        request.accessing = true;
        true
    }

    /// Releases the request's tags. They stay installed until the budget needs the room.
    pub fn end_accessing(&mut self, request: &mut ResourceRequest) {
        if !request.accessing {
            return;
        }
        for t in request.tags.clone() {
            self.unpin(&t);
        }
        request.accessing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::digest::sha256;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-odr-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn payload(tag: &str, len: usize) -> Vec<u8> {
        tag.bytes().cycle().take(len).collect()
    }

    fn spec_for(tag: &str, len: usize) -> DownloadSpec {
        DownloadSpec::new(&format!("{tag}.pack"), len as u64, &hex(&sha256(&payload(tag, len)))).unwrap()
    }

    fn catalog(tags: &[(&str, usize, f64)]) -> Catalog {
        let mut c = Catalog::new();
        for (t, len, p) in tags {
            c.add(t, spec_for(t, *len), *p).unwrap();
        }
        c
    }

    /// A fetch that serves the generated payloads and records the tags it was asked for.
    fn serving(log: &mut Vec<String>) -> impl FnMut(&str, &DownloadSpec) -> Result<Vec<u8>, String> + '_ {
        move |tag, spec| {
            log.push(tag.to_string());
            Ok(payload(tag, spec.expected_size() as usize))
        }
    }

    #[test]
    fn tags_are_validated_and_the_catalog_reads_json() {
        let mut c = Catalog::new();
        assert_eq!(c.add("a/b", spec_for("x", 3), 0.5), Err(OdrError::BadTag("a/b".into())));
        assert_eq!(c.add("", spec_for("x", 3), 0.5), Err(OdrError::BadTag(String::new())));
        c.add("ok-1.2_x", spec_for("x", 3), 7.0).unwrap();
        assert_eq!(c.get("ok-1.2_x").unwrap().preservation, 1.0, "priority is clamped");
        let digest = hex(&sha256(b"abc"));
        let text = format!("{{\"tags\":[{{\"tag\":\"level-2\",\"name\":\"level2.pack\",\"size\":3,\"sha256\":\"{digest}\"}}]}}");
        let parsed = Catalog::parse(&text).unwrap();
        assert_eq!(parsed.get("level-2").unwrap().preservation, 0.5);
        assert_eq!(parsed.tags().collect::<Vec<_>>(), ["level-2"]);
        assert!(matches!(Catalog::parse("{}"), Err(OdrError::BadCatalog(_))));
        assert!(matches!(Catalog::parse("{\"tags\":[{\"tag\":\"t\"}]}"), Err(OdrError::BadCatalog(_))));
    }

    #[test]
    fn a_request_fetches_what_is_missing_once_and_pins_it() {
        let dir = scratch("fetch");
        let mut store = ResourceStore::new(&dir, 1000, catalog(&[("a", 100, 0.5), ("b", 200, 0.5)])).unwrap();
        let mut log = Vec::new();
        let mut req = ResourceRequest::new(&["a", "b", "a"]);
        assert_eq!(req.tags(), ["a", "b"], "duplicates are dropped");
        assert_eq!(store.progress(&req), 0.0);
        store.begin_accessing(&mut req, &mut serving(&mut log)).unwrap();
        assert!(req.is_accessing() && store.is_pinned("a") && store.is_pinned("b"));
        assert_eq!(log, ["a", "b"]);
        assert_eq!(store.installed_bytes(), 300);
        assert_eq!(store.progress(&req), 1.0);
        assert_eq!(fs::read(store.path_of("b").unwrap()).unwrap(), payload("b", 200));
        // a second request for an installed tag fetches nothing
        let mut again = ResourceRequest::new(&["a"]);
        store.begin_accessing(&mut again, &mut serving(&mut log)).unwrap();
        assert_eq!(log, ["a", "b"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn conditional_access_never_fetches() {
        let dir = scratch("cond");
        let mut store = ResourceStore::new(&dir, 1000, catalog(&[("a", 10, 0.5), ("b", 10, 0.5)])).unwrap();
        let mut log = Vec::new();
        let mut first = ResourceRequest::new(&["a"]);
        store.begin_accessing(&mut first, &mut serving(&mut log)).unwrap();
        store.end_accessing(&mut first);
        let mut both = ResourceRequest::new(&["a", "b"]);
        assert!(!store.conditionally_begin_accessing(&mut both));
        assert!(!both.is_accessing() && !store.is_pinned("a"));
        let mut one = ResourceRequest::new(&["a"]);
        assert!(store.conditionally_begin_accessing(&mut one));
        assert!(store.is_pinned("a"));
        assert_eq!(log, ["a"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pins_count_and_release_with_the_last_request() {
        let dir = scratch("pins");
        let mut store = ResourceStore::new(&dir, 1000, catalog(&[("a", 10, 0.5)])).unwrap();
        let mut log = Vec::new();
        let (mut r1, mut r2) = (ResourceRequest::new(&["a"]), ResourceRequest::new(&["a"]));
        store.begin_accessing(&mut r1, &mut serving(&mut log)).unwrap();
        store.begin_accessing(&mut r2, &mut serving(&mut log)).unwrap();
        store.end_accessing(&mut r1);
        assert!(store.is_pinned("a"));
        store.end_accessing(&mut r1);
        assert!(store.is_pinned("a"), "ending twice does not release the other request's hold");
        store.end_accessing(&mut r2);
        assert!(!store.is_pinned("a") && store.is_installed("a"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn purging_drops_the_lowest_priority_then_the_oldest_and_never_a_pinned_tag() {
        let dir = scratch("purge");
        let mut store = ResourceStore::new(&dir, 1000, catalog(&[("keep", 300, 0.9), ("old", 300, 0.2), ("new", 300, 0.2), ("held", 300, 0.0)])).unwrap();
        let mut log = Vec::new();
        for t in ["old", "new", "keep"] {
            let mut r = ResourceRequest::new(&[t]);
            store.begin_accessing(&mut r, &mut serving(&mut log)).unwrap();
            store.end_accessing(&mut r);
        }
        assert_eq!(store.installed_bytes(), 900);
        // the held tag has the lowest priority but is pinned, so room comes from "old", the oldest of the 0.2s
        let mut held = ResourceRequest::new(&["held"]);
        store.begin_accessing(&mut held, &mut serving(&mut log)).unwrap();
        assert!(!store.is_installed("old") && store.is_installed("new") && store.is_installed("keep") && store.is_installed("held"));
        // everything left is pinned by one request or another, and the installed bytes fit the budget
        let mut all = ResourceRequest::new(&["keep", "new", "held"]);
        store.end_accessing(&mut held);
        store.begin_accessing(&mut all, &mut serving(&mut log)).unwrap();
        assert!(store.purge().is_empty(), "everything left is pinned and within the budget");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_request_larger_than_the_unpinned_room_is_refused_and_leaves_no_pins() {
        let dir = scratch("space");
        let mut store = ResourceStore::new(&dir, 500, catalog(&[("a", 300, 0.5), ("b", 300, 0.5)])).unwrap();
        let mut log = Vec::new();
        let mut req = ResourceRequest::new(&["a", "b"]);
        let err = store.begin_accessing(&mut req, &mut serving(&mut log)).unwrap_err();
        assert_eq!(err, OdrError::OutOfSpace { needed: 300 });
        assert!(!req.is_accessing() && !store.is_pinned("a") && !store.is_pinned("b"));
        assert!(store.is_installed("a"), "the tag fetched before the failure stays on disk");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_payloads_and_failed_fetches_are_refused_without_a_trace() {
        let dir = scratch("bad");
        let mut store = ResourceStore::new(&dir, 1000, catalog(&[("a", 10, 0.5)])).unwrap();
        let mut wrong = |_: &str, _: &DownloadSpec| -> Result<Vec<u8>, String> { Ok(vec![0u8; 10]) };
        let mut req = ResourceRequest::new(&["a"]);
        assert!(matches!(store.begin_accessing(&mut req, &mut wrong), Err(OdrError::Download(DownloadError::DigestMismatch { .. }))));
        assert!(!store.is_installed("a") && !dir.join("a").exists());
        let mut down = |_: &str, _: &DownloadSpec| -> Result<Vec<u8>, String> { Err("offline".to_string()) };
        assert_eq!(store.begin_accessing(&mut req, &mut down), Err(OdrError::Fetch { tag: "a".into(), message: "offline".into() }));
        let mut unknown = ResourceRequest::new(&["nope"]);
        assert_eq!(store.begin_accessing(&mut unknown, &mut down), Err(OdrError::UnknownTag("nope".into())));
        assert!(!req.is_accessing());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_short_leftover_file_is_not_installed_and_is_fetched_again() {
        let dir = scratch("short");
        let mut store = ResourceStore::new(&dir, 1000, catalog(&[("a", 50, 0.5)])).unwrap();
        fs::create_dir_all(dir.join("a")).unwrap();
        fs::write(dir.join("a").join("a.pack"), b"cut").unwrap();
        assert!(!store.is_installed("a"));
        let mut log = Vec::new();
        let mut req = ResourceRequest::new(&["a"]);
        store.begin_accessing(&mut req, &mut serving(&mut log)).unwrap();
        assert_eq!(log, ["a"]);
        assert!(store.is_installed("a"));
        let _ = fs::remove_dir_all(&dir);
    }
}
