//! Reference asset layer: finds personal-use reference assets (alert sounds, cursors, wallpapers, icons, symbols,
//! fonts) installed beside the system, so callers can prefer them and fall back to the built-in default when
//! a name is absent. The search order is `$BIFROST_REF_ROOT` (tests and development), `/usr/share/bifrost/ref`,
//! then an optional bundled default root. Packaging converts the files to formats the pure-Rust decoders read
//! (wav and png), so this module only finds paths. Name mappings come from `INDEX.tsv` in a root, never from
//! Rust literals: one line per name, `kind<TAB>name<TAB>file`, with `file` relative to the kind's folder.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The environment variable that names a development root.
pub const ROOT_ENV: &str = "BIFROST_REF_ROOT";
/// The installed root.
pub const SYSTEM_ROOT: &str = "/usr/share/bifrost/ref";

/// What a reference asset is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    AlertSound,
    SystemSound,
    Cursor,
    Wallpaper,
    /// By bundle id or app name.
    AppIcon,
    /// Folder, document, trash and so on.
    SystemIcon,
    Symbol,
    Font,
}

impl Kind {
    /// The folder under a root, and the first column of `INDEX.tsv`.
    pub const fn dir(self) -> &'static str {
        match self {
            Kind::AlertSound => "sounds/alerts",
            Kind::SystemSound => "sounds/system",
            Kind::Cursor => "cursors",
            Kind::Wallpaper => "wallpapers",
            Kind::AppIcon => "icons/apps",
            Kind::SystemIcon => "icons/system",
            Kind::Symbol => "symbols",
            Kind::Font => "fonts",
        }
    }

    /// The extensions the loader accepts, best first. Sources in other formats are converted at packaging time.
    pub const fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::AlertSound | Kind::SystemSound => &["wav", "aiff", "caf"],
            Kind::Cursor | Kind::AppIcon | Kind::SystemIcon | Kind::Symbol => &["png", "icns"],
            Kind::Wallpaper => &["png", "heic", "jpg", "jpeg"],
            Kind::Font => &["ttf", "otf"],
        }
    }
}

/// Lowercases a name and removes a known asset extension, so `Basso.AIFF`, `basso.wav` and `Basso` match.
pub fn normalize(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    if let Some((stem, ext)) = lower.rsplit_once('.') {
        if matches!(ext, "aiff" | "aif" | "caf" | "wav" | "png" | "heic" | "icns" | "jpg" | "jpeg" | "ttf" | "otf") {
            return stem.to_string();
        }
    }
    lower
}

/// Finds reference assets in an ordered list of roots.
pub struct Resolver {
    roots: Vec<PathBuf>,
    cache: Mutex<HashMap<(Kind, String), Option<PathBuf>>>,
}

impl Resolver {
    /// A resolver over explicit roots, searched in order.
    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        Self { roots, cache: Mutex::new(HashMap::new()) }
    }

    /// The standard order: `$BIFROST_REF_ROOT`, the installed root, then `bundled` when given.
    pub fn standard(bundled: Option<&Path>) -> Self {
        let mut roots = Vec::new();
        if let Some(env) = std::env::var_os(ROOT_ENV).filter(|v| !v.is_empty()) {
            roots.push(PathBuf::from(env));
        }
        roots.push(PathBuf::from(SYSTEM_ROOT));
        roots.extend(bundled.map(Path::to_path_buf));
        Self::with_roots(roots)
    }

    /// The path of the named asset, or `None` when no root has it (the caller then uses its built-in default).
    pub fn resolve(&self, kind: Kind, name: &str) -> Option<PathBuf> {
        let key = (kind, normalize(name));
        if key.1.is_empty() {
            return None;
        }
        if let Some(hit) = self.cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return hit;
        }
        let found = self.roots.iter().find_map(|root| find_in_root(root, kind, &key.1));
        if let Ok(mut c) = self.cache.lock() {
            c.insert(key, found.clone());
        }
        found
    }

    /// Every asset of a kind (sorted by path, first root wins per normalised name).
    pub fn list(&self, kind: Kind) -> Vec<PathBuf> {
        let mut seen: HashMap<String, PathBuf> = HashMap::new();
        for root in &self.roots {
            let Ok(rd) = fs::read_dir(root.join(kind.dir())) else { continue };
            for entry in rd.flatten() {
                let path = entry.path();
                let ext = path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).unwrap_or_default();
                if !kind.extensions().contains(&ext.as_str()) {
                    continue;
                }
                let stem = normalize(&path.file_name().unwrap_or_default().to_string_lossy());
                seen.entry(stem).or_insert(path);
            }
        }
        let mut out: Vec<PathBuf> = seen.into_values().collect();
        out.sort();
        out
    }

    /// Forgets cached lookups (after new assets are installed).
    pub fn clear_cache(&self) {
        if let Ok(mut c) = self.cache.lock() {
            c.clear();
        }
    }
}

