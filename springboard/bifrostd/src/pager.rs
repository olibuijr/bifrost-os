//! Swap and memory pressure: the `dynamic_pager` counterpart.
//!
//! At boot `bifrostd zram-setup` (run by `bifrost-zram.service`) puts a zram swap device of half the RAM (at most
//! 8 GiB) at priority 100. While the service runs, [`Pager::tick`] reads `/proc/meminfo`, `/proc/pressure/memory`
//! (PSI) and `/proc/swaps`; under pressure with less than 25 % of swap free it adds one 1 GiB swap file in
//! `/var/cache/bifrost/swap` (priority -2, so zram is used first). After 10 minutes of normal pressure it removes the
//! newest swap file that holds no pages.
//!
//! The swap directory is on the `@cache` subvolume (`BTRFS_LAYOUT` in the installer), which is never snapshotted: an
//! active swap file would make `btrfs subvolume snapshot @system` fail, and the root subvolume is compressed and
//! copy-on-write, which btrfs refuses for swap. A file is created empty, marked `chattr +C` (no copy-on-write, so no
//! compression) before any data is written, then preallocated. Creation needs free space for the file plus a reserve. A
//! failure backs off (10 minutes, doubling); a failure that cannot get better (the filesystem refuses the file for swap)
//! switches disk swap off until bifrostd restarts, and zram carries on alone. The level (`normal`, `warn`, `critical`) is written to
//! `/run/bifrost/memory-pressure` for the shell and tools; apps keep their in-process `DispatchSource` source.
//!
//! The decisions are pure ([`decide`], [`classify`], the parsers) so they can be tested with fixed inputs. The
//! effects (fallocate, mkswap, swapon, swapoff) are in [`create_swap_file`], [`remove_swap_file`] and [`Pager`].

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const GIB: u64 = 1 << 30;
/// zram is never larger than this, whatever the RAM.
pub const ZRAM_MAX: u64 = 8 * GIB;
pub const SWAP_DIR: &str = "/var/cache/bifrost/swap";
pub const SWAP_FILE_BYTES: u64 = GIB;
/// Swap files bifrostd may hold at once (8 GiB of disk swap).
pub const MAX_SWAP_FILES: usize = 8;
/// Free swap below this share (percent of swap total) under pressure adds a file.
pub const LOW_SWAP_PCT: u64 = 25;
/// Free disk space that must remain after a swap file is created.
pub const SPACE_RESERVE: u64 = 2 * GIB;
/// Seconds to wait after the first failed creation; it doubles with each failure, up to [`CREATE_BACKOFF_MAX`].
pub const CREATE_BACKOFF_FIRST: u64 = 600;
pub const CREATE_BACKOFF_MAX: u64 = 6 * 3600;
/// Failures in a row after which disk swap is given up for this run of bifrostd.
pub const MAX_CREATE_FAILURES: u32 = 4;
/// Seconds of normal pressure before an unused swap file is removed.
pub const CALM_FOR: u64 = 600;
/// Where the level is published (the `/run/bifrost` directory is created by privd's RuntimeDirectory).
pub const LEVEL_PATH: &str = "/run/bifrost/memory-pressure";
pub const ZRAM_DEV: &str = "/dev/zram0";
pub const ZRAM_SYS: &str = "/sys/block/zram0";
/// PSI `avg10` thresholds: "some" stall share for a warning, "full" or a high "some" for critical.
pub const WARN_SOME: f64 = 10.0;
pub const CRIT_FULL: f64 = 10.0;
pub const CRIT_SOME: f64 = 40.0;

/// The fields of `/proc/meminfo` the pager reads, in bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meminfo {
    pub total: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_free: u64,
}

