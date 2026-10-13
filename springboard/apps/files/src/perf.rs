//! Large-folder performance: worker-side ordering, the visible row window, a bounded label cache keyed by
//! path and mtime, and incremental folder updates from the SDK `WatchStream`.
use crate::*;
use bifrost_app::fswatch::{EventJournal, Replay, WatchEvent, WatchEventFlags, WatchStream, WatchStreamFlags};
use bifrost_app::kit::Image;
use std::{cell::RefCell, collections::HashMap, ops::Range, rc::Rc};

/// Labels kept between paints. A window shows a few hundred at most; this keeps scrolling back free.
pub(crate) const LABEL_CACHE_CAP: usize = 1024;
/// More events than this in one batch is cheaper to answer with a fresh listing.
const EVENT_RESCAN: usize = 4000;
/// Rows drawn beyond each edge so a scroll step never shows a gap.
const OVERSCAN_ROWS: usize = 2;

/// A least-recently-used map with a fixed capacity.
pub(crate) struct Lru<K, V> {
    cap: usize,
    tick: u64,
    map: HashMap<K, (V, u64)>,
}

impl<K: std::hash::Hash + Eq + Clone, V: Clone> Lru<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Self { cap: cap.max(1), tick: 0, map: HashMap::new() }
    }
    /// The number of entries; the tests check the bound.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
    pub(crate) fn get_or_insert(&mut self, key: K, make: impl FnOnce() -> V) -> V {
        self.tick += 1;
        if let Some((v, used)) = self.map.get_mut(&key) {
            *used = self.tick;
            return v.clone();
        }
        if self.map.len() >= self.cap {
            // evict the oldest eighth in one pass so the scan is amortised
            let mut ages: Vec<u64> = self.map.values().map(|(_, t)| *t).collect();
            let cut = (self.cap / 8).max(1).min(ages.len()) - 1;
            let limit = *ages.select_nth_unstable(cut).1;
            self.map.retain(|_, (_, t)| *t > limit);
        }
        let v = make();
        self.map.insert(key, (v.clone(), self.tick));
        v
    }
}

/// A rendered name: path, mtime (a rename or rewrite changes it), colour, scale and width in px.
pub(crate) type LabelKey = (PathBuf, i64, u32, u32, u32);
pub(crate) type Labels = RefCell<Lru<LabelKey, Rc<Image>>>;

pub(crate) fn new_labels() -> Labels {
    RefCell::new(Lru::new(LABEL_CACHE_CAP))
}

/// The rows (or grid lines) of a view that can touch the viewport: `top` is the first row's y at scroll 0.
pub(crate) fn row_span(scroll: f32, top: f32, row_h: f32, view_top: f32, view_bottom: f32, lines: usize) -> Range<usize> {
    let first = ((scroll + view_top - top) / row_h).floor().max(0.) as usize;
    let last = ((scroll + view_bottom - top) / row_h).ceil().max(0.) as usize;
    let start = first.saturating_sub(OVERSCAN_ROWS).min(lines);
    start..(last + OVERSCAN_ROWS).min(lines).max(start)
}

/// Folders first, then the chosen order (what `sort_rows` does, for a list that was just read).
pub(crate) fn order(list: &mut [Entry], sort: Sort) {
    fs::sort(list, sort);
    list.sort_by_key(|e| !e.is_dir);
}

/// What the listing worker does so the UI thread only swaps the result in: Recently Deleted shows original
/// names, and every list but Recents (newest first already) is ordered.
pub(crate) fn settle(rows: &mut Vec<Entry>, sort: Sort, in_bin: bool, keep_order: bool) {
    if in_bin {
        rows.retain(|e| e.name != ORIGINS);
        for e in rows.iter_mut() {
            if let Some((_, name)) = trash::parse(&e.name) {
                e.name = name.to_string();
            }
        }
    }
    if !keep_order {
        order(rows, sort);
    }
}

fn entry_at(path: &Path) -> Option<Entry> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    if name == trash::DIR || name == springboard::files::tags::SIDECAR {
        return None;
    }
    let md = std::fs::metadata(path).or_else(|_| std::fs::symlink_metadata(path)).ok()?;
    let is_dir = md.is_dir();
    Some(Entry {
        kind: fs::kind_of(&name, is_dir),
        name,
        path: path.to_path_buf(),
        is_dir,
        size: if is_dir { 0 } else { md.len() },
        mtime: md.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64),
        items: None,
    })
}

