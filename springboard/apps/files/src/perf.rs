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
    let is_dir = md.is_dir() && !fs::is_package_name(&name);
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
    #[cfg(test)]
    pub(crate) fn record_events(&mut self, events: Vec<WatchEvent>) {
        for e in events {
            self.journal.record(e);
        }
    }
    /// Writes the events of a shared batch that are about this window's folder into its journal. The one stream
    /// watches every window's folder, and another window's busy folder must not push this one's events out.
    pub(crate) fn record_own(&mut self, events: &[WatchEvent]) {
        for e in events.iter().filter(|e| concerns(&self.path, &e.path)) {
            self.journal.record(e.clone());
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
                self.relist();
                true
            }
        }
    }
}

/// How long a batch of folder events matures before it is handed over.
const WATCH_LATENCY: Duration = Duration::from_millis(120);
/// The mount table is read at most this often on the loop's turns, and this often for `MOUNT_SETTLE` after a disk or
/// a media folder changed (the mount itself follows the folder or the disk by a moment).
pub(crate) const MOUNT_CHECK_EVERY: Duration = Duration::from_secs(1);
pub(crate) const MOUNT_SETTLE: Duration = Duration::from_secs(4);

/// Whether an event at `event` is about the folder `dir`: the folder itself or an item directly in it.
pub(crate) fn concerns(dir: &Path, event: &Path) -> bool {
    event == dir || event.parent() == Some(dir)
}

/// The folders volumes are mounted in for the account whose home is `home`, with the folders above them (the
/// account's media folder is made when its first volume is mounted).
pub(crate) fn media_folders(home: &Path) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/run/media"), PathBuf::from("/media")];
    if let Some(user) = home.file_name() {
        out.insert(0, Path::new("/run/media").join(user));
    }
    out
}

/// The folders one stream watches: the data folder, each window's open folder once, and the media folders that are
/// there. The order is fixed, so the same windows give the same list.
pub(crate) fn watch_roots(data: PathBuf, open: impl Iterator<Item = PathBuf>, media: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut roots = vec![data];
    for dir in open.chain(media.into_iter().filter(|m| m.is_dir())) {
        if !roots.contains(&dir) {
            roots.push(dir);
        }
    }
    roots
}

impl Files {
    /// The folder this window wants watched: the open one when it is a real folder. None for a Get Info window and
    /// for the stand-in listings.
    pub(crate) fn watch_root(&self) -> Option<PathBuf> {
        (!self.info_window && !self.in_recents() && !self.in_bin() && self.path.is_dir()).then(|| self.path.clone())
    }
    /// Builds the one watch stream again when the folders to watch changed: the data folder (Show in Files), the open
    /// folder of this window and of every New Window window, and the media folders. One stream means one descriptor
    /// for the loop to wait on (`App::wake_fd`) and no thread. A stream that is replaced while a batch of its events
    /// is still maturing is kept until that batch is handed over, so no window loses a change to another's navigation.
    pub(crate) fn sync_watch(&mut self) {
        let mut stale = std::mem::take(&mut self.fx.watch_stale);
        for e in &mut self.windows {
            stale |= std::mem::take(&mut e.files.fx.watch_stale);
        }
        let Some(data) = self.data_dir.clone() else { return };
        if !stale {
            return;
        }
        let open = self.watch_root().into_iter().chain(self.windows.iter().filter_map(|e| e.files.watch_root()));
        let roots = watch_roots(data, open, media_folders(&self.home));
        if roots == self.fx.watched && self.watch.is_some() {
            return;
        }
        let now = Instant::now();
        if let Some(mut old) = self.watch.take() {
            let events = old.poll(now);
            self.spread_events(events);
            if old.next_deadline(now).is_some() {
                self.fx.retiring.push(old);
            }
        }
        // only the folders themselves: a recursive stream would walk `/` or the home tree on the UI thread
        let flags = WatchStreamFlags::FILE_EVENTS.union(WatchStreamFlags::SHALLOW);
        let mut w = WatchStream::new(&roots, WATCH_LATENCY, flags);
        self.watch = if w.start() { Some(w) } else { None };
        self.fx.watched = roots;
    }
    /// The events the streams have ready, for `tick`: the live one's and those of the streams being retired.
    pub(crate) fn poll_watch(&mut self, now: Instant) -> Vec<WatchEvent> {
        let mut events = self.watch.as_mut().map(|w| w.poll(now)).unwrap_or_default();
        for old in &mut self.fx.retiring {
            events.extend(old.poll(now));
        }
        self.fx.retiring.retain(|w| w.next_deadline(now).is_some());
        events
    }
    /// When the next batch of folder events matures, from `now`.
    pub(crate) fn watch_deadline(&self, now: Instant) -> Option<Duration> {
        self.watch.iter().chain(self.fx.retiring.iter()).filter_map(|w| w.next_deadline(now)).min()
    }
    /// Hands a batch of events to the windows: each journal takes the events about its own folder, and a change in a
    /// media folder starts the looks at the mount table.
    pub(crate) fn spread_events(&mut self, events: Vec<WatchEvent>) {
        if events.is_empty() {
            return;
        }
        let media = media_folders(&self.home);
        if events.iter().any(|e| e.path.parent().is_some_and(|p| media.iter().any(|m| m == p))) {
            self.expect_mounts(Instant::now());
        }
        for e in &mut self.windows {
            e.files.record_own(&events);
        }
        self.record_own(&events);
    }
    /// A disk or a media folder changed: the mount table is looked at now and every second for a while.
    pub(crate) fn expect_mounts(&mut self, now: Instant) {
        self.fx.mounts_until = Some(now + MOUNT_SETTLE);
        self.fx.mounts_checked = None;
    }
    /// Looks at the mount table, at most once a second unless `force`: when the mounted volumes changed, every
    /// window's sidebar is drawn again with the volumes there are now, and the windows leave the folders of a volume
    /// that is gone. True when they changed. Under the test pin the sidebar lists no volumes, and nothing is read.
    pub(crate) fn follow_mounts(&mut self, now: Instant, force: bool) -> bool {
        if bifrost_app::testing::pinned() {
            return false;
        }
        if self.fx.mounts_until.is_some_and(|until| now >= until) {
            self.fx.mounts_until = None;
        }
        if !force && self.fx.mounts_checked.is_some_and(|at| now.saturating_duration_since(at) < MOUNT_CHECK_EVERY) {
            return false;
        }
        self.fx.mounts_checked = Some(now);
        let mounts = springboard::files::volumes::mounted().into_iter().map(|v| v.mount).collect();
        self.mounts_are(mounts)
    }
    /// The mount points read from the mount table. True when they differ from the last ones (not for the first look).
    pub(crate) fn mounts_are(&mut self, mounts: Vec<PathBuf>) -> bool {
        match self.fx.mounts.replace(mounts) {
            None => false,
            Some(before) if Some(&before) == self.fx.mounts.as_ref() => false,
            Some(_) => {
                self.leave_lost_volumes();
                self.revision += 1;
                for e in &mut self.windows {
                    e.files.revision += 1;
                }
                // a media folder that was not there before can be watched now
                self.fx.watch_stale = true;
                true
            }
        }
    }
}

