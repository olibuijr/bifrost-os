//! Cache purge plan: chooses which per-app caches to clear when free disk space falls below a target.
//!
//! Caches are cleared least recently used first. An app that is running keeps its cache. The plan stops as soon as
//! the cleared bytes cover the shortfall, and reports any shortfall that remains. The plan only names apps; removing
//! their cache folders is the caller's job.

/// One app's cache folder as the purge sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    /// The app identifier that owns the cache.
    pub app: String,
    /// The size of the cache folder in bytes.
    pub bytes: u64,
    /// When the app last ran, in seconds since the epoch. Older entries are purged first.
    pub last_used: u64,
    /// Whether the app is running now. A running app is never purged.
    pub running: bool,
}

/// The outcome of planning a purge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgePlan {
    /// The apps whose caches are cleared, in the order they are chosen.
    pub victims: Vec<String>,
    /// The bytes the chosen caches hold.
    pub reclaimed: u64,
    /// The bytes still missing after every eligible cache is chosen. Zero when the target is reached.
    pub shortfall: u64,
}

/// Plans the purge that brings free space from `free` up to `target_free` bytes.
pub fn plan_purge(entries: &[CacheEntry], free: u64, target_free: u64) -> PurgePlan {
    let need = target_free.saturating_sub(free);
    if need == 0 {
        return PurgePlan { victims: Vec::new(), reclaimed: 0, shortfall: 0 };
    }
    let mut order: Vec<&CacheEntry> = entries.iter().filter(|e| !e.running && e.bytes > 0).collect();
    order.sort_by(|a, b| a.last_used.cmp(&b.last_used).then_with(|| a.app.cmp(&b.app)));
    let mut victims = Vec::new();
    let mut reclaimed: u64 = 0;
    for entry in order {
        if reclaimed >= need {
            break;
        }
        reclaimed = reclaimed.saturating_add(entry.bytes);
        victims.push(entry.app.clone());
    }
    PurgePlan { victims, reclaimed, shortfall: need.saturating_sub(reclaimed) }
}
