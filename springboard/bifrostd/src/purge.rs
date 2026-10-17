//! Purgeable storage: the `deleted_helper` counterpart.
//!
//! When `/` or `/home` has less than 10 GiB or 5 % free, [`Purger::tick`] deletes purgeable data until 20 GiB (or
//! 10 % of the disk) is free. A "Free Up Space" request (the shell writes [`REQUEST_PATH`]) deletes all of it. The
//! candidates are, in this order:
//!
//! 1. app caches: each `~/.cache/<app>` directory of each home (never a symlink, never a file of the user's);
//! 2. the shell's thumbnail and artwork cache (`<root>/cache`, the same folder the Storage page clears);
//! 3. Recently Deleted items older than 30 days, only when `empty_trash_after_30_days = true` in
//!    [`POLICY_PATH`] (the person turned it on in Storage settings);
//! 4. pacman package files beyond the newest [`KEEP_PACKAGES`] of each package (`paccache -r` logic, in Rust).
//!
//! Planning reads the tree and returns the ordered list; [`select`] takes the prefix that reaches a byte budget and
//! [`execute`] deletes it. Only the listed paths are touched.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const GIB: u64 = 1 << 30;
/// Free space below this many bytes is low.
pub const LOW_BYTES: u64 = 10 * GIB;
/// The purge aims for this much free space (or 10 % of the disk, whichever is larger).
pub const TARGET_BYTES: u64 = 20 * GIB;
/// Package versions kept per package (the last two, as the OTA rule asks).
pub const KEEP_PACKAGES: usize = 2;
pub const PACMAN_CACHE: &str = "/var/cache/pacman/pkg";
pub const TRASH_DAYS: u64 = 30;
/// The hidden Recently Deleted folder under a home (springboard/src/files/trash.rs `DIR`).
pub const TRASH_DIR: &str = ".RecentlyDeleted";
/// Separates the deletion time from the original name in a trash item (springboard/src/files/trash.rs `SEP`).
const TRASH_SEP: &str = "__";
/// The shell's "Free Up Space" request. Written by the shell; bifrostd deletes it once the purge ran.
pub const REQUEST_PATH: &str = "/run/bifrost/purge-request";
/// The bytes the last requested purge reclaimed.
pub const RESULT_PATH: &str = "/run/bifrost/purge-result";
/// The bytes the purge could reclaim now (the Storage page's "Purgeable" line).
pub const PURGEABLE_PATH: &str = "/run/bifrost/purgeable";
/// The Storage settings that bifrostd reads (`key = true|false` lines).
pub const POLICY_PATH: &str = "/etc/bifrost/storage.conf";
/// The shell's data root (its `cache/` is the thumbnail cache).
pub const SHELL_ROOT: &str = "/var/lib/springboard";
/// Seconds between two space checks.
pub const CHECK_EVERY: u64 = 60;

/// What a candidate is, in purge order (the derived `Ord` is the order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Class {
    AppCache,
    ThumbnailCache,
    OldTrash,
    PackageCache,
}

/// One path the purge may delete, with the bytes it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub class: Class,
    pub path: PathBuf,
    pub bytes: u64,
}

/// Where the purge looks. `now` is unix seconds (trash ages are measured from it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub homes: Vec<PathBuf>,
    pub thumbnails: PathBuf,
    pub pacman_cache: PathBuf,
    pub trash_auto: bool,
    pub now: u64,
}

/// Free and total bytes of a filesystem (`free` is what an unprivileged user may use).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Space {
    pub free: u64,
    pub total: u64,
}

/// Whether `s` is low: under 10 GiB or under 5 % free.
pub fn low(s: Space) -> bool {
    s.free < LOW_BYTES || s.free.saturating_mul(20) < s.total
}

/// Bytes a purge must free to reach the target. Zero when there is enough.
pub fn need(s: Space) -> u64 {
    TARGET_BYTES.max(s.total / 10).min(s.total).saturating_sub(s.free)
}

/// Reads `key = true|false` lines. Missing or unknown keys are false: the trash is kept unless the person chose it.
pub fn parse_policy(text: &str) -> bool {
    text.lines().any(|line| {
        let Some((k, v)) = line.split_once('=') else { return false };
        k.trim() == "empty_trash_after_30_days" && v.trim() == "true"
    })
}

