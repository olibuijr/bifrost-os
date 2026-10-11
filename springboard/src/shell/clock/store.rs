//! Small persistence helpers shared by alarms, timers and the stopwatch:
//! `key=value` text written through a sibling temporary file and a rename,
//! so a power cut never leaves a half-written store.

use std::path::Path;

/// Writes `text` to `path` atomically.
pub fn write_atomic(path: &str, text: &str) -> std::io::Result<()> {
    crate::shell::fsutil::write_atomic(Path::new(path), text)
}

/// The sibling file `name` next to the alarms file `path`.
pub fn sibling(path: &str, name: &str) -> String {
    Path::new(path).with_file_name(name).to_string_lossy().into_owned()
}

/// When the alarms file and the timers file beside it last changed, and how
/// big they were: the Clock app and the shell service each watch it to pick
/// up the other's edits.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stamp(Option<(std::time::SystemTime, u64)>, Option<(std::time::SystemTime, u64)>);

pub fn stamp(path: &str) -> Stamp {
    let one = |p: String| std::fs::metadata(p).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
    Stamp(one(path.to_string()), one(sibling(path, "clock-timers.ini")))
}

/// Splits `key=value` lines, skipping comments and junk.
pub fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines().filter_map(|l| {
        let l = l.trim();
        if l.starts_with('#') {
            return None;
        }
        l.split_once('=').map(|(k, v)| (k.trim(), v.trim()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_skip_comments_and_junk() {
        let got: Vec<_> = pairs("# c\nnonsense\na = 1\nb=x=y\n").collect();
        assert_eq!(got, [("a", "1"), ("b", "x=y")]);
    }

    #[test]
    fn sibling_replaces_the_file_name() {
        assert_eq!(sibling("/data/x/clock.ini", "timers.ini"), "/data/x/timers.ini");
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_file() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-clock-store").join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.ini");
        let p = p.to_str().unwrap();
        write_atomic(p, "one").unwrap();
        write_atomic(p, "two").unwrap();
        assert_eq!(std::fs::read_to_string(p).unwrap(), "two");
        assert!(!Path::new(&format!("{p}.tmp")).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
