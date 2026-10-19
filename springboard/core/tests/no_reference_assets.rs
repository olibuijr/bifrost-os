//! Guard: BifrOSt loads only its own and openly licensed assets at run time.
//!
//! The reference asset layer (a core module that searched an installed root and a per-user folder for assets that
//! belong to the reference vendor) is gone. This test scans every Rust source under `springboard/` and fails when
//! one names that root again, brings the module back, or lists the vendor symbol font. The needles are assembled
//! from parts, so this file never contains them itself.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            // build output and the packed (gitignored) assets hold no sources of ours; symlinks are never followed
            if !matches!(name.as_str(), "target" | "assets" | ".git") && !name.starts_with("target-") {
                rust_sources(&entry.path(), out);
            }
        } else if kind.is_file() && name.ends_with(".rs") {
            out.push(entry.path());
        }
    }
}

fn sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("springboard/");
    let mut files = Vec::new();
    rust_sources(root, &mut files);
    assert!(files.len() > 100, "found only {} sources under {}", files.len(), root.display());
    files
        .into_iter()
        .filter_map(|p| Some((p.strip_prefix(root).ok()?.to_string_lossy().into_owned(), fs::read_to_string(&p).ok()?)))
        .collect()
}

#[test]
fn no_source_reads_the_reference_asset_root_or_brings_the_resolver_back() {
    let needles = [
        ["usr", "share", "bifrost", "ref"].join("/"),
        ["ref", "assets"].join("_"),
        ["BIFROST", "REF", "ROOT"].join("_"),
        ["bifrost-ref", "assets"].join("/"),
    ];
    let mut hits = Vec::new();
    for (path, text) in sources() {
        for needle in &needles {
            if text.contains(needle.as_str()) {
                hits.push(format!("{path} names `{needle}`"));
            }
        }
    }
    assert!(hits.is_empty(), "run-time reference assets are not allowed:\n{}", hits.join("\n"));
}

#[test]
fn only_the_symbol_font_loader_test_names_the_vendor_symbol_font() {
    let vendor = ["SFUI", "Symbols"].concat();
    let hits: Vec<String> = sources().into_iter().filter(|(_, text)| text.contains(&vendor)).map(|(path, _)| path).collect();
    // render/src/symbols.rs keeps a test that a file with that name beside the assets is never opened
    assert_eq!(hits, ["render/src/symbols.rs"], "the vendor symbol font must not be loaded or listed anywhere else");
}
