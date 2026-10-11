//! Storage hygiene: what the data partition holds, by category (computed on
//! a worker thread, never on the UI thread), the caches that can be cleared
//! at any time, and the guard that clears them by itself when `/data` runs
//! low. The accounting and the choice of what to delete are pure functions
//! over plain data; only `dir_size`, `clear` and the workers touch the disk.

use super::notify::Notification;
use super::{AppId, Shell};
use crate::bare::sys;
use std::path::{Path, PathBuf};
use crate::shell::clock::tz::unix_now;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// The partition the shell accounts for.
pub const PARTITION: &str = "/data";
/// Below this much free space the shell trims the disposable caches itself.
pub const LOW_BYTES: u64 = 500 * 1_000_000;
/// Once a notice was posted, the next one waits this long (seconds).
pub const NOTICE_GAP_S: i64 = 6 * 3600;
/// Old binaries kept by every trim: the newest two.
pub const KEEP_BINARIES: usize = 2;
const BINARY_PREFIX: &str = "springboard.before-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Photos,
    Music,
    Files,
    Deildu,
    Terminal,
    /// Notes, Contacts, settings, wallpapers and anything else of the shell's
    Other,
    Logs,
}

/// Bar and list order.
pub const CATEGORIES: [Category; 7] = [Category::Photos, Category::Music, Category::Files, Category::Deildu, Category::Terminal, Category::Other, Category::Logs];

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::Photos => "Photos",
            Category::Music => "Music",
            Category::Files => "Files",
            Category::Deildu => "Deildu",
            Category::Terminal => "Terminal",
            Category::Other => "Notes, Contacts & Other",
            Category::Logs => "Logs",
        }
    }
}

/// Which category a top-level entry of the data directory belongs to.
/// Thumbnails and artwork (`cache/`) count with the Photos they belong to.
pub fn classify(name: &str) -> Category {
    match name {
        "Pictures" | "cache" => Category::Photos,
        "Music" => Category::Music,
        "Files" => Category::Files,
        "deildu" => Category::Deildu,
        "Terminal" => Category::Terminal,
        "log" | "log.old" => Category::Logs,
        n if n.ends_with(".log") => Category::Logs,
        _ => Category::Other,
    }
}

/// Used and free space on the partition plus the bytes of each category.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Usage {
    pub total: u64,
    pub free: u64,
    pub cats: Vec<(Category, u64)>,
    /// what each Clear action would free right now
    pub reclaim: Vec<(Clear, u64)>,
}

impl Usage {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }

    /// Bytes of the categories we measured.
    pub fn counted(&self) -> u64 {
        self.cats.iter().map(|&(_, b)| b).sum()
    }

    /// Everything on the partition that is not ours (Android, apps, system).
    pub fn system(&self) -> u64 {
        self.used().saturating_sub(self.counted())
    }

    pub fn reclaimable(&self, c: Clear) -> u64 {
        self.reclaim.iter().find(|&&(k, _)| k == c).map_or(0, |&(_, b)| b)
    }

    pub fn bytes(&self, c: Category) -> u64 {
        self.cats.iter().find(|&&(k, _)| k == c).map_or(0, |&(_, b)| b)
    }
}

/// Folds measured `(top-level name, bytes)` entries into one total per category,
/// in `CATEGORIES` order (empty categories included).
pub fn account(entries: &[(String, u64)]) -> Vec<(Category, u64)> {
    CATEGORIES.iter().map(|&c| (c, entries.iter().filter(|(n, _)| classify(n) == c).map(|&(_, b)| b).sum())).collect()
}

/// Segment widths of a bar `width` px wide: one per `(bytes)`, proportional to
/// `total`, summing to at most `width`; a non-empty segment is at least 1 px
/// while there is room, so a small category stays visible.
pub fn bar_widths(bytes: &[u64], total: u64, width: u32) -> Vec<u32> {
    if total == 0 || width == 0 {
        return vec![0; bytes.len()];
    }
    let mut left = width;
    bytes
        .iter()
        .map(|&b| {
            let w = ((b as u128 * width as u128) / total as u128) as u32;
            let w = if b > 0 { w.max(1) } else { 0 }.min(left);
            left -= w;
            w
        })
        .collect()
}

/// "812 MB", "1.4 GB", "36 KB": decimal units, as the About page counts.
pub fn human(bytes: u64) -> String {
    bifrost_ui::format::byte_count(bytes)
}

