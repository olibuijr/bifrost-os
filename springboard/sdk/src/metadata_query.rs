//! File metadata queries (Foundation, "File System"): `MetadataQuery` finds files under search scopes whose
//! attributes satisfy a predicate, gathers the first results, and keeps them current as files change.
//!
//! It is a separate, file-system-only search: it walks the scopes itself and does not use the content index
//! (`quicksearch`) or the UI metadata query.
//!
//! - `predicate` is a [`MetadataPredicate`] built from comparisons on [`MetadataAttribute`]s joined with and, or
//!   and not. No predicate matches every item. `search_scopes` are directories; the scopes themselves are not
//!   results, everything below them is (links are listed, never entered).
//! - `start_query` gathers synchronously and queues `DidStartGathering`, `GatheringProgress` and
//!   `DidFinishGathering`. `is_gathering` stays true until the finish notification is delivered by `poll`.
//! - After that, a [`WatchStream`] over the scopes marks the query dirty; the next `poll` that finds it dirty
//!   rescans, diffs against the previous results and delivers `DidUpdate` with the added, changed and removed paths.
//!   `disable_updates` holds updates back (changes pile up) and `enable_updates` lets the next poll deliver them.
//! - A [`MetadataItem`] carries the attribute values read when it was gathered.
//!
//! The owner calls `poll(now)` from its tick and wakes on `fd()` and `next_deadline(now)`, exactly as for the
//! watch stream.
//!
//! Results are ordered by the query's [`MetadataSortDescriptor`]s and then by path.
//!
//! Gaps: no grouping or value lists, no predicate text parser, no `Foundation`-style key paths,
//! and no content attributes (the content index is not consulted).

use std::cmp::Ordering;
use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::fswatch::{WatchStream, WatchStreamFlags};

/// How long file system events are coalesced before a rescan.
const UPDATE_LATENCY: Duration = Duration::from_millis(30);

/// An attribute a query can test and an item carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataAttribute {
    /// The file name.
    FsName,
    DisplayName,
    /// The absolute path.
    Path,
    /// Size in bytes.
    FsSize,
    /// Last content change, seconds since 1970.
    FsContentChangeDate,
    /// A type identifier derived from the extension.
    ContentType,
    IsDirectory,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MetadataValue {
    Str(String),
    Int(i64),
    /// Seconds since 1970.
    Date(f64),
    Bool(bool),
}