/// Reads `MemTotal`, `MemAvailable`, `SwapTotal` and `SwapFree` (values are kB). None unless the first two exist.
pub fn parse_meminfo(text: &str) -> Option<Meminfo> {
    let (mut total, mut available, mut swap_total, mut swap_free) = (None, None, 0, 0);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(key), Some(val)) = (it.next(), it.next()) else { continue };
        let Ok(kb) = val.parse::<u64>() else { continue };
        let bytes = kb.saturating_mul(1024);
        match key {
            "MemTotal:" => total = Some(bytes),
            "MemAvailable:" => available = Some(bytes),
            "SwapTotal:" => swap_total = bytes,
            "SwapFree:" => swap_free = bytes,
            _ => {}
        }
    }
    Some(Meminfo { total: total?, available: available?, swap_total, swap_free })
}

impl Meminfo {
    /// The live `/proc/meminfo`, None when it cannot be read.
    pub fn from_proc() -> Option<Meminfo> {
        parse_meminfo(&fs::read_to_string("/proc/meminfo").ok()?)
    }
}

/// The `avg10` share of "some" and "full" stall time from `/proc/pressure/memory`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Psi {
    pub some10: f64,
    pub full10: f64,
}

/// Parses the `some` and `full` lines. None unless both are present.
pub fn parse_psi(text: &str) -> Option<Psi> {
    let (mut some, mut full) = (None, None);
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(kind) = it.next() else { continue };
        let avg10 = it.find_map(|kv| kv.strip_prefix("avg10=")).and_then(|v| v.parse::<f64>().ok());
        match (kind, avg10) {
            ("some", Some(v)) => some = Some(v),
            ("full", Some(v)) => full = Some(v),
            _ => {}
        }
    }
    Some(Psi { some10: some?, full10: full? })
}

/// One row of `/proc/swaps`: a swap partition, zram device or swap file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapDev {
    pub path: String,
    pub size: u64,
    pub used: u64,
}

/// Parses `/proc/swaps` (a header line, then `Filename Type Size Used Priority` with sizes in kB).
pub fn parse_swaps(text: &str) -> Vec<SwapDev> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 {
                return None;
            }
            Some(SwapDev {
                path: f[0].to_string(),
                size: f[2].parse::<u64>().ok()?.saturating_mul(1024),
                used: f[3].parse::<u64>().ok()?.saturating_mul(1024),
            })
        })
        .collect()
}

/// The published pressure level. Ordered: `Normal < Warning < Critical`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    #[default]
    Normal,
    Warning,
    Critical,
}

impl Level {
    /// The word written to [`LEVEL_PATH`].
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Normal => "normal",
            Level::Warning => "warn",
            Level::Critical => "critical",
        }
    }
}

/// The level for a PSI sample: critical when every task stalls ("full") or most do, warning when some do.
pub fn classify(p: Psi) -> Level {
    if p.full10 >= CRIT_FULL || p.some10 >= CRIT_SOME {
        Level::Critical
    } else if p.some10 >= WARN_SOME {
        Level::Warning
    } else {
        Level::Normal
    }
}

/// zram size for `ram` bytes: half the RAM, at most [`ZRAM_MAX`]; none below 1 GiB of RAM.
pub fn zram_bytes(ram: u64) -> u64 {
    if ram < GIB {
        0
    } else {
        (ram / 2).min(ZRAM_MAX)
    }
}

/// The pager's memory between ticks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// the level last published
    pub level: Level,
    /// when pressure became normal (unix seconds); None while it is not
    pub calm_since: Option<u64>,
    /// the swap files this pager created (absolute paths, in creation order)
    pub files: Vec<String>,
    /// no new swap file before this time (unix seconds) after a failed creation
    pub create_blocked_until: u64,
    /// creations that failed in a row
    pub create_failures: u32,
    /// disk swap is given up (a definite failure, or too many in a row); zram carries on
    pub swap_disabled: bool,
}

/// The wait before the next creation after `failures` failures in a row: 10 minutes, doubling, at most 6 hours.
pub fn create_backoff(failures: u32) -> u64 {
    CREATE_BACKOFF_FIRST.saturating_mul(1u64 << failures.saturating_sub(1).min(10)).min(CREATE_BACKOFF_MAX)
}