/// Bytes under `path` (a file, or a directory walked without following
/// symbolic links); unreadable entries count as zero.
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    rd.flatten().map(|e| dir_size(&e.path())).sum()
}

/// Measures the partition and every top-level entry of `dir`.
pub fn scan(dir: &Path) -> Usage {
    let (total, free) = sys::disk_space(PARTITION).unwrap_or((0, 0));
    let entries: Vec<(String, u64)> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), dir_size(&e.path()))).collect())
        .unwrap_or_default();
    Usage { total, free, cats: account(&entries), reclaim: CLEARS.iter().map(|&c| (c, c.reclaimable(dir))).collect() }
}

/// A measurement running on a worker thread; the UI polls it each frame.
#[derive(Default)]
pub struct Scanner {
    rx: Option<Receiver<Usage>>,
    pub usage: Option<Usage>,
}

impl Scanner {
    /// Starts measuring `dir`; a run still going is superseded.
    pub fn start(&mut self, dir: PathBuf) {
        self.start_with(dir, None);
    }

    /// Clears `what` first (on the worker), then measures `dir`.
    pub fn start_with(&mut self, dir: PathBuf, what: Option<Clear>) {
        let (tx, rx) = channel();
        self.rx = Some(rx);
        crate::bare::sys::spawn_named("storage-scan", move || {
            crate::gfx::affinity::background_thread();
            if let Some(c) = what {
                clear(c, &dir);
            }
            let _ = tx.send(scan(&dir));
        });
    }

    pub fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Takes a finished result; true when `usage` changed.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.rx else { return false };
        match rx.try_recv() {
            Ok(u) => {
                self.usage = Some(u);
                self.rx = None;
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                false
            }
        }
    }
}

/// What the Clear Cache rows can empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clear {
    DeilduCache,
    /// thumbnails and artwork (`cache/`): rebuilt on demand
    Thumbnails,
    /// the rotated log (`log.old`)
    OldLog,
    /// `springboard.before-*` binaries beyond the newest two
    OldBinaries,
}

pub const CLEARS: [Clear; 4] = [Clear::DeilduCache, Clear::Thumbnails, Clear::OldLog, Clear::OldBinaries];

impl Clear {
    pub fn label(self) -> &'static str {
        match self {
            Clear::DeilduCache => "Clear Deildu Cache",
            Clear::Thumbnails => "Clear Thumbnails & Artwork",
            Clear::OldLog => "Clear Old Log",
            Clear::OldBinaries => "Delete Old Versions",
        }
    }

    pub fn footer(self) -> &'static str {
        match self {
            Clear::DeilduCache => "Downloaded artwork and torrent files; fetched again when needed. Saved torrents and the sign-in stay.",
            Clear::Thumbnails => "Photo thumbnails and Music artwork; rebuilt as you browse.",
            Clear::OldLog => "The previous session log. The current log stays.",
            Clear::OldBinaries => "Backups of earlier SpringBoard builds. The newest two are kept.",
        }
    }

    /// The files this action would delete under `dir`, with their sizes.
    pub fn targets(self, dir: &Path) -> Vec<PathBuf> {
        match self {
            Clear::DeilduCache => vec![dir.join("deildu/cache")],
            Clear::Thumbnails => vec![dir.join("cache")],
            Clear::OldLog => vec![dir.join("log.old")],
            Clear::OldBinaries => old_binaries(&binaries(dir), KEEP_BINARIES).into_iter().map(|n| dir.join(n)).collect(),
        }
    }

    /// Bytes this action would free right now.
    pub fn reclaimable(self, dir: &Path) -> u64 {
        self.targets(dir).iter().map(|p| dir_size(p)).sum()
    }
}

/// `springboard.before-*` files directly under `dir`, with their mtime (s).
fn binaries(dir: &Path) -> Vec<(String, u64)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let m = e.metadata().ok()?;
            let age = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            (name.starts_with(BINARY_PREFIX) && m.is_file()).then_some((name, age))
        })
        .collect()
}

/// Which old binaries to delete: every `springboard.before-*` entry but the
/// `keep` newest by mtime (ties broken by name, later names are newer, which
/// suits the timestamped names the deploy script writes).
pub fn old_binaries(entries: &[(String, u64)], keep: usize) -> Vec<String> {
    let mut mine: Vec<&(String, u64)> = entries.iter().filter(|(n, _)| n.starts_with(BINARY_PREFIX)).collect();
    mine.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    mine.into_iter().skip(keep).map(|(n, _)| n.clone()).collect()
}