/// Current unix time in seconds (0 if the clock is before 1970).
pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Bytes under `path`, not following symlinks (a link counts as its own size).
pub fn size(path: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(path) else { return 0 };
    if md.is_dir() {
        fs::read_dir(path).map(|rd| rd.flatten().map(|e| size(&e.path())).sum()).unwrap_or(0)
    } else {
        md.len()
    }
}

/// The package name of a pacman file (`name-ver-rel-arch.pkg.tar.*`), None for anything else (signatures too).
pub fn pkg_name(file: &str) -> Option<&str> {
    if file.ends_with(".sig") {
        return None;
    }
    let idx = file.find(".pkg.tar")?;
    let base = &file[..idx];
    let parts: Vec<&str> = base.split('-').collect();
    if parts.len() < 4 {
        return None;
    }
    let tail = parts[parts.len() - 3..].join("-");
    let cut = base.len().checked_sub(tail.len() + 1)?;
    (cut > 0).then(|| &base[..cut])
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    v
}

/// The candidates for `cfg`, in purge order, with zero-byte ones left out.
pub fn plan(cfg: &Config) -> Vec<Candidate> {
    let mut out = Vec::new();

    // 1. app caches, biggest first
    let mut apps = Vec::new();
    for home in &cfg.homes {
        for p in entries(&home.join(".cache")) {
            let is_real_dir = fs::symlink_metadata(&p).map(|m| m.is_dir()).unwrap_or(false);
            if is_real_dir {
                apps.push(Candidate { class: Class::AppCache, bytes: size(&p), path: p });
            }
        }
    }
    apps.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.path.cmp(&b.path)));
    out.extend(apps);

    // 2. the shell's thumbnail and artwork cache
    let thumbs = size(&cfg.thumbnails);
    if thumbs > 0 {
        out.push(Candidate { class: Class::ThumbnailCache, path: cfg.thumbnails.clone(), bytes: thumbs });
    }

    // 3. Recently Deleted items past their 30 days, oldest first (only when the person chose it)
    if cfg.trash_auto {
        let mut old = Vec::new();
        for home in &cfg.homes {
            for p in entries(&home.join(TRASH_DIR)) {
                let stamped = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let Some((at, _)) = stamped.split_once(TRASH_SEP) else { continue };
                let Ok(at) = at.parse::<u64>() else { continue };
                if cfg.now.saturating_sub(at) >= TRASH_DAYS * 86_400 {
                    old.push((at, Candidate { class: Class::OldTrash, bytes: size(&p), path: p }));
                }
            }
        }
        old.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend(old.into_iter().map(|(_, c)| c).filter(|c| c.bytes > 0));
    }

    // 4. package files beyond the newest KEEP_PACKAGES versions of each package, oldest first
    let mut groups: BTreeMap<String, Vec<(SystemTime, PathBuf, u64)>> = BTreeMap::new();
    for p in entries(&cfg.pacman_cache) {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Some(pkg) = pkg_name(&name).map(str::to_string) else { continue };
        let Ok(md) = fs::symlink_metadata(&p) else { continue };
        if !md.is_file() {
            continue;
        }
        let when = md.modified().unwrap_or(UNIX_EPOCH);
        groups.entry(pkg).or_default().push((when, p, md.len()));
    }
    let mut old_pkgs = Vec::new();
    for files in groups.values_mut() {
        files.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        for (when, path, bytes) in files.iter().skip(KEEP_PACKAGES) {
            old_pkgs.push((*when, Candidate { class: Class::PackageCache, path: path.clone(), bytes: *bytes }));
        }
    }
    old_pkgs.sort_by(|a, b| a.0.cmp(&b.0));
    out.extend(old_pkgs.into_iter().map(|(_, c)| c).filter(|c| c.bytes > 0));

    out
}

/// The shortest prefix of `list` that frees at least `need` bytes (all of it when `need` is more than it holds).
pub fn select(list: &[Candidate], need: u64) -> Vec<&Candidate> {
    let mut got = 0u64;
    let mut out = Vec::new();
    for c in list {
        if got >= need {
            break;
        }
        got = got.saturating_add(c.bytes);
        out.push(c);
    }
    out
}