impl Files {
    /// The open folder changed: the stream is pointed at it. The main window builds the stream at once, before the
    /// folder's listing is read; a New Window window only says so, and the main window's next `tick` builds it
    /// (`sync_watch`), because the one stream is the main window's.
    pub(crate) fn rewatch(&mut self) {
        self.fx.watch_stale = true;
        if !self.fx.extra_window {
            self.sync_watch();
        }
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
                self.relist();
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

    #[test]
    fn a_file_removed_from_the_open_folder_leaves_the_listing_and_the_background_window_follows() {
        let dir = std::env::temp_dir().join(format!("files-removed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("keep.txt"), "x").unwrap();
        std::fs::write(dir.join("gone.txt"), "y").unwrap();
        let mut files = Files::new();
        files.path = dir.clone();
        files.loading = false;
        files.listing = fs::list(&dir).unwrap();
        files.refresh_rows();
        let names = |f: &Files| f.rows.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert!(names(&files).contains(&"gone.txt".to_string()));
        // rm from a shell while the window is open: the watch reports the removal and the row goes
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        files.record_events(vec![WatchEvent { path: dir.join("gone.txt"), flags: WatchEventFlags::REMOVED }]);
        assert!(files.catch_up(), "the removal changes what the window shows");
        assert_eq!(names(&files), ["keep.txt"]);
        // moved out of the folder (a rename event whose path no longer exists) leaves too
        std::fs::rename(dir.join("keep.txt"), std::env::temp_dir().join(format!("files-moved-{}", std::process::id()))).unwrap();
        files.record_events(vec![WatchEvent { path: dir.join("keep.txt"), flags: WatchEventFlags::RENAMED }]);
        assert!(files.catch_up());
        assert!(files.rows.is_empty());
        // and it keeps doing so while another app is in front
        assert!(App::ticks_in_background(&files));
        let _ = std::fs::remove_file(std::env::temp_dir().join(format!("files-moved-{}", std::process::id())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_stream_watches_every_windows_folder_and_each_journal_takes_its_own_events() {
        let root = std::env::temp_dir().join(format!("files-shared-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["data", "a", "b"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let window = |dir: &str| {
            let mut f = crate::tests::fixture(vec![]);
            f.home = root.clone();
            f.path = root.join(dir);
            f.fx.extra_window = true;
            f
        };
        let mut main = crate::tests::fixture(vec![]);
        main.home = root.clone();
        main.path = root.join("a");
        main.data_dir = Some(root.join("data"));
        main.windows.push(crate::windows::ExtraWindow::new(WindowId(1), Box::new(window("b"))));
        main.windows.push(crate::windows::ExtraWindow::new(WindowId(2), Box::new(window("a"))));
        main.rewatch();
        assert!(main.watch.is_some(), "the main window owns the stream");
        assert_eq!(main.fx.watched[..3], [root.join("data"), root.join("a"), root.join("b")], "each folder once");
        assert!(main.windows.iter().all(|e| e.files.watch.is_none()), "no stream, and no descriptor, per window");
        // a window that opens another folder only says so; the main window's next look builds the stream again
        main.windows[0].files.path = root.join("data");
        main.windows[0].files.rewatch();
        assert!(main.windows[0].files.watch.is_none() && main.windows[0].files.fx.watch_stale);
        main.sync_watch();
        assert_eq!(main.fx.watched[..2], [root.join("data"), root.join("a")]);
        assert!(!main.windows[0].files.fx.watch_stale);
        // a batch goes to the windows it is about, and to no other journal
        let ev = |p: PathBuf| WatchEvent { path: p, flags: WatchEventFlags::CREATED };
        main.spread_events(vec![ev(root.join("a/new.txt")), ev(root.join("data/x")), ev(root.join("b/other"))]);
        assert_eq!(main.journal.latest_id(), 1);
        assert_eq!(main.windows[0].files.journal.latest_id(), 1, "the window showing the data folder");
        assert_eq!(main.windows[1].files.journal.latest_id(), 1, "the second window on the same folder as the main one");
        assert!(concerns(&root.join("a"), &root.join("a")) && !concerns(&root.join("a"), &root.join("a/sub/deep")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_change_in_a_new_window_folder_reaches_that_window() {
        let root = std::env::temp_dir().join(format!("files-window-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("data")).unwrap();
        std::fs::create_dir_all(root.join("shown")).unwrap();
        let mut second = crate::tests::fixture(vec![]);
        second.home = root.clone();
        second.path = root.join("shown");
        second.fx.extra_window = true;
        let mut main = crate::tests::fixture(vec![]);
        main.home = root.clone();
        main.data_dir = Some(root.join("data"));
        main.windows.push(crate::windows::ExtraWindow::new(WindowId(1), Box::new(second)));
        main.rewatch();
        std::fs::write(root.join("shown/new.txt"), "x").unwrap();
        // the kernel's event matures over the stream's latency
        let mut seen = false;
        for _ in 0..200 {
            let events = main.poll_watch(Instant::now());
            main.spread_events(events);
            if main.windows[0].files.catch_up() {
                seen = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(seen, "the second window heard of the new file");
        assert_eq!(main.windows[0].files.rows.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["new.txt"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_relist_keeps_the_scroll_and_the_selection() {
        let dir = std::env::temp_dir().join(format!("files-relist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = crate::tests::fixture((0..200).map(|i| Entry { path: dir.join(format!("f{i:03}")), ..entry(i) }).collect());
        files.home = dir.clone();
        files.path = dir.clone();
        files.grid = false;
        files.select_only(150);
        files.reveal_comfortably(150);
        let place = files.scroll;
        assert!(place > 0., "the revealed row is far down the list");
        files.relist();
        assert_eq!(files.scroll, 0., "the folder is being read again");
        assert_eq!(files.fx.relist, Some((place, vec![dir.join("f150")])));
        // the listing lands with the same rows
        files.listing = (0..200).map(|i| Entry { path: dir.join(format!("f{i:03}")), ..entry(i) }).collect();
        files.loading = false;
        files.refresh_rows();
        files.restore_place();
        assert_eq!(files.scroll, place, "the view is where it was");
        assert_eq!(files.selected_paths(), vec![dir.join("f150")]);
        // going somewhere else forgets the place
        files.relist();
        files.navigate(dir.join("gone"), true);
        assert!(files.fx.relist.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_changed_mount_table_redraws_every_window_and_the_first_look_does_not() {
        let mut files = crate::tests::fixture(vec![]);
        files.windows.push(crate::windows::ExtraWindow::new(WindowId(1), Box::new(crate::tests::fixture(vec![]))));
        let (rev, win_rev) = (files.revision, files.windows[0].files.revision);
        assert!(!files.mounts_are(vec![]), "the first look only remembers");
        assert!(!files.mounts_are(vec![]), "nothing changed");
        assert_eq!(files.revision, rev);
        assert!(files.mounts_are(vec![PathBuf::from("/run/media/me/STICK")]), "a volume was mounted");
        assert!(files.revision > rev && files.windows[0].files.revision > win_rev, "every sidebar is drawn again");
        assert!(files.fx.watch_stale, "its media folder is watched from now on");
        assert!(files.mounts_are(vec![]), "and it went away");
        // a disk event starts a few looks a second apart
        let now = Instant::now();
        files.expect_mounts(now);
        assert_eq!(files.fx.mounts_until, Some(now + MOUNT_SETTLE));
        assert_eq!(media_folders(Path::new("/home/me"))[0], PathBuf::from("/run/media/me"));
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
