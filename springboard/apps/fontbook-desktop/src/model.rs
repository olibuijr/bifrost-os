//! Fonts model: the families the desktop can draw, the user's turned-off list, collections and installs.
//! Pure logic (no window, no drawing) so it can be tested alone; `main.rs` draws it.
//!
//! Scope is per user: turning a family off, collections and installs live in the user's data dir and in
//! `~/.local/share/fonts`. The shell's own font library stays read-only here.

use crate::validate;
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
    /// a rule-based collection
    Smart(String),
}

/// File in the app's data dir holding smart collections, one per line: `name<TAB>rule`.
pub const SMART_FILE: &str = "smart.tsv";
/// File in the app's data dir holding the settings and preview choices, one `key<TAB>value` per line.
pub const PREFS_FILE: &str = "settings.tsv";

/// The rule of a smart collection: a family belongs to it while the rule holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// fixed pitch families
    Fixed,
    /// families with a variation axis
    Variable,
    /// families without a variation axis
    Static,
    /// the file is `.ttf`
    TrueType,
    /// the file is `.otf`
    OpenType,
    /// the family or file name contains the text (ignoring case)
    NameContains(String),
}

impl Rule {
    /// The rule on one line of the smart file: `fixed`, `variable`, `static`, `ttf`, `otf` or `name:<text>`.
    pub fn to_text(&self) -> String {
        match self {
            Rule::Fixed => "fixed".to_string(),
            Rule::Variable => "variable".to_string(),
            Rule::Static => "static".to_string(),
            Rule::TrueType => "ttf".to_string(),
            Rule::OpenType => "otf".to_string(),
            Rule::NameContains(t) => format!("name:{}", clean(t)),
        }
    }

    pub fn parse(text: &str) -> Option<Rule> {
        let text = text.trim();
        match text {
            "fixed" => Some(Rule::Fixed),
            "variable" => Some(Rule::Variable),
            "static" => Some(Rule::Static),
            "ttf" => Some(Rule::TrueType),
            "otf" => Some(Rule::OpenType),
            _ => text.strip_prefix("name:").map(clean).filter(|t| !t.is_empty()).map(Rule::NameContains),
        }
    }

    /// A short title for the menu and for naming a new smart collection.
    pub fn title(&self) -> String {
        match self {
            Rule::Fixed => "Fixed Width".to_string(),
            Rule::Variable => "Variable".to_string(),
            Rule::Static => "Static".to_string(),
            Rule::TrueType => "TrueType".to_string(),
            Rule::OpenType => "OpenType".to_string(),
            Rule::NameContains(t) => format!("Name Contains {t}"),
        }
    }

    pub fn matches(&self, f: &Installed) -> bool {
        match self {
            Rule::Fixed => f.mono,
            Rule::Variable => f.variable,
            Rule::Static => !f.variable,
            Rule::TrueType => f.path.to_lowercase().ends_with(".ttf"),
            Rule::OpenType => f.path.to_lowercase().ends_with(".otf"),
            Rule::NameContains(t) => matches_query(f, t),
        }
    }
}

/// The settings and preview choices (Settings, View).
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    /// check a file before it can be installed (the Install button is offered only for a file that passes)
    pub validate_before_install: bool,
    /// the sample line; empty means `PREVIEW_TEXT`
    pub sample_text: String,
    /// the sample size in points
    pub sample_size: f32,
    /// the font library the list shows (`LIBRARY_COMPUTER`, `LIBRARY_USER` or a name from the libraries file)
    pub library: String,
}

pub const SIZE_MIN: f32 = 8.0;
pub const SIZE_MAX: f32 = 96.0;
pub const SIZE_DEFAULT: f32 = 32.0;

impl Default for Prefs {
    fn default() -> Self {
        Prefs { validate_before_install: true, sample_text: String::new(), sample_size: SIZE_DEFAULT, library: LIBRARY_COMPUTER.to_string() }
    }
}

impl Prefs {
    /// The text the sample draws: the custom line, or the default pangram.
    pub fn sample(&self) -> &str {
        if self.sample_text.trim().is_empty() {
            PREVIEW_TEXT
        } else {
            &self.sample_text
        }
    }