/// Deletes the chosen candidates. A directory is removed with its contents, but a symlink is only unlinked, never
/// followed. Returns the bytes of the items that were removed.
pub fn execute(chosen: &[&Candidate]) -> u64 {
    let mut reclaimed = 0u64;
    for c in chosen {
        let is_dir = fs::symlink_metadata(&c.path).map(|m| m.is_dir()).unwrap_or(false);
        let res = if is_dir { fs::remove_dir_all(&c.path) } else { fs::remove_file(&c.path) };
        if res.is_ok() {
            reclaimed = reclaimed.saturating_add(c.bytes);
            if c.class == Class::PackageCache {
                let _ = fs::remove_file(format!("{}.sig", c.path.display()));
            }
        }
    }
    reclaimed
}

#[cfg(target_os = "linux")]
#[allow(dead_code)]
#[repr(C)]
struct StatVfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    f_spare: [i32; 6],
}

#[cfg(target_os = "linux")]
extern "C" {
    fn statvfs(path: *const std::ffi::c_char, buf: *mut StatVfs) -> i32;
}

/// The space on the filesystem that holds `path` (None if it cannot be read, or off Linux).
#[cfg(target_os = "linux")]
pub fn space(path: &Path) -> Option<Space> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c` is NUL-terminated and `st` is a valid, writable StatVfs for the call.
    let mut st: StatVfs = unsafe { std::mem::zeroed() };
    if unsafe { statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frag = if st.f_frsize > 0 { st.f_frsize } else { st.f_bsize };
    Some(Space { free: st.f_bavail.saturating_mul(frag), total: st.f_blocks.saturating_mul(frag) })
}

#[cfg(not(target_os = "linux"))]
pub fn space(_path: &Path) -> Option<Space> {
    None
}

/// The homes to purge: the directories under `/home`, plus `/root` when it exists.
pub fn homes() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = entries(Path::new("/home")).into_iter().filter(|p| p.is_dir()).collect();
    let root = PathBuf::from("/root");
    if root.is_dir() {
        v.push(root);
    }
    v
}

/// Builds the live configuration from the system paths and the Storage policy.
pub fn live_config() -> Config {
    let trash_auto = fs::read_to_string(POLICY_PATH).map(|t| parse_policy(&t)).unwrap_or(false);
    Config {
        homes: homes(),
        thumbnails: Path::new(SHELL_ROOT).join("cache"),
        pacman_cache: PathBuf::from(PACMAN_CACHE),
        trash_auto,
        now: now_secs(),
    }
}

/// The daemon side of Storage: the low-space check, the explicit request and the Purgeable figure.
#[derive(Default)]
pub struct Purger {
    last_check: u64,
}

impl Purger {
    /// Runs the due work for `now` (unix seconds). Cheap when nothing is due.
    pub fn tick(&mut self, now: u64) {
        if Path::new(REQUEST_PATH).exists() {
            self.request(now);
            return;
        }
        if now.saturating_sub(self.last_check) < CHECK_EVERY {
            return;
        }
        self.last_check = now;
        let cfg = live_config();
        let plan = plan(&cfg);
        let _ = write_number(PURGEABLE_PATH, plan.iter().map(|c| c.bytes).sum());
        let mut freed = 0u64;
        for mount in ["/", "/home"] {
            let Some(s) = space(Path::new(mount)) else { continue };
            if low(s) {
                let chosen = select(&plan, need(s));
                freed += execute(&chosen);
            }
        }
        if freed > 0 {
            eprintln!("bifrostd: low space, purged {freed} bytes");
        }
    }

    /// "Free Up Space": deletes everything purgeable, writes the reclaimed bytes, and removes the request.
    fn request(&mut self, now: u64) {
        let _ = fs::remove_file(REQUEST_PATH);
        let plan = plan(&live_config());
        let all: Vec<&Candidate> = select(&plan, u64::MAX);
        let freed = execute(&all);
        let _ = write_number(RESULT_PATH, freed);
        self.last_check = now;
    }
}

fn write_number(path: &str, n: u64) -> std::io::Result<()> {
    if let Some(dir) = Path::new(path).parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, format!("{n}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const DAY: u64 = 86_400;

    /// A scratch tree under the system temp dir, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let p = std::env::temp_dir().join(format!("bifrostd-purge-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&p);
            fs::create_dir_all(&p).unwrap();
            Tree(p)
        }

        fn file(&self, rel: &str, bytes: usize) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, vec![b'x'; bytes]).unwrap();
            p
        }

        fn dir(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            fs::create_dir_all(&p).unwrap();
            p
        }

        fn age(&self, rel: &str, secs_since_epoch: u64) {
            let f = fs::File::options().write(true).open(self.0.join(rel)).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(secs_since_epoch)).unwrap();
        }

        fn path(&self, rel: &str) -> PathBuf {
            self.0.join(rel)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// alice's caches (appB 300 B, appA 100 B), a thumbnail cache, a documents folder, an old and a new trash
    /// item, and three versions of `foo` plus one of `bar` in the pacman cache.
    fn sample_tree(name: &str) -> (Tree, Config) {
        let t = Tree::new(name);
        let home = t.dir("home/alice");
        t.file("home/alice/.cache/appA/data", 100);
        t.file("home/alice/.cache/appB/data", 300);
        t.file("home/alice/Documents/report.txt", 5000);
        t.file("thumbs/0001.jpg", 50);
        t.file("home/alice/.RecentlyDeleted/1000__old.txt", 40);
        t.file("home/alice/.RecentlyDeleted/9999999999__new.txt", 60);
        t.file("pkg/foo-1.0-1-x86_64.pkg.tar.zst", 10);
        t.file("pkg/foo-1.1-1-x86_64.pkg.tar.zst", 11);
        t.file("pkg/foo-1.2-1-x86_64.pkg.tar.zst", 12);
        t.file("pkg/bar-2-1-any.pkg.tar.zst", 7);
        t.age("pkg/foo-1.0-1-x86_64.pkg.tar.zst", 1000);
        t.age("pkg/foo-1.1-1-x86_64.pkg.tar.zst", 2000);
        t.age("pkg/foo-1.2-1-x86_64.pkg.tar.zst", 3000);
        let cfg = Config {
            homes: vec![home],
            thumbnails: t.path("thumbs"),
            pacman_cache: t.path("pkg"),
            trash_auto: true,
            now: 1000 + 30 * DAY + 1,
        };
        (t, cfg)
    }

    #[test]
    fn plan_orders_caches_thumbnails_trash_then_old_packages() {
        let (_t, cfg) = sample_tree("order");
        let list = plan(&cfg);
        let classes: Vec<Class> = list.iter().map(|c| c.class).collect();
        assert_eq!(
            classes,
            vec![Class::AppCache, Class::AppCache, Class::ThumbnailCache, Class::OldTrash, Class::PackageCache]
        );
        assert!(list[0].path.ends_with("appB"), "biggest app cache first");
        assert!(list[1].path.ends_with("appA"));
        assert_eq!(list[0].bytes, 300);
        assert!(list[3].path.ends_with("1000__old.txt"), "the new trash item is kept");
        assert!(list[4].path.ends_with("foo-1.0-1-x86_64.pkg.tar.zst"), "the newest two foo versions are kept");
        assert_eq!(list.len(), 5, "bar has one version and the documents are never listed");
        assert!(list.iter().all(|c| !c.path.to_string_lossy().contains("Documents")));
    }

    #[test]
    fn trash_is_left_alone_when_the_setting_is_off() {
        let (_t, mut cfg) = sample_tree("trash-off");
        cfg.trash_auto = false;
        let list = plan(&cfg);
        assert!(list.iter().all(|c| c.class != Class::OldTrash));
        assert_eq!(list.len(), 4);
    }

    #[test]
    fn policy_reads_the_trash_key() {
        assert!(parse_policy("empty_trash_after_30_days = true\n"));
        assert!(!parse_policy("empty_trash_after_30_days = false\n"));
        assert!(!parse_policy("other = true\n"), "missing means off");
        assert!(!parse_policy(""));
    }

    #[test]
    fn package_names_are_split_from_the_version() {
        assert_eq!(pkg_name("bifrost-shell-1.2.3-1-x86_64.pkg.tar.zst"), Some("bifrost-shell"));
        assert_eq!(pkg_name("a-b-1-1-any.pkg.tar.xz"), Some("a-b"));
        assert_eq!(pkg_name("foo-1.0-1-x86_64.pkg.tar.zst.sig"), None, "signatures are not counted");
        assert_eq!(pkg_name("notes.txt"), None);
        assert_eq!(pkg_name("1-1-x86_64.pkg.tar.zst"), None, "no name before the version");
    }

    #[test]
    fn select_takes_the_shortest_prefix_that_reaches_the_need() {
        let c = |n: u64| Candidate { class: Class::AppCache, path: PathBuf::from(format!("/x{n}")), bytes: n };
        let list = vec![c(100), c(50), c(10)];
        assert_eq!(select(&list, 0).len(), 0);
        assert_eq!(select(&list, 100).len(), 1);
        assert_eq!(select(&list, 101).len(), 2);
        assert_eq!(select(&list, u64::MAX).len(), 3, "everything when the need is larger than the list");
    }

    #[test]
    fn low_space_and_need_follow_the_thresholds() {
        let disk = 500 * GIB;
        assert!(low(Space { free: 9 * GIB, total: disk }), "under 10 GiB");
        assert!(low(Space { free: 40 * GIB, total: 1000 * GIB }), "under 5 % of a 1 TB disk");
        assert!(!low(Space { free: 60 * GIB, total: disk }));
        assert_eq!(need(Space { free: 5 * GIB, total: disk }), 45 * GIB, "up to 10 % of a 500 GiB disk");
        assert_eq!(need(Space { free: 60 * GIB, total: disk }), 0);
        assert_eq!(need(Space { free: 1 * GIB, total: 2000 * GIB }), 199 * GIB, "10 % of a 2 TB disk is the target");
    }

    #[test]
    fn execute_removes_only_the_planned_items_and_reports_their_bytes() {
        let (t, cfg) = sample_tree("execute");
        let list = plan(&cfg);
        let chosen = select(&list, u64::MAX);
        let freed = execute(&chosen);
        assert_eq!(freed, 300 + 100 + 50 + 40 + 10, "the reported bytes are the removed items'");
        assert!(!t.path("home/alice/.cache/appB").exists());
        assert!(!t.path("home/alice/.RecentlyDeleted/1000__old.txt").exists());
        assert!(!t.path("pkg/foo-1.0-1-x86_64.pkg.tar.zst").exists());
        assert!(t.path("pkg/foo-1.2-1-x86_64.pkg.tar.zst").exists());
        assert!(t.path("pkg/bar-2-1-any.pkg.tar.zst").exists());
        assert!(t.path("home/alice/Documents/report.txt").exists(), "user documents are never deleted");
        assert!(t.path("home/alice/.RecentlyDeleted/9999999999__new.txt").exists());
    }

    #[test]
    fn a_cache_symlink_is_unlinked_and_its_target_kept() {
        let t = Tree::new("symlink");
        let docs = t.file("home/alice/Documents/keep.txt", 500);
        let cache = t.dir("home/alice/.cache");
        std::os::unix::fs::symlink(t.path("home/alice/Documents"), cache.join("link")).unwrap();
        let cfg = Config {
            homes: vec![t.path("home/alice")],
            thumbnails: t.path("none"),
            pacman_cache: t.path("none"),
            trash_auto: false,
            now: 0,
        };
        let list = plan(&cfg);
        assert!(list.is_empty(), "a symlink is not an app cache");
        let link_only = vec![Candidate { class: Class::AppCache, path: cache.join("link"), bytes: 0 }];
        execute(&link_only.iter().collect::<Vec<_>>());
        assert!(docs.exists(), "the link target survives");
    }

    #[test]
    fn purgeable_space_counts_the_planned_bytes() {
        let (_t, cfg) = sample_tree("purgeable");
        let total: u64 = plan(&cfg).iter().map(|c| c.bytes).sum();
        assert_eq!(total, 300 + 100 + 50 + 40 + 10);
    }

    #[test]
    fn statvfs_reads_the_root_filesystem() {
        let s = space(Path::new("/"));
        if let Some(s) = s {
            assert!(s.total >= s.free && s.total > 0);
        }
    }
}
