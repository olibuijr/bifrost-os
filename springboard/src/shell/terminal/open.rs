//! Open a `.command` file or an executable in Terminal. Each file runs in its own new tab, in its own folder, and the tab
//! stays at a login shell when the program ends (as Terminal does for a `.command`). The path is an argument of the
//! wrapper script, never spliced into it.
#![cfg(any(test, feature = "desktop-wayland"))]
use super::*;
#[cfg(feature = "desktop-wayland")]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
#[cfg(feature = "desktop-wayland")]
use std::path::PathBuf;

/// The argv that runs `path` in a new tab, or None when Terminal does not run that kind of file. A `.command` file runs
/// with `sh` when it is not executable; any other file runs only when it is executable. A relative path is refused.
pub fn open_argv(path: &Path, executable: bool) -> Option<Vec<String>> {
    if !path.is_absolute() {
        return None;
    }
    let command = path.extension().is_some_and(|e| e == "command");
    if !command && !executable {
        return None;
    }
    let runner = if command { r#"if [ -x "$0" ]; then "$0"; else sh "$0"; fi"# } else { r#""$0""# };
    let script = format!(r#"cd -- "${{0%/*}}" 2>/dev/null; {runner}; exec "${{SHELL:-/bin/sh}}" -l"#);
    Some(vec!["/bin/sh".into(), "-c".into(), script, path.to_string_lossy().into_owned()])
}

/// Whether `path` is a regular file with an execute bit.
#[cfg(feature = "desktop-wayland")]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

impl Shell {
    /// Runs each document Terminal can run in a new tab. Returns how many tabs opened; a file it cannot run is
    /// named in the message line.
    #[cfg(feature = "desktop-wayland")]
    pub(crate) fn terminal_open_documents(&mut self, paths: &[PathBuf]) -> usize {
        let mut opened = 0;
        for path in paths {
            let Some(argv) = open_argv(path, is_executable(path)) else {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.terminal_ui.message = format!("Terminal does not run {name}");
                continue;
            };
            let refs: Vec<&str> = argv.iter().map(String::as_str).collect();
            if self.spawn_terminal_tab(&refs, None) {
                opened += 1;
            }
        }
        if opened > 0 {
            self.dirty = true;
        }
        opened
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_is_refused() {
        assert_eq!(open_argv(Path::new("run.command"), true), None);
    }

    #[test]
    fn only_command_files_and_executables_run() {
        assert_eq!(open_argv(Path::new("/home/u/notes.txt"), false), None, "a text file is not run");
        assert!(open_argv(Path::new("/home/u/tool"), true).is_some(), "an executable runs");
        assert!(open_argv(Path::new("/home/u/Build.command"), false).is_some(), ".command runs without the exec bit");
    }

    #[test]
    fn the_path_is_an_argument_not_script_text() {
        let path = "/home/u/my dir/it's $(rm -rf x).command";
        let argv = open_argv(Path::new(path), false).unwrap();
        assert_eq!(argv[0], "/bin/sh");
        assert_eq!(argv[1], "-c");
        assert_eq!(argv[3], path, "the path travels as $0");
        assert!(!argv[2].contains("rm -rf"), "the script never contains the path");
        assert!(argv[2].contains("exec \"${SHELL:-/bin/sh}\" -l"), "the tab stays at a login shell");
    }

    #[test]
    fn a_command_file_without_the_exec_bit_is_read_by_sh() {
        let argv = open_argv(Path::new("/tmp/x.command"), false).unwrap();
        assert!(argv[2].contains("sh \"$0\""));
        let exe = open_argv(Path::new("/tmp/x"), true).unwrap();
        assert!(!exe[2].contains("sh \"$0\""), "an executable is run directly");
    }
}