fn remove(path: &Path) {
    let _ = match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    };
}

/// Carries out one clear; returns the bytes freed.
pub fn clear(what: Clear, dir: &Path) -> u64 {
    let targets = what.targets(dir);
    let before: u64 = targets.iter().map(|p| dir_size(p)).sum();
    for t in &targets {
        remove(t);
    }
    before
}

/// Every disposable cache at once (the low-space guard); bytes freed.
pub fn trim_disposable(dir: &Path) -> u64 {
    CLEARS.iter().map(|&c| clear(c, dir)).sum()
}

/// Whether `free` bytes is low enough to trim (0 means unknown).
pub fn is_low(free: u64) -> bool {
    free > 0 && free < LOW_BYTES
}

/// Whether a notice may be posted now, given when the last one was.
pub fn notice_due(last: Option<i64>, now: i64) -> bool {
    last.map_or(true, |t| now.saturating_sub(t) >= NOTICE_GAP_S || now < t)
}

/// The low-space guard's state: one trim worker at a time.
#[derive(Default)]
pub struct Guard {
    rx: Option<Receiver<u64>>,
    last_notice: Option<i64>,
}

impl Guard {
    /// Starts a trim of `dir` on a worker when `free` is low and none runs.
    pub fn check(&mut self, free: u64, dir: &Path) {
        if !is_low(free) || self.rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        self.rx = Some(rx);
        let dir = dir.to_path_buf();
        crate::bare::sys::spawn_named("storage-job", move || {
            crate::gfx::affinity::background_thread();
            let _ = tx.send(trim_disposable(&dir));
        });
    }

    /// Bytes freed by a finished trim, once.
    pub fn poll(&mut self) -> Option<u64> {
        let rx = self.rx.as_ref()?;
        match rx.try_recv() {
            Ok(n) => {
                self.rx = None;
                Some(n)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.rx = None;
                None
            }
        }
    }

    /// Records a posted notice at `now`, or says one is not due yet.
    pub fn note(&mut self, now: i64) -> bool {
        if !notice_due(self.last_notice, now) {
            return false;
        }
        self.last_notice = Some(now);
        true
    }
}