    /// Sets the size, kept between `SIZE_MIN` and `SIZE_MAX`.
    pub fn set_size(&mut self, size: f32) {
        self.sample_size = size.clamp(SIZE_MIN, SIZE_MAX);
    }

    pub fn to_text(&self) -> String {
        format!("validate\t{}\nsample\t{}\nsize\t{}\nlibrary\t{}\n", self.validate_before_install, clean(&self.sample_text), self.sample_size, clean(&self.library))
    }

    pub fn parse(text: &str) -> Prefs {
        let mut p = Prefs::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('\t') else { continue };
            match k {
                "validate" => p.validate_before_install = v.trim() != "false",
                "sample" => p.sample_text = v.trim().to_string(),
                "size" => {
                    if let Ok(n) = v.trim().parse::<f32>() {
                        p.set_size(n);
                    }
                }
                "library" if !v.trim().is_empty() => p.library = v.trim().to_string(),
                _ => {}
            }
        }
        p
    }
}

/// A font file found on disk, for duplicate detection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    pub family: String,
    pub path: PathBuf,
    /// modification time, seconds since the epoch
    pub modified: u64,
}

/// A family with several files: the newest is kept, the others can go (Resolve Duplicates).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub family: String,
    pub keep: PathBuf,
    pub remove: Vec<PathBuf>,
}

/// Families that have more than one file among `files`. The newest file wins; ties go to the later path.
pub fn find_duplicates(files: &[FontFile]) -> Vec<Duplicate> {
    let mut groups: BTreeMap<String, Vec<&FontFile>> = BTreeMap::new();
    for f in files {
        groups.entry(key(&f.family)).or_default().push(f);
    }
    let mut out = Vec::new();
    for (_, mut group) in groups {
        if group.len() < 2 {
            continue;
        }
        group.sort_by(|a, b| b.modified.cmp(&a.modified).then(b.path.cmp(&a.path)));
        out.push(Duplicate { family: group[0].family.clone(), keep: group[0].path.clone(), remove: group[1..].iter().map(|f| f.path.clone()).collect() });
    }
    out
}

/// The `.ttf` and `.otf` files under `dir` (two levels deep) with their family and modification time.
pub fn scan_files(dir: &Path) -> Vec<FontFile> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<FontFile>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                if depth < 2 {
                    walk(&path, depth + 1, out);
                }
                continue;
            }
            let ext = path.extension().and_then(|x| x.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            if ext != "ttf" && ext != "otf" {
                continue;
            }
            let Some(family) = fontprefs::family(&path.to_string_lossy()) else { continue };
            let modified = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
            out.push(FontFile { family, path, modified });
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out
}

/// Removes the older files of each duplicate, only inside `user_dir`. Returns how many files went.
pub fn resolve_duplicates(dups: &[Duplicate], user_dir: &Path) -> usize {
    dups.iter().flat_map(|d| d.remove.iter()).filter(|p| uninstall(p, user_dir).is_ok()).count()
}

/// Parses the smart file: `name<TAB>rule` per line; lines with an unknown rule are skipped.
pub fn parse_smart(text: &str) -> BTreeMap<String, Rule> {
    text.lines()
        .filter_map(|line| {
            let (name, rule) = line.split_once('\t')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some((name.to_string(), Rule::parse(rule)?))
        })
        .collect()
}

