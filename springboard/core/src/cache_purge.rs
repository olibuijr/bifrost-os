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

/// Removes the cache folders a plan names. The app's data folder is `root/<app>` and its cache is the folder `cache`
/// inside it (`Library/Caches` for a container); only that folder is removed, never the data beside it. An app
/// identifier that is not a plain name (empty, containing a separator or `..`) is skipped, and `cache` must be a
/// relative path without `..`, so a plan can never reach outside `root`. A symbolic link or file at the cache path is
/// left alone. Returns how many folders were removed.
pub fn execute(plan: &PurgePlan, root: &std::path::Path, cache: &str) -> std::io::Result<usize> {
    let relative = std::path::Path::new(cache);
    let plain_cache = relative.components().all(|c| matches!(c, std::path::Component::Normal(_))) && !cache.is_empty();
    if !plain_cache {
        return Ok(0);
    }
    let mut removed = 0;
    for app in &plan.victims {
        let plain = !app.is_empty() && app != "." && app != ".." && !app.contains(['/', '\\', '\0']);
        if !plain {
            continue;
        }
        let dir = root.join(app).join(relative);
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
    fn only_the_planned_cache_folders_are_removed_and_the_data_stays() {
        let root = scratch("planned");
        std::fs::create_dir_all(root.join("app.a/Library/Caches")).unwrap();
        std::fs::write(root.join("app.a/Library/Caches/blob"), b"12345").unwrap();
        std::fs::write(root.join("app.a/Library/prefs"), b"keep").unwrap();
        std::fs::create_dir_all(root.join("app.b/Library/Caches")).unwrap();
        let plan = PurgePlan { victims: vec!["app.a".into(), "app.missing".into()], reclaimed: 5, shortfall: 0 };
        assert_eq!(execute(&plan, &root, "Library/Caches").unwrap(), 1);
        assert!(!root.join("app.a/Library/Caches").exists());
        assert!(root.join("app.a/Library/prefs").is_file(), "the app's own data is not removed");
        assert!(root.join("app.b/Library/Caches").is_dir(), "an app that was not planned keeps its cache");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_name_that_leaves_the_root_is_never_removed() {
        let base = scratch("escape");
        let root = base.join("appdata");
        std::fs::create_dir_all(root.join("ok/Library/Caches")).unwrap();
        std::fs::create_dir_all(base.join("outside/Library/Caches")).unwrap();
        let plan = PurgePlan { victims: vec!["../outside".into(), "..".into(), "a/b".into(), String::new()], reclaimed: 0, shortfall: 0 };
        assert_eq!(execute(&plan, &root, "Library/Caches").unwrap(), 0);
        assert!(base.join("outside/Library/Caches").is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_cache_path_that_leaves_the_app_folder_is_refused() {
        let root = scratch("cachepath");
        std::fs::create_dir_all(root.join("app/Library")).unwrap();
        let plan = PurgePlan { victims: vec!["app".into()], reclaimed: 0, shortfall: 0 };
        assert_eq!(execute(&plan, &root, "../..").unwrap(), 0);
        assert_eq!(execute(&plan, &root, "/").unwrap(), 0);
        assert!(root.join("app/Library").is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_file_at_the_cache_path_is_left_alone() {
        let root = scratch("file");
        std::fs::create_dir_all(root.join("app/Library")).unwrap();
        std::fs::write(root.join("app/Library/Caches"), b"x").unwrap();
        let plan = PurgePlan { victims: vec!["app".into()], reclaimed: 1, shortfall: 0 };
        assert_eq!(execute(&plan, &root, "Library/Caches").unwrap(), 0);
        assert!(root.join("app/Library/Caches").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
