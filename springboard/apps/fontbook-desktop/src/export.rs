//! Export of font files into a folder. Each copy gets a name that no file already in the folder, and no earlier file of
//! the same export, uses, so an export never overwrites. Pure planning plus one copy that refuses to replace a file;
//! `main.rs` supplies the folder (the descriptor of the folder panel's answer) and reports the count.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Where each font file goes: `dest/<file name>`, numbered (`name 2.ttf`) when the name is taken by an earlier file in
/// this export or by a name in `existing`. A path with no file name is skipped.
pub fn plan(files: &[PathBuf], dest: &Path, existing: &[String]) -> Vec<(PathBuf, PathBuf)> {
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

/// Copies `src` to `dest`, which must not exist yet: an existing file is an error, never replaced.
pub fn copy_new(src: &Path, dest: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(src)?;
    std::fs::OpenOptions::new().write(true).create_new(true).open(dest)?.write_all(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(plan: &[(PathBuf, PathBuf)]) -> Vec<String> {
        plan.iter().map(|(_, d)| d.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn export_never_overwrites() {
        let files = vec![PathBuf::from("/x/Mono.ttf"), PathBuf::from("/y/Mono.ttf"), PathBuf::from("/z/Serif.otf")];
        let plan = plan(&files, Path::new("/out"), &["Serif.otf".to_string()]);
        assert_eq!(names(&plan), ["Mono.ttf", "Mono 2.ttf", "Serif 2.otf"]);
        assert_eq!(plan[0].0, PathBuf::from("/x/Mono.ttf"));
        assert_eq!(plan[0].1, PathBuf::from("/out/Mono.ttf"));
    }

    #[test]
    fn export_numbers_names_without_an_extension() {
        let files = vec![PathBuf::from("/x/Mono"), PathBuf::from("/y/Mono")];
        assert_eq!(names(&plan(&files, Path::new("/out"), &[])), ["Mono", "Mono 2"]);
    }

    #[test]
    fn export_skips_a_path_with_no_file_name() {
        let plan = plan(&[PathBuf::from("/")], Path::new("/out"), &[]);
        assert!(plan.is_empty(), "a root path has no file name to copy under");
    }

    #[test]
    fn export_keeps_the_order_of_the_families_shown() {
        let files = vec![PathBuf::from("/a/Zed.ttf"), PathBuf::from("/a/Alpha.ttf")];
        assert_eq!(names(&plan(&files, Path::new("/out"), &[])), ["Zed.ttf", "Alpha.ttf"]);
    }
}