impl MetadataValue {
    fn number(&self) -> Option<f64> {
        match self {
            MetadataValue::Int(i) => Some(*i as f64),
            MetadataValue::Date(d) => Some(*d),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataOp {
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    /// Strings only: `*` matches any run of characters and `?` any one.
    Like,
    BeginsWith,
    EndsWith,
    Contains,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MetadataPredicate {
    Compare(MetadataAttribute, MetadataOp, MetadataValue),
    And(Vec<MetadataPredicate>),
    Or(Vec<MetadataPredicate>),
    Not(Box<MetadataPredicate>),
}

impl MetadataPredicate {
    pub fn compare(attribute: MetadataAttribute, op: MetadataOp, value: MetadataValue) -> MetadataPredicate {
        MetadataPredicate::Compare(attribute, op, value)
    }

    pub fn matches(&self, item: &MetadataItem) -> bool {
        match self {
            MetadataPredicate::And(all) => all.iter().all(|p| p.matches(item)),
            MetadataPredicate::Or(any) => any.iter().any(|p| p.matches(item)),
            MetadataPredicate::Not(p) => !p.matches(item),
            MetadataPredicate::Compare(attribute, op, want) => match item.value(*attribute) {
                Some(have) => compare(have, *op, want),
                None => false,
            },
        }
    }
}

fn compare(have: &MetadataValue, op: MetadataOp, want: &MetadataValue) -> bool {
    match (have, want) {
        (MetadataValue::Str(a), MetadataValue::Str(b)) => match op {
            MetadataOp::Equal => a == b,
            MetadataOp::NotEqual => a != b,
            MetadataOp::Less => a < b,
            MetadataOp::LessOrEqual => a <= b,
            MetadataOp::Greater => a > b,
            MetadataOp::GreaterOrEqual => a >= b,
            MetadataOp::Like => glob(b.as_bytes(), a.as_bytes()),
            MetadataOp::BeginsWith => a.starts_with(b.as_str()),
            MetadataOp::EndsWith => a.ends_with(b.as_str()),
            MetadataOp::Contains => a.contains(b.as_str()),
        },
        (MetadataValue::Bool(a), MetadataValue::Bool(b)) => match op {
            MetadataOp::Equal => a == b,
            MetadataOp::NotEqual => a != b,
            _ => false,
        },
        (a, b) => match (a.number(), b.number()) {
            (Some(x), Some(y)) => match (x.partial_cmp(&y), op) {
                (Some(Ordering::Equal), MetadataOp::Equal | MetadataOp::LessOrEqual | MetadataOp::GreaterOrEqual) => true,
                (Some(Ordering::Less), MetadataOp::Less | MetadataOp::LessOrEqual | MetadataOp::NotEqual) => true,
                (Some(Ordering::Greater), MetadataOp::Greater | MetadataOp::GreaterOrEqual | MetadataOp::NotEqual) => true,
                _ => false,
            },
            _ => false,
        },
    }
}

/// One sort key of a query (`NSSortDescriptor` over a metadata attribute). Items are ordered by the first key, ties
/// by the next key, and finally by path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataSortDescriptor {
    pub attribute: MetadataAttribute,
    pub ascending: bool,
}

impl MetadataSortDescriptor {
    pub fn ascending(attribute: MetadataAttribute) -> MetadataSortDescriptor {
        MetadataSortDescriptor { attribute, ascending: true }
    }

    pub fn descending(attribute: MetadataAttribute) -> MetadataSortDescriptor {
        MetadataSortDescriptor { attribute, ascending: false }
    }
}

/// Orders two values of one attribute. Strings compare by code point, numbers and dates by value, booleans with
/// false first. An item without the attribute sorts before one with it, and values of different kinds are equal.
fn order_values(a: Option<&MetadataValue>, b: Option<&MetadataValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(MetadataValue::Str(x)), Some(MetadataValue::Str(y))) => x.cmp(y),
        (Some(MetadataValue::Bool(x)), Some(MetadataValue::Bool(y))) => x.cmp(y),
        (Some(x), Some(y)) => match (x.number(), y.number()) {
            (Some(p), Some(q)) => p.partial_cmp(&q).unwrap_or(Ordering::Equal),
            _ => Ordering::Equal,
        },
    }
}

/// Sorts `items` by `descriptors`, then by path. The sort is stable.
fn sort_items(descriptors: &[MetadataSortDescriptor], items: &mut [MetadataItem]) {
    items.sort_by(|a, b| {
        for d in descriptors {
            let ord = order_values(a.value(d.attribute), b.value(d.attribute));
            let ord = if d.ascending { ord } else { ord.reverse() };
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.path.cmp(&b.path)
    });
}

/// Wildcard match of `text` against `pattern` (`*` any run, `?` any one byte).
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

/// One result and its attributes (`MetadataItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataItem {
    pub path: PathBuf,
    attributes: BTreeMap<MetadataAttribute, MetadataValue>,
}

impl MetadataItem {
    /// `value(forAttribute:)`.
    pub fn value(&self, attribute: MetadataAttribute) -> Option<&MetadataValue> {
        self.attributes.get(&attribute)
    }

    /// `attributes`: the attributes this item has.
    pub fn attributes(&self) -> Vec<MetadataAttribute> {
        self.attributes.keys().copied().collect()
    }

