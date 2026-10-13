//! Save Messages: writes the messages the window shows (or the selected one) to a text file in the user's Documents
//! folder. The text is pure (one line per message, the same line as Copy Message); `save` does the file write and
//! never overwrites a file.

use crate::model::LogEntry;
use crate::services::copy_line;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where saved messages go, relative to the home directory.
pub const EXPORT_DIR: &str = "Documents/Console";

/// One line per message, each ending in a newline. Empty when there are no messages.
pub fn text<'a>(entries: impl IntoIterator<Item = &'a LogEntry>) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str(&copy_line(e));
        out.push('\n');
    }
    out
}

/// The file name for a save made at `unix_secs`.
pub fn file_name(unix_secs: i64) -> String {
    format!("Console-{unix_secs}.log")
}

/// Writes `text` to `dir/name`, creating `dir` first. A name already taken gets a numbered suffix
/// (`Console-5-2.log`), so an earlier save is never replaced. Returns the path written.
pub fn save(dir: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let stem = name.strip_suffix(".log").unwrap_or(name);
    for n in 1..=100u32 {
        let file = if n == 1 { name.to_string() } else { format!("{stem}-{n}.log") };
        let path = dir.join(file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(text.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free file name"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Level;

    fn entry(process: &str, message: &str) -> LogEntry {
        LogEntry { process: process.into(), pid: 9, level: Level::Error, message: message.into(), ..LogEntry::default() }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("console-export-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn text_is_one_newline_terminated_line_per_message() {
        assert_eq!(text(std::iter::empty()), "");
        let rows = [entry("notes", "boom"), entry("books", "ok")];
        let out = text(rows.iter());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("notes[9] Error boom"), "{out}");
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn file_names_carry_the_save_time() {
        assert_eq!(file_name(1_700_000_000), "Console-1700000000.log");
    }

    #[test]
    fn save_writes_the_text_and_never_overwrites() {
        let dir = scratch("save");
        let first = save(&dir, "Console-5.log", "one\n").unwrap();
        let second = save(&dir, "Console-5.log", "two\n").unwrap();
        assert_eq!(first.file_name().unwrap(), "Console-5.log");
        assert_eq!(second.file_name().unwrap(), "Console-5-2.log");
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "two\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
