//! The Script menu: the scripts in `~/.local/share/bifrost/scripts`, listed in the menu bar and run with `sh`.
//!
//! The menu extra shows only while the folder holds a script. Hidden files are not listed, and a chosen name must be
//! a plain file name inside the folder, so a menu row can never run a file outside it. A script runs detached with no
//! input and its output discarded: the shell does not wait for it.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The folder the menu lists: `$HOME/.local/share/bifrost/scripts`.
pub fn folder() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/bifrost/scripts"))
}

/// The script files in `dir` by file name, sorted. Hidden files and folders are left out.
pub fn list(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// The menu title of a script: its file name without a `.sh` extension.
pub fn title(name: &str) -> String {
    name.strip_suffix(".sh").unwrap_or(name).to_string()
}

/// The path of a script inside `dir`, or None when `name` is not a plain hidden-free file name.
pub fn path_of(dir: &Path, name: &str) -> Option<PathBuf> {
    let plain = !name.is_empty() && !name.starts_with('.') && !name.contains('/') && !name.contains('\0');
    plain.then(|| dir.join(name))
}

/// Runs a script of `dir` with `sh`, detached. Output is discarded and no input is given.
pub fn run(dir: &Path, name: &str) -> io::Result<()> {
    let path = path_of(dir, name).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a script name"))?;
    let mut child = Command::new("sh").arg(&path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bifrost-scripts-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_plain_files_sorted_and_skips_hidden_entries() {
        let dir = scratch("list");
        std::fs::write(dir.join("b.sh"), "echo b").unwrap();
        std::fs::write(dir.join("a.sh"), "echo a").unwrap();
        std::fs::write(dir.join(".hidden.sh"), "echo h").unwrap();
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        assert_eq!(list(&dir), vec!["a.sh".to_string(), "b.sh".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_folder_lists_nothing() {
        assert!(list(Path::new("/nonexistent/bifrost-scripts")).is_empty());
    }

    #[test]
    fn titles_drop_the_sh_extension() {
        assert_eq!(title("Backup photos.sh"), "Backup photos");
        assert_eq!(title("notes"), "notes");
    }

    #[test]
    fn only_plain_names_map_to_a_path() {
        let dir = Path::new("/home/u/.local/share/bifrost/scripts");
        assert_eq!(path_of(dir, "a.sh"), Some(dir.join("a.sh")));
        assert_eq!(path_of(dir, "../secret"), None);
        assert_eq!(path_of(dir, "sub/a.sh"), None);
        assert_eq!(path_of(dir, ".hidden"), None);
        assert_eq!(path_of(dir, ""), None);
    }

    #[test]
    fn running_a_listed_script_runs_it_with_sh() {
        let dir = scratch("run");
        let marker = dir.join("ran.txt");
        std::fs::write(dir.join("touch.sh"), format!("echo ok > '{}'", marker.display())).unwrap();
        run(&dir, "touch.sh").unwrap();
        // the script runs detached: wait for its marker rather than the child
        for _ in 0..200 {
            if marker.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(marker.exists(), "the script ran");
        assert!(run(&dir, "../etc").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