/// Why a swap file could not be made. `definite` means retrying cannot help (the filesystem refuses the file for swap).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SwapError {
    pub msg: String,
    pub definite: bool,
}

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl State {
    /// Records a failed creation at `now`: back off, or give up when the failure was definite or too many came in a row.
    pub fn creation_failed(&mut self, e: &SwapError, now: u64) {
        self.create_failures = self.create_failures.saturating_add(1);
        self.create_blocked_until = now.saturating_add(create_backoff(self.create_failures));
        if e.definite || self.create_failures >= MAX_CREATE_FAILURES {
            self.swap_disabled = true;
        }
    }
}

/// What the effects layer must do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    CreateSwapFile(String),
    RemoveSwapFile(String),
    Publish(Level),
}

/// The first `swapN` path under [`SWAP_DIR`] that `files` does not use.
pub fn next_path(files: &[String]) -> String {
    let n = (1u32..).find(|n| !files.iter().any(|f| *f == format!("{SWAP_DIR}/swap{n}"))).unwrap_or(1);
    format!("{SWAP_DIR}/swap{n}")
}

/// The pager's decision for one sample. Returns the actions and the state to keep.
///
/// `m` gives the swap totals, `swaps` says which devices hold pages (`used`), `now` is unix seconds.
pub fn decide(m: &Meminfo, psi: Psi, swaps: &[SwapDev], st: &State, now: u64) -> (Vec<Action>, State) {
    let level = classify(psi);
    let mut next = st.clone();
    let mut acts = Vec::new();
    if level != st.level {
        next.level = level;
        acts.push(Action::Publish(level));
    }
    if level == Level::Normal {
        let since = *next.calm_since.get_or_insert(now);
        if now.saturating_sub(since) >= CALM_FOR {
            let unused = st.files.iter().rev().find(|f| swaps.iter().any(|s| s.path == **f && s.used == 0));
            if let Some(path) = unused.cloned() {
                next.files.retain(|f| *f != path);
                next.calm_since = Some(now);
                acts.push(Action::RemoveSwapFile(path));
            }
        }
    } else {
        next.calm_since = None;
        let free_pct = if m.swap_total == 0 { 0 } else { m.swap_free.saturating_mul(100) / m.swap_total };
        let allowed = !st.swap_disabled && now >= st.create_blocked_until;
        if allowed && (m.swap_total == 0 || free_pct < LOW_SWAP_PCT) && st.files.len() < MAX_SWAP_FILES {
            let path = next_path(&st.files);
            next.files.push(path.clone());
            acts.push(Action::CreateSwapFile(path));
        }
    }
    (acts, next)
}

/// Runs one external tool; its error text names the tool.
fn run(args: &[&str]) -> Result<(), String> {
    let (tool, rest) = args.split_first().ok_or("no command")?;
    let status = Command::new(tool).args(rest).status().map_err(|e| format!("{tool}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{tool} exited with {status}"))
    }
}

/// Free bytes for an unprivileged writer on the filesystem holding `dir`, from the output of `stat -f -c "%a %S" dir`
/// (free blocks, block size).
pub fn parse_free_bytes(text: &str) -> Option<u64> {
    let mut it = text.split_whitespace();
    let blocks: u64 = it.next()?.parse().ok()?;
    let size: u64 = it.next()?.parse().ok()?;
    Some(blocks.saturating_mul(size))
}

/// Whether a swap file of `need` bytes fits in `free` bytes and leaves [`SPACE_RESERVE`].
pub fn has_room(free: u64, need: u64) -> bool {
    free >= need.saturating_add(SPACE_RESERVE)
}

