//! Fonts model: the families the desktop can draw, the user's turned-off list, collections and installs.
//! Pure logic (no window, no drawing) so it can be tested alone; `main.rs` draws it.
//!
//! Scope is per user: turning a family off, collections and installs live in the user's data dir and in
//! `~/.local/share/fonts`. The shell's own font library stays read-only here.

use bifrost_render::fontprefs::{self, Installed};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// File in the app's data dir holding the turned-off family names, one per line.
pub const DISABLED_FILE: &str = "disabled.txt";
/// File in the app's data dir holding collections, one per line: `name<TAB>family<TAB>family...`.
pub const COLLECTIONS_FILE: &str = "collections.tsv";
/// Where installs go, relative to the home directory.
pub const USER_FONTS_DIR: &str = ".local/share/fonts";
/// The system-wide font directory, scanned read-only.
pub const SYSTEM_FONTS_DIR: &str = "/usr/share/fonts";
/// The sample line shown in the preview when nothing else is typed.
pub const PREVIEW_TEXT: &str = "The quick brown fox jumps over the lazy dog";

/// The folders a catalogue is read from: the system fonts, the user's installs, and the shell's library.
pub fn scan_dirs(home: &Path, library: &Path) -> Vec<PathBuf> {
    vec![PathBuf::from(SYSTEM_FONTS_DIR), home.join(USER_FONTS_DIR), library.to_path_buf()]
}

/// Which list the sidebar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    All,
    Disabled,
    Collection(String),
}

/// Everything the window shows: the families found on disk plus the user's settings about them.
#[derive(Debug, Default)]
pub struct Library {
    pub families: Vec<Installed>,
    /// lower-case family names the user turned off
    disabled: BTreeSet<String>,
    /// collection name -> member family names, as the user spelled them
    collections: BTreeMap<String, Vec<String>>,
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// A name safe for one line of a text file.
fn clean(name: &str) -> String {
    name.replace(['\t', '\n', '\r'], " ").trim().to_string()
}

impl Library {
    /// A library over the families `families` (sorted by family name, as `fontprefs::installed` returns them).
    pub fn new(families: Vec<Installed>, disabled: BTreeSet<String>, collections: BTreeMap<String, Vec<String>>) -> Self {
        Library { families, disabled, collections }
    }

    pub fn is_enabled(&self, family: &str) -> bool {
        !self.disabled.contains(&key(family))
    }

    /// Turns a family on or off. Returns false when nothing changed.
    pub fn set_enabled(&mut self, family: &str, on: bool) -> bool {
        if on {
            self.disabled.remove(&key(family))
        } else {
            self.disabled.insert(key(family))
        }
    }

    pub fn disabled_names(&self) -> Vec<String> {
        self.families.iter().filter(|f| !self.is_enabled(&f.family)).map(|f| f.family.clone()).collect()
    }

    pub fn collection_names(&self) -> Vec<String> {
        self.collections.keys().cloned().collect()
    }

    pub fn members(&self, collection: &str) -> Option<&[String]> {
        self.collections.get(collection).map(Vec::as_slice)
    }

    /// Adds a collection. Returns false when the name is empty or already taken (names ignore case).
    pub fn create_collection(&mut self, name: &str) -> bool {
        let name = clean(name);
        if name.is_empty() || self.collections.keys().any(|k| key(k) == key(&name)) {
            return false;
        }
        self.collections.insert(name, Vec::new());
        true
    }

    pub fn delete_collection(&mut self, name: &str) -> bool {
        let found = self.collections.keys().find(|k| key(k) == key(name)).cloned();
        found.is_some_and(|k| self.collections.remove(&k).is_some())
    }

    /// Adds `family` to `collection`. Returns false when it is already a member or the collection is missing.
    pub fn add_to_collection(&mut self, collection: &str, family: &str) -> bool {
        let Some(list) = self.collections.iter_mut().find(|(k, _)| key(k) == key(collection)).map(|(_, v)| v) else {
            return false;
        };
        if list.iter().any(|m| key(m) == key(family)) {
            return false;
        }
        list.push(clean(family));
        list.sort_by_key(|m| key(m));
        true
    }

    pub fn remove_from_collection(&mut self, collection: &str, family: &str) -> bool {
        let Some(list) = self.collections.iter_mut().find(|(k, _)| key(k) == key(collection)).map(|(_, v)| v) else {
            return false;
        };
        let before = list.len();
        list.retain(|m| key(m) != key(family));
        list.len() != before
    }