/// Applies a batch of events for the direct children of `dir` to `list`. `None` when a fresh listing is needed.
pub(crate) fn apply_events(list: &mut Vec<Entry>, dir: &Path, events: &[WatchEvent]) -> Option<bool> {
    if events.len() > EVENT_RESCAN {
        return None;
    }
    let mut changed = false;
    let mut at: HashMap<PathBuf, usize> = HashMap::new();
    for ev in events {
        if ev.path == dir {
            if ev.flags.contains(WatchEventFlags::MUST_SCAN_SUB_DIRS) || ev.flags.contains(WatchEventFlags::REMOVED) {
                return None;
            }
            continue;
        }
        if ev.path.parent() != Some(dir) {
            continue;
        }
        if at.len() != list.len() {
            at = list.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
        }
        match (entry_at(&ev.path), at.get(&ev.path).copied()) {
            (Some(new), Some(i)) => {
                if list[i].size != new.size || list[i].mtime != new.mtime || list[i].is_dir != new.is_dir {
                    let keep = list[i].items;
                    list[i] = Entry { items: if new.is_dir { keep } else { None }, ..new };
                    changed = true;
                }
            }
            (Some(new), None) => {
                at.insert(new.path.clone(), list.len());
                list.push(new);
                changed = true;
            }
            (None, Some(i)) => {
                list.swap_remove(i);
                at.clear();
                changed = true;
            }
            (None, None) => {}
        }
    }
    Some(changed)
}

/// Watch events kept for catching up: a busy folder's churn while its listing loads.
pub(crate) const JOURNAL_CAP: usize = 256;

/// The events a folder still owes from a journal replay, oldest first. `None` when the journal dropped events the
/// folder had not seen: the folder then lists again rather than guess.
pub(crate) fn unseen(replay: Replay) -> Option<Vec<WatchEvent>> {
    replay.complete.then(|| replay.events.into_iter().map(|(_, e)| e).collect())
}

impl Files {
    /// Writes a batch of watch events into the journal.
    pub(crate) fn record_events(&mut self, events: Vec<WatchEvent>) {
        for e in events {
            self.journal.record(e);
        }
    }
    /// Applies the journal events the open folder has not applied yet. While the folder is loading (or a recents or
    /// trash view is open) nothing is applied and the cursor stays, so the events wait for the listing.
    pub(crate) fn catch_up(&mut self) -> bool {
        if self.journal.latest_id() == self.journal_seen || self.loading || self.in_recents() || self.in_bin() {
            return false;
        }
        let replay = self.journal.replay_since(self.journal_seen);
        self.journal_seen = self.journal.latest_id();
        match unseen(replay) {
            Some(events) => self.fold_events(events),
            None => {
                let dir = self.path.clone();
                self.navigate(dir, false);
                true
            }
        }
    }
}

impl Files {
    /// Points the stream at the data folder (Show in Files) and, for a real folder, the open one.
    pub(crate) fn rewatch(&mut self) {
        let Some(data) = self.data_dir.clone() else { return };
        let mut roots = vec![data];
        if !self.in_recents() && !self.in_bin() && self.path.is_dir() {
            roots.push(self.path.clone());
        }
        // only the open folder: a recursive stream would walk `/` or the home tree on the UI thread
        let flags = WatchStreamFlags::FILE_EVENTS.union(WatchStreamFlags::SHALLOW);
        let mut w = WatchStream::new(&roots, Duration::from_millis(120), flags);
        self.watch = if w.start() { Some(w) } else { None };
    }
    /// Folds a batch of events into the open folder without listing it again.
    pub(crate) fn fold_events(&mut self, events: Vec<WatchEvent>) -> bool {
        let data = self.data_dir.clone();
        let events: Vec<WatchEvent> = events.into_iter().filter(|e| data.as_ref().is_none_or(|d| !e.path.starts_with(d))).collect();
        if events.is_empty() || self.loading || self.in_recents() || self.in_bin() {
            return false;
        }
        let selected: Vec<PathBuf> = self.selection.iter().filter_map(|&i| self.rows.get(i).map(|e| e.path.clone())).collect();
        let dir = self.path.clone();
        match apply_events(&mut self.listing, &dir, &events) {
            None => {
                self.navigate(dir, false);
                true
            }
            Some(false) => false,
            Some(true) => {
                order(&mut self.listing, self.sort);
                self.refresh_rows();
                self.selection = selected.iter().filter_map(|p| self.rows.iter().position(|e| &e.path == p)).collect();
                true
            }
        }
    }
}