/// The output of `stat -f -c <format> dir`.
fn stat_fs(dir: &str, format: &str) -> Option<String> {
    let out = Command::new("stat").args(["-f", "-c", format, dir]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn transient(msg: String) -> SwapError {
    SwapError { msg, definite: false }
}

fn definite(msg: String) -> SwapError {
    SwapError { msg, definite: true }
}

/// Creates, formats and enables one swap file of [`SWAP_FILE_BYTES`] at `path` with priority -2.
///
/// The file is made empty and marked no-copy-on-write before it gets any data (btrfs honours the flag only on an empty
/// file), then preallocated. On btrfs a refusal of the flag, or of `swapon`, is definite. Anything left behind by a
/// failure is the caller's to remove.
pub fn create_swap_file(path: &str) -> Result<(), SwapError> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::create_dir_all(SWAP_DIR).map_err(|e| transient(format!("{SWAP_DIR}: {e}")))?;
    let free = stat_fs(SWAP_DIR, "%a %S").and_then(|t| parse_free_bytes(&t)).ok_or_else(|| transient(format!("{SWAP_DIR}: cannot read the free space")))?;
    if !has_room(free, SWAP_FILE_BYTES) {
        return Err(transient(format!("not enough free space for a swap file ({} MiB free)", free >> 20)));
    }
    let on_btrfs = stat_fs(SWAP_DIR, "%T").is_some_and(|t| t == "btrfs");
    let _ = fs::remove_file(path);
    fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| transient(format!("{path}: {e}")))?;
    // an ordinary filesystem may not know the flag; on btrfs the file cannot be swap without it
    if let Err(e) = run(&["chattr", "+C", path]) {
        if on_btrfs {
            return Err(definite(format!("no-copy-on-write flag: {e}")));
        }
    }
    let size = SWAP_FILE_BYTES.to_string();
    run(&["fallocate", "-l", size.as_str(), path]).map_err(transient)?;
    run(&["mkswap", path]).map_err(transient)?;
    run(&["swapon", "-p", "-2", path]).map_err(|e| SwapError { definite: on_btrfs, msg: e })
}

/// Disables and deletes a swap file. The file stays if it cannot be disabled.
pub fn remove_swap_file(path: &str) -> Result<(), String> {
    run(&["swapoff", path])?;
    fs::remove_file(path).map_err(|e| format!("{path}: {e}"))
}

/// Writes the level to `path` atomically (temporary file, then rename), so readers never see a partial word.
pub fn publish(level: Level, path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, format!("{}\n", level.as_str()))?;
    fs::rename(&tmp, path)
}

/// Boot step: sets up the zram swap device for `ram` bytes of RAM. Does nothing when the device already swaps,
/// so a second run (the unit is `RemainAfterExit`) never re-formats it. Returns the size it set up (0 if none).
pub fn zram_setup(ram: u64) -> Result<u64, String> {
    let bytes = zram_bytes(ram);
    if bytes == 0 {
        return Ok(0);
    }
    let active = fs::read_to_string("/proc/swaps").unwrap_or_default();
    if parse_swaps(&active).iter().any(|s| s.path == ZRAM_DEV) {
        return Ok(0);
    }
    if !Path::new(ZRAM_SYS).exists() {
        run(&["modprobe", "zram", "num_devices=1"])?;
    }
    // older kernels have no comp_algorithm: the default compressor is fine then
    let _ = fs::write(format!("{ZRAM_SYS}/comp_algorithm"), "zstd");
    fs::write(format!("{ZRAM_SYS}/disksize"), bytes.to_string()).map_err(|e| format!("zram disksize: {e}"))?;
    run(&["mkswap", ZRAM_DEV])?;
    run(&["swapon", "-p", "100", ZRAM_DEV])?;
    Ok(bytes)
}

/// Reads the three sources the pager decides from. None when `/proc/meminfo` or PSI is missing (no PSI on an old
/// kernel: the pager then does nothing and the swap files stay as they are).
fn sample() -> Option<(Meminfo, Psi, Vec<SwapDev>)> {
    let m = parse_meminfo(&fs::read_to_string("/proc/meminfo").ok()?)?;
    let p = parse_psi(&fs::read_to_string("/proc/pressure/memory").ok()?)?;
    let s = parse_swaps(&fs::read_to_string("/proc/swaps").unwrap_or_default());
    Some((m, p, s))
}

/// The swap files already in [`SWAP_DIR`] (from before a restart of bifrostd), as absolute paths.
fn existing_files() -> Vec<String> {
    let Ok(rd) = fs::read_dir(SWAP_DIR) else { return Vec::new() };
    let mut out: Vec<String> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("swap"))
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The running pager: one [`tick`](Pager::tick) per supervisor loop turn.
pub struct Pager {
    pub state: State,
    pub level_path: PathBuf,
}

