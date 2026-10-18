//! Settings > Search: which sources the Search panel queries, which apps' indexed items it shows, and the folders file
//! search leaves out (the "Privacy" list). Saved as `<data>/search-prefs.txt`, one setting per line:
//! `off <source>` (a source is one section name, such as `Files`), `app <id>` (an app whose indexed items are off) and
//! `exclude <folder>` (a folder whose files are never listed). Anything else is ignored. Nothing is excluded by default.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const FILE: &str = "search-prefs.txt";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPrefs {
    /// Section names the panel does not query.
    pub sources_off: BTreeSet<String>,
    /// Ids of apps whose indexed items are not shown.
    pub apps_off: BTreeSet<String>,
    /// Folders whose files file search leaves out (a file inside one of them is not listed).
    pub excluded_folders: Vec<PathBuf>,
}

impl SearchPrefs {
    pub fn source_on(&self, name: &str) -> bool {
        !self.sources_off.contains(name)
    }

    pub fn app_on(&self, id: &str) -> bool {
        !self.apps_off.contains(id)
    }

    /// True when `path` is one of the excluded folders or inside one (whole path components, so `/a/b` does not cover
    /// `/a/bc`).
    pub fn folder_excluded(&self, path: &Path) -> bool {
        self.excluded_folders.iter().any(|folder| path.starts_with(folder))
    }

    pub fn parse(text: &str) -> SearchPrefs {
        let mut prefs = SearchPrefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else { continue };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match key {
                "off" => {
                    prefs.sources_off.insert(value.to_string());
                }
                "app" => {
                    prefs.apps_off.insert(value.to_string());
                }
                "exclude" if !prefs.excluded_folders.iter().any(|f| f == Path::new(value)) => prefs.excluded_folders.push(PathBuf::from(value)),
                _ => {}
            }
        }
        prefs
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for s in &self.sources_off {
            out.push_str(&format!("off {s}\n"));
        }
        for a in &self.apps_off {
            out.push_str(&format!("app {a}\n"));
        }
        for f in &self.excluded_folders {
            out.push_str(&format!("exclude {}\n", f.display()));
        }
        out
    }
}

/// The saved settings in `dir`; a missing or unreadable file gives the defaults.
pub fn load(dir: &Path) -> SearchPrefs {
    std::fs::read_to_string(dir.join(FILE)).map(|t| SearchPrefs::parse(&t)).unwrap_or_default()
}

/// Saves the settings into `dir`, atomically (a temporary file renamed over the old one).
pub fn save(dir: &Path, prefs: &SearchPrefs) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, prefs.to_text())?;
    std::fs::rename(&tmp, dir.join(FILE))
}

static CURRENT: OnceLock<Mutex<SearchPrefs>> = OnceLock::new();

fn slot() -> &'static Mutex<SearchPrefs> {
    CURRENT.get_or_init(|| Mutex::new(load(&super::apphost::root())))
}

/// The settings in use: loaded once from the shell data dir, then kept in memory.
pub fn current() -> SearchPrefs {
    slot().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Changes the settings in use and saves them.
pub fn update(change: impl FnOnce(&mut SearchPrefs)) -> std::io::Result<()> {
    let mut g = slot().lock().unwrap_or_else(|e| e.into_inner());
    change(&mut g);
    save(&super::apphost::root(), &g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_excluded_by_default() {
        let p = SearchPrefs::default();
        assert!(p.source_on("Files") && p.app_on("is.olibuijr.notes.desktop"));
        assert!(!p.folder_excluded(Path::new("/home/me/Private/x.pdf")));
    }

    #[test]
    fn a_saved_text_reads_back_the_same() {
        let mut p = SearchPrefs::default();
        p.sources_off.insert("Files".into());
        p.apps_off.insert("is.olibuijr.mail.desktop".into());
        p.excluded_folders.push(PathBuf::from("/home/me/Private Notes"));
        assert_eq!(SearchPrefs::parse(&p.to_text()), p);
    }

    #[test]
    fn damaged_lines_are_ignored_and_duplicates_kept_once() {
        let p = SearchPrefs::parse("nonsense\noff \nexclude /a\nexclude /a\napp notes\n\nfoo bar");
        assert_eq!(p.excluded_folders, vec![PathBuf::from("/a")]);
        assert!(p.apps_off.contains("notes"));
        assert!(p.sources_off.is_empty());
    }

    #[test]
    fn folders_match_whole_components_only() {
        let p = SearchPrefs { excluded_folders: vec![PathBuf::from("/home/me/Private")], ..Default::default() };
        assert!(p.folder_excluded(Path::new("/home/me/Private")));
        assert!(p.folder_excluded(Path::new("/home/me/Private/tax/2025.pdf")));
        assert!(!p.folder_excluded(Path::new("/home/me/Privatestuff/x.pdf")));
    }

    #[test]
    fn saves_and_loads_from_a_directory() {
        let dir = std::env::temp_dir().join(format!("search-prefs-test-{}", std::process::id()));
        let mut p = SearchPrefs::default();
        p.sources_off.insert("Contacts".into());
        save(&dir, &p).unwrap();
        assert_eq!(load(&dir), p);
        assert!(!dir.join(format!("{FILE}.tmp")).exists());
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), SearchPrefs::default());
    }
}