/// Everything the window shows: the families found on disk plus the user's settings about them.
#[derive(Debug, Default)]
pub struct Library {
    pub families: Vec<Installed>,
    /// lower-case family names the user turned off
    disabled: BTreeSet<String>,
    /// collection name -> member family names, as the user spelled them
    collections: BTreeMap<String, Vec<String>>,
    /// smart collection name -> its rule
    smart: BTreeMap<String, Rule>,
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
        Library { families, disabled, collections, smart: BTreeMap::new() }
    }

    /// The same library with smart collections `smart` (from `parse_smart`).
    pub fn with_smart(mut self, smart: BTreeMap<String, Rule>) -> Self {
        self.smart = smart;
        self
    }

    pub fn smart_names(&self) -> Vec<String> {
        self.smart.keys().cloned().collect()
    }

    /// Adds a smart collection. False when the name is empty or already used by any collection.
    pub fn create_smart(&mut self, name: &str, rule: Rule) -> bool {
        let name = clean(name);
        if name.is_empty() || self.collections.keys().chain(self.smart.keys()).any(|k| key(k) == key(&name)) {
            return false;
        }
        self.smart.insert(name, rule);
        true
    }

    pub fn delete_smart(&mut self, name: &str) -> bool {
        let found = self.smart.keys().find(|k| key(k) == key(name)).cloned();
        found.is_some_and(|k| self.smart.remove(&k).is_some())
    }

    /// Renames a collection, plain or smart. False when `new` is empty, taken by another collection, or `old` is missing.
    pub fn rename_collection(&mut self, old: &str, new: &str) -> bool {
        let new = clean(new);
        if new.is_empty() {
            return false;
        }
        let taken = self.collections.keys().chain(self.smart.keys()).any(|k| key(k) == key(&new) && key(k) != key(old));
        if taken {
            return false;
        }
        if let Some(k) = self.collections.keys().find(|k| key(k) == key(old)).cloned() {
            let members = self.collections.remove(&k).unwrap_or_default();
            self.collections.insert(new, members);
            return true;
        }
        if let Some(k) = self.smart.keys().find(|k| key(k) == key(old)).cloned() {
            if let Some(rule) = self.smart.remove(&k) {
                self.smart.insert(new, rule);
                return true;
            }
        }
        false
    }

    /// The text to write to the smart file.
    pub fn smart_file(&self) -> String {
        self.smart.iter().map(|(n, r)| format!("{n}\t{}\n", r.to_text())).collect()
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

    #[cfg(test)]
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
                Source::Smart(name) => self.smart.get(name).is_some_and(|r| r.matches(f)),
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

/// The URL scheme this app opens.
pub const URL_SCHEME: &str = "fontbook";

/// The family named by a `fontbook:` URL (`fontbook://Inter`, `fontbook:Fira%20Code`, `fontbook:///Noto+Sans`), or
/// None when `url` is not one of ours or names nothing.
pub fn family_from_url(url: &str) -> Option<String> {
    let rest = url.trim().strip_prefix(URL_SCHEME)?.strip_prefix(':')?;
    let rest = rest.trim_start_matches('/');
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let mut bytes = Vec::new();
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            b'%' if raw.get(i + 1..i + 3).is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit)) => {
                let hex = std::str::from_utf8(&raw[i + 1..i + 3]).unwrap_or("00");
                bytes.push(u8::from_str_radix(hex, 16).unwrap_or(b'?'));
                i += 3;
            }
            b'+' => {
                bytes.push(b' ');
                i += 1;
            }
            b => {
                bytes.push(b);
                i += 1;
            }
        }
    }
    let name = String::from_utf8_lossy(&bytes).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// `base`, or `base 2`, `base 3`... the first that no name in `taken` uses (ignoring case).