impl Pager {
    pub fn new() -> Pager {
        Pager { state: State { files: existing_files(), ..State::default() }, level_path: PathBuf::from(LEVEL_PATH) }
    }

    /// Samples the system, decides and carries out the actions. A failed creation is forgotten (the file is removed)
    /// and backs creation off, or switches disk swap off when it cannot succeed; a failed removal keeps the file in the
    /// state so it is retried after the next calm spell.
    pub fn tick(&mut self, now: u64) {
        let Some((m, p, s)) = sample() else { return };
        let (acts, next) = decide(&m, p, &s, &self.state, now);
        self.state = next;
        for a in acts {
            match a {
                Action::CreateSwapFile(path) => {
                    match create_swap_file(&path) {
                        Ok(()) => self.state.create_failures = 0,
                        Err(e) => {
                            self.state.files.retain(|f| *f != path);
                            let _ = fs::remove_file(&path);
                            self.state.creation_failed(&e, now);
                            if self.state.swap_disabled {
                                eprintln!("bifrostd: swap file {path}: {e}; disk swap is off until bifrostd restarts (zram carries on)");
                            } else {
                                eprintln!("bifrostd: swap file {path}: {e}; next try in {} s", create_backoff(self.state.create_failures));
                            }
                        }
                    }
                }
                Action::RemoveSwapFile(path) => {
                    if let Err(e) = remove_swap_file(&path) {
                        eprintln!("bifrostd: remove swap file {path}: {e}");
                        self.state.files.push(path);
                    }
                }
                Action::Publish(level) => {
                    if let Err(e) = publish(level, &self.level_path) {
                        eprintln!("bifrostd: memory pressure file: {e}");
                    }
                }
            }
        }
    }
}