    fn read(path: &Path) -> Option<MetadataItem> {
        let meta = fs::symlink_metadata(path).ok()?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        let is_dir = meta.is_dir();
        let mut attributes = BTreeMap::new();
        attributes.insert(MetadataAttribute::FsName, MetadataValue::Str(name.clone()));
        attributes.insert(MetadataAttribute::DisplayName, MetadataValue::Str(name.clone()));
        attributes.insert(MetadataAttribute::Path, MetadataValue::Str(path.to_string_lossy().into_owned()));
        attributes.insert(MetadataAttribute::FsSize, MetadataValue::Int(meta.len() as i64));
        attributes
            .insert(MetadataAttribute::FsContentChangeDate, MetadataValue::Date(meta.mtime() as f64 + meta.mtime_nsec() as f64 / 1e9));
        attributes.insert(MetadataAttribute::ContentType, MetadataValue::Str(content_type(&name, is_dir)));
        attributes.insert(MetadataAttribute::IsDirectory, MetadataValue::Bool(is_dir));
        Some(MetadataItem { path: path.to_path_buf(), attributes })
    }
}

fn content_type(name: &str, is_dir: bool) -> String {
    if is_dir {
        return "public.folder".into();
    }
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "txt" | "text" => "public.plain-text",
        "md" | "markdown" => "public.markdown",
        "png" => "public.png",
        "jpg" | "jpeg" => "public.jpeg",
        "gif" => "public.gif",
        "pdf" => "public.pdf",
        "json" => "public.json",
        "html" | "htm" => "public.html",
        "mp3" => "public.mp3",
        "mp4" | "m4v" => "public.mpeg-4",
        "zip" => "public.zip-archive",
        _ => "public.data",
    }
    .to_string()
}

/// What `poll` delivers.
#[derive(Clone, Debug, PartialEq)]
pub enum MetadataNotification {
    DidStartGathering,
    GatheringProgress,
    DidFinishGathering,
    /// Results changed after the first gather. Paths are sorted.
    DidUpdate { added: Vec<PathBuf>, changed: Vec<PathBuf>, removed: Vec<PathBuf> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Stopped,
    Gathering,
    Started,
}

/// The file metadata query: predicate, scopes, gathering and live updates.
pub struct MetadataQuery {
    pub predicate: Option<MetadataPredicate>,
    pub search_scopes: Vec<PathBuf>,
    sort_descriptors: Vec<MetadataSortDescriptor>,
    state: State,
    results: Vec<MetadataItem>,
    queued: Vec<MetadataNotification>,
    stream: Option<WatchStream>,
    dirty: bool,
    updates_disabled: bool,
}

impl Default for MetadataQuery {
    fn default() -> Self {
        MetadataQuery::new()
    }
}

impl MetadataQuery {
    pub fn new() -> MetadataQuery {
        MetadataQuery {
            predicate: None,
            search_scopes: Vec::new(),
            sort_descriptors: Vec::new(),
            state: State::Stopped,
            results: Vec::new(),
            queued: Vec::new(),
            stream: None,
            dirty: false,
            updates_disabled: false,
        }
    }

    /// `start()`: false when there is no scope or the query is already running.
    pub fn start_query(&mut self) -> bool {
        if self.state != State::Stopped || self.search_scopes.is_empty() {
            return false;
        }
        self.queued = vec![MetadataNotification::DidStartGathering];
        self.results = self.gather();
        self.queued.push(MetadataNotification::GatheringProgress);
        self.queued.push(MetadataNotification::DidFinishGathering);
        self.state = State::Gathering;
        let mut stream = WatchStream::new(&self.search_scopes, UPDATE_LATENCY, WatchStreamFlags::NONE);
        self.stream = if stream.start() { Some(stream) } else { None };
        self.dirty = false;
        true
    }

    /// `stop()`: keeps the results, delivers nothing more.
    pub fn stop_query(&mut self) {
        self.stream = None;
        self.queued.clear();
        self.state = State::Stopped;
    }

    pub fn is_started(&self) -> bool {
        self.state != State::Stopped
    }

    pub fn is_gathering(&self) -> bool {
        self.state == State::Gathering
    }

    pub fn is_stopped(&self) -> bool {
        self.state == State::Stopped
    }

    /// `disableUpdates()`.
    pub fn disable_updates(&mut self) {
        self.updates_disabled = true;
    }