pub fn unique_name(taken: &[String], base: &str) -> String {
    (1..)
        .map(|n: u32| if n == 1 { base.to_string() } else { format!("{base} {n}") })
        .find(|c| !taken.iter().any(|k| key(k) == key(c)))
        .unwrap_or_default()
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

/// Rows for the Info panel from a font's check: characters, glyphs, the Unicode blocks covered and the validation.
pub fn repertoire_rows(r: &validate::Report) -> Vec<(String, String)> {
    let coverage = if r.blocks.is_empty() { "None".to_string() } else { r.blocks.join(", ") };
    let validation = match r.verdict() {
        validate::Verdict::Pass => "Valid".to_string(),
        validate::Verdict::Warn => format!("Valid, {} warning{}", r.warnings.len(), if r.warnings.len() == 1 { "" } else { "s" }),
        validate::Verdict::Fail => r.problems.join(" "),
    };
    vec![
        ("Characters".to_string(), r.characters.to_string()),
        ("Glyphs".to_string(), r.glyphs.to_string()),
        ("Coverage".to_string(), coverage),
        ("Validation".to_string(), validation),
    ]
}

/// Installs the font file `src` into `user_dir` (created if needed). Only TrueType and OpenType files the
/// renderer can draw are accepted, and a file that fails validation is refused with its first problem. Returns the
/// new path and the family name. An existing file is not overwritten.
pub fn install(src: &Path, user_dir: &Path, validate: bool) -> Result<(PathBuf, String), String> {
    let ext = src.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext != "ttf" && ext != "otf" {
        return Err("Only .ttf and .otf fonts can be installed.".to_string());
    }
    if validate {
        if let Some(problem) = validate::check_file(src).problems.first() {
            return Err(problem.clone());
        }
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

/// Writes `bytes` under `dir` as a staging copy (for the install preview of a file the app only holds as a stream) and
/// returns its path.
pub fn stage(bytes: &[u8], file_name: &str, dir: &Path) -> std::io::Result<PathBuf> {
    let dir = dir.join("staging");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(safe_file_name(file_name));
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Removes a font the user installed. Only files inside `user_dir` can be removed; system and library fonts stay.
pub fn uninstall(path: &Path, user_dir: &Path) -> Result<(), String> {
    if !path.starts_with(user_dir) || path == user_dir {
        return Err("Only fonts installed for this user can be removed.".to_string());
    }
    std::fs::remove_file(path).map_err(|e| format!("Cannot remove the font: {e}"))
}

/// File in the app's data dir holding the user's libraries, one per line: `name<TAB>folder<TAB>folder...`.
pub const LIBRARIES_FILE: &str = "libraries.tsv";
/// The library of every font the desktop can see: the system fonts, the user's installs and the shell's library.
pub const LIBRARY_COMPUTER: &str = "Computer";
/// The library of the fonts the user installed.
pub const LIBRARY_USER: &str = "User";

/// A named set of font folders: a library the list can be switched to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontSet {
    pub name: String,
    pub dirs: Vec<PathBuf>,
}

fn is_builtin_library(name: &str) -> bool {
    key(name) == key(LIBRARY_COMPUTER) || key(name) == key(LIBRARY_USER)
}

/// Parses the libraries file. Lines without a name or a folder, and names that clash with a built-in library, are
/// skipped; a repeated name merges its folders into the first.
pub fn parse_font_sets(text: &str) -> Vec<FontSet> {
    let mut out: Vec<FontSet> = Vec::new();
    for line in text.lines() {
        let mut fields = line.split('\t').map(str::trim);
        let Some(name) = fields.next().filter(|n| !n.is_empty() && !is_builtin_library(n)) else { continue };
        let dirs: Vec<PathBuf> = fields.filter(|d| !d.is_empty()).map(PathBuf::from).collect();
        if dirs.is_empty() {
            continue;
        }
        match out.iter_mut().find(|s| key(&s.name) == key(name)) {
            Some(set) => {
                for d in dirs {
                    if !set.dirs.contains(&d) {
                        set.dirs.push(d);
                    }
                }
            }
            None => out.push(FontSet { name: name.to_string(), dirs }),
        }
    }
    out
}

/// The text to write to the libraries file.
pub fn font_sets_file(sets: &[FontSet]) -> String {
    sets.iter()
        .map(|s| std::iter::once(clean(&s.name)).chain(s.dirs.iter().map(|d| clean(&d.to_string_lossy()))).collect::<Vec<_>>().join("\t") + "\n")
        .collect()
}

/// Every library name in menu order: Computer, User, then the user's own.
pub fn library_names(sets: &[FontSet]) -> Vec<String> {
    [LIBRARY_COMPUTER, LIBRARY_USER].iter().map(|s| s.to_string()).chain(sets.iter().map(|s| s.name.clone())).collect()
}

/// True when `name` is one of the user's own libraries (the only ones that can be removed).
pub fn is_custom_library(sets: &[FontSet], name: &str) -> bool {
    sets.iter().any(|s| key(&s.name) == key(name))
}

/// The folders the library `active` is read from. An unknown name reads as Computer.
pub fn library_dirs(active: &str, sets: &[FontSet], home: &Path, shell_library: &Path) -> Vec<PathBuf> {
    if key(active) == key(LIBRARY_USER) {
        return vec![home.join(USER_FONTS_DIR)];
    }
    match sets.iter().find(|s| key(&s.name) == key(active)) {
        Some(set) => set.dirs.clone(),
        None => scan_dirs(home, shell_library),
    }
}

/// The name a library made from `dir` gets: the folder's own name.
pub fn folder_label(dir: &Path) -> String {
    dir.file_name().map(|n| clean(&n.to_string_lossy())).filter(|n| !n.is_empty()).unwrap_or_else(|| "Fonts".to_string())
}

/// Adds the folder `dir` as a library named after it. A folder with the name of an existing library of the user's joins
/// it; a name that clashes with a built-in library gets a number. Returns the library's name.
pub fn add_font_set(sets: &mut Vec<FontSet>, dir: &Path) -> Result<String, String> {
    if !dir.is_absolute() {
        return Err("That folder has no path this app can read.".to_string());
    }
    let label = folder_label(dir);
    if let Some(set) = sets.iter_mut().find(|s| key(&s.name) == key(&label)) {
        if !set.dirs.iter().any(|d| d == dir) {
            set.dirs.push(dir.to_path_buf());
        }
        return Ok(set.name.clone());
    }
    let name = unique_name(&library_names(sets), &label);
    sets.push(FontSet { name: name.clone(), dirs: vec![dir.to_path_buf()] });
    Ok(name)
}

/// Removes the user's library `name` (the fonts in its folders stay). False for a built-in or unknown name.
pub fn remove_font_set(sets: &mut Vec<FontSet>, name: &str) -> bool {
    let before = sets.len();
    sets.retain(|s| key(&s.name) != key(name));
    sets.len() != before
}

/// The folder a `/proc/self/fd` link points at, or None when it is not an absolute path or the folder was deleted.
pub fn dir_from_link(target: &Path) -> Option<PathBuf> {
    let text = target.to_string_lossy();
    if !target.is_absolute() || text.ends_with(" (deleted)") {
        return None;
    }
    Some(target.to_path_buf())
}

/// True when the font file at `path` lies inside the user's home, the only place the shell can reveal a file from.
pub fn can_reveal(path: &Path, home: &Path) -> bool {
    !home.as_os_str().is_empty() && path.starts_with(home) && path != home
}

/// The file name the save panel suggests when exporting the font at `path`.
pub fn export_name(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Font.ttf".to_string())
}

/// Plain text for a printed font sample sheet: the family, its info rows, the sample line and the character set. The
/// print service sets plain text in its own typeface, so the sheet describes the font rather than drawing it.
pub fn sample_sheet(family: &str, info: &[(String, String)], sample: &str) -> String {
    let mut out = format!("{family}\n\n");
    for (k, v) in info.iter().filter(|(k, _)| k != "Family") {
        out.push_str(&format!("{k}: {v}\n"));
    }
    out.push_str(&format!("\nSample\n{sample}\n\nCharacters\nABCDEFGHIJKLMNOPQRSTUVWXYZ\nabcdefghijklmnopqrstuvwxyz\n0123456789\n"));
    out
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
        assert!(install(Path::new("/tmp/nope.txt"), &dir, true).is_err());
        assert!(!dir.exists());
    }

    #[test]
    fn install_without_validation_still_needs_a_drawable_font() {
        let dir = std::env::temp_dir().join(format!("fontbook-model-novalidate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("Broken.ttf");
        std::fs::write(&src, b"not a font at all").unwrap();
        let err = install(&src, &dir.join("user"), false).unwrap_err();
        assert_eq!(err, "That file is not a font this desktop can draw.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_writes_a_safe_copy_under_staging() {
        let dir = std::env::temp_dir().join(format!("fontbook-model-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = stage(b"abc", "../My Font.ttf", &dir).unwrap();
        assert!(p.starts_with(dir.join("staging")));
        assert_eq!(std::fs::read(&p).unwrap(), b"abc");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rename_collection_keeps_members_and_rejects_clashes() {
        let mut lib = sample();
        assert!(lib.create_collection("Play"));
        assert!(!lib.rename_collection("Work", "play"), "taken by another");
        assert!(!lib.rename_collection("Work", "  "));
        assert!(!lib.rename_collection("Nope", "X"));
        assert!(lib.rename_collection("Work", "Office"));
        assert_eq!(lib.members("Office").unwrap(), ["Inter"]);
        assert!(lib.members("Work").is_none());
        assert!(lib.rename_collection("Office", "OFFICE"), "case-only change is allowed");
        assert_eq!(lib.collection_names(), ["OFFICE", "Play"]);
    }

    #[test]
    fn smart_collections_follow_their_rule() {
        let mut lib = sample();
        assert!(lib.create_smart("Code fonts", Rule::Fixed));
        assert!(!lib.create_smart("work", Rule::Static), "name used by a collection");
        assert!(!lib.create_smart("", Rule::Static));
        let names = |v: Vec<&Installed>| v.iter().map(|f| f.family.clone()).collect::<Vec<_>>();
        assert_eq!(names(lib.visible(&Source::Smart("Code fonts".into()), "")), ["Code"]);
        assert!(lib.create_smart("Var", Rule::Variable));
        assert_eq!(names(lib.visible(&Source::Smart("Var".into()), "")), ["Inter"]);
        assert!(lib.create_smart("OTF", Rule::OpenType));
        assert_eq!(names(lib.visible(&Source::Smart("OTF".into()), "")), ["Code"]);
        assert!(lib.create_smart("Named", Rule::NameContains("int".into())));
        assert_eq!(names(lib.visible(&Source::Smart("Named".into()), "")), ["Inter"]);
        assert!(lib.rename_collection("Named", "Interish"));
        assert_eq!(lib.smart_names(), ["Interish"]);
        assert!(lib.delete_smart("interish"));
        assert!(!lib.delete_smart("interish"));
        assert_eq!(names(lib.visible(&Source::Smart("Missing".into()), "")), Vec::<String>::new());
    }

    #[test]
    fn smart_rules_round_trip_through_text() {
        let mut lib = sample();
        lib.create_smart("A", Rule::Fixed);
        lib.create_smart("B", Rule::NameContains("sans".into()));
        let parsed = parse_smart(&lib.smart_file());
        assert_eq!(parsed.get("A"), Some(&Rule::Fixed));
        assert_eq!(parsed.get("B"), Some(&Rule::NameContains("sans".into())));
        assert_eq!(parse_smart("X\tbogus\n\tfixed\nnotab\n").len(), 0);
        assert_eq!(Rule::Static.title(), "Static");
    }

    #[test]
    fn prefs_round_trip_clamp_and_default() {
        let mut p = Prefs::default();
        assert!(p.validate_before_install);
        assert_eq!(p.sample(), PREVIEW_TEXT);
        p.validate_before_install = false;
        p.sample_text = "Hello\tworld".into();
        p.set_size(500.0);
        assert_eq!(p.sample_size, SIZE_MAX);
        p.set_size(1.0);
        assert_eq!(p.sample_size, SIZE_MIN);
        p.set_size(40.0);
        let back = Prefs::parse(&p.to_text());
        assert!(!back.validate_before_install);
        assert_eq!(back.sample(), "Hello world");
        assert_eq!(back.sample_size, 40.0);
        assert_eq!(Prefs::parse("size\tabc\njunk\n"), Prefs::default());
    }

    #[test]
    fn duplicates_keep_the_newest_file() {
        let f = |fam: &str, p: &str, m: u64| FontFile { family: fam.into(), path: PathBuf::from(p), modified: m };
        let files = vec![f("Inter", "/u/a.ttf", 10), f("inter", "/u/b.ttf", 30), f("Inter", "/u/c.ttf", 20), f("Code", "/u/code.otf", 5)];
        let d = find_duplicates(&files);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].keep, PathBuf::from("/u/b.ttf"));
        assert_eq!(d[0].remove, vec![PathBuf::from("/u/c.ttf"), PathBuf::from("/u/a.ttf")]);
        assert!(find_duplicates(&files[3..]).is_empty());
    }

    #[test]
    fn resolving_duplicates_removes_only_user_files() {
        let dir = std::env::temp_dir().join(format!("fontbook-model-dups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let user = dir.join("user");
        std::fs::create_dir_all(&user).unwrap();
        let old = user.join("old.ttf");
        std::fs::write(&old, b"x").unwrap();
        let system = dir.join("system.ttf");
        std::fs::write(&system, b"x").unwrap();
        let dup = Duplicate { family: "F".into(), keep: user.join("new.ttf"), remove: vec![old.clone(), system.clone()] };
        assert_eq!(resolve_duplicates(&[dup], &user), 1);
        assert!(!old.exists());
        assert!(system.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn urls_name_a_family() {
        assert_eq!(family_from_url("fontbook://Inter"), Some("Inter".to_string()));
        assert_eq!(family_from_url("fontbook:Fira%20Code"), Some("Fira Code".to_string()));
        assert_eq!(family_from_url("fontbook:///Noto+Sans?x=1"), Some("Noto Sans".to_string()));
        assert_eq!(family_from_url("fontbook://%zz"), Some("%zz".to_string()));
        assert_eq!(family_from_url("fontbook://"), None);
        assert_eq!(family_from_url("https://x"), None);
        assert_eq!(family_from_url("/home/u/a.ttf"), None);
    }

    #[test]
    fn unique_names_count_up_ignoring_case() {
        let taken = vec!["new collection".to_string(), "New Collection 2".to_string()];
        assert_eq!(unique_name(&taken, "New Collection"), "New Collection 3");
        assert_eq!(unique_name(&[], "Fixed Width"), "Fixed Width");
    }

    #[test]
    fn uninstall_only_touches_the_user_dir() {
        let user = Path::new("/home/u/.local/share/fonts");
        assert!(uninstall(Path::new("/usr/share/fonts/x.ttf"), user).is_err());
        assert!(uninstall(user, user).is_err());
    }

    #[test]
    fn install_refuses_a_font_file_that_fails_validation_without_copying_it() {
        let dir = std::env::temp_dir().join(format!("fontbook-model-invalid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("Broken.ttf");
        std::fs::write(&src, b"not a font at all").unwrap();
        let user = dir.join("user");
        let err = install(&src, &user, true).unwrap_err();
        assert_eq!(err, "The file has no TrueType or OpenType header.");
        assert!(!user.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn repertoire_rows_show_characters_coverage_and_validity() {
        let ok = validate::Report { problems: Vec::new(), glyphs: 12, characters: 95, blocks: vec!["Basic Latin".to_string()], warnings: Vec::new() };
        let rows = repertoire_rows(&ok);
        assert!(rows.contains(&("Characters".to_string(), "95".to_string())));
        assert!(rows.contains(&("Coverage".to_string(), "Basic Latin".to_string())));
        assert!(rows.contains(&("Validation".to_string(), "Valid".to_string())));
        let bad = validate::Report { problems: vec!["The maxp table is missing.".to_string()], ..validate::Report::default() };
        let rows = repertoire_rows(&bad);
        assert!(rows.contains(&("Coverage".to_string(), "None".to_string())));
        assert!(rows.contains(&("Validation".to_string(), "The maxp table is missing.".to_string())));
    }

    #[test]
    fn libraries_round_trip_and_skip_bad_lines() {
        let text = "Work\t/a/fonts\t/b/fonts\nwork\t/c\nComputer\t/x\n\tbad\nEmpty\n";
        let sets = parse_font_sets(text);
        assert_eq!(sets.len(), 1, "a built-in name, an empty name and a folderless line are skipped");
        assert_eq!(sets[0].dirs, [PathBuf::from("/a/fonts"), PathBuf::from("/b/fonts"), PathBuf::from("/c")], "a repeated name merges");
        assert_eq!(parse_font_sets(&font_sets_file(&sets)), sets);
        assert_eq!(library_names(&sets), ["Computer", "User", "Work"]);
        assert!(is_custom_library(&sets, "WORK") && !is_custom_library(&sets, "User"));
    }

    #[test]
    fn a_library_reads_its_own_folders() {
        let home = Path::new("/home/a");
        let shell = Path::new("/var/lib/assets/fonts");
        let sets = vec![FontSet { name: "Work".into(), dirs: vec![PathBuf::from("/w")] }];
        assert_eq!(library_dirs("Computer", &sets, home, shell), scan_dirs(home, shell));
        assert_eq!(library_dirs("user", &sets, home, shell), [home.join(USER_FONTS_DIR)]);
        assert_eq!(library_dirs("work", &sets, home, shell), [PathBuf::from("/w")]);
        assert_eq!(library_dirs("Gone", &sets, home, shell), scan_dirs(home, shell), "an unknown library reads as Computer");
    }

    #[test]
    fn adding_a_folder_names_the_library_after_it() {
        let mut sets = Vec::new();
        assert_eq!(add_font_set(&mut sets, Path::new("/m/Fonts")).unwrap(), "Fonts");
        assert_eq!(add_font_set(&mut sets, Path::new("/n/fonts")).unwrap(), "Fonts", "the same name joins the library");
        assert_eq!(add_font_set(&mut sets, Path::new("/n/fonts")).unwrap(), "Fonts");
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].dirs.len(), 2);
        assert_eq!(add_font_set(&mut sets, Path::new("/x/User")).unwrap(), "User 2", "a built-in name gets a number");
        assert!(add_font_set(&mut sets, Path::new("relative")).is_err());
        assert_eq!(folder_label(Path::new("/")), "Fonts");
        assert!(remove_font_set(&mut sets, "fonts"));
        assert!(!remove_font_set(&mut sets, "Computer"));
        assert_eq!(sets.len(), 1);
    }

    #[test]
    fn a_deleted_or_relative_folder_link_is_refused() {
        assert_eq!(dir_from_link(Path::new("/home/a/Fonts")), Some(PathBuf::from("/home/a/Fonts")));
        assert_eq!(dir_from_link(Path::new("/home/a/Fonts (deleted)")), None);
        assert_eq!(dir_from_link(Path::new("anon_inode:[x]")), None);
    }

    #[test]
    fn only_files_in_the_home_can_be_revealed() {
        let home = Path::new("/home/a");
        assert!(can_reveal(Path::new("/home/a/.local/share/fonts/A.ttf"), home));
        assert!(!can_reveal(Path::new("/usr/share/fonts/A.ttf"), home));
        assert!(!can_reveal(home, home));
        assert!(!can_reveal(Path::new("/x"), Path::new("")));
    }

    #[test]
    fn export_names_and_sample_sheets_describe_the_font() {
        assert_eq!(export_name("/x/y/Inter-Regular.otf"), "Inter-Regular.otf");
        assert_eq!(export_name(""), "Font.ttf");
        let info = sample().info("Inter");
        let sheet = sample_sheet("Inter", &info, "Hello");
        assert!(sheet.starts_with("Inter\n\n"));
        assert!(sheet.contains("Pitch: Proportional\n"));
        assert!(!sheet.contains("Family: "), "the title already names the family");
        assert!(sheet.contains("Sample\nHello\n"));
        assert!(sheet.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZ"));
    }

    #[test]
    fn warnings_show_in_the_validation_row_without_failing_it() {
        let warned = validate::Report { warnings: vec!["a".into(), "b".into()], ..validate::Report::default() };
        let rows = repertoire_rows(&warned);
        assert!(rows.contains(&("Validation".to_string(), "Valid, 2 warnings".to_string())));
        let one = validate::Report { warnings: vec!["a".into()], ..validate::Report::default() };
        assert!(repertoire_rows(&one).contains(&("Validation".to_string(), "Valid, 1 warning".to_string())));
    }

    #[test]
    fn the_chosen_library_is_saved_with_the_settings() {
        let mut p = Prefs::default();
        assert_eq!(p.library, LIBRARY_COMPUTER);
        p.library = "Work".into();
        assert_eq!(Prefs::parse(&p.to_text()).library, "Work");
        assert_eq!(Prefs::parse("library\t \n").library, LIBRARY_COMPUTER);
    }
}