impl Default for Pager {
    fn default() -> Self {
        Pager::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       16384000 kB\nMemFree:         1000000 kB\nMemAvailable:    8000000 kB\nSwapTotal:       4096000 kB\nSwapFree:        1024000 kB\n";
    const PSI: &str = "some avg10=12.50 avg60=3.00 avg300=1.00 total=123\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=0\n";

    fn psi(some10: f64, full10: f64) -> Psi {
        Psi { some10, full10 }
    }

    fn swap(path: &str, used: u64) -> SwapDev {
        SwapDev { path: path.to_string(), size: GIB, used }
    }

    #[test]
    fn meminfo_reads_bytes_and_needs_the_ram_lines() {
        let m = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(m.total, 16_384_000 * 1024);
        assert_eq!(m.available, 8_000_000 * 1024);
        assert_eq!(m.swap_total, 4_096_000 * 1024);
        assert_eq!(m.swap_free, 1_024_000 * 1024);
        assert!(parse_meminfo("SwapTotal: 1 kB\n").is_none(), "no MemTotal or MemAvailable");
        let no_swap = parse_meminfo("MemTotal: 1 kB\nMemAvailable: 1 kB\n").unwrap();
        assert_eq!((no_swap.swap_total, no_swap.swap_free), (0, 0));
    }

    #[test]
    fn psi_reads_the_avg10_of_both_lines() {
        assert_eq!(parse_psi(PSI), Some(psi(12.5, 0.0)));
        assert_eq!(parse_psi("some avg10=1.00 avg60=0.00\n"), None, "the full line is missing");
        assert_eq!(parse_psi("some avg10=x\nfull avg10=0.5\n"), None, "a bad number is not a sample");
    }

    #[test]
    fn swaps_skip_the_header_and_read_kib() {
        let text = "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n/dev/zram0 partition 8388604 1024 100\n/var/lib/bifrost/swap/swap1 file 1048576 0 -2\n";
        let s = parse_swaps(text);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0], SwapDev { path: "/dev/zram0".into(), size: 8_388_604 * 1024, used: 1024 * 1024 });
        assert_eq!(s[1].used, 0);
        assert!(parse_swaps("").is_empty());
    }

    #[test]
    fn classify_follows_the_psi_thresholds() {
        assert_eq!(classify(psi(0.0, 0.0)), Level::Normal);
        assert_eq!(classify(psi(9.9, 0.0)), Level::Normal);
        assert_eq!(classify(psi(10.0, 0.0)), Level::Warning);
        assert_eq!(classify(psi(39.9, 9.9)), Level::Warning);
        assert_eq!(classify(psi(10.0, 10.0)), Level::Critical, "full stall is critical");
        assert_eq!(classify(psi(40.0, 0.0)), Level::Critical);
        assert!(Level::Normal < Level::Warning && Level::Warning < Level::Critical);
        assert_eq!(Level::Warning.as_str(), "warn");
    }

    #[test]
    fn zram_is_half_the_ram_capped_at_eight_gib() {
        assert_eq!(zram_bytes(GIB / 2), 0);
        assert_eq!(zram_bytes(4 * GIB), 2 * GIB);
        assert_eq!(zram_bytes(16 * GIB), ZRAM_MAX);
        assert_eq!(zram_bytes(64 * GIB), ZRAM_MAX);
    }

    #[test]
    fn next_path_takes_the_first_free_index() {
        assert_eq!(next_path(&[]), format!("{SWAP_DIR}/swap1"));
        let files = vec![format!("{SWAP_DIR}/swap1"), format!("{SWAP_DIR}/swap3")];
        assert_eq!(next_path(&files), format!("{SWAP_DIR}/swap2"));
    }

    /// The pager's decision table: (case, level, swap free %, files held, expected actions).
    #[test]
    fn decision_table_under_pressure() {
        let mem = |free_pct: u64| Meminfo { total: 16 * GIB, available: 8 * GIB, swap_total: 4 * GIB, swap_free: 4 * GIB * free_pct / 100 };
        let fresh = State::default();
        let path1 = format!("{SWAP_DIR}/swap1");

        // warning with little swap free: a file is added and the level published
        let (a, s) = decide(&mem(10), psi(12.0, 0.0), &[], &fresh, 100);
        assert_eq!(a, vec![Action::Publish(Level::Warning), Action::CreateSwapFile(path1.clone())]);
        assert_eq!(s.files, vec![path1.clone()]);

        // warning with enough swap free: only the level
        let (a, _) = decide(&mem(60), psi(12.0, 0.0), &[], &fresh, 100);
        assert_eq!(a, vec![Action::Publish(Level::Warning)]);

        // critical, no swap at all: a file is added (total 0 counts as low)
        let none = Meminfo { swap_total: 0, swap_free: 0, ..mem(0) };
        let (a, _) = decide(&none, psi(0.0, 12.0), &[], &fresh, 100);
        assert_eq!(a[0], Action::Publish(Level::Critical));
        assert!(matches!(a[1], Action::CreateSwapFile(_)));

        // the file limit stops creation
        let full = State { level: Level::Warning, files: (1..=MAX_SWAP_FILES).map(|i| format!("{SWAP_DIR}/swap{i}")).collect(), ..State::default() };
        let (a, _) = decide(&mem(0), psi(12.0, 0.0), &[], &full, 100);
        assert!(a.is_empty(), "already at {MAX_SWAP_FILES} files, level unchanged");
    }

    #[test]
    fn the_swap_directory_is_off_the_snapshotted_compressed_root() {
        assert_eq!(SWAP_DIR, "/var/cache/bifrost/swap", "@cache, not @system");
        assert!(!SWAP_DIR.starts_with("/var/lib"));
    }

    #[test]
    fn free_space_is_read_from_stat_and_must_leave_a_reserve() {
        assert_eq!(parse_free_bytes("262144 4096\n"), Some(GIB));
        assert_eq!(parse_free_bytes("x 4096"), None);
        assert_eq!(parse_free_bytes(""), None);
        assert!(!has_room(2 * GIB, GIB), "the reserve is kept");
        assert!(has_room(GIB + SPACE_RESERVE, GIB));
    }

    #[test]
    fn creation_backs_off_and_a_definite_failure_ends_disk_swap() {
        assert_eq!(create_backoff(1), 600);
        assert_eq!(create_backoff(2), 1200);
        assert_eq!(create_backoff(3), 2400);
        assert_eq!(create_backoff(40), CREATE_BACKOFF_MAX);

        let mem = Meminfo { total: 16 * GIB, available: GIB, swap_total: 4 * GIB, swap_free: 0 };
        let mut st = State::default();
        st.creation_failed(&transient("no space".into()), 1000);
        assert!(!st.swap_disabled);
        assert_eq!(st.create_blocked_until, 1600);
        // while blocked, pressure creates nothing, and the level is still published
        let (a, _) = decide(&mem, psi(12.0, 0.0), &[], &st, 1500);
        assert_eq!(a, vec![Action::Publish(Level::Warning)]);
        // after the wait it tries again
        let (a, _) = decide(&mem, psi(12.0, 0.0), &[], &st, 1600);
        assert!(a.iter().any(|x| matches!(x, Action::CreateSwapFile(_))));

        // a definite failure gives up at once
        let mut st = State::default();
        st.creation_failed(&definite("swapon: Invalid argument".into()), 1000);
        assert!(st.swap_disabled);
        let (a, _) = decide(&mem, psi(12.0, 0.0), &[], &st, u64::MAX / 2);
        assert!(!a.iter().any(|x| matches!(x, Action::CreateSwapFile(_))), "no retry after a definite failure");

        // repeated transient failures give up too
        let mut st = State::default();
        for i in 0..MAX_CREATE_FAILURES {
            assert!(!st.swap_disabled, "failure {i}");
            st.creation_failed(&transient("busy".into()), 1000);
        }
        assert!(st.swap_disabled);
    }

    #[test]
    fn calm_pressure_removes_an_unused_file_after_ten_minutes() {
        let path1 = format!("{SWAP_DIR}/swap1");
        let path2 = format!("{SWAP_DIR}/swap2");
        let held = State { level: Level::Normal, files: vec![path1.clone(), path2.clone()], ..State::default() };
        let mem = Meminfo { total: 16 * GIB, available: 8 * GIB, swap_total: 0, swap_free: 0 };
        let swaps = vec![swap(&path1, 0), swap(&path2, 4096)];

        // first calm sample only starts the clock
        let (a, s) = decide(&mem, psi(0.0, 0.0), &swaps, &held, 1000);
        assert!(a.is_empty());
        assert_eq!(s.calm_since, Some(1000));

        // 9 minutes is not enough
        let (a, _) = decide(&mem, psi(0.0, 0.0), &swaps, &s, 1000 + CALM_FOR - 1);
        assert!(a.is_empty());

        // 10 minutes: swap2 holds pages, so swap1 (unused) goes
        let (a, s2) = decide(&mem, psi(0.0, 0.0), &swaps, &s, 1000 + CALM_FOR);
        assert_eq!(a, vec![Action::RemoveSwapFile(path1.clone())]);
        assert_eq!(s2.files, vec![path2.clone()]);
        assert_eq!(s2.calm_since, Some(1000 + CALM_FOR), "the next removal waits another spell");

        // pressure returns: the calm clock resets
        let (_, s3) = decide(&mem, psi(12.0, 0.0), &swaps, &s2, 2000);
        assert_eq!(s3.calm_since, None);
    }

    #[test]
    fn a_file_with_pages_in_it_is_never_removed() {
        let path1 = format!("{SWAP_DIR}/swap1");
        let st = State { level: Level::Normal, calm_since: Some(0), files: vec![path1.clone()], ..State::default() };
        let mem = Meminfo { total: GIB, available: GIB, swap_total: GIB, swap_free: 0 };
        let (a, s) = decide(&mem, psi(0.0, 0.0), &[swap(&path1, 1)], &st, CALM_FOR * 3);
        assert!(a.is_empty());
        assert_eq!(s.files, vec![path1]);
    }
}
