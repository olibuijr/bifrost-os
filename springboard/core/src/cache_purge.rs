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

/// Removes the cache folders a plan names. Each app's cache is the folder `root/<app>`. An app identifier that is not a
/// plain name (empty, containing a separator or `..`) is skipped, so a plan can never reach outside `root`, and a
/// symbolic link or file of that name is left alone. Returns how many folders were removed.
pub fn execute(plan: &PurgePlan, root: &std::path::Path) -> std::io::Result<usize> {
    let mut removed = 0;
    for app in &plan.victims {
        let plain = !app.is_empty() && app != "." && app != ".." && !app.contains(['/', '\\', '\0']);
        if !plain {
            continue;
        }
        let dir = root.join(app);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {
                std::fs::remove_dir_all(&dir)?;
                removed += 1;
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod execute_tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("cache-purge-{}-{}-{}", tag, std::process::id(), nanos));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_planned_caches_are_removed_and_the_rest_stay() {
        let root = scratch("planned");
        std::fs::create_dir_all(root.join("app.a/cache")).unwrap();
        std::fs::write(root.join("app.a/cache/blob"), b"12345").unwrap();
        std::fs::create_dir_all(root.join("app.b")).unwrap();
        let plan = PurgePlan { victims: vec!["app.a".into(), "app.missing".into()], reclaimed: 5, shortfall: 0 };
        assert_eq!(execute(&plan, &root).unwrap(), 1);
        assert!(!root.join("app.a").exists());
        assert!(root.join("app.b").is_dir(), "an app that was not planned keeps its cache");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_name_that_leaves_the_root_is_never_removed() {
        let base = scratch("escape");
        let root = base.join("caches");
        std::fs::create_dir_all(root.join("ok")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        let plan = PurgePlan { victims: vec!["../outside".into(), "..".into(), "a/b".into(), String::new()], reclaimed: 0, shortfall: 0 };
        assert_eq!(execute(&plan, &root).unwrap(), 0);
        assert!(base.join("outside").is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_file_with_a_cache_name_is_left_alone() {
        let root = scratch("file");
        std::fs::write(root.join("notes"), b"x").unwrap();
        let plan = PurgePlan { victims: vec!["notes".into()], reclaimed: 1, shortfall: 0 };
        assert_eq!(execute(&plan, &root).unwrap(), 0);
        assert!(root.join("notes").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