impl Files {
    /// Adds a batch of a loading folder's rows. While no folder is expanded and no search is showing, the rows are the
    /// listing itself, so the batch goes onto both without rebuilding either.
    pub(crate) fn append_batch(&mut self, batch: Vec<Entry>) {
        if self.hits.is_none() && self.vo.expanded.is_empty() {
            self.listing.extend(batch.iter().cloned());
            self.rows.extend(batch);
            self.vo.depth.resize(self.rows.len(), 0);
        } else {
            self.listing.extend(batch);
            self.refresh_rows();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn entry(i: usize) -> Entry {
        let name = format!("file {i:05}.txt");
        Entry { kind: Kind::Text, path: PathBuf::from("/d").join(&name), name, is_dir: i % 20 == 0, size: i as u64, mtime: i as i64, items: None }
    }

    #[test]
    fn lru_stays_bounded_and_keeps_recent() {
        let mut c: Lru<usize, usize> = Lru::new(16);
        for i in 0..100 {
            c.get_or_insert(i, || i);
            c.get_or_insert(0, || 0);
            assert!(c.len() <= 16);
        }
        let mut made = false;
        c.get_or_insert(0, || {
            made = true;
            0
        });
        assert!(!made, "the hot key survived eviction");
    }

    #[test]
    fn span_covers_the_viewport_only() {
        let r = row_span(3200., 94., 32., 94., 600., 50_000);
        assert!(r.start <= 100 && r.end >= 115 && r.len() < 30, "{r:?}");
        assert_eq!(row_span(0., 94., 32., 94., 600., 5), 0..5);
        assert!(row_span(1e9, 94., 32., 94., 600., 10).is_empty());
    }

    #[test]
    fn unseen_events_replay_in_order_or_force_a_listing() {
        let mut j = EventJournal::new(2);
        let ev = |n: &str| WatchEvent { path: PathBuf::from("/d").join(n), flags: WatchEventFlags::CREATED };
        j.record(ev("a"));
        j.record(ev("b"));
        assert_eq!(unseen(j.replay_since(0)).unwrap().len(), 2);
        j.record(ev("c"));
        // id 1 was dropped before this folder saw it: a fresh listing is needed
        assert!(unseen(j.replay_since(0)).is_none());
        let got: Vec<PathBuf> = unseen(j.replay_since(1)).unwrap().into_iter().map(|e| e.path).collect();
        assert_eq!(got, vec![PathBuf::from("/d/b"), PathBuf::from("/d/c")]);
    }

    #[test]
    fn events_update_the_listing_in_place() {
        let dir = std::env::temp_dir().join(format!("files-perf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut list = fs::list(&dir).unwrap();
        std::fs::write(dir.join("b.txt"), "yy").unwrap();
        std::fs::remove_file(dir.join("a.txt")).unwrap();
        let ev = |n: &str, f| WatchEvent { path: dir.join(n), flags: f };
        let got = apply_events(&mut list, &dir, &[ev("b.txt", WatchEventFlags::CREATED), ev("a.txt", WatchEventFlags::REMOVED)]);
        assert_eq!(got, Some(true));
        assert_eq!(list.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["b.txt"]);
        let rescan = WatchEvent { path: dir.clone(), flags: WatchEventFlags::MUST_SCAN_SUB_DIRS };
        assert_eq!(apply_events(&mut list, &dir, &[rescan]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn bench(name: &str, runs: usize, mut f: impl FnMut()) {
        let mut t: Vec<f64> = (0..runs)
            .map(|_| {
                let s = Instant::now();
                f();
                s.elapsed().as_secs_f64() * 1e3
            })
            .collect();
        t.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!("BENCH {name} {:.4} ms", t[t.len() / 2]);
    }

    #[test]
    #[ignore]
    fn bench_files_sort_50k() {
        let base: Vec<Entry> = (0..50_000).rev().map(entry).collect();
        bench("files_sort_50k", 5, || {
            let mut v = base.clone();
            order(&mut v, Sort::Name);
            std::hint::black_box(&v);
        });
    }

    #[test]
    #[ignore]
    fn bench_files_row_span_50k() {
        bench("files_row_span_50k", 51, || {
            for s in 0..1000 {
                std::hint::black_box(row_span(s as f32 * 37., 94., 32., 94., 700., 50_000));
            }
        });
    }

    #[test]
    #[ignore]
    fn bench_files_label_cache_hit() {
        let mut c: Lru<usize, Rc<usize>> = Lru::new(LABEL_CACHE_CAP);
        for i in 0..LABEL_CACHE_CAP {
            c.get_or_insert(i, || Rc::new(i));
        }
        bench("files_label_cache_hit", 51, || {
            for i in 0..LABEL_CACHE_CAP {
                std::hint::black_box(c.get_or_insert(i, || Rc::new(i)));
            }
        });
    }

    #[test]
    #[ignore]
    fn bench_files_filter_50k() {
        let rows: Vec<Entry> = (0..50_000).map(entry).collect();
        bench("files_filter_50k", 11, || {
            let n = rows.iter().filter(|e| e.name.contains("123")).count();
            std::hint::black_box(n);
        });
    }
}