fn find_in_root(root: &Path, kind: Kind, name: &str) -> Option<PathBuf> {
    let dir = root.join(kind.dir());
    // the index maps a name to a file; an indexed name wins over a direct stem match
    if let Some(file) = index_lookup(root, kind, name) {
        let path = dir.join(file);
        if path.is_file() {
            return Some(path);
        }
    }
    let entries: Vec<PathBuf> = fs::read_dir(&dir).ok()?.flatten().map(|e| e.path()).collect();
    // case-insensitive stem match, preferring the extension order
    for ext in kind.extensions() {
        let hit = entries.iter().find(|p| {
            p.extension().and_then(|e| e.to_str()).map_or(false, |e| e.eq_ignore_ascii_case(ext))
                && normalize(&p.file_name().unwrap_or_default().to_string_lossy()) == name
        });
        if let Some(p) = hit {
            return Some(p.clone());
        }
    }
    None
}

fn index_lookup(root: &Path, kind: Kind, name: &str) -> Option<String> {
    let text = fs::read_to_string(root.join("INDEX.tsv")).ok()?;
    parse_index(&text).into_iter().find(|(k, n, _)| *k == kind.dir() && *n == name).map(|(_, _, f)| f)
}

/// Parses `INDEX.tsv`: `kind<TAB>name<TAB>file`. Blank lines, `#` comments and short lines are skipped; names are normalised.
pub fn parse_index(text: &str) -> Vec<(String, String, String)> {
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let (kind, name, file) = (cols.next()?, cols.next()?, cols.next()?);
            (!file.trim().is_empty()).then(|| (kind.trim().to_string(), normalize(name), file.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-ref-assets"))
            .join(format!("{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    #[test]
    fn resolves_present_and_none_when_absent() {
        let r = root("present");
        touch(&r, "sounds/alerts/Basso.wav");
        let res = Resolver::with_roots(vec![r.clone()]);
        assert_eq!(res.resolve(Kind::AlertSound, "Basso"), Some(r.join("sounds/alerts/Basso.wav")));
        assert_eq!(res.resolve(Kind::AlertSound, "Glass"), None);
        assert_eq!(res.resolve(Kind::Cursor, "Basso"), None);
        assert_eq!(res.resolve(Kind::AlertSound, ""), None);
        let _ = fs::remove_dir_all(r);
    }

    #[test]
    fn normalises_case_and_extension() {
        let r = root("norm");
        touch(&r, "sounds/alerts/basso.wav");
        touch(&r, "cursors/Arrow.PNG");
        let res = Resolver::with_roots(vec![r.clone()]);
        assert!(res.resolve(Kind::AlertSound, "BASSO.aiff").is_some());
        assert!(res.resolve(Kind::Cursor, "arrow").is_some());
        assert_eq!(normalize("Pop.CAF"), "pop");
        assert_eq!(normalize("Mr. Smith"), "mr. smith");
        let _ = fs::remove_dir_all(r);
    }

    #[test]
    fn earlier_roots_win_and_later_roots_fill_in() {
        let (a, b) = (root("first"), root("second"));
        touch(&a, "wallpapers/one.png");
        touch(&b, "wallpapers/one.png");
        touch(&b, "wallpapers/two.png");
        let res = Resolver::with_roots(vec![a.clone(), b.clone()]);
        assert_eq!(res.resolve(Kind::Wallpaper, "one"), Some(a.join("wallpapers/one.png")));
        assert_eq!(res.resolve(Kind::Wallpaper, "two"), Some(b.join("wallpapers/two.png")));
        assert_eq!(res.list(Kind::Wallpaper), vec![a.join("wallpapers/one.png"), b.join("wallpapers/two.png")]);
        let _ = fs::remove_dir_all(a);
        let _ = fs::remove_dir_all(b);
    }

    #[test]
    fn index_maps_names_to_files() {
        let r = root("index");
        touch(&r, "icons/system/generic-folder.png");
        fs::write(r.join("INDEX.tsv"), "# kind\tname\tfile\nicons/system\tFolder\tgeneric-folder.png\nbad line\n").unwrap();
        let res = Resolver::with_roots(vec![r.clone()]);
        assert_eq!(res.resolve(Kind::SystemIcon, "folder"), Some(r.join("icons/system/generic-folder.png")));
        assert_eq!(parse_index("a\tB\tc.png\n\n#x\ty\n"), vec![("a".into(), "b".into(), "c.png".into())]);
        let _ = fs::remove_dir_all(r);
    }

    #[test]
    fn prefers_extension_order_and_caches() {
        let r = root("order");
        touch(&r, "sounds/system/tink.aiff");
        touch(&r, "sounds/system/tink.wav");
        let res = Resolver::with_roots(vec![r.clone()]);
        assert_eq!(res.resolve(Kind::SystemSound, "tink"), Some(r.join("sounds/system/tink.wav")));
        fs::remove_file(r.join("sounds/system/tink.wav")).unwrap();
        assert_eq!(res.resolve(Kind::SystemSound, "tink"), Some(r.join("sounds/system/tink.wav")), "cached");
        res.clear_cache();
        assert_eq!(res.resolve(Kind::SystemSound, "tink"), Some(r.join("sounds/system/tink.aiff")));
        let _ = fs::remove_dir_all(r);
    }

    #[test]
    fn missing_roots_resolve_nothing() {
        let res = Resolver::with_roots(vec![PathBuf::from("/nonexistent/ref")]);
        assert_eq!(res.resolve(Kind::Font, "x"), None);
        assert!(res.list(Kind::Font).is_empty());
    }
}
