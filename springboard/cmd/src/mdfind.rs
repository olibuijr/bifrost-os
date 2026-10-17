//! `mdfind`: finds files under a folder by their names, from the command line. The desktop's quick search reads a
//! content index that the command line cannot reach yet, so this walks the folder tree instead: it matches each file
//! and folder name against the query, ignoring case, and skips hidden entries and symbolic links.
//! Supported: `-onlyin <dir>` (default: the home folder), `-name` (the name match; it is also the only match today),
//! `-count` (print how many paths matched), `-content` (also match text inside regular files, a case-insensitive
//! substring scan of files up to `CONTENT_LIMIT` bytes that skips binary files; the index's content search is still
//! missing). Gap: `-live` needs the index's change feed, which the command line cannot
//! reach yet, so it is refused rather than run as a one-off search. Metadata queries (`kMDItem...`) are not parsed; the
//! query is plain text.

use std::fs;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: mdfind [-onlyin directory] [-name] [-content] [-count] query";

/// The largest file whose text is scanned by `-content`.
pub const CONTENT_LIMIT: u64 = 4 * 1024 * 1024;

/// How deep the walk descends below the search folder.
pub const MAX_DEPTH: usize = 16;

/// The parsed command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub onlyin: Option<String>,
    pub count: bool,
    /// Also match the text inside files (`-content`).
    pub content: bool,
    /// The query words, joined with a space.
    pub query: String,
}

/// What the command line asks for: the help text, or a search.
#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Help,
    Run(Options),
}

/// Reads the arguments. Options may come before or after the query; `-name` is accepted and changes nothing today.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut o = Options::default();
    let mut words: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "-help" | "--help" => return Ok(Parsed::Help),
            "-onlyin" => {
                let dir = it.next().ok_or("-onlyin needs a folder")?;
                if o.onlyin.replace(dir.clone()).is_some() {
                    return Err("only one -onlyin folder can be given".into());
                }
            }
            "-name" => {}
            "-count" => o.count = true,
            "-content" => o.content = true,
            "-live" => return Err("-live needs the index's live updates, which are not available yet".into()),
            other if other.starts_with('-') && other.len() > 1 => return Err(format!("option {other} is not supported yet")),
            _ => words.push(arg.clone()),
        }
    }
    if words.is_empty() {
        return Err(USAGE.to_string());
    }
    o.query = words.join(" ");
    Ok(Parsed::Run(o))
}

/// Whether a file or folder name matches the query: a case-insensitive substring match on the whole query.
pub fn name_matches(name: &str, query: &str) -> bool {
    !query.is_empty() && name.to_lowercase().contains(&query.to_lowercase())
}

/// Every path under `root` (not `root` itself) whose name matches `query`, in a stable order: each folder's entries
/// sorted by name, depth first. Hidden entries and symbolic links are skipped, and the walk stops at `MAX_DEPTH`.
pub fn search(root: &Path, query: &str) -> Vec<PathBuf> {
    search_with(root, query, false)
}

/// As `search`, and with `content` also matches files whose text contains the query (`-content`).
pub fn search_with(root: &Path, query: &str, content: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, query, content, 0, &mut out);
    out
}

/// Whether a regular file's text contains the query, ignoring case. Files larger than `CONTENT_LIMIT`, unreadable
/// files and binary files (a NUL byte in the first 8 KiB) never match.
pub fn content_matches(path: &Path, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    let Ok(meta) = fs::metadata(path) else { return false };
    if !meta.is_file() || meta.len() > CONTENT_LIMIT {
        return false;
    }
    let Ok(bytes) = fs::read(path) else { return false };
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return false;
    }
    String::from_utf8_lossy(&bytes).to_lowercase().contains(&query.to_lowercase())
}

fn walk(dir: &Path, query: &str, content: bool, depth: usize, out: &mut Vec<PathBuf>) {
    if depth >= MAX_DEPTH {
        return;
    }
    let Ok(read) = fs::read_dir(dir) else { return };
    let mut entries: Vec<fs::DirEntry> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if name_matches(&name, query) || (content && content_matches(&path, query)) {
            out.push(path.clone());
        }
        if kind.is_dir() {
            walk(&path, query, content, depth + 1, out);
        }
    }
}

