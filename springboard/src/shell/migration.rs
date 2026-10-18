//! Migration helper: brings a user's data from another disk, or from a backup archive, into this machine's data
//! folder. A source is either an archive made by `backup::create` or a data folder on another disk. Its contents are
//! listed as categories (one per shell data set, one per app's data folder), and only the categories the user picked
//! are staged. The staged copy goes through the backup restore (`backup::write_entries`), and it is applied at the
//! next start by `backup::apply_pending`, as a backup restore is, so the running shell never writes under itself.
//!
//! An archive is opened with the destination's archive key. An archive made on another machine needs that machine's
//! keychain, so it is only readable here when the keychain came along (a reinstall that keeps the data folder).

use super::backup::{self, PENDING, SOURCES};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// The folder under the data directory that holds each app's own data (`appdata/<app id>/...`).
pub const APP_DATA: &str = "appdata";

/// True for a file inside one app's data folder: `appdata/<app>/<path>` with safe path parts.
pub fn app_data_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(APP_DATA).and_then(|r| r.strip_prefix('/')) else { return false };
    !rest.is_empty()
        && !name.contains('\\')
        && !name.contains('\0')
        && rest.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}

/// True for a file a migration may bring: a shell data set or an app data file.
pub fn migratable(name: &str) -> bool {
    backup::name_ok(name) || app_data_name(name)
}

/// The category a file belongs to: its shell data set (`Notes`, `settings.ini`) or its app folder (`appdata/<app>`).
pub fn category_of(name: &str) -> String {
    let mut parts = name.split('/');
    match parts.next() {
        Some(APP_DATA) => match parts.next() {
            Some(app) => format!("{APP_DATA}/{app}"),
            None => APP_DATA.to_string(),
        },
        Some(first) => first.to_string(),
        None => String::new(),
    }
}

/// One category a source holds, with its file count and size, for the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub key: String,
    pub files: usize,
    pub bytes: u64,
}

/// The categories of `entries`, sorted by key. Entries a migration may not bring are left out.
pub fn categories(entries: &[(String, Vec<u8>)]) -> Vec<Category> {
    let mut map: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for (name, data) in entries.iter().filter(|(name, _)| migratable(name)) {
        let slot = map.entry(category_of(name)).or_insert((0, 0));
        slot.0 += 1;
        slot.1 += data.len() as u64;
    }
    map.into_iter().map(|(key, (files, bytes))| Category { key, files, bytes }).collect()
}

/// Reads and fully verifies a backup archive. `key` supplies the archive key for an encrypted archive.
pub fn read_archive(path: &Path, key: &dyn Fn() -> io::Result<[u8; 32]>) -> io::Result<Vec<(String, Vec<u8>)>> {
    backup::decode(&backup::open(&std::fs::read(path)?, key)?)
}

/// Reads a data folder on another disk: the shell's data sets and every app's data folder, leaving out what the
/// source's backup exclusions name.
pub fn read_folder(src: &Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    let excluded = backup::exclusions(src);
    let mut names = Vec::new();
    for root in SOURCES.iter().copied().chain([APP_DATA]) {
        backup::walk(src, root, &mut names, 0);
    }
    let mut out = Vec::new();
    for name in names {
        if migratable(&name) && !backup::excluded_under(&excluded, src, &name) {
            let data = std::fs::read(src.join(&name))?;
            out.push((name, data));
        }
    }
    Ok(out)
}

