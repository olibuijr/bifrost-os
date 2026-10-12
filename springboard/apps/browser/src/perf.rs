//! Performance hooks the app loop and the devtools features call (main.rs gets one-line call sites only).
use crate::store::Store;

/// Call once per loop turn / timer: flushes debounced history writes. Cheap when nothing is pending.
pub fn tick(store: &mut Store) {
    store.tick();
}

/// Call on quit paths that skip destructors (`process::exit`).
pub fn on_exit(store: &mut Store) {
    store.flush();
}

/// Proportional set size in KiB of process `pid` (`/proc/PID/smaps_rollup`), for app, engine and content processes.
pub fn pss_kib(pid: u32) -> Option<u64> {
    let t = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).ok()?;
    t.lines().find_map(|l| l.strip_prefix("Pss:")?.trim().trim_end_matches("kB").trim().parse().ok())
}

/// PSS in KiB of `pid` plus all its descendants (engine and content processes), walking `/proc/*/stat` parents.
pub fn tree_pss_kib(pid: u32) -> u64 {
    let mut parent: Vec<(u32, u32)> = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/proc") {
        for e in dir.flatten() {
            let Some(p) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{p}/stat")) else { continue };
            // the command name may contain spaces and parentheses: fields restart after the last ')'
            let ppid = stat.rsplit_once(')').and_then(|(_, r)| r.split_whitespace().nth(1)).and_then(|f| f.parse().ok());
            if let Some(pp) = ppid {
                parent.push((p, pp));
            }
        }
    }
    let mut set = vec![pid];
    let mut i = 0;
    while i < set.len() {
        let cur = set[i];
        let kids: Vec<u32> = parent.iter().filter(|(p, pp)| *pp == cur && !set.contains(p)).map(|(p, _)| *p).collect();
        set.extend(kids);
        i += 1;
    }
    set.into_iter().filter_map(pss_kib).sum()
}
