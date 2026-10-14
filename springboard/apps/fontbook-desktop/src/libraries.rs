//! Font libraries: named sets of font folders that can be switched on and off, the list of folders a catalogue is
//! read from once libraries are applied, and export of selected font files into a folder (with names that never
//! overwrite) plus the lines of a printed sample sheet. Pure std; the sidebar and the export button call these.
#![allow(dead_code)] // the Libraries sidebar section and File > Export / Print Samples wire these in next

use std::path::{Path, PathBuf};

/// A named set of font folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub name: String,
    pub dirs: Vec<PathBuf>,
    pub enabled: bool,
}

/// The folders to scan: the base folders, then the folders of every enabled library, each once.
pub fn scan_dirs(base: &[PathBuf], libraries: &[Library]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = vec![];
    for d in base.iter().chain(libraries.iter().filter(|l| l.enabled).flat_map(|l| l.dirs.iter())) {
        if !out.contains(d) {
            out.push(d.clone());
        }
    }
    out
}

/// A library name safe to store: no tabs or newlines, trimmed, at most 40 characters; None when empty.
pub fn clean_name(name: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect();
    let n: String = n.trim().chars().take(40).collect();
    (!n.is_empty()).then_some(n)
}

/// Adds a library; false for a bad or taken name (names are compared ignoring case).
pub fn add(list: &mut Vec<Library>, name: &str, dirs: Vec<PathBuf>) -> bool {
    let Some(name) = clean_name(name) else { return false };
    if list.iter().any(|l| l.name.eq_ignore_ascii_case(&name)) {
        return false;
    }
    list.push(Library { name, dirs, enabled: true });
    true
}

/// Stored form, one library per line: `name<TAB>enabled<TAB>dir<TAB>dir...`.
pub fn to_text(list: &[Library]) -> String {
    list.iter()
        .map(|l| {
            let dirs: Vec<String> = l.dirs.iter().map(|d| d.display().to_string()).collect();
            format!("{}\t{}\t{}\n", l.name, l.enabled as u8, dirs.join("\t"))
        })
        .collect()
}

pub fn from_text(text: &str) -> Vec<Library> {
    let mut out: Vec<Library> = vec![];
    for line in text.lines() {
        let mut parts = line.split('\t');
        let (Some(name), Some(on)) = (parts.next(), parts.next()) else { continue };
        let dirs: Vec<PathBuf> = parts.filter(|d| !d.is_empty()).map(PathBuf::from).collect();
        add(&mut out, name, dirs);
        if let Some(last) = out.last_mut() {
            if last.name == name.trim() {
                last.enabled = on == "1";
            }
        }
    }
    out
}

/// Where each exported font file goes: `dest/<file name>`, numbered (`name 2.ttf`) when the name is taken by an
/// earlier file in this export or by a name in `existing`.
pub fn export_plan(files: &[PathBuf], dest: &Path, existing: &[String]) -> Vec<(PathBuf, PathBuf)> {
    let mut taken: Vec<String> = existing.to_vec();
    let mut out = vec![];
    for f in files {
        let Some(name) = f.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_string(), format!(".{e}")),
            None => (name.clone(), String::new()),
        };
        let mut candidate = name.clone();
        let mut n = 2;
        while taken.contains(&candidate) {
            candidate = format!("{stem} {n}{ext}");
            n += 1;
        }
        taken.push(candidate.clone());
        out.push((f.clone(), dest.join(candidate)));
    }
    out
}

/// The lines of a sample sheet: the family name, then the sample at each size. `sizes` are in points.
pub fn sample_sheet(family: &str, sample: &str, sizes: &[u32]) -> Vec<(String, u32)> {
    let mut out = vec![(family.to_string(), 14)];
    out.extend(sizes.iter().map(|s| (sample.to_string(), *s)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib(name: &str, dirs: &[&str], on: bool) -> Library {
        Library { name: name.into(), dirs: dirs.iter().map(PathBuf::from).collect(), enabled: on }
    }

    #[test]
    fn scan_dirs_include_enabled_libraries_once() {
        let base = vec![PathBuf::from("/a")];
        let libs = vec![lib("L1", &["/b", "/a"], true), lib("L2", &["/c"], false), lib("L3", &["/b", "/d"], true)];
        assert_eq!(scan_dirs(&base, &libs), vec![PathBuf::from("/a"), PathBuf::from("/b"), PathBuf::from("/d")]);
    }

    #[test]
    fn names_are_cleaned_and_unique() {
        let mut l = vec![];
        assert!(add(&mut l, "  Work\tFonts ", vec![]));
        assert!(!add(&mut l, "work fonts", vec![]));
        assert!(!add(&mut l, "   ", vec![]));
        assert_eq!(l[0].name, "WorkFonts");
        assert_eq!(clean_name(&"x".repeat(60)).map(|n| n.len()), Some(40));
    }

    #[test]
    fn stored_form_round_trips() {
        let l = vec![lib("Design", &["/fonts/a", "/fonts/b"], true), lib("Old", &[], false)];
        assert_eq!(from_text(&to_text(&l)), l);
        assert!(from_text("broken line").is_empty());
    }

    #[test]
    fn export_never_overwrites() {
        let files = vec![PathBuf::from("/x/Mono.ttf"), PathBuf::from("/y/Mono.ttf"), PathBuf::from("/z/Serif.otf")];
        let plan = export_plan(&files, Path::new("/out"), &["Serif.otf".to_string()]);
        let names: Vec<String> = plan.iter().map(|(_, d)| d.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["Mono.ttf", "Mono 2.ttf", "Serif 2.otf"]);
        assert_eq!(plan[0].0, PathBuf::from("/x/Mono.ttf"));
    }

    #[test]
    fn sheet_lists_each_size() {
        let s = sample_sheet("Mono", "Hamburgefonstiv", &[12, 24]);
        assert_eq!(s, vec![("Mono".to_string(), 14), ("Hamburgefonstiv".to_string(), 12), ("Hamburgefonstiv".to_string(), 24)]);
    }
}