/// Stages the entries of the picked categories as the destination's pending restore, sealed under its key. The next
/// start brings them. Returns the number of files staged; nothing picked is an error.
pub fn stage(dest: &Path, entries: &[(String, Vec<u8>)], picked: &[&str], key: &dyn Fn() -> io::Result<[u8; 32]>) -> io::Result<usize> {
    let chosen: Vec<(String, Vec<u8>)> = entries
        .iter()
        .filter(|(name, _)| migratable(name) && picked.contains(&category_of(name).as_str()))
        .cloned()
        .collect();
    if chosen.is_empty() {
        return Err(io::Error::other("nothing picked to bring"));
    }
    let sealed = backup::seal(&key()?, &backup::encode(&chosen))?;
    super::prefs::write_atomic(&dest.join(PENDING), &sealed)?;
    Ok(chosen.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sb-migration-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn key() -> io::Result<[u8; 32]> {
        Ok([9u8; 32])
    }

    #[test]
    fn app_data_names_must_stay_inside_one_app_folder() {
        assert!(app_data_name("appdata/is.olibuijr.weather/summary.txt"));
        assert!(!app_data_name("appdata"));
        assert!(!app_data_name("appdata/"));
        assert!(!app_data_name("appdata/../secret"));
        assert!(!app_data_name("appdata//x"));
        assert!(!app_data_name("/appdata/x"));
        assert!(!app_data_name("appdata2/x"));
        assert!(!app_data_name("appdata/a\\b"));
    }

    #[test]
    fn categories_group_by_data_set_and_app_and_skip_unsafe_names() {
        assert_eq!(category_of("Notes/a/b.md"), "Notes");
        assert_eq!(category_of("settings.ini"), "settings.ini");
        assert_eq!(category_of("appdata/is.x/db"), "appdata/is.x");
        let entries = vec![
            ("Notes/a.md".to_string(), b"abc".to_vec()),
            ("Notes/b.md".to_string(), b"de".to_vec()),
            ("appdata/is.x/db".to_string(), b"12345".to_vec()),
            ("../evil".to_string(), b"x".to_vec()),
        ];
        assert_eq!(
            categories(&entries),
            vec![
                Category { key: "Notes".into(), files: 2, bytes: 5 },
                Category { key: "appdata/is.x".into(), files: 1, bytes: 5 },
            ]
        );
    }

    #[test]
    fn a_folder_source_brings_data_sets_and_app_folders_only() {
        let src = scratch("folder-src");
        std::fs::create_dir_all(src.join("Notes")).unwrap();
        std::fs::create_dir_all(src.join("appdata/is.x")).unwrap();
        std::fs::write(src.join("Notes/a.md"), b"hi").unwrap();
        std::fs::write(src.join("appdata/is.x/db"), b"rows").unwrap();
        std::fs::write(src.join("unrelated.txt"), b"no").unwrap();
        let mut names: Vec<String> = read_folder(&src).unwrap().into_iter().map(|(n, _)| n).collect();
        names.sort();
        assert_eq!(names, vec!["Notes/a.md".to_string(), "appdata/is.x/db".to_string()]);
        let _ = std::fs::remove_dir_all(&src);
    }

    #[test]
    fn only_the_picked_categories_are_staged_and_restored() {
        let src = scratch("pick-src");
        let dest = scratch("pick-dest");
        let entries = vec![
            ("Notes/a.md".to_string(), b"note".to_vec()),
            ("appdata/is.x/db".to_string(), b"rows".to_vec()),
        ];
        assert_eq!(stage(&dest, &entries, &["Notes"], &key).unwrap(), 1);
        // the next start restores through the backup path with the destination's key
        let restored = backup::restore_with(&dest, &dest.join(PENDING), &key).unwrap();
        assert_eq!(restored, 1);
        assert_eq!(std::fs::read(dest.join("Notes/a.md")).unwrap(), b"note");
        assert!(!dest.join("appdata/is.x/db").exists(), "the app folder was not picked");
        assert!(stage(&dest, &entries, &["Calendar"], &key).is_err(), "nothing picked to bring is an error");
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn an_archive_round_trips_into_entries() {
        let src = scratch("archive-src");
        let out = scratch("archive-out").join("backup.sbk");
        std::fs::create_dir_all(src.join("Notes")).unwrap();
        std::fs::write(src.join("Notes/a.md"), b"hello").unwrap();
        backup::create_with(&src, &out, &key).unwrap();
        let entries = read_archive(&out, &key).unwrap();
        assert_eq!(entries, vec![("Notes/a.md".to_string(), b"hello".to_vec())]);
        let _ = std::fs::remove_dir_all(&src);
        let _ = std::fs::remove_dir_all(out.parent().unwrap());
    }
}