    /// The families the sidebar `source` lists, narrowed by `query`.
    pub fn visible(&self, source: &Source, query: &str) -> Vec<&Installed> {
        let members: Option<BTreeSet<String>> = match source {
            Source::Collection(name) => Some(self.members(name).unwrap_or(&[]).iter().map(|m| key(m)).collect()),
            _ => None,
        };
        self.families
            .iter()
            .filter(|f| match source {
                Source::All => true,
                Source::Disabled => !self.is_enabled(&f.family),
                Source::Collection(_) => members.as_ref().is_some_and(|m| m.contains(&key(&f.family))),
            })
            .filter(|f| matches_query(f, query))
            .collect()
    }

    /// The family called `name`, ignoring case.
    pub fn find(&self, name: &str) -> Option<&Installed> {
        self.families.iter().find(|f| key(&f.family) == key(name))
    }

    /// Rows for the Info panel: label and value.
    pub fn info(&self, family: &str) -> Vec<(String, String)> {
        let Some(f) = self.find(family) else { return Vec::new() };
        vec![
            ("Family".to_string(), f.family.clone()),
            ("File".to_string(), Path::new(&f.path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            ("Location".to_string(), f.path.clone()),
            ("Pitch".to_string(), if f.mono { "Fixed".to_string() } else { "Proportional".to_string() }),
            ("Axes".to_string(), if f.variable { "Variable (every weight)".to_string() } else { "Static".to_string() }),
            ("Status".to_string(), if self.is_enabled(&f.family) { "On".to_string() } else { "Off".to_string() }),
        ]
    }

    /// The text to write to the disabled file.
    pub fn disabled_file(&self) -> String {
        self.disabled.iter().map(|k| format!("{k}\n")).collect()
    }

    /// The text to write to the collections file.
    pub fn collections_file(&self) -> String {
        self.collections.iter().map(|(name, members)| std::iter::once(name.clone()).chain(members.iter().cloned()).collect::<Vec<_>>().join("\t") + "\n").collect()
    }
}

/// True when the family's name or file name contains `query`, ignoring case. An empty query matches all.
pub fn matches_query(f: &Installed, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let file = Path::new(&f.path).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    f.family.to_lowercase().contains(&q) || file.contains(&q)
}

/// Parses the disabled file. Blank lines are skipped and names are stored lower-case.
pub fn parse_disabled(text: &str) -> BTreeSet<String> {
    text.lines().map(key).filter(|k| !k.is_empty()).collect()
}

/// Parses the collections file: the first field of each line is the name, the rest are members.
pub fn parse_collections(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let mut fields = line.split('\t').map(str::trim);
        let Some(name) = fields.next().filter(|n| !n.is_empty()) else { continue };
        out.entry(name.to_string()).or_insert_with(Vec::new).extend(fields.filter(|f| !f.is_empty()).map(str::to_string));
    }
    out
}

/// Reads a file under the app's data dir; a missing file is empty.
pub fn read_text(dir: &Path, file: &str) -> String {
    std::fs::read_to_string(dir.join(file)).unwrap_or_default()
}

/// Writes a file under the app's data dir, creating the dir first.
pub fn write_text(dir: &Path, file: &str, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(file), text)
}

/// A file name that is safe to create under the install dir: letters, digits, `-`, `_` and `.` only.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '-' }).collect();
    let cleaned = cleaned.trim_start_matches(['.', '-']).to_string();
    if cleaned.is_empty() {
        "Font.ttf".to_string()
    } else {
        cleaned
    }
}