/// The folder a search starts in: `-onlyin`, else the home folder, else the current folder.
pub fn start_folder(onlyin: Option<&str>, home: Option<&str>) -> PathBuf {
    match onlyin {
        Some(dir) => PathBuf::from(dir),
        None => home.filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-mdfind-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn options_and_the_query_are_read_in_any_order() {
        let Parsed::Run(o) = parse(&args(&["-onlyin", "/srv", "report", "-count", "q1"])).unwrap() else { panic!() };
        assert_eq!(o.onlyin.as_deref(), Some("/srv"));
        assert!(o.count);
        assert_eq!(o.query, "report q1");
    }

    #[test]
    fn no_query_prints_usage_and_help_is_help() {
        assert_eq!(parse(&args(&["-count"])), Err(USAGE.to_string()));
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help));
    }

    #[test]
    fn live_and_unknown_options_are_refused_with_their_names() {
        assert!(parse(&args(&["-live", "x"])).unwrap_err().contains("live"));
        assert_eq!(parse(&args(&["-0", "x"])), Err("option -0 is not supported yet".to_string()));
        assert!(parse(&args(&["-onlyin"])).is_err());
    }

    #[test]
    fn names_match_ignoring_case_and_an_empty_query_matches_nothing() {
        assert!(name_matches("Quarterly Report.pdf", "report"));
        assert!(!name_matches("notes.txt", "report"));
        assert!(!name_matches("anything", ""));
    }

    #[test]
    fn search_walks_folders_skips_hidden_entries_and_keeps_order() {
        let root = scratch("walk");
        fs::create_dir_all(root.join("docs/deep")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("docs/report-a.txt"), b"x").unwrap();
        fs::write(root.join("docs/deep/report-b.txt"), b"x").unwrap();
        fs::write(root.join(".hidden/report-c.txt"), b"x").unwrap();
        fs::write(root.join("other.txt"), b"x").unwrap();
        // Order is by entry name within each folder, depth first: `deep` sorts before `report-a.txt`, so the
        // subfolder's match comes first.
        let found = search(&root, "REPORT");
        assert_eq!(found, vec![root.join("docs/deep/report-b.txt"), root.join("docs/report-a.txt")]);
        assert_eq!(search(&root, "other"), vec![root.join("other.txt")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_search_matches_text_and_skips_binary_files() {
        let root = scratch("content");
        fs::write(root.join("letter.txt"), b"The Quarterly Budget is attached").unwrap();
        fs::write(root.join("blob.bin"), b"budget\0\0\0").unwrap();
        fs::write(root.join("plain.txt"), b"nothing here").unwrap();
        assert!(content_matches(&root.join("letter.txt"), "budget"));
        assert!(!content_matches(&root.join("blob.bin"), "budget"));
        assert!(!content_matches(&root.join("plain.txt"), "budget"));
        assert!(!content_matches(&root.join("letter.txt"), ""));
        // Names alone miss `letter.txt`; with -content the text match is included.
        assert_eq!(search_with(&root, "budget", false), Vec::<PathBuf>::new());
        assert_eq!(search_with(&root, "budget", true), vec![root.join("letter.txt")]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn content_flag_is_parsed() {
        let Parsed::Run(o) = parse(&args(&["-content", "budget"])).unwrap() else { panic!() };
        assert!(o.content);
    }

    #[test]
    fn a_missing_folder_has_no_results() {
        assert!(search(Path::new("/definitely/not/here/bifrost"), "x").is_empty());
    }

    #[test]
    fn the_default_folder_is_home_then_the_current_folder() {
        assert_eq!(start_folder(Some("/srv"), Some("/home/u")), PathBuf::from("/srv"));
        assert_eq!(start_folder(None, Some("/home/u")), PathBuf::from("/home/u"));
        assert_eq!(start_folder(None, Some("")), PathBuf::from("."));
    }
}