impl Shell {
    /// Called with every fresh `Info`: trims the caches while `/data` is low
    /// and posts "Storage Almost Full" once the trim has finished.
    pub(super) fn storage_check(&mut self, free: u64) {
        if let Some(freed) = self.storage_guard.poll() {
            if is_low(free) && self.storage_guard.note(unix_now()) {
                let body = format!("Only {} is left. SpringBoard cleared {} of caches; delete photos, music or downloads to free more.", human(free), human(freed));
                self.notify(Notification::new(AppId::Settings, "Storage Almost Full", body, unix_now()));
            }
        }
        let dir = self.data_dir.clone();
        self.storage_guard.check(free, &dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-storage")).join(std::process::id().to_string()).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn e(n: &str, b: u64) -> (String, u64) {
        (n.to_string(), b)
    }

    #[test]
    fn entries_are_classified_by_name() {
        assert_eq!(classify("Pictures"), Category::Photos);
        assert_eq!(classify("cache"), Category::Photos);
        assert_eq!(classify("deildu"), Category::Deildu);
        assert_eq!(classify("log.old"), Category::Logs);
        assert_eq!(classify("battery.log"), Category::Logs);
        assert_eq!(classify("Notes"), Category::Other);
        assert_eq!(classify("springboard.before-1"), Category::Other);
    }

    #[test]
    fn accounting_sums_per_category_in_order() {
        let a = account(&[e("Pictures", 10), e("cache", 5), e("Music", 7), e("Notes", 1), e("Contacts", 2), e("log", 3)]);
        assert_eq!(a.len(), CATEGORIES.len());
        assert_eq!(a[0], (Category::Photos, 15));
        assert_eq!(a[1], (Category::Music, 7));
        assert_eq!(a[2], (Category::Files, 0));
        assert_eq!(a[5], (Category::Other, 3));
        assert_eq!(a[6], (Category::Logs, 3));
        let u = Usage { total: 100, free: 40, cats: a, ..Usage::default() };
        assert_eq!((u.used(), u.counted(), u.system()), (60, 28, 32));
        assert_eq!(u.bytes(Category::Music), 7);
        let over = Usage { total: 10, free: 9, cats: vec![(Category::Files, 50)], ..Usage::default() };
        assert_eq!(over.system(), 0, "never underflows");
    }

    #[test]
    fn bar_segments_are_proportional_and_fit() {
        assert_eq!(bar_widths(&[50, 25, 25], 100, 200), vec![100, 50, 50]);
        let w = bar_widths(&[1, 0, 99], 100, 50);
        assert_eq!(w, vec![1, 0, 49]);
        assert!(bar_widths(&[60, 60], 100, 100).iter().sum::<u32>() <= 100, "overfull input is clipped");
        assert_eq!(bar_widths(&[5], 0, 100), vec![0]);
        assert_eq!(bar_widths(&[5], 10, 0), vec![0]);
    }

    #[test]
    fn sizes_read_like_the_about_page() {
        assert_eq!(human(0), "0 bytes");
        assert_eq!(human(36_000), "36 KB");
        assert_eq!(human(1_500_000), "1.5 MB");
        assert_eq!(human(812_000_000), "812 MB");
        assert_eq!(human(1_400_000_000), "1.4 GB");
        assert_eq!(human(117_000_000_000), "117 GB");
    }

    #[test]
    fn old_binaries_keep_the_newest_two() {
        let list = [e("springboard.before-a", 10), e("springboard.before-b", 30), e("springboard.before-c", 20), e("springboard.before-d", 5), e("springboard", 99), e("log", 1)];
        assert_eq!(old_binaries(&list, 2), vec!["springboard.before-a".to_string(), "springboard.before-d".to_string()]);
        assert!(old_binaries(&list[..2], 2).is_empty());
        let tie = [e("springboard.before-1", 7), e("springboard.before-2", 7), e("springboard.before-3", 7)];
        assert_eq!(old_binaries(&tie, 2), vec!["springboard.before-1".to_string()], "later names count as newer");
        assert_eq!(old_binaries(&tie, 0).len(), 3);
    }

    #[test]
    fn low_space_and_notice_spacing() {
        assert!(!is_low(0), "unknown is not low");
        assert!(is_low(LOW_BYTES - 1));
        assert!(!is_low(LOW_BYTES));
        assert!(notice_due(None, 100));
        assert!(!notice_due(Some(100), 100 + NOTICE_GAP_S - 1));
        assert!(notice_due(Some(100), 100 + NOTICE_GAP_S));
        assert!(notice_due(Some(100), 50), "a clock set back does not silence it forever");
        let mut g = Guard::default();
        assert!(g.note(1000));
        assert!(!g.note(1001));
    }

    #[test]
    fn sizes_and_clears_work_on_a_real_tree() {
        let dir = scratch("tree");
        std::fs::create_dir_all(dir.join("deildu/cache")).unwrap();
        std::fs::create_dir_all(dir.join("deildu/torrents")).unwrap();
        std::fs::create_dir_all(dir.join("cache/thumbs")).unwrap();
        std::fs::write(dir.join("deildu/cache/1.img"), [0u8; 100]).unwrap();
        std::fs::write(dir.join("deildu/torrents/a.torrent"), [0u8; 40]).unwrap();
        std::fs::write(dir.join("cache/thumbs/t"), [0u8; 10]).unwrap();
        std::fs::write(dir.join("log.old"), [0u8; 7]).unwrap();
        std::fs::write(dir.join("log"), [0u8; 3]).unwrap();
        for n in ["a", "b", "c"] {
            std::fs::write(dir.join(format!("springboard.before-{n}")), [0u8; 1]).unwrap();
        }
        assert_eq!(dir_size(&dir.join("deildu")), 140);
        assert_eq!(dir_size(&dir.join("missing")), 0);
        assert_eq!(Clear::DeilduCache.reclaimable(&dir), 100);
        assert_eq!(clear(Clear::DeilduCache, &dir), 100);
        assert!(dir.join("deildu/torrents/a.torrent").exists(), "saved torrents stay");
        assert!(!dir.join("deildu/cache").exists());
        assert_eq!(clear(Clear::OldLog, &dir), 7);
        assert!(dir.join("log").exists());
        let freed = trim_disposable(&dir);
        assert_eq!(freed, 10 + 1, "thumbs plus the one binary beyond the newest two");
        assert!(!dir.join("cache").exists());
        assert_eq!(binaries(&dir).len(), 2);
        let u = account(&[e("deildu", 40), e("log", 3)]);
        assert_eq!(u[3], (Category::Deildu, 40));
    }
}