/// Installs the font file `src` into `user_dir` (created if needed). Only TrueType and OpenType files the
/// renderer can draw are accepted. Returns the new path and the family name. An existing file is not overwritten.
pub fn install(src: &Path, user_dir: &Path) -> Result<(PathBuf, String), String> {
    let ext = src.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext != "ttf" && ext != "otf" {
        return Err("Only .ttf and .otf fonts can be installed.".to_string());
    }
    let src_text = src.to_string_lossy();
    let family = fontprefs::family(&src_text).ok_or_else(|| "That file is not a font this desktop can draw.".to_string())?;
    let name = safe_file_name(&src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    std::fs::create_dir_all(user_dir).map_err(|e| format!("Cannot create {}: {e}", user_dir.display()))?;
    let dest = user_dir.join(name);
    if dest.exists() {
        return Err(format!("{} is already installed.", family));
    }
    std::fs::copy(src, &dest).map_err(|e| format!("Cannot copy the font: {e}"))?;
    Ok((dest, family))
}

/// Removes a font the user installed. Only files inside `user_dir` can be removed; system and library fonts stay.
pub fn uninstall(path: &Path, user_dir: &Path) -> Result<(), String> {
    if !path.starts_with(user_dir) || path == user_dir {
        return Err("Only fonts installed for this user can be removed.".to_string());
    }
    std::fs::remove_file(path).map_err(|e| format!("Cannot remove the font: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fam(name: &str, path: &str, mono: bool, variable: bool) -> Installed {
        Installed { family: name.to_string(), path: path.to_string(), mono, variable }
    }

    fn sample() -> Library {
        let families = vec![fam("Code", "/usr/share/fonts/Code-Regular.otf", true, false), fam("Inter", "/x/InterVariable.ttf", false, true)];
        let mut disabled = BTreeSet::new();
        disabled.insert("code".to_string());
        let mut cols = BTreeMap::new();
        cols.insert("Work".to_string(), vec!["Inter".to_string()]);
        Library::new(families, disabled, cols)
    }

    #[test]
    fn enable_state_ignores_case_and_reports_changes() {
        let mut lib = sample();
        assert!(!lib.is_enabled("CODE"));
        assert!(!lib.set_enabled("Code", false), "already off");
        assert!(lib.set_enabled("Code", true));
        assert!(lib.is_enabled("code"));
        assert_eq!(lib.disabled_names(), Vec::<String>::new());
    }

    #[test]
    fn sources_filter_by_list_and_query() {
        let lib = sample();
        let names = |v: Vec<&Installed>| v.iter().map(|f| f.family.clone()).collect::<Vec<_>>();
        assert_eq!(names(lib.visible(&Source::All, "")), ["Code", "Inter"]);
        assert_eq!(names(lib.visible(&Source::Disabled, "")), ["Code"]);
        assert_eq!(names(lib.visible(&Source::Collection("Work".into()), "")), ["Inter"]);
        assert_eq!(names(lib.visible(&Source::Collection("Nope".into()), "")), Vec::<String>::new());
        assert_eq!(names(lib.visible(&Source::All, "zzz")), Vec::<String>::new());
        assert_eq!(names(lib.visible(&Source::All, "inter")), ["Inter"]);
        assert_eq!(names(lib.visible(&Source::All, "otf")), ["Code"], "search also matches the file name");
    }

    #[test]
    fn collections_add_remove_and_reject_duplicates() {
        let mut lib = sample();
        assert!(!lib.create_collection("  "));
        assert!(!lib.create_collection("work"), "names ignore case");
        assert!(lib.create_collection("Design"));
        assert!(lib.add_to_collection("design", "Code"));
        assert!(!lib.add_to_collection("design", "CODE"), "already a member");
        assert!(!lib.add_to_collection("missing", "Code"));
        assert_eq!(lib.members("Design").unwrap(), ["Code"]);
        assert!(lib.remove_from_collection("Design", "code"));
        assert!(!lib.remove_from_collection("Design", "code"));
        assert!(lib.delete_collection("WORK"));
        assert_eq!(lib.collection_names(), ["Design"]);
    }

    #[test]
    fn prefs_round_trip_through_text() {
        let lib = sample();
        let disabled = parse_disabled(&lib.disabled_file());
        assert_eq!(disabled, parse_disabled("\ncode\n\n"));
        let cols = parse_collections(&lib.collections_file());
        assert_eq!(cols.get("Work").unwrap(), &vec!["Inter".to_string()]);
        assert_eq!(parse_collections("A\tOne\tTwo\n\n\tbad\n"), BTreeMap::from([("A".to_string(), vec!["One".to_string(), "Two".to_string()])]));
    }

    #[test]
    fn info_rows_describe_the_family() {
        let lib = sample();
        let rows = lib.info("code");
        assert!(rows.contains(&("File".to_string(), "Code-Regular.otf".to_string())));
        assert!(rows.contains(&("Pitch".to_string(), "Fixed".to_string())));
        assert!(rows.contains(&("Status".to_string(), "Off".to_string())));
        assert!(lib.info("Missing").is_empty());
    }

    #[test]
    fn safe_names_strip_path_parts_and_odd_characters() {
        assert_eq!(safe_file_name("My Font (Bold).ttf"), "My-Font--Bold-.ttf");
        assert_eq!(safe_file_name("../../etc/x.otf"), "etc-x.otf");
        assert_eq!(safe_file_name("..."), "Font.ttf");
    }

    #[test]
    fn install_rejects_non_fonts_before_touching_disk() {
        let dir = std::env::temp_dir().join("fontbook-model-reject");
        assert!(install(Path::new("/tmp/nope.txt"), &dir).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn uninstall_only_touches_the_user_dir() {
        let user = Path::new("/home/u/.local/share/fonts");
        assert!(uninstall(Path::new("/usr/share/fonts/x.ttf"), user).is_err());
        assert!(uninstall(user, user).is_err());
    }
}
