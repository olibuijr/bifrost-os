//! Spotlight importer plug-ins (`CSImporter`, the metadata importers): read a file's content and return the metadata the
//! search index stores, the title, a description and keywords. An importer declares the content type it handles; for a
//! file the importer whose type is the closest ancestor of the file's type is used (see [`choose`]).
//!
//! Built in: a text importer for any `public.text` type (the first non-empty line is the title, the next one the
//! description, and the most frequent longer words are the keywords) and a universal fallback that takes the file name without its extension as
//! the title and the type's description as the description. A file with NUL bytes is not text, so the plain-text
//! importer declines it and the fallback answers.
//!
//! Reading: [`import_path`] reads at most [`MAX_HEAD`] bytes from the start of a file, so a large file costs a bounded
//! read. Gaps: no plug-in loading from bundles (the table is compiled in), no importers for binary formats (PDF, office
//! and image metadata). The directory walk that feeds the index is `scan_folder` in the shell's `spotindex`.

use super::uttype;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

/// The most bytes of a file that an importer reads.
pub const MAX_HEAD: usize = 64 * 1024;
/// The longest title or description kept, in characters.
pub const MAX_TEXT: usize = 200;
/// The most keywords kept.
pub const MAX_KEYWORDS: usize = 24;
/// Words shorter than this are not keywords.
const MIN_KEYWORD: usize = 4;

/// The metadata one importer produced for a file.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Imported {
    pub title: String,
    pub description: String,
    pub keywords: Vec<String>,
}

/// One importer: the content type it handles and the function that reads a file head. `None` means the importer does
/// not handle this content and the next choice answers.
pub struct Importer {
    pub name: &'static str,
    /// The content type this importer handles. The empty string handles any type.
    pub handles: &'static str,
    pub import: fn(file_name: &str, head: &[u8]) -> Option<Imported>,
}

/// The importers compiled into the shell, in the order they are offered.
pub const BUILT_IN: &[Importer] = &[
    Importer { name: "plain-text", handles: "public.text", import: import_plain_text },
    Importer { name: "file-name", handles: "", import: import_file_name },
];

/// The importer for a content type: the one whose type is the closest ancestor (fewest parent links). The universal
/// importer (empty `handles`) matches every type, at the largest distance, so it answers only when no other does.
pub fn choose(content_type: &str) -> Option<&'static Importer> {
    BUILT_IN
        .iter()
        .filter_map(|imp| {
            if imp.handles.is_empty() {
                Some((usize::MAX, imp))
            } else {
                uttype::distance(content_type, imp.handles).map(|d| (d, imp))
            }
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, imp)| imp)
}

/// Runs the importers for a file name and its head: the chosen importer first, then the universal one when the chosen
/// importer declines. The content type comes from the file name's extension.
pub fn import(file_name: &str, head: &[u8]) -> Imported {
    let content_type = uttype::type_for_extension(file_name).unwrap_or("");
    let head = &head[..head.len().min(MAX_HEAD)];
    choose(content_type)
        .and_then(|imp| (imp.import)(file_name, head))
        .or_else(|| import_file_name(file_name, head))
        .unwrap_or_default()
}

/// Reads at most [`MAX_HEAD`] bytes of the file at `path` and imports it under its file name.
pub fn import_path(path: &Path) -> std::io::Result<Imported> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let mut head = Vec::new();
    std::fs::File::open(path)?.take(MAX_HEAD as u64).read_to_end(&mut head)?;
    Ok(import(name, &head))
}

fn import_plain_text(_file_name: &str, head: &[u8]) -> Option<Imported> {
    if head.contains(&0) {
        return None;
    }
    let text = String::from_utf8_lossy(head);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let title = clip(lines.next().unwrap_or(""));
    let description = clip(lines.next().unwrap_or(""));
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        if word.chars().count() >= MIN_KEYWORD {
            *counts.entry(word.to_lowercase()).or_insert(0) += 1;
        }
    }
    // Most frequent first; ties keep alphabetical order (BTreeMap iteration order).
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1));
    let keywords = ranked.into_iter().take(MAX_KEYWORDS).map(|(w, _)| w).collect();
    Some(Imported { title, description, keywords })
}

fn import_file_name(file_name: &str, _head: &[u8]) -> Option<Imported> {
    let stem = match file_name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => file_name,
    };
    let content_type = uttype::type_for_extension(file_name).unwrap_or("");
    Some(Imported {
        title: clip(stem),
        description: clip(uttype::description(content_type)),
        keywords: Vec::new(),
    })
}

/// Trims text to [`MAX_TEXT`] characters.
fn clip(s: &str) -> String {
    s.chars().take(MAX_TEXT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_takes_title_description_and_keywords() {
        let head = b"\n  Quarterly report  \nRevenue grew in the quarter.\nrevenue revenue planning planning\n";
        let got = import("report.txt", head);
        assert_eq!(got.title, "Quarterly report");
        assert_eq!(got.description, "Revenue grew in the quarter.");
        assert_eq!(got.keywords[0], "revenue");
        assert!(got.keywords.contains(&"quarterly".to_string()));
        assert!(!got.keywords.iter().any(|k| k.len() < MIN_KEYWORD));
    }

    #[test]
    fn keywords_are_capped() {
        let head: String = (0..100).map(|i| format!("word{:03} ", i)).collect();
        let got = import("list.txt", head.as_bytes());
        assert_eq!(got.keywords.len(), MAX_KEYWORDS);
    }

    #[test]
    fn binary_content_falls_back_to_the_file_name() {
        let got = import("photo.png", &[0x89, 0x50, 0x00, 0x47]);
        assert_eq!(got.title, "photo");
        assert_eq!(got.description, uttype::description("public.png"));
        assert!(got.keywords.is_empty());
    }

    #[test]
    fn a_name_without_extension_is_its_own_title() {
        let got = import("Makefile", b"all:\n");
        assert_eq!(got.title, "Makefile");
    }

    #[test]
    fn choose_prefers_the_closest_ancestor() {
        assert_eq!(choose("public.plain-text").map(|i| i.name), Some("plain-text"));
        assert_eq!(choose("public.html").map(|i| i.name), Some("plain-text"));
        assert_eq!(choose("public.png").map(|i| i.name), Some("file-name"));
        assert_eq!(choose("").map(|i| i.name), Some("file-name"));
    }

    #[test]
    fn long_titles_are_clipped_by_characters() {
        let long = "é".repeat(MAX_TEXT + 10);
        let got = import("x.txt", long.as_bytes());
        assert_eq!(got.title.chars().count(), MAX_TEXT);
    }

    #[test]
    fn import_path_reads_the_file_head() {
        let dir = std::env::temp_dir().join(format!("bifrost-mdimport-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notes.md");
        std::fs::write(&path, "Shopping list\nmilk and bread\n").unwrap();
        let got = import_path(&path).unwrap();
        assert_eq!(got.title, "Shopping list");
        assert_eq!(got.description, "milk and bread");
        assert!(import_path(&dir.join("missing.txt")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