    /// `enableUpdates()`.
    pub fn enable_updates(&mut self) {
        self.updates_disabled = false;
    }

    /// `resultCount`.
    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    /// `result(at:)`.
    pub fn result(&self, index: usize) -> Option<&MetadataItem> {
        self.results.get(index)
    }

    /// `results`, ordered by the sort descriptors and then by path.
    pub fn results(&self) -> &[MetadataItem] {
        &self.results
    }

    /// `sortDescriptors`: the keys that order `results`. Empty means path order.
    pub fn sort_descriptors(&self) -> &[MetadataSortDescriptor] {
        &self.sort_descriptors
    }

    /// Sets `sortDescriptors` and re-sorts the current results at once; later rescans keep the order.
    pub fn set_sort_descriptors(&mut self, descriptors: Vec<MetadataSortDescriptor>) {
        self.sort_descriptors = descriptors;
        sort_items(&self.sort_descriptors, &mut self.results);
    }

    /// The descriptor for `App::wake_fd` (None without live updates).
    pub fn fd(&self) -> Option<i32> {
        self.stream.as_ref().and_then(|s| s.fd())
    }

    /// Time until a maturing batch of events is due.
    pub fn next_deadline(&self, now: Instant) -> Option<Duration> {
        self.stream.as_ref().and_then(|s| s.next_deadline(now))
    }

    /// Delivers the notifications that are due. Call from the owner's tick.
    pub fn poll(&mut self, now: Instant) -> Vec<MetadataNotification> {
        let mut out = std::mem::take(&mut self.queued);
        if out.contains(&MetadataNotification::DidFinishGathering) && self.state == State::Gathering {
            self.state = State::Started;
        }
        if let Some(stream) = &mut self.stream {
            if !stream.poll(now).is_empty() {
                self.dirty = true;
            }
        }
        if self.dirty && !self.updates_disabled && self.state == State::Started {
            self.dirty = false;
            let fresh = self.gather();
            if let Some(update) = diff(&self.results, &fresh) {
                out.push(update);
            }
            self.results = fresh;
        }
        out
    }

    /// Every item below the scopes that matches the predicate, ordered by the sort descriptors and then by path.
    fn gather(&self) -> Vec<MetadataItem> {
        let mut found = Vec::new();
        for scope in &self.search_scopes {
            let mut queue = VecDeque::from([scope.clone()]);
            while let Some(dir) = queue.pop_front() {
                let Ok(rd) = fs::read_dir(&dir) else { continue };
                for entry in rd.flatten() {
                    let path = entry.path();
                    let Some(item) = MetadataItem::read(&path) else { continue };
                    if item.value(MetadataAttribute::IsDirectory) == Some(&MetadataValue::Bool(true)) {
                        queue.push_back(path.clone());
                    }
                    if self.predicate.as_ref().map_or(true, |p| p.matches(&item)) {
                        found.push(item);
                    }
                }
            }
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        found.dedup_by(|a, b| a.path == b.path);
        sort_items(&self.sort_descriptors, &mut found);
        found
    }
}

fn diff(old: &[MetadataItem], new: &[MetadataItem]) -> Option<MetadataNotification> {
    let before: BTreeMap<&Path, &MetadataItem> = old.iter().map(|i| (i.path.as_path(), i)).collect();
    let after: BTreeMap<&Path, &MetadataItem> = new.iter().map(|i| (i.path.as_path(), i)).collect();
    let added: Vec<PathBuf> = after.keys().filter(|p| !before.contains_key(*p)).map(|p| p.to_path_buf()).collect();
    let removed: Vec<PathBuf> = before.keys().filter(|p| !after.contains_key(*p)).map(|p| p.to_path_buf()).collect();
    let changed: Vec<PathBuf> =
        after.iter().filter(|(p, item)| before.get(*p).map_or(false, |old| old != *item)).map(|(p, _)| p.to_path_buf()).collect();
    if added.is_empty() && removed.is_empty() && changed.is_empty() {
        None
    } else {
        Some(MetadataNotification::DidUpdate { added, changed, removed })
    }
}
