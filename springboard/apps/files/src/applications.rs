//! The Applications place: the store apps installed under `<shell data>/apps/<id>/current/`, listed by the names the
//! Dock and Launchpad show (the folder itself is named by app ids, which is not what a person looks for).
//!
//! The list is read-only, like Recents: nothing is created, pasted or dropped into it. Opening a row (double-click,
//! Return, File > Open) launches the app through the shell (`workspace::open_application`, by id), or brings it to the
//! front when it runs, instead of browsing into the package.
use crate::*;

/// Where the store apps are installed: the folder the sidebar's Applications row opens when the home folder and the
/// system have no Applications folder of their own.
pub(crate) fn store_dir() -> PathBuf {
    springboard::shell::apphost::root().join("apps")
}

/// Whether `path` is the store apps folder, which Files lists as the Applications place.
pub(crate) fn is_store_dir(path: &Path) -> bool {
    path == store_dir()
}

/// The installed apps in the store folder `dir`, one row per app named by its manifest, each row's path the app's own
/// folder (`<dir>/<id>`). System packages and broken installs are left out, as the Dock leaves them out.
pub(crate) fn installed_rows(dir: &Path) -> Vec<Entry> {
    let Some(root) = dir.parent() else { return vec![] };
    springboard::shell::apphost::scan(root)
        .into_iter()
        .map(|app| {
            let mtime = std::fs::metadata(&app.dir).and_then(|m| m.modified()).ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
            let name = app.manifest.name.clone();
            Entry { kind: fs::kind_of(&name, false), path: dir.join(&app.manifest.id), name, is_dir: false, size: 0, mtime, items: None }
        })
        .collect()
}

/// The app id of an Applications row: its path is `<store>/<id>` (`installed_rows`).
pub(crate) fn app_id_of(e: &Entry) -> Option<String> {
    e.path.file_name().and_then(|n| n.to_str()).filter(|id| !id.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(apps: &Path, id: &str, name: &str) {
        let dir = apps.join(id).join("current");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.toml"), format!("id = \"{id}\"\nname = \"{name}\"\nversion = \"1.0.0\"\nexec = \"run\"\n")).unwrap();
        std::fs::write(dir.join("run"), "#!/bin/sh\n").unwrap();
    }

    #[test]
    fn the_installed_apps_are_listed_by_name_not_by_id() {
        let root = std::env::temp_dir().join(format!("files-applications-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("apps");
        install(&apps, "is.olibuijr.calculator.desktop", "Calculator");
        install(&apps, "is.olibuijr.files.desktop", "Files");
        // a folder without a manifest is not an app
        std::fs::create_dir_all(apps.join("stray")).unwrap();
        let mut rows = installed_rows(&apps);
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        let names: Vec<&str> = rows.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Calculator", "Files"]);
        assert_eq!(rows[0].path, apps.join("is.olibuijr.calculator.desktop"));
        assert!(!rows[0].is_dir, "an app is one item, not a folder to browse into");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_store_folder_is_titled_applications_and_is_read_only() {
        let mut files = crate::tests::fixture(vec![]);
        files.path = store_dir();
        assert_eq!(files.folder_title(), "Applications");
        let items = files.path_items();
        assert_eq!(items.len(), 1, "one path bar item, not Computer > var > lib > springboard > apps");
        assert_eq!(items[0].0, "Applications");
        assert!(files.in_recents(), "nothing is created or pasted into the Applications list");
    }

    #[test]
    fn opening_an_app_row_launches_the_app_by_id() {
        let mut files = crate::tests::fixture(vec![]);
        files.path = store_dir();
        let id = "is.olibuijr.calculator.desktop";
        files.rows = vec![Entry { kind: fs::kind_of("Calculator", false), path: store_dir().join(id), name: "Calculator".into(), is_dir: false, size: 0, mtime: 0, items: None }];
        let path = files.path.clone();
        files.open(0);
        assert!(matches!(files.sys.effects.last(), Some(Effect::OpenApplication(i, n)) if i == id && n == "Calculator"), "the shell is asked to open the app");
        assert_eq!(files.path, path, "the package folder is not browsed into");
        assert!(files.notice.is_none(), "no 'open it from the Dock' notice any more");
    }

    #[test]
    fn an_empty_or_missing_store_lists_nothing() {
        let root = std::env::temp_dir().join(format!("files-applications-none-{}", std::process::id()));
        assert!(installed_rows(&root.join("apps")).is_empty());
        assert!(installed_rows(Path::new("/")).is_empty());
    }
}
